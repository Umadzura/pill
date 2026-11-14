//! Отрисовка GDI. Каждая отрисовка заодно записывает кликабельные зоны в State::hits.
//! Здесь кадр и вкладки, содержимое — по файлу на вкладку.

mod collapsed;
mod ctl;
mod feed;
mod mascot;
mod music;
mod paint;
mod peek;
pub mod settings;
pub mod search;
mod surface;
mod sys;
pub mod viz;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use std::cell::RefCell;

use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, PostMessageW};

use crate::ids::WM_APP_LAYOUT;

use crate::app::{st, Hit, State, Tab, TABS};
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


/// Отступ под последним элементом вкладки и самая низкая панель (логические px).
const PAD_BOTTOM: i32 = 20;
const MIN_H: i32 = 200;

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

thread_local! {
    /// Шрифты живут между кадрами: создавать шесть шрифтов на каждый кадр анимации заметно.
    static FONTS: RefCell<Option<(u32, Fonts)>> = const { RefCell::new(None) };
}

/// Нарисовать кадр текущего размера окна и показать его (UpdateLayeredWindow).
pub unsafe fn render(hwnd: HWND) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    render_sized(hwnd, rc.right, rc.bottom, None, None);
}

/// Нарисовать кадр w×h и показать его; `pos` — заодно поставить окно туда (размер и место
/// меняются одним вызовом с картинкой), `cap` — буфер с запасом на время анимации.
pub unsafe fn render_sized(hwnd: HWND, w: i32, h: i32, pos: Option<POINT>, cap: Option<(i32, i32)>) {
    if w <= 0 || h <= 0 {
        return;
    }
    let dpi = st(|s| s.shell.dpi);
    FONTS.with(|fc| {
        let mut fc = fc.borrow_mut();
        if fc.as_ref().map(|f| f.0) != Some(dpi) {
            *fc = None;
            *fc = Some((dpi, Fonts::new(dpi)));
        }
        let Some((_, fonts)) = fc.as_ref() else { return };
        render_with(hwnd, w, h, pos, cap, fonts);
    });
}

unsafe fn render_with(hwnd: HWND, w: i32, h: i32, pos: Option<POINT>, cap: Option<(i32, i32)>, fonts: &Fonts) {
    surface::with(w, h, cap, |sf| {
        let mem = sf.dc;
        let cv = sf.canvas();
        cv.clear(BG);
        SetBkMode(mem, TRANSPARENT);
        let old_font = GetCurrentObject(mem, OBJ_FONT);

        let (radius, ear, relayout, alpha) = st(|s| {
            let mut p = Painter { hdc: mem, cv, oy: 0, ox: 0, dpi: s.shell.dpi, f: fonts, hits: Vec::new(), hover: s.hover, bottom: 0, viz: None };
            let ear = p.s(EAR);
            // Пока окно меняет размер, сначала гаснет старое содержимое, потом проявляется новое
            // (в полной раскладке по центру, обрезанное размером окна).
            let anim = s.shell.anim_phase();
            let (view, alpha) = anim.unwrap_or((shell::mode(s), 1.0));
            if view == MODE_COLLAPSED {
                p.ox = ear;
                let bottom = h.min(p.s(shell::COLLAPSED_H));
                collapsed::draw(&mut p, s, RECT { left: 0, top: 0, right: w - 2 * ear, bottom });
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
                    p.reach(220);
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
            // Высота панели по содержимому вкладки: поменялась — окно переложится плавно (WM_APP_LAYOUT).
            let mut relayout = false;
            if view == shell::MODE_EXPANDED && p.bottom > 0 {
                let want = (p.bottom * 96 / s.shell.dpi as i32 + PAD_BOTTOM).clamp(MIN_H, EXPANDED_H);
                relayout = std::mem::replace(&mut s.shell.panel_h, want) != want;
            }
            s.hits = std::mem::take(&mut p.hits);
            // Во время анимации визуализатор не рисует поверх: кадр пересобирается целиком.
            s.viz_area = if anim.is_some() { None } else { p.viz };
            SelectObject(mem, old_font);
            // Радиус нижних углов растёт вместе с высотой: от пилюли к панели без скачка.
            let hl = h * 96 / s.shell.dpi as i32;
            let k = ((hl - shell::COLLAPSED_H) as f32 / 150.0).clamp(0.0, 1.0);
            let rad = BOTTOM_R_COLLAPSED as f32 + (BOTTOM_R_EXPANDED - BOTTOM_R_COLLAPSED) as f32 * k;
            ((rad * s.shell.dpi as f32 / 96.0).round() as i32, ear, relayout, alpha)
        });
        if relayout {
            let _ = PostMessageW(hwnd, WM_APP_LAYOUT, WPARAM(0), LPARAM(0));
        }

        let _ = GdiFlush();
        if alpha < 1.0 {
            cv.fade(alpha);
        }
        cv.finish(radius, ear);
        sf.present(hwnd, None, pos);
    });
}
