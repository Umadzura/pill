//! Вкладка «Монитор»: четыре крупных числа (ЦП, память, видеокарта, сеть) с линией истории за минуту,
//! под тонкой чертой процессы простым списком, внизу диск одной строкой. Без карточек: цвет
//! появляется только при перегрузке.

use std::collections::VecDeque;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::State;
use crate::feat::sysmon::{fmt_bytes, fmt_rate, Snapshot, HISTORY, SNAPSHOT};

const WARN_AT: f32 = 85.0;
/// Верх содержимого страницы (под переключателем «Управление / Монитор»).
const TOP: i32 = 54;
const LEFT: i32 = 22;
const COL_GAP: i32 = 20;
/// Строка процесса.
const ROW: i32 = 30;

fn level_color(pct: f32) -> COLORREF {
    if pct >= WARN_AT {
        WARN
    } else {
        FG
    }
}

/// История за минуту ступенчатой линией: новые значения справа.
unsafe fn spark(p: &Painter, area: RECT, hist: &VecDeque<f32>) {
    let col = ((area.right - area.left) as f32 / HISTORY as f32).max(1.0);
    let (h, thick) = ((area.bottom - area.top) as f32, p.s(2));
    for (i, v) in hist.iter().enumerate() {
        let slot = HISTORY - hist.len() + i;
        let l = area.left + (slot as f32 * col) as i32;
        let y = area.bottom - thick - ((h - thick as f32) * v.clamp(0.0, 100.0) / 100.0) as i32;
        p.fill(RECT { left: l, top: y, right: l + (col.ceil() as i32).max(1), bottom: y + thick }, 0, level_color(*v));
    }
}

/// Одна колонка: подпись, крупное число с мелкой единицей, под ним линия истории или вторая строка.
unsafe fn metric(p: &Painter, col: RECT, label: &str, value: &str, unit: &str, color: COLORREF, below: Below) {
    p.text(p.f.small, label, RECT { top: p.s(TOP), bottom: p.s(TOP + 18), ..col }, DIM, DT_LEFT);
    let vr = RECT { top: p.s(TOP + 20), bottom: p.s(TOP + 48), ..col };
    let font = if value.chars().count() > 4 { p.f.title } else { p.f.hero };
    let vw = p.measure(font, value);
    p.text(font, value, vr, color, DT_LEFT);
    p.text(p.f.small, unit, RECT { left: col.left + vw + p.s(3), top: vr.top + p.s(4), ..vr }, if color == FG { DIM } else { color }, DT_LEFT);
    let low = RECT { top: p.s(TOP + 54), bottom: p.s(TOP + 78), ..col };
    match below {
        Below::Hist(h) => spark(p, low, h),
        Below::Text(t) => p.text(p.f.small, &t, low, DIM, DT_LEFT | DT_END_ELLIPSIS),
        Below::Nothing => {}
    }
}

enum Below<'a> {
    Hist(&'a VecDeque<f32>),
    Text(String),
    Nothing,
}

/// «2,4 МБ/с» → («2,4», «МБ/с»).
fn split_unit(s: &str) -> (String, String) {
    match s.rsplit_once(' ') {
        Some((v, u)) => (v.to_string(), u.to_string()),
        None => (s.to_string(), String::new()),
    }
}

pub unsafe fn draw(p: &mut Painter, _s: &State, w: i32) {
    let snap: Option<Snapshot> = SNAPSHOT.lock().unwrap().clone();
    let Some(sn) = snap else {
        p.text(p.f.normal, "Собираю данные…", RECT { left: 0, top: p.s(90), right: w, bottom: p.s(120) }, DIM, DT_CENTER);
        p.reach(130);
        return;
    };
    let total = w - p.s(2 * LEFT);
    let cw = (total - p.s(3 * COL_GAP)) / 4;
    let col = |i: i32| {
        let l = p.s(LEFT) + i * (cw + p.s(COL_GAP));
        RECT { left: l, top: 0, right: l + cw, bottom: 0 }
    };

    metric(p, col(0), "Процессор", &format!("{:.0}", sn.cpu), "%", level_color(sn.cpu), Below::Hist(&sn.cpu_hist));
    match sn.ram {
        Some((used, total)) => {
            let pct = used as f32 * 100.0 / total.max(1) as f32;
            metric(p, col(1), "Память", &format!("{:.0}", pct), "%", level_color(pct), Below::Hist(&sn.ram_hist));
        }
        None => metric(p, col(1), "Память", "—", "", FG, Below::Nothing),
    }
    match sn.gpu {
        Some(g) => metric(p, col(2), "Видеокарта", &format!("{:.0}", g), "%", level_color(g), Below::Hist(&sn.gpu_hist)),
        None => metric(p, col(2), "Видеокарта", "—", "", FG, Below::Text("нет данных".into())),
    }
    match sn.net {
        Some((rx, tx)) => {
            let (v, u) = split_unit(&fmt_rate(rx));
            metric(p, col(3), "Сеть", &format!("↓ {}", v), &u, FG, Below::Text(format!("↑ {}", fmt_rate(tx))));
        }
        None => metric(p, col(3), "Сеть", "—", "", FG, Below::Nothing),
    }

    // Тонкая черта, под ней процессы простым списком.
    let line_y = TOP + 92;
    p.fill(RECT { left: p.s(LEFT), top: p.s(line_y), right: w - p.s(LEFT), bottom: p.s(line_y) + p.s(1).max(1) }, 0, HOVER);
    let (left, right) = (p.s(LEFT), w - p.s(LEFT));
    let mut y = line_y + 8;
    if sn.apps.is_empty() {
        p.text(p.f.small, "…", RECT { left, top: p.s(y), right, bottom: p.s(y + ROW) }, DIM, DT_LEFT);
        y += ROW;
    }
    for a in &sn.apps {
        let row = RECT { left, top: p.s(y), right, bottom: p.s(y + ROW) };
        y += ROW;
        p.text(p.f.normal, &a.name, RECT { right: row.right - p.s(140), ..row }, FG, DT_LEFT | DT_END_ELLIPSIS);
        let c = if a.cpu >= WARN_AT / 2.0 { WARN } else { FG };
        p.text(p.f.normal, &format!("{:.1}%", a.cpu).replace('.', ","), RECT { right: row.right - p.s(72), ..row }, c, DT_RIGHT);
        p.text(p.f.normal, &fmt_bytes(a.ram), row, DIM, DT_RIGHT);
    }

    // Диск одной тихой строкой внизу.
    let mut disk = String::new();
    if let Some((free, total)) = sn.disk {
        disk = format!("Диск C: свободно {} из {}", fmt_bytes(free), fmt_bytes(total));
    }
    if let Some(rate) = sn.disk_rate {
        disk += &format!("  ·  {}", fmt_rate(rate));
    }
    let dr = RECT { left, top: p.s(y + 6), right, bottom: p.s(y + 24) };
    p.text(p.f.small, &disk, dr, DIM, DT_LEFT | DT_END_ELLIPSIS);
    p.reach(y + 24);
}
