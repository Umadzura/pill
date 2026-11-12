//! VPN: встроенные VPN-подключения Windows (телефонные книги + rasdial).

use windows::Win32::NetworkManagement::Rras::*;

use super::launch::{hidden, open};

/// Имена VPN-профилей из телефонных книг Windows.
fn vpn_entries() -> Vec<String> {
    let mut names = Vec::new();
    for var in ["APPDATA", "ProgramData"] {
        let Ok(base) = std::env::var(var) else { continue };
        let pbk = format!(r"{}\Microsoft\Network\Connections\Pbk\rasphone.pbk", base);
        let Ok(data) = std::fs::read(&pbk) else { continue };
        for line in String::from_utf8_lossy(&data).lines() {
            let l = line.trim();
            if l.starts_with('[') && l.ends_with(']') && l.len() > 2 {
                names.push(l[1..l.len() - 1].to_string());
            }
        }
    }
    names
}

fn vpn_connected() -> Vec<String> {
    unsafe {
        let mut conns: [RASCONNW; 8] = std::mem::zeroed();
        for c in conns.iter_mut() {
            c.dwSize = std::mem::size_of::<RASCONNW>() as u32;
        }
        let mut cb = std::mem::size_of_val(&conns) as u32;
        let mut n = 0u32;
        if RasEnumConnectionsW(Some(conns.as_mut_ptr()), &mut cb, &mut n) != 0 {
            return Vec::new();
        }
        conns[..n as usize]
            .iter()
            .map(|c| {
                let len = c.szEntryName.iter().position(|&x| x == 0).unwrap_or(c.szEntryName.len());
                String::from_utf16_lossy(&c.szEntryName[..len])
            })
            .collect()
    }
}

/// (имя профиля, подключён). None, если VPN-профилей нет.
pub fn vpn() -> Option<(String, bool)> {
    let entries = vpn_entries();
    let connected = vpn_connected();
    if let Some(c) = connected.iter().find(|c| entries.contains(c)) {
        return Some((c.clone(), true));
    }
    entries.into_iter().next().map(|e| (e, false))
}

pub fn toggle_vpn() {
    match vpn() {
        Some((name, true)) => hidden("rasdial", &[&name, "/disconnect"]),
        Some((name, false)) => hidden("rasdial", &[&name]),
        None => {
            open("ms-settings:network-vpn", None);
        }
    }
}
