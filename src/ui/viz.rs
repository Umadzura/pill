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
}

/// Нарисовать `n` полосок по текущему спектру в прямоугольник `r`.
pub unsafe fn draw(cv: Canvas, dpi: u32, r: RECT, n: usize) {
    let levels = viz::group(&SPECTRUM.lock().unwrap(), n);
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    // В панели полосок много и зазор шире, в свёрнутой пилюле их четыре и зазор уже.
    let gap = scale(if n > 8 { 3 } else { 2 }, dpi).max(1);
    let bw = ((w - gap * (n as i32 - 1)) / n as i32).max(1);
    let min_h = scale(2, dpi).min(h);
    // Белый с лёгким приглушением, как текст: акцентный синий оставлен переключателям.
    let color = rgb(212, 212, 216);
    for (i, lv) in levels.iter().enumerate() {
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
            draw(cv, dpi, a.rect, a.bars);
            sf.present(hwnd, Some(a.rect));
        });
    }
    // Кадр нарисован: наблюдатель может слать следующий.
    PENDING.store(false, std::sync::atomic::Ordering::Release);
}
