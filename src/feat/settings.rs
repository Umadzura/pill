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
    /// Звуки (пока один: сигнал таймера).
    pub sound: bool,
    extra: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { collapse_ms: COLLAPSE_DELAY_MS, dot_mode: false, sound: true, extra: Vec::new() }
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
        for l in &self.extra {
            out.push_str(l);
            out.push('\n');
        }
        out
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
        assert_eq!(s.serialize().lines().count(), 3);
    }
}
