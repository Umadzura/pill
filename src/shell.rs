//! Окно: размер и место, раскрытие/сворачивание, полноэкранный режим (AppBar), трей.

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{scale, st, Tab};
use crate::feat::ctl;
use crate::ids::*;
use crate::{pill, sys, ui};

// Размеры в логических пикселях (при 100% масштабе).
pub const COLLAPSED_MIN_W: i32 = 190;
pub const COLLAPSED_H: i32 = 32;
pub const EXPANDED_W: i32 = 560;
pub const EXPANDED_H: i32 = 340;
/// Ширина свёрнутой пилюли с полоской громкости.
pub const LEVEL_W: i32 = 240;
const TOP_MARGIN: i32 = 6;

pub struct Shell {
    pub expanded: bool,
    /// TrackMouseEvent уже взведён.
    pub tracking: bool,
    pub tab: Tab,
    pub dpi: u32,
    /// Последний текст свёрнутой пилюли: ширину пересчитываем только при его смене.
    pub last_collapsed: String,
    size: (i32, i32),
    pub taskbar_created: u32,
    pub icon: HICON,
    /// На нашем мониторе полноэкранное окно (игра, видео): пилюлю прячем.
    fullscreen: bool,
    /// Сколько секунд подряд проверка не видит полноэкранного окна.
    fs_miss: u8,
}

impl Shell {
    pub unsafe fn new() -> Self {
        Shell {
            expanded: false,
            tracking: false,
            tab: Tab::Music,
            dpi: 96,
            last_collapsed: String::new(),
            size: (0, 0),
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            icon: sys::icon::make(),
            fullscreen: false,
            fs_miss: 0,
        }
    }
    /// Сбросить запомненный размер: следующий layout() обязательно переставит окно.
    pub fn invalidate_size(&mut self) {
        self.size = (0, 0);
    }
}

/// Ставит окно по центру верхнего края основного монитора и задаёт скруглённую форму.
pub unsafe fn layout(hwnd: HWND) {
    let (expanded, dpi, text, level, lead) =
        st(|s| (s.shell.expanded, s.shell.dpi, pill::text(s), pill::shows_level(s), pill::lead_width(s)));
    let (w, h) = if expanded {
        (scale(EXPANDED_W, dpi), scale(EXPANDED_H, dpi))
    } else if level {
        (scale(LEVEL_W, dpi), scale(COLLAPSED_H, dpi))
    } else {
        let tw = ui::measure(hwnd, dpi, &text) + scale(lead, dpi);
        (tw.max(scale(COLLAPSED_MIN_W - 36, dpi)) + scale(36, dpi), scale(COLLAPSED_H, dpi))
    };
    let changed = st(|s| {
        s.shell.last_collapsed = text;
        let c = s.shell.size != (w, h);
        s.shell.size = (w, h);
        c
    });
    if changed {
        let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
        let y = scale(TOP_MARGIN, dpi);
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
        let radius = if expanded { scale(28, dpi) } else { h };
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius);
        SetWindowRgn(hwnd, rgn, true);
    }
    // В полноэкранном режиме прячемся. Открыли хоткеем — показываемся, пока панель раскрыта.
    let want = st(|s| !s.shell.fullscreen || s.shell.expanded);
    let visible = IsWindowVisible(hwnd).as_bool();
    if want && !visible {
        // Заодно снова встаём поверх: полноэкранные программы любят забирать TOPMOST.
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    } else if !want && visible {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    let _ = InvalidateRect(hwnd, None, false);
}

pub unsafe fn set_expanded(hwnd: HWND, value: bool) {
    let changed = st(|s| {
        let c = s.shell.expanded != value;
        s.shell.expanded = value;
        if c {
            s.hover = None;
            s.ctl.confirm = None;
        }
        c
    });
    if changed {
        if value {
            ctl::refresh();
        }
        layout(hwnd);
    }
}

pub unsafe fn cursor_inside(hwnd: HWND) -> bool {
    let mut pt = POINT::default();
    let mut rc = RECT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = GetWindowRect(hwnd, &mut rc);
    pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom
}

pub unsafe fn schedule_collapse(hwnd: HWND) {
    if !cursor_inside(hwnd) {
        SetTimer(hwnd, TIMER_COLLAPSE, COLLAPSE_DELAY_MS, None);
    }
}

/// TIMER_COLLAPSE: свернуться, если не печатаем и курсор не вернулся.
pub unsafe fn on_collapse_timer(hwnd: HWND) {
    let _ = KillTimer(hwnd, TIMER_COLLAPSE);
    if !st(|s| s.input.active) && !cursor_inside(hwnd) {
        set_expanded(hwnd, false);
    }
}

pub unsafe fn on_dpi(hwnd: HWND, dpi: u32) {
    st(|s| {
        s.shell.dpi = dpi;
        s.shell.invalidate_size();
    });
    layout(hwnd);
}

pub unsafe fn on_display_change(hwnd: HWND) {
    st(|s| s.shell.invalidate_size());
    layout(hwnd);
}

// ---------- полноэкранный режим ----------

/// Регистрация «панели приложения» без резерва места: так Explorer шлёт ABN_FULLSCREENAPP,
/// когда на экране появляется и пропадает полноэкранное окно.
pub unsafe fn appbar(hwnd: HWND, msg: u32) {
    let mut abd = APPBARDATA {
        cbSize: std::mem::size_of::<APPBARDATA>() as u32,
        hWnd: hwnd,
        uCallbackMessage: WM_APP_BAR,
        ..Default::default()
    };
    SHAppBarMessage(msg, &mut abd);
}

/// Активное окно закрывает весь монитор пилюли (игра, видео на весь экран)?
/// Рабочий стол (Progman/WorkerW) тоже «на весь экран», его не считаем.
unsafe fn fullscreen_fg(hwnd: HWND) -> bool {
    let fg = GetForegroundWindow();
    if fg.0.is_null() || fg == hwnd {
        return false;
    }
    let mut cls = [0u16; 64];
    let n = GetClassNameW(fg, &mut cls).max(0) as usize;
    let cls = String::from_utf16_lossy(&cls[..n]);
    if matches!(cls.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd") {
        return false;
    }
    let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY);
    if MonitorFromWindow(fg, MONITOR_DEFAULTTONULL) != mon {
        return false;
    }
    let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
    if !GetMonitorInfoW(mon, &mut mi).as_bool() {
        return false;
    }
    let mut rc = RECT::default();
    let _ = GetWindowRect(fg, &mut rc);
    let m = mi.rcMonitor;
    rc.left <= m.left && rc.top <= m.top && rc.right >= m.right && rc.bottom >= m.bottom
}

unsafe fn set_fullscreen(hwnd: HWND, value: bool) {
    let changed = st(|s| {
        s.shell.fs_miss = 0;
        std::mem::replace(&mut s.shell.fullscreen, value) != value
    });
    if changed {
        layout(hwnd);
    }
}

/// WM_APP_BAR от Explorer.
pub unsafe fn on_appbar(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    if wparam.0 as u32 == ABN_FULLSCREENAPP {
        // lparam != 0 — полноэкранное окно появилось (на каком-то мониторе), 0 — пропало.
        let on = lparam.0 != 0 && fullscreen_fg(hwnd);
        set_fullscreen(hwnd, on);
    }
}

/// Раз в секунду. Страховка: если «вышли из полноэкранного» не пришло, через 2 с без такого окна показываемся.
/// Пока панель открыта хоткеем поверх игры, активна она сама, поэтому не проверяем.
pub unsafe fn fullscreen_tick(hwnd: HWND) {
    if !st(|s| s.shell.fullscreen && !s.shell.expanded) {
        return;
    }
    let still = fullscreen_fg(hwnd);
    let gone = st(|s| {
        s.shell.fs_miss = if still { 0 } else { s.shell.fs_miss + 1 };
        s.shell.fs_miss >= 2
    });
    if gone {
        set_fullscreen(hwnd, false);
    }
}

// ---------- трей ----------

pub unsafe fn tray(hwnd: HWND, op: NOTIFY_ICON_MESSAGE) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: st(|s| s.shell.icon),
        ..Default::default()
    };
    let tip: Vec<u16> = "Pill".encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    let _ = Shell_NotifyIconW(op, &nid);
}

/// Explorer перезапустился: вернуть иконку и AppBar.
pub unsafe fn on_taskbar_created(hwnd: HWND) {
    tray(hwnd, NIM_ADD);
    appbar(hwnd, ABM_NEW);
}

pub unsafe fn on_tray(hwnd: HWND, lparam: LPARAM) {
    let ev = (lparam.0 & 0xFFFF) as u32;
    if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
        tray_menu(hwnd);
    }
}

unsafe fn tray_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let auto = if sys::autostart::enabled() { MF_CHECKED } else { MF_UNCHECKED };
    let _ = AppendMenuW(menu, MF_STRING | auto, MENU_AUTOSTART, w!("Запускать вместе с Windows"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    let _ = AppendMenuW(menu, MF_STRING, MENU_EXIT, w!("Выход"));
    let mut pt = POINT::default();
    let _ = GetCursorPos(&mut pt);
    // Без этого меню не закрывается по клику мимо.
    let _ = SetForegroundWindow(hwnd);
    let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN, pt.x, pt.y, 0, hwnd, None);
    let _ = DestroyMenu(menu);
    match cmd.0 as usize {
        MENU_AUTOSTART => sys::autostart::set(!sys::autostart::enabled()),
        MENU_EXIT => {
            let _ = DestroyWindow(hwnd);
        }
        _ => {}
    }
}
