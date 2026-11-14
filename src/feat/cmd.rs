//! Командная строка: поиск программ из меню Пуск, калькулятор, разбор таймеров, запуск выбранного.
//! Разбор и ранжирование — чистые функции с тестами, запуск — через `sys::launch`.

use std::path::{Path, PathBuf};

use windows::Win32::Foundation::HWND;

use crate::app::{st, State};
use crate::input::close_panel;
use crate::sys;

/// Режимы строки поиска: что попадает в список.
pub const MODE_ALL: u8 = 0;
pub const MODE_APPS: u8 = 1;
pub const MODE_CLIP: u8 = 2;
pub const MODE_ACT: u8 = 3;
pub const MODES: [&str; 4] = ["Всё", "Программы", "Буфер", "Действия"];

pub struct CmdState {
    /// Ярлыки, найденные при последнем открытии поиска.
    pub apps: Vec<App>,
    /// Сколько раз что запускали: (имя в нижнем регистре, число). Файл `launches.txt`.
    pub freq: Vec<(String, u32)>,
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
    /// Запись буфера (индекс в `clip.items`): вставить.
    Clip(usize),
}

/// Что показать в списке поиска для текущего запроса и режима. Порядок = порядок на экране.
pub fn results(s: &State) -> Vec<CmdRes> {
    let q = s.input.text.trim();
    let mode = s.input.mode;
    if mode == MODE_CLIP {
        return s.clip.view(q).into_iter().map(CmdRes::Clip).collect();
    }
    let mut out = Vec::new();
    if q.is_empty() {
        // Пустая строка: закреплённое из буфера и идущие таймеры; в «Программах» — частые.
        match mode {
            MODE_APPS => out.extend(frequent(&s.cmd.apps, &s.cmd.freq, 8).into_iter().map(CmdRes::App)),
            MODE_ALL => out.extend(s.clip.view("").into_iter().filter(|&i| s.clip.items[i].pinned).take(3).map(CmdRes::Clip)),
            _ => {}
        }
        if mode != MODE_APPS {
            out.extend((0..s.timers.list.len()).map(CmdRes::Active));
        }
        return out;
    }
    let acts = mode != MODE_APPS;
    if acts {
        if let Some(v) = calc(q) {
            out.push(CmdRes::Calc(fmt_num(v)));
        }
        if let Some(t) = parse_timer(q) {
            out.push(CmdRes::Timer(t));
        }
    }
    if mode != MODE_ACT && !q.starts_with('>') {
        let limit = if mode == MODE_APPS { 8 } else { 3 };
        out.extend(search(&s.cmd.apps, q, limit).into_iter().map(CmdRes::App));
    }
    if mode == MODE_ALL && !q.starts_with('>') {
        out.extend(s.clip.view(q).into_iter().take(2).map(CmdRes::Clip));
    }
    if acts {
        out.push(CmdRes::Run(q.to_string()));
    }
    out
}

/// Часто запускаемые программы: индексы в `apps`, самые частые первыми.
pub fn frequent(apps: &[App], freq: &[(String, u32)], n: usize) -> Vec<usize> {
    let mut v: Vec<(u32, usize)> = apps.iter().enumerate().filter_map(|(i, a)| freq.iter().find(|f| f.0 == a.lower).map(|f| (f.1, i))).collect();
    v.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().take(n).map(|x| x.1).collect()
}

fn freq_path() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|a| PathBuf::from(a).join("Pill").join("launches.txt"))
}

/// Строки «число<TAB>имя».
pub fn parse_freq(text: &str) -> Vec<(String, u32)> {
    text.lines().filter_map(|l| l.split_once('\t')).filter_map(|(n, name)| Some((name.to_string(), n.parse().ok()?))).collect()
}

pub fn load_freq() -> Vec<(String, u32)> {
    freq_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse_freq(&t)).unwrap_or_default()
}

/// Пишет файл счётчиков. Звать вне `st()`.
fn save_freq(freq: &[(String, u32)]) {
    let Some(p) = freq_path() else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text: String = freq.iter().map(|(name, n)| format!("{}\t{}\n", n, name)).collect();
    let _ = std::fs::write(p, text);
}

/// Запустить программу по индексу в `apps` и запомнить запуск.
pub unsafe fn run_app(hwnd: HWND, i: usize) {
    let got = st(|s| {
        let a = s.cmd.apps.get(i)?;
        let (key, path) = (a.lower.clone(), a.path.to_string_lossy().into_owned());
        match s.cmd.freq.iter_mut().find(|f| f.0 == key) {
            Some(f) => f.1 += 1,
            None => s.cmd.freq.push((key, 1)),
        }
        Some((path, s.cmd.freq.clone()))
    });
    close_panel(hwnd, false);
    if let Some((path, freq)) = got {
        save_freq(&freq);
        sys::launch::open(&path, None);
    }
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
        CmdRes::App(i) => run_app(hwnd, i),
        CmdRes::Clip(i) => {
            // Запись могла пропасть между отрисовкой и нажатием: тогда молча выходим.
            let text = st(|s| s.clip.items.get(i).map(|e| e.text.clone()));
            if let Some(t) = text {
                crate::input::paste(hwnd, &t);
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
