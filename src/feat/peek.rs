//! Карточка, выезжающая из чёлки: чужое уведомление, просьба агента о разрешении (с кнопками),
//! «агент закончил». Больше свёрнутой чёлки, меньше раскрытой панели; размер меняется плавно.
//! Уведомления и «закончил» прячутся сами (TIMER_PEEK), просьба о разрешении ждёт ответа.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::app::{st, Redraw};
use crate::feat::agents::AgentState;
use crate::ids::TIMER_PEEK;
use crate::shell::layout;

/// Ширина карточки (логические px).
pub const W: i32 = 440;
/// На сколько кнопка «Тишина на час» выключает карточки уведомлений.
const QUIET: Duration = Duration::from_secs(3600);

pub enum PeekKind {
    Notif,
    /// Агент ждёт разрешения. answerable — хук держит соединение, ответить можно прямо отсюда.
    Ask { id: String, answerable: bool },
    Done,
}

pub struct Peek {
    pub kind: PeekKind,
    /// Маленькая строка: программа.
    pub app: String,
    pub title: String,
    pub body: String,
    /// Уведомление: последние сообщения этого чата (до трёх, старые сверху) и сколько их всего.
    pub lines: Vec<String>,
    pub count: usize,
    /// Уведомление: идентификатор программы, чтобы открыть её кнопкой.
    pub aumid: String,
    /// Курсор над карточкой: у уведомления появляются кнопки.
    pub hover: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PeekHit {
    Allow,
    Deny,
    /// Клик по самой карточке или «Скрыть»: убрать.
    Dismiss,
    /// Открыть программу, приславшую уведомление.
    Open,
    /// Час не показывать карточки уведомлений.
    Quiet,
}

impl Peek {
    pub fn new(kind: PeekKind, app: String, title: String, body: String) -> Peek {
        Peek { kind, app, title, body, lines: Vec::new(), count: 1, aumid: String::new(), hover: false }
    }
    pub fn answerable(&self) -> bool {
        matches!(self.kind, PeekKind::Ask { answerable: true, .. })
    }
    /// Высота карточки по содержимому.
    pub fn height(&self) -> i32 {
        match self.kind {
            PeekKind::Notif => {
                // Одно сообщение — до двух строк текста, несколько — по строке на каждое.
                let text = if self.lines.len() > 1 { self.lines.len() as i32 * 20 } else { 40 };
                38 + text + if self.hover { 50 } else { 12 }
            }
            _ if self.answerable() => 128,
            _ => 86,
        }
    }
}

/// Показать карточку. ms — через сколько спрятать (None — держать, пока не ответят).
/// Просьбу о разрешении уведомление не перебивает.
pub unsafe fn show(hwnd: HWND, peek: Peek, ms: Option<u32>) {
    if st(|s| s.settings.dot_mode) {
        // «Просто точка»: карточку не показываем, о событии говорит маскот в свёрнутой чёлке.
        // Ждущий хук отпускаем сразу: Claude Code спросит в своём окне.
        if let PeekKind::Ask { id, answerable: true } = &peek.kind {
            crate::bg::api::answer(id, None);
        }
        // Ширина свёрнутой чёлки могла поменяться (счётчик пропущенных).
        layout(hwnd);
        return;
    }
    let shown = st(|s| {
        let busy = s.peek.as_ref().is_some_and(|p| p.answerable());
        if busy && !matches!(peek.kind, PeekKind::Ask { .. }) {
            return false;
        }
        s.peek = Some(peek);
        s.hover = None;
        true
    });
    if !shown {
        return;
    }
    let _ = KillTimer(hwnd, TIMER_PEEK);
    if let Some(ms) = ms {
        SetTimer(hwnd, TIMER_PEEK, ms, None);
    }
    layout(hwnd);
}

pub unsafe fn hide(hwnd: HWND) {
    let _ = KillTimer(hwnd, TIMER_PEEK);
    if st(|s| s.peek.take().is_some()) {
        layout(hwnd);
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

/// Агент перестал ждать разрешения (ответили в его окне, вышло время): убрать его карточку.
pub unsafe fn resolve(hwnd: HWND, id: &str) {
    let ours = st(|s| matches!(&s.peek, Some(Peek { kind: PeekKind::Ask { id: i, .. }, .. }) if i == id));
    if ours {
        hide(hwnd);
    }
}

pub unsafe fn on_hit(hwnd: HWND, h: PeekHit) -> Redraw {
    let (ask, aumid) = st(|s| match &s.peek {
        Some(Peek { kind: PeekKind::Ask { id, .. }, .. }) => (Some(id.clone()), String::new()),
        Some(p) => (None, p.aumid.clone()),
        None => (None, String::new()),
    });
    if let Some(id) = ask {
        // Dismiss отпускает хук без решения: Claude Code спросит в своём окне.
        let decision = match h {
            PeekHit::Allow => Some(true),
            PeekHit::Deny => Some(false),
            _ => None,
        };
        crate::bg::api::answer(&id, decision);
        if let Some(allow) = decision {
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
    Redraw::None
}
