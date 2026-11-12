//! Функции Pill. Каждая: свои данные (`XxxState`), свои зоны клика (`XxxHit`) и `on_hit`,
//! который возвращает, что перерисовать. GDI здесь нет (рисует `ui/`), Windows — через `sys/`.
//! Внутри `st()` только данные, Win32 снаружи (правило из ARCHITECTURE.md, раздел 4).

pub mod clip;
pub mod cmd;
pub mod ctl;
pub mod music;
pub mod osd;
pub mod timers;
pub mod viz;
