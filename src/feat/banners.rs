//! «Прятать баннеры Windows»: сообщения выезжают только из чёлки, в центре уведомлений они остаются.
//!
//! Включили — у всех программ, что уже слали уведомления, выключаем баннер (`sys::banners`).
//! Программа, которая напишет впервые, получает ключ в реестре только после первого уведомления,
//! поэтому её первый баннер Windows ещё покажет, а дальше уже нет (воркер ловит новый AUMID в `seen`).
//! Выключили — возвращаем баннеры ровно тем, кому выключали мы: список в `%APPDATA%\Pill\banners.txt`,
//! выключенное руками в Параметрах не трогаем. Системные уведомления Windows (Bluetooth, автозапуск,
//! защитник) не трогаем: у их баннеров бывают кнопки, которых в чёлке нет.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::sys::banners as reg;

/// Настройка включена (копия `Settings.hide_banners` для воркера).
pub static HIDE: AtomicBool = AtomicBool::new(false);

struct Book {
    /// Кому баннер выключили мы (то, что в файле).
    ours: Vec<String>,
    /// Какие AUMID воркер уже проверил в этом запуске: реестр не читаем на каждом опросе.
    seen: Vec<String>,
}

static BOOK: Mutex<Book> = Mutex::new(Book { ours: Vec::new(), seen: Vec::new() });

/// Программы из папок Windows: так Windows пишет AUMID без ярлыка ({FOLDERID}\путь к exe).
/// System32, Windows, SysWOW64.
const SYSTEM_FOLDERS: [&str; 3] = ["{1ac14e77-02e7-4e5d-b744-2eb1ae5198b7}", "{f38bf404-1d43-42f2-9305-67de0b28fc23}", "{d65231b0-b2f1-4857-a4ce-a8e7c6ea7d27}"];

/// Системные уведомления Windows и пустой AUMID не трогаем.
pub fn skip(aumid: &str) -> bool {
    let a = aumid.to_ascii_lowercase();
    a.is_empty() || a.starts_with("windows.") || a.starts_with("microsoft.windows.") || SYSTEM_FOLDERS.iter().any(|f| a.starts_with(f))
}

fn parse(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !out.iter().any(|x| x == l) {
            out.push(l.to_string());
        }
    }
    out
}

fn path() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|a| PathBuf::from(a).join("Pill").join("banners.txt"))
}

fn save(ours: &[String]) {
    let Some(p) = path() else { return };
    if ours.is_empty() {
        let _ = std::fs::remove_file(p);
        return;
    }
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, ours.join("\n") + "\n");
}

/// Выключить баннер одной программы, если его показывают и это не системная. true — выключили.
fn hide_one(book: &mut Book, aumid: &str) -> bool {
    if skip(aumid) || book.ours.iter().any(|x| x == aumid) || !reg::banner_shown(aumid) {
        return false;
    }
    reg::set_banner(aumid, false);
    book.ours.push(aumid.to_string());
    true
}

/// При старте: прочитать свой список и, если настройка включена, догнать программы, появившиеся без нас.
pub fn init(on: bool) {
    let ours = path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| parse(&t)).unwrap_or_default();
    BOOK.lock().unwrap().ours = ours;
    set(on);
}

/// Включить или выключить настройку. Реестр и файл: звать вне `st()`.
pub fn set(on: bool) {
    HIDE.store(on, Ordering::Relaxed);
    let mut book = BOOK.lock().unwrap();
    let changed = if on {
        let mut any = false;
        for a in reg::apps() {
            any |= hide_one(&mut book, &a);
        }
        any
    } else {
        for a in std::mem::take(&mut book.ours) {
            // Руками включили обратно — уже показывается, трогать нечего.
            if !reg::banner_shown(&a) {
                reg::set_banner(&a, true);
            }
        }
        book.seen.clear();
        true
    };
    if changed {
        save(&book.ours);
    }
}

/// Воркер прочитал центр уведомлений: у программ, написавших впервые, выключить баннер.
pub fn seen<'a>(aumids: impl Iterator<Item = &'a str>) {
    if !HIDE.load(Ordering::Relaxed) {
        return;
    }
    let mut book = BOOK.lock().unwrap();
    let mut changed = false;
    for a in aumids {
        if book.seen.iter().any(|x| x == a) {
            continue;
        }
        book.seen.push(a.to_string());
        changed |= hide_one(&mut book, a);
    }
    if changed {
        save(&book.ours);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_apps_skipped() {
        assert!(skip("") && skip("Windows.SystemToast.BthQuickPair") && skip("windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel"));
        assert!(skip("Microsoft.Windows.InputSwitchToastHandler") && skip("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}") && skip("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\cmd.exe"));
        assert!(!skip("{6D809377-6AF0-444B-8957-A3773F02200E}\\App\\app.exe"));
        assert!(!skip("Telegram.TelegramDesktop") && !skip("Microsoft.PowerToysWin32") && !skip("Spotify.desktop.client"));
    }

    #[test]
    fn list_file() {
        assert_eq!(parse("Telegram.TelegramDesktop\r\n\n Spotify.desktop.client \nTelegram.TelegramDesktop\n"), vec!["Telegram.TelegramDesktop".to_string(), "Spotify.desktop.client".into()]);
        assert!(parse("").is_empty());
    }
}
