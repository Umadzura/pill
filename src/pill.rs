//! Модель свёрнутой пилюли: что в ней сейчас показывать. Рисует `ui::collapsed`.
//! Приоритет (сверху важнее): тревога таймера → временная надпись (OSD) → обычный вид.

use crate::app::State;
use crate::bg::{watch, winrt};
use crate::feat::cmd::fmt_dur;
use crate::feat::osd::{battery_short, Flash};

/// Размеры значков слева от текста (логические px).
pub const EQ_W: i32 = 18;
pub const DOT: i32 = 7;
/// Сторона мини-обложки.
pub const ART: i32 = 20;
/// Ширина маскота агента (9 пикселей по 2 px).
pub const MASCOT_W: i32 = 18;

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

/// Текст свёрнутой пилюли: время [задачи скриптов] [⏱ остаток таймера] [батарея]. Значки слева рисуются отдельно.
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
    parts.extend(s.tasks.badges());
    if let Some(left) = s.timers.nearest_left() {
        parts.push(format!("⏱ {}", fmt_dur(left)));
    }
    if let Some(b) = battery_short(s.osd.battery) {
        parts.push(b);
    }
    parts.join("   ")
}

/// Мини-обложка в свёрнутой чёлке показывается, пока трек играет и обложка есть.
pub fn mini_art() -> Option<std::sync::Arc<winrt::Art>> {
    let m = winrt::MEDIA.lock().unwrap();
    if m.has && m.playing {
        m.art.clone()
    } else {
        None
    }
}

/// Ширина слева от текста (логические px): точки микрофона/камеры и мини-обложка.
pub fn left_width(s: &State) -> i32 {
    if !plain(s) {
        return 0;
    }
    let (mic, cam) = privacy_dots();
    let dots = mic as i32 + cam as i32;
    let mut w = if dots > 0 { dots * (DOT + 4) + 4 } else { 0 };
    if mini_art().is_some() {
        w += ART + 10;
    }
    // Маскот агентов (и их число, если больше одного): 9 пикселей спрайта по 2 px.
    if let Some((_, n)) = s.agents.attention() {
        w += MASCOT_W + 8 + if n > 1 { 12 } else { 0 };
    }
    w
}

/// Ширина справа от текста: полоски визуализатора, пока играет музыка.
pub fn right_width(s: &State) -> i32 {
    if plain(s) && winrt::playing() {
        EQ_W + 10
    } else {
        0
    }
}

/// Сколько ширины значки добавляют к тексту.
pub fn lead_width(s: &State) -> i32 {
    left_width(s) + right_width(s)
}
