//! Отрисовка GDI. Каждая отрисовка заодно записывает кликабельные зоны в State::hits.
//! Здесь кадр и вкладки, содержимое — по файлу на вкладку.

pub mod clip;
mod cmd;
mod collapsed;
mod ctl;
mod music;
mod paint;

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::app::{scale, st, Hit, Tab, TABS};
use crate::pill::time_text;
use paint::*;

/// Ширина текста свёрнутой пилюли в пикселях.
pub unsafe fn measure(hwnd: HWND, dpi: u32, text: &str) -> i32 {
    let hdc = GetDC(hwnd);
    let f = font(dpi, 14, true, w!("Segoe UI"));
    let old = SelectObject(hdc, f);
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide(text), &mut sz);
    SelectObject(hdc, old);
    let _ = DeleteObject(f);
    ReleaseDC(hwnd, hdc);
    sz.cx
}

/// Прямоугольник вкладки i в шапке развёрнутой панели.
fn tab_rect(i: usize, dpi: u32) -> RECT {
    let w = scale(110, dpi);
    let left = scale(20, dpi) + i as i32 * (w + scale(6, dpi));
    RECT { left, top: scale(14, dpi), right: left + w, bottom: scale(44, dpi) }
}

pub unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);

    // Двойная буферизация, чтобы не мерцало.
    let mem = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, w, h);
    let old_bmp = SelectObject(mem, bmp);
    let old_font = GetCurrentObject(mem, OBJ_FONT);

    let bg = CreateSolidBrush(BG);
    FillRect(mem, &rc, bg);
    let _ = DeleteObject(bg);
    SetBkMode(mem, TRANSPARENT);

    st(|s| {
        let fonts = Fonts::new(s.shell.dpi);
        let mut p = Painter { hdc: mem, dpi: s.shell.dpi, f: &fonts, hits: Vec::new(), hover: s.hover };
        if !s.shell.expanded {
            collapsed::draw(&mut p, s, rc);
        } else {
            for (i, (t, name)) in TABS.iter().enumerate() {
                let r = tab_rect(i, s.shell.dpi);
                let h = Hit::Tab(*t);
                if *t == s.shell.tab {
                    p.fill(r, 14, SELECTED_TAB);
                } else if p.hovered(h) {
                    p.fill(r, 14, PANEL);
                }
                p.text(fonts.bold, name, r, if *t == s.shell.tab { FG } else { DIM }, DT_CENTER);
                p.hit(r, h);
            }
            let r = RECT { left: 0, top: p.s(14), right: w - p.s(20), bottom: p.s(44) };
            p.text(fonts.bold, &time_text(), r, ACCENT, DT_RIGHT);

            if let Some(a) = &s.timers.alarm {
                let body = RECT { left: 0, top: p.s(50), right: w, bottom: h };
                p.text(fonts.title, &format!("⏰  {}", a), RECT { bottom: body.top + p.s(150), ..body }, ACCENT, DT_CENTER);
                let sub = RECT { top: body.top + p.s(150), bottom: body.top + p.s(190), ..body };
                p.text(fonts.normal, "Клик, чтобы закрыть", sub, DIM, DT_CENTER);
                p.hit(body, Hit::Alarm);
            } else {
                match s.shell.tab {
                    Tab::Music => music::draw(&mut p, s, w, h),
                    Tab::Clipboard => clip::draw(&mut p, s, w),
                    Tab::Commands => cmd::draw(&mut p, s, w),
                    Tab::Controls => ctl::draw(&mut p, s, w, h),
                }
            }
        }
        s.hits = std::mem::take(&mut p.hits);
        SelectObject(mem, old_font);
        drop(fonts);
    });

    let _ = BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_bmp);
    let _ = DeleteObject(bmp);
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}
