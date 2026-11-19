//! Баннеры Windows по программам: `HKCU\...\Notifications\Settings\<AUMID>`, значение `ShowBanner`.
//! Нет значения — баннер показывается (так по умолчанию). `ShowInActionCenter` не трогаем никогда:
//! Pill читает уведомления из центра уведомлений, без него сообщения пропадут и из чёлки.

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;

const SETTINGS: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Notifications\\Settings";

unsafe fn dword(key: &HSTRING, name: PCWSTR) -> Option<u32> {
    let mut v = 0u32;
    let mut sz = 4u32;
    let r = RegGetValueW(HKEY_CURRENT_USER, key, name, RRF_RT_REG_DWORD, None, Some(&mut v as *mut _ as *mut _), Some(&mut sz));
    (r == ERROR_SUCCESS).then_some(v)
}

/// Главный переключатель «Уведомления» в Параметрах. Выключен — ни одна программа не шлёт уведомлений,
/// центр уведомлений пуст и Pill нечего читать.
pub fn toasts_on() -> bool {
    unsafe { dword(&HSTRING::from("Software\\Microsoft\\Windows\\CurrentVersion\\PushNotifications"), w!("ToastEnabled")) != Some(0) }
}

/// Программы, которые хоть раз слали уведомление (Windows заводит им ключ при первом).
pub fn apps() -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(SETTINGS), 0, KEY_READ, &mut key) != ERROR_SUCCESS {
            return out;
        }
        let mut buf = [0u16; 512];
        for i in 0.. {
            let mut len = buf.len() as u32;
            if RegEnumKeyExW(key, i, PWSTR(buf.as_mut_ptr()), &mut len, None, PWSTR::null(), None, None) != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        let _ = RegCloseKey(key);
    }
    out
}

fn app_key(aumid: &str) -> HSTRING {
    HSTRING::from(format!("{}\\{}", SETTINGS, aumid))
}

/// Показывает ли Windows баннеры этой программы.
pub fn banner_shown(aumid: &str) -> bool {
    unsafe { dword(&app_key(aumid), w!("ShowBanner")) != Some(0) }
}

/// Выключить баннеры программы (`ShowBanner = 0`) или вернуть как по умолчанию (значение удаляется).
pub fn set_banner(aumid: &str, show: bool) {
    let key = app_key(aumid);
    unsafe {
        if show {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, &key, w!("ShowBanner"));
        } else {
            let v = 0u32;
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, &key, w!("ShowBanner"), REG_DWORD.0, Some(&v as *const _ as *const _), 4);
        }
    }
}
