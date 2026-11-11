#![windows_subsystem = "windows"]

//! Pill: лёгкая панель сверху экрана. Чистый Win32 + GDI, без браузерного движка.
//! Перерисовка только по событиям (таймер раз в секунду, наведение, клики, события системы).

mod clip;
mod launcher;
mod media;
mod sys;
mod ui;

use std::cell::RefCell;
use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::DataExchange::AddClipboardFormatListener;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// Размеры в логических пикселях (при 100% масштабе).
pub const COLLAPSED_MIN_W: i32 = 190;
pub const COLLAPSED_H: i32 = 32;
pub const EXPANDED_W: i32 = 560;
pub const EXPANDED_H: i32 = 340;
const TOP_MARGIN: i32 = 6;

const WM_MOUSELEAVE_MSG: u32 = 0x02A3;
const WM_CLIPBOARDUPDATE_MSG: u32 = 0x031D;
const WM_APP_TRAY: u32 = WM_APP + 3;

const TIMER_CLOCK: usize = 1;
const TIMER_COLLAPSE: usize = 2;
const TIMER_PASTE: usize = 3;
const TIMER_CTL: usize = 4;
const COLLAPSE_DELAY_MS: u32 = 350;

const HOTKEY_CMD: i32 = 1;
const HOTKEY_CLIP: i32 = 2;

const MENU_AUTOSTART: usize = 10;
const MENU_EXIT: usize = 11;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Music,
    Clipboard,
    Commands,
    Controls,
}

pub const TABS: [(Tab, &str); 4] = [
    (Tab::Music, "Музыка"),
    (Tab::Clipboard, "Буфер"),
    (Tab::Commands, "Команды"),
    (Tab::Controls, "Управление"),
];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ctl {
    Wifi,
    Bt,
    Vpn,
    Dark,
    Mute,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pw {
    Lock,
    Sleep,
    Restart,
    Shutdown,
}

/// Кликабельные зоны. Заполняются при отрисовке, по ним же ищем клик и наведение.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Tab(Tab),
    Input,
    MediaPrev,
    MediaPlay,
    MediaNext,
    Volume,
    ClipRow(usize),
    ClipPin(usize),
    ClipDel(usize),
    CmdRow(usize),
    Toggle(Ctl),
    Power(Pw),
    Alarm,
}

/// Строка результата в командной строке.
#[derive(Clone, PartialEq)]
pub enum CmdRes {
    Calc(String),
    Timer(u64),
    App(usize),
    Run(String),
    Active(usize),
}

pub struct Timer {
    pub end: Instant,
    pub total: u64,
}

#[derive(Default)]
pub struct Snapshot {
    pub volume: Option<(f32, bool)>,
    pub dark: Option<bool>,
    pub vpn: Option<(String, bool)>,
}

pub struct State {
    pub expanded: bool,
    tracking: bool,
    pub tab: Tab,
    pub dpi: u32,
    pub hits: Vec<(RECT, Hit)>,
    pub hover: Option<Hit>,
    pub input: String,
    pub input_active: bool,
    prev_fg: isize,
    pub sel: usize,
    pub scroll: usize,
    pub clip: clip::History,
    pub apps: Vec<launcher::App>,
    pub timers: Vec<Timer>,
    pub alarm: Option<String>,
    beeps: u8,
    pub ctl: Snapshot,
    pub confirm: Option<(Pw, Instant)>,
    pending_paste: bool,
    last_collapsed: String,
    size: (i32, i32),
    taskbar_created: u32,
    icon: HICON,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Доступ к состоянию. Внутри нельзя звать Win32, которые синхронно шлют сообщения окну.
pub fn st<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(s.borrow_mut().as_mut().expect("state")))
}

pub fn scale(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

fn hwnd_of(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

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

        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                expanded: false,
                tracking: false,
                tab: Tab::Music,
                dpi: 96,
                hits: Vec::new(),
                hover: None,
                input: String::new(),
                input_active: false,
                prev_fg: 0,
                sel: 0,
                scroll: 0,
                clip: clip::History::load(),
                apps: Vec::new(),
                timers: Vec::new(),
                alarm: None,
                beeps: 0,
                ctl: Snapshot::default(),
                confirm: None,
                pending_paste: false,
                last_collapsed: String::new(),
                size: (0, 0),
                taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                icon: sys::make_icon(),
            })
        });

        let hinstance = GetModuleHandleW(None).unwrap();
        let class = w!("PillWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
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
            COLLAPSED_MIN_W,
            COLLAPSED_H,
            None,
            None,
            hinstance,
            None,
        )
        .unwrap();

        let dpi = GetDpiForWindow(hwnd).max(96);
        st(|s| s.dpi = dpi);
        layout(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, TIMER_CLOCK, 1000, None);

        let _ = AddClipboardFormatListener(hwnd);
        // Alt+Space — командная строка (если занято, Ctrl+Alt+Space), Ctrl+Alt+V — буфер.
        if RegisterHotKey(hwnd, HOTKEY_CMD, MOD_ALT | MOD_NOREPEAT, VK_SPACE.0 as u32).is_err() {
            let _ = RegisterHotKey(hwnd, HOTKEY_CMD, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_SPACE.0 as u32);
        }
        let _ = RegisterHotKey(hwnd, HOTKEY_CLIP, MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_V.0 as u32);
        tray(hwnd, NIM_ADD);
        // Если exe перенесли, а автозапуск включён — обновим путь.
        if sys::autostart() {
            sys::set_autostart(true);
        }
        media::start(hwnd);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        tray(hwnd, NIM_DELETE);
    }
}

unsafe fn tray(hwnd: HWND, op: NOTIFY_ICON_MESSAGE) {
    let mut nid = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_APP_TRAY,
        hIcon: st(|s| s.icon),
        ..Default::default()
    };
    let tip: Vec<u16> = "Pill".encode_utf16().collect();
    nid.szTip[..tip.len()].copy_from_slice(&tip);
    let _ = Shell_NotifyIconW(op, &nid);
}

unsafe fn tray_menu(hwnd: HWND) {
    let Ok(menu) = CreatePopupMenu() else { return };
    let auto = if sys::autostart() { MF_CHECKED } else { MF_UNCHECKED };
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
        MENU_AUTOSTART => sys::set_autostart(!sys::autostart()),
        MENU_EXIT => {
            let _ = DestroyWindow(hwnd);
        }
        _ => {}
    }
}

/// Текст свёрнутой пилюли: [♪] время [⏱ остаток таймера] [батарея].
pub fn collapsed_text(s: &State) -> String {
    if let Some(a) = &s.alarm {
        return format!("⏰  {}", a);
    }
    let mut parts = Vec::new();
    let time = ui::time_text();
    parts.push(if media::playing() { format!("♪  {}", time) } else { time });
    if let Some(t) = s.timers.iter().min_by_key(|t| t.end) {
        let left = t.end.saturating_duration_since(Instant::now()).as_secs();
        parts.push(format!("⏱ {}", launcher::fmt_dur(left)));
    }
    if let Some(b) = ui::battery_text() {
        parts.push(b);
    }
    parts.join("   ")
}

/// Ставит окно по центру верхнего края основного монитора и задаёт скруглённую форму.
unsafe fn layout(hwnd: HWND) {
    let (expanded, dpi, text) = st(|s| (s.expanded, s.dpi, collapsed_text(s)));
    let (w, h) = if expanded {
        (scale(EXPANDED_W, dpi), scale(EXPANDED_H, dpi))
    } else {
        let tw = ui::measure(hwnd, dpi, &text);
        (tw.max(scale(COLLAPSED_MIN_W - 36, dpi)) + scale(36, dpi), scale(COLLAPSED_H, dpi))
    };
    let changed = st(|s| {
        s.last_collapsed = text;
        let c = s.size != (w, h);
        s.size = (w, h);
        c
    });
    if changed {
        let x = (GetSystemMetrics(SM_CXSCREEN) - w) / 2;
        let y = scale(TOP_MARGIN, dpi);
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        let radius = if expanded { scale(28, dpi) } else { h };
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius);
        SetWindowRgn(hwnd, rgn, true);
    }
    let _ = InvalidateRect(hwnd, None, false);
}

fn refresh_ctl() {
    let snap = Snapshot { volume: sys::volume(), dark: sys::dark_theme(), vpn: sys::vpn() };
    st(|s| s.ctl = snap);
    media::send(media::Job::Radios);
}

unsafe fn set_expanded(hwnd: HWND, value: bool) {
    let changed = st(|s| {
        let c = s.expanded != value;
        s.expanded = value;
        if c {
            s.hover = None;
            s.confirm = None;
        }
        c
    });
    if changed {
        if value {
            refresh_ctl();
        }
        layout(hwnd);
    }
}

unsafe fn cursor_inside(hwnd: HWND) -> bool {
    let mut pt = POINT::default();
    let mut rc = RECT::default();
    let _ = GetCursorPos(&mut pt);
    let _ = GetWindowRect(hwnd, &mut rc);
    pt.x >= rc.left && pt.x < rc.right && pt.y >= rc.top && pt.y < rc.bottom
}

unsafe fn schedule_collapse(hwnd: HWND) {
    if !cursor_inside(hwnd) {
        SetTimer(hwnd, TIMER_COLLAPSE, COLLAPSE_DELAY_MS, None);
    }
}

/// Включает ввод с клавиатуры: окно становится активным.
unsafe fn activate_input(hwnd: HWND, tab: Tab) {
    let fg = GetForegroundWindow();
    let scan = tab == Tab::Commands;
    let apps = if scan { launcher::scan() } else { Vec::new() };
    st(|s| {
        if fg != hwnd {
            s.prev_fg = fg.0 as isize;
        }
        if s.tab != tab || !s.input_active {
            s.input.clear();
        }
        s.tab = tab;
        s.input_active = true;
        s.sel = 0;
        s.scroll = 0;
        if scan {
            s.apps = apps;
        }
    });
    set_expanded(hwnd, true);
    sys::force_foreground(hwnd);
    let _ = SetFocus(hwnd);
    let _ = InvalidateRect(hwnd, None, false);
}

/// Выключает ввод. restore — вернуть фокус окну, где был пользователь.
unsafe fn deactivate_input(hwnd: HWND, restore: bool) {
    let prev = st(|s| {
        let was = s.input_active;
        s.input_active = false;
        if was {
            s.prev_fg
        } else {
            0
        }
    });
    if restore && prev != 0 && GetForegroundWindow() == hwnd {
        let _ = SetForegroundWindow(hwnd_of(prev));
    }
    let _ = InvalidateRect(hwnd, None, false);
}

unsafe fn close_panel(hwnd: HWND, restore: bool) {
    deactivate_input(hwnd, restore);
    let _ = KillTimer(hwnd, TIMER_COLLAPSE);
    set_expanded(hwnd, false);
}

/// Кладёт текст в буфер и вставляет его в окно, где работал пользователь.
unsafe fn paste(hwnd: HWND, text: &str) {
    if !clip::set_text(hwnd, text) {
        return;
    }
    close_panel(hwnd, true);
    st(|s| s.pending_paste = true);
    // Даём фокусу вернуться в целевое окно.
    SetTimer(hwnd, TIMER_PASTE, 80, None);
}

pub fn cmd_results(s: &State) -> Vec<CmdRes> {
    let q = s.input.trim();
    let mut out = Vec::new();
    if q.is_empty() {
        out.extend((0..s.timers.len()).map(CmdRes::Active));
        return out;
    }
    if let Some(v) = launcher::calc(q) {
        out.push(CmdRes::Calc(launcher::fmt_num(v)));
    }
    if let Some(t) = launcher::parse_timer(q) {
        out.push(CmdRes::Timer(t));
    }
    if !q.starts_with('>') {
        out.extend(launcher::search(&s.apps, q, 6).into_iter().map(CmdRes::App));
    }
    out.push(CmdRes::Run(q.to_string()));
    out
}

unsafe fn run_cmd(hwnd: HWND, res: CmdRes) {
    match res {
        CmdRes::Calc(v) => {
            clip::set_text(hwnd, &v);
            close_panel(hwnd, true);
        }
        CmdRes::Timer(secs) => {
            st(|s| {
                s.timers.push(Timer { end: Instant::now() + Duration::from_secs(secs), total: secs });
                s.input.clear();
                s.sel = 0;
            });
            close_panel(hwnd, true);
        }
        CmdRes::App(i) => {
            let path = st(|s| s.apps.get(i).map(|a| a.path.to_string_lossy().into_owned()));
            close_panel(hwnd, false);
            if let Some(p) = path {
                sys::open(&p, None);
            }
        }
        CmdRes::Run(line) => {
            close_panel(hwnd, false);
            sys::run_line(&line);
        }
        CmdRes::Active(i) => {
            st(|s| {
                if i < s.timers.len() {
                    s.timers.remove(i);
                }
            });
            layout(hwnd);
        }
    }
}

/// Список строк текущей вкладки для навигации стрелками.
fn list_len(s: &State) -> usize {
    match s.tab {
        Tab::Clipboard => s.clip.view(&s.input).len(),
        Tab::Commands => cmd_results(s).len(),
        _ => 0,
    }
}

unsafe fn on_enter(hwnd: HWND) {
    let (tab, sel) = st(|s| (s.tab, s.sel));
    match tab {
        Tab::Clipboard => {
            let text = st(|s| s.clip.view(&s.input).get(sel).map(|&i| s.clip.items[i].text.clone()));
            if let Some(t) = text {
                paste(hwnd, &t);
            }
        }
        Tab::Commands => {
            let res = st(|s| cmd_results(s).get(sel).cloned());
            if let Some(r) = res {
                run_cmd(hwnd, r);
            }
        }
        _ => {}
    }
}

unsafe fn on_char(hwnd: HWND, c: u32) {
    match c {
        0x1B | 0x0D | 0x09 => return, // Esc, Enter, Tab — в WM_KEYDOWN
        0x08 => st(|s| {
            s.input.pop();
        }),
        0x7F => st(|s| {
            // Ctrl+Backspace — удалить слово.
            let t = s.input.trim_end().len();
            s.input.truncate(t);
            let cut = s.input.rfind(' ').map(|p| p + 1).unwrap_or(0);
            s.input.truncate(cut);
        }),
        0x16 => {
            // Ctrl+V в поле ввода — первая строка буфера.
            if let Some(t) = clip::read_text(hwnd) {
                let line = t.lines().next().unwrap_or("").to_string();
                st(|s| s.input.push_str(&line));
            }
        }
        c if c >= 0x20 => {
            if let Some(ch) = char::from_u32(c) {
                st(|s| s.input.push(ch));
            }
        }
        _ => return,
    }
    st(|s| {
        s.sel = 0;
        s.scroll = 0;
    });
    let _ = InvalidateRect(hwnd, None, false);
}

unsafe fn on_key(hwnd: HWND, vk: u16) {
    match VIRTUAL_KEY(vk) {
        VK_ESCAPE => {
            let had = st(|s| {
                let had = !s.input.is_empty();
                s.input.clear();
                s.sel = 0;
                s.scroll = 0;
                had
            });
            if had {
                let _ = InvalidateRect(hwnd, None, false);
            } else {
                close_panel(hwnd, true);
            }
        }
        VK_RETURN => on_enter(hwnd),
        VK_UP | VK_DOWN => {
            st(|s| {
                let n = list_len(s);
                if n == 0 {
                    return;
                }
                s.sel = if vk == VK_UP.0 { s.sel.saturating_sub(1) } else { (s.sel + 1).min(n - 1) };
                let rows = ui::clip_rows(s.dpi);
                if s.sel < s.scroll {
                    s.scroll = s.sel;
                } else if s.sel >= s.scroll + rows {
                    s.scroll = s.sel + 1 - rows;
                }
            });
            let _ = InvalidateRect(hwnd, None, false);
        }
        VK_TAB => {
            let tab = st(|s| s.tab);
            let next = if tab == Tab::Commands { Tab::Clipboard } else { Tab::Commands };
            activate_input(hwnd, next);
        }
        _ => {}
    }
}

fn hit_at(s: &State, x: i32, y: i32) -> Option<(RECT, Hit)> {
    s.hits.iter().rev().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).copied()
}

unsafe fn on_click(hwnd: HWND, x: i32, y: i32) {
    let Some((rect, hit)) = st(|s| hit_at(s, x, y)) else { return };
    match hit {
        Hit::Tab(t) => {
            let active = st(|s| s.input_active);
            if active && matches!(t, Tab::Clipboard | Tab::Commands) {
                activate_input(hwnd, t);
            } else {
                if active {
                    deactivate_input(hwnd, true);
                }
                st(|s| {
                    s.tab = t;
                    s.sel = 0;
                    s.scroll = 0;
                    s.input.clear();
                });
                if t == Tab::Controls {
                    refresh_ctl();
                }
            }
        }
        Hit::Input => {
            let tab = st(|s| s.tab);
            activate_input(hwnd, tab);
        }
        Hit::MediaPrev => media::send(media::Job::Prev),
        Hit::MediaPlay => media::send(media::Job::PlayPause),
        Hit::MediaNext => media::send(media::Job::Next),
        Hit::Volume => {
            let w = (rect.right - rect.left).max(1);
            sys::set_volume((x - rect.left) as f32 / w as f32);
            refresh_ctl();
        }
        Hit::ClipRow(i) => {
            let text = st(|s| s.clip.items.get(i).map(|e| e.text.clone()));
            if let Some(t) = text {
                paste(hwnd, &t);
                return;
            }
        }
        Hit::ClipPin(i) => st(|s| s.clip.toggle_pin(i)),
        Hit::ClipDel(i) => st(|s| s.clip.remove(i)),
        Hit::CmdRow(i) => {
            let res = st(|s| cmd_results(s).get(i).cloned());
            if let Some(r) = res {
                run_cmd(hwnd, r);
                return;
            }
        }
        Hit::Toggle(c) => {
            match c {
                Ctl::Wifi | Ctl::Bt => {
                    let r = *media::RADIOS.lock().unwrap();
                    let (kind, cur) = if c == Ctl::Wifi { (media::Kind::Wifi, r.wifi) } else { (media::Kind::Bt, r.bt) };
                    if let Some(on) = cur {
                        media::send(media::Job::SetRadio(kind, !on));
                    } else {
                        sys::open(if c == Ctl::Wifi { "ms-settings:network-wifi" } else { "ms-settings:bluetooth" }, None);
                    }
                }
                Ctl::Vpn => {
                    sys::toggle_vpn();
                    // rasdial работает несколько секунд — перечитаем статус позже.
                    SetTimer(hwnd, TIMER_CTL, 3000, None);
                }
                Ctl::Dark => {
                    let dark = st(|s| s.ctl.dark.unwrap_or(false));
                    sys::set_dark_theme(!dark);
                }
                Ctl::Mute => sys::toggle_mute(),
            }
            refresh_ctl();
        }
        Hit::Power(p) => {
            let needs_confirm = matches!(p, Pw::Restart | Pw::Shutdown);
            let confirmed = st(|s| s.confirm.map(|(c, t)| c == p && t.elapsed() < Duration::from_secs(4)).unwrap_or(false));
            if needs_confirm && !confirmed {
                st(|s| s.confirm = Some((p, Instant::now())));
            } else {
                close_panel(hwnd, false);
                match p {
                    Pw::Lock => sys::lock(),
                    Pw::Sleep => sys::sleep(),
                    Pw::Restart => sys::restart(),
                    Pw::Shutdown => sys::shutdown(),
                }
                return;
            }
        }
        Hit::Alarm => {
            st(|s| {
                s.alarm = None;
                s.beeps = 0;
            });
            layout(hwnd);
        }
    }
    let _ = InvalidateRect(hwnd, None, false);
}

unsafe fn on_wheel(hwnd: HWND, delta: i32) {
    let tab = st(|s| s.tab);
    let notches = delta / 120;
    let notches = if notches == 0 { delta.signum() } else { notches };
    match tab {
        Tab::Music | Tab::Controls => {
            if let Some((v, _)) = sys::volume() {
                sys::set_volume(v + notches as f32 * 0.02);
            }
            refresh_ctl();
        }
        Tab::Clipboard => st(|s| {
            let n = s.clip.view(&s.input).len();
            let rows = ui::clip_rows(s.dpi);
            let max = n.saturating_sub(rows);
            s.scroll = (s.scroll as i32 - notches * 2).clamp(0, max as i32) as usize;
        }),
        Tab::Commands => {}
    }
    let _ = InvalidateRect(hwnd, None, false);
}

unsafe fn on_tick(hwnd: HWND) {
    let now = Instant::now();
    let fired: Vec<u64> = st(|s| {
        let fired = s.timers.iter().filter(|t| t.end <= now).map(|t| t.total).collect();
        s.timers.retain(|t| t.end > now);
        if s.confirm.map(|(_, t)| t.elapsed() > Duration::from_secs(4)).unwrap_or(false) {
            s.confirm = None;
        }
        fired
    });
    if let Some(&total) = fired.first() {
        st(|s| {
            s.alarm = Some(format!("Таймер {} — время вышло", launcher::fmt_dur(total)));
            s.beeps = 6;
        });
    }
    let beep = st(|s| {
        if s.beeps > 0 {
            s.beeps -= 1;
            true
        } else {
            false
        }
    });
    if beep {
        let _ = MessageBeep(MB_ICONEXCLAMATION);
    }
    // Перерисовываем только если что-то видимое поменялось.
    let (changed, expanded, live) = st(|s| {
        (collapsed_text(s) != s.last_collapsed, s.expanded, !s.timers.is_empty() || s.confirm.is_some())
    });
    if changed {
        layout(hwnd);
    } else if expanded && live {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

fn loword(l: LPARAM) -> i32 {
    (l.0 & 0xFFFF) as i16 as i32
}
fn hiword(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xFFFF) as i16 as i32
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            ui::paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // Клик по окну не забирает фокус у того, где ты печатаешь.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            let _ = KillTimer(hwnd, TIMER_COLLAPSE);
            let need_track = st(|s| !s.tracking);
            if need_track {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                st(|s| s.tracking = true);
            }
            let expanded = st(|s| s.expanded);
            if !expanded {
                set_expanded(hwnd, true);
            } else {
                let (x, y) = (loword(lparam), hiword(lparam));
                let changed = st(|s| {
                    let h = hit_at(s, x, y).map(|h| h.1);
                    let c = s.hover != h;
                    s.hover = h;
                    c
                });
                if changed {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            let active = st(|s| {
                s.tracking = false;
                s.hover = None;
                s.input_active
            });
            // Пока печатаешь, панель не прячется от ухода курсора.
            if !active {
                SetTimer(hwnd, TIMER_COLLAPSE, COLLAPSE_DELAY_MS, None);
            }
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            on_click(hwnd, loword(lparam), hiword(lparam));
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            on_wheel(hwnd, ((wparam.0 >> 16) & 0xFFFF) as i16 as i32);
            LRESULT(0)
        }
        WM_CHAR => {
            if st(|s| s.input_active) {
                on_char(hwnd, wparam.0 as u32);
            }
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if st(|s| s.input_active) {
                on_key(hwnd, wparam.0 as u16);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_ACTIVATE => {
            // Ушли в другое окно — ввод выключаем, панель прячем, если курсор не над ней.
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE && st(|s| s.input_active) {
                st(|s| s.input_active = false);
                schedule_collapse(hwnd);
                let _ = InvalidateRect(hwnd, None, false);
            }
            LRESULT(0)
        }
        WM_HOTKEY => {
            let tab = if wparam.0 as i32 == HOTKEY_CLIP { Tab::Clipboard } else { Tab::Commands };
            let (active, cur) = st(|s| (s.input_active && s.expanded, s.tab));
            if active && cur == tab {
                close_panel(hwnd, true);
            } else {
                activate_input(hwnd, tab);
            }
            LRESULT(0)
        }
        WM_CLIPBOARDUPDATE_MSG => {
            if let Some(text) = clip::read_text(hwnd) {
                let show = st(|s| {
                    s.clip.add(text);
                    s.expanded && s.tab == Tab::Clipboard
                });
                if show {
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        media::WM_APP_MEDIA => {
            // Значок ♪ в свёрнутом виде мог появиться/пропасть.
            layout(hwnd);
            LRESULT(0)
        }
        media::WM_APP_RADIO => {
            let _ = InvalidateRect(hwnd, None, false);
            LRESULT(0)
        }
        WM_APP_TRAY => {
            let ev = (lparam.0 & 0xFFFF) as u32;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                tray_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                TIMER_CLOCK => on_tick(hwnd),
                TIMER_COLLAPSE => {
                    let _ = KillTimer(hwnd, TIMER_COLLAPSE);
                    if !st(|s| s.input_active) && !cursor_inside(hwnd) {
                        set_expanded(hwnd, false);
                    }
                }
                TIMER_PASTE => {
                    let _ = KillTimer(hwnd, TIMER_PASTE);
                    if st(|s| std::mem::take(&mut s.pending_paste)) {
                        sys::send_ctrl_v();
                    }
                }
                TIMER_CTL => {
                    let _ = KillTimer(hwnd, TIMER_CTL);
                    refresh_ctl();
                    let _ = InvalidateRect(hwnd, None, false);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            st(|s| {
                s.dpi = (wparam.0 & 0xFFFF) as u32;
                s.size = (0, 0);
            });
            layout(hwnd);
            LRESULT(0)
        }
        // Смена разрешения / подключение монитора.
        WM_DISPLAYCHANGE => {
            st(|s| s.size = (0, 0));
            layout(hwnd);
            LRESULT(0)
        }
        WM_SETTINGCHANGE => {
            // Тема переключилась извне — обновим плитку.
            if st(|s| s.expanded && s.tab == Tab::Controls) {
                refresh_ctl();
                let _ = InvalidateRect(hwnd, None, false);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => {
            if msg != 0 && msg == st(|s| s.taskbar_created) {
                // Explorer перезапустился — вернуть иконку в трей.
                tray(hwnd, NIM_ADD);
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }
}
