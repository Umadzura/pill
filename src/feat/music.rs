//! Музыка: кнопки плеера и перемотка. Данные трека живут в `bg::winrt::MEDIA` (пишет воркер).

use windows::Win32::Foundation::RECT;

use crate::app::Redraw;
use crate::bg::winrt::{send, Job};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MusicHit {
    Prev,
    Play,
    Next,
    /// Полоска перемотки: доля считается от x клика внутри зоны.
    Seek,
}

pub fn on_hit(h: MusicHit, rect: RECT, x: i32) -> Redraw {
    match h {
        MusicHit::Prev => send(Job::Prev),
        MusicHit::Play => send(Job::PlayPause),
        MusicHit::Next => send(Job::Next),
        MusicHit::Seek => {
            let w = (rect.right - rect.left).max(1);
            send(Job::Seek((x - rect.left) as f64 / w as f64));
        }
    }
    // Плеер ответит событием, тогда и перерисуемся.
    Redraw::None
}
