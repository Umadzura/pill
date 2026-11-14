//! Монитор системы и предупреждение о нагрузке.
//!
//! * Вкладка «Система»: раз в секунду, и только пока она открыта, фоновый воркер собирает
//!   `Snapshot` (ЦП, память, видеокарта, сеть, диск, топ приложений). Закрыли вкладку — всё освобождается.
//! * Предупреждение: на уже существующем секундном тике два дешёвых вызова (время ЦП и память).
//!   ЦП или память в потолке 30 с подряд — пилюля называет приложение, которое грузит больше всех.
//!   Это согласованное исключение из «опроса только в раскрытом виде»: новых таймеров и потоков нет.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::Instant;

use crate::sys::sysinfo::{self, CpuTimes, Disks, Gpu};

/// Сколько секунд истории ЦП показываем.
pub const HISTORY: usize = 60;
pub const TOP_APPS: usize = 5;
/// Порог «в потолке», %, и сколько секунд подряд.
const HIGH: f32 = 90.0;
const SUSTAIN: u32 = 30;
/// После предупреждения молчим, пока нагрузка не спадёт на столько секунд подряд.
const CALM: u32 = 20;

pub fn cpu_pct(prev: CpuTimes, cur: CpuTimes) -> f32 {
    let total = cur.total.saturating_sub(prev.total);
    if total == 0 {
        return 0.0;
    }
    let idle = cur.idle.saturating_sub(prev.idle).min(total);
    (total - idle) as f32 * 100.0 / total as f32
}

// ---------- процессы ----------

#[derive(Clone, Debug, PartialEq)]
pub struct Proc {
    pub pid: usize,
    pub name: String,
    /// Время ЦП (ядро + пользователь), единицы по 100 нс.
    pub cpu: u64,
    /// Личный рабочий набор, байт (как «Память» в диспетчере задач).
    pub ram: u64,
}

// Смещения полей SYSTEM_PROCESS_INFORMATION в 64-битной Windows.
const OFF_NEXT: usize = 0;
const OFF_PRIVATE_WS: usize = 8;
const OFF_USER: usize = 40;
const OFF_KERNEL: usize = 48;
const OFF_NAME_LEN: usize = 56;
const OFF_NAME_PTR: usize = 64;
const OFF_PID: usize = 80;
const MIN_ENTRY: usize = 152;

fn rd<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    b.get(at..at + N)?.try_into().ok()
}

/// Разбор списка процессов ядра. `base` — адрес буфера в памяти: имена лежат по указателям внутрь него.
pub fn parse_processes(buf: &[u8], base: usize) -> Vec<Proc> {
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        if at + MIN_ENTRY > buf.len() {
            break;
        }
        let next = u32::from_le_bytes(rd(buf, at + OFF_NEXT).unwrap_or_default()) as usize;
        let pid = u64::from_le_bytes(rd(buf, at + OFF_PID).unwrap_or_default()) as usize;
        let user = u64::from_le_bytes(rd(buf, at + OFF_USER).unwrap_or_default());
        let kernel = u64::from_le_bytes(rd(buf, at + OFF_KERNEL).unwrap_or_default());
        let ram = u64::from_le_bytes(rd(buf, at + OFF_PRIVATE_WS).unwrap_or_default());
        let len = u16::from_le_bytes(rd(buf, at + OFF_NAME_LEN).unwrap_or_default()) as usize;
        let ptr = u64::from_le_bytes(rd(buf, at + OFF_NAME_PTR).unwrap_or_default()) as usize;
        let name = ptr
            .checked_sub(base)
            .and_then(|off| buf.get(off..off + len))
            .map(|b| String::from_utf16_lossy(&b.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect::<Vec<_>>()))
            .unwrap_or_default();
        // pid 0 — «Бездействие системы», это не нагрузка.
        if pid != 0 {
            out.push(Proc { pid, name, cpu: user + kernel, ram });
        }
        if next == 0 {
            break;
        }
        at += next;
    }
    out
}

/// Приложение = все процессы с одним именем (десяток chrome.exe — одна строка).
#[derive(Clone, Debug, PartialEq)]
pub struct AppUse {
    pub name: String,
    /// % всей машины (все ядра = 100%).
    pub cpu: f32,
    pub ram: u64,
}

fn pretty(exe: &str) -> String {
    let s = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(exe);
    if s.is_empty() { "Система".into() } else { s.to_string() }
}

/// Нагрузка по приложениям между двумя снимками. `dt` — прошедшее время в единицах по 100 нс.
pub fn apps(prev: &[Proc], cur: &[Proc], dt: u64, ncpu: u32) -> Vec<AppUse> {
    let before: HashMap<usize, u64> = prev.iter().map(|p| (p.pid, p.cpu)).collect();
    let budget = (dt.max(1) * ncpu.max(1) as u64) as f32;
    let mut by_name: HashMap<String, AppUse> = HashMap::new();
    for p in cur {
        // Новый процесс без прошлого замера: его ЦП за интервал неизвестен, считаем 0.
        let used = before.get(&p.pid).map(|&b| p.cpu.saturating_sub(b)).unwrap_or(0);
        let e = by_name.entry(p.name.to_lowercase()).or_insert_with(|| AppUse { name: pretty(&p.name), cpu: 0.0, ram: 0 });
        e.cpu += used as f32 * 100.0 / budget;
        e.ram += p.ram;
    }
    by_name.into_values().collect()
}

pub fn top_by_cpu(mut v: Vec<AppUse>, n: usize) -> Vec<AppUse> {
    v.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.ram.cmp(&a.ram)));
    v.truncate(n);
    v
}

pub fn top_by_ram(mut v: Vec<AppUse>, n: usize) -> Vec<AppUse> {
    v.sort_by(|a, b| b.ram.cmp(&a.ram));
    v.truncate(n);
    v
}

/// Загрузка видеокарты как в диспетчере задач: самый занятый движок.
/// `prev`/`cur` — время работы движков (100 нс), `dt` — прошедшее время в тех же единицах.
pub fn gpu_busy(prev: &[i64], cur: &[i64], dt: u64) -> Option<f32> {
    if prev.is_empty() || prev.len() != cur.len() || dt == 0 {
        return None;
    }
    let best = prev.iter().zip(cur).map(|(a, b)| (b - a).max(0) as f64).fold(0.0, f64::max);
    Some((best * 100.0 / dt as f64).clamp(0.0, 100.0) as f32)
}

// ---------- снимок для вкладки ----------

#[derive(Clone, Default)]
pub struct Snapshot {
    pub cpu: f32,
    pub cpu_hist: VecDeque<f32>,
    /// История памяти и видеокарты за ту же минуту, в процентах (у видеокарты пусто, если данных нет).
    pub ram_hist: VecDeque<f32>,
    pub gpu_hist: VecDeque<f32>,
    pub ram: Option<(u64, u64)>,
    pub gpu: Option<f32>,
    /// Байт/с: принято, отправлено.
    pub net: Option<(f64, f64)>,
    pub disk_rate: Option<f64>,
    pub disk: Option<(u64, u64)>,
    pub apps: Vec<AppUse>,
}

/// Последний снимок. Пишет воркер, читает отрисовка вкладки «Система».
pub static SNAPSHOT: Mutex<Option<Snapshot>> = Mutex::new(None);
/// Текст предупреждения о нагрузке, приготовленный воркером (UI забирает и показывает).
pub static LOAD_WARN: Mutex<Option<String>> = Mutex::new(None);

/// Дописать значение в историю за минуту, выкинув самое старое.
fn push_hist(h: &mut VecDeque<f32>, v: f32) {
    if h.len() == HISTORY {
        h.pop_front();
    }
    h.push_back(v);
}

/// Сборщик снимков. Живёт в фоновом воркере, пока открыта вкладка.
pub struct Sampler {
    at: Instant,
    cpu: Option<CpuTimes>,
    procs: Vec<Proc>,
    net: Option<(u64, u64)>,
    hist: VecDeque<f32>,
    ram_hist: VecDeque<f32>,
    gpu_hist: VecDeque<f32>,
    buf: Vec<u8>,
    disks: Option<Disks>,
    disk_prev: Option<u64>,
    gpu: Option<Gpu>,
    gpu_prev: Vec<i64>,
    ncpu: u32,
}

impl Sampler {
    pub fn new() -> Self {
        let disks = Disks::open();
        let disk_prev = disks.as_ref().and_then(|d| d.bytes());
        Sampler {
            at: Instant::now(),
            cpu: sysinfo::cpu_times(),
            procs: Vec::new(),
            net: sysinfo::net_totals(),
            hist: VecDeque::with_capacity(HISTORY),
            ram_hist: VecDeque::with_capacity(HISTORY),
            gpu_hist: VecDeque::with_capacity(HISTORY),
            buf: Vec::new(),
            disks,
            disk_prev,
            gpu: Gpu::open(),
            gpu_prev: Vec::new(),
            ncpu: sysinfo::cpu_count(),
        }
    }

    fn read_procs(&mut self) -> Vec<Proc> {
        match sysinfo::processes(&mut self.buf) {
            Some(base) => parse_processes(&self.buf, base),
            None => Vec::new(),
        }
    }

    pub fn sample(&mut self) -> Snapshot {
        let now = Instant::now();
        let secs = now.duration_since(self.at).as_secs_f64().max(0.001);
        self.at = now;

        let cur = sysinfo::cpu_times();
        let cpu = match (self.cpu, cur) {
            (Some(a), Some(b)) => cpu_pct(a, b),
            _ => 0.0,
        };
        self.cpu = cur;
        push_hist(&mut self.hist, cpu);

        let procs = self.read_procs();
        let dt = (secs * 1e7) as u64;
        let apps = if self.procs.is_empty() { Vec::new() } else { top_by_cpu(apps(&self.procs, &procs, dt, self.ncpu), TOP_APPS) };
        self.procs = procs;

        let net_now = sysinfo::net_totals();
        let net = match (self.net, net_now) {
            (Some((r0, t0)), Some((r1, t1))) => Some((r1.saturating_sub(r0) as f64 / secs, t1.saturating_sub(t0) as f64 / secs)),
            _ => None,
        };
        self.net = net_now;

        let gpu_now = self.gpu.as_ref().map(|g| g.running()).unwrap_or_default();
        let gpu = gpu_busy(&self.gpu_prev, &gpu_now, dt);
        self.gpu_prev = gpu_now;
        let disk_now = self.disks.as_ref().and_then(|d| d.bytes());
        let disk_rate = match (self.disk_prev, disk_now) {
            (Some(a), Some(b)) => Some(b.saturating_sub(a) as f64 / secs),
            _ => None,
        };
        self.disk_prev = disk_now;
        let ram = sysinfo::memory();
        if let Some((used, total)) = ram {
            push_hist(&mut self.ram_hist, used as f32 * 100.0 / total.max(1) as f32);
        }
        if let Some(g) = gpu {
            push_hist(&mut self.gpu_hist, g);
        }

        Snapshot {
            cpu,
            cpu_hist: self.hist.clone(),
            ram_hist: self.ram_hist.clone(),
            gpu_hist: self.gpu_hist.clone(),
            ram,
            gpu,
            net,
            disk_rate,
            disk: sysinfo::system_disk(),
            apps,
        }
    }
}

/// Для предупреждения: снимок процессов, сделанный за пару секунд до решения.
pub struct WarnProbe {
    at: Instant,
    procs: Vec<Proc>,
    buf: Vec<u8>,
}

impl WarnProbe {
    pub fn take() -> Self {
        let mut buf = Vec::new();
        let procs = sysinfo::processes(&mut buf).map(|b| parse_processes(&buf, b)).unwrap_or_default();
        WarnProbe { at: Instant::now(), procs, buf }
    }

    /// Готовый текст предупреждения: кто больше всех грузит ЦП (или занимает память).
    pub fn finish(mut self, kind: Load) -> String {
        let cur = sysinfo::processes(&mut self.buf).map(|b| parse_processes(&self.buf, b)).unwrap_or_default();
        let dt = (self.at.elapsed().as_secs_f64() * 1e7) as u64;
        let all = apps(&self.procs, &cur, dt, sysinfo::cpu_count());
        match kind {
            Load::Cpu => match top_by_cpu(all, 1).first() {
                Some(a) if a.cpu >= 1.0 => format!("ЦП в потолке · {} {:.0}%", a.name, a.cpu),
                _ => "ЦП загружен больше 90% уже полминуты".into(),
            },
            Load::Ram => match top_by_ram(all, 1).first() {
                Some(a) => format!("Память кончается · {} {}", a.name, fmt_bytes(a.ram)),
                None => "Память занята больше 90%".into(),
            },
        }
    }
}

// ---------- наблюдатель нагрузки (секундный тик UI) ----------

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Load {
    Cpu,
    Ram,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LoadAction {
    None,
    /// За пару секунд до порога: снять процессы, чтобы потом посчитать, кто грузит.
    Probe(Load),
    /// Порог пройден: показать предупреждение.
    Warn(Load),
}

#[derive(Default)]
pub struct LoadWatch {
    prev: Option<CpuTimes>,
    cpu_high: u32,
    ram_high: u32,
    /// Уже предупредили: ждём, пока нагрузка спадёт.
    warned: bool,
    calm: u32,
}

impl LoadWatch {
    /// Раз в секунду. Счётчики (`sysinfo::cpu_times`, `sysinfo::memory`) читает вызывающий,
    /// снаружи `st()`: внутри замыкания Win32 не зовём.
    pub fn tick(&mut self, cur: Option<CpuTimes>, mem: Option<(u64, u64)>) -> LoadAction {
        let cpu = match (self.prev, cur) {
            (Some(a), Some(b)) => cpu_pct(a, b),
            _ => 0.0,
        };
        self.prev = cur;
        let ram = mem.map(|(u, t)| u as f32 * 100.0 / t.max(1) as f32).unwrap_or(0.0);
        self.step(cpu, ram)
    }

    /// Логика без Windows (для тестов).
    pub fn step(&mut self, cpu: f32, ram: f32) -> LoadAction {
        self.cpu_high = if cpu >= HIGH { self.cpu_high + 1 } else { 0 };
        self.ram_high = if ram >= HIGH { self.ram_high + 1 } else { 0 };
        if self.warned {
            self.calm = if self.cpu_high == 0 && self.ram_high == 0 { self.calm + 1 } else { 0 };
            if self.calm >= CALM {
                self.warned = false;
                self.calm = 0;
            }
            return LoadAction::None;
        }
        for (streak, kind) in [(self.cpu_high, Load::Cpu), (self.ram_high, Load::Ram)] {
            if streak == SUSTAIN - 2 {
                return LoadAction::Probe(kind);
            }
            if streak >= SUSTAIN {
                self.warned = true;
                self.calm = 0;
                return LoadAction::Warn(kind);
            }
        }
        LoadAction::None
    }
}

// ---------- форматы ----------

pub fn fmt_bytes(b: u64) -> String {
    const K: f64 = 1024.0;
    let b = b as f64;
    if b >= K * K * K {
        format!("{:.1} ГБ", b / (K * K * K)).replace('.', ",")
    } else if b >= K * K {
        format!("{:.0} МБ", b / (K * K))
    } else {
        format!("{:.0} КБ", b / K)
    }
}

pub fn fmt_rate(bps: f64) -> String {
    const K: f64 = 1024.0;
    if bps >= K * K {
        format!("{:.1} МБ/с", bps / (K * K)).replace('.', ",")
    } else {
        format!("{:.0} КБ/с", bps / K)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_percent() {
        let a = CpuTimes { idle: 100, total: 200 };
        let b = CpuTimes { idle: 150, total: 400 };
        assert!((cpu_pct(a, b) - 75.0).abs() < 0.01);
        assert_eq!(cpu_pct(a, a), 0.0);
    }

    /// Собрать буфер из двух записей так, как его отдаёт ядро.
    fn fake_buf(base_hint: &mut Vec<u8>) -> usize {
        let entry = 200usize;
        base_hint.resize(entry * 2 + 64, 0);
        let base = base_hint.as_ptr() as usize;
        let put = |b: &mut Vec<u8>, at: usize, v: &[u8]| b[at..at + v.len()].copy_from_slice(v);
        // Запись 1: chrome.exe, pid 10.
        let name1: Vec<u8> = "chrome.exe".encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
        let name_at = entry * 2;
        put(base_hint, name_at, &name1);
        put(base_hint, OFF_NEXT, &(entry as u32).to_le_bytes());
        put(base_hint, OFF_PRIVATE_WS, &(100u64 << 20).to_le_bytes());
        put(base_hint, OFF_USER, &30u64.to_le_bytes());
        put(base_hint, OFF_KERNEL, &12u64.to_le_bytes());
        put(base_hint, OFF_NAME_LEN, &(name1.len() as u16).to_le_bytes());
        put(base_hint, OFF_NAME_PTR, &((base + name_at) as u64).to_le_bytes());
        put(base_hint, OFF_PID, &10u64.to_le_bytes());
        // Запись 2: Idle (pid 0) — пропускается.
        put(base_hint, entry + OFF_NEXT, &0u32.to_le_bytes());
        base
    }

    #[test]
    fn parses_processes() {
        let mut buf = Vec::new();
        let base = fake_buf(&mut buf);
        let p = parse_processes(&buf, base);
        assert_eq!(p, vec![Proc { pid: 10, name: "chrome.exe".into(), cpu: 42, ram: 100 << 20 }]);
    }

    #[test]
    fn groups_apps() {
        let prev = vec![
            Proc { pid: 1, name: "chrome.exe".into(), cpu: 0, ram: 0 },
            Proc { pid: 2, name: "chrome.exe".into(), cpu: 0, ram: 0 },
            Proc { pid: 3, name: "code.exe".into(), cpu: 0, ram: 0 },
        ];
        let cur = vec![
            Proc { pid: 1, name: "chrome.exe".into(), cpu: 2_500_000, ram: 10 },
            Proc { pid: 2, name: "chrome.exe".into(), cpu: 2_500_000, ram: 20 },
            Proc { pid: 3, name: "code.exe".into(), cpu: 1_000_000, ram: 500 },
            Proc { pid: 4, name: "new.exe".into(), cpu: 9_000_000, ram: 1 },
        ];
        // 1 секунда, 2 ядра: 5 000 000 из 20 000 000 = 25%.
        let top = top_by_cpu(apps(&prev, &cur, 10_000_000, 2), 2);
        assert_eq!(top[0].name, "chrome");
        assert!((top[0].cpu - 25.0).abs() < 0.01);
        assert_eq!(top[0].ram, 30);
        assert_eq!(top[1].name, "code");
        assert_eq!(top_by_ram(apps(&prev, &cur, 10_000_000, 2), 1)[0].name, "code");
    }

    #[test]
    fn gpu_engines() {
        // Три движка за 1 с: 3D занят 0,55 с, видео 0,4 с.
        assert_eq!(gpu_busy(&[0, 100, 0], &[5_500_000, 4_000_100, 0], 10_000_000), Some(55.0));
        assert_eq!(gpu_busy(&[], &[1], 10), None);
        assert_eq!(gpu_busy(&[1, 2], &[1], 10), None);
    }

    #[test]
    fn load_watch_warns_once() {
        let mut w = LoadWatch::default();
        let mut actions = Vec::new();
        for _ in 0..40 {
            actions.push(w.step(97.0, 40.0));
        }
        assert_eq!(actions[SUSTAIN as usize - 3], LoadAction::Probe(Load::Cpu));
        assert_eq!(actions[SUSTAIN as usize - 1], LoadAction::Warn(Load::Cpu));
        assert_eq!(actions.iter().filter(|a| matches!(a, LoadAction::Warn(_))).count(), 1);
        // Спала на CALM секунд — снова может предупредить.
        for _ in 0..CALM {
            w.step(10.0, 40.0);
        }
        let again: Vec<_> = (0..SUSTAIN).map(|_| w.step(10.0, 95.0)).collect();
        assert_eq!(*again.last().unwrap(), LoadAction::Warn(Load::Ram));
        // Короткий всплеск не считается.
        let mut w = LoadWatch::default();
        for i in 0..100 {
            let cpu = if i % 20 < 15 { 99.0 } else { 5.0 };
            assert!(!matches!(w.step(cpu, 0.0), LoadAction::Warn(_)));
        }
    }

    #[test]
    fn formats() {
        assert_eq!(fmt_bytes(3 * 1024 * 1024 * 1024 / 2), "1,5 ГБ");
        assert_eq!(fmt_bytes(250 * 1024 * 1024), "250 МБ");
        assert_eq!(fmt_rate(2.5 * 1024.0 * 1024.0), "2,5 МБ/с");
        assert_eq!(fmt_rate(10240.0), "10 КБ/с");
    }
}
