//! Обновление по кнопке в «О программе» (этап 19). Фоном ничего не проверяем: один запрос к
//! GitHub Releases по нажатию, скачивание по второму нажатию.
//!
//! Формат релиза (его же делает этап 20): тег `vX.Y.Z`, файл `pill.exe` и рядом `pill.exe.sha256`
//! (hex, можно в формате `sha256sum`). Нет файла с хешем — берём `digest` («sha256:…»), который GitHub
//! сам пишет у файла. Без хеша не ставим.
//!
//! Как ставим. Воркер качает `pill.exe.new` рядом с собой и сверяет размер и SHA-256. UI переименовывает
//! себя в `pill.exe.old`, новый файл — в `pill.exe` (путь не меняется: хуки и автозапуск указывают на него)
//! и закрывает окно. Старый процесс отпускает мьютекс, хоткеи и трей, запускает новый с `--after-update`
//! и до 20 с ждёт от него событие `Local\PillUpdateOk` (новый ставит его на первом секундном тике, то есть
//! когда окно создано и цикл сообщений идёт). Не дождался или новый умер — откат: новый файл удаляется,
//! `.old` возвращается на место и запускается с `--update-failed`. Новый удаляет `.old` сам.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use crate::feat::json::{self, Json};
use crate::sys::http;

pub const REPO: &str = "Umadzura/pill";
const ASSET: &str = "pill.exe";
const SHA_ASSET: &str = "pill.exe.sha256";
const API_MAX: usize = 2 << 20;
const EXE_MAX: usize = 32 << 20;
const SHA_MAX: usize = 4096;

pub const ARG_AFTER: &str = "--after-update";
pub const ARG_FAILED: &str = "--update-failed";
const EVENT: windows::core::PCWSTR = windows::core::w!("Local\\PillUpdateOk");
/// Сколько старый процесс ждёт, что новый запустился.
const HANDOVER_TICKS: u32 = 200; // × 100 мс

pub type Version = (u32, u32, u32);

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub url: String,
    pub size: u64,
    /// Адрес `pill.exe.sha256`, если он есть в релизе.
    pub sha_url: Option<String>,
    /// Хеш, который GitHub пишет у файла (hex, без «sha256:»).
    pub digest: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    /// Стоит последняя версия.
    Latest,
    /// Релизов на GitHub ещё нет.
    NoReleases,
    Available(Release),
    Downloading(Release),
    /// Скачано и проверено: UI переставляет файлы и перезапускается.
    Ready,
    Failed(String),
    /// Этот запуск — сразу после обновления.
    Updated,
    /// Новая версия не запустилась, вернули прежнюю.
    RolledBack,
}

pub static STATUS: Mutex<Status> = Mutex::new(Status::Idle);
/// Файлы переставлены: после выхода из цикла сообщений передать управление новому exe (`hand_over`).
pub static PENDING: AtomicBool = AtomicBool::new(false);
/// Запущены после обновления: на первом тике сказать старому процессу «запустился».
static CONFIRM: AtomicBool = AtomicBool::new(false);
/// Сколько ещё секундных тиков пытаться удалить `pill.exe.old` (старый процесс выходит не сразу).
static CLEANUP: AtomicU32 = AtomicU32::new(0);

/// «v0.3.1», «0.3», «0.3.1-beta» → (0, 3, 1). Нечисловое начало — None.
pub fn version(s: &str) -> Option<Version> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.');
    let mut part = |req: bool| -> Option<u32> {
        match it.next() {
            Some(p) => p.parse().ok(),
            None if req => None,
            None => Some(0),
        }
    };
    let v = (part(true)?, part(false)?, part(false)?);
    it.next().is_none().then_some(v)
}

pub fn current() -> Version {
    version(env!("CARGO_PKG_VERSION")).unwrap_or((0, 0, 0))
}

pub fn show(v: Version) -> String {
    format!("{}.{}.{}", v.0, v.1, v.2)
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Ответ `releases/latest`: версия и файлы. Ошибка — текст для окна настроек.
pub fn parse_release(text: &str) -> Result<Release, String> {
    let j = json::parse(text).ok_or("непонятный ответ GitHub")?;
    let tag = j.get("tag_name").and_then(Json::as_str).ok_or("в релизе нет версии")?.to_string();
    let version = version(&tag).ok_or_else(|| format!("непонятная версия {}", tag))?;
    let Some(Json::Arr(assets)) = j.get("assets") else { return Err("в релизе нет файлов".into()) };
    let find = |name: &str| assets.iter().find(|a| a.get("name").and_then(Json::as_str) == Some(name));
    let exe = find(ASSET).ok_or("в релизе нет pill.exe")?;
    let url = exe.get("browser_download_url").and_then(Json::as_str).ok_or("нет ссылки на pill.exe")?.to_string();
    let size = match exe.get("size") {
        Some(Json::Num(n)) => n.parse::<u64>().ok(),
        _ => None,
    }
    .filter(|&n| n > 0 && n <= EXE_MAX as u64)
    .ok_or("неверный размер pill.exe")?;
    let sha_url = find(SHA_ASSET).and_then(|a| a.get("browser_download_url")).and_then(Json::as_str).map(str::to_string);
    let digest = exe.get("digest").and_then(Json::as_str).and_then(|d| d.strip_prefix("sha256:")).filter(|d| is_hex64(d)).map(str::to_ascii_lowercase);
    Ok(Release { version, tag, url, size, sha_url, digest })
}

/// Хеш из файла `pill.exe.sha256`: просто hex или строки `sha256sum` («hex  pill.exe», «hex *pill.exe»).
pub fn parse_sha(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let pick = |l: &&str| {
        let mut it = l.split_whitespace();
        let h = it.next()?;
        let name = it.next().map(|n| n.trim_start_matches('*'));
        (is_hex64(h) && name.map_or(true, |n| n == ASSET)).then(|| h.to_ascii_lowercase())
    };
    lines.iter().find_map(pick)
}

/// Сравнить с установленной версией.
pub fn decide(cur: Version, rel: Release) -> Status {
    if rel.version > cur {
        Status::Available(rel)
    } else {
        Status::Latest
    }
}

/// Адрес API релизов. `PILL_UPDATE_API` — для проверки на своём сервере (`http://` только 127.0.0.1, см. `sys::http`).
fn api_url() -> String {
    std::env::var("PILL_UPDATE_API").unwrap_or_else(|_| format!("https://api.github.com/repos/{}/releases/latest", REPO))
}

/// Воркер: спросить GitHub. Блокирует до 15 с на шаг.
pub fn check() -> Status {
    match http::get(&api_url(), "application/vnd.github+json", API_MAX) {
        Ok(body) => match parse_release(&String::from_utf8_lossy(&body)) {
            Ok(rel) => decide(current(), rel),
            Err(e) => Status::Failed(e),
        },
        // У репозитория без релизов `latest` отвечает 404.
        Err(e) if e == "HTTP 404" => Status::NoReleases,
        Err(e) => Status::Failed(e),
    }
}

fn exe_path() -> Option<PathBuf> {
    std::env::current_exe().ok()
}

/// `pill.exe` → `pill.exe.new` / `.old`.
fn side(ext: &str) -> Option<PathBuf> {
    let mut s = exe_path()?.into_os_string();
    s.push(ext);
    Some(s.into())
}

/// Воркер: скачать, сверить размер и SHA-256, положить `pill.exe.new`.
pub fn download(rel: &Release) -> Result<(), String> {
    let want = match &rel.sha_url {
        Some(u) => parse_sha(&String::from_utf8_lossy(&http::get(u, "application/octet-stream", SHA_MAX)?)).ok_or("непонятный файл с хешем")?,
        None => rel.digest.clone().ok_or("в релизе нет SHA-256, ставить не буду")?,
    };
    let bytes = http::get(&rel.url, "application/octet-stream", EXE_MAX)?;
    if bytes.len() as u64 != rel.size || !bytes.starts_with(b"MZ") {
        return Err("скачался не тот файл".into());
    }
    if http::sha256(&bytes).as_deref() != Some(want.as_str()) {
        return Err("хеш не совпал, файл не тронут".into());
    }
    let new = side(".new").ok_or("не найден свой exe")?;
    std::fs::write(&new, &bytes).map_err(|_| "нет прав писать в папку Pill".to_string())
}

/// UI: переставить файлы. После успеха окно закрывается, а `main` зовёт `hand_over`.
pub fn swap() -> Result<(), String> {
    let (Some(exe), Some(old), Some(new)) = (exe_path(), side(".old"), side(".new")) else { return Err("не найден свой exe".into()) };
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|_| "не удалось переименовать pill.exe".to_string())?;
    if std::fs::rename(&new, &exe).is_err() {
        let _ = std::fs::rename(&old, &exe);
        return Err("не удалось поставить новый файл".into());
    }
    PENDING.store(true, Ordering::Relaxed);
    Ok(())
}

/// После цикла сообщений старого процесса (мьютекс уже закрыт): запустить новый и дождаться,
/// что он поднялся; иначе вернуть прежний exe.
pub fn hand_over() {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    let Some(exe) = exe_path_for_handover() else { return };
    let old = PathBuf::from(format!("{}.old", exe.to_string_lossy()));
    let ev = unsafe { CreateEventW(None, true, false, EVENT) }.ok();
    let mut ok = false;
    if let Ok(mut child) = std::process::Command::new(&exe).arg(ARG_AFTER).spawn() {
        for _ in 0..HANDOVER_TICKS {
            if let Some(e) = ev {
                if unsafe { WaitForSingleObject(e, 100) } == WAIT_OBJECT_0 {
                    ok = true;
                    break;
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
        }
        if !ok {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    if let Some(e) = ev {
        unsafe {
            let _ = CloseHandle(e);
        }
    }
    if ok {
        return;
    }
    // Откат: новый exe мог ещё держаться системой пару мгновений после выхода.
    for _ in 0..20 {
        if std::fs::remove_file(&exe).is_ok() || !exe.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    if std::fs::rename(&old, &exe).is_ok() {
        let _ = std::process::Command::new(&exe).args([ARG_AFTER, ARG_FAILED]).spawn();
    }
}

/// Путь, на котором должен лежать exe. Свой файл уже переименован в `.old`; `current_exe` обычно
/// помнит путь запуска, но на случай, если вернёт новое имя, суффикс срезаем.
fn exe_path_for_handover() -> Option<PathBuf> {
    let cur = exe_path()?;
    let s = cur.to_string_lossy();
    Some(PathBuf::from(s.strip_suffix(".old").unwrap_or(&s).to_string()))
}

/// Запущены с `--after-update` (в том числе после отката).
static AFTER: AtomicBool = AtomicBool::new(false);

pub fn is_after() -> bool {
    AFTER.load(Ordering::Relaxed)
}

/// Разбор командной строки при старте (до окна).
pub fn startup(args: &[String]) {
    let after = args.iter().any(|a| a == ARG_AFTER);
    let failed = args.iter().any(|a| a == ARG_FAILED);
    AFTER.store(after, Ordering::Relaxed);
    if failed {
        *STATUS.lock().unwrap() = Status::RolledBack;
    } else if after {
        *STATUS.lock().unwrap() = Status::Updated;
        CONFIRM.store(true, Ordering::Relaxed);
        CLEANUP.store(30, Ordering::Relaxed);
    } else {
        // Обычный запуск: подчистить хвосты прошлых попыток.
        for ext in [".new", ".old"] {
            if let Some(p) = side(ext) {
                let _ = std::fs::remove_file(p);
            }
        }
    }
}

/// Запущены после обновления: ждать, пока старый процесс отпустит мьютекс.
pub fn after_update(args: &[String]) -> bool {
    args.iter().any(|a| a == ARG_AFTER)
}

/// Секундный тик. Возвращает надпись для пилюли, если только что поднялись после обновления.
pub fn tick() -> Option<String> {
    let mut flash = None;
    if CONFIRM.swap(false, Ordering::Relaxed) {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
        unsafe {
            if let Ok(e) = OpenEventW(EVENT_MODIFY_STATE, false, EVENT) {
                let _ = SetEvent(e);
                let _ = CloseHandle(e);
            }
        }
        flash = Some(format!("Pill обновлён до {}", show(current())));
    }
    let left = CLEANUP.load(Ordering::Relaxed);
    if left > 0 {
        let gone = side(".old").map_or(true, |p| std::fs::remove_file(&p).is_ok() || !p.exists());
        CLEANUP.store(if gone { 0 } else { left - 1 }, Ordering::Relaxed);
    }
    flash
}

/// Строка «Обновления» в окне настроек: подпись и кнопка (текст, номер действия, главная ли).
pub fn row_text(s: &Status) -> (String, Option<(&'static str, u8, bool)>) {
    let cur = show(current());
    let check = Some(("Проверить", 6, false));
    match s {
        Status::Idle => (format!("установлена {}", cur), check),
        Status::Checking => ("проверяю…".into(), None),
        Status::Latest => (format!("установлена {}, это последняя", cur), check),
        Status::NoReleases => (format!("установлена {}, релизов пока нет", cur), check),
        Status::Available(r) => (format!("установлена {}, доступна {}", cur, show(r.version)), Some(("Обновить", 7, true))),
        Status::Downloading(r) => (format!("скачиваю {}…", show(r.version)), None),
        Status::Ready => ("перезапускаюсь…".into(), None),
        Status::Failed(e) => (format!("не вышло: {}", e), check),
        Status::Updated => (format!("обновлено до {}", cur), check),
        Status::RolledBack => (format!("новая версия не запустилась, вернул {}", cur), check),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(version("v0.3.1"), Some((0, 3, 1)));
        assert_eq!(version("0.3"), Some((0, 3, 0)));
        assert_eq!(version("V1"), Some((1, 0, 0)));
        assert_eq!(version("0.3.1-beta.2"), Some((0, 3, 1)));
        assert_eq!(version("release"), None);
        assert_eq!(version("1.2.3.4"), None);
        assert!(version("0.10.0") > version("0.9.9"));
        assert_eq!(version(env!("CARGO_PKG_VERSION")), Some(current()));
    }

    fn release_json(extra_assets: &str, digest: &str) -> String {
        format!(
            r#"{{"tag_name":"v0.3.0","name":"Pill 0.3","assets":[{{"name":"pill.exe","size":1127936,"digest":"{}","browser_download_url":"https://github.com/Umadzura/pill/releases/download/v0.3.0/pill.exe"}}{}]}}"#,
            digest, extra_assets
        )
    }

    #[test]
    fn release() {
        let h = "a".repeat(64);
        let r = parse_release(&release_json(r#",{"name":"pill.exe.sha256","size":75,"browser_download_url":"https://x/pill.exe.sha256"}"#, &format!("sha256:{}", h.to_uppercase()))).unwrap();
        assert_eq!((r.version, r.size, r.tag.as_str()), ((0, 3, 0), 1127936, "v0.3.0"));
        assert_eq!(r.sha_url.as_deref(), Some("https://x/pill.exe.sha256"));
        assert_eq!(r.digest.as_deref(), Some(h.as_str()));
        let bare = parse_release(&release_json("", "")).unwrap();
        assert!(bare.sha_url.is_none() && bare.digest.is_none());
        assert!(parse_release(r#"{"tag_name":"v1","assets":[]}"#).unwrap_err().contains("pill.exe"));
        assert!(parse_release("<html>").is_err());
        // Версия новее — предлагаем, та же или старее — «последняя».
        assert!(matches!(decide((0, 2, 0), r.clone()), Status::Available(_)));
        assert_eq!(decide((0, 3, 0), r.clone()), Status::Latest);
        assert_eq!(decide((0, 4, 0), r), Status::Latest);
    }

    #[test]
    fn sha_file() {
        let h = "0123456789abcdef".repeat(4);
        assert_eq!(parse_sha(&format!("{}\n", h)), Some(h.clone()));
        assert_eq!(parse_sha(&format!("{} *pill.exe\r\n", h.to_uppercase())), Some(h.clone()));
        assert_eq!(parse_sha(&format!("{}  other.zip\n{}  pill.exe\n", "f".repeat(64), h)), Some(h.clone()));
        assert_eq!(parse_sha("nope"), None);
        assert_eq!(parse_sha(&"a".repeat(63)), None);
    }
}
