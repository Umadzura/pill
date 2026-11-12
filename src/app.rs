//! Слой приложения: `State` из частей функций, `st()`, оконная процедура как диспетчер событий.
//! Логика функций живёт в `feat/`, окно в `shell`, клавиатура в `input`, отрисовка в `ui/`.

use std::cell::RefCell;
use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::bg::winrt;
use crate::feat::clip::{self, ClipHit};
use crate::feat::ctl::{self, CtlHit};
use crate::feat::music::{self, MusicHit};
use crate::feat::sysmon::{self, LoadAction};
use crate::feat::api;
use crate::feat::{cmd, osd, timers};
use crate::ids::*;
use crate::{input, pill, shell, sys, ui};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tab {
    Music,
    Clipboard,
    Commands,
    Controls,
    System,
}

pub const TABS: [(Tab, &str); 5] = [
    (Tab::Music, "Музыка"),
    (Tab::Clipboard, "Буфер"),
    (Tab::Commands, "Команды"),
    (Tab::Controls, "Управление"),
    (Tab::System, "Система"),
];

/// Кликабельные зоны. Заполняются при отрисовке, по ним же ищем клик и наведение.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    Tab(Tab),
    Input,
    /// Полоска громкости (есть и в «Музыке», и в «Управлении»).
    Volume,
    /// Полоска яркости в «Управлении» (только если есть чем управлять).
    Brightness,
    Music(MusicHit),
    Clip(ClipHit),
    /// Строка результата командной строки.
    Cmd(usize),
    Ctl(CtlHit),
    Alarm,
}

/// Что перерисовать после обработки события.
#[derive(Clone, Copy, PartialEq)]
pub enum Redraw {
    None,
    /// Поменялось содержимое: InvalidateRect.
    Content,
    /// Поменялся размер или вид пилюли: layout().
    Layout,
}

pub struct State {
    pub shell: shell::Shell,
    pub input: input::Input,
    pub hits: Vec<(RECT, Hit)>,
    pub hover: Option<Hit>,
    /// Где сейчас нарисованы полоски визуализатора (None — не видны).
    pub viz_area: Option<ui::viz::VizArea>,
    pub clip: clip::History,
    pub cmd: cmd::CmdState,
    pub ctl: ctl::CtlState,
    pub timers: timers::Timers,
    pub osd: osd::Osd,
    pub load: sysmon::LoadWatch,
    /// Задачи от скриптов (локальный API).
    pub tasks: api::Tasks,
    /// Воркер сейчас собирает снимки для вкладки «Система».
    pub sys_sampling: bool,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Доступ к состоянию. Внутри нельзя звать Win32, которые синхронно шлют сообщения окну.
pub fn st<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|s| f(s.borrow_mut().as_mut().expect("state")))
}

pub unsafe fn init_state() {
    let s = State {
        shell: shell::Shell::new(),
        input: input::Input::default(),
        hits: Vec::new(),
        hover: None,
        viz_area: None,
        clip: clip::History::load(),
        cmd: cmd::CmdState::default(),
        ctl: ctl::CtlState::default(),
        timers: timers::Timers::default(),
        osd: osd::Osd::new(),
        load: sysmon::LoadWatch::default(),
        tasks: api::Tasks::default(),
        sys_sampling: false,
    };
    STATE.with(|cell| *cell.borrow_mut() = Some(s));
}

pub fn scale(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

pub fn hwnd_of(raw: isize) -> HWND {
    HWND(raw as *mut _)
}

unsafe fn redraw(hwnd: HWND, r: Redraw) {
    match r {
        Redraw::None => {}
        Redraw::Content => {
            let _ = InvalidateRect(hwnd, None, false);
        }
        Redraw::Layout => shell::layout(hwnd),
    }
}

fn hit_at(s: &State, x: i32, y: i32) -> Option<(RECT, Hit)> {
    s.hits.iter().rev().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).copied()
}

unsafe fn on_click(hwnd: HWND, x: i32, y: i32) {
    let Some((rect, hit)) = st(|s| hit_at(s, x, y)) else { return };
    let r = match hit {
        Hit::Tab(t) => on_tab(hwnd, t),
        Hit::Input => {
            let tab = st(|s| s.shell.tab);
            input::activate(hwnd, tab);
            Redraw::None
        }
        Hit::Volume => ctl::volume_at(rect, x),
        Hit::Brightness => ctl::brightness_at(rect, x),
        Hit::Music(h) => music::on_hit(h, rect, x),
        Hit::Clip(h) => clip::on_hit(hwnd, h),
        Hit::Cmd(i) => {
            cmd::run_index(hwnd, i);
            Redraw::None
        }
        Hit::Ctl(h) => ctl::on_hit(hwnd, h),
        Hit::Alarm => {
            st(|s| s.timers.dismiss());
            Redraw::Layout
        }
    };
    redraw(hwnd, r);
}

unsafe fn on_tab(hwnd: HWND, t: Tab) -> Redraw {
    let active = st(|s| s.input.active);
    if active && matches!(t, Tab::Clipboard | Tab::Commands) {
        input::activate(hwnd, t);
        return Redraw::None;
    }
    if active {
        input::deactivate(hwnd, true);
    }
    st(|s| {
        s.shell.tab = t;
        s.input.sel = 0;
        s.input.scroll = 0;
        s.input.text.clear();
    });
    if t == Tab::Controls {
        ctl::refresh();
    }
    sys_sampling_tick();
    Redraw::Content
}

unsafe fn on_wheel(hwnd: HWND, delta: i32) {
    let notches = delta / 120;
    let notches = if notches == 0 { delta.signum() } else { notches };
    let (tab, hover) = st(|s| (s.shell.tab, s.hover));
    let r = match tab {
        Tab::Controls if hover == Some(Hit::Brightness) => ctl::brightness_wheel(notches),
        Tab::Music | Tab::Controls => ctl::volume_wheel(notches),
        Tab::Clipboard => {
            st(|s| input::scroll_clip(s, notches));
            Redraw::Content
        }
        Tab::Commands | Tab::System => Redraw::None,
    };
    redraw(hwnd, r);
}

/// Вкладка «Система» открыта — просим воркер снять снимок; закрыли — освободить.
fn sys_sampling_tick() {
    let (want, was) = st(|s| {
        let want = s.shell.expanded && s.shell.tab == Tab::System;
        (want, std::mem::replace(&mut s.sys_sampling, want))
    });
    if want {
        winrt::send(winrt::Job::SysSample);
    } else if was {
        winrt::send(winrt::Job::SysStop);
    }
}

unsafe fn on_tick(hwnd: HWND) {
    // Два дешёвых счётчика для предупреждения о нагрузке — вне st().
    let (cpu, mem) = (sys::sysinfo::cpu_times(), sys::sysinfo::memory());
    let now = Instant::now();
    let (beep, confirm_gone, load) = st(|s| {
        s.tasks.expire(now);
        (s.timers.tick(now), s.ctl.tick(), s.load.tick(cpu, mem))
    });
    if beep {
        let _ = MessageBeep(MB_ICONEXCLAMATION);
    }
    match load {
        LoadAction::None => {}
        LoadAction::Probe(_) => winrt::send(winrt::Job::LoadProbe),
        LoadAction::Warn(k) => winrt::send(winrt::Job::LoadWarn(k)),
    }
    sys_sampling_tick();
    shell::fullscreen_tick(hwnd);
    // Перерисовываем только если что-то видимое поменялось.
    let (changed, expanded, live) =
        st(|s| (pill::text(s) != s.shell.last_collapsed, s.shell.expanded, !s.timers.list.is_empty() || s.ctl.confirm.is_some()));
    // Пока играет музыка, раз в секунду двигаются прогресс и эквалайзер.
    if changed {
        shell::layout(hwnd);
    } else if (expanded && (live || confirm_gone)) || winrt::playing() {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

unsafe fn on_mouse_move(hwnd: HWND, lparam: LPARAM) {
    let _ = KillTimer(hwnd, TIMER_COLLAPSE);
    if st(|s| !std::mem::replace(&mut s.shell.tracking, true)) {
        let mut tme = TRACKMOUSEEVENT {
            cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
            dwFlags: TME_LEAVE,
            hwndTrack: hwnd,
            dwHoverTime: 0,
        };
        let _ = TrackMouseEvent(&mut tme);
    }
    if !st(|s| s.shell.expanded) {
        shell::set_expanded(hwnd, true);
        return;
    }
    let (x, y) = (loword(lparam), hiword(lparam));
    let changed = st(|s| {
        let h = hit_at(s, x, y).map(|h| h.1);
        std::mem::replace(&mut s.hover, h) != h
    });
    if changed {
        let _ = InvalidateRect(hwnd, None, false);
    }
}

unsafe fn on_mouse_leave(hwnd: HWND) {
    let active = st(|s| {
        s.shell.tracking = false;
        s.hover = None;
        s.input.active
    });
    // Пока печатаешь, панель не прячется от ухода курсора.
    if !active {
        SetTimer(hwnd, TIMER_COLLAPSE, COLLAPSE_DELAY_MS, None);
    }
    let _ = InvalidateRect(hwnd, None, false);
}

unsafe fn on_timer(hwnd: HWND, id: usize) {
    match id {
        TIMER_CLOCK => on_tick(hwnd),
        TIMER_COLLAPSE => shell::on_collapse_timer(hwnd),
        TIMER_PASTE => input::on_paste_timer(hwnd),
        TIMER_SHOT => {
            let _ = KillTimer(hwnd, TIMER_SHOT);
            sys::launch::open("ms-screenclip:", None);
        }
        TIMER_FLASH => {
            let _ = KillTimer(hwnd, TIMER_FLASH);
            osd::hide(hwnd);
        }
        TIMER_CTL => {
            let _ = KillTimer(hwnd, TIMER_CTL);
            ctl::refresh();
            let _ = InvalidateRect(hwnd, None, false);
        }
        _ => {}
    }
}

fn loword(l: LPARAM) -> i32 {
    (l.0 & 0xFFFF) as i16 as i32
}
fn hiword(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xFFFF) as i16 as i32
}

pub unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => ui::paint(hwnd),
        WM_ERASEBKGND => return LRESULT(1),
        // Клик по окну не забирает фокус у того, где ты печатаешь.
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => on_mouse_move(hwnd, lparam),
        WM_MOUSELEAVE => on_mouse_leave(hwnd),
        WM_LBUTTONDOWN => on_click(hwnd, loword(lparam), hiword(lparam)),
        WM_MOUSEWHEEL => on_wheel(hwnd, ((wparam.0 >> 16) & 0xFFFF) as i16 as i32),
        WM_CHAR => {
            if st(|s| s.input.active) {
                input::on_char(hwnd, wparam.0 as u32);
            }
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if !st(|s| s.input.active) {
                return DefWindowProcW(hwnd, msg, wparam, lparam);
            }
            input::on_key(hwnd, wparam.0 as u16);
        }
        WM_ACTIVATE => {
            // Ушли в другое окно — ввод выключаем, панель прячем, если курсор не над ней.
            if (wparam.0 & 0xFFFF) as u32 == WA_INACTIVE {
                input::on_inactive(hwnd);
            }
        }
        WM_HOTKEY => {
            let tab = if wparam.0 as i32 == HOTKEY_CLIP { Tab::Clipboard } else { Tab::Commands };
            input::on_hotkey(hwnd, tab);
        }
        WM_CLIPBOARDUPDATE => {
            if clip::on_update(hwnd) {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
        // Значок ♪ и точки микрофона/камеры меняют ширину пилюли.
        WM_APP_MEDIA => {
            shell::layout(hwnd);
            // Заиграло или встало на паузу: включить/выключить захват звука.
            crate::feat::viz::refresh_want();
        }
        WM_APP_PRIVACY => shell::layout(hwnd),
        WM_APP_VIZ => ui::viz::on_frame(hwnd),
        WM_APP_API => api::on_events(hwnd),
        WM_APP_SYS => {
            let warn = sysmon::LOAD_WARN.lock().unwrap().take();
            if let Some(text) = warn {
                osd::show(hwnd, osd::Flash::Text(text, true), 8000);
            } else if st(|s| s.shell.expanded && s.shell.tab == Tab::System) {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
        WM_POWERBROADCAST => {
            if wparam.0 as u32 == PBT_POWERSETTINGCHANGE {
                if let Some(ev) = sys::power::parse_notify(lparam) {
                    osd::on_power(hwnd, ev);
                }
            }
            return LRESULT(1);
        }
        WM_APP_RADIO | WM_APP_BRIGHT => {
            let _ = InvalidateRect(hwnd, None, false);
        }
        WM_APP_VOLUME => osd::on_volume(hwnd, wparam.0 as f32 / 1000.0, lparam.0 != 0),
        WM_APP_AUDIODEV => osd::on_audio_device(hwnd),
        WM_APP_BAR => shell::on_appbar(hwnd, wparam, lparam),
        WM_APP_TRAY => shell::on_tray(hwnd, lparam),
        WM_TIMER => on_timer(hwnd, wparam.0),
        WM_DPICHANGED => shell::on_dpi(hwnd, (wparam.0 & 0xFFFF) as u32),
        // Смена разрешения / подключение монитора.
        WM_DISPLAYCHANGE => shell::on_display_change(hwnd),
        WM_SETTINGCHANGE => {
            // Тема переключилась извне — обновим плитку.
            if st(|s| s.shell.expanded && s.shell.tab == Tab::Controls) {
                ctl::refresh();
                let _ = InvalidateRect(hwnd, None, false);
            }
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
        WM_DESTROY => {
            shell::appbar(hwnd, windows::Win32::UI::Shell::ABM_REMOVE);
            PostQuitMessage(0);
        }
        _ => {
            if msg != 0 && msg == st(|s| s.shell.taskbar_created) {
                shell::on_taskbar_created(hwnd);
                return LRESULT(0);
            }
            return DefWindowProcW(hwnd, msg, wparam, lparam);
        }
    }
    LRESULT(0)
}
