//! Карточка из чёлки. Уведомление: кружок с первой буквой отправителя, имя, программа, текст и
//! (под курсором) кнопки. Просьба агента и «агент закончил»: значок, заголовок, текст, кнопки ответа.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::feat::peek::{Peek, PeekHit, PeekKind, W};

/// Первая буква для кружка или значка.
fn letter(s: &str) -> String {
    s.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// Кнопка-капсула шириной по подписи. Возвращает её правый край.
unsafe fn button(p: &mut Painter, x: i32, top: i32, label: &str, hit: PeekHit, primary: bool) -> i32 {
    let w = p.measure(p.f.bold, label) + p.s(26);
    let r = RECT { left: x, top, right: x + w, bottom: top + p.s(28) };
    let h = Hit::Peek(hit);
    let bg = match (primary, p.hovered(h)) {
        (true, false) => ACCENT,
        (true, true) => rgb(64, 156, 255),
        (false, false) => HOVER,
        (false, true) => TRACK,
    };
    p.fill(r, R_PILL, bg);
    p.text(p.f.bold, label, r, FG, DT_CENTER);
    p.hit(r, h);
    r.right
}

unsafe fn notif(p: &mut Painter, pk: &Peek, h: i32) {
    // Кружок с первой буквой отправителя, цвет по имени: настоящую аватарку Windows не отдаёт.
    let av = p.r(16, 14, 60, 58);
    p.fill(av, R_PILL, super::mascot::body_color(&pk.title));
    p.text(p.f.title, &letter(&pk.title), av, rgb(17, 17, 17), DT_CENTER);

    let (x, right) = (p.s(74), p.s(W - 16));
    let top = RECT { left: x, top: p.s(12), right, bottom: p.s(34) };
    let aw = p.measure(p.f.small, &pk.app).min(p.s(150));
    p.text(p.f.small, &pk.app, RECT { left: right - aw, ..top }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    let name_right = right - aw - p.s(12);
    let nw = p.measure(p.f.bold, &pk.title).min(name_right - x - p.s(34));
    p.text(p.f.bold, &pk.title, RECT { right: x + nw, ..top }, FG, DT_LEFT | DT_END_ELLIPSIS);
    if pk.count > 1 {
        // Сколько сообщений из этого чата накопилось.
        let n = pk.count.to_string();
        let cw = p.measure(p.f.small, &n) + p.s(12);
        let chip = RECT { left: x + nw + p.s(8), top: top.top + p.s(3), right: x + nw + p.s(8) + cw, bottom: top.bottom - p.s(3) };
        p.fill(chip, R_PILL, HOVER);
        p.text(p.f.small, &n, chip, DIM, DT_CENTER);
    }

    if pk.lines.len() > 1 {
        for (i, l) in pk.lines.iter().enumerate() {
            let y = 36 + i as i32 * 20;
            p.text(p.f.normal, l, RECT { left: x, top: p.s(y), right, bottom: p.s(y + 20) }, FG, DT_LEFT | DT_END_ELLIPSIS);
        }
    } else {
        // Одно сообщение: до двух строк с переносом.
        p.text_wrap(p.f.normal, &pk.body, RECT { left: x, top: p.s(36), right, bottom: p.s(76) }, FG);
    }

    if pk.hover {
        let y = h - p.s(42);
        let open = format!("Открыть {}", pk.app);
        let label = if pk.app.is_empty() || pk.aumid.is_empty() { "Открыть" } else { open.as_str() };
        let mut bx = p.s(16);
        if !pk.aumid.is_empty() {
            bx = button(p, bx, y, label, PeekHit::Open, true) + p.s(8);
        }
        button(p, bx, y, "Скрыть", PeekHit::Dismiss, false);
        // Справа тихая кнопка без подложки.
        let q = "Тишина на час";
        let qw = p.measure(p.f.small, q) + p.s(16);
        let qr = RECT { left: right - qw, top: y, right, bottom: y + p.s(28) };
        let qh = Hit::Peek(PeekHit::Quiet);
        p.text(p.f.small, q, qr, if p.hovered(qh) { FG } else { DIM }, DT_RIGHT);
        p.hit(qr, qh);
    }
}

unsafe fn agent(p: &mut Painter, pk: &Peek) {
    let color = if matches!(pk.kind, PeekKind::Ask { .. }) { WARN } else { CAM };
    // Значок: цветной скруглённый квадрат с первой буквой программы.
    let icon = p.r(20, 22, 62, 64);
    p.fill(icon, 22, color);
    p.text(p.f.title, &letter(&pk.app), icon, BG, DT_CENTER);

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

pub unsafe fn draw(p: &mut Painter, s: &State) {
    // Карточку уже убрали, окно ещё сворачивается: рисовать нечего.
    let Some(pk) = &s.peek else { return };
    let h = p.s(pk.height());
    p.hit(RECT { left: 0, top: 0, right: p.s(W), bottom: h }, Hit::Peek(PeekHit::Dismiss));
    match pk.kind {
        PeekKind::Notif => notif(p, pk, h),
        _ => agent(p, pk),
    }
}
