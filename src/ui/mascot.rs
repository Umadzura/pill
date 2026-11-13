//! Маскот агента: маленькое привидение 9×8, рисуется прямоугольниками прямо в коде.
//! Цвет тела зависит от имени проекта (у каждого агента свой), состояние видно по лицу:
//! работает — глаза бегают раз в секунду, нужно разрешение — глаза круглые и рот «о»,
//! ждёт ответа — глаза в сторону, закончил — довольные прищуренные глаза и улыбка.
//! Таймера нет: «кадр» берётся из `State.notif_flip`, который и так переключается раз в секунду.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::feat::agents::{Agent, AgentState};

pub const W: i32 = 9;
pub const H: i32 = 8;

/// Тело: '#', глаз: 'e', рот: 'm'. Глаза и рот накладываются по состоянию.
const BODY: [&str; 8] = [
    "..#####..",
    ".#######.",
    "#########",
    "#########",
    "#########",
    "#########",
    "#########",
    "#.#.#.#.#",
];

const BODY_COLORS: [(u8, u8, u8); 6] = [
    (100, 210, 255), // голубой
    (191, 144, 255), // сиреневый
    (255, 140, 170), // розовый
    (125, 220, 160), // мятный
    (255, 205, 100), // песочный
    (150, 175, 255), // васильковый
];

/// Цвет тела по имени: один и тот же проект всегда одного цвета.
pub fn body_color(name: &str) -> COLORREF {
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    let (r, g, b) = BODY_COLORS[(h % BODY_COLORS.len() as u32) as usize];
    rgb(r, g, b)
}

/// Накладываемые пиксели лица: (x, y, '#' глаз | 'm' рот).
fn face(state: AgentState, flip: bool) -> Vec<(i32, i32)> {
    match state {
        // Глаза смотрят то влево, то вправо.
        AgentState::Work => {
            let dx = if flip { 1 } else { 0 };
            vec![(2 + dx, 3), (2 + dx, 4), (5 + dx, 3), (5 + dx, 4)]
        }
        // Круглые глаза и рот «о».
        AgentState::Ask => vec![(2, 3), (2, 4), (6, 3), (6, 4), (4, 6)],
        // Смотрит вбок, рта нет.
        AgentState::Wait => vec![(3, 3), (3, 4), (6, 3), (6, 4)],
        // Прищур и улыбка.
        AgentState::Done | AgentState::Gone => vec![(1, 4), (2, 4), (6, 4), (7, 4), (3, 6), (5, 6), (4, 7)],
    }
}

/// Рисует маскота с левым верхним углом (x, y); `px` — размер одного пикселя спрайта в физических px.
/// Все смещения физические: вызывающий уже пересчитал логические размеры через `Painter::s`.
pub unsafe fn draw(p: &Painter, x: i32, y: i32, px: i32, a: &Agent, flip: bool) {
    let body = body_color(&a.name);
    // Тот, кто ждёт человека или закончил, темнее: «не горит».
    let body = if matches!(a.state, AgentState::Done | AgentState::Wait) { dim(body) } else { body };
    for (row, line) in BODY.iter().enumerate() {
        let cols = line.as_bytes();
        let mut c = 0;
        while c < cols.len() {
            if cols[c] != b'#' {
                c += 1;
                continue;
            }
            let start = c;
            while c < cols.len() && cols[c] == b'#' {
                c += 1;
            }
            let r = RECT {
                left: x + start as i32 * px,
                top: y + row as i32 * px,
                right: x + c as i32 * px,
                bottom: y + (row as i32 + 1) * px,
            };
            fill_px(p, r, body);
        }
    }
    let ink = rgb(20, 20, 24);
    for (fx, fy) in face(a.state, flip) {
        let r = RECT { left: x + fx * px, top: y + fy * px, right: x + (fx + 1) * px, bottom: y + (fy + 1) * px };
        fill_px(p, r, ink);
    }
    // Значок состояния над правым плечом: «!» оранжевым, когда нужно разрешение.
    if a.state == AgentState::Ask {
        let r = RECT { left: x + 8 * px, top: y - px, right: x + 9 * px + px / 2, bottom: y + px / 2 };
        fill_px(p, r, WARN);
    }
}

fn dim(c: COLORREF) -> COLORREF {
    let (r, g, b) = (c.0 & 0xff, (c.0 >> 8) & 0xff, (c.0 >> 16) & 0xff);
    rgb((r * 6 / 10) as u8, (g * 6 / 10) as u8, (b * 6 / 10) as u8)
}

/// Прямоугольник в физических координатах без повторного масштабирования и сдвига `oy`/`ox` Painter'а.
unsafe fn fill_px(p: &Painter, r: RECT, c: COLORREF) {
    let _ = GdiFlush();
    p.cv.fill_round(RECT { left: r.left + p.ox, top: r.top + p.oy, right: r.right + p.ox, bottom: r.bottom + p.oy }, 0, c);
}

/// Какого агента показать в свёрнутой пилюле: сначала нужно разрешение, потом работающие.
pub fn lead(list: &[Agent]) -> Option<&Agent> {
    list.iter()
        .find(|a| a.state == AgentState::Ask)
        .or_else(|| list.iter().find(|a| a.state == AgentState::Work))
}
