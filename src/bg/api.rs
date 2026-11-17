//! Поток локального API: слушает 127.0.0.1:PORT и раздаёт запросы в `feat::api`.
//! В простое спит в `accept` (0% CPU). Соединения обрабатываются по одному и коротко:
//! таймаут чтения 2 с, не больше MAX_REQUEST байт. Порт занят — API молча выключен.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::feat::api::{self, Reply, MAX_REQUEST, PORT, QUEUE};
use crate::ids::WM_APP_API;

/// Слушающий сокет не открылся (порт занят): окно настроек пишет «API не поднялся».
pub static DOWN: AtomicBool = AtomicBool::new(false);

pub fn start(hwnd: HWND) {
    let raw = hwnd.0 as isize;
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)) else {
        DOWN.store(true, Ordering::Relaxed);
        return;
    };
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            if let Ok(s) = conn {
                serve(s, raw);
            }
        }
    });
}

fn serve(mut s: TcpStream, hwnd: isize) {
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    // Читаем, пока не придут заголовки и всё тело (Content-Length) или не кончится лимит.
    loop {
        match s.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
        if buf.len() > MAX_REQUEST {
            break;
        }
        if let Some(need) = api::expected_len(&buf) {
            if buf.len() >= need {
                break;
            }
        }
    }
    let (reply, event) = api::handle(&buf);
    match &event {
        // Хук ждёт решения человека: ответим, когда он нажмёт кнопку в чёлке (или выйдет время).
        Some(api::Event::Agent(u)) if u.answerable => HELD.lock().unwrap().push((u.id.clone(), s, Instant::now(), u.extra.suggestions.clone())),
        _ => respond(&mut s, &reply),
    }
    if let Some(ev) = event {
        QUEUE.lock().unwrap().push(ev);
        unsafe {
            let _ = PostMessageW(HWND(hwnd as *mut _), WM_APP_API, WPARAM(0), LPARAM(0));
        }
    }
}

/// Хуки PermissionRequest, которым ещё не ответили: (id агента, соединение, когда пришёл,
/// правила «больше не спрашивать», которые прислал Claude Code).
static HELD: Mutex<Vec<(String, TcpStream, Instant, Option<String>)>> = Mutex::new(Vec::new());
/// Дольше не держим: Claude Code спросит в своём окне. Таймаут хука в settings.json больше этого.
const HOLD: Duration = Duration::from_secs(30);

const ALLOW: &str = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#;
const DENY: &str = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Отклонено в Pill"}}}"#;

/// Решение человека по просьбе агента.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Answer {
    Allow,
    /// Разрешить и больше не спрашивать: в ответе возвращаем правила, которые прислал сам Claude Code
    /// (`updatedPermissions` = `permission_suggestions`). Если он их не прислал, это обычное «разрешить».
    Always,
    Deny,
}

/// Ответить ждущему хуку агента. None — без решения (пусть спрашивает в своём окне).
/// Зовёт UI-поток: запись в локальный сокет мгновенная.
pub fn answer(id: &str, decision: Option<Answer>) {
    let mut held = HELD.lock().unwrap();
    let mut i = 0;
    while i < held.len() {
        if held[i].0 == id {
            let (_, mut s, _, rules) = held.remove(i);
            let always = match (decision, rules) {
                (Some(Answer::Always), Some(r)) => Some(format!(r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{{"behavior":"allow","updatedPermissions":{}}}}}}}"#, r)),
                _ => None,
            };
            let body = match (decision, &always) {
                (_, Some(b)) => b.as_str(),
                (Some(Answer::Allow | Answer::Always), None) => ALLOW,
                (Some(Answer::Deny), None) => DENY,
                (None, None) => "",
            };
            respond_text(&mut s, 200, body);
        } else {
            i += 1;
        }
    }
}

/// Ответить ждущему хуку готовым телом (ответ на вопрос агента).
pub fn answer_body(id: &str, body: &str) {
    let mut held = HELD.lock().unwrap();
    if let Some(i) = held.iter().position(|h| h.0 == id) {
        let (_, mut s, _, _) = held.remove(i);
        respond_text(&mut s, 200, body);
    }
}

/// Раз в секунду: отпустить хуки, которым не ответили вовремя. Возвращает id их агентов.
pub fn expire() -> Vec<String> {
    let old: Vec<String> = HELD.lock().unwrap().iter().filter(|h| h.2.elapsed() >= HOLD).map(|h| h.0.clone()).collect();
    for id in &old {
        answer(id, None);
    }
    old
}

fn respond(s: &mut TcpStream, r: &Reply) {
    respond_text(s, r.code, r.body);
}

fn respond_text(s: &mut TcpStream, code: u16, body: &str) {
    let status = match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Payload Too Large",
    };
    let msg = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        code,
        status,
        body.len(),
        body
    );
    let _ = s.write_all(msg.as_bytes());
}
