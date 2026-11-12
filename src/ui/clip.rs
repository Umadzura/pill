//! Вкладка «Буфер»: поиск и список истории.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::feat::clip::ClipHit;
use crate::shell::EXPANDED_H;

const TOP: i32 = 94;
const ROW_H: i32 = 28;
/// Сколько строк истории влезает (нужно и для прокрутки стрелками).
pub const ROWS: usize = ((EXPANDED_H - 14 - TOP) / ROW_H) as usize;

/// Однострочное превью: переносы и табы заменяем на пробелы.
fn preview(t: &str) -> String {
    t.trim().chars().take(300).map(|c| if c.is_whitespace() { ' ' } else { c }).collect()
}

pub unsafe fn draw(p: &mut Painter, s: &State, w: i32) {
    p.input(w, s, "Поиск в истории (клик или Ctrl+Alt+V)");
    let view = s.clip.view(&s.input.text);
    if view.is_empty() {
        let msg = if s.clip.items.is_empty() { "История пуста — скопируй что-нибудь" } else { "Ничего не найдено" };
        p.text(p.f.normal, msg, RECT { left: 0, top: p.s(100), right: w, bottom: p.s(200) }, DIM, DT_CENTER);
        return;
    }
    let scroll = s.input.scroll;
    for (n, &idx) in view.iter().enumerate().skip(scroll).take(ROWS) {
        let y = TOP + (n - scroll) as i32 * ROW_H;
        let row = RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + ROW_H) };
        let e = &s.clip.items[idx];
        let (h_row, h_pin, h_del) = (Hit::Clip(ClipHit::Row(idx)), Hit::Clip(ClipHit::Pin(idx)), Hit::Clip(ClipHit::Del(idx)));
        let hov = p.hovered(h_row) || p.hovered(h_pin) || p.hovered(h_del);
        if hov || (s.input.active && s.input.sel == n) {
            p.fill(row, 10, HOVER);
        }
        p.hit(row, h_row);
        let tr = RECT { left: row.left + p.s(12), right: row.right - p.s(64), ..row };
        p.text(p.f.normal, &preview(&e.text), tr, FG, DT_LEFT | DT_END_ELLIPSIS);

        let pin = RECT { left: row.right - p.s(60), right: row.right - p.s(32), ..row };
        let del = RECT { left: row.right - p.s(30), right: row.right - p.s(4), ..row };
        if e.pinned || hov {
            let c = if e.pinned { ACCENT } else if p.hovered(h_pin) { FG } else { DIM };
            p.text(p.f.icon, if e.pinned { I_PINNED } else { I_PIN }, pin, c, DT_CENTER);
        }
        if hov {
            p.text(p.f.icon, I_DEL, del, if p.hovered(h_del) { FG } else { DIM }, DT_CENTER);
        }
        p.hit(pin, h_pin);
        p.hit(del, h_del);
    }
    if view.len() > scroll + ROWS {
        let more = format!("ещё {} ↓", view.len() - scroll - ROWS);
        p.text(p.f.small, &more, RECT { left: 0, top: p.s(320), right: w - p.s(28), bottom: p.s(336) }, DIM, DT_RIGHT);
    }
}
