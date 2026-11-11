//! Системные штуки: громкость, тема, VPN, питание, автозапуск, вставка, запуск, иконка трея.

use std::os::windows::process::CommandExt;
use std::process::Command;

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::NetworkManagement::Rras::*;
use windows::Win32::System::Com::*;
use windows::Win32::System::Power::SetSuspendState;
use windows::Win32::System::Registry::*;
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::*;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const THEME_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

fn hidden(cmd: &str, args: &[&str]) {
    let _ = Command::new(cmd).args(args).creation_flags(CREATE_NO_WINDOW).spawn();
}

// ---------- громкость ----------

unsafe fn endpoint() -> Option<IAudioEndpointVolume> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
    let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
    dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
}

/// (уровень 0..1, без звука)
pub fn volume() -> Option<(f32, bool)> {
    unsafe {
        let v = endpoint()?;
        let level = v.GetMasterVolumeLevelScalar().ok()?;
        let muted = v.GetMute().map(|m| m.as_bool()).unwrap_or(false);
        Some((level, muted))
    }
}

pub fn set_volume(level: f32) {
    unsafe {
        if let Some(v) = endpoint() {
            let _ = v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
            if level > 0.0 {
                let _ = v.SetMute(false, std::ptr::null());
            }
        }
    }
}

pub fn toggle_mute() {
    unsafe {
        if let Some(v) = endpoint() {
            let m = v.GetMute().map(|m| m.as_bool()).unwrap_or(false);
            let _ = v.SetMute(!m, std::ptr::null());
        }
    }
}

// ---------- реестр ----------

unsafe fn reg_dword(key: PCWSTR, name: PCWSTR) -> Option<u32> {
    let mut v = 0u32;
    let mut sz = 4u32;
    let r = RegGetValueW(HKEY_CURRENT_USER, key, name, RRF_RT_REG_DWORD, None, Some(&mut v as *mut _ as *mut _), Some(&mut sz));
    (r == ERROR_SUCCESS).then_some(v)
}

unsafe fn set_reg_dword(key: PCWSTR, name: PCWSTR, v: u32) {
    let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key, name, REG_DWORD.0, Some(&v as *const _ as *const _), 4);
}

pub fn dark_theme() -> Option<bool> {
    unsafe { reg_dword(THEME_KEY, w!("AppsUseLightTheme")).map(|v| v == 0) }
}

pub fn set_dark_theme(dark: bool) {
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

fn exe_path() -> Option<String> {
    std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned())
}

pub fn autostart() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Pill"), RRF_RT_REG_SZ, None, None, None) == ERROR_SUCCESS }
}

pub fn set_autostart(on: bool) {
    unsafe {
        if !on {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Pill"));
            return;
        }
        let Some(exe) = exe_path() else { return };
        let val: Vec<u16> = format!("\"{}\"", exe).encode_utf16().chain(Some(0)).collect();
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("Pill"), REG_SZ.0, Some(val.as_ptr() as *const _), (val.len() * 2) as u32);
    }
}

// ---------- VPN (встроенные VPN-подключения Windows) ----------

/// Имена VPN-профилей из телефонных книг Windows.
fn vpn_entries() -> Vec<String> {
    let mut names = Vec::new();
    for var in ["APPDATA", "ProgramData"] {
        let Ok(base) = std::env::var(var) else { continue };
        let pbk = format!(r"{}\Microsoft\Network\Connections\Pbk\rasphone.pbk", base);
        let Ok(data) = std::fs::read(&pbk) else { continue };
        for line in String::from_utf8_lossy(&data).lines() {
            let l = line.trim();
            if l.starts_with('[') && l.ends_with(']') && l.len() > 2 {
                names.push(l[1..l.len() - 1].to_string());
            }
        }
    }
    names
}

fn vpn_connected() -> Vec<String> {
    unsafe {
        let mut conns: [RASCONNW; 8] = std::mem::zeroed();
        for c in conns.iter_mut() {
            c.dwSize = std::mem::size_of::<RASCONNW>() as u32;
        }
        let mut cb = std::mem::size_of_val(&conns) as u32;
        let mut n = 0u32;
        if RasEnumConnectionsW(Some(conns.as_mut_ptr()), &mut cb, &mut n) != 0 {
            return Vec::new();
        }
        conns[..n as usize]
            .iter()
            .map(|c| {
                let len = c.szEntryName.iter().position(|&x| x == 0).unwrap_or(c.szEntryName.len());
                String::from_utf16_lossy(&c.szEntryName[..len])
            })
            .collect()
    }
}

/// (имя профиля, подключён). None, если VPN-профилей нет.
pub fn vpn() -> Option<(String, bool)> {
    let entries = vpn_entries();
    let connected = vpn_connected();
    if let Some(c) = connected.iter().find(|c| entries.contains(c)) {
        return Some((c.clone(), true));
    }
    entries.into_iter().next().map(|e| (e, false))
}

pub fn toggle_vpn() {
    match vpn() {
        Some((name, true)) => hidden("rasdial", &[&name, "/disconnect"]),
        Some((name, false)) => hidden("rasdial", &[&name]),
        None => {
            open("ms-settings:network-vpn", None);
        }
    }
}

// ---------- питание ----------

pub fn lock() {
    unsafe {
        let _ = LockWorkStation();
    }
}
pub fn sleep() {
    unsafe {
        let _ = SetSuspendState(false, false, false);
    }
}
pub fn restart() {
    hidden("shutdown", &["/r", "/t", "0"]);
}
pub fn shutdown() {
    hidden("shutdown", &["/s", "/t", "0"]);
}

// ---------- запуск и ввод ----------

pub fn open(file: &str, params: Option<&str>) -> bool {
    unsafe {
        let f = HSTRING::from(file);
        let p = params.map(HSTRING::from);
        let r = ShellExecuteW(
            None,
            w!("open"),
            &f,
            p.as_ref().map(|p| PCWSTR(p.as_ptr())).unwrap_or(PCWSTR::null()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
        r.0 as isize > 32
    }
}

/// Строка из командной строки: «> команда» в cmd, иначе как файл/URL/программа с аргументами.
pub fn run_line(line: &str) -> bool {
    let line = line.trim();
    if let Some(cmd) = line.strip_prefix('>') {
        return open("cmd.exe", Some(&format!("/k {}", cmd.trim())));
    }
    if open(line, None) {
        return true;
    }
    match line.split_once(' ') {
        Some((f, args)) => open(f, Some(args)),
        None => false,
    }
}

pub fn send_ctrl_v() {
    unsafe {
        let key = |vk: VIRTUAL_KEY, up: bool| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [key(VK_CONTROL, false), key(VK_V, false), key(VK_V, true), key(VK_CONTROL, true)];
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Делает окно активным, даже если Windows упирается (через AttachThreadInput).
pub unsafe fn force_foreground(hwnd: HWND) {
    if SetForegroundWindow(hwnd).as_bool() {
        return;
    }
    let fg = GetForegroundWindow();
    let fg_thread = GetWindowThreadProcessId(fg, None);
    let me = GetCurrentThreadId();
    let attached = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
    let _ = SetForegroundWindow(hwnd);
    let _ = BringWindowToTop(hwnd);
    if attached {
        let _ = AttachThreadInput(me, fg_thread, false);
    }
}

// ---------- иконка трея ----------

/// Рисует маленькую пилюлю со сглаживанием прямо в пикселях (ресурсов в exe нет).
pub unsafe fn make_icon() -> HICON {
    let size = GetSystemMetrics(SM_CXSMICON).max(16);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let Ok(color) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
        return LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
    };
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
    let s = size as f32;
    let r = s * 0.25;
    let (cy, x0, x1) = (s / 2.0, s * 0.12 + r, s * 0.88 - r);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = if fx < x0 { x0 - fx } else if fx > x1 { fx - x1 } else { 0.0 };
            let d = (dx * dx + (fy - cy) * (fy - cy)).sqrt() - r;
            let a = (0.5 - d).clamp(0.0, 1.0);
            let a8 = (a * 255.0) as u32;
            // Премультиплицированный белый.
            px[(y * size + x) as usize] = a8 << 24 | a8 << 16 | a8 << 8 | a8;
        }
    }
    let mask = CreateBitmap(size, size, 1, 1, None);
    let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let icon = CreateIconIndirect(&info).unwrap_or_default();
    let _ = DeleteObject(mask);
    let _ = DeleteObject(color);
    icon
}
