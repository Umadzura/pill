//! Вкладка «События»: две страницы с переключателем — карточки сессий ИИ-агентов и уведомления других программ.

use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::feat::agents::{fmt_age, Agent, AgentState};
use crate::feat::notifs::{self, Group, Notif};

/// Высота карточки агента и шаг между карточками.
const CARD_H: i32 = 50;
const CARD_STEP: i32 = 56;
/// Сколько карточек видно сразу, остальные листаются колесом.
const CARDS: usize = 5;
const LIST_TOP: i32 = 90;
/// Карточка чата на странице уведомлений и шаг между ними.
const NOTIF_H: i32 = 46;
const NOTIF_STEP: i32 = 50;

fn state_color(s: AgentState) -> COLORREF {
    match s {
        AgentState::Work => ACCENT,
        AgentState::Ask => WARN,
        AgentState::Wait => FG,
        AgentState::Done | AgentState::Gone => CAM,
    }
}

/// Переключатель страниц под шапкой и сводка справа от него.
unsafe fn pages(p: &mut Painter, s: &State, w: i32, notif_count: usize) {
    let items = [(false, format!("Агенты · {}", s.agents.list.len())), (true, format!("Уведомления · {}", notif_count))];
    let mut left = p.s(22);
    for (page, name) in items.iter() {
        let r = RECT { left, top: p.s(54), right: left + p.measure(p.f.normal, name), bottom: p.s(80) };
        left = r.right + p.s(18);
        let h = Hit::FeedPage(*page);
        let on = *page == s.feed_notifs;
        p.text(p.f.normal, name, r, if on { FG } else if p.hovered(h) { DIM } else { FAINT }, DT_LEFT);
        p.hit(r, h);
    }
    let frame = RECT { left: p.s(22), top: p.s(54), right: left, bottom: p.s(80) };
    let (ask, work) = s.agents.summary();
    let mut parts = Vec::new();
    if ask > 0 {
        parts.push(format!("{} ждёт тебя", ask));
    }
    if work > 0 {
        parts.push(format!("{} работает", work));
    }
    let r = RECT { left: frame.right + p.s(10), top: frame.top, right: w - p.s(20), bottom: frame.bottom };
    p.text(p.f.small, &parts.join(" · "), r, if ask > 0 { WARN } else { DIM }, DT_RIGHT);
}

/// Карточка сессии: маскот, имя, ветка, состояние и время в нём; второй строкой запрос человека
/// и текущий инструмент, у закончившего — последний ответ.
unsafe fn card(p: &mut Painter, s: &State, w: i32, y: i32, i: usize, a: &Agent, now: Instant) {
    let row = RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + CARD_H) };
    let hit = Hit::AgentDrop(i);
    let hov = p.hovered(hit);
    let c = state_color(a.state);
    if a.state == AgentState::Ask {
        // Тому, кто ждёт разрешения, рамка цветом состояния.
        p.fill(row, R_TILE, c);
        let b = p.s(1);
        p.fill(RECT { left: row.left + b, top: row.top + b, right: row.right - b, bottom: row.bottom - b }, R_TILE, if hov { HOVER } else { PANEL });
    } else {
        p.fill(row, R_TILE, if hov { HOVER } else { PANEL });
    }
    let mid = (row.top + row.bottom) / 2;
    let px = p.s(3);
    super::mascot::draw(p, row.left + p.s(12), mid - px * super::mascot::H / 2, px, a, s.notif_flip);

    let (left, right) = (row.left + p.s(50), row.right - p.s(12));
    let top = RECT { left, top: row.top + p.s(6), right, bottom: row.top + p.s(26) };
    let mut x = left;
    let nw = p.measure(p.f.bold, &a.name).min(p.s(150));
    p.text(p.f.bold, &a.name, RECT { left: x, right: x + nw, ..top }, FG, DT_LEFT | DT_END_ELLIPSIS);
    x += nw + p.s(8);
    if let Some(b) = a.branch.as_ref().filter(|b| !b.is_empty()) {
        let bw = (p.measure(p.f.small, b) + p.s(14)).min(p.s(130));
        let chip = RECT { left: x, top: top.top + p.s(1), right: x + bw, bottom: top.bottom - p.s(1) };
        p.fill(chip, R_PILL, if hov { TRACK } else { HOVER });
        p.text(p.f.small, b, RECT { left: chip.left + p.s(7), right: chip.right - p.s(7), ..chip }, DIM, DT_LEFT | DT_END_ELLIPSIS);
        x += bw + p.s(8);
    }
    p.text(p.f.normal, a.state.label(), RECT { left: x, right: right - p.s(56), ..top }, c, DT_LEFT | DT_END_ELLIPSIS);
    // Справа: сколько в этом состоянии; под курсором — крестик «убрать».
    let tail = RECT { left: right - p.s(52), ..top };
    if hov {
        p.text(p.f.icon, I_DEL, tail, FG, DT_RIGHT);
    } else {
        p.text(p.f.small, &fmt_age(now.duration_since(a.since)), tail, DIM, DT_RIGHT);
    }

    let low = RECT { left, top: row.top + p.s(26), right, bottom: row.bottom - p.s(6) };
    let done = matches!(a.state, AgentState::Done | AgentState::Gone);
    if done && !a.answer.is_empty() {
        p.text(p.f.small, &format!("ответ: {}", a.answer), low, DIM, DT_LEFT | DT_END_ELLIPSIS);
    } else {
        // Справа инструмент (не шире половины), слева запрос человека.
        let tw = if a.tool.is_empty() || done { 0 } else { p.measure(p.f.small, &a.tool).min((right - left) / 2) };
        if tw > 0 {
            p.text(p.f.small, &a.tool, RECT { left: right - tw, ..low }, FG, DT_LEFT | DT_END_ELLIPSIS);
        }
        if !a.note.is_empty() {
            let gap = if tw > 0 { tw + p.s(12) } else { 0 };
            p.text(p.f.small, &format!("ты: {}", a.note), RECT { right: right - gap, ..low }, DIM, DT_LEFT | DT_END_ELLIPSIS);
        }
    }
    p.hit(row, hit);
}

unsafe fn agents_page(p: &mut Painter, s: &State, w: i32, h: i32) {
    if s.agents.list.is_empty() {
        let r = RECT { left: p.s(20), top: p.s(LIST_TOP), right: w - p.s(20), bottom: p.s(LIST_TOP + 24) };
        p.text(p.f.small, "Пока никого. Claude Code появится после первого запроса (хуки шлют события в /hook).", r, DIM, DT_LEFT | DT_END_ELLIPSIS);
        p.reach(LIST_TOP + 24);
        return;
    }
    let order = s.agents.order();
    let start = s.feed_scroll.min(order.len().saturating_sub(CARDS));
    let now = Instant::now();
    for (n, &i) in order.iter().skip(start).take(CARDS).enumerate() {
        card(p, s, w, LIST_TOP + n as i32 * CARD_STEP, i, &s.agents.list[i], now);
    }
    let shown = order.len().saturating_sub(start).min(CARDS) as i32;
    let end = LIST_TOP + shown * CARD_STEP - (CARD_STEP - CARD_H);
    p.reach(end);
    let more = order.len().saturating_sub(start + CARDS);
    if more > 0 || start > 0 {
        // Подпись сразу под последней карточкой: панель по высоте содержимого.
        let foot = RECT { left: p.s(20), top: p.s(end + 6), right: w - p.s(24), bottom: p.s(end + 24) };
        let t = if more > 0 { format!("ещё {} ↓", more) } else { "↑ к началу".to_string() };
        p.text(p.f.small, &t, foot, DIM, DT_RIGHT);
        p.reach(end + 24);
    }
    let _ = h;
}

unsafe fn notifs_page(p: &mut Painter, w: i32, h: i32, list: &[Notif]) {
    let mut y = LIST_TOP;
    if list.is_empty() {
        let off = notifs::OFF.load(std::sync::atomic::Ordering::Relaxed);
        let msg = if off { "Нет доступа: Параметры → Конфиденциальность → Уведомления" } else { "Новых нет" };
        p.text(p.f.small, msg, RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + 24) }, DIM, DT_LEFT);
        p.reach(y + 24);
        return;
    }
    let now_ft = crate::sys::toasts::now_filetime();
    let groups: Vec<Group> = notifs::groups(list);
    let mut app = "";
    for g in &groups {
        let label = g.app != app;
        if p.s(y + NOTIF_H + if label { 20 } else { 0 }) > h - p.s(10) {
            break;
        }
        if label {
            // Подпись программы и сколько от неё уведомлений.
            app = &g.app;
            let total: usize = groups.iter().filter(|x| x.app == g.app).map(|x| x.count).sum();
            let r = RECT { left: p.s(22), top: p.s(y), right: w - p.s(22), bottom: p.s(y + 18) };
            p.text(p.f.small, &format!("{} · {}", g.app, total), r, DIM, DT_LEFT | DT_END_ELLIPSIS);
            y += 20;
        }
        let row = RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + NOTIF_H) };
        y += NOTIF_STEP;
        let hit = Hit::NotifGroup(g.newest);
        let hov = p.hovered(hit);
        p.fill(row, R_TILE, if hov { HOVER } else { PANEL });
        // Кружок с первой буквой отправителя, цвет по имени.
        let (d, mid) = (p.s(28), (row.top + row.bottom) / 2);
        let av = RECT { left: row.left + p.s(10), top: mid - d / 2, right: row.left + p.s(10) + d, bottom: mid - d / 2 + d };
        p.fill(av, R_PILL, super::mascot::body_color(&g.title));
        let letter: String = g.title.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
        p.text(p.f.bold, &letter, av, rgb(17, 17, 17), DT_CENTER);

        let (left, right) = (row.left + p.s(48), row.right - p.s(12));
        let top = RECT { left, top: row.top + p.s(5), right, bottom: row.top + p.s(25) };
        let nw = p.measure(p.f.bold, &g.title).min(right - left - p.s(110));
        p.text(p.f.bold, &g.title, RECT { right: left + nw, ..top }, FG, DT_LEFT | DT_END_ELLIPSIS);
        if g.count > 1 {
            let n = g.count.to_string();
            let cw = p.measure(p.f.small, &n) + p.s(12);
            let chip = RECT { left: left + nw + p.s(8), top: top.top + p.s(2), right: left + nw + p.s(8) + cw, bottom: top.bottom - p.s(2) };
            p.fill(chip, R_PILL, if hov { TRACK } else { HOVER });
            p.text(p.f.small, &n, chip, DIM, DT_CENTER);
        }
        let tail = RECT { left: right - p.s(52), ..top };
        if hov {
            p.text(p.f.icon, I_DEL, tail, FG, DT_RIGHT);
        } else {
            p.text(p.f.small, &fmt_age(notifs::age(g.created, now_ft)), tail, DIM, DT_RIGHT);
        }
        // Вторая строка: последнее сообщение чата.
        let body = g.last.last().map(String::as_str).unwrap_or("");
        p.text(p.f.small, body, RECT { left, top: row.top + p.s(24), right, bottom: row.bottom - p.s(5) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
        p.hit(row, hit);
        p.reach(y - (NOTIF_STEP - NOTIF_H));
    }
}

pub unsafe fn draw(p: &mut Painter, s: &State, w: i32, h: i32) {
    let list: Vec<Notif> = notifs::INBOX.lock().unwrap().list.clone();
    pages(p, s, w, list.len());
    if s.feed_notifs {
        notifs_page(p, w, h, &list);
    } else {
        agents_page(p, s, w, h);
    }
}
