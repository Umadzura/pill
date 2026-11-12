//! Поток-наблюдатель: один `WaitForMultipleObjects` на все ядерные события сразу.
//!
//! * Кто держит микрофон и камеру. Windows отмечает это в реестре
//!   (CapabilityAccessManager\ConsentStore): у занятого устройства LastUsedTimeStop = 0.
//!   Ждём `RegNotifyChangeKeyValue`, просыпаемся только при изменениях.
//! * Визуализатор музыки (`feat::viz`). Пока UI просит захват, ждём с тайм-аутом ~33 мс
//!   и на каждом тайм-ауте забираем звук и считаем кадр. Иначе ждём бесконечно, как раньше.

use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Registry::*;
use windows::Win32::System::Threading::{CreateEventW, WaitForMultipleObjects, INFINITE};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::feat::viz::{self, Analyzer, PENDING, SPECTRUM};
use crate::ids::{WM_APP_PRIVACY, WM_APP_VIZ};
use crate::sys::loopback::Loopback;

const BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\";
/// ~30 кадров в секунду: глазу хватает, а будим UI вдвое реже, чем 60 fps.
const FRAME_MS: u32 = 33;
/// Устройство вывода пропало (сменились наушники): переоткрываем не чаще раза в секунду.
const REOPEN: Duration = Duration::from_secs(1);

/// Имена приложений, которые сейчас используют микрофон и камеру.
#[derive(Clone, Default, PartialEq)]
pub struct InUse {
    pub mic: Vec<String>,
    pub cam: Vec<String>,
}

pub static IN_USE: Mutex<InUse> = Mutex::new(InUse { mic: Vec::new(), cam: Vec::new() });

pub fn start(hwnd: HWND) {
    let raw = hwnd.0 as isize;
    std::thread::spawn(move || unsafe {
        // Захвату звука нужен COM.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        watch(raw)
    });
}

fn post(hwnd: isize, msg: u32) {
    unsafe {
        let _ = PostMessageW(HWND(hwnd as *mut _), msg, WPARAM(0), LPARAM(0));
    }
}

/// Захват звука для визуализатора: открыт только пока viz::want().
struct Viz {
    cap: Option<(Loopback, Analyzer)>,
    retry_at: Option<Instant>,
}

impl Viz {
    /// Привести захват в соответствие с want(). true — захват идёт (ждать с тайм-аутом).
    fn sync(&mut self, hwnd: isize) -> bool {
        if !viz::want() {
            if self.cap.take().is_some() {
                viz::clear();
                post_frame(hwnd);
            }
            self.retry_at = None;
            return false;
        }
        if self.cap.is_none() && self.retry_at.map(|t| Instant::now() >= t).unwrap_or(true) {
            match Loopback::open() {
                Some(lb) => {
                    let an = Analyzer::new(lb.rate);
                    self.cap = Some((lb, an));
                    self.retry_at = None;
                }
                None => self.retry_at = Some(Instant::now() + REOPEN),
            }
        }
        true
    }

    /// Тайм-аут кадра: забрать звук, посчитать, разбудить UI, если что-то видно.
    fn frame(&mut self, hwnd: isize) {
        let Some((lb, an)) = self.cap.as_mut() else { return };
        let got = match lb.read(|v| an.push(v)) {
            Ok(n) => n,
            Err(()) => {
                // Устройство пропало: закрыть, через секунду sync() откроет новое.
                self.cap = None;
                self.retry_at = Some(Instant::now() + REOPEN);
                viz::clear();
                post_frame(hwnd);
                return;
            }
        };
        let levels = if got > 0 {
            an.frame()
        } else if an.fade() {
            an.levels()
        } else {
            // Тишина, полоски уже лежат: UI не будим вовсе.
            let mut cur = SPECTRUM.lock().unwrap();
            if cur.iter().all(|&v| v == 0.0) {
                return;
            }
            *cur = [0.0; viz::BANDS];
            drop(cur);
            post_frame(hwnd);
            return;
        };
        *SPECTRUM.lock().unwrap() = levels;
        post_frame(hwnd);
    }
}

/// Разбудить UI, если прошлый кадр он уже нарисовал.
fn post_frame(hwnd: isize) {
    if !PENDING.swap(true, Ordering::AcqRel) {
        post(hwnd, WM_APP_VIZ);
    }
}

unsafe fn watch(hwnd: isize) {
    // События: [микрофон, камера] (если ключи открылись) + пробуждение визуализатора последним.
    let mut keys: Vec<HKEY> = Vec::new();
    let mut events: Vec<HANDLE> = Vec::new();
    for cap in ["microphone", "webcam"] {
        let path = HSTRING::from(format!("{}{}", BASE, cap));
        let mut key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, &path, 0, KEY_READ | KEY_NOTIFY, &mut key) != ERROR_SUCCESS {
            continue;
        }
        let Ok(ev) = CreateEventW(None, false, false, None) else { continue };
        arm(key, ev);
        keys.push(key);
        events.push(ev);
    }
    let Ok(wake) = CreateEventW(None, false, false, None) else { return };
    viz::set_wake(wake);
    events.push(wake);
    let n_reg = keys.len();

    let mut viz = Viz { cap: None, retry_at: None };
    let mut rescan = true;
    loop {
        if rescan && n_reg > 0 {
            scan_privacy(hwnd, &keys);
        }
        let capturing = viz.sync(hwnd);
        let timeout = if capturing { FRAME_MS } else { INFINITE };
        let r = WaitForMultipleObjects(&events, false, timeout);
        rescan = false;
        if r == WAIT_TIMEOUT {
            viz.frame(hwnd);
            continue;
        }
        let i = r.0 as usize;
        if i < n_reg {
            // Уведомление одноразовое: взводим заново до пересчёта, чтобы не пропустить изменение.
            arm(keys[i], events[i]);
            rescan = true;
        } else if i >= events.len() {
            return;
        }
        // i == n_reg — разбудили: sync() в начале цикла включит или выключит захват.
    }
}

unsafe fn scan_privacy(hwnd: isize, keys: &[HKEY]) {
    let now = InUse { mic: keys.first().map(|&k| scan(k)).unwrap_or_default(), cam: keys.get(1).map(|&k| scan(k)).unwrap_or_default() };
    let changed = {
        let mut cur = IN_USE.lock().unwrap();
        let c = *cur != now;
        *cur = now;
        c
    };
    if changed {
        post(hwnd, WM_APP_PRIVACY);
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
