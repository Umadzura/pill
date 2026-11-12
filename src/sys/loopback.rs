//! Захват того, что играет на устройстве вывода по умолчанию (WASAPI loopback), в моно-сэмплы.
//! Ничего не пишет на диск и никуда не отправляет: только уровни для визуализатора.

use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

#[derive(Clone, Copy)]
enum Fmt {
    F32,
    I16,
}

pub struct Loopback {
    client: IAudioClient,
    capture: IAudioCaptureClient,
    channels: usize,
    fmt: Fmt,
    pub rate: u32,
}

/// Формат смешивания: float32 или PCM16. Другие (24 бита и т. п.) не поддерживаем — визуализатор просто молчит.
unsafe fn format(wf: *const WAVEFORMATEX) -> Option<(Fmt, usize, u32)> {
    let w = &*wf;
    const PCM: u16 = 1;
    const FLOAT: u16 = 3;
    const EXTENSIBLE: u16 = 0xFFFE;
    let tag = if w.wFormatTag == EXTENSIBLE {
        // SubFormat — GUID, у которого первые 4 байта совпадают с обычным тегом формата.
        let ext = &*(wf as *const WAVEFORMATEXTENSIBLE);
        let sub = std::ptr::read_unaligned(std::ptr::addr_of!(ext.SubFormat));
        sub.data1 as u16
    } else {
        w.wFormatTag
    };
    let fmt = match (tag, w.wBitsPerSample) {
        (FLOAT, 32) => Fmt::F32,
        (PCM, 16) => Fmt::I16,
        _ => return None,
    };
    Some((fmt, w.nChannels.max(1) as usize, w.nSamplesPerSec))
}

impl Loopback {
    /// Зовётся из потока с инициализированным COM (MTA).
    pub fn open() -> Option<Loopback> {
        unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).ok()?;
            let wf = client.GetMixFormat().ok()?;
            let parsed = format(wf);
            // Буфер 200 мс: читаем ~30 раз в секунду, запас на подвисания.
            let init = client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000, 0, wf, None);
            CoTaskMemFree(Some(wf as *const _));
            let (fmt, channels, rate) = parsed?;
            init.ok()?;
            let capture: IAudioCaptureClient = client.GetService().ok()?;
            client.Start().ok()?;
            Some(Loopback { client, capture, channels, fmt, rate })
        }
    }

    /// Забрать всё накопленное, отдавая моно-сэмплы в `f`. Err — устройство пропало (переоткрыть).
    pub fn read(&mut self, mut f: impl FnMut(f32)) -> Result<usize, ()> {
        let mut total = 0;
        unsafe {
            loop {
                let n = self.capture.GetNextPacketSize().map_err(|_| ())?;
                if n == 0 {
                    break;
                }
                let mut data: *mut u8 = std::ptr::null_mut();
                let mut frames = 0u32;
                let mut flags = 0u32;
                self.capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None).map_err(|_| ())?;
                let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0;
                let ch = self.channels;
                for i in 0..frames as usize {
                    let mut sum = 0.0f32;
                    if !silent && !data.is_null() {
                        for c in 0..ch {
                            sum += match self.fmt {
                                Fmt::F32 => *(data as *const f32).add(i * ch + c),
                                Fmt::I16 => *(data as *const i16).add(i * ch + c) as f32 / 32768.0,
                            };
                        }
                    }
                    f(sum / ch as f32);
                }
                total += frames as usize;
                let _ = self.capture.ReleaseBuffer(frames);
            }
        }
        Ok(total)
    }
}

impl Drop for Loopback {
    fn drop(&mut self) {
        unsafe {
            let _ = self.client.Stop();
        }
    }
}
