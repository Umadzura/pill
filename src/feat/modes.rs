//! Режимы ПК: Обычный, Монтаж, Рендер, Игра. Не обычный режим — это тишина от Pill (карточки
//! чужих уведомлений не выезжают, панель сама не раскрывается; просьбы агентов остаются) и вежливое
//! закрытие программ из списка режима. Включается вручную; когда активной становится программа из
//! списка режима, пилюля один раз предлагает его включить (событие смены окна, без опроса).

use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::InvalidateRect;

use crate::app::st;
use crate::feat::peek::{self, Peek, PeekKind};
use crate::shell::layout;
use crate::sys;

pub const NAMES: [&str; 4] = ["Обычный", "Монтаж", "Рендер", "Игра"];

/// Сейчас тишина от Pill (режим не обычный).
pub fn silent(mode: u8) -> bool {
    mode != 0
}

/// Включить режим. При включении не обычного режима закрываются программы из его списка.
pub unsafe fn set(hwnd: HWND, m: u8) {
    let m = m.min(3);
    let cfg = st(|s| {
        s.settings.pc_mode = m;
        s.settings.clone()
    });
    cfg.save();
    if m > 0 {
        sys::wnd::close_by_names(&cfg.mode_close[m as usize - 1]);
    }
    layout(hwnd);
    let _ = InvalidateRect(hwnd, None, false);
}

/// Режим, в чей список «программы режима» входит это имя файла.
pub fn mode_for(apps: &[Vec<String>; 3], exe: &str) -> Option<u8> {
    apps.iter().position(|l| l.iter().any(|a| a.eq_ignore_ascii_case(exe))).map(|i| i as u8 + 1)
}

/// Сменилось активное окно: если это программа из списка какого-то режима и режим обычный,
/// один раз за запуск этой программы предложить его включить.
pub unsafe fn on_foreground(hwnd: HWND, fg: HWND) {
    if fg.0.is_null() || fg == hwnd {
        return;
    }
    let Some(exe) = sys::wnd::process_name(fg) else { return };
    let offer = st(|s| {
        if s.settings.pc_mode != 0 || s.mode_hint.eq_ignore_ascii_case(&exe) {
            return None;
        }
        let m = mode_for(&s.settings.mode_apps, &exe)?;
        s.mode_hint = exe.clone();
        Some(m)
    });
    let Some(m) = offer else { return };
    let app = exe.trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
    let pk = Peek::new(PeekKind::Mode(m), app, format!("Включить «{}»?", NAMES[m as usize]), "тишина от Pill и закрытие лишних программ".into());
    peek::show(hwnd, pk, Some(8000));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_mode() {
        let apps = [vec!["Resolve.exe".to_string()], vec!["blender.exe".to_string()], vec![]];
        assert_eq!(mode_for(&apps, "resolve.EXE"), Some(1));
        assert_eq!(mode_for(&apps, "blender.exe"), Some(2));
        assert_eq!(mode_for(&apps, "chrome.exe"), None);
    }
}
