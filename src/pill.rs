//! Модель свёрнутой пилюли: что в ней сейчас показывать. Рисует `ui::collapsed`.
//! Приоритет (сверху важнее): тревога таймера → временная надпись (OSD) → обычный вид.

use crate::app::State;
use crate::bg::{watch, winrt};
use crate::feat::cmd::fmt_dur;
use crate::feat::osd::{battery_short, Flash};

/// Размеры значков слева от текста (логические px).
pub const EQ_W: i32 = 18;
pub const DOT: i32 = 7;

pub fn time_text() -> String {
    let (h, m) = crate::sys::local_hm();
    format!("{:02}:{:02}", h, m)
}

/// Обычный вид (не тревога и не временная надпись).
pub fn plain(s: &State) -> bool {
    s.timers.alarm.is_none() && s.osd.flash.is_none()
}

/// Сейчас показывается полоска уровня (громкость, яркость), а не текст.
pub fn shows_level(s: &State) -> bool {
    s.timers.alarm.is_none() && matches!(s.osd.flash, Some(Flash::Volume(..) | Flash::Brightness(_)))
}

/// (микрофон занят, камера занята)
pub fn privacy_dots() -> (bool, bool) {
    let u = watch::IN_USE.lock().unwrap();
    (!u.mic.is_empty(), !u.cam.is_empty())
}

/// Текст свёрнутой пилюли: время [⏱ остаток таймера] [батарея]. Значки слева рисуются отдельно.
pub fn text(s: &State) -> String {
    if let Some(a) = &s.timers.alarm {
        return format!("⏰  {}", a);
    }
    match &s.osd.flash {
        Some(Flash::Text(t, _)) => return t.clone(),
        // Полоска рисуется отдельно (ui::collapsed::level_bar), ширина фиксированная.
        Some(Flash::Volume(..) | Flash::Brightness(_)) => return String::new(),
        None => {}
    }
    let mut parts = vec![time_text()];
    if let Some(left) = s.timers.nearest_left() {
        parts.push(format!("⏱ {}", fmt_dur(left)));
    }
    if let Some(b) = battery_short(s.osd.battery) {
        parts.push(b);
    }
    parts.join("   ")
}

/// Ширина значков слева от текста в логических px: точки микрофона/камеры и эквалайзер.
pub fn lead_width(s: &State) -> i32 {
    if !plain(s) {
        return 0;
    }
    let (mic, cam) = privacy_dots();
    let dots = mic as i32 + cam as i32;
    let mut w = if dots > 0 { dots * (DOT + 4) + 4 } else { 0 };
    if winrt::playing() {
        w += EQ_W + 8;
    }
    w
}
