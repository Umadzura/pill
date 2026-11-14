//! Агенты в пилюле: статус ИИ-агентов (Claude Code и любых других, кто шлёт события в локальный API).
//!
//! ```text
//! POST /hook                                        тело — JSON хука Claude Code как есть (stdin хука)
//! GET|POST /agent?id=a1&name=pill&state=work&text=… любой другой агент: work | ask | wait | done | gone
//! ```
//! Состояния: работает, нужно разрешение, ждёт ответа, закончил. В свёрнутой пилюле видны только
//! работающие и ждущие разрешения, остальное — короткой надписью и во вкладке «События».
//! Здесь логика без Windows, с тестами.

use std::collections::HashMap;
use std::time::{Duration, Instant};

const MAX_AGENTS: usize = 8;
const MAX_NAME: usize = 24;
const MAX_NOTE: usize = 160;
/// «Работает» без единого события дольше этого — сессию убили, хук «закончил» не пришёл.
const WORK_STALE: Duration = Duration::from_secs(30 * 60);
/// Остальные состояния держим дольше: агент честно ждёт человека.
const IDLE_STALE: Duration = Duration::from_secs(12 * 60 * 60);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum AgentState {
    Work,
    /// Нужно разрешение (диалог «можно запустить?»).
    Ask,
    /// Ждёт ответа человека.
    Wait,
    Done,
    /// Сессия закрыта: убрать из списка.
    Gone,
}

impl AgentState {
    pub fn label(self) -> &'static str {
        match self {
            AgentState::Work => "работает",
            AgentState::Ask => "нужно разрешение",
            AgentState::Wait => "ждёт ответа",
            AgentState::Done | AgentState::Gone => "закончил",
        }
    }
}

/// Событие от агента (из /hook или /agent).
#[derive(Clone, PartialEq, Debug)]
pub struct Update {
    pub id: String,
    pub name: Option<String>,
    pub state: AgentState,
    pub note: Option<String>,
    /// Хук ждёт решения человека: ответить можно из чёлки (PermissionRequest у Claude Code).
    pub answerable: bool,
    pub extra: Extra,
}

/// Подробности события для карточки сессии. Чего в событии нет, то None и остаётся прежним.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Extra {
    /// Текущий инструмент: «Bash: cargo build».
    pub tool: Option<String>,
    /// Папка проекта (по ней читается ветка git).
    pub cwd: Option<String>,
    /// Последний ответ агента (если хук его прислал).
    pub answer: Option<String>,
    /// Просьба о разрешении: строки того, что хочет сделать инструмент, и сколько строк он добавляет и убирает.
    pub ask_lines: Vec<String>,
    pub ask_stat: (usize, usize),
    /// Просьба о разрешении: готовые правила «больше не спрашивать», как их прислал Claude Code
    /// (JSON-массив `permission_suggestions`). Их же возвращаем в ответе «Всегда».
    pub suggestions: Option<String>,
}

/// Сколько строк кода показываем в карточке просьбы.
const ASK_LINES: usize = 6;

/// Что именно просит инструмент, по его параметрам: строки для карточки («-» убрано, «+» добавлено,
/// пробел — команда как есть) и счётчики добавленных и убранных строк.
pub fn ask_detail(input: &HashMap<String, String>) -> (Vec<String>, (usize, usize)) {
    let clip = |s: &str| -> String { s.replace('\t', "    ").chars().filter(|c| !c.is_control()).take(110).collect() };
    let mut lines = Vec::new();
    let (mut plus, mut minus) = (0, 0);
    let (old, new) = (input.get("old_string"), input.get("new_string").or(input.get("content")));
    if old.is_some() || new.is_some() {
        for l in old.map(|o| o.lines()).into_iter().flatten() {
            minus += 1;
            if lines.len() < ASK_LINES / 2 {
                lines.push(format!("-{}", clip(l)));
            }
        }
        for l in new.map(|n| n.lines()).into_iter().flatten() {
            plus += 1;
            if lines.len() < ASK_LINES {
                lines.push(format!("+{}", clip(l)));
            }
        }
    } else if let Some(c) = input.get("command") {
        lines.extend(c.lines().take(ASK_LINES).map(|l| format!(" {}", clip(l))));
    }
    (lines, (plus, minus))
}

/// Что показать после события агента.
#[derive(Debug, PartialEq)]
pub enum Notice {
    /// Нужно разрешение: карточка (с кнопками, если answerable).
    Ask { name: String, text: String, answerable: bool },
    /// Закончил; secs — сколько работал над последним запросом.
    Done { name: String, text: String, secs: u64 },
    /// Перестал ждать разрешения: убрать его карточку и отпустить хук.
    Resolved,
}

pub struct Agent {
    pub id: String,
    pub name: String,
    pub state: AgentState,
    /// Последний запрос человека или текст вопроса.
    pub note: String,
    /// С какого момента агент в этом состоянии.
    pub since: Instant,
    /// Когда пришёл последний запрос человека (для «работал 6 минут»).
    pub started: Instant,
    updated: Instant,
    /// Чем занят сейчас: «Bash: cargo build». Пусто — неизвестно.
    pub tool: String,
    /// Последний ответ агента. Пусто — хук его не прислал.
    pub answer: String,
    /// Ветка git. None — ещё не читали, пустая строка — не репозиторий.
    pub branch: Option<String>,
    pub cwd: String,
    /// Последняя просьба о разрешении: строки кода и счётчики (см. `ask_detail`).
    pub ask_lines: Vec<String>,
    pub ask_stat: (usize, usize),
    /// На последнюю просьбу можно ответить «всегда» (хук прислал готовое правило).
    pub ask_always: bool,
}

#[derive(Default)]
pub struct Agents {
    pub list: Vec<Agent>,
}

/// Одна строка без управляющих символов, не длиннее n.
pub fn one_line(s: &str, n: usize) -> String {
    s.trim().chars().map(|c| if c.is_whitespace() { ' ' } else { c }).filter(|c| !c.is_control()).take(n).collect()
}

impl Agents {
    /// Применить событие. Возвращает, что показать человеку (только «нужно разрешение» и «закончил»).
    pub fn apply(&mut self, u: Update, now: Instant) -> Option<Notice> {
        let pos = self.list.iter().position(|a| a.id == u.id);
        let prev = pos.map(|i| self.list[i].state);
        let id = u.id.clone();
        if u.state == AgentState::Gone {
            if let Some(i) = pos {
                self.list.remove(i);
            }
            return (prev == Some(AgentState::Ask)).then_some(Notice::Resolved);
        }
        let name = u
            .name
            .map(|n| one_line(&n, MAX_NAME))
            .filter(|n| !n.is_empty())
            .or_else(|| pos.map(|i| self.list[i].name.clone()))
            .unwrap_or_else(|| one_line(&u.id, 8));
        // Новый запрос человека: прежние инструмент, ответ и ветка устарели.
        let new_prompt = u.state == AgentState::Work && u.note.is_some();
        let extra = u.extra;
        let note = u.note.map(|n| one_line(&n, MAX_NOTE));
        let changed = match pos {
            Some(i) => {
                let a = &mut self.list[i];
                let changed = a.state != u.state;
                a.name = name.clone();
                a.state = u.state;
                a.updated = now;
                if changed {
                    a.since = now;
                }
                if let Some(n) = note {
                    a.note = n;
                }
                changed
            }
            None => {
                if self.list.len() >= MAX_AGENTS {
                    // Вытесняем того, о ком дольше всех не слышали.
                    if let Some(old) = (0..self.list.len()).min_by_key(|&i| self.list[i].updated) {
                        self.list.remove(old);
                    }
                }
                self.list.push(Agent { id: u.id, name: name.clone(), state: u.state, note: note.unwrap_or_default(), since: now, started: now, updated: now, tool: String::new(), answer: String::new(), branch: None, cwd: String::new(), ask_lines: Vec::new(), ask_stat: (0, 0), ask_always: false });
                true
            }
        };
        if let Some(a) = self.list.iter_mut().find(|a| a.id == id) {
            if new_prompt {
                a.started = now;
                a.tool.clear();
                a.answer.clear();
                a.branch = None;
            }
            if let Some(t) = extra.tool {
                a.tool = one_line(&t, MAX_NOTE);
            }
            if let Some(t) = extra.answer {
                a.answer = one_line(&t, MAX_NOTE);
            }
            if let Some(c) = extra.cwd {
                a.cwd = c;
            }
            if u.state == AgentState::Ask {
                a.ask_lines = extra.ask_lines;
                a.ask_stat = extra.ask_stat;
                a.ask_always = extra.suggestions.is_some();
            }
        }
        // Что показать в карточке: у просьбы из хука — инструмент, иначе текст события.
        let text = self.list.iter().find(|a| a.id == id).map(|a| if u.state == AgentState::Ask && u.answerable && !a.tool.is_empty() { a.tool.clone() } else { a.note.clone() }).unwrap_or_default();
        match u.state {
            // Хук с ожиданием ответа показываем всегда: он мог прийти после простого уведомления.
            AgentState::Ask if changed || u.answerable => Some(Notice::Ask { name, text, answerable: u.answerable }),
            AgentState::Done if changed => {
                let secs = self.list.iter().find(|a| a.id == id).map(|a| now.duration_since(a.started).as_secs()).unwrap_or(0);
                Some(Notice::Done { name, text, secs })
            }
            _ if changed && prev == Some(AgentState::Ask) => Some(Notice::Resolved),
            _ => None,
        }
    }

    /// Поставить состояние без события (ответили из чёлки).
    pub fn set_state(&mut self, id: &str, state: AgentState) {
        if let Some(a) = self.list.iter_mut().find(|a| a.id == id) {
            a.state = state;
        }
    }

    /// Папка агента, у которого ветка ещё не прочитана.
    pub fn wants_branch(&self, id: &str) -> Option<String> {
        self.list.iter().find(|a| a.id == id && a.branch.is_none() && !a.cwd.is_empty()).map(|a| a.cwd.clone())
    }

    pub fn set_branch(&mut self, id: &str, branch: String) {
        if let Some(a) = self.list.iter_mut().find(|a| a.id == id) {
            a.branch = Some(branch);
        }
    }

    /// Сколько ждут разрешения и сколько работают: для сводки над списком.
    pub fn summary(&self) -> (usize, usize) {
        let n = |st: AgentState| self.list.iter().filter(|a| a.state == st).count();
        (n(AgentState::Ask), n(AgentState::Work))
    }

    /// Порядок показа: сначала те, кому нужно разрешение, потом работающие, ждущие, закончившие.
    pub fn order(&self) -> Vec<usize> {
        let rank = |s: AgentState| match s {
            AgentState::Ask => 0,
            AgentState::Work => 1,
            AgentState::Wait => 2,
            AgentState::Done | AgentState::Gone => 3,
        };
        let mut idx: Vec<usize> = (0..self.list.len()).collect();
        idx.sort_by_key(|&i| rank(self.list[i].state));
        idx
    }

    /// Убрать тех, о ком давно не слышали. true — что-то пропало.
    pub fn expire(&mut self, now: Instant) -> bool {
        let n = self.list.len();
        self.list.retain(|a| {
            let limit = if a.state == AgentState::Work { WORK_STALE } else { IDLE_STALE };
            now.duration_since(a.updated) < limit
        });
        self.list.len() != n
    }

    /// Для точки в свёрнутой чёлке: (кому-то нужно разрешение, сколько агентов работает или ждёт разрешения).
    pub fn attention(&self) -> Option<(bool, usize)> {
        let n = self.list.iter().filter(|a| matches!(a.state, AgentState::Work | AgentState::Ask)).count();
        (n > 0).then(|| (self.list.iter().any(|a| a.state == AgentState::Ask), n))
    }
}

/// Содержимое `.git/HEAD` → имя ветки («ref: refs/heads/main» → «main») или короткий хеш.
pub fn branch_from_head(head: &str) -> String {
    let h = head.trim();
    match h.strip_prefix("ref:") {
        Some(r) => r.trim().trim_start_matches("refs/heads/").to_string(),
        None => h.chars().take(7).collect(),
    }
}

/// Ветка проекта в папке cwd. Пусто — не репозиторий. Читает маленький файл: звать вне `st()`.
pub fn read_branch(cwd: &str) -> String {
    std::fs::read_to_string(std::path::Path::new(cwd).join(".git").join("HEAD")).map(|h| branch_from_head(&h)).unwrap_or_default()
}

/// Что просит инструмент, простыми словами: («хочет изменить файл», «feed.rs»).
/// `tool` — строка вида «Edit: C:\\p\\src\\feed.rs» (инструмент и главное из его параметров).
pub fn ask_phrase(tool: &str) -> (String, String) {
    let (name, detail) = tool.split_once(": ").unwrap_or((tool, ""));
    let base = |p: &str| p.rsplit(['/', '\\']).find(|x| !x.is_empty()).unwrap_or(p).to_string();
    let host = |u: &str| {
        let u = u.split("://").nth(1).unwrap_or(u);
        u.split('/').next().unwrap_or(u).trim_start_matches("www.").to_string()
    };
    let short = |s: &str| one_line(s, 60);
    let (verb, target) = match name {
        "" => ("просит разрешение", String::new()),
        "Edit" | "MultiEdit" | "NotebookEdit" => ("хочет изменить файл", base(detail)),
        "Write" => ("хочет создать файл", base(detail)),
        "Read" => ("хочет прочитать файл", base(detail)),
        "Bash" | "PowerShell" => ("хочет выполнить команду", short(detail)),
        "WebFetch" => ("хочет открыть сайт", host(detail)),
        "WebSearch" => ("хочет поискать в интернете", short(detail)),
        "Glob" | "Grep" => ("хочет поискать в файлах", short(detail)),
        "Task" | "Agent" => ("хочет запустить помощника", short(detail)),
        n if n.starts_with("mcp__") => ("хочет использовать", n.trim_start_matches("mcp__").replace("__", " · ")),
        n => ("хочет использовать", n.to_string()),
    };
    (verb.to_string(), target)
}

/// «работал меньше минуты», «работал 6 мин», «работал 1 ч 20 мин».
pub fn fmt_worked(secs: u64) -> String {
    match secs / 60 {
        0 => "работал меньше минуты".into(),
        m @ 1..=59 => format!("работал {} мин", m),
        m => format!("работал {} ч {} мин", m / 60, m % 60),
    }
}

/// «сейчас», «5 мин», «2 ч» — сколько агент в этом состоянии.
pub fn fmt_age(d: Duration) -> String {
    let m = d.as_secs() / 60;
    match m {
        0 => "сейчас".into(),
        1..=59 => format!("{} мин", m),
        _ => format!("{} ч", m / 60),
    }
}

// ---------- хуки Claude Code ----------

/// JSON, который Claude Code подаёт хуку на stdin → событие агента. None — событие нам не интересно.
pub fn from_hook(json: &str) -> Option<Update> {
    let (f, objs) = json_fields(json);
    let id = f.get("session_id").filter(|s| !s.is_empty())?.clone();
    // Имя агента — папка проекта.
    let name = f.get("cwd").and_then(|c| c.rsplit(['/', '\\']).find(|p| !p.is_empty())).map(str::to_string);
    // «Bash: cargo build» — инструмент и главное из его параметров.
    let input = objs.get("tool_input").map(|o| json_strings(o)).unwrap_or_default();
    let tool = f.get("tool_name").map(|tool| match ["command", "file_path", "url", "pattern", "description", "prompt"].iter().find_map(|k| input.get(*k)) {
        // У команды в заголовке только первая строка: целиком она идёт в блок кода.
        Some(d) => format!("{}: {}", tool, d.lines().next().unwrap_or("")),
        None => tool.clone(),
    });
    let (ask_lines, ask_stat) = ask_detail(&input);
    let extra = Extra { tool: tool.clone(), cwd: f.get("cwd").cloned(), answer: f.get("last_assistant_message").cloned(), ask_lines, ask_stat, suggestions: objs.get("permission_suggestions").filter(|s| s.starts_with('[') && s.len() > 2).cloned() };
    let (state, note) = match f.get("hook_event_name")?.as_str() {
        "UserPromptSubmit" => (AgentState::Work, f.get("prompt").cloned()),
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => (AgentState::Work, None),
        "PermissionRequest" => {
            // Запрос человека (note) не трогаем: что просит инструмент, лежит в extra.
            return Some(Update { id, name, state: AgentState::Ask, note: None, answerable: true, extra });
        }
        "Notification" => match f.get("notification_type").map(String::as_str) {
            Some("permission_prompt") | Some("elicitation_dialog") => (AgentState::Ask, f.get("message").cloned()),
            Some("idle_prompt") => (AgentState::Wait, None),
            _ => return None,
        },
        "Stop" => (AgentState::Done, None),
        "SessionEnd" => (AgentState::Gone, None),
        _ => return None,
    };
    Some(Update { id, name, state, note, answerable: false, extra })
}

/// Строковые поля верхнего уровня JSON-объекта. Вложенные объекты и массивы пропускаются.
/// Оборванный JSON (тело хука длиннее лимита запроса) не ошибка: отдаём то, что успели прочитать.
pub fn json_strings(s: &str) -> HashMap<String, String> {
    json_fields(s).0
}

/// То же, плюс вложенные объекты верхнего уровня как текст (их можно разобрать ещё раз).
pub fn json_fields(s: &str) -> (HashMap<String, String>, HashMap<String, String>) {
    let b = s.as_bytes();
    let mut out = HashMap::new();
    let mut objs = HashMap::new();
    let mut i = skip_ws(b, 0);
    if b.get(i) != Some(&b'{') {
        return (out, objs);
    }
    i += 1;
    loop {
        i = skip_ws(b, i);
        while b.get(i) == Some(&b',') {
            i = skip_ws(b, i + 1);
        }
        let Some((key, next)) = json_str(b, i) else { break };
        i = skip_ws(b, next);
        if b.get(i) != Some(&b':') {
            break;
        }
        i = skip_ws(b, i + 1);
        if b.get(i) == Some(&b'"') {
            let Some((val, next)) = json_str(b, i) else { break };
            out.insert(key, val);
            i = next;
        } else {
            let Some(next) = skip_value(b, i) else { break };
            if b[i] == b'{' || b[i] == b'[' {
                objs.insert(key, String::from_utf8_lossy(&b[i..next]).into_owned());
            }
            i = next;
        }
    }
    (out, objs)
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while matches!(b.get(i), Some(b' ' | b'\t' | b'\r' | b'\n')) {
        i += 1;
    }
    i
}

/// Строка JSON с позиции открывающей кавычки → (значение, позиция после закрывающей). None — оборвана.
fn json_str(b: &[u8], mut i: usize) -> Option<(String, usize)> {
    if b.get(i) != Some(&b'"') {
        return None;
    }
    i += 1;
    let mut out: Vec<u8> = Vec::new();
    let push = |c: u32, out: &mut Vec<u8>| {
        let ch = char::from_u32(c).unwrap_or('\u{FFFD}');
        out.extend_from_slice(ch.encode_utf8(&mut [0u8; 4]).as_bytes());
    };
    loop {
        match *b.get(i)? {
            b'"' => return Some((String::from_utf8_lossy(&out).into_owned(), i + 1)),
            b'\\' => {
                i += 1;
                match *b.get(i)? {
                    b'n' => out.push(b'\n'),
                    b't' => out.push(b'\t'),
                    b'r' => out.push(b'\r'),
                    b'b' | b'f' => out.push(b' '),
                    // Битая «\u» (не четыре hex-цифры) — не повод терять весь JSON: берём букву как есть.
                    b'u' if hex4(b, i + 1).is_none() => out.push(b'u'),
                    b'u' => {
                        let hi = hex4(b, i + 1)?;
                        i += 4;
                        // Суррогатная пара: \uD83D\uDE00.
                        if (0xD800..0xDC00).contains(&hi) && b.get(i + 1) == Some(&b'\\') && b.get(i + 2) == Some(&b'u') {
                            let lo = hex4(b, i + 3)?;
                            if (0xDC00..0xE000).contains(&lo) {
                                i += 6;
                                push(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00), &mut out);
                            } else {
                                push(hi, &mut out);
                            }
                        } else {
                            push(hi, &mut out);
                        }
                    }
                    c => out.push(c),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
}

fn hex4(b: &[u8], i: usize) -> Option<u32> {
    u32::from_str_radix(std::str::from_utf8(b.get(i..i + 4)?).ok()?, 16).ok()
}

/// Пропустить значение (число, true/null, объект, массив) → позиция сразу после него. None — оборвано.
fn skip_value(b: &[u8], mut i: usize) -> Option<usize> {
    let mut depth = 0usize;
    loop {
        match *b.get(i)? {
            b'"' => {
                i = json_str(b, i)?.1;
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => {
                // Закрылся сам объект верхнего уровня.
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            b',' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn up(id: &str, state: AgentState) -> Update {
        Update { id: id.into(), name: Some("pill".into()), state, note: None, answerable: false, extra: Extra::default() }
    }

    #[test]
    fn phrases() {
        let p = |t: &str| ask_phrase(t);
        assert_eq!(p("Edit: C:\\p\\src\\ui\\feed.rs"), ("хочет изменить файл".into(), "feed.rs".into()));
        assert_eq!(p("Write: /home/a/new.txt"), ("хочет создать файл".into(), "new.txt".into()));
        assert_eq!(p("Bash: cargo build --release"), ("хочет выполнить команду".into(), "cargo build --release".into()));
        assert_eq!(p("WebFetch: https://www.example.com/a/b"), ("хочет открыть сайт".into(), "example.com".into()));
        assert_eq!(p("mcp__github__create_issue"), ("хочет использовать".into(), "github · create_issue".into()));
        assert_eq!(p(""), ("просит разрешение".into(), String::new()));
        assert_eq!((fmt_worked(30), fmt_worked(360), fmt_worked(4800)), ("работал меньше минуты".into(), "работал 6 мин".into(), "работал 1 ч 20 мин".into()));
    }

    #[test]
    fn ask_details() {
        let input = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        // Правка файла: сначала убранные строки, потом добавленные, счётчики по всем строкам.
        let (lines, stat) = ask_detail(&input(&[("old_string", "a\nb"), ("new_string", "a\n\tc\nd")]));
        assert_eq!(lines, vec!["-a", "-b", "+a", "+    c", "+d"]);
        assert_eq!(stat, (3, 2));
        // Команда: как есть, без счётчиков.
        assert_eq!(ask_detail(&input(&[("command", "cargo build --release")])), (vec![" cargo build --release".to_string()], (0, 0)));
        // Новый файл: только добавленные строки, не больше лимита.
        let (lines, stat) = ask_detail(&input(&[("content", "1\n2\n3\n4\n5\n6\n7\n8")]));
        assert_eq!((lines.len(), stat), (ASK_LINES, (8, 0)));

        // Просьба из хука не затирает запрос человека и отдаёт в карточку инструмент.
        let now = Instant::now();
        let mut a = Agents::default();
        let hook = |ev: &str, rest: &str| from_hook(&format!(r#"{{"session_id":"s1","cwd":"C:\\p\\pill","hook_event_name":"{}"{}}}"#, ev, rest)).unwrap();
        a.apply(hook("UserPromptSubmit", r#","prompt":"собери""#), now);
        let n = a.apply(hook("PermissionRequest", r#","tool_name":"Bash","tool_input":{"command":"cargo build\nls"}"#), now);
        assert_eq!(n, Some(Notice::Ask { name: "pill".into(), text: "Bash: cargo build".into(), answerable: true }));
        assert_eq!((a.list[0].note.as_str(), a.list[0].ask_lines.len(), a.list[0].ask_always), ("собери", 2, false));
        // Claude Code прислал готовое правило: его можно вернуть как «всегда».
        let u = hook("PermissionRequest", r#","tool_name":"Bash","tool_input":{"command":"ls"},"permission_suggestions":[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"ls"}],"behavior":"allow","destination":"localSettings"}]"#);
        assert_eq!(u.extra.suggestions.as_deref(), Some(r#"[{"type":"addRules","rules":[{"toolName":"Bash","ruleContent":"ls"}],"behavior":"allow","destination":"localSettings"}]"#));
    }

    #[test]
    fn branch_and_session_details() {
        assert_eq!(branch_from_head("ref: refs/heads/feature/header\n"), "feature/header");
        assert_eq!(branch_from_head("0123456789abcdef\n"), "0123456");
        let now = Instant::now();
        let mut a = Agents::default();
        let hook = |ev: &str, rest: &str| from_hook(&format!(r#"{{"session_id":"s1","cwd":"C:\\p\\pill","hook_event_name":"{}"{}}}"#, ev, rest)).unwrap();
        a.apply(hook("UserPromptSubmit", r#","prompt":"собери""#), now);
        assert_eq!(a.wants_branch("s1").as_deref(), Some("C:\\p\\pill"));
        a.set_branch("s1", "main".into());
        a.apply(hook("PostToolUse", r#","tool_name":"Bash","tool_input":{"command":"cargo build"}"#), now);
        assert_eq!((a.list[0].tool.as_str(), a.list[0].note.as_str(), a.wants_branch("s1")), ("Bash: cargo build", "собери", None));
        a.apply(hook("Stop", r#","last_assistant_message":"Готово""#), now);
        assert_eq!(a.list[0].answer, "Готово");
        // Новый запрос сбрасывает инструмент, ответ и ветку.
        a.apply(hook("UserPromptSubmit", r#","prompt":"ещё""#), now);
        assert!(a.list[0].tool.is_empty() && a.list[0].answer.is_empty() && a.list[0].branch.is_none());
        // Сначала тот, кому нужно разрешение.
        a.apply(Update { id: "s2".into(), name: Some("site".into()), state: AgentState::Ask, note: None, answerable: false, extra: Extra::default() }, now);
        assert_eq!((a.order(), a.summary()), (vec![1, 0], (1, 1)));
    }

    #[test]
    fn json_top_level() {
        let f = json_strings(r#"{"a":"x","n":12,"o":{"a":"inner","l":[1,"]"]},"b":"\u041f\u0440 \"q\"\n\ud83d\ude00","t":true,"c":"C:\\dir"}"#);
        assert_eq!(f["a"], "x");
        assert_eq!(f["b"], "Пр \"q\"\n😀");
        assert_eq!(f["c"], "C:\\dir");
        assert_eq!(f.len(), 3);
        // Оборванный JSON: что успели — отдали.
        let f = json_strings(r#"{"session_id":"s1","hook_event_name":"Stop","big":"aaaa"#);
        assert_eq!(f["session_id"], "s1");
        assert_eq!(f["hook_event_name"], "Stop");
        assert!(!f.contains_key("big"));
        assert!(json_strings("не json").is_empty());
    }

    #[test]
    fn hooks() {
        let u = from_hook(r#"{"session_id":"s1","transcript_path":"x","cwd":"C:\\Users\\me\\projects\\pill","hook_event_name":"UserPromptSubmit","prompt":"сделай\nкрасиво"}"#).unwrap();
        assert_eq!(u, Update { id: "s1".into(), name: Some("pill".into()), state: AgentState::Work, note: Some("сделай\nкрасиво".into()), answerable: false, extra: u.extra.clone() });
        let ask = from_hook(r#"{"session_id":"s1","cwd":"/home/me/app/","hook_event_name":"Notification","message":"Claude needs your permission to use Bash","notification_type":"permission_prompt"}"#).unwrap();
        assert_eq!((ask.name.as_deref(), ask.state), (Some("app"), AgentState::Ask));
        assert_eq!(from_hook(r#"{"session_id":"s1","hook_event_name":"Notification","notification_type":"idle_prompt"}"#).unwrap().state, AgentState::Wait);
        assert_eq!(from_hook(r#"{"session_id":"s1","hook_event_name":"Stop"}"#).unwrap().state, AgentState::Done);
        assert_eq!(from_hook(r#"{"session_id":"s1","hook_event_name":"SessionEnd"}"#).unwrap().state, AgentState::Gone);
        // tool_input с тем же ключом внутри не путает разбор.
        let u = from_hook(r#"{"session_id":"s1","hook_event_name":"PostToolUse","tool_input":{"hook_event_name":"Stop","session_id":"zz"}}"#).unwrap();
        assert_eq!((u.id.as_str(), u.state), ("s1", AgentState::Work));
        assert!(from_hook(r#"{"session_id":"s1","hook_event_name":"SubagentStop"}"#).is_none());
        assert!(from_hook(r#"{"hook_event_name":"Stop"}"#).is_none());
    }

    #[test]
    fn flow() {
        let mut a = Agents::default();
        let now = Instant::now();
        assert_eq!(a.apply(up("s1", AgentState::Work), now), None);
        assert_eq!(a.attention(), Some((false, 1)));
        // Повтор того же состояния ничего не показывает.
        assert_eq!(a.apply(up("s1", AgentState::Work), now), None);
        assert!(matches!(a.apply(up("s1", AgentState::Ask), now), Some(Notice::Ask { answerable: false, .. })));
        assert_eq!(a.apply(up("s1", AgentState::Ask), now), None);
        // Хук с ожиданием ответа показываем, даже если состояние то же.
        let hook = Update { id: "s1".into(), name: None, state: AgentState::Ask, note: Some("Bash: ls".into()), answerable: true, extra: Extra::default() };
        assert_eq!(a.apply(hook, now), Some(Notice::Ask { name: "pill".into(), text: "Bash: ls".into(), answerable: true }));
        a.apply(Update { id: "s2".into(), name: Some("site".into()), state: AgentState::Work, note: Some(" запрос \n".into()), answerable: false, extra: Extra::default() }, now);
        assert_eq!(a.attention(), Some((true, 2)));
        assert_eq!(a.list[1].note, "запрос");
        // Событие без имени и заметки сохраняет старые; «ждёт ответа» молчит.
        assert_eq!(a.apply(Update { id: "s2".into(), name: None, state: AgentState::Done, note: None, answerable: false, extra: Extra::default() }, now), Some(Notice::Done { name: "site".into(), text: "запрос".into(), secs: 0 }));
        assert_eq!(a.apply(up("s2", AgentState::Wait), now), None);
        // Разрешили в окне агента: карточку убрать.
        assert_eq!(a.apply(up("s1", AgentState::Work), now), Some(Notice::Resolved));
        a.set_state("s1", AgentState::Ask);
        assert_eq!(a.apply(up("s1", AgentState::Gone), now), Some(Notice::Resolved));
        assert_eq!(a.list.len(), 1);
        assert_eq!(a.attention(), None);
        // Закончивший живёт долго, зависший «работает» — полчаса.
        a.apply(up("s3", AgentState::Work), now);
        assert!(a.expire(now + WORK_STALE + Duration::from_secs(1)));
        assert_eq!(a.list.len(), 1);
        assert!(a.expire(now + IDLE_STALE + Duration::from_secs(1)));
        assert!(a.list.is_empty());
    }

    #[test]
    fn permission_hook() {
        let u = from_hook(r#"{"session_id":"s1","cwd":"/p/pill","hook_event_name":"PermissionRequest","tool_name":"Bash","tool_input":{"command":"cargo build","description":"Сборка"}}"#).unwrap();
        assert_eq!(u, Update { id: "s1".into(), name: Some("pill".into()), state: AgentState::Ask, note: None, answerable: true, extra: u.extra.clone() });
        let u = from_hook(r#"{"session_id":"s1","hook_event_name":"PermissionRequest","tool_name":"Mystery","tool_input":{"n":1}}"#).unwrap();
        // Инструмент без известных параметров: в карточке просто его имя.
        assert_eq!((u.note.as_deref(), u.extra.tool.as_deref()), (None, Some("Mystery")));
    }

    #[test]
    fn limit_and_age() {
        let mut a = Agents::default();
        let now = Instant::now();
        for i in 0..MAX_AGENTS + 2 {
            a.apply(up(&format!("s{}", i), AgentState::Work), now + Duration::from_secs(i as u64));
        }
        assert_eq!(a.list.len(), MAX_AGENTS);
        assert_eq!(a.list[0].id, "s2");
        assert_eq!(fmt_age(Duration::from_secs(20)), "сейчас");
        assert_eq!(fmt_age(Duration::from_secs(5 * 60)), "5 мин");
        assert_eq!(fmt_age(Duration::from_secs(3 * 3600)), "3 ч");
    }
}

