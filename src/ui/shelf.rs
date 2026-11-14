//! Вкладка «Полка»: файлы миниатюрами по четыре в ряд, ниже последние скопированные тексты.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{DT_CENTER, DT_END_ELLIPSIS, DT_LEFT, DT_RIGHT};

use super::paint::*;
use crate::app::{Hit, State};

const TOP: i32 = 60;
const LEFT: i32 = 24;
const GAP: i32 = 14;
const COLS: i32 = 4;
const THUMB_H: i32 = 74;
/// Плитка целиком: миниатюра, имя, подпись.
const TILE_H: i32 = THUMB_H + 44;
const ROW: i32 = 34;

pub unsafe fn draw(p: &mut Painter, s: &State, w: i32) {
    let lw = w * 96 / p.dpi as i32;
    let col_w = (lw - 2 * LEFT - (COLS - 1) * GAP) / COLS;
    let items = &s.shelf.items;
    let mut y = TOP;
    if items.is_empty() {
        let r = p.r(LEFT, y, lw - LEFT, y + THUMB_H);
        p.fill(r, 24, PANEL);
        p.text(p.f.normal, "Перетащи файл на пилюлю, и он появится здесь", r, DIM, DT_CENTER);
        y += THUMB_H;
    } else {
        for (i, it) in items.iter().enumerate() {
            let (c, row) = (i as i32 % COLS, i as i32 / COLS);
            let x = LEFT + c * (col_w + GAP);
            let ty = TOP + row * (TILE_H + GAP);
            let (open, del) = (Hit::Shelf(i), Hit::ShelfDel(i));
            let hov = p.hovered(open) || p.hovered(del);
            let th = p.r(x, ty, x + col_w, ty + THUMB_H);
            match &it.thumb {
                Some(a) => p.art(th, a, 24),
                None => {
                    p.fill(th, 24, if hov { HOVER } else { PANEL });
                    p.text(p.f.small, &it.label(), th, FAINT, DT_CENTER);
                }
            }
            p.text(p.f.normal, &it.name, p.r(x, ty + THUMB_H + 6, x + col_w, ty + THUMB_H + 25), FG, DT_LEFT | DT_END_ELLIPSIS);
            p.text(p.f.small, &it.caption(), p.r(x, ty + THUMB_H + 25, x + col_w, ty + TILE_H), DIM, DT_LEFT | DT_END_ELLIPSIS);
            p.hit(p.r(x, ty, x + col_w, ty + TILE_H), open);
            if hov {
                // Крестик «убрать с полки» в углу миниатюры; сам файл не трогается.
                let b = p.r(x + col_w - 30, ty + 6, x + col_w - 6, ty + 30);
                p.fill(b, R_PILL, if p.hovered(del) { TRACK } else { BG });
                p.text(p.f.small, "✕", b, FG, DT_CENTER);
                p.hit(b, del);
            }
            y = ty + TILE_H;
        }
    }

    // Последние скопированные тексты: при двух рядах файлов одна строка, иначе три.
    let clip_rows = if items.len() > COLS as usize { 1 } else { 3 };
    let recent: Vec<_> = s.clip.items.iter().enumerate().take(clip_rows).collect();
    if !recent.is_empty() {
        y += GAP;
        p.fill(RECT { left: p.s(LEFT), top: p.s(y), right: w - p.s(LEFT), bottom: p.s(y) + p.s(1).max(1) }, 0, HOVER);
        y += GAP;
        p.text(p.f.small, "Скопированное", p.r(LEFT, y, lw - LEFT, y + 16), DIM, DT_LEFT);
        y += 20;
        let now = crate::feat::clip::now_secs();
        for (i, e) in recent {
            let h = Hit::ShelfClip(i);
            let row = p.r(LEFT - 10, y, lw - LEFT + 10, y + ROW);
            if p.hovered(h) {
                p.fill(row, R_ROW, HOVER);
            }
            super::search::clip_icon(p, row, e, false);
            let line: String = e.text.split_whitespace().collect::<Vec<_>>().join(" ");
            p.text(p.f.normal, &line, RECT { left: row.left + p.s(44), right: row.right - p.s(70), ..row }, FG, DT_LEFT | DT_END_ELLIPSIS);
            let age = crate::feat::clip::age_text(e.time, now);
            p.text(p.f.small, &age, RECT { left: row.right - p.s(66), right: row.right - p.s(10), ..row }, DIM, DT_RIGHT);
            p.hit(row, h);
            y += ROW;
        }
    }
    // Подсказка, пока на полке места хватает.
    if items.len() <= COLS as usize {
        y += 8;
        p.text(p.f.small, "Перетащи файл на пилюлю, чтобы положить сюда · тащи отсюда в любое окно", p.r(LEFT, y, lw - LEFT, y + 16), FAINT, DT_LEFT | DT_END_ELLIPSIS);
        y += 16;
    }
    p.reach(y);
}
