//! Кто сейчас держит микрофон и камеру. Windows отмечает это в реестре
//! (CapabilityAccessManager\ConsentStore): у занятого устройства LastUsedTimeStop = 0.
//! Фоновый поток спит на RegNotifyChangeKeyValue и просыпается только при изменениях.

use std::sync::Mutex;

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects, INFINITE};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::ids::WM_APP_PRIVACY;

const BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\";

/// Имена приложений, которые сейчас используют микрофон и камеру.
#[derive(Clone, Default, PartialEq)]
pub struct InUse {
    pub mic: Vec<String>,
    pub cam: Vec<String>,
}

pub static IN_USE: Mutex<InUse> = Mutex::new(InUse { mic: Vec::new(), cam: Vec::new() });

pub fn start(hwnd: HWND) {
    let raw = hwnd.0 as isize;
    std::thread::spawn(move || unsafe { watch(raw) });
}

unsafe fn watch(hwnd: isize) {
    let mut keys = [HKEY::default(); 2];
    let mut events = [HANDLE::default(); 2];
    for (i, cap) in ["microphone", "webcam"].iter().enumerate() {
        let path = HSTRING::from(format!("{}{}", BASE, cap));
        if RegOpenKeyExW(HKEY_CURRENT_USER, &path, 0, KEY_READ | KEY_NOTIFY, &mut keys[i]) != ERROR_SUCCESS {
            return;
        }
        let Ok(ev) = CreateEventW(None, false, false, None) else { return };
        events[i] = ev;
        arm(keys[i], ev);
    }
    loop {
        let now = InUse { mic: scan(keys[0]), cam: scan(keys[1]) };
        let changed = {
            let mut cur = IN_USE.lock().unwrap();
            let c = *cur != now;
            *cur = now;
            c
        };
        if changed {
            let _ = PostMessageW(HWND(hwnd as *mut _), WM_APP_PRIVACY, WPARAM(0), LPARAM(0));
        }
        let r = WaitForMultipleObjects(&events, false, INFINITE).0 as usize;
        if r >= events.len() {
            return;
        }
        // Уведомление одноразовое: взводим заново до пересчёта, чтобы не пропустить изменение.
        arm(keys[r], events[r]);
    }
}

unsafe fn arm(key: HKEY, ev: HANDLE) {
    let _ = RegNotifyChangeKeyValue(key, true, REG_NOTIFY_CHANGE_NAME | REG_NOTIFY_CHANGE_LAST_SET, ev, true);
}

/// Подключи ключа: и UWP-приложения, и обычные программы в NonPackaged.
unsafe fn subkeys(key: HKEY) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = [0u16; 512];
    for i in 0.. {
        let mut len = buf.len() as u32;
        let r = RegEnumKeyExW(key, i, PWSTR(buf.as_mut_ptr()), &mut len, None, PWSTR::null(), None, None);
        if r != ERROR_SUCCESS {
            break;
        }
        out.push(String::from_utf16_lossy(&buf[..len as usize]));
    }
    out
}

unsafe fn qword(key: HKEY, sub: &str, name: PCWSTR) -> Option<u64> {
    let mut v = 0u64;
    let mut sz = 8u32;
    let sub = HSTRING::from(sub);
    let r = RegGetValueW(key, &sub, name, RRF_RT_REG_QWORD, None, Some(&mut v as *mut _ as *mut _), Some(&mut sz));
    (r == ERROR_SUCCESS).then_some(v)
}

unsafe fn scan(key: HKEY) -> Vec<String> {
    let mut names = Vec::new();
    for sub in subkeys(key) {
        if sub == "NonPackaged" {
            let mut np = HKEY::default();
            if RegOpenKeyExW(key, w!("NonPackaged"), 0, KEY_READ, &mut np) == ERROR_SUCCESS {
                for exe in subkeys(np) {
                    if active(np, &exe) {
                        names.push(pretty(&exe));
                    }
                }
                let _ = RegCloseKey(np);
            }
        } else if active(key, &sub) {
            names.push(pretty(&sub));
        }
    }
    names.sort();
    names.dedup();
    names
}

unsafe fn active(key: HKEY, sub: &str) -> bool {
    let start = qword(key, sub, w!("LastUsedTimeStart")).unwrap_or(0);
    let stop = qword(key, sub, w!("LastUsedTimeStop"));
    start != 0 && stop == Some(0)
}

/// «C:#Program Files#Discord#Discord.exe» -> «Discord», «Microsoft.WindowsCamera_8wekyb3d8bbwe» -> «WindowsCamera».
fn pretty(id: &str) -> String {
    if let Some(exe) = id.rsplit('#').next().filter(|_| id.contains('#')) {
        let e = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(exe);
        return e.to_string();
    }
    let s = id.split('_').next().unwrap_or(id);
    s.rsplit('.').next().unwrap_or(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pretty_names() {
        assert_eq!(pretty("C:#Program Files#Discord_x#Discord.exe"), "Discord");
        assert_eq!(pretty("Microsoft.WindowsCamera_8wekyb3d8bbwe"), "WindowsCamera");
    }
}
