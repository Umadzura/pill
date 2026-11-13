//! Вкладка «Управление»: плитки 3×2 с состоянием словами, громкость, яркость, батарея, питание,
//! кто держит микрофон/камеру.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::bg::{watch, winrt};
use crate::feat::ctl::{Ctl, CtlHit, Pw};
use crate::feat::osd::battery_long;
use crate::shell::EXPANDED_W;

const TILE_H: i32 = 52;
const TILE_TOP: i32 = 58;
/// Диаметр круглой кнопки питания.
const PW: i32 = 34;

fn word(state: Option<bool>, on: &str, off: &str, none: &str) -> String {
    match state {
        Some(true) => on,
        Some(false) => off,
        None => none,
    }
    .to_string()
}

pub unsafe fn draw(p: &mut Painter, s: &State, w: i32, h: i32) {
    let radios = *winrt::RADIOS.lock().unwrap();
    let snap = &s.ctl.snap;
    let vpn = winrt::VPN.lock().unwrap().clone();
    let vpn_on = vpn.as_ref().map(|v| v.1);
    let vpn_label = vpn.map(|v| v.0).unwrap_or_else(|| "VPN".to_string());
    let muted = snap.volume.map(|v| v.1);
    let sound = match snap.volume {
        Some((_, true)) => "без звука".to_string(),
        Some((v, false)) => format!("{}%", (v * 100.0).round() as i32),
        None => "нет устройства".to_string(),
    };
    // (плитка, значок, название, состояние словами, включено / выключено / недоступно)
    let tiles: [(Ctl, &str, String, String, Option<bool>); 6] = [
        (Ctl::Wifi, I_WIFI, "Wi-Fi".into(), word(radios.wifi, "включён", "выключен", "нет адаптера"), radios.wifi),
        (Ctl::Bt, I_BT, "Bluetooth".into(), word(radios.bt, "включён", "выключен", "нет адаптера"), radios.bt),
        (Ctl::Vpn, I_VPN, vpn_label, word(vpn_on, "подключён", "отключён", "не настроен"), vpn_on),
        (Ctl::Dark, I_THEME, "Тема".into(), word(snap.dark, "тёмная", "светлая", "недоступно"), snap.dark),
        (Ctl::Mute, if muted == Some(true) { I_MUTE } else { I_VOL }, "Звук".into(), sound, muted.map(|m| !m)),
        // Не переключатель, а кнопка: всегда «выключена».
        (Ctl::Shot, I_SHOT, "Скриншот".into(), "область экрана".into(), Some(false)),
    ];
    let gap = p.s(8);
    let left = p.s(20);
    let tw = (w - 2 * left - 2 * gap) / 3;
    for (i, (c, icon, label, sub, state)) in tiles.iter().enumerate() {
        let (col, row) = ((i % 3) as i32, (i / 3) as i32);
        let x = left + col * (tw + gap);
        let top = p.s(TILE_TOP + row * (TILE_H + 8));
        let r = RECT { left: x, top, right: x + tw, bottom: top + p.s(TILE_H) };
        let hit = Hit::Ctl(CtlHit::Toggle(*c));
        let bg = match state {
            Some(true) => ON,
            Some(false) if p.hovered(hit) => HOVER,
            Some(false) => PANEL,
            None if p.hovered(hit) => PANEL,
            None => OFF_DIM,
        };
        p.fill(r, R_TILE, bg);
        let fg = if state.is_some() { FG } else { DIM };
        // На синей плитке серый текст не читается: состояние тем же цветом, что название.
        let sub_fg = if *state == Some(true) { FG } else { DIM };
        p.text(p.f.icon, icon, RECT { left: r.left + p.s(14), right: r.left + p.s(40), ..r }, fg, DT_LEFT);
        let tx = RECT { left: r.left + p.s(46), right: r.right - p.s(8), ..r };
        p.text(p.f.bold, label, RECT { top: r.top + p.s(7), bottom: r.top + p.s(28), ..tx }, fg, DT_LEFT | DT_END_ELLIPSIS);
        p.text(p.f.small, sub, RECT { top: r.top + p.s(26), bottom: r.bottom - p.s(6), ..tx }, sub_fg, DT_LEFT | DT_END_ELLIPSIS);
        p.hit(r, hit);
    }

    let mut y = TILE_TOP + 2 * TILE_H + 8 + 12;
    p.volume_row(20, y, EXPANDED_W - 20, snap.volume);
    y += 30;
    // Яркость показываем, только если есть чем управлять (ноутбук или монитор с DDC/CI).
    let bright = *winrt::BRIGHTNESS.lock().unwrap();
    if let Some(b) = bright {
        p.slider_row(20, y, EXPANDED_W - 20, I_BRIGHT, Some(b as f32 / 100.0), false, Hit::Brightness);
        y += 30;
    }
    y += 8;

    // Питание: четыре круглые кнопки справа. Слева батарея, а под курсором — что делает кнопка.
    let buttons = [
        (Pw::Lock, I_LOCK, "Блокировка"),
        (Pw::Sleep, I_SLEEP, "Сон"),
        (Pw::Restart, I_RESTART, "Перезагрузка"),
        (Pw::Shutdown, I_POWER, "Выключение"),
    ];
    let mut note = (battery_long(), DIM);
    let n = buttons.len() as i32;
    let x0 = EXPANDED_W - 20 - n * PW - (n - 1) * 6;
    for (i, (pw, icon, label)) in buttons.iter().enumerate() {
        let x = x0 + i as i32 * (PW + 6);
        let r = p.r(x, y, x + PW, y + PW);
        let hit = Hit::Ctl(CtlHit::Power(*pw));
        let confirming = s.ctl.confirming(*pw);
        if confirming {
            note = (format!("{}: нажми ещё раз", label), DANGER);
        } else if p.hovered(hit) && note.1 != DANGER {
            note = (label.to_string(), FG);
        }
        p.fill(r, R_PILL, if confirming { DANGER } else if p.hovered(hit) { HOVER } else { PANEL });
        p.text(p.f.icon, icon, r, FG, DT_CENTER);
        p.hit(r, hit);
    }
    p.text(p.f.small, &note.0, p.r(20, y, x0 - 12, y + PW), note.1, DT_LEFT | DT_END_ELLIPSIS);
    y += PW + 8;

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
        p.text(p.f.small, &format!("●  {}", parts.join("   ·   ")), p.r(20, y, EXPANDED_W - 20, y + 20), c, DT_LEFT | DT_END_ELLIPSIS);
    }
    p.hint(w, h, "Колесо — громкость, над яркостью — яркость · Нет плитки? Клик откроет настройки");
}
