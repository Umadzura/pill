//! Поток локального API: слушает 127.0.0.1:PORT и раздаёт запросы в `feat::api`.
//! В простое спит в `accept` (0% CPU). Соединения обрабатываются по одному и коротко:
//! весь запрос за 2 с, не больше MAX_REQUEST байт. Порт занят — API молча выключен.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
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
    // Срок общий на весь запрос: клиент, шлющий по байту, не держит единственный поток дольше него.
    let deadline = Instant::now() + READ_TIME;
    let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 2048];
    // Читаем, пока не придут заголовки и всё тело (Content-Length) или не кончится лимит или срок.
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() || s.set_read_timeout(Some(left)).is_err() {
            break;
        }
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
    let event = match event {
        // Хук ждёт решения человека: ответим, когда он нажмёт кнопку в чёлке (или выйдет время).
        Some(api::Event::Agent(mut u)) if u.answerable => {
            let mut held = HELD.lock().unwrap();
            if held.len() < MAX_HELD {
                let req = NEXT_REQ.fetch_add(1, Ordering::Relaxed);
                held.push(Held { req, id: u.id.clone(), s, at: Instant::now(), rules: u.extra.suggestions.clone() });
                Some(api::Event::Held(u, req))
            } else {
                drop(held);
                // Ждущих слишком много: сразу пустой ответ (Claude Code спросит в своём окне),
                // в чёлке просьба без кнопок.
                respond(&mut s, &reply);
                u.answerable = false;
                Some(api::Event::Agent(u))
            }
        }
        ev => {
            respond(&mut s, &reply);
            ev
        }
    };
    if let Some(ev) = event {
        QUEUE.lock().unwrap().push(ev);
        unsafe {
            let _ = PostMessageW(HWND(hwnd as *mut _), WM_APP_API, WPARAM(0), LPARAM(0));
        }
    }
}

/// Хук PermissionRequest, которому ещё не ответили.
struct Held<S> {
    /// Номер запроса (`PeekKind::Ask.req`): у каждой просьбы свой, даже внутри одной сессии.
    req: u64,
    /// id агента (сессии).
    id: String,
    s: S,
    /// Когда пришёл.
    at: Instant,
    /// Правила «больше не спрашивать», которые прислал Claude Code.
    rules: Option<String>,
}

static HELD: Mutex<Vec<Held<TcpStream>>> = Mutex::new(Vec::new());
/// Номер следующего ждущего запроса. 0 не выдаётся: это «хук не ждёт ответа».
static NEXT_REQ: AtomicU64 = AtomicU64::new(1);
/// Больше ждущих соединений не держим: лишним сразу пустой ответ.
const MAX_HELD: usize = 16;
/// За сколько должен прийти весь запрос.
const READ_TIME: Duration = Duration::from_secs(2);
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

/// Тело ответа хуку на решение человека. Пустое — без решения.
fn decision_body(decision: Option<Answer>, rules: Option<&str>) -> String {
    match (decision, rules) {
        (Some(Answer::Always), Some(r)) => format!(r#"{{"hookSpecificOutput":{{"hookEventName":"PermissionRequest","decision":{{"behavior":"allow","updatedPermissions":{}}}}}}}"#, r),
        (Some(Answer::Allow | Answer::Always), _) => ALLOW.into(),
        (Some(Answer::Deny), _) => DENY.into(),
        (None, _) => String::new(),
    }
}

/// Забрать ждущий запрос с этим номером. Другие запросы той же сессии остаются ждать.
fn take_req<S>(held: &mut Vec<Held<S>>, req: u64) -> Option<Held<S>> {
    let i = held.iter().position(|h| h.req == req)?;
    Some(held.remove(i))
}

/// Забрать все ждущие запросы сессии.
fn take_session<S>(held: &mut Vec<Held<S>>, id: &str) -> Vec<Held<S>> {
    let (out, keep) = std::mem::take(held).into_iter().partition(|h| h.id == id);
    *held = keep;
    out
}

/// Забрать запросы сессий, где хоть один ждёт дольше HOLD (сессию целиком). Возвращает и id этих сессий.
fn take_old<S>(held: &mut Vec<Held<S>>, now: Instant) -> (Vec<String>, Vec<Held<S>>) {
    let mut ids: Vec<String> = Vec::new();
    for h in held.iter() {
        if now.duration_since(h.at) >= HOLD && !ids.contains(&h.id) {
            ids.push(h.id.clone());
        }
    }
    let (out, keep) = std::mem::take(held).into_iter().partition(|h| ids.contains(&h.id));
    *held = keep;
    (ids, out)
}

/// Ответить ровно одному ждущему хуку: номер запроса из `PeekKind::Ask.req`.
/// None — без решения (пусть спрашивает в своём окне). Зовёт UI-поток: запись в локальный сокет мгновенная.
pub fn answer(req: u64, decision: Option<Answer>) {
    // Лок только на поиск, в сокет пишем уже без него.
    let Some(mut h) = take_req(&mut HELD.lock().unwrap(), req) else { return };
    respond_text(&mut h.s, 200, &decision_body(decision, h.rules.as_deref()));
}

/// Ответить ждущему хуку готовым телом (ответ на вопрос агента).
pub fn answer_body(req: u64, body: &str) {
    let Some(mut h) = take_req(&mut HELD.lock().unwrap(), req) else { return };
    respond_text(&mut h.s, 200, body);
}

/// Отпустить без решения все ждущие хуки сессии (агент перестал ждать: ответили в его окне).
pub fn release(id: &str) {
    let out = take_session(&mut HELD.lock().unwrap(), id);
    for mut h in out {
        respond_text(&mut h.s, 200, "");
    }
}

/// Раз в секунду: отпустить хуки, которым не ответили вовремя (все запросы их сессий). Возвращает id агентов.
pub fn expire() -> Vec<String> {
    let (ids, out) = take_old(&mut HELD.lock().unwrap(), Instant::now());
    for mut h in out {
        respond_text(&mut h.s, 200, "");
    }
    ids
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

#[cfg(test)]
mod tests {
    use super::*;

    fn held(req: u64, id: &str, at: Instant) -> Held<()> {
        Held { req, id: id.into(), s: (), at, rules: None }
    }

    #[test]
    fn one_answer_one_request() {
        let now = Instant::now();
        let mut list = vec![held(1, "s1", now), held(2, "s1", now), held(3, "s2", now)];
        // Ответ первому запросу сессии не трогает второй.
        assert_eq!(take_req(&mut list, 1).map(|h| h.req), Some(1));
        assert_eq!(list.iter().map(|h| h.req).collect::<Vec<_>>(), vec![2, 3]);
        // Повторный ответ тому же номеру и чужой номер — ничего.
        assert!(take_req(&mut list, 1).is_none());
        assert!(take_req(&mut list, 0).is_none());
        // Отпуск по сессии забирает все её запросы.
        list.push(held(4, "s1", now));
        assert_eq!(take_session(&mut list, "s1").iter().map(|h| h.req).collect::<Vec<_>>(), vec![2, 4]);
        assert_eq!(list.iter().map(|h| h.req).collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn expire_by_session() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_secs(20);
        let mut list = vec![held(1, "s1", t0), held(2, "s1", t1), held(3, "s2", t1)];
        let (ids, out) = take_old(&mut list, t0 + HOLD - Duration::from_secs(1));
        assert!(ids.is_empty() && out.is_empty());
        // Первый запрос s1 протух: уходит вся сессия s1, s2 ждёт дальше.
        let (ids, out) = take_old(&mut list, t0 + HOLD);
        assert_eq!(ids, vec!["s1".to_string()]);
        assert_eq!(out.iter().map(|h| h.req).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(list.iter().map(|h| h.req).collect::<Vec<_>>(), vec![3]);
    }

    #[test]
    fn decision_bodies() {
        assert_eq!(decision_body(Some(Answer::Allow), None), ALLOW);
        assert_eq!(decision_body(Some(Answer::Always), None), ALLOW);
        assert_eq!(decision_body(Some(Answer::Deny), Some("[1]")), DENY);
        assert_eq!(decision_body(None, Some("[1]")), "");
        assert!(decision_body(Some(Answer::Always), Some(r#"[{"type":"addRules"}]"#)).contains(r#""updatedPermissions":[{"type":"addRules"}]}"#));
    }
}
