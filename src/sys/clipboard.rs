//! Чтение и запись буфера обмена Windows (только текст).

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::Win32::System::Threading::Sleep;

unsafe fn open(hwnd: HWND) -> bool {
    // Буфер может быть занят другой программой: пара коротких попыток.
    for _ in 0..4 {
        if OpenClipboard(hwnd).is_ok() {
            return true;
        }
        Sleep(15);
    }
    false
}

/// Текст из буфера. None, если там не текст или менеджер паролей попросил не сохранять.
pub unsafe fn read_text(hwnd: HWND) -> Option<String> {
    let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
    if exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok() {
        return None;
    }
    if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() || !open(hwnd) {
        return None;
    }
    let mut result = None;
    if let Ok(h) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
        let hg = HGLOBAL(h.0);
        let p = GlobalLock(hg) as *const u16;
        if !p.is_null() {
            let max = GlobalSize(hg) / 2;
            let mut n = 0;
            while n < max && *p.add(n) != 0 {
                n += 1;
            }
            result = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
            let _ = GlobalUnlock(hg);
        }
    }
    let _ = CloseClipboard();
    result
}

pub unsafe fn set_text(hwnd: HWND, text: &str) -> bool {
    let w: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    if !open(hwnd) {
        return false;
    }
    let mut ok = false;
    if EmptyClipboard().is_ok() {
        if let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(hg) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(hg);
                ok = SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(hg.0)).is_ok();
            }
            if !ok {
                let _ = GlobalFree(hg);
            }
        }
    }
    let _ = CloseClipboard();
    ok
}
