//! Карточка, выезжающая из чёлки: чужое уведомление, просьба агента о разрешении (с кнопками),
//! «агент закончил». Больше свёрнутой чёлки, меньше раскрытой панели; размер меняется плавно.
//! Уведомления и «закончил» прячутся сами (TIMER_PEEK), просьба о разрешении ждёт ответа.

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};

use crate::app::{st, Redraw};
use crate::feat::agents::AgentState;
use crate::ids::TIMER_PEEK;
use crate::shell::layout;

/// Ширина карточки (логические px).
pub const W: i32 = 420;

pub enum PeekKind {
    Notif,
    /// Агент ждёт разрешения. answerable — хук держит соединение, ответить можно прямо отсюда.
    Ask { id: String, answerable: bool },
    Done,
}

pub struct Peek {
    pub kind: PeekKind,
    /// Маленькая строка сверху: программа.
    pub app: String,
    pub title: String,
    pub body: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PeekHit {
    Allow,
    Deny,
    /// Клик по самой карточке: убрать.
    Dismiss,
}

impl Peek {
    pub fn answerable(&self) -> bool {
        matches!(self.kind, PeekKind::Ask { answerable: true, .. })
    }
    /// Высота карточки: с кнопками выше.
    pub fn height(&self) -> i32 {
        if self.answerable() {
            128
        } else {
            86
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

/// Агент перестал ждать разрешения (ответили в его окне, вышло время): убрать его карточку.
pub unsafe fn resolve(hwnd: HWND, id: &str) {
    let ours = st(|s| matches!(&s.peek, Some(Peek { kind: PeekKind::Ask { id: i, .. }, .. }) if i == id));
    if ours {
        hide(hwnd);
    }
}

pub unsafe fn on_hit(hwnd: HWND, h: PeekHit) -> Redraw {
    let ask = st(|s| match &s.peek {
        Some(Peek { kind: PeekKind::Ask { id, .. }, .. }) => Some(id.clone()),
        _ => None,
    });
    if let Some(id) = ask {
        // Dismiss отпускает хук без решения: Claude Code спросит в своём окне.
        let decision = match h {
            PeekHit::Allow => Some(true),
            PeekHit::Deny => Some(false),
            PeekHit::Dismiss => None,
        };
        crate::bg::api::answer(&id, decision);
        if let Some(allow) = decision {
            st(|s| s.agents.set_state(&id, if allow { AgentState::Work } else { AgentState::Wait }));
        }
    }
    hide(hwnd);
    Redraw::None
}
