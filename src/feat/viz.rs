//! Визуализатор музыки: спектр того, что играет, полосками.
//!
//! Осознанное исключение из правила «0% CPU»: пока играет музыка и пилюля видна, поток-наблюдатель
//! (`bg::watch`) ~30 раз в секунду забирает звук (WASAPI loopback), считает FFT и будит UI.
//! На паузе, в тишине без плеера и в полноэкранном режиме захват полностью выключен.
//! Здесь чистая математика (с тестами) и выключатель; захват — `sys::loopback`, рисование — `ui::viz`.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;

use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Threading::SetEvent;

pub const BANDS: usize = 24;
const FFT_N: usize = 1024;
/// Диапазон полос: ниже 40 Гц почти нечего показывать, выше 16 кГц почти ничего не слышно.
const F_LO: f32 = 40.0;
const F_HI: f32 = 16_000.0;
/// Уровни в дБ, которые растягиваем на высоту полоски.
const DB_FLOOR: f32 = -62.0;
const DB_CEIL: f32 = -8.0;
/// Подъём к высоким полосам (дБ на весь диапазон): в музыке энергия падает с частотой,
/// без наклона правые полоски почти не двигаются. Так же делают анализаторы в плеерах.
const TILT_DB: f32 = 18.0;
/// Сколько от прошлого кадра остаётся при спаде (подъём мгновенный): полоски «падают» плавно.
const DECAY: f32 = 0.78;

/// Текущие уровни полос 0..1. Пишет поток-наблюдатель, читает отрисовка.
pub static SPECTRUM: Mutex<[f32; BANDS]> = Mutex::new([0.0; BANDS]);

/// Нужен ли захват сейчас (решает UI-поток).
static WANT: AtomicBool = AtomicBool::new(false);
/// Событие, которым будим наблюдателя при смене WANT (создаёт он сам).
static WAKE: AtomicIsize = AtomicIsize::new(0);
/// Кадр отправлен в UI, но ещё не нарисован: новых не шлём, чтобы не забить очередь сообщений.
pub static PENDING: AtomicBool = AtomicBool::new(false);

pub fn want() -> bool {
    WANT.load(Ordering::Relaxed)
}

/// Наблюдатель сообщает событие, на котором спит.
pub fn set_wake(ev: HANDLE) {
    WAKE.store(ev.0 as isize, Ordering::Relaxed);
}

/// Включить/выключить захват. Будит наблюдателя только при реальной смене.
pub fn set_want(on: bool) {
    if WANT.swap(on, Ordering::Relaxed) != on {
        let ev = WAKE.load(Ordering::Relaxed);
        if ev != 0 {
            unsafe {
                let _ = SetEvent(HANDLE(ev as *mut _));
            }
        }
    }
}

/// Пересчитать, нужен ли захват: музыка играет и пилюлю видно.
pub fn refresh_want() {
    let visible = crate::app::st(|s| s.shell.visible());
    set_want(visible && crate::bg::winrt::playing());
}

/// Обнулить полосы (захват остановлен).
pub fn clear() {
    *SPECTRUM.lock().unwrap() = [0.0; BANDS];
}

/// Сводит полосы к `n` столбикам (для свёрнутой пилюли), беря максимум в группе.
pub fn group(levels: &[f32; BANDS], n: usize) -> Vec<f32> {
    let n = n.clamp(1, BANDS);
    (0..n)
        .map(|i| {
            let (a, b) = (i * BANDS / n, ((i + 1) * BANDS / n).max(i * BANDS / n + 1));
            levels[a..b].iter().copied().fold(0.0, f32::max)
        })
        .collect()
}

/// Считает спектр из потока сэмплов. Живёт в потоке-наблюдателе.
pub struct Analyzer {
    ring: Vec<f32>,
    pos: usize,
    window: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    /// Границы полос в номерах бинов FFT (BANDS + 1 штук).
    edges: Vec<usize>,
    smooth: [f32; BANDS],
}

impl Analyzer {
    pub fn new(rate: u32) -> Self {
        let window = (0..FFT_N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (FFT_N - 1) as f32).cos()).collect();
        Analyzer {
            ring: vec![0.0; FFT_N],
            pos: 0,
            window,
            re: vec![0.0; FFT_N],
            im: vec![0.0; FFT_N],
            edges: band_edges(rate as f32, FFT_N),
            smooth: [0.0; BANDS],
        }
    }

    /// Добавить моно-сэмпл (-1..1).
    pub fn push(&mut self, v: f32) {
        self.ring[self.pos] = v;
        self.pos = (self.pos + 1) % FFT_N;
    }

    /// Посчитать кадр по последним FFT_N сэмплам (со сглаживанием спада).
    pub fn frame(&mut self) -> [f32; BANDS] {
        for i in 0..FFT_N {
            self.re[i] = self.ring[(self.pos + i) % FFT_N] * self.window[i];
            self.im[i] = 0.0;
        }
        fft(&mut self.re, &mut self.im);
        // Окно Ханна вдвое уменьшает амплитуду: синус с пиком 1 даёт |X| ≈ N/4.
        let norm = 4.0 / FFT_N as f32;
        for b in 0..BANDS {
            let (lo, hi) = (self.edges[b], self.edges[b + 1]);
            let mut peak = 0.0f32;
            for k in lo..hi {
                peak = peak.max((self.re[k] * self.re[k] + self.im[k] * self.im[k]).sqrt());
            }
            let db = 20.0 * (peak * norm + 1e-9).log10() + TILT_DB * b as f32 / (BANDS - 1) as f32;
            let lv = ((db - DB_FLOOR) / (DB_CEIL - DB_FLOOR)).clamp(0.0, 1.0);
            self.smooth[b] = lv.max(self.smooth[b] * DECAY);
        }
        self.smooth
    }

    /// Данных нет (тишина): полоски плавно опускаются. Возвращает false, когда всё уже на нуле.
    pub fn fade(&mut self) -> bool {
        let mut any = false;
        for v in self.smooth.iter_mut() {
            *v *= DECAY;
            if *v < 0.01 {
                *v = 0.0;
            } else {
                any = true;
            }
        }
        any
    }

    pub fn levels(&self) -> [f32; BANDS] {
        self.smooth
    }
}

/// Логарифмические границы полос в бинах FFT; каждая полоса минимум в один бин.
fn band_edges(rate: f32, n: usize) -> Vec<usize> {
    let bin_hz = rate / n as f32;
    let hi = F_HI.min(rate / 2.0 - bin_hz);
    let mut edges = Vec::with_capacity(BANDS + 1);
    let mut prev = 0usize;
    for i in 0..=BANDS {
        let f = F_LO * (hi / F_LO).powf(i as f32 / BANDS as f32);
        let k = ((f / bin_hz).round() as usize).clamp(1, n / 2 - 1).max(prev + if i == 0 { 0 } else { 1 });
        edges.push(k);
        prev = k;
    }
    edges
}

/// Быстрое преобразование Фурье на месте (radix-2, длина — степень двойки).
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let ang = -std::f32::consts::TAU / len as f32;
        let (wr, wi) = (ang.cos(), ang.sin());
        for start in (0..n).step_by(len) {
            let (mut cr, mut ci) = (1.0f32, 0.0f32);
            for k in 0..len / 2 {
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cr - im[b] * ci, re[b] * ci + im[b] * cr);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                let nr = cr * wr - ci * wi;
                ci = cr * wi + ci * wr;
                cr = nr;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fft_finds_bin() {
        let n = 64;
        let mut re: Vec<f32> = (0..n).map(|i| (std::f32::consts::TAU * 5.0 * i as f32 / n as f32).sin()).collect();
        let mut im = vec![0.0; n];
        fft(&mut re, &mut im);
        let mags: Vec<f32> = (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt()).collect();
        let best = mags.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(best, 5);
        assert!((mags[5] - n as f32 / 2.0).abs() < 0.01);
    }

    #[test]
    fn edges_grow() {
        let e = band_edges(48_000.0, FFT_N);
        assert_eq!(e.len(), BANDS + 1);
        assert!(e.windows(2).all(|w| w[1] > w[0]));
        assert!(*e.last().unwrap() < FFT_N / 2);
    }

    #[test]
    fn sine_lights_its_band() {
        let rate = 48_000;
        let mut a = Analyzer::new(rate);
        for i in 0..FFT_N {
            a.push(0.5 * (std::f32::consts::TAU * 1000.0 * i as f32 / rate as f32).sin());
        }
        let lv = a.frame();
        let best = lv.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).unwrap().0;
        let bin_hz = rate as f32 / FFT_N as f32;
        let (lo, hi) = (a.edges[best] as f32 * bin_hz, a.edges[best + 1] as f32 * bin_hz);
        assert!(lo <= 1000.0 + bin_hz && 1000.0 - bin_hz <= hi, "1 кГц в полосе {lo}..{hi}");
        assert!(lv[best] > 0.8);
        // Тишина: полоски опускаются до нуля.
        while a.fade() {}
        assert!(a.levels().iter().all(|&v| v == 0.0));
    }

    #[test]
    fn grouping() {
        let mut l = [0.0; BANDS];
        l[0] = 0.3;
        l[23] = 0.9;
        let g = group(&l, 4);
        assert_eq!(g.len(), 4);
        assert_eq!(g[0], 0.3);
        assert_eq!(g[3], 0.9);
    }
}
