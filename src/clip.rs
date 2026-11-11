//! История буфера обмена: текстовые записи, закрепление, файл в %APPDATA%\Pill.
//! Вместо SQLite простой текстовый файл: под GNU-тулчейном без C-компилятора так надёжнее и легче.

use std::path::PathBuf;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::Sleep;

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

unsafe fn open(hwnd: HWND) -> bool {
    // Буфер может быть занят другой программой: пара коротких попыток.
    for _ in 0..4 {
        if OpenClipboard(hwnd).is_ok() {
            return true;
        }
        Sleep(15);
    }
    false
}

/// Текст из буфера. None, если там не текст или менеджер паролей попросил не сохранять.
pub unsafe fn read_text(hwnd: HWND) -> Option<String> {
    let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
    if exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok() {
        return None;
    }
    if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() || !open(hwnd) {
        return None;
    }
    let mut result = None;
    if let Ok(h) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
        let hg = HGLOBAL(h.0);
        let p = GlobalLock(hg) as *const u16;
        if !p.is_null() {
            let max = GlobalSize(hg) / 2;
            let mut n = 0;
            while n < max && *p.add(n) != 0 {
                n += 1;
            }
            result = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
            let _ = GlobalUnlock(hg);
        }
    }
    let _ = CloseClipboard();
    result
}

pub unsafe fn set_text(hwnd: HWND, text: &str) -> bool {
    let w: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    if !open(hwnd) {
        return false;
    }
    let mut ok = false;
    if EmptyClipboard().is_ok() {
        if let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(hg) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(hg);
                ok = SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(hg.0)).is_ok();
            }
            if !ok {
                let _ = GlobalFree(hg);
            }
        }
    }
    let _ = CloseClipboard();
    ok
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
