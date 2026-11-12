//! Таймеры из командной строки и тревога, когда время вышло.

use std::time::{Duration, Instant};

use crate::feat::cmd::fmt_dur;

pub struct Timer {
    pub end: Instant,
    pub total: u64,
}

#[derive(Default)]
pub struct Timers {
    pub list: Vec<Timer>,
    /// Текст тревоги, пока её не закрыли кликом.
    pub alarm: Option<String>,
    /// Сколько раз ещё пискнуть (раз в секунду).
    beeps: u8,
}

impl Timers {
    pub fn add(&mut self, secs: u64) {
        self.list.push(Timer { end: Instant::now() + Duration::from_secs(secs), total: secs });
    }

    pub fn cancel(&mut self, i: usize) {
        if i < self.list.len() {
            self.list.remove(i);
        }
    }

    pub fn dismiss(&mut self) {
        self.alarm = None;
        self.beeps = 0;
    }

    /// Ближайший таймер: сколько секунд осталось.
    pub fn nearest_left(&self) -> Option<u64> {
        let t = self.list.iter().min_by_key(|t| t.end)?;
        Some(t.end.saturating_duration_since(Instant::now()).as_secs())
    }

    /// Раз в секунду: снять сработавшие, завести тревогу. true — пора пискнуть.
    pub fn tick(&mut self, now: Instant) -> bool {
        if let Some(t) = self.list.iter().find(|t| t.end <= now) {
            self.alarm = Some(format!("Таймер {} — время вышло", fmt_dur(t.total)));
            self.beeps = 6;
        }
        self.list.retain(|t| t.end > now);
        if self.beeps > 0 {
            self.beeps -= 1;
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fires_and_beeps() {
        let mut t = Timers::default();
        t.add(1);
        let later = Instant::now() + Duration::from_secs(2);
        assert!(t.tick(later));
        assert!(t.list.is_empty());
        assert!(t.alarm.as_deref().unwrap().contains("0:01"));
        for _ in 0..5 {
            assert!(t.tick(later));
        }
        assert!(!t.tick(later));
        t.dismiss();
        assert!(t.alarm.is_none());
    }
}
