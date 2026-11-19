//! Тонкие обёртки над Windows. Ничего не знают про `State` и друг про друга (кроме `launch::hidden`).
//! Ошибки не пробрасываем: функция возвращает `Option`/`bool`, а UI прячет элемент или пишет «—».

pub mod audio;
pub mod autostart;
pub mod banners;
pub mod clipboard;
pub mod display;
pub mod dragdrop;
pub mod icon;
pub mod launch;
pub mod loopback;
pub mod net;
pub mod power;
pub mod sysinfo;
pub mod theme;
pub mod toasts;
pub mod wnd;

use windows::Win32::System::SystemInformation::GetLocalTime;

/// Часы и минуты по местному времени.
pub fn local_hm() -> (u16, u16) {
    let t = unsafe { GetLocalTime() };
    (t.wHour, t.wMinute)
}

/// `n` случайных байт из системного генератора (BCryptGenRandom) строкой hex. Не вышло — None.
pub fn random_hex(n: usize) -> Option<String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_ALG_HANDLE, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut buf = vec![0u8; n];
    let st = unsafe { BCryptGenRandom(BCRYPT_ALG_HANDLE::default(), &mut buf, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    st.is_ok().then(|| buf.iter().map(|b| format!("{:02x}", b)).collect())
}
