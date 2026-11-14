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
        return;
    }
    let art = p.r(24, 58, 172, 206);
    match &m.art {
        Some(a) => p.art(art, a, R_TILE),
        None => {
            p.fill(art, R_TILE, PANEL);
            p.text(p.f.icon_big, I_MUSIC, art, DIM, DT_CENTER);
        }
    }
    let x = p.s(192);
    let right = w - p.s(24);
    p.text(p.f.small, &m.app, RECT { left: x, top: p.s(62), right, bottom: p.s(80) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    let title = if m.title.is_empty() { "Без названия" } else { &m.title };
    p.text(p.f.title, title, RECT { left: x, top: p.s(82), right, bottom: p.s(110) }, FG, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.normal, &m.artist, RECT { left: x, top: p.s(110), right, bottom: p.s(132) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    let playing = m.playing;
    let progress = m.progress();
    let can_seek = m.can_seek;
    // Лок отпускаем до кнопок: никогда не держим его дольше, чем нужно.
    drop(m);

    p.icon_button(p.r(186, 154, 230, 198), I_PREV, Hit::Music(MusicHit::Prev), false);
    let play = p.r(236, 150, 288, 202);
    // Главная кнопка белая с чёрным значком.
    p.fill(play, 52, if p.hovered(Hit::Music(MusicHit::Play)) { rgb(220, 220, 224) } else { ACCENT });
    p.text(p.f.icon_big, if playing { I_PAUSE } else { I_PLAY }, play, INK, DT_CENTER);
    p.hit(play, Hit::Music(MusicHit::Play));
    p.icon_button(p.r(294, 154, 338, 198), I_NEXT, Hit::Music(MusicHit::Next), false);
    // Куда играет звук.
    if let Some(dev) = &s.ctl.snap.out_dev {
        let tw = p.measure(p.f.small, dev).min(p.s(180));
        let right = p.s(EXPANDED_W - 24);
        let chip = RECT { left: right - tw - p.s(44), top: p.s(164), right, bottom: p.s(188) };
        p.fill(chip, R_PILL, PANEL);
        p.text(p.f.icon, I_VOL, RECT { left: chip.left + p.s(8), right: chip.left + p.s(30), ..chip }, DIM, DT_LEFT);
        p.text(p.f.small, dev, RECT { left: chip.left + p.s(32), right: chip.right - p.s(10), ..chip }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    }
    // Спектр на всю ширину под перемоткой: раньше нижняя треть панели пустовала.
    if playing {
        p.viz(p.r(24, 248, EXPANDED_W - 24, 288), crate::feat::viz::BANDS);
    }

    // Время трека и полоска перемотки.
    if let Some((cur, dur)) = progress {
        let right = EXPANDED_W - 24;
        p.text(p.f.small, &fmt_time(cur), p.r(24, 216, 70, 234), DIM, DT_LEFT);
        p.text(p.f.small, &fmt_time(dur), p.r(right - 56, 216, right, 234), DIM, DT_RIGHT);
        let bar = p.r(76, 223, right - 62, 227);
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

    p.volume_row(24, 300, EXPANDED_W - 24, s.ctl.snap.volume);
}
