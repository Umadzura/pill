//! Запуск программ и ссылок, вставка Ctrl+V, перевод фокуса.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Консольная программа без окна (rasdial, shutdown).
pub fn hidden(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd).args(args).creation_flags(CREATE_NO_WINDOW).spawn();
}

pub fn open(file: &str, params: Option<&str>) -> bool {
    unsafe {
        let f = HSTRING::from(file);
        let p = params.map(HSTRING::from);
        let r = ShellExecuteW(
            None,
            w!("open"),
            &f,
            p.as_ref().map(|p| PCWSTR(p.as_ptr())).unwrap_or(PCWSTR::null()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        r.0 as isize > 32
    }
}

/// Строка из командной строки: «> команда» в cmd, иначе как файл/URL/программа с аргументами.
pub fn run_line(line: &str) -> bool {
    let line = line.trim();
    if let Some(cmd) = line.strip_prefix('>') {
        return open("cmd.exe", Some(&format!("/k {}", cmd.trim())));
    }
    if open(line, None) {
        return true;
    }
    match line.split_once(' ') {
        Some((f, args)) => open(f, Some(args)),
        None => false,
    }
}

pub fn send_ctrl_v() {
    unsafe {
        let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Делает окно активным, даже если Windows упирается (через AttachThreadInput).
pub unsafe fn force_foreground(hwnd: HWND) {
    if SetForegroundWindow(hwnd).as_bool() {
        return;
    }
    let fg = GetForegroundWindow();
    let fg_thread = GetWindowThreadProcessId(fg, None);
    let me = GetCurrentThreadId();
    let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
    let _ = SetForegroundWindow(hwnd);
    let _ = BringWindowToTop(hwnd);
    if attached {
        let _ = AttachThreadInput(me, fg_thread, false);
    }
}
