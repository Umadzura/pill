//! Карточка из чёлки: значок программы, заголовок, текст и (для просьбы о разрешении) кнопки.

use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::feat::peek::{PeekHit, PeekKind, W};

pub unsafe fn draw(p: &mut Painter, s: &State) {
    // Карточку уже убрали, окно ещё сворачивается: рисовать нечего.
    let Some(pk) = &s.peek else { return };
    let h = pk.height();
    p.hit(p.r(0, 0, W, h), Hit::Peek(PeekHit::Dismiss));
    let color = match pk.kind {
        PeekKind::Notif => ACCENT,
        PeekKind::Ask { .. } => WARN,
        PeekKind::Done => CAM,
    };
    // Значок: цветной скруглённый квадрат с первой буквой программы.
    let icon = p.r(20, 22, 62, 64);
    p.fill(icon, 22, color);
    let letter: String = pk.app.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
    p.text(p.f.title, &letter, icon, BG, DT_CENTER);

    let (x, right) = (78, W - 22);
    p.text(p.f.small, &pk.app, p.r(x, 12, right, 30), DIM, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.bold, &pk.title, p.r(x, 30, right, 52), FG, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.normal, &pk.body, p.r(x, 52, right, 74), DIM, DT_LEFT | DT_END_ELLIPSIS);

    if pk.answerable() {
        let (mid, top, bottom) = (W / 2, 84, 116);
        for (r, label, hit, on) in [
            (p.r(22, top, mid - 5, bottom), "Отклонить", PeekHit::Deny, false),
            (p.r(mid + 5, top, W - 22, bottom), "Разрешить", PeekHit::Allow, true),
        ] {
            let h = Hit::Peek(hit);
            let hov = p.hovered(h);
            let bg = match (on, hov) {
                (true, false) => ACCENT,
                (true, true) => rgb(64, 156, 255),
                (false, false) => PANEL,
                (false, true) => HOVER,
            };
            p.fill(r, R_PILL, bg);
            p.text(p.f.bold, label, r, FG, DT_CENTER);
            p.hit(r, h);
        }
    }
}
