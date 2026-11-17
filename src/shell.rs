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

/// Как меняется размер во времени.
#[derive(Clone, Copy)]
enum Ease {
    /// Быстрый старт, мягкая остановка: смена высоты между вкладками.
    Out,
    /// Чуть проскакивает цель и возвращается, как пружина: раскрытие.
    Spring,
    /// Плавный разгон и торможение: сворачивание.
    InOut,
}

impl Ease {
    fn at(self, t: f32) -> f32 {
        match self {
            Ease::Out => 1.0 - (1.0 - t).powi(3),
            Ease::Spring => {
                let (c1, u) = (1.2, t - 1.0);
                1.0 + (c1 + 1.0) * u.powi(3) + c1 * u * u
            }
            Ease::InOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
                }
            }
        }
    }
}

/// Плавная смена размера окна: раскрытие, сворачивание, карточка, высота вкладки.
struct Anim {
    /// Какой вид был и какой будет (MODE_*): старое содержимое гаснет, новое проявляется.
    from_mode: u8,
    to_mode: u8,
    from: (i32, i32),
    to: (i32, i32),
    start: std::time::Instant,
    ms: f32,
    ease: Ease,
    /// Буфер с запасом на весь переход (с учётом проскока пружины).
    cap: (i32, i32),
    /// Курсор был над окном, когда переход начался (открыли наведением, а не хоткеем или скриптом).
    hovered: bool,
}

impl Anim {
    fn new(from_mode: u8, to_mode: u8, from: (i32, i32), to: (i32, i32)) -> Anim {
        let (ms, ease) = if from_mode == to_mode {
            (220.0, Ease::Out)
        } else if to_mode > from_mode {
            (if to_mode == MODE_EXPANDED { 360.0 } else { 300.0 }, Ease::Spring)
        } else {
            (240.0, Ease::InOut)
        };
        let cap = (from.0.max(to.0) * 11 / 10, from.1.max(to.1) * 11 / 10);
        Anim { from_mode, to_mode, from, to, start: std::time::Instant::now(), ms, ease, cap, hovered: false }
    }
    fn t(&self) -> f32 {
        (self.start.elapsed().as_secs_f32() * 1000.0 / self.ms).min(1.0)
    }
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
    /// Каким окно было при прошлой раскладке (MODE_*): смена запускает анимацию.
    shown_mode: u8,
    pub taskbar_created: u32,
    pub icon: HICON,
    /// На нашем мониторе полноэкранное окно (игра, видео): пилюлю прячем.
    fullscreen: bool,
    /// Сколько секунд подряд проверка не видит полноэкранного окна.
    fs_miss: u8,
    /// Высота раскрытой панели (логические px) по содержимому открытой вкладки; считает отрисовка.
    pub panel_h: i32,
    /// Курсор «ушёл» во время анимации: проверить, когда она закончится.
    pub leave_pending: bool,
    /// На время анимации включена точность таймеров 1 мс (timeBeginPeriod), её надо вернуть.
    hires: bool,
    /// Пилюлю спрятали сочетанием клавиш (Ctrl+Alt+H), чтобы достать то, что под ней.
    pub hidden: bool,
    /// Панель свернули кнопкой «Скриншот»: до этого момента (или пока курсор не уйдёт)
    /// наведение её не раскрывает, иначе она висит поверх выделения области.
    pub no_hover: Option<std::time::Instant>,
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
            shown_mode: MODE_COLLAPSED,
            taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
            icon: sys::icon::make(),
            fullscreen: false,
            fs_miss: 0,
            hidden: false,
            no_hover: None,
            hires: false,
            leave_pending: false,
            panel_h: EXPANDED_H,
        }
    }
    /// Пилюлю сейчас видно (в полноэкранном режиме прячемся, если панель не открыта хоткеем).
    pub fn visible(&self) -> bool {
        !self.hidden && (!self.fullscreen || self.expanded)
    }
    /// Идёт анимация: какой вид рисовать и насколько он проявлен (0..1). Сначала гаснет старый вид,
    /// потом проявляется новый; при росте окна новый появляется раньше, при сворачивании позже.
    pub fn anim_phase(&self) -> Option<(u8, f32)> {
        let a = self.anim.as_ref()?;
        if a.from_mode == a.to_mode {
            return Some((a.to_mode, 1.0));
        }
        let t = a.t();
        let split = if a.to_mode > a.from_mode { 0.25 } else { 0.4 };
        let smooth = |x: f32| x * x * (3.0 - 2.0 * x);
        Some(if t < split {
            (a.from_mode, smooth(1.0 - t / split))
        } else {
            (a.to_mode, smooth(((t - split) / (1.0 - split)).min(1.0)))
        })
    }
    /// Курсор ушёл, пока окно меняет размер. true — решение отложено до конца анимации.
    /// Если окно открыли не наведением, курсор мог попасть в окно только из-за проскока пружины:
    /// такой уход просто не считается.
    pub fn leave_during_anim(&mut self) -> bool {
        match self.anim.as_ref().map(|a| a.hovered) {
            Some(true) => {
                self.leave_pending = true;
                true
            }
            Some(false) => true,
            None => false,
        }
    }
    /// Сбросить запомненный размер: следующий layout() обязательно переставит окно.
    pub fn invalidate_size(&mut self) {
        self.size = (0, 0);
    }
}

/// Поставить окно размером w×h по центру верхнего края и сразу показать кадр этого размера.
/// Размер, место и картинка меняются одним вызовом UpdateLayeredWindow: если сначала двигать окно,
/// а потом рисовать, на один кадр видна старая картинка в новом месте, и анимация дёргается.
unsafe fn place(hwnd: HWND, w: i32, h: i32) {
    let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
    let cap = st(|s| s.shell.anim.as_ref().map(|a| a.cap));
    ui::render_sized(hwnd, w, h, Some(POINT { x, y: TOP_MARGIN }), cap);
}

/// Запустить анимацию. На её время таймеры Windows точнее (1 мс вместо 15,6), иначе кадры идут рывками.
unsafe fn start_anim(hwnd: HWND, mut a: Anim) {
    let first = st(|s| {
        a.hovered = s.shell.tracking;
        let first = !s.shell.hires;
        s.shell.hires = true;
        s.shell.anim = Some(a);
        first
    });
    if first {
        let _ = windows::Win32::Media::timeBeginPeriod(1);
    }
    SetTimer(hwnd, TIMER_ANIM, 8, None);
}

/// Считает размер окна по содержимому и ставит его по центру верхнего края основного монитора.
/// Форму (ушки, скруглённый низ) даёт отрисовка. Раскрытие и сворачивание идут плавно (TIMER_ANIM).
pub unsafe fn layout(hwnd: HWND) {
    let (now_mode, peek_h, dpi, text, level, lead, panel_h) = st(|s| {
        (mode(s), s.peek.as_ref().map(|p| (p.width(), p.height())), s.shell.dpi, pill::text(s), pill::shows_level(s), pill::lead_width(s), s.shell.panel_h)
    });
    let (w, h) = if now_mode == MODE_EXPANDED {
        (scale(EXPANDED_W, dpi), scale(panel_h, dpi))
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
        s.shell.last_collapsed = text;
        let c = s.shell.size != (w, h);
        s.shell.size = (w, h);
        (c, std::mem::replace(&mut s.shell.shown_mode, now_mode))
    });
    let old_mode = flip;
    let flip = flip != now_mode;
    if flip && visible {
        // Вид сменился: едем от текущего размера окна к новому.
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        start_anim(hwnd, Anim::new(old_mode, now_mode, (rc.right, rc.bottom), (w, h)));
    } else if st(|s| {
        s.shell.anim.as_mut().map(|a| {
            // Анимация уже идёт: поменялась только цель.
            a.to = (w, h);
            a.cap = (a.cap.0.max(w * 11 / 10), a.cap.1.max(h * 11 / 10));
        })
    })
    .is_some()
    {
    } else if changed && visible && now_mode == MODE_EXPANDED {
        // Другая вкладка — другая высота: панель подтягивается плавно, а не прыгает.
        let mut rc = RECT::default();
        let _ = GetClientRect(hwnd, &mut rc);
        start_anim(hwnd, Anim::new(MODE_EXPANDED, MODE_EXPANDED, (rc.right, rc.bottom), (w, h)));
    } else if changed {
        place(hwnd, w, h);
    }
    // В полноэкранном режиме прячемся. Открыли хоткеем — показываемся, пока панель раскрыта.
    let want = st(|s| s.shell.visible());
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
        let t = a.t();
        let k = a.ease.at(t);
        let lerp = |from: i32, to: i32| (from + ((to - from) as f32 * k).round() as i32).max(1);
        let size = (lerp(a.from.0, a.to.0), lerp(a.from.1, a.to.1));
        if t >= 1.0 {
            s.shell.anim = None;
        }
        Some((size, t >= 1.0))
    });
    let stop = |hwnd: HWND| {
        let _ = KillTimer(hwnd, TIMER_ANIM);
        if st(|s| std::mem::take(&mut s.shell.hires)) {
            let _ = windows::Win32::Media::timeEndPeriod(1);
        }
    };
    match step {
        Some((size, done)) => {
            if done {
                stop(hwnd);
            }
            // Последний кадр рисуется уже без запаса: буфер ужмётся до размера окна.
            place(hwnd, size.0, size.1);
            if done && st(|s| std::mem::take(&mut s.shell.leave_pending)) {
                // Курсор уходил, пока окно ехало: сворачиваемся, только если он и правда снаружи.
                schedule_collapse(hwnd);
            }
        }
        None => stop(hwnd),
    }
}

pub unsafe fn set_expanded(hwnd: HWND, value: bool) {
    let changed = st(|s| {
        let c = s.shell.expanded != value;
        s.shell.expanded = value;
        // Панель открыли (хоткеем поиска): спрятанная пилюля возвращается.
        if value {
            s.shell.hidden = false;
        }
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

/// Ctrl+Alt+H: спрятать пилюлю или вернуть её. Пока спрятана, окна нет вовсе: клики идут в то, что под ней.
/// Возвращают её то же сочетание, хоткей поиска и просьба агента о разрешении.
pub unsafe fn toggle_hidden(hwnd: HWND) {
    let hidden = st(|s| {
        s.shell.hidden = !s.shell.hidden;
        s.shell.hidden
    });
    if hidden {
        crate::input::close_panel(hwnd, true);
    }
    layout(hwnd);
    crate::feat::viz::refresh_want();
}

pub unsafe fn cursor_inside(hwnd: HWND) -> bool {
    let mut pt = POINT::default();
    let mut rc = RECT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = GetWindowRect(hwnd, &mut rc);
    pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom
}

/// Окно вот-вот сожмётся под неподвижным курсором (свернули кнопкой, убрали карточку):
/// не раскрываться от наведения, пока курсор не уйдёт (но не дольше 10 с).
pub fn hold_hover() {
    st(|s| s.shell.no_hover = Some(std::time::Instant::now() + std::time::Duration::from_secs(10)));
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

pub unsafe fn tray_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let auto = if sys::autostart::enabled() { MF_CHECKED } else { MF_UNCHECKED };
    let _ = AppendMenuW(menu, MF_STRING, MENU_SETTINGS, w!("Настройки…"));
    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, None);
    let _ = AppendMenuW(menu, MF_STRING | auto, MENU_AUTOSTART, w!("Запускать вместе с Windows"));
    let mut cfg = st(|s| s.settings.clone());
    let check = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
    let _ = AppendMenuW(menu, MF_STRING | check(cfg.dot_mode), MENU_DOT, w!("Просто точка: карточки не выезжают"));
    let _ = AppendMenuW(menu, MF_STRING | check(cfg.sound), MENU_SOUND, w!("Звуки: таймер, просьбы агентов, «закончил»"));
    let _ = AppendMenuW(menu, MF_STRING | check(cfg.quiet.is_some()), MENU_QUIET, w!("Тихие часы 23:00–8:00"));
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
        MENU_SETTINGS => ui::settings::open(hwnd),
        MENU_EXIT => {
            let _ = DestroyWindow(hwnd);
        }
        id @ (MENU_DOT | MENU_SOUND | MENU_QUIET) | id @ MENU_DELAY..=MENU_DELAY_LAST => {
            match id {
                MENU_DOT => cfg.dot_mode = !cfg.dot_mode,
                MENU_SOUND => cfg.sound = !cfg.sound,
                MENU_QUIET => cfg.quiet = if cfg.quiet.is_some() { None } else { Some((23, 8)) },
                _ => cfg.collapse_ms = delays[id - MENU_DELAY].0,
            }
            st(|s| s.settings = cfg.clone());
            cfg.save();
        }
        _ => {}
    }
}
