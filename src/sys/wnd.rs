//! Чужие окна: имя программы окна, заголовок активного окна, поиск окна по заголовку.
//! Про `State` ничего не знает.

use windows::core::PWSTR;
use windows::Win32::Foundation::*;
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Имя файла программы, которой принадлежит окно: «Telegram.exe».
pub fn process_name(hwnd: HWND) -> Option<String> {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 520];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit('\\').next().map(str::to_string)
    }
}

pub fn title(hwnd: HWND) -> String {
    unsafe {
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, &mut buf).max(0) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

pub fn foreground_title() -> String {
    unsafe { title(GetForegroundWindow()) }
}

/// Заголовок содержит имя отдельным словом (без учёта регистра): «pill — Visual Studio Code» да,
/// «spillway» нет.
pub fn title_has(title: &str, name: &str) -> bool {
    let (t, n) = (title.to_lowercase(), name.to_lowercase());
    if n.is_empty() {
        return false;
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut from = 0;
    while let Some(pos) = t[from..].find(&n) {
        let at = from + pos;
        let before = t[..at].chars().next_back();
        let after = t[at + n.len()..].chars().next();
        if !word(before) && !word(after) {
            return true;
        }
        from = at + n.len().max(1);
    }
    false
}

/// Видимые окна верхнего уровня (кроме своих).
fn top_windows() -> Vec<HWND> {
    unsafe extern "system" fn each(h: HWND, l: LPARAM) -> BOOL {
        let v = &mut *(l.0 as *mut Vec<HWND>);
        if IsWindowVisible(h).as_bool() && GetWindow(h, GW_OWNER).map(|o| o.0.is_null()).unwrap_or(true) {
            v.push(h);
        }
        TRUE
    }
    let mut v: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut v as *mut _ as isize));
        let me = GetCurrentProcessId();
        v.retain(|&h| {
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            pid != me
        });
    }
    v
}

/// Поднять окно, в заголовке которого есть имя (папка проекта агента): терминал или редактор.
/// Окна терминалов и редакторов в приоритете. false — подходящего окна нет.
pub fn focus_titled(name: &str) -> bool {
    let rank = |h: HWND| {
        let p = process_name(h).unwrap_or_default().to_lowercase();
        match p.as_str() {
            "windowsterminal.exe" | "wezterm-gui.exe" | "alacritty.exe" | "conhost.exe" | "openconsole.exe" => 0,
            "code.exe" | "cursor.exe" | "windsurf.exe" => 1,
            _ => 2,
        }
    };
    let mut best: Option<(i32, HWND)> = None;
    for h in top_windows() {
        if !title_has(&title(h), name) {
            continue;
        }
        let r = rank(h);
        if best.is_none_or(|b| r < b.0) {
            best = Some((r, h));
        }
    }
    let Some((_, h)) = best else { return false };
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        crate::sys::launch::force_foreground(h);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::title_has;

    #[test]
    fn title_words() {
        assert!(title_has("feed.rs - pill - Visual Studio Code", "pill"));
        assert!(title_has("PILL: claude", "pill"));
        assert!(!title_has("spillway notes", "pill"));
        assert!(!title_has("pills", "pill"));
        assert!(title_has("site — npm run build", "site"));
        assert!(!title_has("anything", ""));
    }
}
