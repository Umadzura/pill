//! Светлая/тёмная тема Windows (реестр Personalize + рассылка WM_SETTINGCHANGE).

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const THEME_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");

unsafe fn reg_dword(key: PCWSTR, name: PCWSTR) -> Option<u32> {
    let mut v = 0u32;
    let mut sz = 4u32;
    let r = RegGetValueW(HKEY_CURRENT_USER, key, name, RRF_RT_REG_DWORD, None, Some(&mut v as *mut _ as *mut _), Some(&mut sz));
    (r == ERROR_SUCCESS).then_some(v)
}

unsafe fn set_reg_dword(key: PCWSTR, name: PCWSTR, v: u32) {
    let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key, name, REG_DWORD.0, Some(&v as *const _ as *const _), 4);
}

pub fn dark() -> Option<bool> {
    unsafe { reg_dword(THEME_KEY, w!("AppsUseLightTheme")).map(|v| v == 0) }
}

pub fn set_dark(dark: bool) {
    unsafe {
        let v = if dark { 0 } else { 1 };
        set_reg_dword(THEME_KEY, w!("AppsUseLightTheme"), v);
        set_reg_dword(THEME_KEY, w!("SystemUsesLightTheme"), v);
    }
    // Рассылка может подвиснуть на зависших окнах, поэтому в отдельном потоке.
    std::thread::spawn(|| unsafe {
        let _ = SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("ImmersiveColorSet").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            500,
            None,
        );
    });
}
