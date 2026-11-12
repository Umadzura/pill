//! Полоски визуализатора. Кадр рисуется только в своём прямоугольнике (`on_frame`), без полной
//! перерисовки панели: 30 раз в секунду перерисовывать всё окно со шрифтами было бы заметно.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::{ACCENT, BG};
use crate::app::{scale, st};
use crate::feat::viz::{self, PENDING, SPECTRUM};

/// Где на экране полоски и сколько их (записывает отрисовка, читает `on_frame`).
#[derive(Clone, Copy, PartialEq)]
pub struct VizArea {
    pub rect: RECT,
    pub bars: usize,
}

/// Нарисовать `n` полосок по текущему спектру в прямоугольник `r` (координаты контекста `hdc`).
pub unsafe fn draw(hdc: HDC, dpi: u32, r: RECT, n: usize) {
    let levels = viz::group(&SPECTRUM.lock().unwrap(), n);
    let w = r.right - r.left;
    let h = r.bottom - r.top;
    let gap = scale(2, dpi).max(1);
    let bw = ((w - gap * (n as i32 - 1)) / n as i32).max(1);
    let min_h = scale(2, dpi).max(1);
    let br = CreateSolidBrush(ACCENT);
    for (i, lv) in levels.iter().enumerate() {
        let bh = (min_h + ((h - min_h) as f32 * lv) as i32).min(h);
        let left = r.left + i as i32 * (bw + gap);
        // Полоски растут от центра: так смотрятся живее и ровнее в низкой пилюле.
        let top = r.top + (h - bh) / 2;
        let rad = bw.min(scale(3, dpi));
        let rg = CreateRoundRectRgn(left, top, left + bw + 1, top + bh + 1, rad, rad);
        let _ = FillRgn(hdc, rg, br);
        let _ = DeleteObject(rg);
    }
    let _ = DeleteObject(br);
}

/// WM_APP_VIZ: новый кадр спектра. Перерисовываем только полоски.
pub unsafe fn on_frame(hwnd: HWND) {
    let (area, dpi) = st(|s| (s.viz_area, s.shell.dpi));
    if let Some(a) = area {
        let (w, h) = (a.rect.right - a.rect.left, a.rect.bottom - a.rect.top);
        if w > 0 && h > 0 {
            let hdc = GetDC(hwnd);
            let mem = CreateCompatibleDC(hdc);
            let bmp = CreateCompatibleBitmap(hdc, w, h);
            let old = SelectObject(mem, bmp);
            let bg = CreateSolidBrush(BG);
            FillRect(mem, &RECT { left: 0, top: 0, right: w, bottom: h }, bg);
            let _ = DeleteObject(bg);
            draw(mem, dpi, RECT { left: 0, top: 0, right: w, bottom: h }, a.bars);
            let _ = BitBlt(hdc, a.rect.left, a.rect.top, w, h, mem, 0, 0, SRCCOPY);
            SelectObject(mem, old);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(mem);
            ReleaseDC(hwnd, hdc);
        }
    }
    // Кадр нарисован: наблюдатель может слать следующий.
    PENDING.store(false, std::sync::atomic::Ordering::Release);
}
