//! Поток локального API: слушает 127.0.0.1:PORT и раздаёт запросы в `feat::api`.
//! В простое спит в `accept` (0% CPU). Соединения обрабатываются по одному и коротко:
//! таймаут чтения 2 с, не больше MAX_REQUEST байт. Порт занят — API молча выключен.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::feat::api::{self, Reply, MAX_REQUEST, PORT, QUEUE};
use crate::ids::WM_APP_API;

pub fn start(hwnd: HWND) {
    let raw = hwnd.0 as isize;
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, PORT)) else { return };
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
        Some(api::Event::Agent(u)) if u.answerable => HELD.lock().unwrap().push((u.id.clone(), s, Instant::now())),
        _ => respond(&mut s, &reply),
    }
    if let Some(ev) = event {
        QUEUE.lock().unwrap().push(ev);
        unsafe {
            let _ = PostMessageW(HWND(hwnd as *mut _), WM_APP_API, WPARAM(0), LPARAM(0));
        }
    }
}

/// Хуки PermissionRequest, которым ещё не ответили: (id агента, соединение, когда пришёл).
static HELD: Mutex<Vec<(String, TcpStream, Instant)>> = Mutex::new(Vec::new());
/// Дольше не держим: Claude Code спросит в своём окне. Таймаут хука в settings.json больше этого.
const HOLD: Duration = Duration::from_secs(30);

const ALLOW: &str = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}"#;
const DENY: &str = r#"{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"Отклонено в Pill"}}}"#;

/// Ответить ждущему хуку агента: Some(true) — разрешить, Some(false) — отклонить,
/// None — без решения (пусть спрашивает в своём окне). Зовёт UI-поток: запись в локальный сокет мгновенная.
pub fn answer(id: &str, decision: Option<bool>) {
    let mut held = HELD.lock().unwrap();
    let mut i = 0;
    while i < held.len() {
        if held[i].0 == id {
            let (_, mut s, _) = held.remove(i);
            let body = match decision {
                Some(true) => ALLOW,
                Some(false) => DENY,
                None => "",
            };
            respond(&mut s, &Reply { code: 200, body });
        } else {
            i += 1;
        }
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
    let status = match r.code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        _ => "Payload Too Large",
    };
    let msg = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        r.code,
        status,
        r.body.len(),
        r.body
    );
    let _ = s.write_all(msg.as_bytes());
}
