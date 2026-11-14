//! Отрисовка GDI. Каждая отрисовка заодно записывает кликабельные зоны в State::hits.
//! Здесь кадр и вкладки, содержимое — по файлу на вкладку.

mod collapsed;
mod ctl;
mod feed;
mod mascot;
mod music;
mod paint;
mod peek;
pub mod search;
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
    // Две страницы двумя словами: активная белая, вторая тихая.
    let items = [(Tab::Controls, "Управление"), (Tab::System, "Монитор")];
    let mut left = p.s(22);
    for (t, name) in items.iter() {
        let r = RECT { left, top: p.s(54), right: left + p.measure(p.f.normal, name), bottom: p.s(80) };
        left = r.right + p.s(18);
        let h = Hit::Tab(*t);
        let on = *t == s.shell.tab;
        p.text(p.f.normal, name, r, if on { FG } else if p.hovered(h) { DIM } else { FAINT }, DT_LEFT);
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
                let pw = s.peek.as_ref().map(|k| k.width()).unwrap_or(crate::feat::peek::W);
                p.ox = ear + (w - 2 * ear - p.s(pw)) / 2;
                peek::draw(&mut p, s);
            } else {
                // Раскладка всегда под полный размер; во время анимации она по центру и обрезана.
                let body = w - 2 * ear;
                let (w, h) = (p.s(EXPANDED_W), p.s(EXPANDED_H));
                p.ox = ear + (body - w) / 2;
                // Вкладки шириной по подписи. «Система» — группа из двух страниц.
                let mut left = p.s(16);
                let searching = s.shell.tab == Tab::Commands;
                if searching {
                    search::header(&mut p, s, w);
                }
                for (t, name) in TABS.iter().filter(|_| !searching) {
                    // На «Событиях» оранжевый счётчик: скольким агентам нужно разрешение.
                    let badge = if *t == Tab::Feed { s.agents.summary().0 } else { 0 };
                    let extra = if badge > 0 { p.s(22) } else { 0 };
                    let r = RECT { left, top: p.s(14), right: left + p.measure(fonts.bold, name) + p.s(24) + extra, bottom: p.s(44) };
                    left = r.right + p.s(4);
                    let group = *t == Tab::Controls;
                    let on = *t == s.shell.tab || (group && s.shell.tab == Tab::System);
                    let h = Hit::Tab(if group { s.shell.sys_page } else { *t });
                    // Без капсул: активная вкладка белая, остальные тихие.
                    let color = if on { FG } else if p.hovered(h) { DIM } else { FAINT };
                    p.text(fonts.bold, name, RECT { right: r.right - extra, ..r }, color, DT_CENTER);
                    if badge > 0 {
                        let (d, mid) = (p.s(18), (r.top + r.bottom) / 2);
                        let b = RECT { left: r.right - p.s(10) - d, top: mid - d / 2, right: r.right - p.s(10), bottom: mid - d / 2 + d };
                        p.fill(b, R_PILL, WARN);
                        p.text(fonts.small, &badge.to_string(), b, BG, DT_CENTER);
                    }
                    p.hit(r, h);
                }
                if !searching {
                    // Справа лупа (открывает поиск), левее время и батарея.
                    // Справа шестерёнка (настройки) и лупа (поиск), левее время и батарея.
                    let gear = RECT { left: w - p.s(48), top: p.s(14), right: w - p.s(18), bottom: p.s(44) };
                    p.icon_button(gear, I_GEAR, Hit::Settings, false);
                    let lens = RECT { left: gear.left - p.s(32), top: p.s(14), right: gear.left - p.s(2), bottom: p.s(44) };
                    p.icon_button(lens, I_SEARCH, Hit::Mode(0), false);
                    let r = RECT { left: 0, top: p.s(14), right: lens.left - p.s(8), bottom: p.s(44) };
                    let clock = match crate::feat::osd::battery_short(s.osd.battery) {
                        Some(b) => format!("{} · {}", time_text(), b),
                        None => time_text(),
                    };
                    p.text(fonts.normal, &clock, r, DIM, DT_RIGHT);
                }

                if let Some(a) = &s.timers.alarm {
                    let body = RECT { left: 0, top: p.s(50), right: w, bottom: h };
                    p.text(fonts.title, &format!("⏰  {}", a), RECT { bottom: body.top + p.s(150), ..body }, ACCENT, DT_CENTER);
                    let sub = RECT { top: body.top + p.s(150), bottom: body.top + p.s(190), ..body };
                    p.text(fonts.normal, "Клик, чтобы закрыть", sub, DIM, DT_CENTER);
                    p.hit(body, Hit::Alarm);
                } else {
                    match s.shell.tab {
                        Tab::Music => music::draw(&mut p, s, w, h),
                        Tab::Commands => search::draw(&mut p, s, w, h),
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
