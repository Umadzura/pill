//! Иконка трея, нарисованная в коде (ресурсов в exe нет).

use windows::Win32::Foundation::TRUE;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// Маленькая белая пилюля со сглаживанием прямо в пикселях.
pub unsafe fn make() -> HICON {
    let size = GetSystemMetrics(SM_CXSMICON).max(16);
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: size,
            biHeight: -size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let Ok(color) = CreateDIBSection(None, &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
        return LoadIconW(None, IDI_APPLICATION).unwrap_or_default();
    };
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
    let s = size as f32;
    let r = s * 0.25;
    let (cy, x0, x1) = (s / 2.0, s * 0.12 + r, s * 0.88 - r);
    for y in 0..size {
        for x in 0..size {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let dx = if fx < x0 { x0 - fx } else if fx > x1 { fx - x1 } else { 0.0 };
            let d = (dx * dx + (fy - cy) * (fy - cy)).sqrt() - r;
            let a = (0.5 - d).clamp(0.0, 1.0);
            let a8 = (a * 255.0) as u32;
            // Премультиплицированный белый.
            px[(y * size + x) as usize] = a8 << 24 | a8 << 16 | a8 << 8 | a8;
        }
    }
    let mask = CreateBitmap(size, size, 1, 1, None);
    let info = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let icon = CreateIconIndirect(&info).unwrap_or_default();
    let _ = DeleteObject(mask);
    let _ = DeleteObject(color);
    icon
}
