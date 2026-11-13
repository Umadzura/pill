//! Вкладка «Монитор»: слева карточки 2×2 (ЦП, память, видеокарта, сеть) с историей за минуту и диск,
//! справа топ приложений.

use std::collections::VecDeque;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::State;
use crate::feat::sysmon::{fmt_bytes, fmt_rate, Snapshot, HISTORY, SNAPSHOT};

const WARN_AT: f32 = 85.0;
/// Левая колонка: две карточки в ряд.
const CARD_W: i32 = 144;
const CARD_H: i32 = 116;
const TOP: i32 = 56;
/// Где начинается правая колонка.
const RIGHT_X: i32 = 20 + 2 * CARD_W + 8 + 14;

fn level_color(pct: f32) -> COLORREF {
    if pct >= WARN_AT {
        WARN
    } else {
        ACCENT
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

/// Карточка: подпись, крупное значение, строка под ним и история (если есть).
unsafe fn card(p: &Painter, i: i32, label: &str, value: &str, sub: &str, pct: Option<f32>, hist: Option<&VecDeque<f32>>) {
    let x = 20 + (i % 2) * (CARD_W + 8);
    let y = TOP + (i / 2) * (CARD_H + 8);
    let r = p.r(x, y, x + CARD_W, y + CARD_H);
    p.fill(r, R_TILE, PANEL);
    let (l, rt) = (r.left + p.s(12), r.right - p.s(10));
    p.text(p.f.small, label, RECT { left: l, top: r.top + p.s(6), right: rt, bottom: r.top + p.s(22) }, DIM, DT_LEFT);
    let c = if pct.is_some_and(|v| v >= WARN_AT) { WARN } else { FG };
    p.text(p.f.title, value, RECT { left: l, top: r.top + p.s(22), right: rt, bottom: r.top + p.s(46) }, c, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.small, sub, RECT { left: l, top: r.top + p.s(46), right: rt, bottom: r.top + p.s(62) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    if let Some(h) = hist {
        spark(p, RECT { left: l, top: r.top + p.s(66), right: rt, bottom: r.bottom - p.s(8) }, h);
    }
}

pub unsafe fn draw(p: &mut Painter, _s: &State, w: i32) {
    let snap: Option<Snapshot> = SNAPSHOT.lock().unwrap().clone();
    let Some(sn) = snap else {
        p.text(p.f.normal, "Собираю данные…", RECT { left: 0, top: p.s(120), right: w, bottom: p.s(160) }, DIM, DT_CENTER);
        return;
    };

    card(p, 0, "Процессор", &format!("{:.0}%", sn.cpu), &format!("{} потоков", sn.ncpu), Some(sn.cpu), Some(&sn.cpu_hist));
    match sn.ram {
        Some((used, total)) => {
            let pct = used as f32 * 100.0 / total.max(1) as f32;
            card(p, 1, "Память", &format!("{:.0}%", pct), &format!("{} из {}", fmt_bytes(used), fmt_bytes(total)), Some(pct), Some(&sn.ram_hist));
        }
        None => card(p, 1, "Память", "—", "", None, None),
    }
    match sn.gpu {
        Some(g) => card(p, 2, "Видеокарта", &format!("{:.0}%", g), "самый занятый движок", Some(g), Some(&sn.gpu_hist)),
        None => card(p, 2, "Видеокарта", "—", "нет данных", None, None),
    }
    match sn.net {
        Some((rx, tx)) => card(p, 3, "Сеть", &format!("↓ {}", fmt_rate(rx)), &format!("↑ {}", fmt_rate(tx)), None, None),
        None => card(p, 3, "Сеть", "—", "", None, None),
    }

    // Диск: одна строка под карточками.
    let mut disk = String::new();
    if let Some((free, total)) = sn.disk {
        disk = format!("Диск: свободно {} из {}", fmt_bytes(free), fmt_bytes(total));
    }
    if let Some(rate) = sn.disk_rate {
        disk += &format!("  ·  {}", fmt_rate(rate));
    }
    let dy = TOP + 2 * (CARD_H + 8);
    let dr = p.r(20, dy, 20 + 2 * CARD_W + 8, dy + 26);
    p.fill(dr, R_ROW, PANEL);
    p.text(p.f.small, &disk, RECT { left: dr.left + p.s(12), right: dr.right - p.s(10), ..dr }, DIM, DT_LEFT | DT_END_ELLIPSIS);

    // Топ приложений справа.
    let (left, right) = (p.s(RIGHT_X), w - p.s(20));
    let head = RECT { left, top: p.s(TOP), right, bottom: p.s(TOP + 18) };
    p.text(p.f.small, "Больше всего грузят", head, DIM, DT_LEFT);
    p.text(p.f.small, "ЦП · память", head, DIM, DT_RIGHT);
    if sn.apps.is_empty() {
        p.text(p.f.small, "…", RECT { top: p.s(TOP + 22), bottom: p.s(TOP + 46), ..head }, DIM, DT_LEFT);
    }
    for (i, a) in sn.apps.iter().enumerate() {
        let y = TOP + 22 + i as i32 * 30;
        let row = RECT { left, top: p.s(y), right, bottom: p.s(y + 28) };
        if i % 2 == 0 {
            p.fill(row, R_ROW, PANEL);
        }
        let name = RECT { left: row.left + p.s(10), right: row.right - p.s(124), ..row };
        p.text(p.f.normal, &a.name, name, FG, DT_LEFT | DT_END_ELLIPSIS);
        let c = if a.cpu >= WARN_AT / 2.0 { WARN } else { FG };
        p.text(p.f.normal, &format!("{:.1}%", a.cpu).replace('.', ","), RECT { right: row.right - p.s(70), ..row }, c, DT_RIGHT);
        p.text(p.f.normal, &fmt_bytes(a.ram), RECT { right: row.right - p.s(8), ..row }, DIM, DT_RIGHT);
    }
}
