//! Клавиатура и строка поиска (программы, буфер, действия): фокус, символы, стрелки, вставка.

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::InvalidateRect;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::app::{hwnd_of, st, State, Tab};
use crate::feat::cmd::{self, CmdRes, MODES, MODE_CLIP};
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
    /// Выбранная строка и прокрутка списка.
    pub sel: usize,
    pub scroll: usize,
    pending_paste: bool,
    /// Режим строки поиска (`cmd::MODE_*`).
    pub mode: u8,
    /// Вкладка, с которой открыли поиск: на неё возвращаемся, когда панель свернётся.
    pub back: Option<Tab>,
}

/// Открывает строку поиска в режиме `mode` и включает ввод: окно становится активным.
pub unsafe fn activate(hwnd: HWND, mode: u8) {
    let fg = GetForegroundWindow();
    let apps = cmd::scan();
    st(|s| {
        if fg != hwnd {
            s.input.prev_fg = fg.0 as isize;
        }
        if s.shell.tab != Tab::Commands {
            s.input.back = Some(s.shell.tab);
            s.input.text.clear();
        } else if !s.input.active || s.input.mode != mode {
            s.input.text.clear();
        }
        s.shell.tab = Tab::Commands;
        s.input.mode = mode;
        s.input.active = true;
        s.input.sel = 0;
        s.input.scroll = 0;
        s.cmd.apps = apps;
    });
    set_expanded(hwnd, true);
    sys::launch::force_foreground(hwnd);
    let _ = SetFocus(hwnd);
    let _ = InvalidateRect(hwnd, None, false);
}

/// Сменить режим (Tab или клик по режиму). Набранный текст остаётся.
pub unsafe fn switch_mode(hwnd: HWND, mode: u8) {
    let typing = st(|s| s.shell.tab == Tab::Commands && s.input.active);
    if !typing {
        activate(hwnd, mode);
        return;
    }
    st(|s| {
        s.input.mode = mode;
        s.input.sel = 0;
        s.input.scroll = 0;
    });
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

/// Хоткей: повторное нажатие в том же режиме закрывает панель.
pub unsafe fn on_hotkey(hwnd: HWND, mode: u8) {
    let same = st(|s| s.input.active && s.shell.expanded && s.shell.tab == Tab::Commands && s.input.mode == mode);
    if same {
        close_panel(hwnd, true);
    } else {
        activate(hwnd, mode);
    }
}

/// Запись буфера под выбранной строкой (только в режиме «Буфер»).
fn selected_clip(s: &State) -> Option<usize> {
    if s.input.mode != MODE_CLIP {
        return None;
    }
    match cmd::results(s).get(s.input.sel) {
        Some(CmdRes::Clip(i)) => Some(*i),
        _ => None,
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
        0x10 => {
            // Ctrl+P — закрепить выбранную запись буфера.
            st(|s| {
                if let Some(i) = selected_clip(s) {
                    s.clip.toggle_pin(i);
                }
            });
            let _ = InvalidateRect(hwnd, None, false);
            return;
        }
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
    // Alt+1…9 — выполнить строку по номеру.
    if (0x31..=0x39).contains(&vk) && GetKeyState(VK_MENU.0 as i32) < 0 {
        cmd::run_index(hwnd, (vk - 0x31) as usize);
        return;
    }
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
        VK_RETURN => {
            let sel = st(|s| s.input.sel);
            cmd::run_index(hwnd, sel);
        }
        VK_UP | VK_DOWN => {
            st(|s| {
                let n = cmd::results(s).len();
                if n == 0 {
                    return;
                }
                let i = &mut s.input;
                i.sel = if vk == VK_UP.0 { i.sel.saturating_sub(1) } else { (i.sel + 1).min(n - 1) };
                // Прокручивается только длинный список режима «Буфер».
                let rows = ui::search::CLIP_ROWS;
                if i.mode != MODE_CLIP {
                    i.scroll = 0;
                } else if i.sel < i.scroll {
                    i.scroll = i.sel;
                } else if i.sel >= i.scroll + rows {
                    i.scroll = i.sel + 1 - rows;
                }
            });
            let _ = InvalidateRect(hwnd, None, false);
        }
        VK_DELETE => {
            // Del в режиме «Буфер» — убрать выбранную запись.
            st(|s| {
                if let Some(i) = selected_clip(s) {
                    s.clip.remove(i);
                    let n = cmd::results(s).len();
                    s.input.sel = s.input.sel.min(n.saturating_sub(1));
                }
            });
            let _ = InvalidateRect(hwnd, None, false);
        }
        VK_TAB => {
            let back = GetKeyState(VK_SHIFT.0 as i32) < 0;
            let n = MODES.len() as u8;
            let mode = st(|s| (s.input.mode + if back { n - 1 } else { 1 }) % n);
            switch_mode(hwnd, mode);
        }
        _ => {}
    }
}

/// Колесо в режиме «Буфер»: прокрутка по две строки.
pub fn scroll_list(s: &mut State, notches: i32) {
    if s.input.mode != MODE_CLIP {
        return;
    }
    let n = cmd::results(s).len();
    let max = n.saturating_sub(ui::search::CLIP_ROWS);
    s.input.scroll = (s.input.scroll as i32 - notches * 2).clamp(0, max as i32) as usize;
}
