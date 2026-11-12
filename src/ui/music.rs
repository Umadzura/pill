//! Вкладка «Музыка»: обложка, трек, кнопки, перемотка, громкость.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::bg::winrt;
use crate::feat::music::MusicHit;
use crate::shell::EXPANDED_W;

/// Время трека «1:23» / «1:02:03».
fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

pub unsafe fn draw(p: &mut Painter, s: &State, w: i32, h: i32) {
    let m = winrt::MEDIA.lock().unwrap();
    if !m.has {
        let r = RECT { left: 0, top: p.s(50), right: w, bottom: h - p.s(20) };
        p.text(p.f.icon_big, I_MUSIC, RECT { bottom: r.top + p.s(120), ..r }, DIM, DT_CENTER);
        p.text(p.f.normal, "Сейчас ничего не играет", RECT { top: r.top + p.s(110), bottom: r.top + p.s(140), ..r }, DIM, DT_CENTER);
        drop(m);
        p.volume_row(24, 230, EXPANDED_W - 24, s.ctl.snap.volume);
        p.hint(w, h, "Колесо мыши — громкость");
        return;
    }
    let art = p.r(24, 62, 156, 194);
    match &m.art {
        Some(a) => {
            let rg = CreateRoundRectRgn(art.left, art.top, art.right + 1, art.bottom + 1, p.s(18), p.s(18));
            SelectClipRgn(p.hdc, rg);
            SetStretchBltMode(p.hdc, HALFTONE);
            let _ = SetBrushOrgEx(p.hdc, 0, 0, None);
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: a.w,
                    biHeight: -a.h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            StretchDIBits(
                p.hdc,
                art.left,
                art.top,
                art.right - art.left,
                art.bottom - art.top,
                0,
                0,
                a.w,
                a.h,
                Some(a.px.as_ptr() as *const _),
                &bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            SelectClipRgn(p.hdc, None);
            let _ = DeleteObject(rg);
        }
        None => {
            p.fill(art, 18, PANEL);
            p.text(p.f.icon_big, I_MUSIC, art, DIM, DT_CENTER);
        }
    }
    let x = p.s(176);
    let right = w - p.s(24);
    p.text(p.f.small, &m.app, RECT { left: x, top: p.s(64), right, bottom: p.s(82) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    let title = if m.title.is_empty() { "Без названия" } else { &m.title };
    p.text(p.f.title, title, RECT { left: x, top: p.s(84), right, bottom: p.s(112) }, FG, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.normal, &m.artist, RECT { left: x, top: p.s(112), right, bottom: p.s(134) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    let playing = m.playing;
    let progress = m.progress();
    let can_seek = m.can_seek;
    // Лок отпускаем до кнопок: никогда не держим его дольше, чем нужно.
    drop(m);

    p.icon_button(p.r(170, 146, 214, 190), I_PREV, Hit::Music(MusicHit::Prev), false);
    let play = p.r(220, 142, 272, 194);
    p.fill(play, 52, rgb(44, 44, 54));
    p.icon_button(play, if playing { I_PAUSE } else { I_PLAY }, Hit::Music(MusicHit::Play), true);
    p.icon_button(p.r(278, 146, 322, 190), I_NEXT, Hit::Music(MusicHit::Next), false);

    // Время трека и полоска перемотки.
    if let Some((cur, dur)) = progress {
        let right = EXPANDED_W - 24;
        p.text(p.f.small, &fmt_time(cur), p.r(24, 204, 70, 222), DIM, DT_LEFT);
        p.text(p.f.small, &fmt_time(dur), p.r(right - 56, 204, right, 222), DIM, DT_RIGHT);
        let bar = p.r(76, 211, right - 62, 215);
        p.fill(bar, 4, PANEL);
        let fill = RECT { right: bar.left + ((bar.right - bar.left) as f64 * cur / dur) as i32, ..bar };
        let seek = Hit::Music(MusicHit::Seek);
        if fill.right - fill.left >= p.s(4) {
            p.fill(fill, 4, if p.hovered(seek) { ACCENT } else { FG });
        }
        if can_seek {
            p.hit(RECT { top: bar.top - p.s(8), bottom: bar.bottom + p.s(8), ..bar }, seek);
        }
    }

    p.volume_row(24, 240, EXPANDED_W - 24, s.ctl.snap.volume);
    p.hint(w, h, "Колесо мыши — громкость");
}
