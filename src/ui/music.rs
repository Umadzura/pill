//! Вкладка «Музыка»: обложка, трек, кнопки, перемотка, громкость.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::bg::winrt;
use crate::feat::music::MusicHit;
use crate::shell::EXPANDED_W;

/// Левый край текстового столбца справа от обложки (логические px).
const X: i32 = 268;

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
    let _ = h;
    let m = winrt::MEDIA.lock().unwrap();
    if !m.has {
        let r = RECT { left: 0, top: p.s(60), right: w, bottom: p.s(150) };
        p.text(p.f.icon_big, I_MUSIC, RECT { bottom: r.top + p.s(40), ..r }, FAINT, DT_CENTER);
        p.text(p.f.normal, "Сейчас ничего не играет", RECT { top: r.top + p.s(48), bottom: r.top + p.s(70), ..r }, DIM, DT_CENTER);
        drop(m);
        p.volume_row(24, 160, EXPANDED_W - 24, s.ctl.snap.volume);
        p.reach(186);
        return;
    }
    // Обложка на всю высоту блока слева.
    let art = p.r(24, 54, 248, 278);
    let colors = match &m.art {
        Some(a) => {
            p.art(art, a, R_TILE);
            super::viz::tint(a)
        }
        None => {
            p.fill(art, R_TILE, PANEL);
            p.text(p.f.icon_big, I_MUSIC, art, FAINT, DT_CENTER);
            (super::viz::PLAIN, super::viz::PLAIN)
        }
    };
    let (x, right) = (p.s(X), w - p.s(24));
    // Сверху мелко: программа и куда играет звук.
    let src = match &s.ctl.snap.out_dev {
        Some(dev) => format!("{} · {}", m.app, dev),
        None => m.app.clone(),
    };
    p.text(p.f.small, &src, RECT { left: x, top: p.s(56), right, bottom: p.s(74) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    let title = if m.title.is_empty() { "Без названия" } else { &m.title };
    p.text(p.f.hero, title, RECT { left: x, top: p.s(78), right, bottom: p.s(108) }, FG, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.normal, &m.artist, RECT { left: x, top: p.s(108), right, bottom: p.s(128) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    let playing = m.playing;
    let progress = m.progress();
    let can_seek = m.can_seek;
    // Лок отпускаем до кнопок: никогда не держим его дольше, чем нужно.
    drop(m);

    // Кнопки: назад, белая «играть», вперёд.
    p.icon_button(p.r(X - 4, 139, X + 32, 175), I_PREV, Hit::Music(MusicHit::Prev), false);
    let play = p.r(X + 42, 135, X + 86, 179);
    p.fill(play, 60, if p.hovered(Hit::Music(MusicHit::Play)) { rgb(220, 220, 224) } else { ACCENT });
    p.text(p.f.icon_big, if playing { I_PAUSE } else { I_PLAY }, play, INK, DT_CENTER);
    p.hit(play, Hit::Music(MusicHit::Play));
    p.icon_button(p.r(X + 96, 139, X + 132, 175), I_NEXT, Hit::Music(MusicHit::Next), false);

    // Справа два слова: что показывать в нижнем углу.
    let mut wx = right;
    for (label, lyr) in [("Текст", true), ("Спектр", false)] {
        let lw = p.measure(p.f.small, label);
        let r = RECT { left: wx - lw, top: p.s(146), right: wx, bottom: p.s(168) };
        let hit = Hit::Music(MusicHit::View(lyr));
        let on = s.lyrics == lyr;
        p.text(p.f.small, label, r, if on { FG } else if p.hovered(hit) { DIM } else { FAINT }, DT_RIGHT);
        p.hit(RECT { left: r.left - p.s(4), right: r.right + p.s(4), ..r }, hit);
        wx = r.left - p.s(14);
    }

    // Правый нижний угол: спектр цветом обложки или текст песни.
    let corner = RECT { left: x, top: p.s(196), right, bottom: p.s(278) };
    if s.lyrics {
        // Текстов пока нет (этап 10): место уже заложено, раскладка не поменяется.
        p.text(p.f.normal, "Текст песни появится позже", RECT { top: corner.top + p.s(20), bottom: corner.top + p.s(42), ..corner }, FAINT, DT_LEFT);
    } else {
        // На паузе полоски лежат ровной линией: угол не пустует.
        p.viz(corner, crate::feat::viz::BANDS, colors);
    }

    // Перемотка тонкой линией.
    if let Some((cur, dur)) = progress {
        let r_end = EXPANDED_W - 24;
        p.text(p.f.small, &fmt_time(cur), p.r(24, 292, 70, 310), DIM, DT_LEFT);
        p.text(p.f.small, &fmt_time(dur), p.r(r_end - 46, 292, r_end, 310), DIM, DT_RIGHT);
        let bar = p.r(70, 300, r_end - 52, 303);
        p.fill(bar, 3, HOVER);
        let fill = RECT { right: bar.left + ((bar.right - bar.left) as f64 * cur / dur) as i32, ..bar };
        let seek = Hit::Music(MusicHit::Seek);
        if fill.right - fill.left >= p.s(3) {
            p.fill(fill, 3, if p.hovered(seek) { DIM } else { FG });
        }
        if can_seek {
            p.hit(RECT { top: bar.top - p.s(8), bottom: bar.bottom + p.s(8), ..bar }, seek);
        }
    }

    p.volume_row(24, 316, EXPANDED_W - 24, s.ctl.snap.volume);
    p.reach(342);
}
