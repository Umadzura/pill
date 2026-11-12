//! Буфер обмена: история текстовых записей, закрепление, файл в %APPDATA%\Pill, клики по строкам.
//! Вместо SQLite простой текстовый файл: под GNU-тулчейном без C-компилятора так надёжнее и легче.
//! Чтение и запись самого буфера Windows лежат в `sys::clipboard`.

use std::path::PathBuf;

use windows::Win32::Foundation::HWND;

use crate::app::{st, Redraw};

const MAX_UNPINNED: usize = 100;
const MAX_LEN: usize = 20_000;

pub struct Entry {
    pub text: String,
    pub pinned: bool,
}

pub struct History {
    pub items: Vec<Entry>,
    path: Option<PathBuf>,
}

impl History {
    pub fn load() -> Self {
        let path = std::env::var("APPDATA").ok().map(|a| PathBuf::from(a).join("Pill").join("clipboard.txt"));
        let mut items = Vec::new();
        if let Some(data) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            for line in data.lines() {
                let Some((flag, text)) = line.split_once('\t') else { continue };
                items.push(Entry { text: unescape(text), pinned: flag == "P" });
            }
        }
        History { items, path }
    }

    fn save(&self) {
        let Some(path) = &self.path else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let mut out = String::new();
        for e in &self.items {
            out.push_str(if e.pinned { "P\t" } else { "-\t" });
            out.push_str(&escape(&e.text));
            out.push('\n');
        }
        let _ = std::fs::write(path, out);
    }

    /// Новая запись наверх. Повтор поднимается наверх с сохранением закрепления.
    pub fn add(&mut self, mut text: String) {
        if text.trim().is_empty() {
            return;
        }
        if text.len() > MAX_LEN {
            let mut cut = MAX_LEN;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            text.truncate(cut);
        }
        if self.items.first().map(|e| e.text == text).unwrap_or(false) {
            return;
        }
        let mut pinned = false;
        if let Some(pos) = self.items.iter().position(|e| e.text == text) {
            pinned = self.items.remove(pos).pinned;
        }
        self.items.insert(0, Entry { text, pinned });
        let mut unpinned = 0;
        self.items.retain(|e| {
            if e.pinned {
                return true;
            }
            unpinned += 1;
            unpinned <= MAX_UNPINNED
        });
        self.save();
    }

    pub fn toggle_pin(&mut self, i: usize) {
        if let Some(e) = self.items.get_mut(i) {
            e.pinned = !e.pinned;
            self.save();
        }
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.items.len() {
            self.items.remove(i);
            self.save();
        }
    }

    /// Индексы для показа: сначала закреплённые, потом свежие; с фильтром по подстроке.
    pub fn view(&self, q: &str) -> Vec<usize> {
        let q = q.trim().to_lowercase();
        let ok = |e: &Entry| q.is_empty() || e.text.to_lowercase().contains(&q);
        let pinned = self.items.iter().enumerate().filter(|(_, e)| e.pinned && ok(e)).map(|x| x.0);
        let rest = self.items.iter().enumerate().filter(|(_, e)| !e.pinned && ok(e)).map(|x| x.0);
        pinned.chain(rest).collect()
    }
}

/// Зоны клика вкладки «Буфер». Индекс — в `History::items`, а не номер строки на экране.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ClipHit {
    Row(usize),
    Pin(usize),
    Del(usize),
}

pub unsafe fn on_hit(hwnd: HWND, h: ClipHit) -> Redraw {
    match h {
        ClipHit::Row(i) => {
            // Запись могла пропасть между отрисовкой и кликом: тогда молча выходим.
            let text = st(|s| s.clip.items.get(i).map(|e| e.text.clone()));
            if let Some(t) = text {
                crate::input::paste(hwnd, &t);
            }
            Redraw::None
        }
        ClipHit::Pin(i) => {
            st(|s| s.clip.toggle_pin(i));
            Redraw::Content
        }
        ClipHit::Del(i) => {
            st(|s| s.clip.remove(i));
            Redraw::Content
        }
    }
}

/// Новое содержимое буфера Windows (WM_CLIPBOARDUPDATE). true — вкладка открыта, надо перерисовать.
pub unsafe fn on_update(hwnd: HWND) -> bool {
    let Some(text) = crate::sys::clipboard::read_text(hwnd) else { return false };
    st(|s| {
        s.clip.add(text);
        s.shell.expanded && s.shell.tab == crate::app::Tab::Clipboard
    })
}

fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            c => o.push(c),
        }
    }
    o
}

fn unescape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            o.push(c);
            continue;
        }
        match it.next() {
            Some('n') => o.push('\n'),
            Some('r') => o.push('\r'),
            Some('t') => o.push('\t'),
            Some(c) => o.push(c),
            None => {}
        }
    }
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_roundtrip() {
        let s = "a\\b\nc\td\r";
        assert_eq!(unescape(&escape(s)), s);
    }

    #[test]
    fn history_order() {
        let mut h = History { items: Vec::new(), path: None };
        h.add("one".into());
        h.add("two".into());
        h.toggle_pin(1); // one закреплён
        h.add("one".into());
        assert_eq!(h.items[0].text, "one");
        assert!(h.items[0].pinned);
        h.add("three".into());
        let v = h.view("");
        assert_eq!(h.items[v[0]].text, "one");
        assert_eq!(h.view("thr").len(), 1);
    }
}
