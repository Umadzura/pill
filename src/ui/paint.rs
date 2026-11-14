//! Общее для всей отрисовки: цвета, значки, шрифты, `Painter` и виджеты (кнопка, ползунок, поле ввода).
//! Весь текст рисуется только через `Painter::text`: там защита от пустой строки.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::surface::Canvas;
use crate::app::{scale, Hit, State};

// Палитра «чёрное стекло»: чёрный, белый и серые. Синего нет: главное и включённое — белое
// с чёрным текстом, цвет остаётся только у состояний (оранжевый, зелёный, красный) и маскотов.
pub const BG: COLORREF = rgb(0, 0, 0);
pub const FG: COLORREF = rgb(245, 245, 247);
pub const DIM: COLORREF = rgb(142, 142, 147);
/// Совсем тихий текст: неактивные вкладки, номера строк.
pub const FAINT: COLORREF = rgb(92, 92, 97);
/// Главное действие и «включено»: белый.
pub const ACCENT: COLORREF = FG;
/// Текст и значки поверх белого.
pub const INK: COLORREF = rgb(0, 0, 0);
pub const PANEL: COLORREF = rgb(20, 20, 21);
pub const HOVER: COLORREF = rgb(36, 36, 38);
pub const ON: COLORREF = ACCENT;
pub const OFF_DIM: COLORREF = rgb(18, 18, 19);
pub const DANGER: COLORREF = rgb(255, 69, 58);
pub const WARN: COLORREF = rgb(255, 159, 10);
pub const TRACK: COLORREF = rgb(54, 54, 57);
pub const MIC: COLORREF = rgb(255, 159, 10);
pub const CAM: COLORREF = rgb(48, 209, 88);

// Скругления (диаметр в логических px, как у прежнего CreateRoundRectRgn).
/// Плитки и панели.
pub const R_TILE: i32 = 28;
/// Строки списков.
pub const R_ROW: i32 = 18;
/// «Капсула»: вкладки, поле ввода — скругление на всю высоту.
pub const R_PILL: i32 = 60;

// Значки шрифта Segoe MDL2 Assets.
pub const I_PREV: &str = "\u{E892}";
pub const I_NEXT: &str = "\u{E893}";
pub const I_PLAY: &str = "\u{E768}";
pub const I_PAUSE: &str = "\u{E769}";
pub const I_VOL: &str = "\u{E767}";
pub const I_MUTE: &str = "\u{E74F}";
pub const I_WIFI: &str = "\u{E701}";
pub const I_BT: &str = "\u{E702}";
pub const I_VPN: &str = "\u{E705}";
pub const I_THEME: &str = "\u{E771}";
pub const I_LOCK: &str = "\u{E72E}";
pub const I_SLEEP: &str = "\u{E708}";
pub const I_RESTART: &str = "\u{E72C}";
pub const I_POWER: &str = "\u{E7E8}";
pub const I_PIN: &str = "\u{E718}";
pub const I_PINNED: &str = "\u{E840}";
pub const I_DEL: &str = "\u{E711}";
pub const I_SEARCH: &str = "\u{E721}";
pub const I_GEAR: &str = "\u{E713}";
pub const I_TIMER: &str = "\u{E916}";
pub const I_CALC: &str = "\u{E8EF}";
pub const I_APP: &str = "\u{E71D}";
pub const I_RUN: &str = "\u{E756}";
pub const I_MUSIC: &str = "\u{E8D6}";
pub const I_SHOT: &str = "\u{E8C6}"; // ножницы, как у «Ножниц» Windows
pub const I_BRIGHT: &str = "\u{E706}";
/// Громкость по уровням: 0, 1/3, 2/3, полная.
pub const I_VOL_LEVELS: [&str; 4] = ["\u{E992}", "\u{E993}", "\u{E994}", "\u{E995}"];

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub struct Fonts {
    /// Крупный заголовок: название трека, имя агента.
    pub hero: HFONT,
    pub bold: HFONT,
    pub title: HFONT,
    pub normal: HFONT,
    pub small: HFONT,
    pub icon: HFONT,
    pub icon_big: HFONT,
}

/// Шрифт интерфейса: Segoe UI Variable (Windows 11), на Windows 10 его нет — обычный Segoe UI.
pub fn face() -> PCWSTR {
    static HAS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    let has = *HAS.get_or_init(|| {
        let dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
        std::path::Path::new(&dir).join("Fonts").join("SegUIVar.ttf").exists()
    });
    if has {
        w!("Segoe UI Variable Text")
    } else {
        w!("Segoe UI")
    }
}

impl Fonts {
    pub unsafe fn new(dpi: u32) -> Self {
        Fonts {
            hero: font(dpi, 22, true, face()),
            bold: font(dpi, 14, true, face()),
            title: font(dpi, 17, true, face()),
            normal: font(dpi, 13, false, face()),
            small: font(dpi, 11, false, face()),
            icon: font(dpi, 14, false, w!("Segoe MDL2 Assets")),
            icon_big: font(dpi, 20, false, w!("Segoe MDL2 Assets")),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.hero, self.bold, self.title, self.normal, self.small, self.icon, self.icon_big] {
                let _ = DeleteObject(f);
            }
        }
    }
}

pub unsafe fn font(dpi: u32, size: i32, bold: bool, face: PCWSTR) -> HFONT {
    CreateFontW(
        -scale(size, dpi),
        0,
        0,
        0,
        if bold { FW_SEMIBOLD.0 as i32 } else { FW_NORMAL.0 as i32 },
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        CLEARTYPE_QUALITY.0 as u32,
        0,
        face,
    )
}

pub struct Painter<'a> {
    pub hdc: HDC,
    /// Пиксели того же буфера: сглаженные фигуры рисуются напрямую, текст — через GDI.
    pub cv: Canvas,
    /// Сдвиг содержимого вниз в пикселях (под переключателем подвкладок).
    pub oy: i32,
    /// Сдвиг содержимого вправо в пикселях: ушко слева и центрирование во время анимации размера.
    pub ox: i32,
    pub dpi: u32,
    pub f: &'a Fonts,
    pub hits: Vec<(RECT, Hit)>,
    pub hover: Option<Hit>,
    /// Нижний край содержимого вкладки в пикселях окна (по нему считается высота панели).
    pub bottom: i32,
    /// Где нарисованы полоски визуализатора в этом кадре.
    pub viz: Option<super::viz::VizArea>,
}

impl Painter<'_> {
    pub fn s(&self, v: i32) -> i32 {
        scale(v, self.dpi)
    }
    pub fn r(&self, l: i32, t: i32, rr: i32, b: i32) -> RECT {
        RECT { left: self.s(l), top: self.s(t), right: self.s(rr), bottom: self.s(b) }
    }
    /// Прямоугольник содержимого → пиксели окна.
    pub fn off(&self, r: RECT) -> RECT {
        RECT { left: r.left + self.ox, top: r.top + self.oy, right: r.right + self.ox, bottom: r.bottom + self.oy }
    }
    /// Обложка трека со сглаженными углами (radius — диаметр скругления в логических px).
    pub unsafe fn art(&self, r: RECT, a: &crate::bg::winrt::Art, radius: i32) {
        let r = self.off(r);
        SetStretchBltMode(self.hdc, HALFTONE);
        let _ = SetBrushOrgEx(self.hdc, 0, 0, None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: a.w,
                biHeight: -a.h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        StretchDIBits(self.hdc, r.left, r.top, r.right - r.left, r.bottom - r.top, 0, 0, a.w, a.h, Some(a.px.as_ptr() as *const _), &bmi, DIB_RGB_COLORS, SRCCOPY);
        // Скругляем уже нарисованное: сглаженно, без региона отсечения.
        let _ = GdiFlush();
        self.cv.round_off(r, self.s(radius), BG);
    }
    /// Сглаженный прямоугольник; radius — диаметр скругления в логических px (0 — прямые углы).
    pub unsafe fn fill(&self, r: RECT, radius: i32, c: COLORREF) {
        // GDI мог ещё не дописать текст в буфер: прямой доступ к пикселям только после сброса.
        let _ = GdiFlush();
        self.cv.fill_round(self.off(r), self.s(radius), c);
    }
    pub unsafe fn text(&self, font: HFONT, s: &str, r: RECT, c: COLORREF, flags: DRAW_TEXT_FORMAT) {
        // Пустой срез даёт DrawTextW висячий указатель — падение. Не убирать.
        if s.is_empty() {
            return;
        }
        SelectObject(self.hdc, font);
        SetTextColor(self.hdc, c);
        let mut buf = wide(s);
        let mut r = self.off(r);
        DrawTextW(self.hdc, &mut buf, &mut r, flags | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER);
    }
    /// Многострочный текст с переносом по словам, от верхнего края; что не влезло, обрезается.
    pub unsafe fn text_wrap(&self, font: HFONT, s: &str, r: RECT, c: COLORREF) {
        if s.is_empty() {
            return;
        }
        SelectObject(self.hdc, font);
        SetTextColor(self.hdc, c);
        let mut buf = wide(s);
        let mut r = self.off(r);
        DrawTextW(self.hdc, &mut buf, &mut r, DT_WORDBREAK | DT_EDITCONTROL | DT_NOPREFIX);
    }
    pub unsafe fn measure(&self, font: HFONT, s: &str) -> i32 {
        if s.is_empty() {
            return 0;
        }
        SelectObject(self.hdc, font);
        let mut sz = SIZE::default();
        let _ = GetTextExtentPoint32W(self.hdc, &wide(s), &mut sz);
        sz.cx
    }
    /// Содержимое доходит до y (логические px, от верха панели, с учётом сдвига `oy`).
    pub fn reach(&mut self, y: i32) {
        self.bottom = self.bottom.max(self.s(y) + self.oy);
    }
    pub fn hit(&mut self, r: RECT, h: Hit) {
        self.hits.push((self.off(r), h));
    }
    pub fn hovered(&self, h: Hit) -> bool {
        self.hover == Some(h)
    }
    /// Полоски визуализатора: рисуем и запоминаем место для покадровой перерисовки.
    /// `colors` — цвет полосок слева и справа (из обложки), между ними плавный переход.
    pub unsafe fn viz(&mut self, r: RECT, bars: usize, colors: (COLORREF, COLORREF)) {
        let r = self.off(r);
        let _ = GdiFlush();
        super::viz::draw(self.cv, self.dpi, r, bars, colors);
        self.viz = Some(super::viz::VizArea { rect: r, bars, colors });
    }
    /// Круглая кнопка-значок.
    pub unsafe fn icon_button(&mut self, r: RECT, icon: &str, h: Hit, big: bool) {
        if self.hovered(h) {
            self.fill(r, R_PILL, HOVER);
        }
        let f = if big { self.f.icon_big } else { self.f.icon };
        self.text(f, icon, r, FG, DT_CENTER);
        self.hit(r, h);
    }
    /// Полоска громкости с процентами справа.
    pub unsafe fn volume_row(&mut self, x: i32, y: i32, right: i32, vol: Option<(f32, bool)>) {
        let muted = vol.map(|v| v.1).unwrap_or(false);
        self.slider_row(x, y, right, if muted { I_MUTE } else { I_VOL }, vol.map(|v| v.0), muted, Hit::Volume);
    }
    /// Ползунок: значок, полоска, проценты. level None — значение неизвестно («—»). x, y, right в логических px.
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn slider_row(&mut self, x: i32, y: i32, right: i32, icon: &str, level: Option<f32>, dim: bool, h: Hit) {
        let lv = level.unwrap_or(0.0).clamp(0.0, 1.0);
        self.text(self.f.icon, icon, self.r(x, y, x + 24, y + 26), DIM, DT_LEFT);
        // Тонкая линия вместо толстой полосы.
        let bar = self.r(x + 32, y + 12, right - 44, y + 15);
        self.fill(bar, 3, HOVER);
        let mut filled = bar;
        filled.right = bar.left + ((bar.right - bar.left) as f32 * lv) as i32;
        if filled.right > filled.left {
            self.fill(filled, 3, if dim { DIM } else { ACCENT });
        }
        let pct = if level.is_some() { format!("{}", (lv * 100.0).round() as i32) } else { "—".into() };
        self.text(self.f.small, &pct, self.r(right - 40, y, right, y + 26), DIM, DT_RIGHT);
        // Зона клика выше и ниже полоски, чтобы легко попасть.
        self.hit(RECT { top: bar.top - self.s(10), bottom: bar.bottom + self.s(10), ..bar }, h);
    }
    /// Поле ввода (поиск в буфере / командная строка).
    pub unsafe fn input(&mut self, w: i32, s: &State, placeholder: &str) {
        let inp = &s.input;
        let r = RECT { left: self.s(20), top: self.s(56), right: w - self.s(20), bottom: self.s(86) };
        self.fill(r, R_PILL, if inp.active { HOVER } else { PANEL });
        self.text(self.f.icon, I_SEARCH, RECT { left: r.left + self.s(12), ..r }, DIM, DT_LEFT);
        let tr = RECT { left: r.left + self.s(36), right: r.right - self.s(12), ..r };
        if inp.text.is_empty() {
            let ph = if inp.active { "" } else { placeholder };
            self.text(self.f.normal, ph, tr, DIM, DT_LEFT | DT_END_ELLIPSIS);
            if inp.active {
                self.text(self.f.normal, "|", tr, ACCENT, DT_LEFT);
            }
        } else {
            let caret = if inp.active { "|" } else { "" };
            // Длинный ввод показываем хвостом.
            let align = if inp.text.chars().count() > 55 { DT_RIGHT } else { DT_LEFT };
            self.text(self.f.normal, &format!("{}{}", inp.text, caret), tr, FG, align);
        }
        self.hit(r, Hit::Input);
    }
}
