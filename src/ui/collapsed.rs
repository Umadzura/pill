//! Свёрнутая пилюля: значки, текст, полоска прогресса трека, полоска громкости.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::State;
use crate::bg::winrt;
use crate::feat::osd::Flash;
use crate::pill::{self, DOT, EQ_W};

pub unsafe fn draw(p: &mut Painter, s: &State, rc: RECT) {
    if pill::shows_level(s) {
        match &s.osd.flash {
            Some(Flash::Volume(level, muted)) => {
                let icon = if *muted { I_MUTE } else { I_VOL_LEVELS[((level * 3.0).ceil() as usize).min(3)] };
                let pct = if *muted { "выкл".to_string() } else { format!("{}%", (level * 100.0).round() as i32) };
                level_bar(p, rc, icon, &pct, *level, *muted);
            }
            Some(Flash::Brightness(level)) => {
                level_bar(p, rc, I_BRIGHT, &format!("{}%", (level * 100.0).round() as i32), *level, false);
            }
            _ => {}
        }
        return;
    }
    let color = match (&s.timers.alarm, &s.osd.flash) {
        (Some(_), _) => ACCENT,
        (None, Some(Flash::Text(_, true))) => WARN,
        (None, Some(Flash::Text(_, false))) => ACCENT,
        _ => FG,
    };
    let text = pill::text(s);
    let lead = p.s(pill::lead_width(s));
    if lead == 0 {
        p.text(p.f.bold, &text, rc, color, DT_CENTER);
    } else {
        // Значки + текст одним блоком по центру.
        let total = lead + p.measure(p.f.bold, &text);
        let x = (rc.right - total) / 2;
        draw_lead(p, x, rc.bottom / 2);
        p.text(p.f.bold, &text, RECT { left: x + lead, ..rc }, color, DT_LEFT);
    }
    if pill::plain(s) {
        progress(p, rc);
    }
}

/// Значки слева от текста: точки микрофона/камеры и полоски музыки.
unsafe fn draw_lead(p: &mut Painter, mut x: i32, mid: i32) {
    let (mic, cam) = pill::privacy_dots();
    for (on, c) in [(mic, MIC), (cam, CAM)] {
        if on {
            let d = p.s(DOT);
            p.fill(RECT { left: x, top: mid - d / 2, right: x + d, bottom: mid - d / 2 + d }, DOT, c);
            x += p.s(DOT + 4);
        }
    }
    if mic || cam {
        x += p.s(4);
    }
    if winrt::playing() {
        // Настоящий спектр того, что играет (feat::viz), 4 столбика.
        let r = RECT { left: x, top: mid - p.s(7), right: x + p.s(EQ_W), bottom: mid + p.s(7) };
        p.viz(r, 4);
    }
}

/// Тонкая полоска прогресса трека по низу (если играет и плеер сообщает время).
unsafe fn progress(p: &Painter, rc: RECT) {
    let m = winrt::MEDIA.lock().unwrap();
    if !m.playing {
        return;
    }
    let Some((cur, dur)) = m.progress() else { return };
    drop(m);
    // Отступ на полвысоты: там уже начинается скругление.
    let inset = rc.bottom / 2;
    let bar = RECT { left: inset, top: rc.bottom - p.s(4), right: rc.right - inset, bottom: rc.bottom - p.s(2) };
    p.fill(bar, 0, TRACK);
    let fill = RECT { right: bar.left + ((bar.right - bar.left) as f64 * cur / dur) as i32, ..bar };
    p.fill(fill, 0, ACCENT);
}

/// Уровень (громкость, яркость): значок, полоска, подпись.
unsafe fn level_bar(p: &mut Painter, rc: RECT, icon: &str, label: &str, level: f32, muted: bool) {
    let (l, r) = (rc.left + p.s(16), rc.right - p.s(16));
    p.text(p.f.icon, icon, RECT { left: l, right: l + p.s(22), ..rc }, FG, DT_LEFT);
    p.text(p.f.bold, label, RECT { left: r - p.s(44), right: r, ..rc }, FG, DT_RIGHT);
    let mid = (rc.top + rc.bottom) / 2;
    let bar = RECT { left: l + p.s(30), top: mid - p.s(3), right: r - p.s(52), bottom: mid + p.s(3) };
    p.fill(bar, 6, TRACK);
    let mut filled = bar;
    filled.right = bar.left + ((bar.right - bar.left) as f32 * level.clamp(0.0, 1.0)) as i32;
    if filled.right - filled.left >= p.s(4) {
        p.fill(filled, 6, if muted { DIM } else { FG });
    }
}
