//! Командная строка: поиск программ из меню Пуск, калькулятор, разбор таймеров, запуск выбранного.
//! Разбор и ранжирование — чистые функции с тестами, запуск — через `sys::launch`.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;

use crate::app::{st, State};
use crate::input::close_panel;
use crate::sys;

/// Ярлыки, найденные при последнем открытии командной строки.
#[derive(Default)]
pub struct CmdState {
    pub apps: Vec<App>,
}

/// Строка результата в командной строке.
#[derive(Clone, PartialEq)]
pub enum CmdRes {
    Calc(String),
    Timer(u64),
    App(usize),
    Run(String),
    /// Уже запущенный таймер (индекс в `timers.list`): клик отменяет.
    Active(usize),
}

pub fn results(s: &State) -> Vec<CmdRes> {
    let q = s.input.text.trim();
    let mut out = Vec::new();
    if q.is_empty() {
        out.extend((0..s.timers.list.len()).map(CmdRes::Active));
        return out;
    }
    if let Some(v) = calc(q) {
        out.push(CmdRes::Calc(fmt_num(v)));
    }
    if let Some(t) = parse_timer(q) {
        out.push(CmdRes::Timer(t));
    }
    if !q.starts_with('>') {
        out.extend(search(&s.cmd.apps, q, 6).into_iter().map(CmdRes::App));
    }
    out.push(CmdRes::Run(q.to_string()));
    out
}

/// Выполнить строку номер `i` из текущих результатов.
pub unsafe fn run_index(hwnd: HWND, i: usize) {
    let res = st(|s| results(s).get(i).cloned());
    if let Some(r) = res {
        run(hwnd, r);
    }
}

unsafe fn run(hwnd: HWND, res: CmdRes) {
    match res {
        CmdRes::Calc(v) => {
            sys::clipboard::set_text(hwnd, &v);
            close_panel(hwnd, true);
        }
        CmdRes::Timer(secs) => {
            st(|s| {
                s.timers.add(secs);
                s.input.text.clear();
                s.input.sel = 0;
            });
            close_panel(hwnd, true);
        }
        CmdRes::App(i) => {
            let path = st(|s| s.cmd.apps.get(i).map(|a| a.path.to_string_lossy().into_owned()));
            close_panel(hwnd, false);
            if let Some(p) = path {
                sys::launch::open(&p, None);
            }
        }
        CmdRes::Run(line) => {
            close_panel(hwnd, false);
            sys::launch::run_line(&line);
        }
        CmdRes::Active(i) => {
            st(|s| s.timers.cancel(i));
            crate::shell::layout(hwnd);
        }
    }
}

pub struct App {
    pub name: String,
    lower: String,
    pub path: PathBuf,
}

/// Ярлыки из меню Пуск (пользовательское + общее). Занимает миллисекунды, зовём при открытии.
pub fn scan() -> Vec<App> {
    let mut out = Vec::new();
    for var in ["APPDATA", "ProgramData"] {
        if let Ok(base) = std::env::var(var) {
            walk(&Path::new(&base).join(r"Microsoft\Windows\Start Menu\Programs"), &mut out, 0);
        }
    }
    out.sort_by(|a, b| a.lower.cmp(&b.lower));
    out.dedup_by(|a, b| a.lower == b.lower);
    out
}

fn walk(dir: &Path, out: &mut Vec<App>, depth: u32) {
    if depth > 6 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let path = e.path();
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            walk(&path, out, depth + 1);
            continue;
        }
        let ext = path.extension().and_then(|x| x.to_str()).unwrap_or("").to_ascii_lowercase();
        if !matches!(ext.as_str(), "lnk" | "url" | "appref-ms") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|x| x.to_str()) else { continue };
        let lower = name.to_lowercase();
        if lower.contains("uninstall") || lower.contains("удал") || lower.contains("readme") {
            continue;
        }
        out.push(App { name: name.to_string(), lower, path });
    }
}

/// Индексы подходящих программ, лучшие первыми.
pub fn search(apps: &[App], q: &str, limit: usize) -> Vec<usize> {
    let q = q.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(u32, usize, usize)> = apps
        .iter()
        .enumerate()
        .filter_map(|(i, a)| score(&a.lower, &q).map(|s| (s, a.lower.len(), i)))
        .collect();
    scored.sort();
    scored.into_iter().take(limit).map(|x| x.2).collect()
}

fn score(name: &str, q: &str) -> Option<u32> {
    if name.starts_with(q) {
        return Some(0);
    }
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| w.starts_with(q)) {
        return Some(1);
    }
    let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();
    if initials.starts_with(q) {
        return Some(1);
    }
    if name.contains(q) {
        return Some(2);
    }
    // Буквы запроса идут по порядку (vscode -> visual studio code).
    let mut it = name.chars();
    if q.chars().filter(|c| !c.is_whitespace()).all(|c| it.any(|n| n == c)) {
        return Some(3);
    }
    None
}

// ---------- калькулятор ----------

/// Считает выражение, если строка на него похожа (цифры и хотя бы один оператор).
pub fn calc(q: &str) -> Option<f64> {
    let s: String = q
        .trim()
        .trim_start_matches('=')
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| match c {
            ',' => '.',
            '×' | 'x' | 'х' => '*',
            '÷' | ':' => '/',
            c => c,
        })
        .collect();
    if s.is_empty()
        || !s.chars().any(|c| c.is_ascii_digit())
        || !s.chars().any(|c| "+-*/^%()".contains(c))
        || !s.chars().all(|c| c.is_ascii_digit() || ".+-*/^%()".contains(c))
    {
        return None;
    }
    let mut p = Parser { b: s.as_bytes(), i: 0 };
    let v = p.expr()?;
    (p.i == p.b.len() && v.is_finite()).then_some(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }
    fn expr(&mut self) -> Option<f64> {
        let mut v = self.term()?;
        while let Some(c @ (b'+' | b'-')) = self.peek() {
            self.i += 1;
            let r = self.term()?;
            v = if c == b'+' { v + r } else { v - r };
        }
        Some(v)
    }
    fn term(&mut self) -> Option<f64> {
        let mut v = self.unary()?;
        while let Some(c @ (b'*' | b'/')) = self.peek() {
            self.i += 1;
            let r = self.unary()?;
            v = if c == b'*' { v * r } else { v / r };
        }
        Some(v)
    }
    fn unary(&mut self) -> Option<f64> {
        match self.peek() {
            Some(b'-') => {
                self.i += 1;
                Some(-self.unary()?)
            }
            Some(b'+') => {
                self.i += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }
    fn power(&mut self) -> Option<f64> {
        let base = self.primary()?;
        if self.peek() == Some(b'^') {
            self.i += 1;
            return Some(base.powf(self.unary()?));
        }
        Some(base)
    }
    fn primary(&mut self) -> Option<f64> {
        let mut v = if self.peek() == Some(b'(') {
            self.i += 1;
            let v = self.expr()?;
            if self.peek() != Some(b')') {
                return None;
            }
            self.i += 1;
            v
        } else {
            let start = self.i;
            while matches!(self.peek(), Some(b'0'..=b'9' | b'.')) {
                self.i += 1;
            }
            std::str::from_utf8(&self.b[start..self.i]).ok()?.parse().ok()?
        };
        // 15% = 0.15
        while self.peek() == Some(b'%') {
            self.i += 1;
            v /= 100.0;
        }
        Some(v)
    }
}

pub fn fmt_num(v: f64) -> String {
    if (v - v.round()).abs() < 1e-9 && v.abs() < 1e15 {
        return format!("{}", v.round() as i64);
    }
    let s = format!("{:.10}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

// ---------- таймеры ----------

/// «таймер 5м», «timer 90s», «т 10», «1ч30м», «25m». Возвращает секунды.
pub fn parse_timer(q: &str) -> Option<u64> {
    let s = q.trim().to_lowercase();
    let mut rest = s.as_str();
    let mut prefixed = false;
    for p in ["таймер", "timer", "т ", "t "] {
        if let Some(r) = rest.strip_prefix(p) {
            rest = r.trim();
            prefixed = true;
            break;
        }
    }
    let mut total = 0u64;
    let mut had_unit = false;
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0;
    let mut any = false;
    while i < chars.len() {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if start == i {
            return None;
        }
        let n: u64 = chars[start..i].iter().collect::<String>().parse().ok()?;
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let ustart = i;
        while i < chars.len() && chars[i].is_alphabetic() {
            i += 1;
        }
        let unit: String = chars[ustart..i].iter().collect();
        let mult = match unit.as_str() {
            "" => {
                if !prefixed || i < chars.len() {
                    return None;
                }
                60
            }
            "ч" | "час" | "часа" | "часов" | "h" | "hr" | "hour" | "hours" => 3600,
            "м" | "мин" | "минут" | "минуты" | "минута" | "m" | "min" | "mins" => 60,
            "с" | "сек" | "секунд" | "s" | "sec" | "secs" => 1,
            _ => return None,
        };
        if !unit.is_empty() {
            had_unit = true;
        }
        total += n * mult;
        any = true;
    }
    if !any || !(prefixed || had_unit) || total == 0 || total > 24 * 3600 {
        return None;
    }
    Some(total)
}

pub fn fmt_dur(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calc_works() {
        assert_eq!(calc("2+2*3"), Some(8.0));
        assert_eq!(calc("-2^2"), Some(-4.0));
        assert_eq!(calc("(1,5+0.5)*4"), Some(8.0));
        assert_eq!(calc("200*15%"), Some(30.0));
        assert_eq!(calc("hello"), None);
        assert_eq!(calc("42"), None);
        assert_eq!(fmt_num(1.0 / 3.0), "0.3333333333");
    }

    #[test]
    fn timer_works() {
        assert_eq!(parse_timer("таймер 5м"), Some(300));
        assert_eq!(parse_timer("t 10"), Some(600));
        assert_eq!(parse_timer("1ч30м"), Some(5400));
        assert_eq!(parse_timer("90s"), Some(90));
        assert_eq!(parse_timer("5"), None);
        assert_eq!(parse_timer("telegram"), None);
        assert_eq!(fmt_dur(3725), "1:02:05");
    }

    #[test]
    fn score_works() {
        assert_eq!(score("visual studio code", "vsc"), Some(1));
        assert_eq!(score("google chrome", "chr"), Some(1));
        assert_eq!(score("telegram", "tg"), Some(3));
    }
}
