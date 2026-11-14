//! Перетаскивание файлов: приём на окно пилюли (`IDropTarget`) и отдача с полки (`SHDoDragDrop`).
//! `State` не знает: о входе, уходе и броске сообщает окну через `WM_APP_DROP`, пути кладёт в `DROPPED`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use windows::core::{implement, Result, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, POINTL, WPARAM};
use windows::Win32::System::Com::{IBindCtx, IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    IDropSource, IDropTarget, IDropTarget_Impl, OleInitialize, RegisterDragDrop, ReleaseStgMedium, CF_HDROP, DROPEFFECT,
    DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, IShellItem, SHCreateItemFromParsingName, SHDoDragDrop, BHID_DataObject, HDROP};
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::ids::WM_APP_DROP;

/// `wParam` у `WM_APP_DROP`: файл над окном, ушёл, брошен (пути в `DROPPED`).
pub const ENTER: usize = 0;
pub const LEAVE: usize = 1;
pub const DROP: usize = 2;

/// Пути последнего броска; UI-поток забирает их по `WM_APP_DROP`.
pub static DROPPED: Mutex<Vec<String>> = Mutex::new(Vec::new());
/// Сейчас тащим с полки сами: на своё окно такое не принимаем.
static OUTGOING: AtomicBool = AtomicBool::new(false);
/// Больше файлов за один бросок не берём.
const MAX_FILES: u32 = 32;

fn format() -> FORMATETC {
    FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

#[implement(IDropTarget)]
struct Target {
    hwnd: isize,
    ok: AtomicBool,
}

impl Target {
    fn post(&self, what: usize) {
        unsafe {
            let _ = PostMessageW(HWND(self.hwnd as *mut _), WM_APP_DROP, WPARAM(what), LPARAM(0));
        }
    }
}

/// Копируем, а не переносим: полка хранит только путь, файл остаётся где был.
unsafe fn effect(ok: bool, allowed: *mut DROPEFFECT) {
    if allowed.is_null() {
        return;
    }
    let a = (*allowed).0;
    *allowed = if !ok {
        DROPEFFECT_NONE
    } else if a & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY
    } else if a & DROPEFFECT_LINK.0 != 0 {
        DROPEFFECT_LINK
    } else {
        DROPEFFECT_NONE
    };
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Option<&IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        let ok = !OUTGOING.load(Ordering::Relaxed) && data.is_some_and(|d| unsafe { d.QueryGetData(&format()) }.is_ok());
        self.ok.store(ok, Ordering::Relaxed);
        unsafe { effect(ok, eff) };
        if ok {
            self.post(ENTER);
        }
        Ok(())
    }
    fn DragOver(&self, _: MODIFIERKEYS_FLAGS, _: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        unsafe { effect(self.ok.load(Ordering::Relaxed), eff) };
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        if self.ok.swap(false, Ordering::Relaxed) {
            self.post(LEAVE);
        }
        Ok(())
    }
    fn Drop(&self, data: Option<&IDataObject>, _: MODIFIERKEYS_FLAGS, _: &POINTL, eff: *mut DROPEFFECT) -> Result<()> {
        let ok = self.ok.swap(false, Ordering::Relaxed);
        unsafe { effect(ok, eff) };
        if !ok {
            return Ok(());
        }
        let paths = data.map(|d| unsafe { paths(d) }).unwrap_or_default();
        *DROPPED.lock().unwrap() = paths;
        self.post(DROP);
        Ok(())
    }
}

unsafe fn paths(d: &IDataObject) -> Vec<String> {
    let Ok(mut m) = d.GetData(&format()) else { return Vec::new() };
    let h = HDROP(m.u.hGlobal.0);
    let n = DragQueryFileW(h, u32::MAX, None).min(MAX_FILES);
    let mut out = Vec::new();
    for i in 0..n {
        let len = DragQueryFileW(h, i, None) as usize;
        if len == 0 {
            continue;
        }
        let mut buf = vec![0u16; len + 1];
        let got = DragQueryFileW(h, i, Some(&mut buf)) as usize;
        out.push(String::from_utf16_lossy(&buf[..got.min(len)]));
    }
    ReleaseStgMedium(&mut m);
    out
}

/// Окно пилюли принимает файлы. Звать один раз после создания окна.
pub unsafe fn register(hwnd: HWND) {
    // Поток уже в STA (CoInitializeEx), OleInitialize нужен самому RegisterDragDrop.
    let _ = OleInitialize(None);
    let t: IDropTarget = Target { hwnd: hwnd.0 as isize, ok: AtomicBool::new(false) }.into();
    let _ = RegisterDragDrop(hwnd, &t);
}

/// Вытащить файл в другое окно (копией или ярлыком, не переносом). Модальный цикл до броска:
/// звать вне `st()`. false — файла нет или перетаскивание не началось.
pub unsafe fn drag_out(hwnd: HWND, path: &str) -> bool {
    let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path), None::<&IBindCtx>) else { return false };
    let Ok(data) = item.BindToHandler::<_, IDataObject>(None::<&IBindCtx>, &BHID_DataObject) else { return false };
    OUTGOING.store(true, Ordering::Relaxed);
    let r = SHDoDragDrop(hwnd, &data, None::<&IDropSource>, DROPEFFECT(DROPEFFECT_COPY.0 | DROPEFFECT_LINK.0));
    OUTGOING.store(false, Ordering::Relaxed);
    r.is_ok()
}
