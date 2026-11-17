//! Свёрнутая пилюля: значки, текст, полоска прогресса трека, полоска громкости.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::mascot;
use super::paint::*;
use crate::app::State;
use crate::bg::winrt;
use crate::feat::osd::Flash;
use crate::pill::{self, Lead, Tone, ART, DOT, EQ_W};

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
    let v = pill::view(s);
    let color = match v.tone {
        Tone::Plain => FG,
        Tone::Accent => ACCENT,
        Tone::Warn => WARN,
    };
    let text = &v.text;
    let (left, right) = (p.s(v.left_width()), p.s(v.right_width()));
    if left + right == 0 {
        let tw = p.measure(p.f.bold, text);
        main_text(p, &v, RECT { left: (rc.right - tw) / 2, ..rc }, color);
    } else {
        // Как в чёлке мака: слева значки, по центру текст, справа полоски. Всё одним блоком по центру.
        let tw = p.measure(p.f.bold, text);
        let x = (rc.right - (left + tw + right)) / 2;
        let mid = rc.bottom / 2;
        draw_left(p, s, &v.lead, x, mid);
        main_text(p, &v, RECT { left: x + left, ..rc }, color);
        let mut rx = x + left + tw;
        if v.unread > 0 {
            // Синий счётчик пропущенных уведомлений.
            rx += p.s(pill::UNREAD_GAP);
            let (w, h) = (p.s(pill::UNREAD_W), p.s(16));
            let r = RECT { left: rx, top: mid - h / 2, right: rx + w, bottom: mid - h / 2 + h };
            p.fill(r, R_PILL, ACCENT);
            let n = if v.unread > 9 { "9+".to_string() } else { v.unread.to_string() };
            p.text(p.f.small, &n, r, INK, DT_CENTER);
            rx += w;
        }
        if v.viz {
            // Настоящий спектр того, что играет (feat::viz), 4 столбика.
            let vx = rx + p.s(10);
            let colors = pill::mini_art().map(|a| super::viz::tint(&a)).unwrap_or((super::viz::PLAIN, super::viz::PLAIN));
            p.viz(RECT { left: vx, top: mid - p.s(7), right: vx + p.s(EQ_W), bottom: mid + p.s(7) }, 4, colors);
        }
    }
    if v.plain {
        progress(p, rc, v.task_bar);
    }
}

/// Текст центра от левого края `r`: основная часть цветом `color`, заряд батареи серым.
unsafe fn main_text(p: &Painter, v: &pill::View, r: RECT, color: COLORREF) {
    match v.dim_from {
        Some(i) => {
            let (head, tail) = v.text.split_at(i);
            p.text(p.f.bold, head, r, color, DT_LEFT);
            p.text(p.f.bold, tail, RECT { left: r.left + p.measure(p.f.bold, head), ..r }, DIM, DT_LEFT);
        }
        None => p.text(p.f.bold, &v.text, r, color, DT_LEFT),
    }
}

/// Значки левого слота в том порядке, в каком их собрал `pill::assemble`.
unsafe fn draw_left(p: &mut Painter, s: &State, lead: &[Lead], mut x: i32, mid: i32) {
    let mut after_dots = false;
    for l in lead {
        // Между точками микрофона/камеры и следующим значком зазор.
        let dot = matches!(l, Lead::Mic | Lead::Cam);
        if after_dots && !dot {
            x += p.s(pill::DOT_GAP);
        }
        after_dots = dot;
        match *l {
            Lead::Mic | Lead::Cam => {
                let d = p.s(DOT);
                let c = if *l == Lead::Mic { MIC } else { CAM };
                p.fill(RECT { left: x, top: mid - d / 2, right: x + d, bottom: mid - d / 2 + d }, DOT, c);
            }
            Lead::Art => {
                if let Some(a) = pill::mini_art() {
                    let side = p.s(ART);
                    p.art(RECT { left: x, top: mid - side / 2, right: x + side, bottom: mid - side / 2 + side }, &a, 10);
                }
            }
            // Маскот главного агента (кому нужно разрешение, иначе работающий), число — если их больше одного.
            // Стопка маскотов: главный (кому нужно разрешение, иначе работающий) впереди и ярко,
            // до двух следующих за ним тусклее; если агентов больше трёх — «+N».
            // За стопкой — «детки»: работающие субагенты главного маленькими маскотами по низу.
            Lead::Mascot { count, kids } => {
                let px = p.s(2);
                let y = mid - px * mascot::H / 2;
                let stack = s.agents.stack();
                let shown = stack.len().min(3);
                for i in (0..shown).rev() {
                    let k = [1.0, 0.45, 0.22][i];
                    mascot::draw_faded(p, x + p.s(pill::STACK_DX) * i as i32, y, px, stack[i], s.notif_flip, k);
                }
                let mut tx = x + px * mascot::W + p.s(pill::STACK_DX) * (shown.max(1) as i32 - 1) + p.s(4);
                if let (Some(front), true) = (stack.first(), kids > 0) {
                    tx = mascot::kids(p, tx, y + px * mascot::H, p.s(1).max(1), front, s.notif_flip, pill::KIDS_MAX);
                    if kids > pill::KIDS_MAX {
                        let r = RECT { left: tx, top: 0, right: tx + p.s(14), bottom: mid * 2 };
                        p.text(p.f.small, &format!("+{}", kids - pill::KIDS_MAX), r, DIM, DT_LEFT);
                        tx += p.s(14);
                    }
                }
                if count > 3 {
                    let r = RECT { left: tx, top: 0, right: tx + p.s(18), bottom: mid * 2 };
                    p.text(p.f.small, &format!("+{}", count - 3), r, DIM, DT_LEFT);
                }
            }
        }
        x += p.s(l.width());
    }
}

/// Тонкая полоска по низу: прогресс задачи скрипта, а если её нет — трека (играет и плеер сообщает время).
unsafe fn progress(p: &Painter, rc: RECT, task: Option<u8>) {
    let (cur, dur) = match task {
        Some(pct) => (pct as f64, 100.0),
        None => {
            let m = winrt::MEDIA.lock().unwrap();
            if !m.playing {
                return;
            }
            let Some(t) = m.progress() else { return };
            t
        }
    };
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
