//! Локальный API для скриптов: уведомления и задачи с прогрессом в пилюле.
//!
//! ```text
//! GET|POST /notify?text=Рендер готов&secs=5         всплывающая надпись (1–60 с, по умолчанию 4)
//! GET|POST /task?id=render&title=Рендер&progress=45 задача в пилюле: «Рендер 45%»
//! GET|POST /task?id=render&state=done               готово: задача пропадает, «✓ Рендер»
//! GET|POST /task?id=render&state=error              ошибка: «✗ Рендер: ошибка»
//! GET      /ping                                     «pill» — проверить, что жив
//! POST     /hook                                     JSON хука Claude Code (см. `feat::agents`)
//! GET|POST /agent?id=a1&name=pill&state=work&text=…  статус любого агента: work | ask | wait | done | gone
//! ```
//! Параметры — в адресе или в теле (`application/x-www-form-urlencoded`). Для /notify тело
//! без параметров целиком становится текстом. Слушаем только 127.0.0.1 (поток — `bg::api`).
//! Запросы из браузера отклоняем (Origin/Sec-Fetch-*, чужой Host): сайт не сможет слать надписи.
//! Здесь разбор и логика без Windows, с тестами.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;

use crate::app::st;
use crate::feat::agents::{self, AgentState};
use crate::feat::osd::{self, Flash};
use crate::feat::peek::{self, Peek, PeekKind};

pub const PORT: u16 = 47821;
/// Сколько байт запроса читаем максимум (заголовки + тело).
pub const MAX_REQUEST: usize = 16 * 1024;
const MAX_TEXT: usize = 200;
const MAX_TITLE: usize = 60;
const MAX_TASKS: usize = 8;
/// Задача без обновлений дольше этого пропадает (скрипт упал и не сказал «готово»).
const TASK_STALE: Duration = Duration::from_secs(15 * 60);
/// Сколько задач показывать в свёрнутой пилюле.
pub const SHOWN_TASKS: usize = 2;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TaskState {
    Run,
    Done,
    Error,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Event {
    Notify { text: String, secs: u32 },
    Task { id: String, title: Option<String>, progress: Option<u8>, state: TaskState },
    Agent(agents::Update),
    /// Раскрыть панель на вкладке (или свернуть): для скриптов и для сверки вида по снимку экрана.
    Open(String),
}

/// Ответ для клиента: код и текст.
#[derive(Clone, PartialEq, Debug)]
pub struct Reply {
    pub code: u16,
    pub body: &'static str,
}

const OK: Reply = Reply { code: 200, body: "ok\n" };
/// Хукам отвечаем пустым телом: Claude Code подмешивает stdout хука в контекст.
const QUIET: Reply = Reply { code: 200, body: "" };
const PONG: Reply = Reply { code: 200, body: "pill\n" };
const BAD: Reply = Reply { code: 400, body: "bad request\n" };
const FORBIDDEN: Reply = Reply { code: 403, body: "forbidden\n" };
const NOT_FOUND: Reply = Reply { code: 404, body: "not found: /notify, /task, /agent, /hook, /ping\n" };
const TOO_BIG: Reply = Reply { code: 413, body: "too large\n" };

/// Сколько всего байт должен занять запрос (заголовки + Content-Length), если заголовки уже пришли.
pub fn expected_len(buf: &[u8]) -> Option<usize> {
    let end = buf.windows(4).position(|w| w == b"\r\n\r\n")? + 4;
    let head = std::str::from_utf8(&buf[..end]).ok()?;
    let len = head
        .lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.trim().parse::<usize>().ok())
        .unwrap_or(0);
    Some(end + len)
}

/// Разобрать запрос целиком: что ответить и что сделать.
pub fn handle(buf: &[u8]) -> (Reply, Option<Event>) {
    let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else { return (BAD, None) };
    let Ok(head) = std::str::from_utf8(&buf[..end]) else { return (BAD, None) };
    let body = &buf[end + 4..];
    let mut lines = head.split("\r\n");
    let mut first = lines.next().unwrap_or("").split(' ');
    let (method, target) = (first.next().unwrap_or(""), first.next().unwrap_or(""));
    if method != "GET" && method != "POST" {
        return (BAD, None);
    }
    let mut host_ok = false;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        let k = k.trim().to_ascii_lowercase();
        // Браузер всегда шлёт Origin или Sec-Fetch-* — скрипты и curl нет.
        if k == "origin" || k.starts_with("sec-fetch-") {
            return (FORBIDDEN, None);
        }
        if k == "host" {
            let h = v.trim();
            let name = h.rsplit_once(':').map(|(n, _)| n).unwrap_or(h);
            // Защита от DNS rebinding: только локальные имена.
            host_ok = matches!(name, "127.0.0.1" | "localhost" | "[::1]");
        }
    }
    if !host_ok {
        return (FORBIDDEN, None);
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path == "/hook" {
        // Тело хука бывает длиннее лимита (большой запрос, вывод инструмента): нужные поля идут первыми,
        // поэтому разбираем и обрезанное.
        return (QUIET, agents::from_hook(&String::from_utf8_lossy(body)).map(Event::Agent));
    }
    if buf.len() > MAX_REQUEST {
        return (TOO_BIG, None);
    }
    let mut params = parse_form(query);
    let body_text = String::from_utf8_lossy(body).trim().to_string();
    if !body_text.is_empty() {
        let form = parse_form(&body_text);
        if form.is_empty() || !body_text.contains('=') {
            params.entry("text".into()).or_insert(body_text.clone());
        } else {
            for (k, v) in form {
                params.entry(k).or_insert(v);
            }
        }
    }
    match path {
        "/ping" => (PONG, None),
        "/open" => (OK, Some(Event::Open(params.get("tab").map(|t| t.trim().to_ascii_lowercase()).unwrap_or_default()))),
        "/notify" => {
            let text = clip(params.get("text").map(|s| s.trim()).unwrap_or(""), MAX_TEXT);
            if text.is_empty() {
                return (BAD, None);
            }
            let secs = params.get("secs").and_then(|s| s.parse::<u32>().ok()).unwrap_or(4).clamp(1, 60);
            (OK, Some(Event::Notify { text, secs }))
        }
        "/task" => {
            let title = params.get("title").map(|t| clip(t.trim(), MAX_TITLE)).filter(|t| !t.is_empty());
            let id = params.get("id").map(|s| clip(s.trim(), MAX_TITLE)).filter(|s| !s.is_empty()).or_else(|| title.clone());
            let Some(id) = id else { return (BAD, None) };
            let progress = params.get("progress").and_then(|s| s.trim().trim_end_matches('%').parse::<f32>().ok()).map(|p| p.clamp(0.0, 100.0) as u8);
            let state = match params.get("state").map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                None | Some("") | Some("run") | Some("running") => TaskState::Run,
                Some("done") | Some("ok") | Some("finished") => TaskState::Done,
                Some("error") | Some("fail") | Some("failed") => TaskState::Error,
                _ => return (BAD, None),
            };
            (OK, Some(Event::Task { id, title, progress, state }))
        }
        "/agent" => {
            let Some(id) = params.get("id").map(|s| clip(s.trim(), MAX_TITLE)).filter(|s| !s.is_empty()) else { return (BAD, None) };
            let state = match params.get("state").map(|s| s.trim().to_ascii_lowercase()).as_deref() {
                None | Some("") | Some("work") | Some("run") | Some("running") => AgentState::Work,
                Some("ask") | Some("permission") => AgentState::Ask,
                Some("wait") | Some("idle") => AgentState::Wait,
                Some("done") | Some("ok") | Some("finished") => AgentState::Done,
                Some("gone") | Some("end") => AgentState::Gone,
                _ => return (BAD, None),
            };
            let name = params.get("name").map(|s| clip(s.trim(), MAX_TITLE)).filter(|s| !s.is_empty());
            let note = params.get("text").map(|s| clip(s.trim(), MAX_TEXT)).filter(|s| !s.is_empty());
            (OK, Some(Event::Agent(agents::Update { id, name, state, note, answerable: false, extra: agents::Extra::default() })))
        }
        _ => (NOT_FOUND, None),
    }
}

fn clip(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect()
}

/// `a=1&b=%D0%BF%D1%80+x` → {a: "1", b: "пр x"}.
pub fn parse_form(s: &str) -> HashMap<String, String> {
    s.split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            Some((url_decode(k)?, url_decode(v)?))
        })
        .collect()
}

/// Процентное декодирование UTF-8, «+» — пробел. Битая последовательность — None.
pub fn url_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 2;
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8(out).ok()
}

// ---------- задачи в пилюле ----------

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub progress: Option<u8>,
    updated: Instant,
}

#[derive(Default)]
pub struct Tasks {
    pub list: Vec<Task>,
}

/// Что показать после события задачи.
#[derive(Debug, PartialEq)]
pub enum Notice {
    None,
    Done(String),
    Failed(String),
}

impl Tasks {
    pub fn apply(&mut self, id: String, title: Option<String>, progress: Option<u8>, state: TaskState, now: Instant) -> Notice {
        let pos = self.list.iter().position(|t| t.id == id);
        let name = title.clone().or_else(|| pos.map(|i| self.list[i].title.clone())).unwrap_or_else(|| id.clone());
        match state {
            TaskState::Done | TaskState::Error => {
                if let Some(i) = pos {
                    self.list.remove(i);
                }
                if state == TaskState::Done {
                    Notice::Done(name)
                } else {
                    Notice::Failed(name)
                }
            }
            TaskState::Run => {
                let t = Task { id, title: name, progress, updated: now };
                match pos {
                    Some(i) => {
                        self.list.remove(i);
                        self.list.insert(0, t);
                    }
                    None => {
                        self.list.insert(0, t);
                        self.list.truncate(MAX_TASKS);
                    }
                }
                Notice::None
            }
        }
    }

    /// Убрать задачи, о которых давно не сообщали. true — что-то пропало.
    pub fn expire(&mut self, now: Instant) -> bool {
        let n = self.list.len();
        self.list.retain(|t| now.duration_since(t.updated) < TASK_STALE);
        self.list.len() != n
    }

    /// Прогресс для полоски в свёрнутой пилюле: первая из показанных задач, у которой он есть.
    pub fn bar(&self) -> Option<u8> {
        self.list.iter().take(SHOWN_TASKS).find_map(|t| t.progress).map(|p| p.min(100))
    }

    /// Для свёрнутой пилюли: «Рендер 45%», «Загрузка…».
    pub fn badges(&self) -> Vec<String> {
        self.list
            .iter()
            .take(SHOWN_TASKS)
            .map(|t| match t.progress {
                Some(p) => format!("{} {}%", t.title, p),
                None => format!("{}…", t.title),
            })
            .collect()
    }
}

/// Очередь событий от потока API (он кладёт, UI забирает по WM_APP_API).
pub static QUEUE: std::sync::Mutex<Vec<Event>> = std::sync::Mutex::new(Vec::new());

/// WM_APP_API: применить всё, что пришло.
pub unsafe fn on_events(hwnd: HWND) {
    let events = std::mem::take(&mut *QUEUE.lock().unwrap());
    let now = Instant::now();
    let mut flash: Option<(String, bool, u32)> = None;
    for ev in events {
        match ev {
            Event::Notify { text, secs } => flash = Some((text, false, secs * 1000)),
            Event::Task { id, title, progress, state } => match st(|s| s.tasks.apply(id, title, progress, state, now)) {
                Notice::None => {}
                Notice::Done(name) => flash = Some((format!("✓ {}", name), false, 4000)),
                Notice::Failed(name) => flash = Some((format!("✗ {}: ошибка", name), true, 8000)),
            },
            Event::Open(tab) => {
                use crate::app::Tab;
                let t = match tab.as_str() {
                    "music" => Some(Tab::Music),
                    "clip" => Some(Tab::Clipboard),
                    "cmd" => Some(Tab::Commands),
                    "ctl" => Some(Tab::Controls),
                    "mon" => Some(Tab::System),
                    "feed" | "notifs" => Some(Tab::Feed),
                    _ => None,
                };
                if let Some(t) = t {
                    st(|s| {
                        s.shell.tab = t;
                        if matches!(t, Tab::Controls | Tab::System) {
                            s.shell.sys_page = t;
                        }
                        s.feed_notifs = tab == "notifs";
                    });
                }
                crate::shell::set_expanded(hwnd, t.is_some());
                crate::shell::layout(hwnd);
            }
            Event::Agent(u) => {
                let id = u.id.clone();
                let notice = st(|s| s.agents.apply(u, now));
                // Ветку git читаем один раз на запрос человека, вне st().
                if let Some(cwd) = st(|s| s.agents.wants_branch(&id)) {
                    let branch = agents::read_branch(&cwd);
                    st(|s| s.agents.set_branch(&id, branch));
                }
                match notice {
                    Some(agents::Notice::Ask { name, text, answerable }) => {
                        let kind = PeekKind::Ask { id, answerable };
                        let pk = Peek { kind, app: "Claude Code".into(), title: format!("{}: нужно разрешение", name), body: text };
                        // С кнопками ждём ответа, без кнопок это просто напоминание.
                        peek::show(hwnd, pk, if answerable { None } else { Some(8000) });
                    }
                    Some(agents::Notice::Done { name, text }) => {
                        let pk = Peek { kind: PeekKind::Done, app: "Claude Code".into(), title: format!("{}: закончил", name), body: text };
                        peek::show(hwnd, pk, Some(4000));
                    }
                    Some(agents::Notice::Resolved) => {
                        crate::bg::api::answer(&id, None);
                        peek::resolve(hwnd, &id);
                    }
                    None => {}
                }
            }
        }
    }
    match flash {
        Some((text, warn, ms)) => osd::show(hwnd, Flash::Text(text, warn), ms),
        None => crate::shell::layout(hwnd),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(s: &str) -> (Reply, Option<Event>) {
        handle(s.as_bytes())
    }

    #[test]
    fn decode() {
        assert_eq!(url_decode("%D0%A0%D0%B5%D0%BD%D0%B4%D0%B5%D1%80+ok").as_deref(), Some("Рендер ok"));
        assert_eq!(url_decode("%ZZ"), None);
        assert_eq!(url_decode("%D0"), None);
    }

    #[test]
    fn notify_and_task() {
        let (r, e) = req("GET /notify?text=%D0%93%D0%BE%D1%82%D0%BE%D0%B2%D0%BE&secs=99 HTTP/1.1\r\nHost: 127.0.0.1:47821\r\n\r\n");
        assert_eq!(r.code, 200);
        assert_eq!(e, Some(Event::Notify { text: "Готово".into(), secs: 60 }));

        let body = "Привет из скрипта";
        let raw = format!("POST /notify HTTP/1.1\r\nHost: localhost\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
        assert_eq!(expected_len(raw.as_bytes()), Some(raw.len()));
        assert_eq!(handle(raw.as_bytes()).1, Some(Event::Notify { text: body.into(), secs: 4 }));

        let (_, e) = req("POST /task HTTP/1.1\r\nHost: localhost\r\nContent-Length: 29\r\n\r\nid=r&title=Render&progress=45");
        assert_eq!(e, Some(Event::Task { id: "r".into(), title: Some("Render".into()), progress: Some(45), state: TaskState::Run }));
        let (_, e) = req("GET /task?id=r&state=done HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert_eq!(e, Some(Event::Task { id: "r".into(), title: None, progress: None, state: TaskState::Done }));
    }

    #[test]
    fn rejects() {
        // Браузер с чужого сайта.
        assert_eq!(req("GET /notify?text=x HTTP/1.1\r\nHost: localhost\r\nOrigin: https://evil.example\r\n\r\n").0.code, 403);
        assert_eq!(req("GET /notify?text=x HTTP/1.1\r\nHost: localhost\r\nSec-Fetch-Mode: no-cors\r\n\r\n").0.code, 403);
        // DNS rebinding.
        assert_eq!(req("GET /notify?text=x HTTP/1.1\r\nHost: evil.example:47821\r\n\r\n").0.code, 403);
        assert_eq!(req("GET /notify HTTP/1.1\r\nHost: localhost\r\n\r\n").0.code, 400);
        assert_eq!(req("GET /task?state=done HTTP/1.1\r\nHost: localhost\r\n\r\n").0.code, 400);
        assert_eq!(req("GET /task?id=a&state=what HTTP/1.1\r\nHost: localhost\r\n\r\n").0.code, 400);
        assert_eq!(req("GET /nope HTTP/1.1\r\nHost: localhost\r\n\r\n").0.code, 404);
        assert_eq!(req("DELETE /ping HTTP/1.1\r\nHost: localhost\r\n\r\n").0.code, 400);
        assert_eq!(req("GET /ping HTTP/1.1\r\nHost: localhost\r\n\r\n").0, PONG);
    }

    #[test]
    fn agents_endpoints() {
        let body = r#"{"session_id":"s1","cwd":"C:\\p\\pill","hook_event_name":"Stop"}"#;
        let raw = format!("POST /hook HTTP/1.1\r\nHost: 127.0.0.1:47821\r\nContent-Length: {}\r\n\r\n{}", body.len(), body);
        let (r, e) = handle(raw.as_bytes());
        assert_eq!((r.code, r.body), (200, ""));
        assert_eq!(e, Some(Event::Agent(agents::Update { id: "s1".into(), name: Some("pill".into()), state: AgentState::Done, note: None, answerable: false, extra: agents::Extra { cwd: Some("C:\\p\\pill".into()), ..Default::default() } })));
        // Тело длиннее лимита: поля в начале всё равно читаются.
        let big = format!(r#"{{"session_id":"s2","hook_event_name":"UserPromptSubmit","prompt":"{}"#, "я".repeat(MAX_REQUEST));
        let raw = format!("POST /hook HTTP/1.1\r\nHost: localhost\r\n\r\n{}", big);
        let (r, e) = handle(raw.as_bytes());
        assert_eq!(r.code, 200);
        assert!(matches!(e, Some(Event::Agent(agents::Update { state: AgentState::Work, note: None, .. }))));
        // Неинтересное событие хука — молча ок.
        assert_eq!(req("POST /hook HTTP/1.1\r\nHost: localhost\r\n\r\n{\"session_id\":\"s\",\"hook_event_name\":\"SubagentStop\"}"), (QUIET, None));
        let (_, e) = req("GET /agent?id=a&name=bot&state=ask&text=%D0%BC%D0%BE%D0%B6%D0%BD%D0%BE%3F HTTP/1.1\r\nHost: localhost\r\n\r\n");
        assert_eq!(e, Some(Event::Agent(agents::Update { id: "a".into(), name: Some("bot".into()), state: AgentState::Ask, note: Some("можно?".into()), answerable: false, extra: agents::Extra::default() })));
        assert_eq!(req("GET /agent?name=bot HTTP/1.1\r\nHost: localhost\r\n\r\n").0.code, 400);
        assert_eq!(req("GET /agent?id=a&state=zzz HTTP/1.1\r\nHost: localhost\r\n\r\n").0.code, 400);
    }

    #[test]
    fn tasks_flow() {
        let mut t = Tasks::default();
        let now = Instant::now();
        assert_eq!(t.apply("r".into(), Some("Рендер".into()), Some(10), TaskState::Run, now), Notice::None);
        t.apply("u".into(), Some("Загрузка".into()), None, TaskState::Run, now);
        assert_eq!(t.badges(), vec!["Загрузка…".to_string(), "Рендер 10%".to_string()]);
        // Обновление без названия сохраняет старое и поднимает задачу наверх.
        t.apply("r".into(), None, Some(55), TaskState::Run, now);
        assert_eq!(t.badges()[0], "Рендер 55%");
        assert_eq!(t.apply("r".into(), None, None, TaskState::Done, now), Notice::Done("Рендер".into()));
        assert_eq!(t.list.len(), 1);
        assert_eq!(t.apply("x".into(), None, None, TaskState::Error, now), Notice::Failed("x".into()));
        assert!(!t.expire(now));
        assert!(t.expire(now + TASK_STALE + Duration::from_secs(1)));
        assert!(t.list.is_empty());
    }
}
