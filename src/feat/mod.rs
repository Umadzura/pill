//! Функции Pill. Каждая: свои данные (`XxxState`), свои зоны клика (`XxxHit`) и `on_hit`,
//! который возвращает, что перерисовать. GDI здесь нет (рисует `ui/`), Windows — через `sys/`.
//! Внутри `st()` только данные, Win32 снаружи (правило из ARCHITECTURE.md, раздел 4).

pub mod agents;
pub mod api;
pub mod banners;
pub mod clip;
pub mod cmd;
pub mod hooks;
pub mod ctl;
pub mod json;
pub mod music;
pub mod notifs;
pub mod osd;
pub mod peek;
pub mod settings;
pub mod shelf;
pub mod snap;
pub mod sysmon;
pub mod timers;
pub mod update;
pub mod viz;
