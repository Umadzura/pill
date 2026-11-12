//! Автозапуск через HKCU\...\Run.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::*;

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

pub fn enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Pill"), RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

pub fn set(on: bool) {
    unsafe {
        if !on {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Pill"));
            return;
        }
        let Some(exe) = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned()) else { return };
        let val: Vec<u16> = format!("\"{}\"", exe).encode_utf16().chain(Some(0)).collect();
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Pill"), REG_SZ.0, Some(val.as_ptr() as *const _), (val.len() * 2) as u32);
    }
}
