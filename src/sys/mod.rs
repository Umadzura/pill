//! Тонкие обёртки над Windows. Ничего не знают про `State` и друг про друга (кроме `launch::hidden`).
//! Ошибки не пробрасываем: функция возвращает `Option`/`bool`, а UI прячет элемент или пишет «—».

pub mod audio;
pub mod autostart;
pub mod clipboard;
pub mod display;
pub mod icon;
pub mod launch;
pub mod loopback;
pub mod net;
pub mod power;
pub mod sysinfo;
pub mod theme;
pub mod toasts;

use windows::Win32::System::SystemInformation::GetLocalTime;

/// Часы и минуты по местному времени.
pub fn local_hm() -> (u16, u16) {
    let t = unsafe { GetLocalTime() };
    (t.wHour, t.wMinute)
}
