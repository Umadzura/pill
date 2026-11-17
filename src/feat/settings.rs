//! Настройки: `%APPDATA%\Pill\settings.txt`, строки «ключ=значение».
//! Читаются один раз при старте, пишутся при смене из меню трея.
//! Незнакомые строки (ключи будущих версий, комментарии) сохраняются как есть.

use std::path::PathBuf;

use crate::ids::COLLAPSE_DELAY_MS;

#[derive(Clone, PartialEq, Debug)]
pub struct Settings {
    /// Через сколько мс после ухода курсора панель сворачивается.
    pub collapse_ms: u32,
    /// «Просто точка»: карточки из чёлки не выезжают, о событии говорит только свёрнутая чёлка.
    pub dot_mode: bool,
    /// Звуки: таймер, просьбы агентов, «готов».
    pub sound: bool,
    /// Тихие часы (с какого часа по какой): ни звуков, ни карточек чужих уведомлений.
    pub quiet: Option<(u8, u8)>,
    /// Не показывать карточки агента, если его окно (терминал, редактор) и так активно.
    pub skip_focused: bool,
    /// История буфера только в памяти: `clipboard.txt` не пишется, после выхода записи пропадают.
    pub clip_memory: bool,
    extra: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            collapse_ms: COLLAPSE_DELAY_MS,
            dot_mode: false,
            sound: true,
            quiet: None,
            skip_focused: true,
            clip_memory: false,
            extra: Vec::new(),
        }
    }
}

fn flag(v: &str) -> Option<bool> {
    match v {
        "1" | "true" | "on" => Some(true),
        "0" | "false" | "off" => Some(false),
        _ => None,
    }
}

impl Settings {
    /// Разбор файла. Знакомый ключ с кривым значением остаётся по умолчанию.
    pub fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let known = match line.split_once('=') {
                Some((k, v)) => {
                    let v = v.trim();
                    match k.trim() {
                        "collapse_ms" => {
                            if let Ok(n) = v.parse::<u32>() {
                                s.collapse_ms = n.clamp(100, 5000);
                            }
                            true
                        }
                        "dot_mode" => {
                            s.dot_mode = flag(v).unwrap_or(s.dot_mode);
                            true
                        }
                        "sound" => {
                            s.sound = flag(v).unwrap_or(s.sound);
                            true
                        }
                        "quiet" => {
                            s.quiet = v.split_once('-').and_then(|(a, b)| Some((a.trim().parse::<u8>().ok()?.min(23), b.trim().parse::<u8>().ok()?.min(23))));
                            true
                        }
                        "skip_focused" => {
                            s.skip_focused = flag(v).unwrap_or(s.skip_focused);
                            true
                        }
                        "clip_memory" => {
                            s.clip_memory = flag(v).unwrap_or(s.clip_memory);
                            true
                        }
                        _ => false,
                    }
                }
                None => false,
            };
            if !known {
                s.extra.push(line.to_string());
            }
        }
        s
    }

    pub fn serialize(&self) -> String {
        let mut out = format!("collapse_ms={}\ndot_mode={}\nsound={}\n", self.collapse_ms, self.dot_mode as u8, self.sound as u8);
        match self.quiet {
            Some((a, b)) => out.push_str(&format!("quiet={}-{}\n", a, b)),
            None => out.push_str("quiet=off\n"),
        }
        out.push_str(&format!("skip_focused={}\n", self.skip_focused as u8));
        out.push_str(&format!("clip_memory={}\n", self.clip_memory as u8));
        for l in &self.extra {
            out.push_str(l);
            out.push('\n');
        }
        out
    }

    /// Сейчас тихие часы (`hour` — местный час).
    pub fn quiet_now(&self, hour: u16) -> bool {
        match self.quiet {
            Some((a, b)) if a <= b => (a as u16..b as u16).contains(&hour),
            Some((a, b)) => hour >= a as u16 || hour < b as u16,
            None => false,
        }
    }

    /// Можно ли сейчас звучать.
    pub fn sounds_on(&self) -> bool {
        self.sound && !self.quiet_now(crate::sys::local_hm().0)
    }

    pub fn load() -> Settings {
        path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Settings::parse(&t)).unwrap_or_default()
    }

    /// Пишет файл. Звать вне `st()`.
    pub fn save(&self) {
        let Some(p) = path() else { return };
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, self.serialize());
    }
}

fn path() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|a| PathBuf::from(a).join("Pill").join("settings.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_roundtrip() {
        assert_eq!(Settings::parse(""), Settings::default());
        let s = Settings::parse("collapse_ms=600\r\n# моя пометка\ndot_mode=1\nsound=off\nfuture_key=abc\n");
        assert_eq!((s.collapse_ms, s.dot_mode, s.sound), (600, true, false));
        // Незнакомое не теряется при записи.
        let again = Settings::parse(&s.serialize());
        assert_eq!(again, s);
        assert!(s.serialize().contains("future_key=abc") && s.serialize().contains("# моя пометка"));
    }

    #[test]
    fn bad_values_keep_defaults() {
        let s = Settings::parse("collapse_ms=abc\ndot_mode=maybe\ncollapse_ms=99999\n");
        assert_eq!(s.collapse_ms, 5000);
        assert!(!s.dot_mode);
        // Кривые строки знакомых ключей не копятся в файле.
        assert_eq!(s.serialize().lines().count(), Settings::default().serialize().lines().count());
    }

    #[test]
    fn quiet_hours() {
        let s = Settings::parse("quiet=23-8\nskip_focused=0\n");
        assert!(s.quiet_now(23) && s.quiet_now(3) && !s.quiet_now(8) && !s.quiet_now(15));
        assert!(!s.skip_focused);
        assert_eq!(Settings::parse(&s.serialize()), s);
        let day = Settings::parse("quiet=9-18\n");
        assert!(day.quiet_now(9) && !day.quiet_now(18));
        assert!(!Settings::parse("quiet=off\n").quiet_now(3));
    }

    #[test]
    fn clip_memory_flag() {
        assert!(!Settings::default().clip_memory);
        let s = Settings::parse("clip_memory=1\n");
        assert!(s.clip_memory);
        assert!(s.serialize().contains("clip_memory=1\n"));
        assert_eq!(Settings::parse(&s.serialize()), s);
        // Кривое значение оставляет историю на диске и не копится в `extra`.
        let bad = Settings::parse("clip_memory=maybe\n");
        assert!(!bad.clip_memory);
        assert_eq!(bad.serialize(), Settings::default().serialize());
    }
}
