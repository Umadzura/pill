//! Яркость: встроенный экран ноутбука (WMI, `ROOT\WMI`) и внешние мониторы ПК по DDC/CI (dxva2).
//! Всё здесь медленное (WMI — десятки мс, DDC — 40–100 мс на монитор), поэтому зовётся
//! только из фонового воркера. События смены яркости ноутбука приходят отдельно,
//! через `RegisterPowerSettingNotification` (см. `register_notify`), без WMI-подписки.

use windows::core::{w, BSTR, VARIANT};
use windows::Win32::Devices::Display::*;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};
use windows::Win32::System::Com::*;
use windows::Win32::System::Power::{RegisterPowerSettingNotification, POWERBROADCAST_SETTING};
use windows::Win32::System::Rpc::{RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE};
use windows::Win32::System::SystemServices::GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS;
use windows::Win32::System::Wmi::*;
use windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE;

/// Яркость всех экранов, которыми умеем управлять. Живёт в фоновом потоке (COM-объекты).
pub struct Displays {
    /// `None` — WMI-подключение не удалось или встроенного экрана нет (ПК).
    wmi: Option<IWbemServices>,
    /// Пробовали ли уже подключаться к WMI (не повторяем неудачу при каждом раскрытии).
    wmi_tried: bool,
}

impl Displays {
    pub const fn new() -> Self {
        Displays { wmi: None, wmi_tried: false }
    }

    fn wmi(&mut self) -> Option<&IWbemServices> {
        if !self.wmi_tried {
            self.wmi_tried = true;
            // Встроенный экран бывает только у ноутбука, а у ноутбука есть батарея. На ПК WMI
            // не трогаем: его DLL добавляют ~3 МБ памяти, а яркость там идёт по DDC/CI.
            if super::power::battery().is_none() {
                return None;
            }
            self.wmi = unsafe { connect_wmi() };
            // Подключились, но встроенного экрана нет: не держим WMI зря.
            if self.wmi.as_ref().map(|s| unsafe { laptop_get(s) }.is_none()).unwrap_or(false) {
                self.wmi = None;
            }
        }
        self.wmi.as_ref()
    }

    /// Текущая яркость 0..100: встроенный экран, иначе первый монитор с DDC/CI. None — управлять нечем.
    pub fn get(&mut self) -> Option<u8> {
        if let Some(s) = self.wmi() {
            if let Some(v) = unsafe { laptop_get(s) } {
                return Some(v);
            }
        }
        ddc_get()
    }

    /// Поставить яркость 0..100 на встроенный экран и на все мониторы с DDC/CI.
    pub fn set(&mut self, pct: u8) {
        let pct = pct.min(100);
        if let Some(s) = self.wmi() {
            unsafe { laptop_set(s, pct) };
        }
        ddc_set(pct);
    }
}

// ---------- ноутбук: WMI ----------

unsafe fn connect_wmi() -> Option<IWbemServices> {
    let loc: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER).ok()?;
    let svc = loc
        .ConnectServer(&BSTR::from("ROOT\\WMI"), &BSTR::new(), &BSTR::new(), &BSTR::new(), 0, &BSTR::new(), None::<&IWbemContext>)
        .ok()?;
    CoSetProxyBlanket(&svc, RPC_C_AUTHN_WINNT, RPC_C_AUTHZ_NONE, None, RPC_C_AUTHN_LEVEL_CALL, RPC_C_IMP_LEVEL_IMPERSONATE, None, EOAC_NONE).ok()?;
    Some(svc)
}

/// Первый объект из WQL-запроса.
unsafe fn first(svc: &IWbemServices, query: &str) -> Option<IWbemClassObject> {
    let en = svc
        .ExecQuery(&BSTR::from("WQL"), &BSTR::from(query), WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY, None::<&IWbemContext>)
        .ok()?;
    let mut objs = [None];
    let mut n = 0u32;
    let _ = en.Next(WBEM_INFINITE, &mut objs, &mut n);
    if n == 0 {
        return None;
    }
    objs[0].take()
}

unsafe fn laptop_get(svc: &IWbemServices) -> Option<u8> {
    let obj = first(svc, "SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE")?;
    let mut v = VARIANT::default();
    obj.Get(w!("CurrentBrightness"), 0, &mut v, None, None).ok()?;
    u32::try_from(&v).ok().map(|x| x.min(100) as u8)
}

unsafe fn laptop_set(svc: &IWbemServices, pct: u8) -> Option<()> {
    let inst = first(svc, "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active=TRUE")?;
    let mut path = VARIANT::default();
    inst.Get(w!("__PATH"), 0, &mut path, None, None).ok()?;
    let path = BSTR::try_from(&path).ok()?;

    let mut class = None;
    svc.GetObject(&BSTR::from("WmiMonitorBrightnessMethods"), WBEM_GENERIC_FLAG_TYPE(0), None::<&IWbemContext>, Some(&mut class), None)
        .ok()?;
    let mut in_sig = None;
    let mut out_sig = None;
    class?.GetMethod(w!("WmiSetBrightness"), 0, &mut in_sig, &mut out_sig).ok()?;
    let args = in_sig?.SpawnInstance(0).ok()?;
    // Типы аргументов uint32/uint8, но WMI сам приводит VT_I4 — так делают и примеры Microsoft.
    args.Put(w!("Timeout"), 0, &VARIANT::from(0i32), 0).ok()?;
    args.Put(w!("Brightness"), 0, &VARIANT::from(pct as i32), 0).ok()?;
    svc.ExecMethod(&path, &BSTR::from("WmiSetBrightness"), WBEM_GENERIC_FLAG_TYPE(0), None::<&IWbemContext>, &args, None, None)
        .ok()
}

// ---------- монитор ПК: DDC/CI ----------

unsafe extern "system" fn collect(mon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
    let list = &mut *(data.0 as *mut Vec<HMONITOR>);
    list.push(mon);
    TRUE
}

/// Пройти по всем физическим мониторам: f(handle, min, cur, max) для тех, кто ответил по DDC/CI.
fn each_ddc(mut f: impl FnMut(windows::Win32::Foundation::HANDLE, u32, u32, u32)) {
    unsafe {
        let mut mons: Vec<HMONITOR> = Vec::new();
        let _ = EnumDisplayMonitors(None, None, Some(collect), LPARAM(&mut mons as *mut _ as isize));
        for m in mons {
            let mut n = 0u32;
            if GetNumberOfPhysicalMonitorsFromHMONITOR(m, &mut n).is_err() || n == 0 {
                continue;
            }
            let mut phys = vec![PHYSICAL_MONITOR::default(); n as usize];
            if GetPhysicalMonitorsFromHMONITOR(m, &mut phys).is_err() {
                continue;
            }
            for p in &phys {
                let (mut min, mut cur, mut max) = (0u32, 0u32, 0u32);
                if GetMonitorBrightness(p.hPhysicalMonitor, &mut min, &mut cur, &mut max) != 0 && max > min {
                    f(p.hPhysicalMonitor, min, cur, max);
                }
            }
            let _ = DestroyPhysicalMonitors(&phys);
        }
    }
}

fn ddc_get() -> Option<u8> {
    let mut out = None;
    each_ddc(|_, min, cur, max| {
        if out.is_none() {
            out = Some(to_pct(min, cur, max));
        }
    });
    out
}

fn ddc_set(pct: u8) {
    each_ddc(|h, min, _, max| unsafe {
        let _ = SetMonitorBrightness(h, from_pct(min, max, pct));
    });
}

/// Значение монитора (min..max) в проценты.
fn to_pct(min: u32, cur: u32, max: u32) -> u8 {
    let span = (max - min).max(1);
    ((cur.clamp(min, max) - min) * 100 / span) as u8
}

/// Проценты в значение монитора (min..max), с округлением.
fn from_pct(min: u32, max: u32, pct: u8) -> u32 {
    min + ((max - min) * pct.min(100) as u32 + 50) / 100
}

// ---------- события яркости ноутбука ----------

/// Подписка окна на смену яркости встроенного экрана: Windows шлёт WM_POWERBROADCAST /
/// PBT_POWERSETTINGCHANGE (и сразу одно сообщение с текущим значением). На ПК не приходит.
pub fn register_notify(hwnd: HWND) {
    unsafe {
        let _ = RegisterPowerSettingNotification(hwnd, &GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS, DEVICE_NOTIFY_WINDOW_HANDLE);
    }
}

/// Разобрать lparam от PBT_POWERSETTINGCHANGE. Some(процент), если это наша настройка.
pub unsafe fn parse_notify(lparam: LPARAM) -> Option<u8> {
    let s = (lparam.0 as *const POWERBROADCAST_SETTING).as_ref()?;
    if s.PowerSetting != GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS || s.DataLength < 4 {
        return None;
    }
    let v = std::ptr::read_unaligned(s.Data.as_ptr() as *const u32);
    Some(v.min(100) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_roundtrip() {
        assert_eq!(to_pct(0, 50, 100), 50);
        assert_eq!(to_pct(0, 0, 100), 0);
        assert_eq!(to_pct(20, 120, 220), 50);
        assert_eq!(from_pct(0, 100, 37), 37);
        assert_eq!(from_pct(20, 220, 50), 120);
        // Монитор со шкалой 0..255.
        assert_eq!(to_pct(0, from_pct(0, 255, 40), 255), 40);
    }
}
