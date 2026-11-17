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

unsafe fn excluded() -> bool {
    let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
    exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok()
}

/// Картинка из буфера. Сначала PNG («Ножницы» кладут и его), иначе CF_DIB.
/// GetClipboardData может попросить владельца отрисовать данные: звать вне `st()`.
pub unsafe fn read_image(hwnd: HWND) -> Option<Image> {
    if excluded() {
        return None;
    }
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
    if excluded() {
        return None;
    }
    if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() || !open(hwnd) {
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
    let w: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    if !open(hwnd) {
        return false;
    }
    let mut ok = false;
    if EmptyClipboard().is_ok() {
        if let Ok(hg) = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2) {
            let p = GlobalLock(hg) as *mut u16;
            if !p.is_null() {
                std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
                let _ = GlobalUnlock(hg);
                ok = SetClipboardData(CF_UNICODETEXT.0 as u32, HANDLE(hg.0)).is_ok();
            }
            if !ok {
                let _ = GlobalFree(hg);
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
}
