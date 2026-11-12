//! Питание: батарея, блокировка, сон, перезагрузка, выключение.

use windows::Win32::System::Power::{GetSystemPowerStatus, SetSuspendState, SYSTEM_POWER_STATUS};
use windows::Win32::System::Shutdown::LockWorkStation;

use super::launch::hidden;

pub struct Battery {
    pub percent: u8,
    pub ac: bool,
    /// Сколько секунд осталось от батареи, если Windows знает.
    pub left_secs: Option<u32>,
}

/// None, если батареи нет (стационарный ПК).
pub fn battery() -> Option<Battery> {
    let mut ps = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut ps).ok()? };
    // 128 = батареи нет, 255 = неизвестно.
    if ps.BatteryFlag & 128 != 0 || ps.BatteryLifePercent > 100 {
        return None;
    }
    Some(Battery {
        percent: ps.BatteryLifePercent,
        ac: ps.ACLineStatus == 1,
        left_secs: (ps.BatteryLifeTime != u32::MAX).then_some(ps.BatteryLifeTime),
    })
}

pub fn lock() {
    unsafe {
        let _ = LockWorkStation();
    }
}
pub fn sleep() {
    unsafe {
        let _ = SetSuspendState(false, false, false);
    }
}
pub fn restart() {
    hidden("shutdown", &["/r", "/t", "0"]);
}
pub fn shutdown() {
    hidden("shutdown", &["/s", "/t", "0"]);
}
