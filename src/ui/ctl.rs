//! Вкладка «Управление»: плитки, громкость, батарея, питание, кто держит микрофон/камеру.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::bg::{watch, winrt};
use crate::feat::ctl::{Ctl, CtlHit, Pw};
use crate::feat::osd::battery_long;
use crate::shell::EXPANDED_W;

pub unsafe fn draw(p: &mut Painter, s: &State, w: i32, h: i32) {
    let radios = *winrt::RADIOS.lock().unwrap();
    let snap = &s.ctl.snap;
    let vpn_label = match &snap.vpn {
        Some((name, _)) => name.clone(),
        None => "VPN".to_string(),
    };
    let muted = snap.volume.map(|v| v.1);
    let tiles: [(Ctl, &str, String, Option<bool>); 6] = [
        (Ctl::Wifi, I_WIFI, "Wi-Fi".into(), radios.wifi),
        (Ctl::Bt, I_BT, "Bluetooth".into(), radios.bt),
        (Ctl::Vpn, I_VPN, vpn_label, snap.vpn.as_ref().map(|v| v.1)),
        (Ctl::Dark, I_THEME, "Тёмная тема".into(), snap.dark),
        (Ctl::Mute, if muted == Some(true) { I_MUTE } else { I_VOL }, "Звук".into(), muted.map(|m| !m)),
        // Не переключатель, а кнопка: всегда «выключена».
        (Ctl::Shot, I_SHOT, "Скриншот".into(), Some(false)),
    ];
    let gap = p.s(8);
    let left = p.s(20);
    let n = tiles.len() as i32;
    let tw = (w - 2 * left - (n - 1) * gap) / n;
    for (i, (c, icon, label, state)) in tiles.iter().enumerate() {
        let x = left + i as i32 * (tw + gap);
        let r = RECT { left: x, top: p.s(58), right: x + tw, bottom: p.s(134) };
        let hit = Hit::Ctl(CtlHit::Toggle(*c));
        let bg = match state {
            Some(true) => ON,
            Some(false) if p.hovered(hit) => HOVER,
            Some(false) => PANEL,
            None if p.hovered(hit) => PANEL,
            None => OFF_DIM,
        };
        p.fill(r, 14, bg);
        let fg = if state.is_some() { FG } else { DIM };
        p.text(p.f.icon_big, icon, RECT { bottom: r.top + p.s(46), top: r.top + p.s(8), ..r }, fg, DT_CENTER);
        let lr = RECT { left: r.left + p.s(6), right: r.right - p.s(6), top: r.top + p.s(46), bottom: r.bottom - p.s(8) };
        p.text(p.f.small, label, lr, fg, DT_CENTER | DT_END_ELLIPSIS);
        p.hit(r, hit);
    }

    p.volume_row(20, 150, EXPANDED_W - 20, snap.volume);
    p.text(p.f.small, &battery_long(), p.r(20, 184, EXPANDED_W - 20, 206), DIM, DT_LEFT);

    let buttons = [
        (Pw::Lock, I_LOCK, "Блокировка"),
        (Pw::Sleep, I_SLEEP, "Сон"),
        (Pw::Restart, I_RESTART, "Перезагрузка"),
        (Pw::Shutdown, I_POWER, "Выключение"),
    ];
    let bw = (w - 2 * left - 3 * gap) / 4;
    for (i, (pw, icon, label)) in buttons.iter().enumerate() {
        let x = left + i as i32 * (bw + gap);
        let r = RECT { left: x, top: p.s(222), right: x + bw, bottom: p.s(268) };
        let hit = Hit::Ctl(CtlHit::Power(*pw));
        let confirming = s.ctl.confirming(*pw);
        p.fill(r, 14, if confirming { DANGER } else if p.hovered(hit) { HOVER } else { PANEL });
        let label = if confirming { "Точно? Ещё раз" } else { label };
        p.text(p.f.icon, icon, RECT { left: r.left + p.s(12), right: r.left + p.s(36), ..r }, FG, DT_LEFT);
        p.text(p.f.small, label, RECT { left: r.left + p.s(36), right: r.right - p.s(6), ..r }, FG, DT_LEFT | DT_END_ELLIPSIS);
        p.hit(r, hit);
    }
    // Кто держит микрофон и камеру.
    let u = watch::IN_USE.lock().unwrap().clone();
    let mut parts = Vec::new();
    if !u.mic.is_empty() {
        parts.push(format!("Микрофон: {}", u.mic.join(", ")));
    }
    if !u.cam.is_empty() {
        parts.push(format!("Камера: {}", u.cam.join(", ")));
    }
    if !parts.is_empty() {
        let c = if u.cam.is_empty() { MIC } else { CAM };
        p.text(p.f.small, &format!("●  {}", parts.join("   ·   ")), p.r(20, 274, EXPANDED_W - 20, 294), c, DT_LEFT | DT_END_ELLIPSIS);
    }
    p.hint(w, h, "Колесо мыши — громкость · Нет плитки? Клик откроет настройки");
}
