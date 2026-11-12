//! Фоновые потоки. Каждый владеет своими объектами Windows, результат кладёт в свой `static Mutex`
//! и будит UI через `PostMessageW` (никогда `SendMessage`). `State` не трогают.
//!
//! Потоков не больше четырёх: UI, `winrt`, `watch`, `api`.

/// Локальный API для скриптов: слушает 127.0.0.1, в простое спит в accept.
pub mod api;
/// WinRT-воркер (MTA): музыка (GSMTC), обложки, Wi-Fi/Bluetooth.
pub mod winrt;
/// Наблюдатель: ждёт ядерные события (реестр микрофона/камеры) на одном `WaitForMultipleObjects`.
pub mod watch;
