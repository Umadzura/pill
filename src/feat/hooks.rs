//! Хуки Claude Code: поставить и убрать записи в `~/.claude/settings.json`, через которые Claude Code
//! шлёт события в Pill. Наши записи узнаются по адресу в команде (`MARK`), чужие не трогаем.
//! Перед записью старый файл копируется рядом в `settings.json.pill-backup`.

use std::path::PathBuf;

use super::json::{parse, Json};

/// По этой подстроке в команде хука узнаём свои записи.
pub const MARK: &str = "127.0.0.1:47821/hook";
/// События без ответа: хук работает в фоне и Claude Code не ждёт.
const EVENTS: [&str; 5] = ["UserPromptSubmit", "PostToolUse", "Notification", "Stop", "SessionEnd"];
/// Запрос разрешения: хук ждёт ответа из пилюли, поэтому синхронный и с запасом по времени.
const ASK: &str = "PermissionRequest";
const CMD: &str = "curl.exe -s -m 2 --connect-timeout 1 --data-binary @- http://127.0.0.1:47821/hook";
const CMD_ASK: &str = "curl.exe -s -m 40 --connect-timeout 1 --data-binary @- http://127.0.0.1:47821/hook";

fn ours(h: &Json) -> bool {
    h.get("command").and_then(|c| c.as_str()).is_some_and(|c| c.contains(MARK))
}

/// Убрать все наши записи; пустые группы и пустые списки событий тоже.
pub fn strip(root: &mut Json) {
    let Some(Json::Obj(events)) = root.entry("hooks", Json::Obj(Vec::new())) else { return };
    for (_, groups) in events.iter_mut() {
        let Json::Arr(gs) = groups else { continue };
        for g in gs.iter_mut() {
            if let Some(Json::Arr(hs)) = g.entry("hooks", Json::Arr(Vec::new())) {
                hs.retain(|h| !ours(h));
            }
        }
        gs.retain(|g| !matches!(g.get("hooks"), Some(Json::Arr(h)) if h.is_empty()));
    }
    events.retain(|(_, g)| !matches!(g, Json::Arr(v) if v.is_empty()));
    if matches!(root.get("hooks"), Some(Json::Obj(v)) if v.is_empty()) {
        root.remove("hooks");
    }
}

fn group(cmd: &str, ask: bool) -> Json {
    let mut h = vec![("type".to_string(), Json::Str("command".into())), ("command".to_string(), Json::Str(cmd.into()))];
    if ask {
        h.push(("timeout".to_string(), Json::Num("45".into())));
    } else {
        h.push(("async".to_string(), Json::Bool(true)));
    }
    Json::Obj(vec![("hooks".to_string(), Json::Arr(vec![Json::Obj(h)]))])
}

/// Поставить наши записи (старые свои сначала убираются, чтобы не было дублей).
pub fn install(root: &mut Json) {
    strip(root);
    let Some(events) = root.entry("hooks", Json::Obj(Vec::new())) else { return };
    for e in EVENTS.iter().chain([ASK].iter()) {
        if let Some(Json::Arr(gs)) = events.entry(e, Json::Arr(Vec::new())) {
            gs.push(if *e == ASK { group(CMD_ASK, true) } else { group(CMD, false) });
        }
    }
}

/// Стоят ли наши хуки на всех нужных событиях.
pub fn installed(root: &Json) -> bool {
    let Some(events) = root.get("hooks") else { return false };
    EVENTS.iter().chain([ASK].iter()).all(|e| match events.get(e) {
        Some(Json::Arr(gs)) => gs.iter().any(|g| matches!(g.get("hooks"), Some(Json::Arr(hs)) if hs.iter().any(ours))),
        _ => false,
    })
}

pub fn path() -> Option<PathBuf> {
    std::env::var("USERPROFILE").ok().map(|h| PathBuf::from(h).join(".claude").join("settings.json"))
}

/// Прочитать настройки Claude Code. Нет файла — пустой объект. Битый файл — ошибка словами.
fn read() -> Result<Json, String> {
    let p = path().ok_or("не найдена папка пользователя")?;
    match std::fs::read_to_string(&p) {
        Ok(t) if t.trim().is_empty() => Ok(Json::Obj(Vec::new())),
        Ok(t) => parse(&t).filter(|v| matches!(v, Json::Obj(_))).ok_or_else(|| "settings.json не разобрать: поправь его вручную".to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Json::Obj(Vec::new())),
        Err(_) => Err("settings.json не прочитать".into()),
    }
}

/// Стоят ли хуки сейчас. Читает маленький файл: звать вне `st()`.
pub fn status() -> Result<bool, String> {
    read().map(|v| installed(&v))
}

/// Поставить (true) или убрать (false) хуки и записать файл. Звать вне `st()`.
pub fn apply(on: bool) -> Result<(), String> {
    let mut v = read()?;
    if on {
        install(&mut v);
    } else {
        strip(&mut v);
    }
    let p = path().ok_or("не найдена папка пользователя")?;
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if p.exists() {
        let _ = std::fs::copy(&p, p.with_file_name("settings.json.pill-backup"));
    }
    std::fs::write(&p, v.dump()).map_err(|_| "settings.json не записать".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_keeps_foreign_hooks() {
        let src = r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo mine"}]},{"hooks":[{"type":"command","command":"curl.exe -s -m 2 --data-binary @- http://127.0.0.1:47821/hook","async":true}]}]}}"#;
        let mut v = parse(src).unwrap();
        assert!(!installed(&v));
        install(&mut v);
        assert!(installed(&v));
        // Чужой хук на месте, свой ровно один на событие.
        let stop = match v.get("hooks").and_then(|h| h.get("Stop")) {
            Some(Json::Arr(g)) => g.clone(),
            _ => panic!(),
        };
        assert_eq!(stop.len(), 2);
        assert!(v.dump().contains("echo mine"));
        assert_eq!(v.dump().matches(MARK).count(), 6);
        // Синхронный хук разрешений с тайм-аутом.
        assert!(v.dump().contains("\"timeout\": 45"));
        strip(&mut v);
        assert!(!installed(&v) && v.dump().contains("echo mine") && !v.dump().contains(MARK));
        assert_eq!(v.get("model").and_then(|m| m.as_str()), Some("opus"));
        // Только наши хуки — после удаления раздела hooks не остаётся.
        let mut only = Json::Obj(vec![]);
        install(&mut only);
        strip(&mut only);
        assert_eq!(only, Json::Obj(vec![]));
    }
}
