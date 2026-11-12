//! Общее для всей отрисовки: цвета, значки, шрифты, `Painter` и виджеты (кнопка, ползунок, поле ввода).
//! Весь текст рисуется только через `Painter::text`: там защита от пустой строки.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use crate::app::{scale, Hit, State};

pub const BG: COLORREF = rgb(18, 18, 20);
pub const FG: COLORREF = rgb(235, 235, 240);
pub const DIM: COLORREF = rgb(140, 140, 150);
pub const ACCENT: COLORREF = rgb(120, 150, 255);
pub const PANEL: COLORREF = rgb(30, 30, 36);
pub const HOVER: COLORREF = rgb(42, 42, 52);
pub const SELECTED_TAB: COLORREF = rgb(40, 40, 48);
pub const ON: COLORREF = rgb(56, 78, 160);
pub const OFF_DIM: COLORREF = rgb(24, 24, 28);
pub const DANGER: COLORREF = rgb(150, 50, 55);
pub const WARN: COLORREF = rgb(255, 150, 70);
pub const TRACK: COLORREF = rgb(52, 52, 60);
pub const MIC: COLORREF = rgb(255, 150, 70);
pub const CAM: COLORREF = rgb(80, 200, 120);

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
pub const I_TIMER: &str = "\u{E916}";
pub const I_CALC: &str = "\u{E8EF}";
pub const I_APP: &str = "\u{E71D}";
pub const I_RUN: &str = "\u{E756}";
pub const I_MUSIC: &str = "\u{E8D6}";
pub const I_SHOT: &str = "\u{E8C6}"; // ножницы, как у «Ножниц» Windows
/// Громкость по уровням: 0, 1/3, 2/3, полная.
pub const I_VOL_LEVELS: [&str; 4] = ["\u{E992}", "\u{E993}", "\u{E994}", "\u{E995}"];

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub struct Fonts {
    pub bold: HFONT,
    pub title: HFONT,
    pub normal: HFONT,
    pub small: HFONT,
    pub icon: HFONT,
    pub icon_big: HFONT,
}

impl Fonts {
    pub unsafe fn new(dpi: u32) -> Self {
        Fonts {
            bold: font(dpi, 14, true, w!("Segoe UI")),
            title: font(dpi, 17, true, w!("Segoe UI")),
            normal: font(dpi, 13, false, w!("Segoe UI")),
            small: font(dpi, 11, false, w!("Segoe UI")),
            icon: font(dpi, 14, false, w!("Segoe MDL2 Assets")),
            icon_big: font(dpi, 20, false, w!("Segoe MDL2 Assets")),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        unsafe {
            for f in [self.bold, self.title, self.normal, self.small, self.icon, self.icon_big] {
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
    pub dpi: u32,
    pub f: &'a Fonts,
    pub hits: Vec<(RECT, Hit)>,
    pub hover: Option<Hit>,
}

impl Painter<'_> {
    pub fn s(&self, v: i32) -> i32 {
        scale(v, self.dpi)
    }
    pub fn r(&self, l: i32, t: i32, rr: i32, b: i32) -> RECT {
        RECT { left: self.s(l), top: self.s(t), right: self.s(rr), bottom: self.s(b) }
    }
    pub unsafe fn fill(&self, r: RECT, radius: i32, c: COLORREF) {
        let br = CreateSolidBrush(c);
        if radius > 0 {
            let rad = self.s(radius);
            let rg = CreateRoundRectRgn(r.left, r.top, r.right + 1, r.bottom + 1, rad, rad);
            let _ = FillRgn(self.hdc, rg, br);
            let _ = DeleteObject(rg);
        } else {
            FillRect(self.hdc, &r, br);
        }
        let _ = DeleteObject(br);
    }
    pub unsafe fn text(&self, font: HFONT, s: &str, r: RECT, c: COLORREF, flags: DRAW_TEXT_FORMAT) {
        // Пустой срез даёт DrawTextW висячий указатель — падение. Не убирать.
        if s.is_empty() {
            return;
        }
        SelectObject(self.hdc, font);
        SetTextColor(self.hdc, c);
        let mut buf = wide(s);
        let mut r = r;
        DrawTextW(self.hdc, &mut buf, &mut r, flags | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER);
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
    pub fn hit(&mut self, r: RECT, h: Hit) {
        self.hits.push((r, h));
    }
    pub fn hovered(&self, h: Hit) -> bool {
        self.hover == Some(h)
    }
    /// Круглая кнопка-значок.
    pub unsafe fn icon_button(&mut self, r: RECT, icon: &str, h: Hit, big: bool) {
        if self.hovered(h) {
            self.fill(r, 40, HOVER);
        }
        let f = if big { self.f.icon_big } else { self.f.icon };
        self.text(f, icon, r, FG, DT_CENTER);
        self.hit(r, h);
    }
    /// Полоска громкости с процентами справа.
    pub unsafe fn volume_row(&mut self, x: i32, y: i32, right: i32, vol: Option<(f32, bool)>) {
        let (level, muted) = vol.unwrap_or((0.0, false));
        self.text(self.f.icon, if muted { I_MUTE } else { I_VOL }, self.r(x, y, x + 24, y + 26), DIM, DT_LEFT);
        let bar = self.r(x + 32, y + 10, right - 52, y + 16);
        self.fill(bar, 6, PANEL);
        let mut filled = bar;
        filled.right = bar.left + ((bar.right - bar.left) as f32 * level) as i32;
        if filled.right > filled.left {
            self.fill(filled, 6, if muted { DIM } else { ACCENT });
        }
        let pct = if vol.is_some() { format!("{}%", (level * 100.0).round() as i32) } else { "—".into() };
        self.text(self.f.normal, &pct, self.r(right - 46, y, right, y + 26), DIM, DT_RIGHT);
        // Зона клика выше и ниже полоски, чтобы легко попасть.
        self.hit(RECT { top: bar.top - self.s(10), bottom: bar.bottom + self.s(10), ..bar }, Hit::Volume);
    }
    /// Поле ввода (поиск в буфере / командная строка).
    pub unsafe fn input(&mut self, w: i32, s: &State, placeholder: &str) {
        let inp = &s.input;
        let r = RECT { left: self.s(20), top: self.s(56), right: w - self.s(20), bottom: self.s(86) };
        self.fill(r, 12, if inp.active { HOVER } else { PANEL });
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
    /// Серая подсказка внизу вкладки.
    pub unsafe fn hint(&self, w: i32, h: i32, text: &str) {
        let r = RECT { left: self.s(20), top: h - self.s(40), right: w - self.s(20), bottom: h - self.s(16) };
        self.text(self.f.small, text, r, DIM, DT_CENTER);
    }
}
