//! Поток локального API: слушает 127.0.0.1:PORT и раздаёт запросы в `feat::api`.
//! В простое спит в `accept` (0% CPU). Соединения обрабатываются по одному и коротко:
//! таймаут чтения 2 с, не больше MAX_REQUEST байт. Порт занят — API молча выключен.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::time::Duration;

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
    respond(&mut s, &reply);
    if let Some(ev) = event {
        QUEUE.lock().unwrap().push(ev);
        unsafe {
            let _ = PostMessageW(HWND(hwnd as *mut _), WM_APP_API, WPARAM(0), LPARAM(0));
        }
    }
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
