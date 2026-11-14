//! Карточка из чёлки. Уведомление: кружок с первой буквой отправителя, имя, программа, текст и
//! (под курсором) кнопки. Просьба агента с ответом: маскот, проект, ветка, что именно он хочет
//! сделать (строки кода с «+» и «−»), кнопки с клавишами. «Агент закончил»: значок, заголовок, текст.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::feat::peek::{Peek, PeekHit, PeekKind, W, W_ASK};

/// Цвета добавленных и убранных строк.
const ADDED: COLORREF = rgb(123, 228, 149);
const REMOVED: COLORREF = rgb(255, 154, 147);

/// Первая буква для кружка или значка.
fn letter(s: &str) -> String {
    s.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// Кнопка-капсула шириной по подписи; `key` — клавиша мелко справа от подписи. Возвращает правый край.
unsafe fn button(p: &mut Painter, x: i32, top: i32, label: &str, key: &str, hit: PeekHit, primary: bool) -> i32 {
    let lw = p.measure(p.f.bold, label);
    let kw = if key.is_empty() { 0 } else { p.measure(p.f.small, key) + p.s(8) };
    let r = RECT { left: x, top, right: x + lw + kw + p.s(26), bottom: top + p.s(28) };
    let h = Hit::Peek(hit);
    let bg = match (primary, p.hovered(h)) {
        (true, false) => ACCENT,
        (true, true) => rgb(64, 156, 255),
        (false, false) => HOVER,
        (false, true) => TRACK,
    };
    p.fill(r, R_PILL, bg);
    p.text(p.f.bold, label, RECT { left: r.left + p.s(13), ..r }, FG, DT_LEFT);
    if !key.is_empty() {
        p.text(p.f.small, key, RECT { right: r.right - p.s(12), ..r }, if primary { FG } else { DIM }, DT_RIGHT);
    }
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
        let mut bx = p.s(16);
        if !pk.aumid.is_empty() {
            bx = button(p, bx, y, if pk.app.is_empty() { "Открыть" } else { open.as_str() }, "", PeekHit::Open, true) + p.s(8);
        }
        button(p, bx, y, "Скрыть", "", PeekHit::Dismiss, false);
        // Справа тихая кнопка без подложки.
        let q = "Тишина на час";
        let qw = p.measure(p.f.small, q) + p.s(16);
        let qr = RECT { left: right - qw, top: y, right, bottom: y + p.s(28) };
        let qh = Hit::Peek(PeekHit::Quiet);
        p.text(p.f.small, q, qr, if p.hovered(qh) { FG } else { DIM }, DT_RIGHT);
        p.hit(qr, qh);
    }
}

/// Просьба агента, на которую можно ответить отсюда.
unsafe fn ask(p: &mut Painter, s: &State, pk: &Peek) {
    let (left, right) = (p.s(20), p.s(W_ASK - 20));
    // Маскот того, кто просит: лицо «нужно разрешение».
    if let Some(a) = s.agents.list.iter().find(|a| a.name == pk.app) {
        let px = p.s(4);
        super::mascot::draw(p, left, p.s(14), px, a, s.notif_flip);
    }
    let x = p.s(74);
    let top = RECT { left: x, top: p.s(12), right, bottom: p.s(36) };
    let nw = p.measure(p.f.title, &pk.title).min(p.s(170));
    p.text(p.f.title, &pk.title, RECT { right: x + nw, ..top }, FG, DT_LEFT | DT_END_ELLIPSIS);
    let mut cx = x + nw + p.s(10);
    if !pk.branch.is_empty() {
        let bw = (p.measure(p.f.small, &pk.branch) + p.s(14)).min(p.s(130));
        let chip = RECT { left: cx, top: top.top + p.s(3), right: cx + bw, bottom: top.bottom - p.s(3) };
        p.fill(chip, R_PILL, PANEL);
        p.text(p.f.small, &pk.branch, RECT { left: chip.left + p.s(7), right: chip.right - p.s(7), ..chip }, DIM, DT_LEFT | DT_END_ELLIPSIS);
        cx += bw + p.s(10);
    }
    p.text(p.f.normal, "просит разрешение", RECT { left: cx, ..top }, WARN, DT_LEFT);
    // Сколько ещё просьб ждёт за этой.
    if !s.asks.is_empty() {
        p.text(p.f.small, &format!("ещё {} в очереди", s.asks.len()), top, DIM, DT_RIGHT);
    }
    if !pk.body.is_empty() {
        p.text(p.f.small, &format!("ты: {}", pk.body), RECT { left: x, top: p.s(36), right, bottom: p.s(54) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    }

    // Инструмент и на что он нацелен; справа сколько строк добавляет и убирает.
    let row = RECT { left, top: p.s(62), right, bottom: p.s(82) };
    let (tool, target) = pk.tool.split_once(": ").unwrap_or((pk.tool.as_str(), ""));
    let tw = p.measure(p.f.bold, tool);
    p.text(p.f.bold, tool, row, FG, DT_LEFT);
    let (plus, minus) = pk.stat;
    let stat_w = if plus + minus > 0 { p.s(90) } else { 0 };
    p.text(p.f.small, target, RECT { left: left + tw + p.s(10), right: right - stat_w, ..row }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    if plus + minus > 0 {
        let m = format!("−{}", minus);
        let mw = p.measure(p.f.small, &m);
        p.text(p.f.small, &m, row, REMOVED, DT_RIGHT);
        p.text(p.f.small, &format!("+{}", plus), RECT { right: right - mw - p.s(10), ..row }, ADDED, DT_RIGHT);
    }

    // Строки кода: убранные красным, добавленные зелёным, команда серым.
    let ch = pk.code_height();
    if ch > 0 {
        let bx = RECT { left, top: p.s(88), right, bottom: p.s(88 + ch) };
        p.fill(bx, R_TILE, PANEL);
        for (i, l) in pk.lines.iter().enumerate() {
            let y = 88 + 8 + i as i32 * 18;
            let (mark, color) = match l.chars().next() {
                Some('+') => ("+ ", ADDED),
                Some('-') => ("− ", REMOVED),
                _ => ("", FG),
            };
            let text = format!("{}{}", mark, l.get(1..).unwrap_or(""));
            p.text(p.f.small, &text, RECT { left: bx.left + p.s(12), top: p.s(y), right: bx.right - p.s(12), bottom: p.s(y + 18) }, color, DT_LEFT | DT_END_ELLIPSIS);
        }
    }

    let by = p.s(88 + ch + 10);
    let mut bx = button(p, left, by, "Разрешить", "Alt Y", PeekHit::Allow, true) + p.s(8);
    if pk.can_always {
        bx = button(p, bx, by, "Всегда", "Alt A", PeekHit::Always, false) + p.s(8);
    }
    button(p, bx, by, "Отклонить", "Alt N", PeekHit::Deny, false);
    // Справа тихая кнопка: отпустить просьбу в окно агента.
    let t = "В терминал   Alt T";
    let tr = RECT { left: right - p.measure(p.f.small, t) - p.s(8), top: by, right, bottom: by + p.s(28) };
    let th = Hit::Peek(PeekHit::Dismiss);
    p.text(p.f.small, t, tr, if p.hovered(th) { FG } else { DIM }, DT_RIGHT);
    p.hit(tr, th);

    let foot = RECT { left, top: by + p.s(36), right, bottom: by + p.s(56) };
    p.text(p.f.small, "Клавиши работают, пока запрос ждёт · Без ответа 30 с запрос вернётся в терминал", foot, DIM, DT_LEFT | DT_END_ELLIPSIS);
}

/// «Агент закончил» и просьба без ответа (просто напоминание).
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
}

pub unsafe fn draw(p: &mut Painter, s: &State) {
    // Карточку уже убрали, окно ещё сворачивается: рисовать нечего.
    let Some(pk) = &s.peek else { return };
    let h = p.s(pk.height());
    if pk.answerable() {
        // У просьбы клик мимо кнопок ничего не делает: случайно отправить её в терминал нельзя.
        ask(p, s, pk);
        return;
    }
    p.hit(RECT { left: 0, top: 0, right: p.s(pk.width()), bottom: h }, Hit::Peek(PeekHit::Dismiss));
    match pk.kind {
        PeekKind::Notif => notif(p, pk, h),
        _ => agent(p, pk),
    }
}
