//! Счётчики системы прямо из Windows, без драйверов и прав админа: ЦП, память, процессы,
//! сеть, диск, видеокарта (D3DKMT). Разбор и арифметика — в `feat::sysmon`.

use windows::Wdk::System::SystemInformation::{NtQuerySystemInformation, SystemProcessInformation};
use windows::Wdk::Graphics::Direct3D::*;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, LUID};
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows::Win32::Storage::FileSystem::{CreateFileW, GetDiskFreeSpaceExW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::System::IO::DeviceIoControl;
use windows::Win32::System::Ioctl::{DISK_PERFORMANCE, IOCTL_DISK_PERFORMANCE};
use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
use windows::Win32::System::Threading::{GetActiveProcessorCount, GetSystemTimes};

/// Сумма времени ЦП всех ядер (единицы по 100 нс): простой и всё вместе.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct CpuTimes {
    pub idle: u64,
    pub total: u64,
}

fn ft(f: FILETIME) -> u64 {
    (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64
}

/// Один системный вызов, микросекунды: можно звать раз в секунду даже в свёрнутом виде.
pub fn cpu_times() -> Option<CpuTimes> {
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()? };
    // Время ядра уже включает простой.
    Some(CpuTimes { idle: ft(idle), total: ft(kernel) + ft(user) })
}

/// (занято, всего) байт физической памяти.
pub fn memory() -> Option<(u64, u64)> {
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe { GlobalMemoryStatusEx(&mut m).ok()? };
    Some((m.ullTotalPhys - m.ullAvailPhys, m.ullTotalPhys))
}

pub fn cpu_count() -> u32 {
    // 0xFFFF = все группы процессоров.
    unsafe { GetActiveProcessorCount(0xFFFF) }.max(1)
}

/// Список процессов ядра (SYSTEM_PROCESS_INFORMATION подряд) в переиспользуемый буфер.
/// Возвращает адрес начала буфера: имена процессов — указатели внутрь него.
pub fn processes(buf: &mut Vec<u8>) -> Option<usize> {
    const MISMATCH: i32 = 0xC000_0004u32 as i32;
    if buf.len() < 256 * 1024 {
        buf.resize(256 * 1024, 0);
    }
    for _ in 0..4 {
        let mut need = 0u32;
        let st = unsafe { NtQuerySystemInformation(SystemProcessInformation, buf.as_mut_ptr().cast(), buf.len() as u32, &mut need) };
        if st.0 == MISMATCH {
            // Процессы появляются между вызовами: берём с запасом.
            buf.resize((need as usize).max(buf.len()) + 64 * 1024, 0);
            continue;
        }
        return (st.0 >= 0).then_some(buf.as_ptr() as usize);
    }
    None
}

/// Сколько байт принято и отправлено через настоящие сетевые адаптеры (без петли, виртуальных и фильтров).
pub fn net_totals() -> Option<(u64, u64)> {
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t).is_err() || t.is_null() {
            return None;
        }
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), (*t).NumEntries as usize);
        let (mut rx, mut tx) = (0u64, 0u64);
        for r in rows {
            let flags = r.InterfaceAndOperStatusFlags._bitfield;
            // Бит 0 — настоящее железо, бит 1 — фильтр (дублирует счётчики адаптера).
            let hardware = flags & 1 != 0 && flags & 2 == 0;
            if hardware && r.OperStatus == IfOperStatusUp {
                rx += r.InOctets;
                tx += r.OutOctets;
            }
        }
        FreeMibTable(t as *const _);
        Some((rx, tx))
    }
}

/// (свободно, всего) байт на системном диске.
pub fn system_disk() -> Option<(u64, u64)> {
    let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()) + "\\";
    let drive = windows::core::HSTRING::from(drive);
    let (mut free, mut total) = (0u64, 0u64);
    unsafe { GetDiskFreeSpaceExW(&drive, Some(&mut free), Some(&mut total), None).ok()? };
    Some((free, total))
}

/// Активность дисков прямо от драйвера (IOCTL_DISK_PERFORMANCE): сколько байт прочитано и записано.
/// Открывается вместе со вкладкой «Система» и закрывается с ней.
/// PDH не используем: даже один счётчик держит ~5 МБ, которые не отдаются после закрытия.
pub struct Disks {
    handles: Vec<HANDLE>,
}

impl Disks {
    pub fn open() -> Option<Disks> {
        let mut handles = Vec::new();
        for i in 0..16 {
            let path = windows::core::HSTRING::from(format!(r"\\.\PhysicalDrive{}", i));
            // Доступ 0: читаем только статистику, прав админа не нужно.
            let h = unsafe { CreateFileW(&path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) };
            match h {
                Ok(h) if !h.is_invalid() => handles.push(h),
                _ => {}
            }
        }
        (!handles.is_empty()).then_some(Disks { handles })
    }

    /// Всего байт прочитано + записано всеми дисками с загрузки.
    pub fn bytes(&self) -> Option<u64> {
        let mut total = 0u64;
        let mut any = false;
        for &h in &self.handles {
            let mut perf = DISK_PERFORMANCE::default();
            let mut got = 0u32;
            let ok = unsafe {
                DeviceIoControl(
                    h,
                    IOCTL_DISK_PERFORMANCE,
                    None,
                    0,
                    Some(&mut perf as *mut _ as *mut _),
                    std::mem::size_of::<DISK_PERFORMANCE>() as u32,
                    Some(&mut got),
                    None,
                )
            };
            if ok.is_ok() {
                total += (perf.BytesRead.max(0) + perf.BytesWritten.max(0)) as u64;
                any = true;
            }
        }
        any.then_some(total)
    }
}

impl Drop for Disks {
    fn drop(&mut self) {
        for &h in &self.handles {
            unsafe {
                let _ = CloseHandle(h);
            }
        }
    }
}

/// Видеокарты как их видит графическое ядро Windows (так считает и диспетчер задач):
/// у каждого движка (3D, копирование, видео…) есть счётчик времени работы.
pub struct Gpu {
    /// (LUID адаптера, сколько у него движков).
    adapters: Vec<(LUID, u32)>,
}

unsafe fn kmt_stats(q: &mut D3DKMT_QUERYSTATISTICS) -> bool {
    D3DKMTQueryStatistics(q).0 >= 0
}

impl Gpu {
    pub fn open() -> Option<Gpu> {
        unsafe {
            let mut en = D3DKMT_ENUMADAPTERS2::default();
            if D3DKMTEnumAdapters2(&mut en).0 < 0 || en.NumAdapters == 0 {
                return None;
            }
            let mut list = vec![D3DKMT_ADAPTERINFO::default(); en.NumAdapters as usize];
            en.pAdapters = list.as_mut_ptr();
            if D3DKMTEnumAdapters2(&mut en).0 < 0 {
                return None;
            }
            list.truncate(en.NumAdapters as usize);
            let mut adapters = Vec::new();
            for a in &list {
                let mut q = D3DKMT_QUERYSTATISTICS { Type: D3DKMT_QUERYSTATISTICS_ADAPTER, AdapterLuid: a.AdapterLuid, ..Default::default() };
                if kmt_stats(&mut q) {
                    let nodes = q.QueryResult.AdapterInformation.NodeCount;
                    if nodes > 0 {
                        adapters.push((a.AdapterLuid, nodes));
                    }
                }
                let _ = D3DKMTCloseAdapter(&D3DKMT_CLOSEADAPTER { hAdapter: a.hAdapter });
            }
            (!adapters.is_empty()).then_some(Gpu { adapters })
        }
    }

    /// Время работы каждого движка всех адаптеров (единицы по 100 нс), в постоянном порядке.
    pub fn running(&self) -> Vec<i64> {
        let mut out = Vec::new();
        for &(luid, nodes) in &self.adapters {
            for n in 0..nodes {
                let mut q = D3DKMT_QUERYSTATISTICS { Type: D3DKMT_QUERYSTATISTICS_NODE, AdapterLuid: luid, ..Default::default() };
                q.Anonymous.QueryNode.NodeId = n;
                let t = unsafe { if kmt_stats(&mut q) { q.QueryResult.NodeInformation.GlobalInformation.RunningTime } else { 0 } };
                out.push(t);
            }
        }
        out
    }
}
