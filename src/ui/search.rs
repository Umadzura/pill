//! Строка поиска (Alt+Space, Ctrl+Alt+V, лупа в шапке): программы, буфер и действия в одном списке.
//! Режим «Буфер» — список слева и полный текст выбранной записи справа.

use std::time::Instant;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State, Tab};
use crate::feat::agents::one_line;
use crate::feat::clip::{age_text, kind, now_secs, ClipHit, Entry, Kind};
use crate::feat::cmd::{fmt_dur, frequent, results, CmdRes, MODES, MODE_ALL, MODE_APPS, MODE_CLIP};

/// Сколько записей буфера видно сразу (нужно и для прокрутки стрелками).
pub const CLIP_ROWS: usize = 7;
const TOP: i32 = 94;
const ROW: i32 = 28;
const CALC_H: i32 = 44;
const LABEL_H: i32 = 18;
/// Правая граница списка в режиме «Буфер» и левая граница предпросмотра.
const SPLIT: i32 = 298;

const I_LINK: &str = "\u{E71B}";
const I_TEXT: &str = "\u{E8A5}";
const I_FOLDER: &str = "\u{E8B7}";

/// Клавиша в рамке и подпись после неё. Возвращает x для следующей.
unsafe fn key(p: &Painter, x: i32, top: i32, k: &str, label: &str) -> i32 {
    let kw = p.measure(p.f.small, k) + p.s(10);
    let r = RECT { left: x, top, right: x + kw, bottom: top + p.s(18) };
    p.fill(r, 8, PANEL);
    p.text(p.f.small, k, r, FG, DT_CENTER);
    let lw = p.measure(p.f.small, label);
    p.text(p.f.small, label, RECT { left: r.right + p.s(5), right: r.right + p.s(5) + lw, ..r }, DIM, DT_LEFT);
    r.right + p.s(5) + lw + p.s(14)
}

/// Строка подсказок по клавишам внизу панели.
unsafe fn hints(p: &Painter, h: i32, items: &[(&str, &str)]) {
    let mut x = p.s(20);
    for (k, label) in items {
        x = key(p, x, h - p.s(28), k, label);
    }
}

/// Шапка поиска вместо вкладок: режимы, подсказка про Tab и крестик «назад к вкладкам».
pub unsafe fn header(p: &mut Painter, s: &State, w: i32) {
    let mut left = p.s(16);
    for (i, name) in MODES.iter().enumerate() {
        let r = RECT { left, top: p.s(14), right: left + p.measure(p.f.bold, name) + p.s(24), bottom: p.s(44) };
        left = r.right + p.s(4);
        let h = Hit::Mode(i as u8);
        let on = s.input.mode == i as u8;
        if on {
            p.fill(r, R_PILL, SELECTED_TAB);
        } else if p.hovered(h) {
            p.fill(r, R_PILL, PANEL);
        }
        p.text(p.f.bold, name, r, if on { FG } else { DIM }, DT_CENTER);
        p.hit(r, h);
    }
    let close = RECT { left: w - p.s(50), top: p.s(14), right: w - p.s(20), bottom: p.s(44) };
    p.icon_button(close, I_DEL, Hit::Tab(s.input.back.unwrap_or(Tab::Music)), false);
    let width = p.measure(p.f.small, "Tab") + p.s(10) + p.s(5) + p.measure(p.f.small, "режим");
    key(p, close.left - p.s(10) - width, p.s(20), "Tab", "режим");
}

/// Подпись группы. Возвращает y под ней.
unsafe fn label(p: &Painter, right: i32, y: i32, title: &str, note: &str) -> i32 {
    let r = RECT { left: p.s(22), top: p.s(y), right, bottom: p.s(y + LABEL_H) };
    p.text(p.f.small, title, r, DIM, DT_LEFT);
    p.text(p.f.small, note, r, DIM, DT_RIGHT);
    y + LABEL_H
}

/// Значок записи буфера слева в строке: булавка у закреплённой, иначе тип (ссылка, путь, цвет, текст).
unsafe fn clip_icon(p: &Painter, row: RECT, e: &Entry, pin: bool) {
    let r = RECT { left: row.left + p.s(10), right: row.left + p.s(34), ..row };
    if pin && e.pinned {
        p.text(p.f.icon, I_PINNED, r, ACCENT, DT_LEFT);
        return;
    }
    match kind(&e.text) {
        Kind::Color(cr, cg, cb) => {
            let (d, mid) = (p.s(14), (row.top + row.bottom) / 2);
            p.fill(RECT { left: r.left, top: mid - d / 2, right: r.left + d, bottom: mid - d / 2 + d }, 8, rgb(cr, cg, cb));
        }
        Kind::Link => p.text(p.f.icon, I_LINK, r, DIM, DT_LEFT),
        Kind::Path => p.text(p.f.icon, I_FOLDER, r, DIM, DT_LEFT),
        Kind::Text => p.text(p.f.icon, I_TEXT, r, DIM, DT_LEFT),
    }
}

/// Пустая строка в режиме «Всё»: плитки программ, которые запускают чаще всего. Возвращает y под ними.
unsafe fn frequent_tiles(p: &mut Painter, s: &State, w: i32, mut y: i32) -> i32 {
    let (left, right) = (p.s(20), w - p.s(20));
    y = label(p, right, y, "Часто запускаю", "");
    let top = frequent(&s.cmd.apps, &s.cmd.freq, 7);
    if top.is_empty() {
        let r = RECT { left: p.s(22), top: p.s(y), right, bottom: p.s(y + 24) };
        p.text(p.f.small, "Здесь появятся программы, которые ты запускаешь чаще всего", r, DIM, DT_LEFT);
        return y + 30;
    }
    let gap = p.s(6);
    let tw = (right - left - 6 * gap) / 7;
    for (n, &i) in top.iter().enumerate() {
        let x = left + n as i32 * (tw + gap);
        let cell = RECT { left: x, top: p.s(y), right: x + tw, bottom: p.s(y + 66) };
        let hit = Hit::AppTile(i);
        let side = p.s(44);
        let ic = RECT { left: x + (tw - side) / 2, top: cell.top, right: x + (tw - side) / 2 + side, bottom: cell.top + side };
        p.fill(ic, 24, if p.hovered(hit) { HOVER } else { PANEL });
        let name = &s.cmd.apps[i].name;
        let letter: String = name.chars().find(|c| c.is_alphanumeric()).map(|c| c.to_uppercase().collect()).unwrap_or_default();
        p.text(p.f.title, &letter, ic, super::mascot::body_color(name), DT_CENTER);
        p.text(p.f.small, name, RECT { top: p.s(y + 48), ..cell }, DIM, DT_CENTER | DT_END_ELLIPSIS);
        p.hit(cell, hit);
    }
    y + 74
}

/// Режимы «Всё», «Программы», «Действия»: один список с подписями групп.
unsafe fn list_mode(p: &mut Painter, s: &State, w: i32, h: i32, res: &[CmdRes]) {
    let q_empty = s.input.text.trim().is_empty();
    let (left, right) = (p.s(20), w - p.s(20));
    let mut y = TOP;
    if q_empty && s.input.mode == MODE_ALL {
        y = frequent_tiles(p, s, w, y);
    }
    if res.is_empty() {
        let msg = match (q_empty, s.input.mode) {
            (false, _) => "Ничего не найдено",
            (true, MODE_APPS) => "Начни печатать название программы",
            (true, MODE_ALL) => "",
            (true, _) => "Расчёт: 2+2*3, 15%*200 · Таймер: 5м, 1ч30м · Команда: > ipconfig",
        };
        p.text(p.f.small, msg, RECT { left: p.s(22), top: p.s(y + 4), right, bottom: p.s(y + 28) }, DIM, DT_LEFT);
        return;
    }
    let now = Instant::now();
    let mut group = "";
    for (i, r) in res.iter().enumerate() {
        let g = match r {
            CmdRes::App(_) => "Программы",
            CmdRes::Clip(_) if q_empty => "Закреплённое из буфера",
            CmdRes::Clip(_) => "Буфер",
            CmdRes::Active(_) => "Сейчас идёт",
            _ => "",
        };
        let new_group = !g.is_empty() && g != group;
        let rh = if matches!(r, CmdRes::Calc(_)) { CALC_H } else { ROW };
        if p.s(y + rh + if new_group { LABEL_H } else { 0 }) > h - p.s(32) {
            break;
        }
        if new_group {
            group = g;
            y = label(p, right, y + 2, g, "");
        }
        let row = RECT { left, top: p.s(y), right, bottom: p.s(y + rh) };
        y += rh;
        let hit = Hit::Cmd(i);
        let sel = p.hovered(hit) || (s.input.active && s.input.sel == i);
        if let CmdRes::Calc(v) = r {
            // Результат калькулятора крупной строкой.
            let card = RECT { bottom: row.bottom - p.s(4), ..row };
            p.fill(card, R_TILE, if sel { HOVER } else { PANEL });
            p.text(p.f.icon_big, I_CALC, RECT { left: card.left + p.s(12), right: card.left + p.s(44), ..card }, ACCENT, DT_LEFT);
            p.text(p.f.title, v, RECT { left: card.left + p.s(46), right: card.right - p.s(150), ..card }, FG, DT_LEFT | DT_END_ELLIPSIS);
            p.text(p.f.small, "Enter: скопировать", RECT { right: card.right - p.s(12), ..card }, DIM, DT_RIGHT);
            p.hit(row, hit);
            continue;
        }
        if sel {
            p.fill(row, R_ROW, HOVER);
        }
        let ir = RECT { left: row.left + p.s(10), right: row.left + p.s(34), ..row };
        let (text, color) = match r {
            CmdRes::Timer(t) => {
                p.text(p.f.icon, I_TIMER, ir, DIM, DT_LEFT);
                (format!("Таймер на {}", fmt_dur(*t)), FG)
            }
            CmdRes::App(a) => {
                p.text(p.f.icon, I_APP, ir, DIM, DT_LEFT);
                (s.cmd.apps[*a].name.clone(), FG)
            }
            CmdRes::Run(q) => {
                p.text(p.f.icon, I_RUN, ir, DIM, DT_LEFT);
                (format!("Выполнить: {}", q), DIM)
            }
            CmdRes::Active(t) => {
                p.text(p.f.icon, I_TIMER, ir, DIM, DT_LEFT);
                let tm = &s.timers.list[*t];
                let rest = tm.end.saturating_duration_since(now).as_secs();
                (format!("Таймер: осталось {} из {} · Enter отменяет", fmt_dur(rest), fmt_dur(tm.total)), FG)
            }
            CmdRes::Clip(c) => {
                let e = &s.clip.items[*c];
                clip_icon(p, row, e, true);
                (one_line(&e.text, 200), FG)
            }
            CmdRes::Calc(_) => (String::new(), FG),
        };
        p.text(p.f.normal, &text, RECT { left: row.left + p.s(38), right: row.right - p.s(64), ..row }, color, DT_LEFT | DT_END_ELLIPSIS);
        if i < 9 {
            p.text(p.f.small, &format!("Alt {}", i + 1), RECT { right: row.right - p.s(10), ..row }, DIM, DT_RIGHT);
        }
        p.hit(row, hit);
    }
}

/// Режим «Буфер»: слева список с прокруткой, справа полный текст выбранной записи.
unsafe fn clip_mode(p: &mut Painter, s: &State, w: i32, h: i32, res: &[CmdRes]) {
    let left = p.s(20);
    if res.is_empty() {
        let msg = if s.clip.items.is_empty() { "История пуста — скопируй что-нибудь" } else { "Ничего не найдено" };
        p.text(p.f.small, msg, RECT { left: p.s(22), top: p.s(TOP + 4), right: w - p.s(20), bottom: p.s(TOP + 28) }, DIM, DT_LEFT);
        return;
    }
    let scroll = s.input.scroll.min(res.len() - 1);
    let now = now_secs();
    let (mut y, mut section) = (TOP, 0u8);
    for (n, r) in res.iter().enumerate().skip(scroll).take(CLIP_ROWS) {
        let CmdRes::Clip(idx) = r else { continue };
        let e = &s.clip.items[*idx];
        let sec = if e.pinned { 1 } else { 2 };
        if sec != section {
            section = sec;
            y = label(p, p.s(SPLIT), y, if e.pinned { "Закреплённые" } else { "Недавние" }, "");
        }
        let row = RECT { left, top: p.s(y), right: p.s(SPLIT), bottom: p.s(y + ROW) };
        y += ROW;
        let (h_row, h_pin, h_del) = (Hit::Cmd(n), Hit::Clip(ClipHit::Pin(*idx)), Hit::Clip(ClipHit::Del(*idx)));
        let hov = p.hovered(h_row) || p.hovered(h_pin) || p.hovered(h_del);
        if hov || s.input.sel == n {
            p.fill(row, R_ROW, HOVER);
        }
        p.hit(row, h_row);
        clip_icon(p, row, e, true);
        let age = age_text(e.time, now);
        // Под курсором вместо времени — закрепить и убрать.
        let tail = if hov { p.s(60) } else if age.is_empty() { p.s(10) } else { p.measure(p.f.small, &age) + p.s(18) };
        p.text(p.f.normal, &one_line(&e.text, 200), RECT { left: row.left + p.s(36), right: row.right - tail, ..row }, FG, DT_LEFT | DT_END_ELLIPSIS);
        if hov {
            let pin = RECT { left: row.right - p.s(56), right: row.right - p.s(30), ..row };
            let del = RECT { left: row.right - p.s(28), right: row.right - p.s(4), ..row };
            let c = if e.pinned { ACCENT } else if p.hovered(h_pin) { FG } else { DIM };
            p.text(p.f.icon, if e.pinned { I_PINNED } else { I_PIN }, pin, c, DT_CENTER);
            p.text(p.f.icon, I_DEL, del, if p.hovered(h_del) { FG } else { DIM }, DT_CENTER);
            p.hit(pin, h_pin);
            p.hit(del, h_del);
        } else {
            p.text(p.f.small, &age, RECT { right: row.right - p.s(10), ..row }, DIM, DT_RIGHT);
        }
    }
    if res.len() > scroll + CLIP_ROWS {
        let more = format!("ещё {} ↓", res.len() - scroll - CLIP_ROWS);
        p.text(p.f.small, &more, RECT { left, top: p.s(y + 2), right: p.s(SPLIT) - p.s(10), bottom: p.s(y + 20) }, DIM, DT_RIGHT);
    }

    // Предпросмотр: запись под курсором, иначе выбранная стрелками.
    let cur = match s.hover {
        Some(Hit::Cmd(n)) => n,
        _ => s.input.sel,
    };
    let Some(CmdRes::Clip(idx)) = res.get(cur.min(res.len() - 1)) else { return };
    let e = &s.clip.items[*idx];
    let panel = RECT { left: p.s(SPLIT + 12), top: p.s(TOP), right: w - p.s(20), bottom: h - p.s(40) };
    p.fill(panel, R_TILE, PANEL);
    let inner = RECT { left: panel.left + p.s(12), top: panel.top + p.s(10), right: panel.right - p.s(12), bottom: panel.bottom - p.s(48) };
    // Длинную запись целиком не рисуем: в окошко влезает несколько сотен символов.
    let shown: String = e.text.chars().take(1200).collect();
    p.text_wrap(p.f.normal, &shown, inner, FG);
    let what = match kind(&e.text) {
        Kind::Link => "Ссылка",
        Kind::Path => "Путь",
        Kind::Color(..) => "Цвет",
        Kind::Text => "Текст",
    };
    let meta = format!("{} · {} симв. · {} стр.", what, e.text.chars().count(), e.text.lines().count().max(1));
    let m1 = RECT { left: inner.left, top: panel.bottom - p.s(44), right: inner.right, bottom: panel.bottom - p.s(26) };
    p.text(p.f.small, &meta, m1, DIM, DT_LEFT | DT_END_ELLIPSIS);
    // У записей, сохранённых до появления времени, его нет.
    if e.time != 0 {
        let when = format!("Скопировано: {}", age_text(e.time, now));
        p.text(p.f.small, &when, RECT { top: m1.bottom, bottom: m1.bottom + p.s(18), ..m1 }, DIM, DT_LEFT);
    }
}

pub unsafe fn draw(p: &mut Painter, s: &State, w: i32, h: i32) {
    let placeholder = match s.input.mode {
        MODE_CLIP => "Поиск в истории буфера",
        MODE_APPS => "Название программы",
        MODE_ALL => "Программа, расчёт, таймер или текст из буфера",
        _ => "Расчёт, таймер или > команда",
    };
    p.input(w, s, placeholder);
    let res = results(s);
    if s.input.mode == MODE_CLIP {
        clip_mode(p, s, w, h, &res);
        hints(p, h, &[("Enter", "вставить"), ("Ctrl P", "закрепить"), ("Del", "убрать"), ("↑↓", "выбор"), ("Esc", "закрыть")]);
    } else {
        list_mode(p, s, w, h, &res);
        hints(p, h, &[("Enter", "открыть"), ("Alt 1–9", "по номеру"), ("=", "расчёт"), (">", "команда"), ("5м", "таймер"), ("Esc", "закрыть")]);
    }
}
