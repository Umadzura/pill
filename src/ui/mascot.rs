//! Маскот агента: свой пиксельный персонаж 10×9 с двумя ножками, рисуется прямоугольниками прямо в коде.
//! Цвет тела зависит от имени проекта (у каждого агента свой), состояние видно по лицу:
//! работает — зрачки бегают раз в секунду, нужно разрешение — круглые глаза, рот «о» и оранжевая
//! метка над плечом, ждёт ответа — смотрит в сторону, закончил — прищур и улыбка.
//! Таймера нет: «кадр» берётся из `State.notif_flip`, который и так переключается раз в секунду.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::feat::agents::{Agent, AgentState};

pub const W: i32 = 10;
pub const H: i32 = 9;

const BODY: [&str; 9] = [
    "..######..",
    ".########.",
    "##########",
    "##########",
    "##########",
    "##########",
    "##########",
    ".########.",
    ".##....##.",
];

/// Цвета тела. Синего, оранжевого, зелёного и красного здесь нет: они заняты под состояния.
const BODY_COLORS: [(u8, u8, u8); 6] = [
    (125, 122, 255), // фиолетово-синий
    (64, 200, 224),  // бирюзовый
    (255, 100, 130), // розовый
    (255, 214, 10),  // жёлтый
    (99, 230, 190),  // мятный
    (191, 90, 242),  // сиреневый
];

/// Цвет тела по имени: один и тот же проект всегда одного цвета.
pub fn body_color(name: &str) -> COLORREF {
    let h = name.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
    let (r, g, b) = BODY_COLORS[(h % BODY_COLORS.len() as u32) as usize];
    rgb(r, g, b)
}

const WHITE: COLORREF = rgb(255, 255, 255);
const INK: COLORREF = rgb(17, 17, 17);

/// Пиксели лица поверх тела: (x, y, цвет). Глаза стоят в столбцах 2–3 и 6–7.
fn face(state: AgentState, flip: bool) -> Vec<(i32, i32, COLORREF)> {
    let mut px = Vec::new();
    for (n, x) in [2, 6].into_iter().enumerate() {
        match state {
            // Белый глаз 2×2, зрачок слева или справа: у работающего бегает, у ждущего смотрит в сторону.
            AgentState::Work | AgentState::Wait => {
                let p = x + if state == AgentState::Wait || flip { 1 } else { 0 };
                px.extend([(x, 3, WHITE), (x + 1, 3, WHITE), (x, 4, WHITE), (x + 1, 4, WHITE), (p, 3, INK), (p, 4, INK)]);
            }
            // Круглые глаза на три строки, зрачки сведены к носу.
            AgentState::Ask => {
                for y in 2..=4 {
                    px.extend([(x, y, WHITE), (x + 1, y, WHITE)]);
                }
                px.push((x + if n == 0 { 1 } else { 0 }, 3, INK));
            }
            // Прищур.
            AgentState::Done | AgentState::Gone => px.extend([(x, 4, INK), (x + 1, 4, INK)]),
        }
    }
    match state {
        // Рот «о» и оранжевая метка над правым плечом.
        AgentState::Ask => px.extend([(4, 6, INK), (5, 6, INK), (4, 7, INK), (5, 7, INK), (8, 0, WARN), (9, 0, WARN), (9, 1, WARN)]),
        // Улыбка.
        AgentState::Done | AgentState::Gone => px.extend([(3, 6, INK), (6, 6, INK), (4, 7, INK), (5, 7, INK)]),
        _ => {}
    }
    px
}

/// Рисует маскота с левым верхним углом (x, y); `px` — размер одного пикселя спрайта в физических px.
/// Все смещения физические: вызывающий уже пересчитал логические размеры через `Painter::s`.
pub unsafe fn draw(p: &Painter, x: i32, y: i32, px: i32, a: &Agent, flip: bool) {
    let body = body_color(&a.name);
    // Тот, кто ждёт человека или закончил, темнее: «не горит».
    let body = if matches!(a.state, AgentState::Done | AgentState::Gone | AgentState::Wait) { dim(body) } else { body };
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
            let r = RECT { left: x + start as i32 * px, top: y + row as i32 * px, right: x + c as i32 * px, bottom: y + (row as i32 + 1) * px };
            fill_px(p, r, body);
        }
    }
    for (fx, fy, c) in face(a.state, flip) {
        fill_px(p, RECT { left: x + fx * px, top: y + fy * px, right: x + (fx + 1) * px, bottom: y + (fy + 1) * px }, c);
    }
}

fn dim(c: COLORREF) -> COLORREF {
    let (r, g, b) = (c.0 & 0xff, (c.0 >> 8) & 0xff, (c.0 >> 16) & 0xff);
    rgb((r * 55 / 100) as u8, (g * 55 / 100) as u8, (b * 55 / 100) as u8)
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
