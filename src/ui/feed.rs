//! Вкладка «События»: ИИ-агенты с их состоянием и уведомления других программ.

use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::feat::agents::{fmt_age, AgentState};
use crate::feat::notifs::{self, Notif};

const AGENT_H: i32 = 32;
const NOTIF_H: i32 = 28;

fn state_color(s: AgentState) -> COLORREF {
    match s {
        AgentState::Work => ACCENT,
        AgentState::Ask => WARN,
        AgentState::Wait => FG,
        AgentState::Done | AgentState::Gone => CAM,
    }
}

/// Подпись раздела. Возвращает y (логические px) под ней.
unsafe fn section(p: &Painter, w: i32, y: i32, title: &str, note: &str) -> i32 {
    let left = p.s(20);
    p.text(p.f.small, title, RECT { left, top: p.s(y), right: w - left, bottom: p.s(y + 18) }, DIM, DT_LEFT);
    p.text(p.f.small, note, RECT { left, top: p.s(y), right: w - left, bottom: p.s(y + 18) }, DIM, DT_RIGHT);
    y + 20
}

pub unsafe fn draw(p: &mut Painter, s: &State, w: i32, h: i32) {
    let mut y = section(p, w, 52, "Агенты", "");
    if s.agents.list.is_empty() {
        let r = RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + 24) };
        p.text(p.f.small, "Пока никого. Claude Code появится после первого запроса (хуки шлют события в /hook).", r, DIM, DT_LEFT | DT_END_ELLIPSIS);
        y += 28;
    }
    let now = Instant::now();
    for (i, a) in s.agents.list.iter().enumerate() {
        let row = RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + AGENT_H - 4) };
        y += AGENT_H;
        let hit = Hit::AgentDrop(i);
        let hov = p.hovered(hit);
        p.fill(row, R_ROW, if hov { HOVER } else { PANEL });
        let c = state_color(a.state);
        // Маскот: цвет по имени, состояние по лицу.
        let mid = (row.top + row.bottom) / 2;
        let px = p.s(2);
        super::mascot::draw(p, row.left + p.s(10), mid - px * super::mascot::H / 2, px, a, s.notif_flip);
        // Имя, состояние, последний запрос.
        let mut x = row.left + p.s(36);
        let nw = p.measure(p.f.bold, &a.name).min(p.s(150));
        p.text(p.f.bold, &a.name, RECT { left: x, right: x + nw, ..row }, FG, DT_LEFT | DT_END_ELLIPSIS);
        x += nw + p.s(10);
        let label = a.state.label();
        let lw = p.measure(p.f.normal, label);
        p.text(p.f.normal, label, RECT { left: x, right: x + lw, ..row }, c, DT_LEFT);
        x += lw + p.s(12);
        let right = row.right - p.s(64);
        if right > x {
            p.text(p.f.small, &a.note, RECT { left: x, right, ..row }, DIM, DT_LEFT | DT_END_ELLIPSIS);
        }
        // Справа: сколько в этом состоянии; под курсором — крестик «убрать».
        let tail = RECT { left: row.right - p.s(60), right: row.right - p.s(10), ..row };
        if hov {
            p.text(p.f.icon, I_DEL, tail, FG, DT_RIGHT);
        } else {
            p.text(p.f.small, &fmt_age(now.duration_since(a.since)), tail, DIM, DT_RIGHT);
        }
        p.hit(row, hit);
    }

    // Уведомления: сколько влезет до низа панели.
    let list: Vec<Notif> = notifs::INBOX.lock().unwrap().list.clone();
    let off = notifs::OFF.load(std::sync::atomic::Ordering::Relaxed);
    let count = if list.is_empty() { String::new() } else { list.len().to_string() };
    y = section(p, w, y + 4, "Уведомления", &count);
    if list.is_empty() {
        let msg = if off { "Нет доступа: Параметры → Конфиденциальность → Уведомления" } else { "Новых нет" };
        p.text(p.f.small, msg, RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + 24) }, DIM, DT_LEFT);
        return;
    }
    let now_ft = crate::sys::toasts::now_filetime();
    for n in &list {
        if p.s(y + NOTIF_H) > h - p.s(12) {
            break;
        }
        let row = RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + NOTIF_H - 2) };
        y += NOTIF_H;
        let hit = Hit::NotifDrop(n.id);
        let hov = p.hovered(hit);
        if hov {
            p.fill(row, R_ROW, HOVER);
        }
        // Программа, заголовок, текст.
        let mut x = row.left + p.s(12);
        let aw = p.measure(p.f.small, &n.app).min(p.s(110));
        p.text(p.f.small, &n.app, RECT { left: x, right: x + aw, ..row }, ACCENT, DT_LEFT | DT_END_ELLIPSIS);
        x += aw + p.s(10);
        let tw = p.measure(p.f.normal, &n.title).min(p.s(170));
        p.text(p.f.normal, &n.title, RECT { left: x, right: x + tw, ..row }, FG, DT_LEFT | DT_END_ELLIPSIS);
        x += tw + p.s(10);
        let right = row.right - p.s(64);
        if right > x {
            p.text(p.f.small, &n.body, RECT { left: x, right, ..row }, DIM, DT_LEFT | DT_END_ELLIPSIS);
        }
        let tail = RECT { left: row.right - p.s(60), right: row.right - p.s(10), ..row };
        if hov {
            p.text(p.f.icon, I_DEL, tail, FG, DT_RIGHT);
        } else {
            p.text(p.f.small, &fmt_age(notifs::age(n.created, now_ft)), tail, DIM, DT_RIGHT);
        }
        p.hit(row, hit);
    }
}
