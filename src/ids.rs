//! Все «магические» номера в одном месте: сообщения окна, таймеры, хоткеи, пункты меню.
//! Добавил новый, впиши ещё и в ARCHITECTURE.md, раздел 5.

use windows::Win32::UI::WindowsAndMessaging::WM_APP;

// ---------- сообщения окна (фоновые потоки и система → UI) ----------

/// WinRT-воркер: трек или статус плеера изменились.
pub const WM_APP_MEDIA: u32 = WM_APP + 1;
/// WinRT-воркер: прочитаны Wi-Fi и Bluetooth.
pub const WM_APP_RADIO: u32 = WM_APP + 2;
/// Иконка в трее.
pub const WM_APP_TRAY: u32 = WM_APP + 3;
/// Explorer: `ABN_FULLSCREENAPP`.
pub const WM_APP_BAR: u32 = WM_APP + 4;
/// COM-колбэк громкости. wparam = громкость × 1000, lparam = 1, если звук выключен.
pub const WM_APP_VOLUME: u32 = WM_APP + 5;
/// Сменилось устройство вывода по умолчанию (наушники, колонки).
pub const WM_APP_AUDIODEV: u32 = WM_APP + 6;
/// Наблюдатель: поменялось, кто держит микрофон или камеру.
pub const WM_APP_PRIVACY: u32 = WM_APP + 7;
/// Воркер: прочитана яркость экрана (`bg::winrt::BRIGHTNESS`).
pub const WM_APP_BRIGHT: u32 = WM_APP + 8;
/// Наблюдатель: новый кадр спектра (`feat::viz::SPECTRUM`). Не шлётся, пока прошлый не нарисован.
pub const WM_APP_VIZ: u32 = WM_APP + 9;

/// Нет в крейте `windows` (WM_CLIPBOARDUPDATE там есть).
pub const WM_MOUSELEAVE: u32 = 0x02A3;

// ---------- таймеры (id в wparam у WM_TIMER) ----------

/// 1 с, всегда. Единственный постоянный таймер.
pub const TIMER_CLOCK: usize = 1;
/// 350 мс, разовый: свернуться после ухода курсора.
pub const TIMER_COLLAPSE: usize = 2;
/// 80 мс, разовый: Ctrl+V после того, как фокус вернулся в целевое окно.
pub const TIMER_PASTE: usize = 3;
/// 3 с, разовый: перечитать VPN после rasdial.
pub const TIMER_CTL: usize = 4;
/// Разовый: убрать временную надпись (громкость, зарядка, предупреждения).
pub const TIMER_FLASH: usize = 5;
/// 250 мс, разовый: открыть выделение области, когда панель уже свернулась.
pub const TIMER_SHOT: usize = 6;

pub const COLLAPSE_DELAY_MS: u32 = 350;

// ---------- хоткеи и меню ----------

/// Alt+Space (запасной Ctrl+Alt+Space).
pub const HOTKEY_CMD: i32 = 1;
/// Ctrl+Alt+V.
pub const HOTKEY_CLIP: i32 = 2;

pub const MENU_AUTOSTART: usize = 10;
pub const MENU_EXIT: usize = 11;
