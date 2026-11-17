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
        (true, true) => rgb(220, 220, 224),
        (false, false) => HOVER,
        (false, true) => TRACK,
    };
    p.fill(r, R_PILL, bg);
    // На белой кнопке текст чёрный.
    p.text(p.f.bold, label, RECT { left: r.left + p.s(13), ..r }, if primary { INK } else { FG }, DT_LEFT);
    if !key.is_empty() {
        p.text(p.f.small, key, RECT { right: r.right - p.s(12), ..r }, if primary { rgb(84, 84, 88) } else { DIM }, DT_RIGHT);
    }
    p.hit(r, h);
    r.right
}

unsafe fn notif(p: &mut Painter, pk: &Peek, h: i32) {
    // Кружок с первой буквой отправителя, цвет по имени: настоящую аватарку Windows не отдаёт.
    let av = p.r(16, 14, 56, 54);
    p.fill(av, R_PILL, super::mascot::body_color(&pk.title));
    p.text(p.f.title, &letter(&pk.title), av, rgb(17, 17, 17), DT_CENTER);

    let (x, right) = (p.s(70), p.s(W - 16));
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
        p.text(p.f.small, &n, chip, DIM, DT_LEFT);
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

/// Тихая ссылка-слово справа. Возвращает её левый край.
unsafe fn link(p: &mut Painter, right: i32, top: i32, label: &str, hit: PeekHit) -> i32 {
    let w = p.measure(p.f.small, label);
    let r = RECT { left: right - w, top, right, bottom: top + p.s(28) };
    let h = Hit::Peek(hit);
    p.text(p.f.small, label, r, if p.hovered(h) { FG } else { DIM }, DT_RIGHT);
    p.hit(RECT { left: r.left - p.s(4), right: r.right + p.s(4), ..r }, h);
    r.left
}

/// Просьба агента, на которую можно ответить отсюда: маскот, одна фраза простыми словами, две кнопки.
/// Код и запрос человека — за словом «подробнее».
unsafe fn ask(p: &mut Painter, s: &State, pk: &Peek) {
    let (left, right) = (p.s(20), p.s(W_ASK - 20));
    if let Some(a) = s.agents.list.iter().find(|a| a.name == pk.app) {
        let px = p.s(4);
        super::mascot::draw(p, left, p.s(18), px, a, s.notif_flip);
    }
    let x = p.s(72);
    let (verb, target) = crate::feat::agents::ask_phrase(&pk.tool);
    // Сверху мелко «pill хочет изменить файл», под ним крупно на что.
    let cap = RECT { left: x, top: p.s(14), right, bottom: p.s(32) };
    let nw = p.measure(p.f.bold, &pk.title);
    p.text(p.f.bold, &pk.title, cap, FG, DT_LEFT);
    p.text(p.f.small, &verb, RECT { left: x + nw + p.s(6), ..cap }, WARN, DT_LEFT | DT_END_ELLIPSIS);
    if !s.asks.is_empty() {
        p.text(p.f.small, &format!("ещё {} в очереди", s.asks.len()), cap, DIM, DT_RIGHT);
    }
    let hero = if target.is_empty() { pk.body.clone() } else { target };
    p.text(p.f.hero, &hero, RECT { left: x, top: p.s(32), right, bottom: p.s(64) }, FG, DT_LEFT | DT_END_ELLIPSIS);

    let mut y = 78;
    if pk.more {
        if !pk.body.is_empty() {
            p.text(p.f.small, &format!("ты: {}", pk.body), RECT { left, top: p.s(y), right, bottom: p.s(y + 18) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
        }
        y += 22;
        // Строки кода за тонкой чертой: убранные красным, добавленные зелёным, команда белым.
        let ch = pk.code_height();
        if ch > 0 {
            p.fill(RECT { left, top: p.s(y), right: left + p.s(1).max(1), bottom: p.s(y + ch - 8) }, 0, HOVER);
            for (i, l) in pk.lines.iter().enumerate() {
                let ly = y + i as i32 * 18;
                let (mark, color) = match l.chars().next() {
                    Some('+') => ("+ ", ADDED),
                    Some('-') => ("− ", REMOVED),
                    _ => ("", FG),
                };
                let text = format!("{}{}", mark, l.get(1..).unwrap_or(""));
                p.text(p.f.small, &text, RECT { left: left + p.s(12), top: p.s(ly), right, bottom: p.s(ly + 18) }, color, DT_LEFT | DT_END_ELLIPSIS);
            }
        }
        y += ch + 12;
    }

    let by = p.s(y);
    let bx = button(p, left, by, "Разрешить", "Alt Y", PeekHit::Allow, true) + p.s(8);
    button(p, bx, by, "Нет", "Alt N", PeekHit::Deny, false);
    // Справа тихие слова: в терминал, всегда (если можно), подробнее.
    let mut rx = right;
    rx = link(p, rx, by, "в терминал", PeekHit::Dismiss) - p.s(12);
    if pk.can_always {
        rx = link(p, rx, by, "всегда", PeekHit::Always) - p.s(12);
    }
    link(p, rx, by, if pk.more { "скрыть" } else { "подробнее" }, PeekHit::More);
}

/// Вопрос агента: маскот, «имя спрашивает», текст вопроса, варианты строками с номером клавиши.
unsafe fn question(p: &mut Painter, s: &State, pk: &Peek, q: &crate::feat::agents::Question) {
    let (left, right) = (p.s(20), p.s(W_ASK - 20));
    if let Some(a) = s.agents.list.iter().find(|a| a.name == pk.app) {
        super::mascot::draw(p, left, p.s(18), p.s(4), a, s.notif_flip);
    }
    let x = p.s(72);
    let cap = RECT { left: x, top: p.s(14), right, bottom: p.s(32) };
    let nw = p.measure(p.f.bold, &pk.title);
    p.text(p.f.bold, &pk.title, cap, FG, DT_LEFT);
    p.text(p.f.small, "спрашивает", RECT { left: x + nw + p.s(6), ..cap }, WARN, DT_LEFT);
    p.text_wrap(p.f.normal, &q.text, RECT { left: x, top: p.s(34), right, bottom: p.s(74) }, FG);
    let mut y = 84;
    if pk.picks() == 0 && q.count > 1 {
        let r = RECT { left, top: p.s(y), right, bottom: p.s(y + 24) };
        p.text(p.f.small, &format!("Вопросов сразу {}: ответь в терминале", q.count), r, DIM, DT_LEFT);
    }
    for (i, label) in q.options.iter().enumerate().take(pk.picks()) {
        let row = RECT { left, top: p.s(y), right, bottom: p.s(y + 34) };
        let hit = Hit::Peek(PeekHit::Pick(i as u8));
        p.fill(row, R_ROW, if p.hovered(hit) { HOVER } else { PANEL });
        p.text(p.f.small, &format!("Alt {}", i + 1), RECT { left: row.left + p.s(12), right: row.left + p.s(52), ..row }, DIM, DT_LEFT);
        p.text(p.f.normal, label, RECT { left: row.left + p.s(56), right: row.right - p.s(12), ..row }, FG, DT_LEFT | DT_END_ELLIPSIS);
        p.hit(row, hit);
        y += 40;
    }
    link(p, right, p.s(y + 2), "ответить своими словами в терминале · Alt T", PeekHit::Dismiss);
}

/// «Агент готов»: его маскот с улыбкой, крупно «notes готов», под ним сколько работал.
/// Просьба без ответа (напоминание из уведомления Claude Code): значок, заголовок, текст.
unsafe fn agent(p: &mut Painter, s: &State, pk: &Peek) {
    if matches!(pk.kind, PeekKind::Done) {
        if let Some(a) = s.agents.list.iter().find(|a| a.name == pk.app) {
            let px = p.s(4);
            super::mascot::draw(p, p.s(20), p.s(20), px, a, s.notif_flip);
        }
        let x = p.s(72);
        p.text(p.f.hero, &pk.title, RECT { left: x, top: p.s(14), right: p.s(W - 20), bottom: p.s(44) }, FG, DT_LEFT | DT_END_ELLIPSIS);
        p.text(p.f.small, &pk.body, RECT { left: x, top: p.s(44), right: p.s(W - 20), bottom: p.s(62) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
        return;
    }
    let icon = p.r(20, 22, 62, 64);
    p.fill(icon, 22, WARN);
    p.text(p.f.title, &letter(&pk.app), icon, BG, DT_CENTER);
    let (x, right) = (78, W - 22);
    p.text(p.f.small, &pk.app, p.r(x, 12, right, 30), DIM, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.bold, &pk.title, p.r(x, 30, right, 52), FG, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.normal, &pk.body, p.r(x, 52, right, 74), DIM, DT_LEFT | DT_END_ELLIPSIS);
}

/// Снимок экрана: миниатюра слева (щелчок открывает), справа «Снимок экрана» и размер, ниже кнопки.
unsafe fn snap(p: &mut Painter, pk: &Peek) {
    let th = p.r(16, 16, 16 + 127, 16 + 74);
    let open = Hit::Peek(PeekHit::Open);
    match &pk.art {
        Some(a) => p.art(th, a, 14),
        None => {
            p.fill(th, 14, PANEL);
            p.text(p.f.small, "PNG", th, FAINT, DT_CENTER);
        }
    }
    p.hit(th, open);
    let (x, right) = (16 + 127 + 16, W - 16);
    p.text(p.f.bold, &pk.title, p.r(x, 22, right, 44), FG, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.small, &pk.body, p.r(x, 46, right, 64), DIM, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.small, "в буфере, можно вставлять", p.r(x, 64, right, 82), FAINT, DT_LEFT | DT_END_ELLIPSIS);
    let y = p.s(16 + 74 + 14);
    let mut bx = button(p, p.s(16), y, "Открыть", "", PeekHit::Open, true) + p.s(8);
    bx = button(p, bx, y, "На полку", "", PeekHit::Shelf, false) + p.s(8);
    button(p, bx, y, "Сохранить как", "", PeekHit::SaveAs, false);
}

pub unsafe fn draw(p: &mut Painter, s: &State) {
    // Карточку уже убрали, окно ещё сворачивается: рисовать нечего.
    let Some(pk) = &s.peek else { return };
    let h = p.s(pk.height());
    if pk.answerable() {
        // У просьбы клик мимо кнопок ничего не делает: случайно отправить её в терминал нельзя.
        match &pk.question {
            Some(q) => question(p, s, pk, q),
            None => ask(p, s, pk),
        }
        return;
    }
    p.hit(RECT { left: 0, top: 0, right: p.s(pk.width()), bottom: h }, Hit::Peek(PeekHit::Dismiss));
    match pk.kind {
        PeekKind::Notif => notif(p, pk, h),
        PeekKind::Snap => snap(p, pk),
        _ => agent(p, s, pk),
    }
}
