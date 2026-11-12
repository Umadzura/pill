//! Временные надписи в свёрнутой пилюле (OSD): громкость, яркость, зарядка, предупреждения о батарее.
//! Новое событие заменяет старое, одноразовый TIMER_FLASH убирает надпись.
//! Батарея и яркость приходят событиями WM_POWERBROADCAST, без опроса.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::System::Diagnostics::Debug::MessageBeep;
use windows::Win32::UI::WindowsAndMessaging::{SetTimer, MB_ICONEXCLAMATION};

use crate::app::st;
use crate::ids::TIMER_FLASH;
use crate::shell::layout;
use crate::sys;
use crate::sys::power::{Battery, PowerEvent};

const FLASH_LEVEL_MS: u32 = 1500;

pub enum Flash {
    /// Громкость: уровень 0..1, без звука.
    Volume(f32, bool),
    /// Яркость экрана: уровень 0..1.
    Brightness(f32),
    /// Текст; true — предупреждение (другой цвет).
    Text(String, bool),
}

#[derive(Default)]
pub struct Osd {
    pub flash: Option<Flash>,
    /// Подписка на громкость; живёт до выхода из COM.
    pub audio: Option<sys::audio::Watch>,
    /// Последняя известная громкость: флешим только при реальном изменении.
    last_vol: Option<(f32, bool)>,
    /// Последнее состояние батареи (из события). None — батареи нет.
    pub battery: Option<Battery>,
    /// Было ли питание от сети в прошлом событии (None — ещё не знаем или батареи нет).
    last_ac: Option<bool>,
    /// Последняя яркость из события: первое событие после подписки только запоминаем.
    last_bright: Option<u8>,
    /// Какое предупреждение о заряде уже показали: 0, 20 или 10.
    batt_warned: u8,
}

impl Osd {
    pub fn new() -> Self {
        Osd { last_vol: sys::audio::volume(), battery: sys::power::battery(), ..Default::default() }
    }
}

/// Показать временную надпись на ms миллисекунд (новая заменяет старую).
pub unsafe fn show(hwnd: HWND, flash: Flash, ms: u32) {
    st(|s| s.osd.flash = Some(flash));
    SetTimer(hwnd, TIMER_FLASH, ms, None);
    layout(hwnd);
}

pub unsafe fn hide(hwnd: HWND) {
    st(|s| s.osd.flash = None);
    layout(hwnd);
}

/// Событие громкости от системы (колбэк в sys::audio).
pub unsafe fn on_volume(hwnd: HWND, level: f32, muted: bool) {
    let (changed, expanded) = st(|s| {
        let prev = s.osd.last_vol.replace((level, muted));
        // Драйверы иногда шлют одно и то же по несколько раз.
        let changed = prev.map(|(l, m)| (l - level).abs() > 0.004 || m != muted).unwrap_or(true);
        if s.shell.expanded {
            s.ctl.snap.volume = Some((level, muted));
        }
        (changed, s.shell.expanded)
    });
    if expanded {
        // В раскрытой панели громкость и так видна.
        let _ = InvalidateRect(hwnd, None, false);
    } else if changed {
        show(hwnd, Flash::Volume(level, muted), FLASH_LEVEL_MS);
    }
}

/// Переключились на другое устройство вывода: переподписываемся молча, без вспышки.
pub unsafe fn on_audio_device(hwnd: HWND) {
    let mut w = st(|s| s.osd.audio.take());
    if let Some(w) = w.as_mut() {
        w.rebind();
    }
    let vol = sys::audio::volume();
    st(|s| {
        s.osd.audio = w;
        s.osd.last_vol = vol;
        s.ctl.snap.volume = vol;
    });
    let _ = InvalidateRect(hwnd, None, false);
}

/// Что сказать про батарею на этом тике: (текст, предупреждение).
/// Предупреждаем один раз на 20% и один раз на 10%; сброс после зарядки или выше 25%.
fn battery_event(prev_ac: Option<bool>, warned: &mut u8, pct: u8, ac: bool) -> Option<(String, bool)> {
    if ac || pct > 25 {
        *warned = 0;
    }
    if !ac && pct <= 10 && *warned != 10 {
        *warned = 10;
        return Some((format!("Батарея {}% — подключи зарядку", pct), true));
    }
    if !ac && pct <= 20 && *warned == 0 {
        *warned = 20;
        return Some((format!("Батарея {}% — пора на зарядку", pct), true));
    }
    match prev_ac {
        Some(false) if ac => Some((format!("⚡ Заряжается · {}%", pct), false)),
        Some(true) if !ac => Some((format!("От батареи · {}%", pct), false)),
        _ => None,
    }
}

/// WM_POWERBROADCAST / PBT_POWERSETTINGCHANGE.
pub unsafe fn on_power(hwnd: HWND, ev: PowerEvent) {
    match ev {
        PowerEvent::Battery => on_battery(hwnd),
        PowerEvent::Brightness(v) => on_brightness(hwnd, v),
    }
}

/// Зарядку подключили/отключили или сменился процент. На ПК без батареи не приходит.
unsafe fn on_battery(hwnd: HWND) {
    let b = sys::power::battery();
    let ev = st(|s| {
        s.osd.battery = b;
        let b = b?;
        let prev = s.osd.last_ac.replace(b.ac);
        battery_event(prev, &mut s.osd.batt_warned, b.percent, b.ac)
    });
    match ev {
        Some((text, warn)) => {
            if warn {
                let _ = MessageBeep(MB_ICONEXCLAMATION);
            }
            show(hwnd, Flash::Text(text, warn), if warn { 8000 } else { 3000 });
        }
        // Процент в пилюле мог поменяться.
        None => layout(hwnd),
    }
}

/// Яркость встроенного экрана поменялась (клавиши, ползунок Windows, датчик, мы сами).
unsafe fn on_brightness(hwnd: HWND, v: u8) {
    *crate::bg::winrt::BRIGHTNESS.lock().unwrap() = Some(v);
    let (changed, expanded) = st(|s| {
        let prev = s.osd.last_bright.replace(v);
        (prev.is_some() && prev != Some(v), s.shell.expanded)
    });
    if expanded {
        let _ = InvalidateRect(hwnd, None, false);
    } else if changed {
        show(hwnd, Flash::Brightness(v as f32 / 100.0), FLASH_LEVEL_MS);
    }
}

/// «⚡64%» для свёрнутой пилюли. None, если батареи нет.
pub fn battery_short(b: Option<Battery>) -> Option<String> {
    let b = b?;
    Some(format!("{}{}%", if b.ac { "⚡" } else { "" }, b.percent))
}

/// Строка про питание во вкладке «Управление».
pub fn battery_long() -> String {
    match sys::power::battery() {
        Some(b) if b.ac => format!("Батарея {}%, заряжается", b.percent),
        Some(b) => {
            let mut s = format!("Батарея {}%", b.percent);
            if let Some(secs) = b.left_secs {
                let m = secs / 60;
                s += &format!(", осталось ≈ {} ч {} мин", m / 60, m % 60);
            }
            s
        }
        None => "Питание от сети".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_events() {
        let mut w = 0;
        // Подключили и отключили зарядку.
        assert_eq!(battery_event(Some(false), &mut w, 64, true).unwrap(), ("⚡ Заряжается · 64%".into(), false));
        assert_eq!(battery_event(Some(true), &mut w, 64, false).unwrap(), ("От батареи · 64%".into(), false));
        assert!(battery_event(Some(false), &mut w, 63, false).is_none());
        // 20% — одно предупреждение, дрожание 21/20 не повторяет его.
        assert!(battery_event(Some(false), &mut w, 21, false).is_none());
        assert!(battery_event(Some(false), &mut w, 20, false).unwrap().1);
        assert!(battery_event(Some(false), &mut w, 21, false).is_none());
        assert!(battery_event(Some(false), &mut w, 20, false).is_none());
        // 10% — второе предупреждение, тоже один раз.
        assert!(battery_event(Some(false), &mut w, 10, false).unwrap().1);
        assert!(battery_event(Some(false), &mut w, 9, false).is_none());
        // Зарядили — счёт сбрасывается, следующая разрядка снова предупредит.
        assert!(!battery_event(Some(false), &mut w, 9, true).unwrap().1);
        assert_eq!(w, 0);
        // Выдернули зарядку на 15%: сразу предупреждение, а не «от батареи».
        assert!(battery_event(Some(true), &mut w, 15, false).unwrap().1);
        // Запуск уже на 8% от батареи: предупреждаем сразу.
        let mut w = 0;
        assert!(battery_event(None, &mut w, 8, false).unwrap().0.contains("8%"));
        // Первый тик на зарядке: молчим.
        let mut w = 0;
        assert!(battery_event(None, &mut w, 50, true).is_none());
    }
}
