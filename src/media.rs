//! Фоновый поток WinRT: текущий трек (GSMTC), обложка, Wi-Fi/Bluetooth.
//! Ничего не опрашивает: просыпается только по событиям системы или по команде из UI.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};

use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows::Foundation::{EventRegistrationToken, TypedEventHandler};
use windows::Graphics::Imaging::*;
use windows::Media::Control::*;
use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

pub const WM_APP_MEDIA: u32 = WM_APP + 1;
pub const WM_APP_RADIO: u32 = WM_APP + 2;
const ART: u32 = 128;

pub struct Art {
    pub w: i32,
    pub h: i32,
    pub px: Vec<u8>, // BGRA, премультиплицированный
}

pub struct Media {
    pub has: bool,
    pub title: String,
    pub artist: String,
    pub app: String,
    pub playing: bool,
    pub art: Option<Arc<Art>>,
}

pub static MEDIA: Mutex<Media> = Mutex::new(Media {
    has: false,
    title: String::new(),
    artist: String::new(),
    app: String::new(),
    playing: false,
    art: None,
});

#[derive(Clone, Copy, Default)]
pub struct Radios {
    pub wifi: Option<bool>,
    pub bt: Option<bool>,
}

pub static RADIOS: Mutex<Radios> = Mutex::new(Radios { wifi: None, bt: None });

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Wifi,
    Bt,
}

#[derive(PartialEq)]
pub enum Job {
    Refresh,
    PlayPause,
    Next,
    Prev,
    Radios,
    SetRadio(Kind, bool),
}

static TX: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
static HWND_RAW: AtomicIsize = AtomicIsize::new(0);

pub fn send(job: Job) {
    if let Some(tx) = TX.get() {
        let _ = tx.lock().unwrap().send(job);
    }
}

fn notify(msg: u32) {
    let h = HWND(HWND_RAW.load(Ordering::Relaxed) as *mut _);
    unsafe {
        let _ = PostMessageW(h, msg, WPARAM(0), LPARAM(0));
    }
}

pub fn playing() -> bool {
    let m = MEDIA.lock().unwrap();
    m.has && m.playing
}

pub fn start(hwnd: HWND) {
    HWND_RAW.store(hwnd.0 as isize, Ordering::Relaxed);
    let (tx, rx) = mpsc::channel();
    let _ = TX.set(Mutex::new(tx));
    std::thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        worker(rx);
    });
}

struct Sub {
    session: GlobalSystemMediaTransportControlsSession,
    id: String,
    props: EventRegistrationToken,
    playback: EventRegistrationToken,
}

impl Drop for Sub {
    fn drop(&mut self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.props);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
    }
}

fn worker(rx: mpsc::Receiver<Job>) {
    let mgr = GlobalSystemMediaTransportControlsSessionManager::RequestAsync().and_then(|op| op.get()).ok();
    if let Some(m) = &mgr {
        let _ = m.CurrentSessionChanged(&TypedEventHandler::new(|_, _| {
            send(Job::Refresh);
            Ok(())
        }));
        let _ = m.SessionsChanged(&TypedEventHandler::new(|_, _| {
            send(Job::Refresh);
            Ok(())
        }));
    }
    let mut sub: Option<Sub> = None;
    let mut art_key = String::new();
    let mut radios: Vec<Radio> = Vec::new();

    send(Job::Refresh);
    while let Ok(first) = rx.recv() {
        // Схлопываем пачку одинаковых событий в одно.
        let mut jobs = vec![first];
        while let Ok(j) = rx.try_recv() {
            if !jobs.contains(&j) {
                jobs.push(j);
            }
        }
        for job in jobs {
            match job {
                Job::Refresh => {
                    if let Some(m) = &mgr {
                        refresh(m, &mut sub, &mut art_key);
                    }
                    notify(WM_APP_MEDIA);
                }
                Job::PlayPause | Job::Next | Job::Prev => {
                    if let Some(s) = &sub {
                        let op = match job {
                            Job::PlayPause => s.session.TryTogglePlayPauseAsync(),
                            Job::Next => s.session.TrySkipNextAsync(),
                            _ => s.session.TrySkipPreviousAsync(),
                        };
                        let _ = op.and_then(|o| o.get());
                    }
                }
                Job::Radios => {
                    read_radios(&mut radios);
                    notify(WM_APP_RADIO);
                }
                Job::SetRadio(kind, on) => {
                    if radios.is_empty() {
                        read_radios(&mut radios);
                    }
                    let want = if kind == Kind::Wifi { RadioKind::WiFi } else { RadioKind::Bluetooth };
                    let state = if on { RadioState::On } else { RadioState::Off };
                    for r in &radios {
                        if r.Kind().ok() == Some(want) {
                            let _ = r.SetStateAsync(state).and_then(|o| o.get());
                        }
                    }
                    read_radios(&mut radios);
                    notify(WM_APP_RADIO);
                }
            }
        }
    }
}

fn read_radios(radios: &mut Vec<Radio>) {
    if radios.is_empty() {
        let _ = Radio::RequestAccessAsync().and_then(|o| o.get());
        if let Ok(list) = Radio::GetRadiosAsync().and_then(|o| o.get()) {
            radios.extend(list);
        }
    }
    let mut out = Radios::default();
    for r in radios.iter() {
        let on = r.State().map(|s| s == RadioState::On).unwrap_or(false);
        match r.Kind() {
            Ok(RadioKind::WiFi) => out.wifi = Some(out.wifi.unwrap_or(false) || on),
            Ok(RadioKind::Bluetooth) => out.bt = Some(out.bt.unwrap_or(false) || on),
            _ => {}
        }
    }
    *RADIOS.lock().unwrap() = out;
}

fn refresh(mgr: &GlobalSystemMediaTransportControlsSessionManager, sub: &mut Option<Sub>, art_key: &mut String) {
    let session = mgr.GetCurrentSession().ok();
    let id = session.as_ref().and_then(|s| s.SourceAppUserModelId().ok()).map(|h| h.to_string());

    // Подписка на события текущей сессии (переподписываемся только при смене плеера).
    if sub.as_ref().map(|s| &s.id) != id.as_ref() {
        *sub = None;
        if let (Some(s), Some(id)) = (session.clone(), id.clone()) {
            let props = s.MediaPropertiesChanged(&TypedEventHandler::new(|_, _| {
                send(Job::Refresh);
                Ok(())
            }));
            let playback = s.PlaybackInfoChanged(&TypedEventHandler::new(|_, _| {
                send(Job::Refresh);
                Ok(())
            }));
            if let (Ok(props), Ok(playback)) = (props, playback) {
                *sub = Some(Sub { session: s, id, props, playback });
            }
        }
    }

    let Some(s) = sub.as_ref().map(|s| s.session.clone()) else {
        let mut m = MEDIA.lock().unwrap();
        m.has = false;
        m.art = None;
        art_key.clear();
        return;
    };
    let props = s.TryGetMediaPropertiesAsync().and_then(|o| o.get()).ok();
    let title = props.as_ref().and_then(|p| p.Title().ok()).map(|h| h.to_string()).unwrap_or_default();
    let artist = props.as_ref().and_then(|p| p.Artist().ok()).map(|h| h.to_string()).unwrap_or_default();
    let playing = s
        .GetPlaybackInfo()
        .and_then(|p| p.PlaybackStatus())
        .map(|st| st == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
        .unwrap_or(false);
    let app = pretty_app(id.as_deref().unwrap_or(""));

    let key = format!("{}\u{1}{}\u{1}{}", app, title, artist);
    let need_art = key != *art_key || MEDIA.lock().unwrap().art.is_none();
    let art = if need_art { props.as_ref().and_then(load_art).map(Arc::new) } else { None };

    let mut m = MEDIA.lock().unwrap();
    m.has = true;
    m.title = title;
    m.artist = artist;
    m.app = app;
    m.playing = playing;
    if need_art {
        m.art = art;
        *art_key = key;
    }
}

/// «Spotify.exe» -> «Spotify», «Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic» -> «ZuneMusic».
fn pretty_app(id: &str) -> String {
    let s = id.split('!').next().unwrap_or(id);
    let s = s.split('_').next().unwrap_or(s);
    let s = s.strip_suffix(".exe").or_else(|| s.strip_suffix(".EXE")).unwrap_or(s);
    let s = s.rsplit('.').next().unwrap_or(s);
    s.to_string()
}

fn load_art(props: &GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<Art> {
    let stream = props.Thumbnail().ok()?.OpenReadAsync().ok()?.get().ok()?;
    let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let (w, h) = (decoder.PixelWidth().ok()?, decoder.PixelHeight().ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    // Обрезаем до квадрата по центру и ужимаем до ART x ART.
    let side = w.min(h);
    let k = ART as f64 / side as f64;
    let (sw, sh) = (((w as f64) * k).round() as u32, ((h as f64) * k).round() as u32);
    let t = BitmapTransform::new().ok()?;
    t.SetScaledWidth(sw.max(ART)).ok()?;
    t.SetScaledHeight(sh.max(ART)).ok()?;
    t.SetInterpolationMode(BitmapInterpolationMode::Fant).ok()?;
    t.SetBounds(BitmapBounds {
        X: (sw.max(ART) - ART) / 2,
        Y: (sh.max(ART) - ART) / 2,
        Width: ART,
        Height: ART,
    })
    .ok()?;
    let data = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Bgra8,
            BitmapAlphaMode::Premultiplied,
            &t,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .ok()?
        .get()
        .ok()?
        .DetachPixelData()
        .ok()?;
    if data.len() < (ART * ART * 4) as usize {
        return None;
    }
    Some(Art { w: ART as i32, h: ART as i32, px: data.to_vec() })
}
