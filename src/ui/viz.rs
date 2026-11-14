//! Полоски визуализатора. Кадр рисуется только в своём прямоугольнике (`on_frame`), без полной
//! перерисовки панели: 30 раз в секунду перерисовывать всё окно со шрифтами было бы заметно.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::GdiFlush;

use super::paint::{rgb, BG};
use super::surface::{self, Canvas};
use crate::app::{scale, st};
use crate::feat::viz::{self, PENDING, SPECTRUM};

/// Где в окне полоски и сколько их (записывает отрисовка, читает `on_frame`).
#[derive(Clone, Copy, PartialEq)]
pub struct VizArea {
    pub rect: RECT,
    pub bars: usize,
    pub colors: (COLORREF, COLORREF),
}

/// Нейтральный цвет полосок, когда обложки нет или она серая.
pub const PLAIN: COLORREF = rgb(212, 212, 216);

/// Цвета полосок из обложки: левой и правой половины. Берутся только насыщенные пиксели,
/// потом цвет осветляется, чтобы читался на чёрном. Серая обложка даёт нейтральный цвет.
pub fn tint(a: &crate::bg::winrt::Art) -> (COLORREF, COLORREF) {
    let half = |x0: i32, x1: i32| -> COLORREF {
        let (mut sr, mut sg, mut sb, mut sw, mut n) = (0f32, 0f32, 0f32, 0f32, 0u32);
        for y in (0..a.h).step_by(3) {
            for x in (x0..x1).step_by(3) {
                let i = ((y * a.w + x) * 4) as usize;
                let Some(p) = a.px.get(i..i + 4) else { continue };
                let (b, g, r) = (p[0] as f32, p[1] as f32, p[2] as f32);
                n += 1;
                let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
                if mx < 50.0 {
                    continue;
                }
                let sat = (mx - mn) / mx;
                if sat < 0.25 {
                    continue;
                }
                sr += r * sat;
                sg += g * sat;
                sb += b * sat;
                sw += sat;
            }
        }
        // Насыщенных пикселей почти нет: обложка чёрно-белая.
        if n == 0 || sw < n as f32 * 0.04 {
            return PLAIN;
        }
        let (r, g, b) = (sr / sw, sg / sw, sb / sw);
        let k = 235.0 / r.max(g).max(b).max(1.0);
        rgb((r * k).min(255.0) as u8, (g * k).min(255.0) as u8, (b * k).min(255.0) as u8)
    };
    (half(0, a.w / 2), half(a.w / 2, a.w))
}

fn lerp(a: COLORREF, b: COLORREF, t: f32) -> COLORREF {
    let ch = |sh: u32| {
        let (x, y) = (((a.0 >> sh) & 0xFF) as f32, ((b.0 >> sh) & 0xFF) as f32);
        ((x + (y - x) * t).round() as u32) << sh
    };
    COLORREF(ch(0) | ch(8) | ch(16))
}

/// Нарисовать `n` полосок по текущему спектру в прямоугольник `r`; цвет плавно идёт от `colors.0` к `colors.1`.
pub unsafe fn draw(cv: Canvas, dpi: u32, r: RECT, n: usize, colors: (COLORREF, COLORREF)) {
    let levels = viz::group(&SPECTRUM.lock().unwrap(), n);
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    // В панели полосок много и зазор шире, в свёрнутой пилюле их четыре и зазор уже.
    let gap = scale(if n > 8 { 3 } else { 2 }, dpi).max(1);
    let bw = ((w - gap * (n as i32 - 1)) / n as i32).max(1);
    let min_h = scale(2, dpi).min(h);
    for (i, lv) in levels.iter().enumerate() {
        let color = lerp(colors.0, colors.1, if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 });
        let bh = (min_h + ((h - min_h) as f32 * lv) as i32).min(h);
        let left = r.left + i as i32 * (bw + gap);
        // Полоски растут от нижнего края.
        cv.fill_round(RECT { left, top: r.bottom - bh, right: left + bw, bottom: r.bottom }, scale(4, dpi).min(bw), color);
    }
}

/// WM_APP_VIZ: новый кадр спектра. Перерисовываем только полоски.
pub unsafe fn on_frame(hwnd: HWND) {
    let (area, dpi) = st(|s| (s.viz_area, s.shell.dpi));
    if let Some(a) = area {
        surface::with_current(|sf| {
            let cv = sf.canvas();
            let _ = GdiFlush();
            cv.fill_round(a.rect, 0, BG);
            draw(cv, dpi, a.rect, a.bars, a.colors);
            sf.present(hwnd, Some(a.rect), None);
        });
    }
    // Кадр нарисован: наблюдатель может слать следующий.
    PENDING.store(false, std::sync::atomic::Ordering::Release);
}
