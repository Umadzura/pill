//! «Управление»: плитки-переключатели, громкость, яркость, питание с подтверждением.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::SetTimer;

use crate::app::{st, Redraw};
use crate::bg::winrt::{self, Job, Kind, BRIGHTNESS, RADIOS, VPN};
use crate::ids::{TIMER_CTL, TIMER_SHOT};
use crate::input::close_panel;
use crate::sys;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Ctl {
    Wifi,
    Bt,
    Vpn,
    Dark,
    Mute,
    Shot,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pw {
    Lock,
    Sleep,
    Restart,
    Shutdown,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum CtlHit {
    Toggle(Ctl),
    Power(Pw),
    /// Кнопка «Выключить / включить микрофон».
    MicMute,
}

/// Состояние плиток, прочитанное при раскрытии панели (быстрое; VPN читает воркер — `winrt::VPN`).
#[derive(Default)]
pub struct Snapshot {
    pub volume: Option<(f32, bool)>,
    pub dark: Option<bool>,
    /// Имя устройства вывода звука.
    pub out_dev: Option<String>,
    /// Микрофон по умолчанию выключен (None — микрофона нет).
    pub mic_muted: Option<bool>,
}

#[derive(Default)]
pub struct CtlState {
    pub snap: Snapshot,
    /// Перезагрузка/выключение ждут второго нажатия до этого момента + 4 с.
    pub confirm: Option<(Pw, Instant)>,
}

const CONFIRM: Duration = Duration::from_secs(4);

impl CtlState {
    pub fn confirming(&self, p: Pw) -> bool {
        self.confirm.map(|(c, _)| c == p).unwrap_or(false)
    }
    /// Раз в секунду: подтверждение протухло. true — надо перерисовать.
    pub fn tick(&mut self) -> bool {
        if self.confirm.map(|(_, t)| t.elapsed() > CONFIRM).unwrap_or(false) {
            self.confirm = None;
            return true;
        }
        false
    }
}

/// Перечитать плитки. Радио, VPN и яркость читает воркер и сам пришлёт WM_APP_RADIO / WM_APP_BRIGHT.
pub fn refresh() {
    let snap = Snapshot { volume: sys::audio::volume(), dark: sys::theme::dark(), out_dev: sys::audio::output_name(), mic_muted: sys::audio::mic_muted() };
    st(|s| s.ctl.snap = snap);
    winrt::send(Job::Radios);
    winrt::send(Job::Vpn);
    winrt::send(Job::Brightness);
}

/// После смены громкости перечитываем только её: имя устройства и микрофон от неё не зависят,
/// а колесо зовёт это на каждый щелчок.
fn refresh_volume() {
    let v = sys::audio::volume();
    st(|s| s.ctl.snap.volume = v);
}

/// Поставить яркость: сразу показываем новое значение, а медленный DDC/WMI делает воркер.
fn set_brightness(pct: i32) -> Redraw {
    let pct = pct.clamp(0, 100) as u8;
    *BRIGHTNESS.lock().unwrap() = Some(pct);
    winrt::send(Job::SetBrightness(pct));
    Redraw::Content
}

/// Клик по полоске яркости.
pub fn brightness_at(rect: RECT, x: i32) -> Redraw {
    let w = (rect.right - rect.left).max(1);
    set_brightness(((x - rect.left) * 100 + w / 2) / w)
}

/// Колесо над полоской яркости: ±5% за щелчок.
pub fn brightness_wheel(notches: i32) -> Redraw {
    let Some(cur) = *BRIGHTNESS.lock().unwrap() else { return Redraw::None };
    set_brightness(cur as i32 + notches * 5)
}

/// Клик по полоске громкости (общая для «Музыки» и «Управления»).
pub fn volume_at(rect: RECT, x: i32) -> Redraw {
    let w = (rect.right - rect.left).max(1);
    sys::audio::set_volume((x - rect.left) as f32 / w as f32);
    refresh_volume();
    Redraw::Content
}

/// Колесо мыши: ±2% за щелчок.
pub fn volume_wheel(notches: i32) -> Redraw {
    if let Some((v, _)) = sys::audio::volume() {
        sys::audio::set_volume(v + notches as f32 * 0.02);
    }
    refresh_volume();
    Redraw::Content
}

pub unsafe fn on_hit(hwnd: HWND, h: CtlHit) -> Redraw {
    match h {
        CtlHit::Toggle(c) => toggle(hwnd, c),
        CtlHit::Power(p) => power(hwnd, p),
        CtlHit::MicMute => {
            sys::audio::toggle_mic();
            refresh();
            Redraw::Content
        }
    }
}

unsafe fn toggle(hwnd: HWND, c: Ctl) -> Redraw {
    match c {
        Ctl::Wifi | Ctl::Bt => {
            let r = *RADIOS.lock().unwrap();
            let (kind, cur) = if c == Ctl::Wifi { (Kind::Wifi, r.wifi) } else { (Kind::Bt, r.bt) };
            if let Some(on) = cur {
                winrt::send(Job::SetRadio(kind, !on));
            } else {
                sys::launch::open(if c == Ctl::Wifi { "ms-settings:network-wifi" } else { "ms-settings:bluetooth" }, None);
            }
        }
        Ctl::Vpn => {
            if VPN.lock().unwrap().is_some() {
                winrt::send(Job::ToggleVpn);
                // rasdial работает несколько секунд — перечитаем статус позже.
                SetTimer(hwnd, TIMER_CTL, 3000, None);
            } else {
                sys::launch::open("ms-settings:network-vpn", None);
            }
        }
        Ctl::Dark => {
            let dark = st(|s| s.ctl.snap.dark.unwrap_or(false));
            sys::theme::set_dark(!dark);
            winrt::send(Job::ThemeBroadcast);
        }
        Ctl::Mute => sys::audio::toggle_mute(),
        Ctl::Shot => {
            // Сначала сворачиваемся, чтобы панель не попала в кадр, потом открываем выделение области.
            close_panel(hwnd, true);
            SetTimer(hwnd, TIMER_SHOT, 250, None);
            return Redraw::None;
        }
    }
    refresh();
    Redraw::Content
}

unsafe fn power(hwnd: HWND, p: Pw) -> Redraw {
    let needs_confirm = matches!(p, Pw::Restart | Pw::Shutdown);
    let confirmed = st(|s| s.ctl.confirm.map(|(c, t)| c == p && t.elapsed() < CONFIRM).unwrap_or(false));
    if needs_confirm && !confirmed {
        st(|s| s.ctl.confirm = Some((p, Instant::now())));
        return Redraw::Content;
    }
    close_panel(hwnd, false);
    match p {
        Pw::Lock => sys::power::lock(),
        Pw::Sleep => sys::power::sleep(),
        Pw::Restart => sys::power::restart(),
        Pw::Shutdown => sys::power::shutdown(),
    }
    Redraw::None
}
