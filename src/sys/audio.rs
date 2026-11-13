//! Громкость устройства вывода: чтение, установка и события без опроса.
//! Система сама зовёт колбэк при смене громкости или устройства, а колбэк только шлёт сообщение окну.

use windows::core::{implement, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::Win32::UI::WindowsAndMessaging::PostMessageW;

use crate::ids::{WM_APP_AUDIODEV, WM_APP_VOLUME};

unsafe fn endpoint() -> Option<IAudioEndpointVolume> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
    let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
    dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
}

/// (уровень 0..1, без звука)
pub fn volume() -> Option<(f32, bool)> {
    unsafe {
        let v = endpoint()?;
        let level = v.GetMasterVolumeLevelScalar().ok()?;
        let muted = v.GetMute().map(|m| m.as_bool()).unwrap_or(false);
        Some((level, muted))
    }
}

pub fn set_volume(level: f32) {
    unsafe {
        if let Some(v) = endpoint() {
            let _ = v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
            if level > 0.0 {
                let _ = v.SetMute(false, std::ptr::null());
            }
        }
    }
}

pub fn toggle_mute() {
    unsafe {
        if let Some(v) = endpoint() {
            let m = v.GetMute().map(|m| m.as_bool()).unwrap_or(false);
            let _ = v.SetMute(!m, std::ptr::null());
        }
    }
}

/// PKEY_Device_FriendlyName: «Наушники (WH-1000XM5)».
const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 14 };

/// Имя устройства вывода по умолчанию.
pub fn output_name() -> Option<String> {
    unsafe {
        let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        let store = dev.OpenPropertyStore(STGM_READ).ok()?;
        let name = store.GetValue(&FRIENDLY_NAME).ok()?.to_string();
        (!name.is_empty()).then_some(name)
    }
}

unsafe fn mic() -> Option<IAudioEndpointVolume> {
    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
    let dev = en.GetDefaultAudioEndpoint(eCapture, eConsole).ok()?;
    dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
}

/// Микрофон по умолчанию выключен? None — микрофона нет.
pub fn mic_muted() -> Option<bool> {
    unsafe { mic()?.GetMute().ok().map(|m| m.as_bool()) }
}

pub fn toggle_mic() {
    unsafe {
        if let Some(v) = mic() {
            let m = v.GetMute().map(|m| m.as_bool()).unwrap_or(false);
            let _ = v.SetMute(!m, std::ptr::null());
        }
    }
}

// ---------- события ----------

fn post(hwnd: isize, msg: u32, w: usize, l: isize) {
    unsafe {
        let _ = PostMessageW(HWND(hwnd as *mut _), msg, WPARAM(w), LPARAM(l));
    }
}

#[implement(IAudioEndpointVolumeCallback)]
struct VolumeCb(isize);

impl IAudioEndpointVolumeCallback_Impl for VolumeCb_Impl {
    fn OnNotify(&self, data: *mut AUDIO_VOLUME_NOTIFICATION_DATA) -> Result<()> {
        if let Some(d) = unsafe { data.as_ref() } {
            let level = (d.fMasterVolume.clamp(0.0, 1.0) * 1000.0).round() as usize;
            post(self.0, WM_APP_VOLUME, level, d.bMuted.as_bool() as isize);
        }
        Ok(())
    }
}

#[implement(IMMNotificationClient)]
struct DeviceCb(isize);

impl IMMNotificationClient_Impl for DeviceCb_Impl {
    fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> Result<()> {
        Ok(())
    }
    fn OnDeviceAdded(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDeviceRemoved(&self, _: &PCWSTR) -> Result<()> {
        Ok(())
    }
    fn OnDefaultDeviceChanged(&self, flow: EDataFlow, role: ERole, _: &PCWSTR) -> Result<()> {
        if flow == eRender && role == eConsole {
            post(self.0, WM_APP_AUDIODEV, 0, 0);
        }
        Ok(())
    }
    fn OnPropertyValueChanged(&self, _: &PCWSTR, _: &PROPERTYKEY) -> Result<()> {
        Ok(())
    }
}

/// Подписка на громкость текущего устройства вывода. При смене устройства зовём rebind().
pub struct Watch {
    en: IMMDeviceEnumerator,
    dev_cb: IMMNotificationClient,
    vol_cb: IAudioEndpointVolumeCallback,
    ep: Option<IAudioEndpointVolume>,
}

impl Watch {
    pub fn start(hwnd: HWND) -> Option<Watch> {
        unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let raw = hwnd.0 as isize;
            let dev_cb: IMMNotificationClient = DeviceCb(raw).into();
            en.RegisterEndpointNotificationCallback(&dev_cb).ok()?;
            let mut w = Watch { en, dev_cb, vol_cb: VolumeCb(raw).into(), ep: None };
            w.rebind();
            Some(w)
        }
    }

    pub fn rebind(&mut self) {
        unsafe {
            if let Some(ep) = self.ep.take() {
                let _ = ep.UnregisterControlChangeNotify(&self.vol_cb);
            }
            let ep = self
                .en
                .GetDefaultAudioEndpoint(eRender, eConsole)
                .and_then(|d| d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None));
            if let Ok(ep) = ep {
                if ep.RegisterControlChangeNotify(&self.vol_cb).is_ok() {
                    self.ep = Some(ep);
                }
            }
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        unsafe {
            if let Some(ep) = self.ep.take() {
                let _ = ep.UnregisterControlChangeNotify(&self.vol_cb);
            }
            let _ = self.en.UnregisterEndpointNotificationCallback(&self.dev_cb);
        }
    }
}
