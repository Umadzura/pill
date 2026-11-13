//! Поверхность окна: 32-битный буфер, в который рисует вся отрисовка, и вывод его на экран через
//! `UpdateLayeredWindow` (окно с попиксельной прозрачностью). Так края чёлки и все скругления
//! внутри сглажены: `SetWindowRgn` и `CreateRoundRectRgn` давали лесенку.
//!
//! Буфер живёт между кадрами (нужен визуализатору: он перерисовывает только свои полоски) и
//! пересоздаётся при смене размера окна. Раскрытая панель на 200% масштаба — около 3,8 МБ.

use std::cell::RefCell;

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{UpdateLayeredWindowIndirect, ULW_ALPHA, UPDATELAYEREDWINDOWINFO};

/// Пиксели буфера: строки сверху вниз, 0xAARRGGBB. Рисует сглаженные фигуры напрямую, мимо GDI.
#[derive(Clone, Copy)]
pub struct Canvas {
    bits: *mut u32,
    w: i32,
    h: i32,
}

fn px(c: COLORREF) -> u32 {
    let (r, g, b) = (c.0 & 0xFF, (c.0 >> 8) & 0xFF, (c.0 >> 16) & 0xFF);
    0xFF00_0000 | r << 16 | g << 8 | b
}

/// Смешать цвет `c` с пикселем `d` в доле `k` (0..1).
fn mix(d: u32, c: u32, k: f32) -> u32 {
    let ch = |sh: u32| {
        let (a, b) = (((d >> sh) & 0xFF) as f32, ((c >> sh) & 0xFF) as f32);
        ((a + (b - a) * k + 0.5) as u32).min(255) << sh
    };
    0xFF00_0000 | ch(16) | ch(8) | ch(0)
}

impl Canvas {
    unsafe fn at(&self, x: i32, y: i32) -> Option<*mut u32> {
        (x >= 0 && y >= 0 && x < self.w && y < self.h).then(|| self.bits.add((y * self.w + x) as usize))
    }

    /// Залить всё одним цветом.
    pub unsafe fn clear(&self, c: COLORREF) {
        std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize).fill(px(c));
    }

    /// Доля закрашивания пикселя в углу скругления: i-й столбец от края, dy — глубина строки в зоне угла.
    fn corner(rad: f32, i: i32, dy: f32) -> f32 {
        let dx = rad - (i as f32 + 0.5);
        if dx <= 0.0 {
            return 1.0;
        }
        (rad - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0)
    }

    /// Насколько строка y заходит в верхнюю или нижнюю зону скругления прямоугольника (0 — не заходит).
    fn depth(r: &RECT, rad: f32, y: i32) -> f32 {
        let fy = y as f32 + 0.5;
        if fy < r.top as f32 + rad {
            r.top as f32 + rad - fy
        } else if fy > r.bottom as f32 - rad {
            fy - (r.bottom as f32 - rad)
        } else {
            0.0
        }
    }

    fn radius(r: &RECT, diam: i32) -> f32 {
        (diam as f32 / 2.0).min((r.right - r.left) as f32 / 2.0).min((r.bottom - r.top) as f32 / 2.0).max(0.0)
    }

    /// Сглаженный скруглённый прямоугольник. `diam` — диаметр скругления в пикселях (0 — прямые углы).
    pub unsafe fn fill_round(&self, r: RECT, diam: i32, c: COLORREF) {
        if r.right <= r.left || r.bottom <= r.top {
            return;
        }
        let rad = Self::radius(&r, diam);
        let ri = rad.ceil() as i32;
        let c = px(c);
        for y in r.top.max(0)..r.bottom.min(self.h) {
            let dy = Self::depth(&r, rad, y);
            let (mut xs, mut xe) = (r.left, r.right);
            if dy > 0.0 {
                for i in 0..ri {
                    let (xl, xr) = (r.left + i, r.right - 1 - i);
                    if xl > xr {
                        break;
                    }
                    let k = Self::corner(rad, i, dy);
                    if k <= 0.0 {
                        continue;
                    }
                    for x in if xl == xr { [xl, i32::MIN] } else { [xl, xr] } {
                        if let Some(p) = self.at(x, y) {
                            *p = if k >= 1.0 { c } else { mix(*p, c, k) };
                        }
                    }
                }
                xs = r.left + ri;
                xe = r.right - ri;
            }
            let (xs, xe) = (xs.max(0), xe.min(self.w));
            if xe > xs {
                std::slice::from_raw_parts_mut(self.bits.add((y * self.w + xs) as usize), (xe - xs) as usize).fill(c);
            }
        }
    }

    /// Скруглить уже нарисованную картинку (обложку): закрасить цветом фона то, что снаружи углов.
    pub unsafe fn round_off(&self, r: RECT, diam: i32, bg: COLORREF) {
        let rad = Self::radius(&r, diam);
        let ri = rad.ceil() as i32;
        let bg = px(bg);
        for y in r.top.max(0)..r.bottom.min(self.h) {
            let dy = Self::depth(&r, rad, y);
            if dy <= 0.0 {
                continue;
            }
            for i in 0..ri {
                let k = 1.0 - Self::corner(rad, i, dy);
                if k <= 0.0 {
                    continue;
                }
                for x in [r.left + i, r.right - 1 - i] {
                    if let Some(p) = self.at(x, y) {
                        *p = mix(*p, bg, k);
                    }
                }
            }
        }
    }

    /// Последний шаг кадра: GDI обнуляет альфу там, где рисовал текст, — делаем всё непрозрачным
    /// и вырезаем форму чёлки. Верх прижат к краю экрана и расходится к нему «ушками» шириной
    /// `ear` (вогнутые углы, как у выреза в экране), нижние углы тела скруглены радиусом `rad_px`.
    pub unsafe fn finish(&self, rad_px: i32, ear: i32) {
        for p in std::slice::from_raw_parts_mut(self.bits, (self.w * self.h) as usize) {
            *p |= 0xFF00_0000;
        }
        // Оставить пикселю долю k непрозрачности (премультиплицированная альфа: так её ждёт UpdateLayeredWindow).
        let dim = |x: i32, y: i32, k: f32| {
            if k >= 1.0 {
                return;
            }
            if let Some(p) = self.at(x, y) {
                let a = (k.max(0.0) * 255.0 + 0.5) as u32;
                let ch = |sh: u32| (((*p >> sh) & 0xFF) * a / 255) << sh;
                *p = a << 24 | ch(16) | ch(8) | ch(0);
            }
        };
        // Ушки: чёрное только снаружи четверти круга с центром в нижнем внешнем углу ушка.
        let e = ear as f32;
        for y in 0..self.h {
            for i in 0..ear.min(self.w / 2) {
                let k = if y < ear {
                    let (dx, dy) = (i as f32 + 0.5, e - (y as f32 + 0.5));
                    ((dx * dx + dy * dy).sqrt() - e + 0.5).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                dim(i, y, k);
                dim(self.w - 1 - i, y, k);
            }
        }
        // Нижние углы тела.
        let body = self.w - 2 * ear;
        let rad = (rad_px.min(body / 2).min(self.h)).max(0) as f32;
        let ri = rad.ceil() as i32;
        for y in (self.h - ri).max(0)..self.h {
            let dy = y as f32 + 0.5 - (self.h as f32 - rad);
            if dy <= 0.0 {
                continue;
            }
            for i in 0..ri {
                let k = Self::corner(rad, i, dy);
                dim(ear + i, y, k);
                dim(self.w - 1 - ear - i, y, k);
            }
        }
    }
}

pub struct Surface {
    pub dc: HDC,
    bmp: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u32,
    w: i32,
    h: i32,
}

impl Surface {
    unsafe fn new(w: i32, h: i32) -> Option<Surface> {
        let dc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let Ok(bmp) = CreateDIBSection(dc, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
            let _ = DeleteDC(dc);
            return None;
        };
        if bits.is_null() {
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(dc);
            return None;
        }
        let old = SelectObject(dc, bmp);
        Some(Surface { dc, bmp, old, bits: bits as *mut u32, w, h })
    }

    pub fn canvas(&self) -> Canvas {
        Canvas { bits: self.bits, w: self.w, h: self.h }
    }

    /// Показать буфер на экране. `dirty` — поменялся только этот прямоугольник.
    pub unsafe fn present(&self, hwnd: HWND, dirty: Option<RECT>) {
        let size = SIZE { cx: self.w, cy: self.h };
        let src = POINT::default();
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        let info = UPDATELAYEREDWINDOWINFO {
            cbSize: std::mem::size_of::<UPDATELAYEREDWINDOWINFO>() as u32,
            psize: &size,
            hdcSrc: self.dc,
            pptSrc: &src,
            pblend: &blend,
            dwFlags: ULW_ALPHA,
            prcDirty: dirty.as_ref().map(|r| r as *const RECT).unwrap_or(std::ptr::null()),
            ..Default::default()
        };
        let _ = UpdateLayeredWindowIndirect(hwnd, &info);
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.bmp);
            let _ = DeleteDC(self.dc);
        }
    }
}

thread_local! {
    static SURFACE: RefCell<Option<Surface>> = const { RefCell::new(None) };
}

/// Буфер размера w×h (пересоздаётся, если размер окна сменился).
pub unsafe fn with(w: i32, h: i32, f: impl FnOnce(&Surface)) {
    SURFACE.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.as_ref().map(|s| (s.w, s.h)) != Some((w, h)) {
            // Сначала освобождаем старый: два буфера сразу — лишние мегабайты.
            *slot = None;
            *slot = Surface::new(w, h);
        }
        if let Some(s) = slot.as_ref() {
            f(s);
        }
    });
}

/// Буфер последнего кадра как есть (для покадровой перерисовки визуализатора).
pub unsafe fn with_current(f: impl FnOnce(&Surface)) {
    SURFACE.with(|cell| {
        if let Some(s) = cell.borrow().as_ref() {
            f(s);
        }
    });
}
