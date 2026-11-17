//! Снимок экрана открывает чёлку: картинка, которую «Ножницы» (Win+Shift+S или кнопка «Скриншот»)
//! положили в буфер, выезжает карточкой с миниатюрой и кнопками «Открыть», «На полку», «Сохранить как».
//! Буфер читает UI-поток (`sys::clipboard::read_image`), PNG пишет и миниатюру делает воркер
//! (`winrt::Job::Snap` -> `WM_APP_SNAP`). Файлы лежат в `%LOCALAPPDATA%\Pill\snips`, последние 20.

use std::sync::Arc;
use std::time::{Duration, Instant};

use windows::core::{w, HSTRING};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::{FileSaveDialog, IFileSaveDialog, SIGDN_FILESYSPATH, FOS_OVERWRITEPROMPT, FOS_FORCEFILESYSTEM};

use crate::app::st;
use crate::bg::winrt;
use crate::feat::peek::{self, Peek, PeekKind};
use crate::sys;

/// Сколько карточка снимка держится без курсора.
pub const PEEK_MS: u32 = 8000;
/// Картинка в буфере в течение этого времени после кнопки «Скриншот» — точно наш снимок.
const AFTER_SHOT: Duration = Duration::from_secs(120);

#[derive(Default)]
pub struct Snaps {
    /// Нажали «Скриншот»: до этого момента любая картинка в буфере считается снимком.
    pub wait: Option<Instant>,
    /// Номер содержимого буфера, которое уже разобрали.
    pub seq: u32,
    /// Отпечаток последней картинки: одну и ту же программы кладут в буфер дважды подряд
    /// (с разными номерами), второй карточки и файла не нужно.
    pub last: u64,
}

/// Дешёвый отпечаток картинки: длина и каждый 4096-й байт (весь снимок 4K гонять незачем).
fn fingerprint(b: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.len().hash(&mut h);
    b.iter().step_by(4096).for_each(|x| x.hash(&mut h));
    b[b.len().saturating_sub(64)..].hash(&mut h);
    h.finish()
}

/// Программы, которые кладут в буфер снимок экрана.
fn is_snipper(exe: &str) -> bool {
    let e = exe.to_ascii_lowercase();
    e == "screenclippinghost.exe" || e == "snippingtool.exe"
}

/// Нажата кнопка «Скриншот» в «Управлении».
pub fn armed() {
    st(|s| s.snaps.wait = Some(Instant::now() + AFTER_SHOT));
}

/// В буфере не текст (WM_CLIPBOARDUPDATE): если это снимок экрана, отдать его воркеру.
pub unsafe fn on_clipboard(hwnd: HWND) {
    let seq = sys::clipboard::sequence();
    let waited = st(|s| {
        if s.snaps.seq == seq {
            return None;
        }
        s.snaps.seq = seq;
        Some(s.snaps.wait.is_some_and(|t| Instant::now() < t))
    });
    let Some(waited) = waited else { return };
    let owner = sys::clipboard::owner();
    let snipper = !owner.0.is_null() && sys::wnd::process_name(owner).is_some_and(|n| is_snipper(&n));
    if !waited && !snipper {
        return;
    }
    let Some(image) = sys::clipboard::read_image(hwnd) else { return };
    let fp = match &image {
        sys::clipboard::Image::Png(b) | sys::clipboard::Image::Dib(b) => fingerprint(b),
    };
    let keep = st(|s| {
        if std::mem::replace(&mut s.snaps.last, fp) == fp {
            return None;
        }
        s.snaps.wait = None;
        Some(s.shelf.items.iter().map(|i| i.path.clone()).collect())
    });
    let Some(keep) = keep else { return };
    winrt::send(winrt::Job::Snap { image, keep });
}

/// Воркер записал снимок (`WM_APP_SNAP`): показать карточку.
pub unsafe fn on_ready(hwnd: HWND) {
    let Some((path, thumb)) = winrt::SNAP.lock().unwrap().take() else { return };
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let mut caption = crate::feat::shelf::size_text(size);
    let mut pk = Peek::new(PeekKind::Snap, String::new(), "Снимок экрана".into(), String::new());
    if let Some((art, w, h)) = thumb {
        caption = format!("{}×{} · {}", w, h, caption);
        pk.art = Some(Arc::new(art));
    }
    pk.body = caption;
    pk.file = path;
    peek::show(hwnd, pk, Some(PEEK_MS));
}

/// «Сохранить как»: системное окно сохранения, потом копия файла. Модальное: звать вне `st()`.
pub unsafe fn save_as(hwnd: HWND, path: &str) {
    let Some(name) = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()) else { return };
    let Ok(dlg) = CoCreateInstance::<_, IFileSaveDialog>(&FileSaveDialog, None, CLSCTX_INPROC_SERVER) else { return };
    if let Ok(opts) = dlg.GetOptions() {
        let _ = dlg.SetOptions(opts | FOS_OVERWRITEPROMPT | FOS_FORCEFILESYSTEM);
    }
    let _ = dlg.SetFileName(&HSTRING::from(name));
    let _ = dlg.SetDefaultExtension(w!("png"));
    if dlg.Show(hwnd).is_err() {
        return; // отменили
    }
    let Ok(item) = dlg.GetResult() else { return };
    let Ok(p) = item.GetDisplayName(SIGDN_FILESYSPATH) else { return };
    let to = p.to_string().unwrap_or_default();
    CoTaskMemFree(Some(p.0 as *const _));
    if to.is_empty() || to.eq_ignore_ascii_case(path) {
        return;
    }
    let msg = match std::fs::copy(path, &to) {
        Ok(_) => format!("Сохранено: {}", std::path::Path::new(&to).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or(to)),
        Err(_) => "Не удалось сохранить снимок".into(),
    };
    crate::feat::osd::show(hwnd, crate::feat::osd::Flash::Text(msg, false), 2500);
}

#[cfg(test)]
mod tests {
    #[test]
    fn snippers() {
        assert!(super::is_snipper("ScreenClippingHost.exe") && super::is_snipper("SnippingTool.exe"));
        assert!(!super::is_snipper("chrome.exe"));
    }
}
