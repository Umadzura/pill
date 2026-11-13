//! Отрисовка GDI. Каждая отрисовка заодно записывает кликабельные зоны в State::hits.
//! Здесь кадр и вкладки, содержимое — по файлу на вкладку.

pub mod clip;
mod cmd;
mod collapsed;
mod ctl;
mod feed;
mod mascot;
mod music;
mod paint;
mod peek;
mod surface;
mod sys;
pub mod viz;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::GetClientRect;

use crate::app::{scale, st, Hit, State, Tab, TABS};
use crate::shell::{self, BOTTOM_R_COLLAPSED, BOTTOM_R_EXPANDED, EAR, EXPANDED_H, EXPANDED_W, MODE_COLLAPSED, MODE_PEEK};
use crate::pill::time_text;
use paint::*;

/// Ширина текста свёрнутой пилюли в пикселях.
pub unsafe fn measure(hwnd: HWND, dpi: u32, text: &str) -> i32 {
    let hdc = GetDC(hwnd);
    let f = font(dpi, 14, true, face());
    let old = SelectObject(hdc, f);
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide(text), &mut sz);
    SelectObject(hdc, old);
    let _ = DeleteObject(f);
    ReleaseDC(hwnd, hdc);
    sz.cx
}


/// Высота переключателя подвкладок (логические px): на столько сдвигается содержимое под ним.
const SUB_H: i32 = 30;

/// Переключатель «Управление | Монитор» под шапкой вкладки «Система».
unsafe fn subtabs(p: &mut Painter, s: &State) {
    let items = [(Tab::Controls, "Управление"), (Tab::System, "Монитор")];
    let (seg, pad) = (p.s(104), p.s(3));
    let frame = RECT { left: p.s(20), top: p.s(54), right: p.s(20) + seg * items.len() as i32 + 2 * pad, bottom: p.s(80) };
    p.fill(frame, R_PILL, PANEL);
    for (i, (t, name)) in items.iter().enumerate() {
        let left = frame.left + pad + i as i32 * seg;
        let r = RECT { left, top: frame.top + pad, right: left + seg, bottom: frame.bottom - pad };
        let h = Hit::Tab(*t);
        let on = *t == s.shell.tab;
        if on {
            p.fill(r, R_PILL, TRACK);
        }
        p.text(p.f.small, name, r, if on || p.hovered(h) { FG } else { DIM }, DT_CENTER);
        p.hit(r, h);
    }
}

/// WM_PAINT: окно слоёное, система его не рисует — только снимаем запрос и выводим кадр сами.
pub unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    BeginPaint(hwnd, &mut ps);
    let _ = EndPaint(hwnd, &ps);
    render(hwnd);
}

/// Нарисовать кадр в буфер и показать его (UpdateLayeredWindow).
pub unsafe fn render(hwnd: HWND) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);
    if w <= 0 || h <= 0 {
        return;
    }
    surface::with(w, h, |sf| {
        let mem = sf.dc;
        let cv = sf.canvas();
        cv.clear(BG);
        SetBkMode(mem, TRANSPARENT);
        let old_font = GetCurrentObject(mem, OBJ_FONT);

        let (radius, ear) = st(|s| {
            let fonts = Fonts::new(s.shell.dpi);
            let mut p = Painter { hdc: mem, cv, oy: 0, ox: 0, dpi: s.shell.dpi, f: &fonts, hits: Vec::new(), hover: s.hover, viz: None };
            let ear = p.s(EAR);
            // Пока окно меняет размер, показываем больший из двух видов, обрезанный его размером.
            let view = s.shell.anim_view().unwrap_or(shell::mode(s));
            let expanded = view != MODE_COLLAPSED;
            if view == MODE_COLLAPSED {
                p.ox = ear;
                collapsed::draw(&mut p, s, RECT { left: 0, top: 0, right: w - 2 * ear, bottom: h });
            } else if view == MODE_PEEK {
                // Карточка в полной раскладке по центру.
                p.ox = ear + (w - 2 * ear - p.s(crate::feat::peek::W)) / 2;
                peek::draw(&mut p, s);
            } else {
                // Раскладка всегда под полный размер; во время анимации она по центру и обрезана.
                let body = w - 2 * ear;
                let (w, h) = (p.s(EXPANDED_W), p.s(EXPANDED_H));
                p.ox = ear + (body - w) / 2;
                // Вкладки шириной по подписи. «Система» — группа из двух страниц.
                let mut left = p.s(16);
                for (t, name) in TABS.iter() {
                    let r = RECT { left, top: p.s(14), right: left + p.measure(fonts.bold, name) + p.s(24), bottom: p.s(44) };
                    left = r.right + p.s(4);
                    let group = *t == Tab::Controls;
                    let on = *t == s.shell.tab || (group && s.shell.tab == Tab::System);
                    let h = Hit::Tab(if group { s.shell.sys_page } else { *t });
                    if on {
                        p.fill(r, R_PILL, SELECTED_TAB);
                    } else if p.hovered(h) {
                        p.fill(r, R_PILL, PANEL);
                    }
                    p.text(fonts.bold, name, r, if on { FG } else { DIM }, DT_CENTER);
                    p.hit(r, h);
                }
                let r = RECT { left: 0, top: p.s(14), right: w - p.s(20), bottom: p.s(44) };
                p.text(fonts.bold, &time_text(), r, DIM, DT_RIGHT);

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
                        Tab::Controls | Tab::System => {
                            subtabs(&mut p, s);
                            // Страницы рисуют от тех же координат, что и раньше: сдвигаем их под переключатель.
                            p.oy = p.s(SUB_H);
                            if s.shell.tab == Tab::Controls {
                                let inner = h - p.oy;
                                ctl::draw(&mut p, s, w, inner);
                            } else {
                                sys::draw(&mut p, s, w);
                            }
                            p.oy = 0;
                        }
                        Tab::Feed => feed::draw(&mut p, s, w, h),
                    }
                }
            }
            s.hits = std::mem::take(&mut p.hits);
            s.viz_area = p.viz;
            SelectObject(mem, old_font);
            drop(fonts);
            (scale(if expanded { BOTTOM_R_EXPANDED } else { BOTTOM_R_COLLAPSED }, s.shell.dpi), ear)
        });

        let _ = GdiFlush();
        cv.finish(radius, ear);
        sf.present(hwnd, None);
    });
}
