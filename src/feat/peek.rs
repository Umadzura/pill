//! Карточка, выезжающая из чёлки: чужое уведомление, просьба агента о разрешении (с кнопками),
//! «агент закончил». Больше свёрнутой чёлки, меньше раскрытой панели; размер меняется плавно.
//! Уведомления и «закончил» прячутся сами (TIMER_PEEK), просьба о разрешении ждёт ответа.
//! Просьбы, пришедшие, пока одна уже на экране, встают в очередь и показываются по одной.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Input::KeyboardAndMouse::{RegisterHotKey, UnregisterHotKey, MOD_ALT, MOD_NOREPEAT};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::app::{st, Redraw};
use crate::feat::agents::AgentState;
use crate::ids::{HOTKEY_ALLOW, HOTKEY_DENY, HOTKEY_PICK, HOTKEY_PICK_LAST, HOTKEY_TERM, TIMER_PEEK};
use crate::shell::layout;

/// Ширина карточки уведомления и «агент закончил» (логические px).
pub const W: i32 = 440;
/// Ширина карточки с просьбой о разрешении (в раскрытом «подробнее» видны строки кода).
pub const W_ASK: i32 = 520;
/// На сколько кнопка «Тишина на час» выключает карточки уведомлений.
const QUIET: Duration = Duration::from_secs(3600);
/// Сколько после появления просьбы клавиши ответа не действуют: нажатие предназначалось не ей
/// (печатали в другом окне или отвечали на прошлую просьбу, а следующая встала на её место).
const ARM: Duration = Duration::from_millis(500);
/// Строка под целью: предупреждение о файле или «команда длиннее, чем видно».
pub const NOTE_H: i32 = 18;

pub enum PeekKind {
    Notif,
    /// Агент ждёт разрешения. answerable — хук держит соединение, ответить можно прямо отсюда.
    /// id — сессия агента, req — номер ждущего запроса в `bg::api` (ответ уходит только ему).
    Ask { id: String, req: u64, answerable: bool },
    Done,
    /// Снимок экрана из буфера: миниатюра, «Открыть», «На полку», «Сохранить как».
    Snap,
}

pub struct Peek {
    pub kind: PeekKind,
    /// Маленькая строка: программа. У просьбы агента — имя проекта (по нему цвет маскота).
    pub app: String,
    /// Карточка агента: id его сессии (по нему ищется маскот: имена папок у сессий могут совпасть).
    pub agent: String,
    pub title: String,
    pub body: String,
    /// Уведомление: последние сообщения чата (до трёх, старые сверху).
    /// Просьба агента: строки того, что он хочет сделать («-» убрано, «+» добавлено, пробел — команда,
    /// «…» — показано не всё), «!» — предупреждение, «=» — правило «всегда» (`agents::ask_detail`).
    pub lines: Vec<String>,
    /// Уведомление: сколько сообщений в чате.
    pub count: usize,
    /// Уведомление: идентификатор программы, чтобы открыть её кнопкой.
    pub aumid: String,
    /// Курсор над карточкой: у уведомления появляются кнопки.
    pub hover: bool,
    /// Просьба агента: инструмент («Edit: src/ui/feed.rs»), ветка git, сколько строк добавляет и убирает.
    pub tool: String,
    pub branch: String,
    pub stat: (usize, usize),
    /// Просьба агента: можно ли ответить «всегда» (хук прислал готовое правило).
    pub can_always: bool,
    /// Просьба агента: раскрыто «подробнее» (запрос человека и строки кода).
    pub more: bool,
    /// Вопрос агента с вариантами вместо просьбы о разрешении.
    pub question: Option<crate::feat::agents::Question>,
    /// Снимок: PNG-файл и его миниатюра.
    pub file: String,
    pub art: Option<std::sync::Arc<crate::bg::winrt::Art>>,
    /// Когда карточка появилась на экране (у просьбы из очереди — когда до неё дошла очередь).
    pub shown: Instant,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PeekHit {
    Allow,
    /// Разрешить и больше не спрашивать про такое.
    Always,
    Deny,
    /// Клик по самой карточке, «Скрыть» или «В терминал»: убрать без решения.
    Dismiss,
    /// Открыть программу, приславшую уведомление.
    Open,
    /// Час не показывать карточки уведомлений.
    Quiet,
    /// Просьба агента: показать или спрятать подробности.
    More,
    /// Вопрос агента: выбран вариант.
    Pick(u8),
    /// Снимок: положить на полку.
    Shelf,
    /// Снимок: сохранить копию куда-нибудь.
    SaveAs,
}

impl Peek {
    pub fn new(kind: PeekKind, app: String, title: String, body: String) -> Peek {
        Peek { kind, app, agent: String::new(), title, body, lines: Vec::new(), count: 1, aumid: String::new(), hover: false, tool: String::new(), branch: String::new(), stat: (0, 0), can_always: false, more: false, question: None, file: String::new(), art: None, shown: Instant::now() }
    }
    /// Просьба агента: строки кода и пометки «…» (без предупреждения и правила).
    pub fn code(&self) -> impl Iterator<Item = &String> {
        self.lines.iter().filter(|l| !l.starts_with(['!', '=']))
    }
    /// Просьба агента: предупреждение о файле (вне папки проекта, настройки Claude Code).
    pub fn warning(&self) -> Option<&str> {
        self.lines.iter().find_map(|l| l.strip_prefix('!'))
    }
    /// Просьба агента: что именно добавит «всегда».
    pub fn rule(&self) -> Option<&str> {
        self.lines.iter().find_map(|l| l.strip_prefix('='))
    }
    /// Команда не видна на карточке целиком (несколько строк или длинная): «Разрешить» и Alt+Y
    /// не действуют, пока не раскрыто «подробнее».
    pub fn needs_more(&self) -> bool {
        let (name, detail) = self.tool.split_once(": ").unwrap_or((&self.tool, ""));
        matches!(name, "Bash" | "PowerShell") && (self.code().count() > 1 || !crate::feat::agents::hero_fits(detail))
    }
    /// «Разрешить» сейчас нельзя: команду ещё не показали целиком.
    pub fn blocked(&self) -> bool {
        self.needs_more() && !self.more
    }
    /// Есть строка под целью (предупреждение или «открой подробнее»).
    pub fn has_note(&self) -> bool {
        self.warning().is_some() || self.blocked()
    }
    pub fn answerable(&self) -> bool {
        matches!(self.kind, PeekKind::Ask { answerable: true, .. })
    }
    pub fn width(&self) -> i32 {
        if self.answerable() {
            W_ASK
        } else {
            W
        }
    }
    /// Высота блока со строками кода в просьбе агента (0 — показывать нечего).
    pub fn code_height(&self) -> i32 {
        match self.code().count() {
            0 => 0,
            n => n as i32 * 18 + 16,
        }
    }
    /// Варианты, на которые можно ответить отсюда (пусто — вопроса нет или их несколько сразу).
    pub fn picks(&self) -> usize {
        match &self.question {
            Some(q) if q.count == 1 => q.options.len(),
            _ => 0,
        }
    }
    /// Высота карточки по содержимому.
    pub fn height(&self) -> i32 {
        if self.answerable() && self.question.is_some() {
            // Шапка, вопрос в две строки, варианты по строке, «в терминал».
            return 84 + self.picks() as i32 * 40 + 40;
        }
        match self.kind {
            PeekKind::Notif => {
                // Одно сообщение — до двух строк текста, несколько — по строке на каждое.
                let text = if self.lines.len() > 1 { self.lines.len() as i32 * 20 } else { 40 };
                38 + text + if self.hover { 50 } else { 12 }
            }
            // Маскот и одна фраза (под ней, может быть, предупреждение), кнопки; в «подробнее» ещё
            // запрос человека, строки кода и правило «всегда».
            _ if self.answerable() => {
                let note = if self.has_note() { NOTE_H } else { 0 };
                if self.more {
                    let rule = if self.can_always { 22 } else { 0 };
                    78 + note + 22 + self.code_height() + rule + 12 + 28 + 16
                } else {
                    78 + note + 28 + 16
                }
            }
            PeekKind::Done => 78,
            // Миниатюра с подписью, под ней ряд кнопок.
            PeekKind::Snap => 16 + 74 + 14 + 28 + 16,
            _ => 86,
        }
    }
}

/// Клавиши ответа живут только пока просьба на экране: Pill не берёт фокус, поэтому они глобальные.
/// `picks` — сколько вариантов ответа: на них ещё Alt+1…4. У «всегда» клавиши нет: оно добавляет
/// правило навсегда, поэтому только кликом, рядом с самим правилом в «подробнее».
unsafe fn hotkeys(hwnd: HWND, on: bool, picks: usize) {
    let keys = [(HOTKEY_ALLOW, b'Y'), (HOTKEY_DENY, b'N'), (HOTKEY_TERM, b'T')];
    for (id, key) in keys {
        if on {
            let _ = RegisterHotKey(hwnd, id, MOD_ALT | MOD_NOREPEAT, key as u32);
        } else {
            let _ = UnregisterHotKey(hwnd, id);
        }
    }
    for i in 0..=(HOTKEY_PICK_LAST - HOTKEY_PICK) {
        if on && (i as usize) < picks {
            let _ = RegisterHotKey(hwnd, HOTKEY_PICK + i, MOD_ALT | MOD_NOREPEAT, b'1' as u32 + i as u32);
        } else {
            let _ = UnregisterHotKey(hwnd, HOTKEY_PICK + i);
        }
    }
}

/// Показать карточку. ms — через сколько спрятать (None — держать, пока не ответят).
/// Просьбу о разрешении уведомление не перебивает, а вторая просьба встаёт в очередь.
pub unsafe fn show(hwnd: HWND, peek: Peek, ms: Option<u32>) {
    if st(|s| s.settings.dot_mode) {
        // «Просто точка»: карточку не показываем, о событии говорит маскот в свёрнутой чёлке.
        // Ждущий хук отпускаем сразу: Claude Code спросит в своём окне.
        if let PeekKind::Ask { req, answerable: true, .. } = &peek.kind {
            crate::bg::api::answer(*req, None);
        }
        // Ширина свёрнутой чёлки могла поменяться (счётчик пропущенных).
        layout(hwnd);
        return;
    }
    let ask = peek.answerable();
    let picks = peek.picks();
    let shown = st(|s| {
        // Свежий снимок уведомление тоже не перебивает (тост «Ножниц» «Снимок скопирован» приходит следом).
        let snap = s.peek.as_ref().is_some_and(|p| matches!(p.kind, PeekKind::Snap)) && matches!(peek.kind, PeekKind::Notif);
        let busy = s.peek.as_ref().is_some_and(|p| p.answerable());
        if snap {
            return false;
        }
        if busy {
            if ask {
                s.asks.push(peek);
            }
            return false;
        }
        let mut peek = peek;
        peek.shown = Instant::now();
        s.peek = Some(peek);
        s.hover = None;
        true
    });
    if !shown {
        // Очередь выросла: на карточке меняется «ещё N».
        layout(hwnd);
        return;
    }
    let _ = KillTimer(hwnd, TIMER_PEEK);
    if let Some(ms) = ms {
        SetTimer(hwnd, TIMER_PEEK, ms, None);
    }
    hotkeys(hwnd, ask, picks);
    if ask {
        // Просьба важнее: спрятанная пилюля возвращается.
        st(|s| s.shell.hidden = false);
    }
    layout(hwnd);
}

/// Убрать карточку. Если в очереди ждёт ещё просьба, показать её.
pub unsafe fn hide(hwnd: HWND) {
    let _ = KillTimer(hwnd, TIMER_PEEK);
    let (had, next) = st(|s| {
        let had = s.peek.take().is_some();
        let next = if s.asks.is_empty() { None } else { Some(s.asks.remove(0)) };
        (had, next)
    });
    hotkeys(hwnd, false, 0);
    match next {
        Some(p) => show(hwnd, p, None),
        None if had => layout(hwnd),
        None => {}
    }
}

/// Курсор зашёл на карточку или ушёл с неё: у уведомления появляются и пропадают кнопки.
pub unsafe fn set_hover(hwnd: HWND, on: bool) {
    let changed = st(|s| match &mut s.peek {
        Some(p) if matches!(p.kind, PeekKind::Notif) && p.hover != on => {
            p.hover = on;
            true
        }
        _ => false,
    });
    if changed {
        layout(hwnd);
    }
}

/// Агент перестал ждать разрешения (ответили в его окне, вышло время): убрать его карточку,
/// в том числе из очереди. По id сессии: уходят все её просьбы (хуки отпущены `bg::api::release`/`expire`).
pub unsafe fn resolve(hwnd: HWND, id: &str) {
    let same = |p: &Peek| matches!(&p.kind, PeekKind::Ask { id: i, .. } if i == id);
    let ours = st(|s| {
        s.asks.retain(|p| !same(p));
        s.peek.as_ref().is_some_and(same)
    });
    if ours {
        hide(hwnd);
    } else {
        layout(hwnd);
    }
}

pub unsafe fn on_hit(hwnd: HWND, h: PeekHit) -> Redraw {
    if h == PeekHit::More {
        // Подробности меняют только высоту карточки, ответа нет.
        st(|s| {
            if let Some(p) = &mut s.peek {
                p.more = !p.more;
            }
        });
        layout(hwnd);
        return Redraw::None;
    }
    if matches!(h, PeekHit::Allow | PeekHit::Always) && st(|s| s.peek.as_ref().is_some_and(|p| p.answerable() && p.blocked())) {
        // Команда видна не целиком: разрешить можно, только посмотрев её в «подробнее».
        return Redraw::None;
    }
    // Карточка сейчас уедет из-под курсора: сжимающееся окно не должно раскрыться панелью.
    // Только если курсор над окном (щелчок, а не Alt+Y), иначе «ушёл» не придёт и запрет не снимется.
    if crate::shell::cursor_inside(hwnd) {
        crate::shell::hold_hover();
    }
    let (ask, aumid, can_always, who, question, file) = st(|s| match &s.peek {
        Some(p @ Peek { kind: PeekKind::Ask { id, req, .. }, .. }) => (Some((id.clone(), *req)), String::new(), p.can_always, p.app.clone(), p.question.clone(), String::new()),
        Some(p) => (None, p.aumid.clone(), false, String::new(), None, p.file.clone()),
        None => (None, String::new(), false, String::new(), None, String::new()),
    });
    if !file.is_empty() {
        // Снимок: сначала убрать карточку, потом действие (окно сохранения модальное).
        hide(hwnd);
        match h {
            PeekHit::Open => {
                crate::sys::launch::open(&file, None);
            }
            PeekHit::Shelf => crate::feat::shelf::add_paths(hwnd, vec![file]),
            PeekHit::SaveAs => crate::feat::snap::save_as(hwnd, &file),
            _ => {}
        }
        return Redraw::None;
    }
    if let (Some((id, req)), PeekHit::Pick(i)) = (&ask, h) {
        // Ответ на вопрос: хук разрешает инструмент и отдаёт выбранный вариант.
        if let Some(body) = question.as_ref().and_then(|q| crate::feat::agents::answer_json(q, i as usize)) {
            crate::bg::api::answer_body(*req, &body);
            st(|s| s.agents.set_state(id, AgentState::Work));
            hide(hwnd);
        }
        return Redraw::None;
    }
    if let Some((id, req)) = ask {
        // Без решения хук отпускается, и Claude Code спрашивает в своём окне.
        // Отвечаем только запросу этой карточки: другие просьбы той же сессии ждут в очереди.
        let decision = match h {
            PeekHit::Allow => Some(crate::bg::api::Answer::Allow),
            PeekHit::Always if can_always => Some(crate::bg::api::Answer::Always),
            PeekHit::Always => Some(crate::bg::api::Answer::Allow),
            PeekHit::Deny => Some(crate::bg::api::Answer::Deny),
            _ => None,
        };
        let allow = decision.as_ref().map(|d| !matches!(d, crate::bg::api::Answer::Deny));
        crate::bg::api::answer(req, decision);
        if let Some(allow) = allow {
            st(|s| s.agents.set_state(&id, if allow { AgentState::Work } else { AgentState::Wait }));
        }
    }
    match h {
        PeekHit::Quiet => st(|s| s.quiet_until = Some(Instant::now() + QUIET)),
        PeekHit::Open if !aumid.is_empty() => {
            crate::sys::launch::open(&format!("shell:AppsFolder\\{}", aumid), None);
        }
        _ => {}
    }
    hide(hwnd);
    // «В терминал»: заодно поднять окно агента, если найдётся.
    if h == PeekHit::Dismiss && !who.is_empty() {
        crate::sys::wnd::focus_titled(&who);
    }
    Redraw::None
}

/// Горячая клавиша ответа (Alt+Y / N / T, Alt+1…4), пока просьба на экране.
/// Первые `ARM` после появления карточки нажатия не принимаются.
pub unsafe fn on_hotkey(hwnd: HWND, id: i32) {
    let hit = match id {
        HOTKEY_PICK..=HOTKEY_PICK_LAST => PeekHit::Pick((id - HOTKEY_PICK) as u8),
        HOTKEY_ALLOW => PeekHit::Allow,
        HOTKEY_DENY => PeekHit::Deny,
        _ => PeekHit::Dismiss,
    };
    if st(|s| s.peek.as_ref().is_some_and(|p| p.answerable() && p.shown.elapsed() >= ARM)) {
        on_hit(hwnd, hit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(tool: &str, lines: &[&str]) -> Peek {
        let mut p = Peek::new(PeekKind::Ask { id: "s".into(), req: 1, answerable: true }, "pill".into(), "pill".into(), String::new());
        p.tool = tool.into();
        p.lines = lines.iter().map(|l| l.to_string()).collect();
        p
    }

    #[test]
    fn whole_command_first() {
        // Короткая команда видна целиком: разрешать можно сразу.
        let p = ask("Bash: ls -la", &[" ls -la"]);
        assert!(!p.needs_more() && !p.blocked() && !p.has_note());
        assert_eq!(p.height(), 78 + 28 + 16);
        // Несколько строк или длинная строка: сначала «подробнее».
        let mut p = ask("Bash: cd x", &[" cd x", " rm -rf ~"]);
        assert!(p.blocked() && p.has_note());
        assert_eq!(p.height(), 78 + NOTE_H + 28 + 16);
        p.more = true;
        assert!(p.needs_more() && !p.blocked() && !p.has_note());
        assert!(ask("PowerShell: Get-ChildItem -Recurse -Force C:\\Users | Remove-Item", &[" Get-ChildItem -Recurse -Force C:\\Users | Remove-Item"]).blocked());
        // Правка файла: строки кода и так в «подробнее», кнопка не ждёт.
        assert!(!ask("Edit: C:\\p\\a.rs", &["-a", "+b", "+c"]).needs_more());
    }

    #[test]
    fn marks() {
        let mut p = ask("Write: D:\\x.txt", &["!файл вне папки проекта: D:\\x.txt", "+hi", "…ещё 2 строки", "=Write(D:\\**)"]);
        p.can_always = true;
        assert_eq!((p.warning(), p.rule(), p.code().count()), (Some("файл вне папки проекта: D:\\x.txt"), Some("Write(D:\\**)"), 2));
        assert_eq!(p.code_height(), 2 * 18 + 16);
        assert_eq!(p.height(), 78 + NOTE_H + 28 + 16);
        p.more = true;
        assert_eq!(p.height(), 78 + NOTE_H + 22 + (2 * 18 + 16) + 22 + 12 + 28 + 16);
    }
}
