//! Чтение и запись буфера обмена Windows: текст, а для снимков экрана ещё картинка (PNG или CF_DIB).

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::*;
use windows::Win32::System::Ole::{CF_DIB, CF_UNICODETEXT};
use windows::Win32::System::Threading::Sleep;

/// Картинка больше этого в буфере не берём (8K с запасом).
const IMAGE_MAX_BYTES: usize = 160 << 20;

/// Картинка из буфера: готовый PNG или «сырой» CF_DIB (его разбирает `dib_to_bgra`).
pub enum Image {
    Png(Vec<u8>),
    Dib(Vec<u8>),
}

/// Номер текущего содержимого буфера: одно и то же событие может прийти дважды.
pub fn sequence() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// Окно, положившее в буфер то, что там сейчас (по нему узнаём «Ножницы»).
pub fn owner() -> HWND {
    unsafe { GetClipboardOwner().unwrap_or_default() }
}

/// Владелец буфера попросил не сохранять запись: менеджеры паролей и вообще все, кто прячет
/// содержимое от истории Windows (Win+V). Три признака: `ExcludeClipboardContentFromMonitorProcessing`,
/// `Clipboard Viewer Ignore` (есть формат — не берём) и `CanIncludeInClipboardHistory` со значением 0.
/// Звать при открытом буфере: значение последнего формата читается через GetClipboardData.
unsafe fn excluded() -> bool {
    let has = |name: windows::core::PCWSTR| {
        let f = RegisterClipboardFormatW(name);
        f != 0 && IsClipboardFormatAvailable(f).is_ok()
    };
    if has(w!("ExcludeClipboardContentFromMonitorProcessing")) || has(w!("Clipboard Viewer Ignore")) {
        return true;
    }
    let can = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
    if can == 0 || IsClipboardFormatAvailable(can).is_err() {
        return false;
    }
    let Ok(h) = GetClipboardData(can) else { return false };
    let hg = HGLOBAL(h.0);
    let p = GlobalLock(hg) as *const u8;
    if p.is_null() {
        return false;
    }
    let deny = history_denied(std::slice::from_raw_parts(p, GlobalSize(hg).min(4)));
    let _ = GlobalUnlock(hg);
    deny
}

/// Значение `CanIncludeInClipboardHistory`: DWORD, 0 — не класть в историю, 1 — можно.
/// Короткие и кривые данные запретом не считаем.
fn history_denied(d: &[u8]) -> bool {
    d.len() >= 4 && u32::from_le_bytes([d[0], d[1], d[2], d[3]]) == 0
}

/// Картинка из буфера. Сначала PNG («Ножницы» кладут и его), иначе CF_DIB.
/// GetClipboardData может попросить владельца отрисовать данные: звать вне `st()`.
pub unsafe fn read_image(hwnd: HWND) -> Option<Image> {
    let png = RegisterClipboardFormatW(w!("PNG"));
    let fmt = if png != 0 && IsClipboardFormatAvailable(png).is_ok() {
        png
    } else if IsClipboardFormatAvailable(CF_DIB.0 as u32).is_ok() {
        CF_DIB.0 as u32
    } else {
        return None;
    };
    if !open(hwnd) {
        return None;
    }
    if excluded() {
        let _ = CloseClipboard();
        return None;
    }
    let mut bytes = None;
    if let Ok(h) = GetClipboardData(fmt) {
        let hg = HGLOBAL(h.0);
        let n = GlobalSize(hg);
        let p = GlobalLock(hg) as *const u8;
        if !p.is_null() {
            if n > 0 && n <= IMAGE_MAX_BYTES {
                bytes = Some(std::slice::from_raw_parts(p, n).to_vec());
            }
            let _ = GlobalUnlock(hg);
        }
    }
    let _ = CloseClipboard();
    let b = bytes?;
    Some(if fmt == png { Image::Png(b) } else { Image::Dib(b) })
}

/// CF_DIB (BITMAPINFOHEADER и пиксели) -> ширина, высота и BGRA сверху вниз, альфа 255.
/// Берём только 24 и 32 бита без сжатия (или 32 бита с обычными масками): так кладут снимки экрана.
pub fn dib_to_bgra(d: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let u32_at = |o: usize| d.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let hsize = u32_at(0)? as usize;
    let w = u32_at(4)? as i32;
    let h = u32_at(8)? as i32;
    let bpp = d.get(14..16).map(|b| u16::from_le_bytes([b[0], b[1]]))?;
    let comp = u32_at(16)?;
    if hsize < 40 || w <= 0 || h == 0 || w > 16384 || h.unsigned_abs() > 16384 {
        return None;
    }
    let mut off = hsize;
    match (bpp, comp) {
        (24, 0) | (32, 0) => {}
        (32, 3) => {
            // Маски цветов: в коротком заголовке они идут сразу за ним, в V4/V5 — внутри.
            let (r, g, b) = (u32_at(40)?, u32_at(44)?, u32_at(48)?);
            if (r, g, b) != (0xFF_0000, 0xFF00, 0xFF) {
                return None;
            }
            if hsize == 40 {
                off += 12;
            }
        }
        _ => return None,
    }
    let (w, top_down, h) = (w as usize, h < 0, h.unsigned_abs() as usize);
    let px = bpp as usize / 8;
    let stride = (w * px + 3) & !3;
    if d.len() < off + stride * h {
        return None;
    }
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        let src_y = if top_down { y } else { h - 1 - y };
        let row = &d[off + src_y * stride..];
        let dst = &mut out[y * w * 4..(y + 1) * w * 4];
        for x in 0..w {
            let s = &row[x * px..x * px + 3];
            dst[x * 4..x * 4 + 4].copy_from_slice(&[s[0], s[1], s[2], 255]);
        }
    }
    Some((w as u32, h as u32, out))
}

unsafe fn open(hwnd: HWND) -> bool {
    // Буфер может быть занят другой программой: пара коротких попыток.
    for _ in 0..4 {
        if OpenClipboard(hwnd).is_ok() {
            return true;
        }
        Sleep(15);
    }
    false
}

/// Текст из буфера. None, если там не текст или менеджер паролей попросил не сохранять.
pub unsafe fn read_text(hwnd: HWND) -> Option<String> {
    if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() || !open(hwnd) {
        return None;
    }
    if excluded() {
        let _ = CloseClipboard();
        return None;
    }
    let mut result = None;
    if let Ok(h) = GetClipboardData(CF_UNICODETEXT.0 as u32) {
        let hg = HGLOBAL(h.0);
        let p = GlobalLock(hg) as *const u16;
        if !p.is_null() {
            let max = GlobalSize(hg) / 2;
            let mut n = 0;
            while n < max && *p.add(n) != 0 {
                n += 1;
            }
            result = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
            let _ = GlobalUnlock(hg);
        }
    }
    let _ = CloseClipboard();
    result
}

pub unsafe fn set_text(hwnd: HWND, text: &str) -> bool {
    put_text(hwnd, text, false)
}

/// Секрет (ключ API): с пометками «не сохранять», чтобы он не осел ни в истории Pill,
/// ни в журнале буфера Windows (Win+V), ни у других менеджеров буфера.
pub unsafe fn set_secret(hwnd: HWND, text: &str) -> bool {
    put_text(hwnd, text, true)
}

/// Положить байты в буфер в формате `fmt` (буфер уже открыт и очищен).
unsafe fn put(fmt: u32, bytes: &[u8]) -> bool {
    let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else { return false };
    let p = GlobalLock(hg) as *mut u8;
    let mut ok = false;
    if !p.is_null() {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(hg);
        ok = SetClipboardData(fmt, HANDLE(hg.0)).is_ok();
    }
    if !ok {
        let _ = GlobalFree(hg);
    }
    ok
}

unsafe fn put_text(hwnd: HWND, text: &str, secret: bool) -> bool {
    let bytes: Vec<u8> = text.encode_utf16().chain(Some(0)).flat_map(u16::to_le_bytes).collect();
    if !open(hwnd) {
        return false;
    }
    let mut ok = false;
    if EmptyClipboard().is_ok() {
        ok = put(CF_UNICODETEXT.0 as u32, &bytes);
        if ok && secret {
            let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
            let history = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
            let cloud = RegisterClipboardFormatW(w!("CanUploadToCloudClipboard"));
            put(exclude, &[0]);
            for f in [history, cloud] {
                put(f, &0u32.to_le_bytes());
            }
        }
    }
    let _ = CloseClipboard();
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(w: i32, h: i32, bpp: u16, comp: u32) -> Vec<u8> {
        let mut d = Vec::new();
        for v in [40u32, w as u32, h as u32] {
            d.extend_from_slice(&v.to_le_bytes());
        }
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&bpp.to_le_bytes());
        d.extend_from_slice(&comp.to_le_bytes());
        d.resize(40, 0);
        d
    }

    #[test]
    fn dib_rows_and_formats() {
        // 24 бита, 1×2, снизу вверх, строка дополнена до 4 байт: нижняя строка в файле первой.
        let mut d = header(1, 2, 24, 0);
        d.extend_from_slice(&[1, 2, 3, 0, 4, 5, 6, 0]);
        let (w, h, px) = dib_to_bgra(&d).unwrap();
        assert_eq!((w, h), (1, 2));
        assert_eq!(px, vec![4, 5, 6, 255, 1, 2, 3, 255]);
        // 32 бита с масками за коротким заголовком, сверху вниз; мусорная альфа заменяется на 255.
        let mut d = header(2, -1, 32, 3);
        for m in [0xFF_0000u32, 0xFF00, 0xFF] {
            d.extend_from_slice(&m.to_le_bytes());
        }
        d.extend_from_slice(&[10, 20, 30, 0, 40, 50, 60, 7]);
        assert_eq!(dib_to_bgra(&d).unwrap().2, vec![10, 20, 30, 255, 40, 50, 60, 255]);
        // Палитра и обрезанные данные не берём.
        assert!(dib_to_bgra(&header(1, 1, 8, 0)).is_none());
        assert!(dib_to_bgra(&header(4, 4, 32, 0)).is_none());
    }

    #[test]
    fn history_flag() {
        assert!(history_denied(&[0, 0, 0, 0]));
        assert!(!history_denied(&[1, 0, 0, 0]));
        assert!(!history_denied(&[0, 0]));
        assert!(!history_denied(&[]));
    }
}
