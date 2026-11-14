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
pub const COLLAPSED_MIN_W: i32 = 104;
pub const COLLAPSED_H: i32 = 32;
pub const EXPANDED_W: i32 = 600;
pub const EXPANDED_H: i32 = 400;
/// Ширина свёрнутой пилюли с полоской громкости.
pub const LEVEL_W: i32 = 240;
/// Чёлка прижата к верхнему краю экрана.
const TOP_MARGIN: i32 = 0;
/// Радиус нижних углов чёлки (верхние прямые).
pub const BOTTOM_R_COLLAPSED: i32 = 15;
pub const BOTTOM_R_EXPANDED: i32 = 28;
/// Ширина «ушка»: вогнутого перехода чёлки в верхний край экрана, с каждой стороны.
pub const EAR: i32 = 9;
/// Виды окна: свёрнутая чёлка, карточка (уведомление, просьба агента), раскрытая панель.
pub const MODE_COLLAPSED: u8 = 0;
pub const MODE_PEEK: u8 = 1;
pub const MODE_EXPANDED: u8 = 2;

/// Какой вид сейчас нужен.
pub fn mode(s: &crate::app::State) -> u8 {
    if s.shell.expanded {
        MODE_EXPANDED
    } else if s.peek.is_some() {
        MODE_PEEK
    } else {
        MODE_COLLAPSED
    }
}

/// Длительность раскрытия и сворачивания.
const ANIM_MS: f32 = 200.0;

/// Плавная смена размера окна при раскрытии и сворачивании.
struct Anim {
    /// Что рисовать, пока едем: больший из двух видов (он обрезается размером окна).
    view: u8,
    from: (i32, i32),
    to: (i32, i32),
    start: std::time::Instant,
}

pub struct Shell {
    pub expanded: bool,
    /// TrackMouseEvent уже взведён.
    pub tracking: bool,
    pub tab: Tab,
    /// Какая страница группы «Система» открывалась последней (Управление или Монитор).
    pub sys_page: Tab,
    pub dpi: u32,
    /// Последний текст свёрнутой пилюли: ширину пересчитываем только при его смене.
    pub last_collapsed: String,
    size: (i32, i32),
    anim: Option<Anim>,
    /// Вид, который был на экране до текущей анимации.
    last_view: u8,
    /// Каким окно было при прошлой раскладке (MODE_*): смена запускает анимацию.
    shown_mode: u8,
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
            sys_page: Tab::Controls,
            dpi: 96,
            last_collapsed: String::new(),
            size: (0, 0),
            anim: None,
            last_view: MODE_COLLAPSED,
            shown_mode: MODE_COLLAPSED,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            icon: sys::icon::make(),
            fullscreen: false,
            fs_miss: 0,
        }
    }
    /// Пилюлю сейчас видно (в полноэкранном режиме прячемся, если панель не открыта хоткеем).
    pub fn visible(&self) -> bool {
        !self.fullscreen || self.expanded
    }
    /// Идёт анимация: какой вид рисовать (MODE_*).
    pub fn anim_view(&self) -> Option<u8> {
        self.anim.as_ref().map(|a| a.view)
    }
    /// Сбросить запомненный размер: следующий layout() обязательно переставит окно.
    pub fn invalidate_size(&mut self) {
        self.size = (0, 0);
    }
}

/// Поставить окно размером w×h по центру верхнего края и сразу показать кадр этого размера
/// (слоёное окно само не перерисовывается).
unsafe fn place(hwnd: HWND, w: i32, h: i32) {
    let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
    let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, TOP_MARGIN, w, h, SWP_NOACTIVATE);
    ui::render(hwnd);
}

/// Считает размер окна по содержимому и ставит его по центру верхнего края основного монитора.
/// Форму (ушки, скруглённый низ) даёт отрисовка. Раскрытие и сворачивание идут плавно (TIMER_ANIM).
pub unsafe fn layout(hwnd: HWND) {
    let (now_mode, peek_h, dpi, text, level, lead) =
        st(|s| (mode(s), s.peek.as_ref().map(|p| (p.width(), p.height())), s.shell.dpi, pill::text(s), pill::shows_level(s), pill::lead_width(s)));
    let (w, h) = if now_mode == MODE_EXPANDED {
        (scale(EXPANDED_W, dpi), scale(EXPANDED_H, dpi))
    } else if let Some((pw, ph)) = peek_h {
        (scale(pw, dpi), scale(ph, dpi))
    } else if level {
        (scale(LEVEL_W, dpi), scale(COLLAPSED_H, dpi))
    } else {
        let tw = ui::measure(hwnd, dpi, &text) + scale(lead, dpi);
        (tw.max(scale(COLLAPSED_MIN_W - 28, dpi)) + scale(28, dpi), scale(COLLAPSED_H, dpi))
    };
    let w = w + 2 * scale(EAR, dpi);
    let visible = IsWindowVisible(hwnd).as_bool();
    let (changed, flip) = st(|s| {
        if s.shell.anim.is_none() {
            s.shell.last_view = s.shell.shown_mode;
        }
        s.shell.last_collapsed = text;
        let c = s.shell.size != (w, h);
        s.shell.size = (w, h);
        (c, std::mem::replace(&mut s.shell.shown_mode, now_mode))
    });
    let flip = flip != now_mode;
    if flip && visible {
        // Вид сменился: едем от текущего размера окна к новому.
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        st(|s| {
            let old = s.shell.anim_view().unwrap_or(0);
            let view = old.max(now_mode).max(if rc.bottom > scale(COLLAPSED_H, dpi) { s.shell.last_view } else { 0 });
            s.shell.anim = Some(Anim { view, from: (rc.right, rc.bottom), to: (w, h), start: std::time::Instant::now() })
        });
        SetTimer(hwnd, TIMER_ANIM, 15, None);
    } else if st(|s| s.shell.anim.as_mut().map(|a| a.to = (w, h)).is_some()) {
        // Анимация уже идёт: поменялась только цель.
    } else if changed {
        place(hwnd, w, h);
    }
    // В полноэкранном режиме прячемся. Открыли хоткеем — показываемся, пока панель раскрыта.
    let want = st(|s| !s.shell.fullscreen || s.shell.expanded);
    if want && !visible {
        // Заодно снова встаём поверх: полноэкранные программы любят забирать TOPMOST.
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW);
    } else if !want && visible {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    let _ = InvalidateRect(hwnd, None, false);
}

/// TIMER_ANIM: следующий кадр раскрытия или сворачивания.
pub unsafe fn on_anim_timer(hwnd: HWND) {
    let step = st(|s| {
        let a = s.shell.anim.as_ref()?;
        let t = (a.start.elapsed().as_secs_f32() * 1000.0 / ANIM_MS).min(1.0);
        // Быстрый старт, мягкая остановка.
        let k = 1.0 - (1.0 - t).powi(3);
        let lerp = |from: i32, to: i32| from + ((to - from) as f32 * k).round() as i32;
        let size = (lerp(a.from.0, a.to.0), lerp(a.from.1, a.to.1));
        if t >= 1.0 {
            s.shell.anim = None;
        }
        Some((size, t >= 1.0))
    });
    match step {
        Some((size, done)) => {
            if done {
                let _ = KillTimer(hwnd, TIMER_ANIM);
            }
            place(hwnd, size.0, size.1);
        }
        None => {
            let _ = KillTimer(hwnd, TIMER_ANIM);
        }
    }
}

pub unsafe fn set_expanded(hwnd: HWND, value: bool) {
    let changed = st(|s| {
        let c = s.shell.expanded != value;
        s.shell.expanded = value;
        // Поиск — не вкладка: свернувшись, возвращаемся туда, откуда его открыли.
        if !value && s.shell.tab == Tab::Commands {
            s.shell.tab = s.input.back.unwrap_or(Tab::Music);
        }
        if c {
            s.hover = None;
            s.ctl.confirm = None;
        }
        c
    });
    if changed {
        if value {
            ctl::refresh();
            // Раскрыли прямо на «Событиях»: пропущенные уведомления считаются увиденными.
            if st(|s| s.shell.tab == Tab::Feed) {
                crate::feat::notifs::INBOX.lock().unwrap().unread = 0;
            }
        }
        layout(hwnd);
        crate::feat::viz::refresh_want();
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
        SetTimer(hwnd, TIMER_COLLAPSE, st(|s| s.settings.collapse_ms), None);
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
        crate::feat::viz::refresh_want();
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
    let mut cfg = st(|s| s.settings.clone());
    let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
    let _ = AppendMenuW(menu, MF_STRING | check(cfg.dot_mode), MENU_DOT, w!("Просто точка: карточки не выезжают"));
    let _ = AppendMenuW(menu, MF_STRING | check(cfg.sound), MENU_SOUND, w!("Звуки: таймер, просьбы агентов, «закончил»"));
    let delays = [(200, w!("Быстро, 0,2 с")), (350, w!("Обычно, 0,35 с")), (600, w!("Медленно, 0,6 с")), (1000, w!("Очень медленно, 1 с"))];
    if let Ok(sub) = CreatePopupMenu() {
        for (i, (ms, label)) in delays.iter().enumerate() {
            let _ = AppendMenuW(sub, MF_STRING | check(cfg.collapse_ms == *ms), MENU_DELAY + i, *label);
        }
        // Подменю уничтожается вместе с главным меню.
        let _ = AppendMenuW(menu, MF_POPUP, sub.0 as usize, w!("Сворачиваться после ухода курсора"));
    }
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
        id @ (MENU_DOT | MENU_SOUND) | id @ MENU_DELAY..=MENU_DELAY_LAST => {
            match id {
                MENU_DOT => cfg.dot_mode = !cfg.dot_mode,
                MENU_SOUND => cfg.sound = !cfg.sound,
                _ => cfg.collapse_ms = delays[id - MENU_DELAY].0,
            }
            st(|s| s.settings = cfg.clone());
            cfg.save();
        }
        _ => {}
    }
}
