#![windows_subsystem = "windows"]

//! Pill: лёгкая панель сверху экрана. Чистый Win32 + GDI, без браузерного движка.
//! Перерисовка только по событиям (таймер раз в секунду, наведение, клики, события системы).
//! Устройство и правила — в ARCHITECTURE.md.

mod app;
mod bg;
mod feat;
mod ids;
mod input;
mod pill;
mod shell;
mod sys;
mod ui;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::DataExchange::AddClipboardFormatListener;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::{ABM_NEW, NIM_ADD, NIM_DELETE};
use windows::Win32::UI::WindowsAndMessaging::*;

use app::st;
use ids::*;

fn main() {
    // При падении оставляем след в %APPDATA%\Pill\crash.log (окна консоли нет).
    std::panic::set_hook(Box::new(|info| {
        if let Ok(dir) = std::env::var("APPDATA") {
            let dir = std::path::Path::new(&dir).join("Pill");
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(dir.join("crash.log"), format!("{}\n", info));
        }
    }));
    unsafe {
        let _mutex = CreateMutexW(None, true, w!("Local\\PillSingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        app::init_state();

        let hinstance = GetModuleHandleW(None).unwrap();
        let class = w!("PillWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(app::wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Pill"),
            WS_POPUP,
            0,
            0,
            shell::COLLAPSED_MIN_W,
            shell::COLLAPSED_H,
            None,
            None,
            hinstance,
            None,
        )
        .unwrap();

        let dpi = GetDpiForWindow(hwnd).max(96);
        st(|s| s.shell.dpi = dpi);
        shell::layout(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, TIMER_CLOCK, 1000, None);

        let _ = AddClipboardFormatListener(hwnd);
        // Alt+Space — командная строка (если занято, Ctrl+Alt+Space), Ctrl+Alt+V — буфер.
        if RegisterHotKey(hwnd, HOTKEY_CMD, MOD_ALT | MOD_NOREPEAT, VK_SPACE.0 as u32).is_err() {
            let _ = RegisterHotKey(hwnd, HOTKEY_CMD, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_SPACE.0 as u32);
        }
        let _ = RegisterHotKey(hwnd, HOTKEY_CLIP, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_V.0 as u32);
        shell::tray(hwnd, NIM_ADD);
        shell::appbar(hwnd, ABM_NEW);
        // Батарея и яркость ноутбука — событиями, без опроса.
        sys::power::register_notify(hwnd);
        // Если exe перенесли, а автозапуск включён — обновим путь.
        if sys::autostart::enabled() {
            sys::autostart::set(true);
        }
        bg::winrt::start(hwnd);
        bg::watch::start(hwnd);
        let watch = sys::audio::Watch::start(hwnd);
        st(|s| s.osd.audio = watch);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        shell::tray(hwnd, NIM_DELETE);
        // Отписка от звука до выхода из COM.
        drop(st(|s| s.osd.audio.take()));
    }
}
