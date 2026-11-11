//! События громкости без опроса: система сама зовёт колбэк при смене громкости
//! или устройства вывода, а колбэк только шлёт сообщение окну.

use windows::core::{implement, Result, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::Endpoints::*;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// wparam = громкость × 1000, lparam = 1, если звук выключен.
pub const WM_APP_VOLUME: u32 = WM_APP + 5;
/// Сменилось устройство вывода по умолчанию (наушники, колонки).
pub const WM_APP_AUDIODEV: u32 = WM_APP + 6;

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
