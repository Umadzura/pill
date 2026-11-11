#![windows_subsystem = "windows"]

//! Pill: Р»С‘РіРєР°СЏ РїР°РЅРµР»СЊ СЃРІРµСЂС…Сѓ СЌРєСЂР°РЅР°. Р§РёСЃС‚С‹Р№ Win32 + GDI, Р±РµР· Р±СЂР°СѓР·РµСЂРЅРѕРіРѕ РґРІРёР¶РєР°.
//! РџРµСЂРµСЂРёСЃРѕРІРєР° С‚РѕР»СЊРєРѕ РїРѕ СЃРѕР±С‹С‚РёСЏРј (С‚Р°Р№РјРµСЂ СЂР°Р· РІ СЃРµРєСѓРЅРґСѓ, РЅР°РІРµРґРµРЅРёРµ), РІ РїСЂРѕСЃС‚РѕРµ CPU ~0.

use std::cell::RefCell;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// Р Р°Р·РјРµСЂС‹ РІ Р»РѕРіРёС‡РµСЃРєРёС… РїРёРєСЃРµР»СЏС… (РїСЂРё 100% РјР°СЃС€С‚Р°Р±Рµ).
const COLLAPSED_W: i32 = 190;
const COLLAPSED_H: i32 = 32;
const EXPANDED_W: i32 = 560;
const EXPANDED_H: i32 = 320;
const TOP_MARGIN: i32 = 6;
const WM_MOUSELEAVE_MSG: u32 = 0x02A3;

const TIMER_CLOCK: usize = 1;
const TIMER_COLLAPSE: usize = 2;
const COLLAPSE_DELAY_MS: u32 = 350;

const BG: COLORREF = rgb(18, 18, 20);
const FG: COLORREF = rgb(235, 235, 240);
const DIM: COLORREF = rgb(140, 140, 150);
const ACCENT: COLORREF = rgb(120, 150, 255);

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Music,
    Clipboard,
    Commands,
    Controls,
}

const TABS: [(Tab, &str); 4] = [
    (Tab::Music, "РњСѓР·С‹РєР°"),
    (Tab::Clipboard, "Р‘СѓС„РµСЂ"),
    (Tab::Commands, "РљРѕРјР°РЅРґС‹"),
    (Tab::Controls, "РЈРїСЂР°РІР»РµРЅРёРµ"),
];

struct State {
    expanded: bool,
    tracking: bool,
    tab: Tab,
    dpi: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State {
        expanded: false,
        tracking: false,
        tab: Tab::Music,
        dpi: 96,
    });
}

fn scale(v: i32, dpi: u32) -> i32 {
    v * dpi as i32 / 96
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

fn main() {
    unsafe {
        // РћРґРёРЅ СЌРєР·РµРјРїР»СЏСЂ.
        let _mutex = CreateMutexW(None, true, w!("Local\\PillSingleInstance"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return;
        }

        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        let hinstance = GetModuleHandleW(None).unwrap();
        let class = w!("PillWindow");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        };
        RegisterClassW(&wc);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Pill"),
            WS_POPUP,
            0,
            0,
            COLLAPSED_W,
            COLLAPSED_H,
            None,
            None,
            hinstance,
            None,
        )
        .unwrap();

        let dpi = GetDpiForWindow(hwnd).max(96);
        STATE.with(|s| s.borrow_mut().dpi = dpi);
        layout(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        SetTimer(hwnd, TIMER_CLOCK, 1000, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// РЎС‚Р°РІРёС‚ РѕРєРЅРѕ РїРѕ С†РµРЅС‚СЂСѓ РІРµСЂС…РЅРµРіРѕ РєСЂР°СЏ РѕСЃРЅРѕРІРЅРѕРіРѕ РјРѕРЅРёС‚РѕСЂР° Рё Р·Р°РґР°С‘С‚ СЃРєСЂСѓРіР»С‘РЅРЅСѓСЋ С„РѕСЂРјСѓ.
unsafe fn layout(hwnd: HWND) {
    let (expanded, dpi) = STATE.with(|s| {
        let s = s.borrow();
        (s.expanded, s.dpi)
    });
    let (w, h) = if expanded {
        (EXPANDED_W, EXPANDED_H)
    } else {
        (COLLAPSED_W, COLLAPSED_H)
    };
    let (w, h) = (scale(w, dpi), scale(h, dpi));
    let screen_w = GetSystemMetrics(SM_CXSCREEN);
    let x = (screen_w - w) / 2;
    let y = scale(TOP_MARGIN, dpi);
    let _ = SetWindowPos(
        hwnd,
        HWND_TOPMOST,
        x,
        y,
        w,
        h,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
    let radius = if expanded { scale(28, dpi) } else { h };
    let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius);
    SetWindowRgn(hwnd, rgn, true);
    let _ = InvalidateRect(hwnd, None, false);
}

fn time_text() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:02}:{:02}", t.wHour, t.wMinute)
}

/// None, РµСЃР»Рё Р±Р°С‚Р°СЂРµРё РЅРµС‚ (СЃС‚Р°С†РёРѕРЅР°СЂРЅС‹Р№ РџРљ).
fn battery_text() -> Option<String> {
    let mut ps = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut ps).ok()? };
    // 128 = РЅРµС‚ Р±Р°С‚Р°СЂРµРё, 255 = РЅРµРёР·РІРµСЃС‚РЅРѕ.
    if ps.BatteryFlag & 128 != 0 || ps.BatteryLifePercent > 100 {
        return None;
    }
    let charging = if ps.ACLineStatus == 1 { "вљЎ" } else { "" };
    Some(format!("{}{}%", charging, ps.BatteryLifePercent))
}

unsafe fn make_font(dpi: u32, size: i32, bold: bool) -> HFONT {
    CreateFontW(
        -scale(size, dpi),
        0,
        0,
        0,
        if bold { FW_SEMIBOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        0,
        w!("Segoe UI"),
    )
}

unsafe fn draw_text(hdc: HDC, text: &str, rect: &mut RECT, color: COLORREF, flags: DRAW_TEXT_FORMAT) {
    SetTextColor(hdc, color);
    let mut buf = wide(text);
    DrawTextW(hdc, &mut buf, rect, flags | DT_SINGLELINE | DT_NOPREFIX);
}

/// РџСЂСЏРјРѕСѓРіРѕР»СЊРЅРёРє РІРєР»Р°РґРєРё i РІ С€Р°РїРєРµ СЂР°Р·РІС‘СЂРЅСѓС‚РѕР№ РїР°РЅРµР»Рё.
fn tab_rect(i: usize, dpi: u32) -> RECT {
    let w = scale(110, dpi);
    let left = scale(20, dpi) + i as i32 * (w + scale(6, dpi));
    RECT {
        left,
        top: scale(14, dpi),
        right: left + w,
        bottom: scale(44, dpi),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);

    // Р”РІРѕР№РЅР°СЏ Р±СѓС„РµСЂРёР·Р°С†РёСЏ, С‡С‚РѕР±С‹ РЅРµ РјРµСЂС†Р°Р»Рѕ.
    let mem = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, w, h);
    let old_bmp = SelectObject(mem, bmp);

    let bg = CreateSolidBrush(BG);
    FillRect(mem, &rc, bg);
    let _ = DeleteObject(bg);
    SetBkMode(mem, TRANSPARENT);

    let (expanded, tab, dpi) = STATE.with(|s| {
        let s = s.borrow();
        (s.expanded, s.tab, s.dpi)
    });

    if !expanded {
        let font = make_font(dpi, 14, true);
        let old_font = SelectObject(mem, font);
        let text = match battery_text() {
            Some(b) => format!("{}   {}", time_text(), b),
            None => time_text(),
        };
        let mut r = rc;
        draw_text(mem, &text, &mut r, FG, DT_CENTER | DT_VCENTER);
        SelectObject(mem, old_font);
        let _ = DeleteObject(font);
    } else {
        let font = make_font(dpi, 14, true);
        let small = make_font(dpi, 13, false);
        let old_font = SelectObject(mem, font);

        // РЁР°РїРєР°: РІРєР»Р°РґРєРё + РІСЂРµРјСЏ СЃРїСЂР°РІР°.
        for (i, (t, name)) in TABS.iter().enumerate() {
            let mut r = tab_rect(i, dpi);
            if *t == tab {
                let br = CreateSolidBrush(rgb(40, 40, 48));
                let pill = CreateRoundRectRgn(r.left, r.top, r.right, r.bottom, scale(14, dpi), scale(14, dpi));
                let _ = FillRgn(mem, pill, br);
                let _ = DeleteObject(pill);
                let _ = DeleteObject(br);
            }
            let color = if *t == tab { FG } else { DIM };
            draw_text(mem, name, &mut r, color, DT_CENTER | DT_VCENTER);
        }
        let mut r = RECT { left: 0, top: scale(14, dpi), right: w - scale(20, dpi), bottom: scale(44, dpi) };
        draw_text(mem, &time_text(), &mut r, ACCENT, DT_RIGHT | DT_VCENTER);

        // РўРµР»Рѕ РІРєР»Р°РґРєРё (РїРѕРєР° Р·Р°РіР»СѓС€РєРё).
        SelectObject(mem, small);
        let body = match tab {
            Tab::Music => "Р—РґРµСЃСЊ Р±СѓРґРµС‚ С‚РµРєСѓС‰РёР№ С‚СЂРµРє Рё СѓРїСЂР°РІР»РµРЅРёРµ РїР»РµРµСЂРѕРј",
            Tab::Clipboard => "Р—РґРµСЃСЊ Р±СѓРґРµС‚ РёСЃС‚РѕСЂРёСЏ Р±СѓС„РµСЂР° РѕР±РјРµРЅР°",
            Tab::Commands => "Р—РґРµСЃСЊ Р±СѓРґРµС‚ РєРѕРјР°РЅРґРЅР°СЏ СЃС‚СЂРѕРєР°",
            Tab::Controls => "Р—РґРµСЃСЊ Р±СѓРґСѓС‚ Wi-Fi, Bluetooth, РіСЂРѕРјРєРѕСЃС‚СЊ, РїРёС‚Р°РЅРёРµ",
        };
        let mut r = RECT { left: scale(20, dpi), top: scale(60, dpi), right: w - scale(20, dpi), bottom: h - scale(20, dpi) };
        draw_text(mem, body, &mut r, DIM, DT_CENTER | DT_VCENTER);

        SelectObject(mem, old_font);
        let _ = DeleteObject(font);
        let _ = DeleteObject(small);
    }

    let _ = BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_bmp);
    let _ = DeleteObject(bmp);
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn set_expanded(hwnd: HWND, value: bool) {
    let changed = STATE.with(|s| {
        let mut s = s.borrow_mut();
        let changed = s.expanded != value;
        s.expanded = value;
        changed
    });
    if changed {
        layout(hwnd);
    }
}

fn loword(l: LPARAM) -> i32 {
    (l.0 & 0xFFFF) as i16 as i32
}
fn hiword(l: LPARAM) -> i32 {
    ((l.0 >> 16) & 0xFFFF) as i16 as i32
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        // РљР»РёРє РїРѕ РѕРєРЅСѓ РЅРµ Р·Р°Р±РёСЂР°РµС‚ С„РѕРєСѓСЃ Сѓ С‚РѕРіРѕ, РіРґРµ С‚С‹ РїРµС‡Р°С‚Р°РµС€СЊ.
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            let _ = KillTimer(hwnd, TIMER_COLLAPSE);
            let need_track = STATE.with(|s| !s.borrow().tracking);
            if need_track {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut tme);
                STATE.with(|s| s.borrow_mut().tracking = true);
            }
            set_expanded(hwnd, true);
            LRESULT(0)
        }
        WM_MOUSELEAVE_MSG => {
            STATE.with(|s| s.borrow_mut().tracking = false);
            SetTimer(hwnd, TIMER_COLLAPSE, COLLAPSE_DELAY_MS, None);
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = (loword(lparam), hiword(lparam));
            let dpi = STATE.with(|s| s.borrow().dpi);
            for (i, (t, _)) in TABS.iter().enumerate() {
                let r = tab_rect(i, dpi);
                if x >= r.left && x < r.right && y >= r.top && y < r.bottom {
                    STATE.with(|s| s.borrow_mut().tab = *t);
                    let _ = InvalidateRect(hwnd, None, false);
                }
            }
            LRESULT(0)
        }
        WM_TIMER => {
            match wparam.0 {
                TIMER_CLOCK => {
                    let _ = InvalidateRect(hwnd, None, false);
                }
                TIMER_COLLAPSE => {
                    let _ = KillTimer(hwnd, TIMER_COLLAPSE);
                    set_expanded(hwnd, false);
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            STATE.with(|s| s.borrow_mut().dpi = (wparam.0 & 0xFFFF) as u32);
            layout(hwnd);
            LRESULT(0)
        }
        // РЎРјРµРЅР° СЂР°Р·СЂРµС€РµРЅРёСЏ / РїРѕРґРєР»СЋС‡РµРЅРёРµ РјРѕРЅРёС‚РѕСЂР°.
        WM_DISPLAYCHANGE => {
            layout(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[allow(dead_code)]
fn _unused(_: PCWSTR) {}
