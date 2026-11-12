//! Клавиатура и поле ввода (поиск в буфере, командная строка): фокус, символы, стрелки, вставка.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{hwnd_of, st, State, Tab};
use crate::feat::cmd;
use crate::ids::{TIMER_COLLAPSE, TIMER_PASTE};
use crate::shell::set_expanded;
use crate::{sys, ui};

#[derive(Default)]
pub struct Input {
    pub text: String,
    /// Окно наше активно и принимает клавиатуру.
    pub active: bool,
    /// Где был пользователь до нас: туда возвращаем фокус и вставляем.
    prev_fg: isize,
    /// Выбранная строка и прокрутка списка текущей вкладки.
    pub sel: usize,
    pub scroll: usize,
    pending_paste: bool,
}

/// Включает ввод с клавиатуры: окно становится активным.
pub unsafe fn activate(hwnd: HWND, tab: Tab) {
    let fg = GetForegroundWindow();
    let scan = tab == Tab::Commands;
    let apps = if scan { cmd::scan() } else { Vec::new() };
    st(|s| {
        if fg != hwnd {
            s.input.prev_fg = fg.0 as isize;
        }
        if s.shell.tab != tab || !s.input.active {
            s.input.text.clear();
        }
        s.shell.tab = tab;
        s.input.active = true;
        s.input.sel = 0;
        s.input.scroll = 0;
        if scan {
            s.cmd.apps = apps;
        }
    });
    set_expanded(hwnd, true);
    sys::launch::force_foreground(hwnd);
    let _ = SetFocus(hwnd);
    let _ = InvalidateRect(hwnd, None, false);
}

/// Выключает ввод. restore — вернуть фокус окну, где был пользователь.
pub unsafe fn deactivate(hwnd: HWND, restore: bool) {
    let prev = st(|s| {
        let was = s.input.active;
        s.input.active = false;
        if was {
            s.input.prev_fg
        } else {
            0
        }
    });
    if restore && prev != 0 && GetForegroundWindow() == hwnd {
        let _ = SetForegroundWindow(hwnd_of(prev));
    }
    let _ = InvalidateRect(hwnd, None, false);
}

/// Окно ушло в фон (WM_ACTIVATE / WA_INACTIVE).
pub unsafe fn on_inactive(hwnd: HWND) {
    if st(|s| std::mem::take(&mut s.input.active)) {
        crate::shell::schedule_collapse(hwnd);
        let _ = InvalidateRect(hwnd, None, false);
    }
}

pub unsafe fn close_panel(hwnd: HWND, restore: bool) {
    deactivate(hwnd, restore);
    let _ = KillTimer(hwnd, TIMER_COLLAPSE);
    set_expanded(hwnd, false);
}

/// Кладёт текст в буфер и вставляет его в окно, где работал пользователь.
pub unsafe fn paste(hwnd: HWND, text: &str) {
    if !sys::clipboard::set_text(hwnd, text) {
        return;
    }
    close_panel(hwnd, true);
    st(|s| s.input.pending_paste = true);
    // Даём фокусу вернуться в целевое окно.
    SetTimer(hwnd, TIMER_PASTE, 80, None);
}

pub unsafe fn on_paste_timer(hwnd: HWND) {
    let _ = KillTimer(hwnd, TIMER_PASTE);
    if st(|s| std::mem::take(&mut s.input.pending_paste)) {
        sys::launch::send_ctrl_v();
    }
}

/// Хоткей: повторное нажатие на той же вкладке закрывает панель.
pub unsafe fn on_hotkey(hwnd: HWND, tab: Tab) {
    let (active, cur) = st(|s| (s.input.active && s.shell.expanded, s.shell.tab));
    if active && cur == tab {
        close_panel(hwnd, true);
    } else {
        activate(hwnd, tab);
    }
}

/// Сколько строк в списке текущей вкладки (для стрелок).
fn list_len(s: &State) -> usize {
    match s.shell.tab {
        Tab::Clipboard => s.clip.view(&s.input.text).len(),
        Tab::Commands => cmd::results(s).len(),
        _ => 0,
    }
}

unsafe fn on_enter(hwnd: HWND) {
    let (tab, sel) = st(|s| (s.shell.tab, s.input.sel));
    match tab {
        Tab::Clipboard => {
            let text = st(|s| s.clip.view(&s.input.text).get(sel).map(|&i| s.clip.items[i].text.clone()));
            if let Some(t) = text {
                paste(hwnd, &t);
            }
        }
        Tab::Commands => cmd::run_index(hwnd, sel),
        _ => {}
    }
}

pub unsafe fn on_char(hwnd: HWND, c: u32) {
    match c {
        0x1B | 0x0D | 0x09 => return, // Esc, Enter, Tab — в WM_KEYDOWN
        0x08 => st(|s| {
            s.input.text.pop();
        }),
        0x7F => st(|s| {
            // Ctrl+Backspace — удалить слово.
            let t = s.input.text.trim_end().len();
            s.input.text.truncate(t);
            let cut = s.input.text.rfind(' ').map(|p| p + 1).unwrap_or(0);
            s.input.text.truncate(cut);
        }),
        0x16 => {
            // Ctrl+V в поле ввода — первая строка буфера.
            if let Some(t) = sys::clipboard::read_text(hwnd) {
                let line = t.lines().next().unwrap_or("").to_string();
                st(|s| s.input.text.push_str(&line));
            }
        }
        c if c >= 0x20 => {
            if let Some(ch) = char::from_u32(c) {
                st(|s| s.input.text.push(ch));
            }
        }
        _ => return,
    }
    st(|s| {
        s.input.sel = 0;
        s.input.scroll = 0;
    });
    let _ = InvalidateRect(hwnd, None, false);
}

pub unsafe fn on_key(hwnd: HWND, vk: u16) {
    match VIRTUAL_KEY(vk) {
        VK_ESCAPE => {
            let had = st(|s| {
                let had = !s.input.text.is_empty();
                s.input.text.clear();
                s.input.sel = 0;
                s.input.scroll = 0;
                had
            });
            if had {
                let _ = InvalidateRect(hwnd, None, false);
            } else {
                close_panel(hwnd, true);
            }
        }
        VK_RETURN => on_enter(hwnd),
        VK_UP | VK_DOWN => {
            st(|s| {
                let n = list_len(s);
                if n == 0 {
                    return;
                }
                let i = &mut s.input;
                i.sel = if vk == VK_UP.0 { i.sel.saturating_sub(1) } else { (i.sel + 1).min(n - 1) };
                let rows = ui::clip::ROWS;
                if i.sel < i.scroll {
                    i.scroll = i.sel;
                } else if i.sel >= i.scroll + rows {
                    i.scroll = i.sel + 1 - rows;
                }
            });
            let _ = InvalidateRect(hwnd, None, false);
        }
        VK_TAB => {
            let tab = st(|s| s.shell.tab);
            let next = if tab == Tab::Commands { Tab::Clipboard } else { Tab::Commands };
            activate(hwnd, next);
        }
        _ => {}
    }
}

/// Колесо во вкладке «Буфер»: прокрутка по две строки.
pub fn scroll_clip(s: &mut State, notches: i32) {
    let n = s.clip.view(&s.input.text).len();
    let max = n.saturating_sub(ui::clip::ROWS);
    s.input.scroll = (s.input.scroll as i32 - notches * 2).clamp(0, max as i32) as usize;
}
