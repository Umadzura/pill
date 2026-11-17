//! Хуки Claude Code: поставить и убрать записи в `~/.claude/settings.json`, через которые Claude Code
//! шлёт события в Pill. Наши записи узнаются по адресу в команде (`MARK`), чужие не трогаем.
//! В команду вписан ключ API (заголовок `X-Pill-Key`): без него `/hook` отвечает 403.
//! Перед записью старый файл копируется рядом в `settings.json.pill-backup`.

use std::path::PathBuf;

use super::json::{parse, Json};

/// По этой подстроке в команде хука узнаём свои записи.
pub const MARK: &str = "127.0.0.1:47821/hook";
/// События без ответа: хук работает в фоне и Claude Code не ждёт.
/// `SubagentStart`/`SubagentStop` и `PreToolUse` по Agent — субагенты в чёлке (этап 23).
const EVENTS: [&str; 8] = ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Notification", "Stop", "SessionEnd", "SubagentStart", "SubagentStop"];
/// У каких событий хук ставится не на всё, а по имени инструмента: `PreToolUse` нужен только
/// для описания задачи субагента, на каждый инструмент его гонять незачем.
fn matcher(e: &str) -> Option<&'static str> {
    (e == "PreToolUse").then_some("Agent")
}
/// Запрос разрешения: хук ждёт ответа из пилюли, поэтому синхронный и с запасом по времени.
const ASK: &str = "PermissionRequest";

/// Что стоит в settings.json.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum HookState {
    /// Наших записей нет.
    Off,
    /// Наши есть, но не такие, как ставит эта версия (без ключа, с другим ключом, не на всех событиях):
    /// нужно переустановить, иначе `/hook` ответит 403.
    Stale,
    On,
}

/// Команда хука. Заголовок `-H X-Pill-Key:ключ` без пробелов и кавычек: так одинаково читают
/// bash, cmd и PowerShell (ключ — только hex). Синхронный хук разрешений ждёт ответа до 40 с.
fn command(key: &str, ask: bool) -> String {
    format!("curl.exe -s -m {} --connect-timeout 1 -H X-Pill-Key:{} --data-binary @- http://{}", if ask { 40 } else { 2 }, key, MARK)
}

fn ours(h: &Json) -> bool {
    h.get("command").and_then(|c| c.as_str()).is_some_and(|c| c.contains(MARK))
}

/// Команды наших хуков на событии.
fn our_commands<'a>(events: &'a Json, e: &str) -> Vec<&'a str> {
    let Some(Json::Arr(gs)) = events.get(e) else { return Vec::new() };
    gs.iter()
        .filter_map(|g| match g.get("hooks") {
            Some(Json::Arr(hs)) => Some(hs),
            _ => None,
        })
        .flatten()
        .filter(|h| ours(h))
        .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
        .collect()
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

fn group(cmd: &str, ask: bool, matcher: Option<&str>) -> Json {
    let mut h = vec![("type".to_string(), Json::Str("command".into())), ("command".to_string(), Json::Str(cmd.into()))];
    if ask {
        h.push(("timeout".to_string(), Json::Num("45".into())));
    } else {
        h.push(("async".to_string(), Json::Bool(true)));
    }
    let mut g = Vec::new();
    if let Some(m) = matcher {
        g.push(("matcher".to_string(), Json::Str(m.into())));
    }
    g.push(("hooks".to_string(), Json::Arr(vec![Json::Obj(h)])));
    Json::Obj(g)
}

/// Поставить наши записи с ключом (старые свои сначала убираются, чтобы не было дублей).
pub fn install(root: &mut Json, key: &str) {
    strip(root);
    let Some(events) = root.entry("hooks", Json::Obj(Vec::new())) else { return };
    for e in EVENTS.iter().chain([ASK].iter()) {
        if let Some(Json::Arr(gs)) = events.entry(e, Json::Arr(Vec::new())) {
            gs.push(group(&command(key, *e == ASK), *e == ASK, matcher(e)));
        }
    }
}

/// Стоят ли наши хуки, и ровно ли такие, как ставит эта версия с этим ключом.
pub fn state(root: &Json, key: &str) -> HookState {
    let Some(events) = root.get("hooks") else { return HookState::Off };
    let (mut any, mut fresh) = (false, !key.is_empty());
    for e in EVENTS.iter().chain([ASK].iter()) {
        let cmds = our_commands(events, e);
        any |= !cmds.is_empty();
        fresh &= cmds.len() == 1 && cmds[0] == command(key, *e == ASK);
    }
    // Свои записи на других событиях (от прежних версий) тоже повод переустановить.
    if let Json::Obj(all) = events {
        let extra = all.iter().any(|(e, _)| !EVENTS.contains(&e.as_str()) && e != ASK && !our_commands(events, e).is_empty());
        any |= extra;
        fresh &= !extra;
    }
    match (any, fresh) {
        (false, _) => HookState::Off,
        (true, true) => HookState::On,
        (true, false) => HookState::Stale,
    }
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

/// Стоят ли хуки сейчас (`key` — ключ API из настроек). Читает маленький файл: звать вне `st()`.
pub fn status(key: &str) -> Result<HookState, String> {
    read().map(|v| state(&v, key))
}

/// Поставить (true) с ключом `key` или убрать (false) хуки и записать файл. Звать вне `st()`.
pub fn apply(on: bool, key: &str) -> Result<(), String> {
    if on && key.is_empty() {
        return Err("нет ключа API: settings.txt Pill не записать".into());
    }
    let mut v = read()?;
    if on {
        install(&mut v, key);
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

    const K: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn install_keeps_foreign_hooks() {
        let src = r#"{"model":"opus","hooks":{"Stop":[{"hooks":[{"type":"command","command":"echo mine"}]},{"hooks":[{"type":"command","command":"curl.exe -s -m 2 --data-binary @- http://127.0.0.1:47821/hook","async":true}]}]}}"#;
        let mut v = parse(src).unwrap();
        assert_eq!(state(&v, K), HookState::Stale);
        install(&mut v, K);
        assert_eq!(state(&v, K), HookState::On);
        // Чужой хук на месте, свой ровно один на событие.
        let stop = match v.get("hooks").and_then(|h| h.get("Stop")) {
            Some(Json::Arr(g)) => g.clone(),
            _ => panic!(),
        };
        assert_eq!(stop.len(), 2);
        assert!(v.dump().contains("echo mine"));
        assert_eq!(v.dump().matches(MARK).count(), 9);
        // Ключ в каждой команде.
        assert_eq!(v.dump().matches(&format!("-H X-Pill-Key:{} ", K)).count(), 9);
        // Синхронный хук разрешений с тайм-аутом.
        assert!(v.dump().contains("\"timeout\": 45"));
        strip(&mut v);
        assert!(state(&v, K) == HookState::Off && v.dump().contains("echo mine") && !v.dump().contains(MARK));
        assert_eq!(v.get("model").and_then(|m| m.as_str()), Some("opus"));
        // Только наши хуки — после удаления раздела hooks не остаётся.
        let mut only = Json::Obj(vec![]);
        install(&mut only, K);
        strip(&mut only);
        assert_eq!(only, Json::Obj(vec![]));
    }

    #[test]
    fn old_hooks_need_reinstall() {
        // Полный набор прежней версии без ключа: «нужно переустановить».
        let old = "curl.exe -s -m 2 --connect-timeout 1 --data-binary @- http://127.0.0.1:47821/hook";
        let mut v = Json::Obj(vec![]);
        install(&mut v, K);
        let v_old = parse(&v.dump().replace(&format!("-H X-Pill-Key:{} ", K), "")).unwrap();
        assert!(v_old.dump().contains(old));
        assert_eq!(state(&v_old, K), HookState::Stale);
        // С другим ключом тоже.
        assert_eq!(state(&v, "ffffffffffffffffffffffffffffffff"), HookState::Stale);
        // Ключа нет (settings.txt не записался) — считать установленными нельзя.
        assert_eq!(state(&v, ""), HookState::Stale);
        // Переустановка заменяет старые записи, дублей нет.
        let mut again = v_old.clone();
        install(&mut again, K);
        assert_eq!(state(&again, K), HookState::On);
        assert_eq!(again.dump().matches(MARK).count(), 9);
        // PreToolUse только по Agent: на каждый инструмент хук не гоняем.
        assert!(again.dump().split_whitespace().collect::<String>().contains(r#""matcher":"Agent""#));
        assert!(!again.dump().contains(old));
        // Не на всех событиях — тоже переустановить.
        let mut part = v.clone();
        if let Some(Json::Obj(ev)) = part.entry("hooks", Json::Obj(vec![])) {
            ev.retain(|(e, _)| e != "Stop");
        }
        assert_eq!(state(&part, K), HookState::Stale);
        // Свой хук на лишнем событии (прежняя версия) — тоже.
        let mut more = v.clone();
        if let Some(Json::Obj(ev)) = more.entry("hooks", Json::Obj(vec![])) {
            ev.push(("PreCompact".into(), Json::Arr(vec![group(old, false, None)])));
        }
        assert_eq!(state(&more, K), HookState::Stale);
        install(&mut more, K);
        assert_eq!(state(&more, K), HookState::On);
    }
}
