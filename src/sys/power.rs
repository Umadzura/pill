//! Питание: батарея и её события, блокировка, сон, перезагрузка, выключение.

use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, RegisterPowerSettingNotification, SetSuspendState, POWERBROADCAST_SETTING, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::Shutdown::LockWorkStation;
use windows::Win32::System::SystemServices::{GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING};
use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE;

use super::display;
use super::launch::hidden;

#[derive(Clone, Copy, PartialEq, Debug)]
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

/// Что поменялось по WM_POWERBROADCAST / PBT_POWERSETTINGCHANGE.
pub enum PowerEvent {
    /// Зарядку подключили/отключили или изменился процент: перечитать `battery()`.
    Battery,
    /// Яркость встроенного экрана, 0..100.
    Brightness(u8),
}

/// Подписка окна на события питания вместо опроса. Сразу после подписки Windows
/// присылает по одному сообщению с текущими значениями. На ПК без батареи ничего не приходит.
pub fn register_notify(hwnd: HWND) {
    unsafe {
        for g in [GUID_ACDC_POWER_SOURCE, GUID_BATTERY_PERCENTAGE_REMAINING] {
            let _ = RegisterPowerSettingNotification(hwnd, &g, DEVICE_NOTIFY_WINDOW_HANDLE);
        }
    }
    display::register_notify(hwnd);
}

/// Разобрать lparam от PBT_POWERSETTINGCHANGE.
pub unsafe fn parse_notify(lparam: LPARAM) -> Option<PowerEvent> {
    if let Some(v) = display::parse_notify(lparam) {
        return Some(PowerEvent::Brightness(v));
    }
    let s = (lparam.0 as *const POWERBROADCAST_SETTING).as_ref()?;
    (s.PowerSetting == GUID_ACDC_POWER_SOURCE || s.PowerSetting == GUID_BATTERY_PERCENTAGE_REMAINING).then_some(PowerEvent::Battery)
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
