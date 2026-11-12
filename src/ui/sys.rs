//! Вкладка «Система»: ЦП, память, видеокарта, сеть, история ЦП за минуту, диск, топ приложений.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::State;
use crate::feat::sysmon::{fmt_bytes, fmt_rate, Snapshot, HISTORY, SNAPSHOT};

const WARN_AT: f32 = 85.0;

fn level_color(pct: f32) -> COLORREF {
    if pct >= WARN_AT {
        WARN
    } else {
        ACCENT
    }
}

/// Плитка: подпись, крупное значение, строка под ним и полоска заполнения.
unsafe fn tile(p: &Painter, r: RECT, label: &str, value: &str, sub: &str, pct: Option<f32>) {
    p.fill(r, 14, PANEL);
    let x = r.left + p.s(12);
    p.text(p.f.small, label, RECT { left: x, top: r.top + p.s(6), right: r.right - p.s(8), bottom: r.top + p.s(22) }, DIM, DT_LEFT);
    p.text(p.f.title, value, RECT { left: x, top: r.top + p.s(22), right: r.right - p.s(8), bottom: r.top + p.s(46) }, FG, DT_LEFT);
    p.text(p.f.small, sub, RECT { left: x, top: r.top + p.s(46), right: r.right - p.s(8), bottom: r.top + p.s(62) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    if let Some(v) = pct {
        let bar = RECT { left: x, top: r.bottom - p.s(9), right: r.right - p.s(12), bottom: r.bottom - p.s(5) };
        p.fill(bar, 4, TRACK);
        let fill = RECT { right: bar.left + ((bar.right - bar.left) as f32 * v.clamp(0.0, 100.0) / 100.0) as i32, ..bar };
        if fill.right - fill.left >= p.s(3) {
            p.fill(fill, 4, level_color(v));
        }
    }
}

pub unsafe fn draw(p: &mut Painter, _s: &State, w: i32) {
    let snap: Option<Snapshot> = SNAPSHOT.lock().unwrap().clone();
    let Some(sn) = snap else {
        p.text(p.f.normal, "Собираю данные…", RECT { left: 0, top: p.s(120), right: w, bottom: p.s(160) }, DIM, DT_CENTER);
        return;
    };
    let gap = p.s(8);
    let left = p.s(20);
    let tw = (w - 2 * left - 3 * gap) / 4;
    let tile_r = |i: i32| RECT { left: left + i * (tw + gap), top: p.s(56), right: left + i * (tw + gap) + tw, bottom: p.s(128) };

    tile(p, tile_r(0), "Процессор", &format!("{:.0}%", sn.cpu), &format!("{} потоков", sn.ncpu), Some(sn.cpu));
    match sn.ram {
        Some((used, total)) => {
            let pct = used as f32 * 100.0 / total.max(1) as f32;
            tile(p, tile_r(1), "Память", &format!("{:.0}%", pct), &format!("{} из {}", fmt_bytes(used), fmt_bytes(total)), Some(pct));
        }
        None => tile(p, tile_r(1), "Память", "—", "", None),
    }
    match sn.gpu {
        Some(g) => tile(p, tile_r(2), "Видеокарта", &format!("{:.0}%", g), "самый занятый движок", Some(g)),
        None => tile(p, tile_r(2), "Видеокарта", "—", "нет данных", None),
    }
    match sn.net {
        Some((rx, tx)) => tile(p, tile_r(3), "Сеть", &format!("↓ {}", fmt_rate(rx)), &format!("↑ {}", fmt_rate(tx)), None),
        None => tile(p, tile_r(3), "Сеть", "—", "", None),
    }

    // История ЦП за минуту: столбики справа налево, новые справа.
    let graph = RECT { left, top: p.s(140), right: w - left, bottom: p.s(184) };
    p.fill(graph, 10, PANEL);
    let inner = RECT { left: graph.left + p.s(8), top: graph.top + p.s(6), right: graph.right - p.s(8), bottom: graph.bottom - p.s(6) };
    let col = ((inner.right - inner.left) as f32 / HISTORY as f32).max(1.0);
    let n = sn.cpu_hist.len();
    for (i, v) in sn.cpu_hist.iter().enumerate() {
        let slot = HISTORY - n + i;
        let l = inner.left + (slot as f32 * col) as i32;
        let h = ((inner.bottom - inner.top) as f32 * v / 100.0) as i32;
        if h > 0 {
            let r = RECT { left: l, top: inner.bottom - h, right: l + (col as i32 - 1).max(1), bottom: inner.bottom };
            p.fill(r, 0, level_color(*v));
        }
    }
    p.text(p.f.small, "ЦП за минуту", RECT { left: inner.left + p.s(2), top: inner.top, right: inner.right, bottom: inner.top + p.s(14) }, DIM, DT_LEFT);

    // Диск.
    let mut disk = String::new();
    if let Some((free, total)) = sn.disk {
        disk = format!("Системный диск: свободно {} из {}", fmt_bytes(free), fmt_bytes(total));
    }
    if let Some(rate) = sn.disk_rate {
        disk += &format!("   ·   активность {}", fmt_rate(rate));
    }
    p.text(p.f.small, &disk, RECT { left, top: p.s(190), right: w - left, bottom: p.s(208) }, DIM, DT_LEFT | DT_END_ELLIPSIS);

    // Топ приложений.
    p.text(p.f.small, "Больше всего грузят", RECT { left, top: p.s(214), right: w - left, bottom: p.s(232) }, DIM, DT_LEFT);
    p.text(p.f.small, "ЦП", RECT { left, top: p.s(214), right: w - left - p.s(90), bottom: p.s(232) }, DIM, DT_RIGHT);
    p.text(p.f.small, "Память", RECT { left, top: p.s(214), right: w - left, bottom: p.s(232) }, DIM, DT_RIGHT);
    if sn.apps.is_empty() {
        p.text(p.f.small, "…", RECT { left, top: p.s(234), right: w - left, bottom: p.s(256) }, DIM, DT_LEFT);
    }
    for (i, a) in sn.apps.iter().enumerate() {
        let y = 234 + i as i32 * 24;
        let row = RECT { left, top: p.s(y), right: w - left, bottom: p.s(y + 24) };
        if i % 2 == 0 {
            p.fill(row, 8, PANEL);
        }
        let name = RECT { left: row.left + p.s(10), right: row.right - p.s(170), ..row };
        p.text(p.f.normal, &a.name, name, FG, DT_LEFT | DT_END_ELLIPSIS);
        let c = if a.cpu >= WARN_AT / 2.0 { WARN } else { FG };
        p.text(p.f.normal, &format!("{:.1}%", a.cpu).replace('.', ","), RECT { right: row.right - p.s(90), ..row }, c, DT_RIGHT);
        p.text(p.f.normal, &fmt_bytes(a.ram), RECT { right: row.right - p.s(10), ..row }, DIM, DT_RIGHT);
    }
}
