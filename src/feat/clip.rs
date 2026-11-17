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
    /// Когда скопировано, секунды Unix. 0 — неизвестно (запись из файла старого формата).
    pub time: u64,
}

/// Что за запись, по самому тексту: от этого зависит значок в списке.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Text,
    Link,
    Path,
    Color(u8, u8, u8),
}

pub fn kind(text: &str) -> Kind {
    let s = text.trim();
    if s.is_empty() || s.contains(char::is_whitespace) {
        return Kind::Text;
    }
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            let v = u32::from_str_radix(hex, 16).unwrap_or(0);
            return Kind::Color((v >> 16) as u8, (v >> 8) as u8, v as u8);
        }
    }
    let low = s.to_ascii_lowercase();
    if low.starts_with("http://") || low.starts_with("https://") || low.starts_with("www.") {
        return Kind::Link;
    }
    let b = s.as_bytes();
    if s.starts_with("\\\\") || (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && (b[2] == b'\\' || b[2] == b'/')) {
        return Kind::Path;
    }
    // «github.com/user/repo»: домен с точкой, потом путь.
    if let Some((host, _)) = s.split_once('/') {
        if host.contains('.') && host.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'-') {
            return Kind::Link;
        }
    }
    Kind::Text
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// «сейчас», «5 мин», «2 ч», «3 д». Пусто, если время записи неизвестно.
pub fn age_text(time: u64, now: u64) -> String {
    if time == 0 {
        return String::new();
    }
    let s = now.saturating_sub(time);
    match s {
        0..=59 => "сейчас".into(),
        60..=3599 => format!("{} мин", s / 60),
        3600..=86_399 => format!("{} ч", s / 3600),
        _ => format!("{} д", s / 86_400),
    }
}

/// Строка файла истории → запись. Новый формат: флаг, время, текст; в старом времени нет.
/// Табы внутри текста экранированы, поэтому лишний таб в строке — это всегда разделитель.
fn parse_line(line: &str) -> Option<Entry> {
    let (flag, rest) = line.split_once('\t')?;
    let (time, text) = match rest.split_once('\t') {
        Some((t, text)) if !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()) => (t.parse().unwrap_or(0), text),
        _ => (0, rest),
    };
    Some(Entry { text: unescape(text), pinned: flag == "P", time })
}

pub struct History {
    pub items: Vec<Entry>,
    path: Option<PathBuf>,
    /// Писать ли `clipboard.txt` (`!settings.clip_memory`). Выключено — история живёт только в памяти.
    pub disk: bool,
}

fn file_path() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|a| PathBuf::from(a).join("Pill").join("clipboard.txt"))
}

/// Удалить `clipboard.txt` (включили «не хранить историю на диске»). Звать вне `st()`.
pub fn remove_file() {
    if let Some(p) = file_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// Записать готовый текст истории (`History::serialize`) в `clipboard.txt`. Звать вне `st()`.
pub fn write_file(text: &str) {
    let Some(path) = file_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, text);
}

impl History {
    /// `disk = false`: файл не читается, а оставшийся от прошлого раза удаляется
    /// (настройку могли включить правкой `settings.txt`). Зовётся до `st()`, при старте.
    pub fn load(disk: bool) -> Self {
        let path = file_path();
        let mut items = Vec::new();
        if !disk {
            remove_file();
        } else if let Some(data) = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            items.extend(data.lines().filter_map(parse_line));
        }
        History { items, path, disk }
    }

    fn save(&self) {
        if !self.disk || self.path.is_none() {
            return;
        }
        write_file(&self.serialize());
    }

    /// Текст файла истории: строка на запись.
    pub fn serialize(&self) -> String {
        let mut out = String::new();
        for e in &self.items {
            out.push_str(if e.pinned { "P\t" } else { "-\t" });
            out.push_str(&e.time.to_string());
            out.push('\t');
            out.push_str(&escape(&e.text));
            out.push('\n');
        }
        out
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
        self.items.insert(0, Entry { text, pinned, time: now_secs() });
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
    Pin(usize),
    Del(usize),
}

pub unsafe fn on_hit(_hwnd: HWND, h: ClipHit) -> Redraw {
    match h {
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

/// Новое содержимое буфера Windows (WM_CLIPBOARDUPDATE). Some(true) — вкладка открыта, надо перерисовать;
/// None — в буфере не текст (или его просили не сохранять): может быть картинка.
pub unsafe fn on_update(hwnd: HWND) -> Option<bool> {
    let text = crate::sys::clipboard::read_text(hwnd)?;
    Some(st(|s| {
        s.clip.add(text);
        s.shell.expanded && s.shell.tab == crate::app::Tab::Commands
    }))
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
    fn file_formats_and_kinds() {
        // Старая строка без времени и новая со временем; таб в тексте экранирован.
        let old = parse_line("P\tпривет\\tмир").unwrap();
        assert_eq!((old.text.as_str(), old.pinned, old.time), ("привет\tмир", true, 0));
        let new = parse_line("-\t1700000000\t12345").unwrap();
        assert_eq!((new.text.as_str(), new.pinned, new.time), ("12345", false, 1_700_000_000));
        // Старая запись из одних цифр не принимается за время.
        assert_eq!(parse_line("-\t12345").unwrap().text, "12345");

        assert_eq!(kind("https://example.com/a"), Kind::Link);
        assert_eq!(kind("github.com/user/repo"), Kind::Link);
        assert_eq!(kind("#0A84FF"), Kind::Color(0x0A, 0x84, 0xFF));
        assert_eq!(kind("C:\\Users\\me\\file.txt"), Kind::Path);
        assert_eq!(kind("cargo build --release"), Kind::Text);
        assert_eq!((age_text(0, 100), age_text(100, 130), age_text(100, 100 + 7200)), (String::new(), "сейчас".into(), "2 ч".into()));
    }

    #[test]
    fn history_order() {
        let mut h = History { items: Vec::new(), path: None, disk: false };
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
        // То, что пишется в файл при возврате истории на диск, читается обратно.
        let back: Vec<Entry> = h.serialize().lines().filter_map(parse_line).collect();
        assert_eq!(back.len(), h.items.len());
        assert!(back.iter().zip(&h.items).all(|(a, b)| a.text == b.text && a.pinned == b.pinned && a.time == b.time));
    }
}
