//! Отрисовка GDI. Каждая отрисовка заодно записывает кликабельные зоны в State::hits.

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::*;

pub const BG: COLORREF = rgb(18, 18, 20);
pub const FG: COLORREF = rgb(235, 235, 240);
pub const DIM: COLORREF = rgb(140, 140, 150);
pub const ACCENT: COLORREF = rgb(120, 150, 255);
const PANEL: COLORREF = rgb(30, 30, 36);
const HOVER: COLORREF = rgb(42, 42, 52);
const ON: COLORREF = rgb(56, 78, 160);
const OFF_DIM: COLORREF = rgb(24, 24, 28);
const DANGER: COLORREF = rgb(150, 50, 55);

// Значки шрифта Segoe MDL2 Assets.
const I_PREV: &str = "\u{E892}";
const I_NEXT: &str = "\u{E893}";
const I_PLAY: &str = "\u{E768}";
const I_PAUSE: &str = "\u{E769}";
const I_VOL: &str = "\u{E767}";
const I_MUTE: &str = "\u{E74F}";
const I_WIFI: &str = "\u{E701}";
const I_BT: &str = "\u{E702}";
const I_VPN: &str = "\u{E705}";
const I_THEME: &str = "\u{E771}";
const I_LOCK: &str = "\u{E72E}";
const I_SLEEP: &str = "\u{E708}";
const I_RESTART: &str = "\u{E72C}";
const I_POWER: &str = "\u{E7E8}";
const I_PIN: &str = "\u{E718}";
const I_PINNED: &str = "\u{E840}";
const I_DEL: &str = "\u{E711}";
const I_SEARCH: &str = "\u{E721}";
const I_TIMER: &str = "\u{E916}";
const I_CALC: &str = "\u{E8EF}";
const I_APP: &str = "\u{E71D}";
const I_RUN: &str = "\u{E756}";
const I_MUSIC: &str = "\u{E8D6}";

pub const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

pub fn time_text() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:02}:{:02}", t.wHour, t.wMinute)
}

fn power() -> Option<SYSTEM_POWER_STATUS> {
    let mut ps = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut ps).ok()? };
    // 128 = батареи нет, 255 = неизвестно.
    (ps.BatteryFlag & 128 == 0 && ps.BatteryLifePercent <= 100).then_some(ps)
}

/// None, если батареи нет (стационарный ПК).
pub fn battery_text() -> Option<String> {
    let ps = power()?;
    let charging = if ps.ACLineStatus == 1 { "⚡" } else { "" };
    Some(format!("{}{}%", charging, ps.BatteryLifePercent))
}

fn battery_long() -> String {
    match power() {
        Some(ps) if ps.ACLineStatus == 1 => format!("Батарея {}%, заряжается", ps.BatteryLifePercent),
        Some(ps) => {
            let mut s = format!("Батарея {}%", ps.BatteryLifePercent);
            if ps.BatteryLifeTime != u32::MAX {
                let m = ps.BatteryLifeTime / 60;
                s += &format!(", осталось ≈ {} ч {} мин", m / 60, m % 60);
            }
            s
        }
        None => "Питание от сети".to_string(),
    }
}

struct Fonts {
    bold: HFONT,
    title: HFONT,
    normal: HFONT,
    small: HFONT,
    icon: HFONT,
    icon_big: HFONT,
}

impl Fonts {
    unsafe fn new(dpi: u32) -> Self {
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

unsafe fn font(dpi: u32, size: i32, bold: bool, face: PCWSTR) -> HFONT {
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

/// Ширина текста свёрнутой пилюли в пикселях.
pub unsafe fn measure(hwnd: HWND, dpi: u32, text: &str) -> i32 {
    let hdc = GetDC(hwnd);
    let f = font(dpi, 14, true, w!("Segoe UI"));
    let old = SelectObject(hdc, f);
    let mut sz = SIZE::default();
    let _ = GetTextExtentPoint32W(hdc, &wide(text), &mut sz);
    SelectObject(hdc, old);
    let _ = DeleteObject(f);
    ReleaseDC(hwnd, hdc);
    sz.cx
}

/// Сколько строк истории буфера влезает.
pub fn clip_rows(_dpi: u32) -> usize {
    ((EXPANDED_H - 14 - 94) / 28) as usize
}

struct Painter<'a> {
    hdc: HDC,
    dpi: u32,
    f: &'a Fonts,
    hits: Vec<(RECT, Hit)>,
    hover: Option<Hit>,
}

impl Painter<'_> {
    fn s(&self, v: i32) -> i32 {
        scale(v, self.dpi)
    }
    fn r(&self, l: i32, t: i32, rr: i32, b: i32) -> RECT {
        RECT { left: self.s(l), top: self.s(t), right: self.s(rr), bottom: self.s(b) }
    }
    unsafe fn fill(&self, r: RECT, radius: i32, c: COLORREF) {
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
    unsafe fn text(&self, font: HFONT, s: &str, r: RECT, c: COLORREF, flags: DRAW_TEXT_FORMAT) {
        // Пустой срез даёт DrawTextW висячий указатель — падение.
        if s.is_empty() {
            return;
        }
        SelectObject(self.hdc, font);
        SetTextColor(self.hdc, c);
        let mut buf = wide(s);
        let mut r = r;
        DrawTextW(self.hdc, &mut buf, &mut r, flags | DT_SINGLELINE | DT_NOPREFIX | DT_VCENTER);
    }
    fn hit(&mut self, r: RECT, h: Hit) {
        self.hits.push((r, h));
    }
    fn hovered(&self, h: Hit) -> bool {
        self.hover == Some(h)
    }
    /// Круглая кнопка-значок.
    unsafe fn icon_button(&mut self, r: RECT, icon: &str, h: Hit, big: bool) {
        if self.hovered(h) {
            self.fill(r, 40, HOVER);
        }
        let f = if big { self.f.icon_big } else { self.f.icon };
        self.text(f, icon, r, FG, DT_CENTER);
        self.hit(r, h);
    }
    /// Полоска громкости с процентами справа.
    unsafe fn volume_row(&mut self, x: i32, y: i32, right: i32, vol: Option<(f32, bool)>) {
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
    unsafe fn input(&mut self, w: i32, s: &State, placeholder: &str) {
        let r = RECT { left: self.s(20), top: self.s(56), right: w - self.s(20), bottom: self.s(86) };
        self.fill(r, 12, if s.input_active { HOVER } else { PANEL });
        self.text(self.f.icon, I_SEARCH, RECT { left: r.left + self.s(12), ..r }, DIM, DT_LEFT);
        let tr = RECT { left: r.left + self.s(36), right: r.right - self.s(12), ..r };
        if s.input.is_empty() {
            let ph = if s.input_active { "" } else { placeholder };
            self.text(self.f.normal, ph, tr, DIM, DT_LEFT | DT_END_ELLIPSIS);
            if s.input_active {
                self.text(self.f.normal, "|", tr, ACCENT, DT_LEFT);
            }
        } else {
            let caret = if s.input_active { "|" } else { "" };
            // Длинный ввод показываем хвостом.
            let align = if s.input.chars().count() > 55 { DT_RIGHT } else { DT_LEFT };
            self.text(self.f.normal, &format!("{}{}", s.input, caret), tr, FG, align);
        }
        self.hit(r, Hit::Input);
    }
}

/// Прямоугольник вкладки i в шапке развёрнутой панели.
fn tab_rect(i: usize, dpi: u32) -> RECT {
    let w = scale(110, dpi);
    let left = scale(20, dpi) + i as i32 * (w + scale(6, dpi));
    RECT { left, top: scale(14, dpi), right: left + w, bottom: scale(44, dpi) }
}

pub unsafe fn paint(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = BeginPaint(hwnd, &mut ps);
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);

    // Двойная буферизация, чтобы не мерцало.
    let mem = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, w, h);
    let old_bmp = SelectObject(mem, bmp);
    let old_font = GetCurrentObject(mem, OBJ_FONT);

    let bg = CreateSolidBrush(BG);
    FillRect(mem, &rc, bg);
    let _ = DeleteObject(bg);
    SetBkMode(mem, TRANSPARENT);

    st(|s| {
        let fonts = Fonts::new(s.dpi);
        let mut p = Painter { hdc: mem, dpi: s.dpi, f: &fonts, hits: Vec::new(), hover: s.hover };
        if !s.expanded {
            let color = if s.alarm.is_some() { ACCENT } else { FG };
            p.text(fonts.bold, &collapsed_text(s), rc, color, DT_CENTER);
        } else {
            for (i, (t, name)) in TABS.iter().enumerate() {
                let r = tab_rect(i, s.dpi);
                let h = Hit::Tab(*t);
                if *t == s.tab {
                    p.fill(r, 14, rgb(40, 40, 48));
                } else if p.hovered(h) {
                    p.fill(r, 14, PANEL);
                }
                p.text(fonts.bold, name, r, if *t == s.tab { FG } else { DIM }, DT_CENTER);
                p.hit(r, h);
            }
            let r = RECT { left: 0, top: scale(14, s.dpi), right: w - scale(20, s.dpi), bottom: scale(44, s.dpi) };
            p.text(fonts.bold, &time_text(), r, ACCENT, DT_RIGHT);

            if let Some(a) = &s.alarm {
                let body = RECT { left: 0, top: scale(50, s.dpi), right: w, bottom: h };
                p.text(fonts.title, &format!("⏰  {}", a), RECT { bottom: body.top + scale(150, s.dpi), ..body }, ACCENT, DT_CENTER);
                p.text(fonts.normal, "Клик, чтобы закрыть", RECT { top: body.top + scale(150, s.dpi), bottom: body.top + scale(190, s.dpi), ..body }, DIM, DT_CENTER);
                p.hit(body, Hit::Alarm);
            } else {
                match s.tab {
                    Tab::Music => music(&mut p, s, w, h),
                    Tab::Clipboard => clipboard(&mut p, s, w),
                    Tab::Commands => commands(&mut p, s, w),
                    Tab::Controls => controls(&mut p, s, w),
                }
            }
        }
        s.hits = std::mem::take(&mut p.hits);
        SelectObject(mem, old_font);
        drop(fonts);
    });

    let _ = BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
    SelectObject(mem, old_bmp);
    let _ = DeleteObject(bmp);
    let _ = DeleteDC(mem);
    let _ = EndPaint(hwnd, &ps);
}

unsafe fn music(p: &mut Painter, s: &State, w: i32, h: i32) {
    let m = media::MEDIA.lock().unwrap();
    if !m.has {
        let r = RECT { left: 0, top: p.s(50), right: w, bottom: h - p.s(20) };
        p.text(p.f.icon_big, I_MUSIC, RECT { bottom: r.top + p.s(120), ..r }, DIM, DT_CENTER);
        p.text(p.f.normal, "Сейчас ничего не играет", RECT { top: r.top + p.s(110), bottom: r.top + p.s(140), ..r }, DIM, DT_CENTER);
        drop(m);
        p.volume_row(24, 230, EXPANDED_W - 24, s.ctl.volume);
        hint(p, w, h, "Колесо мыши — громкость");
        return;
    }
    let art = p.r(24, 62, 156, 194);
    match &m.art {
        Some(a) => {
            let rg = CreateRoundRectRgn(art.left, art.top, art.right + 1, art.bottom + 1, p.s(18), p.s(18));
            SelectClipRgn(p.hdc, rg);
            SetStretchBltMode(p.hdc, HALFTONE);
            let _ = SetBrushOrgEx(p.hdc, 0, 0, None);
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
            StretchDIBits(
                p.hdc,
                art.left,
                art.top,
                art.right - art.left,
                art.bottom - art.top,
                0,
                0,
                a.w,
                a.h,
                Some(a.px.as_ptr() as *const _),
                &bmi,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            SelectClipRgn(p.hdc, None);
            let _ = DeleteObject(rg);
        }
        None => {
            p.fill(art, 18, PANEL);
            p.text(p.f.icon_big, I_MUSIC, art, DIM, DT_CENTER);
        }
    }
    let x = p.s(176);
    let right = w - p.s(24);
    p.text(p.f.small, &m.app, RECT { left: x, top: p.s(64), right, bottom: p.s(82) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    let title = if m.title.is_empty() { "Без названия" } else { &m.title };
    p.text(p.f.title, title, RECT { left: x, top: p.s(84), right, bottom: p.s(112) }, FG, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.normal, &m.artist, RECT { left: x, top: p.s(112), right, bottom: p.s(134) }, DIM, DT_LEFT | DT_END_ELLIPSIS);
    let playing = m.playing;
    drop(m);

    p.icon_button(p.r(170, 146, 214, 190), I_PREV, Hit::MediaPrev, false);
    let play = p.r(220, 142, 272, 194);
    p.fill(play, 52, rgb(44, 44, 54));
    p.icon_button(play, if playing { I_PAUSE } else { I_PLAY }, Hit::MediaPlay, true);
    p.icon_button(p.r(278, 146, 322, 190), I_NEXT, Hit::MediaNext, false);

    p.volume_row(24, 230, EXPANDED_W - 24, s.ctl.volume);
    hint(p, w, h, "Колесо мыши — громкость");
}

unsafe fn hint(p: &Painter, w: i32, h: i32, text: &str) {
    let r = RECT { left: p.s(20), top: h - p.s(40), right: w - p.s(20), bottom: h - p.s(16) };
    p.text(p.f.small, text, r, DIM, DT_CENTER);
}

/// Однострочное превью: переносы и табы заменяем на пробелы.
fn preview(t: &str) -> String {
    let mut out = String::new();
    for c in t.trim().chars().take(300) {
        out.push(if c.is_whitespace() { ' ' } else { c });
    }
    out
}

unsafe fn clipboard(p: &mut Painter, s: &State, w: i32) {
    p.input(w, s, "Поиск в истории (клик или Ctrl+Alt+V)");
    let view = s.clip.view(&s.input);
    if view.is_empty() {
        let msg = if s.clip.items.is_empty() { "История пуста — скопируй что-нибудь" } else { "Ничего не найдено" };
        p.text(p.f.normal, msg, RECT { left: 0, top: p.s(100), right: w, bottom: p.s(200) }, DIM, DT_CENTER);
        return;
    }
    let rows = clip_rows(s.dpi);
    for (n, &idx) in view.iter().enumerate().skip(s.scroll).take(rows) {
        let y = 94 + (n - s.scroll) as i32 * 28;
        let row = RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + 28) };
        let e = &s.clip.items[idx];
        let hov = p.hovered(Hit::ClipRow(idx)) || p.hovered(Hit::ClipPin(idx)) || p.hovered(Hit::ClipDel(idx));
        if hov || (s.input_active && s.sel == n) {
            p.fill(row, 10, HOVER);
        }
        p.hit(row, Hit::ClipRow(idx));
        let tr = RECT { left: row.left + p.s(12), right: row.right - p.s(64), ..row };
        p.text(p.f.normal, &preview(&e.text), tr, FG, DT_LEFT | DT_END_ELLIPSIS);

        let pin = RECT { left: row.right - p.s(60), right: row.right - p.s(32), ..row };
        let del = RECT { left: row.right - p.s(30), right: row.right - p.s(4), ..row };
        if e.pinned || hov {
            let c = if e.pinned { ACCENT } else if p.hovered(Hit::ClipPin(idx)) { FG } else { DIM };
            p.text(p.f.icon, if e.pinned { I_PINNED } else { I_PIN }, pin, c, DT_CENTER);
        }
        if hov {
            p.text(p.f.icon, I_DEL, del, if p.hovered(Hit::ClipDel(idx)) { FG } else { DIM }, DT_CENTER);
        }
        p.hit(pin, Hit::ClipPin(idx));
        p.hit(del, Hit::ClipDel(idx));
    }
    if view.len() > s.scroll + rows {
        let more = format!("ещё {} ↓", view.len() - s.scroll - rows);
        p.text(p.f.small, &more, RECT { left: 0, top: p.s(320), right: w - p.s(28), bottom: p.s(336) }, DIM, DT_RIGHT);
    }
}

unsafe fn commands(p: &mut Painter, s: &State, w: i32) {
    p.input(w, s, "Нажми сюда или Alt+Space");
    let res = cmd_results(s);
    if res.is_empty() {
        let lines = [
            "Программа: начни печатать название",
            "Калькулятор: 2+2*3, 15%*200, 2^10",
            "Таймер: «таймер 5м», «1ч30м», «90s»",
            "Команда: «> ipconfig», ссылка или путь",
        ];
        for (i, l) in lines.iter().enumerate() {
            let y = 108 + i as i32 * 28;
            p.text(p.f.normal, l, RECT { left: p.s(36), top: p.s(y), right: w - p.s(20), bottom: p.s(y + 28) }, DIM, DT_LEFT);
        }
        return;
    }
    let now = std::time::Instant::now();
    for (i, r) in res.iter().enumerate().take(7) {
        let y = 94 + i as i32 * 30;
        let row = RECT { left: p.s(20), top: p.s(y), right: w - p.s(20), bottom: p.s(y + 30) };
        let h = Hit::CmdRow(i);
        if p.hovered(h) || (s.input_active && s.sel == i) {
            p.fill(row, 10, HOVER);
        }
        p.hit(row, h);
        let (icon, label, right, color) = match r {
            CmdRes::Calc(v) => (I_CALC, format!("= {}", v), "Enter — скопировать", ACCENT),
            CmdRes::Timer(t) => (I_TIMER, format!("Таймер на {}", launcher::fmt_dur(*t)), "Enter — запустить", FG),
            CmdRes::App(a) => (I_APP, s.apps[*a].name.clone(), "программа", FG),
            CmdRes::Run(q) => (I_RUN, format!("Выполнить: {}", q), "команда", DIM),
            CmdRes::Active(t) => {
                let tm = &s.timers[*t];
                let left = tm.end.saturating_duration_since(now).as_secs();
                (I_TIMER, format!("Осталось {} из {}", launcher::fmt_dur(left), launcher::fmt_dur(tm.total)), "клик — отменить", FG)
            }
        };
        p.text(p.f.icon, icon, RECT { left: row.left + p.s(12), right: row.left + p.s(36), ..row }, DIM, DT_LEFT);
        p.text(p.f.normal, &label, RECT { left: row.left + p.s(40), right: row.right - p.s(150), ..row }, color, DT_LEFT | DT_END_ELLIPSIS);
        p.text(p.f.small, right, RECT { right: row.right - p.s(12), ..row }, DIM, DT_RIGHT);
    }
}

unsafe fn controls(p: &mut Painter, s: &State, w: i32) {
    let radios = *media::RADIOS.lock().unwrap();
    let vpn_label = match &s.ctl.vpn {
        Some((name, _)) => name.clone(),
        None => "VPN".to_string(),
    };
    let muted = s.ctl.volume.map(|v| v.1);
    let tiles: [(Ctl, &str, String, Option<bool>); 5] = [
        (Ctl::Wifi, I_WIFI, "Wi-Fi".into(), radios.wifi),
        (Ctl::Bt, I_BT, "Bluetooth".into(), radios.bt),
        (Ctl::Vpn, I_VPN, vpn_label, s.ctl.vpn.as_ref().map(|v| v.1)),
        (Ctl::Dark, I_THEME, "Тёмная тема".into(), s.ctl.dark),
        (Ctl::Mute, if muted == Some(true) { I_MUTE } else { I_VOL }, "Звук".into(), muted.map(|m| !m)),
    ];
    let gap = p.s(8);
    let left = p.s(20);
    let tw = (w - 2 * left - 4 * gap) / 5;
    for (i, (c, icon, label, state)) in tiles.iter().enumerate() {
        let x = left + i as i32 * (tw + gap);
        let r = RECT { left: x, top: p.s(58), right: x + tw, bottom: p.s(134) };
        let h = Hit::Toggle(*c);
        let bg = match state {
            Some(true) => ON,
            Some(false) if p.hovered(h) => HOVER,
            Some(false) => PANEL,
            None if p.hovered(h) => PANEL,
            None => OFF_DIM,
        };
        p.fill(r, 14, bg);
        let fg = if state.is_some() { FG } else { DIM };
        p.text(p.f.icon_big, icon, RECT { bottom: r.top + p.s(46), top: r.top + p.s(8), ..r }, fg, DT_CENTER);
        let lr = RECT { left: r.left + p.s(6), right: r.right - p.s(6), top: r.top + p.s(46), bottom: r.bottom - p.s(8) };
        p.text(p.f.small, label, lr, fg, DT_CENTER | DT_END_ELLIPSIS);
        p.hit(r, h);
    }

    p.volume_row(20, 150, EXPANDED_W - 20, s.ctl.volume);
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
        let h = Hit::Power(*pw);
        let confirming = s.confirm.map(|(c, _)| c == *pw).unwrap_or(false);
        p.fill(r, 14, if confirming { DANGER } else if p.hovered(h) { HOVER } else { PANEL });
        let label = if confirming { "Точно? Ещё раз" } else { label };
        p.text(p.f.icon, icon, RECT { left: r.left + p.s(12), right: r.left + p.s(36), ..r }, FG, DT_LEFT);
        p.text(p.f.small, label, RECT { left: r.left + p.s(36), right: r.right - p.s(6), ..r }, FG, DT_LEFT | DT_END_ELLIPSIS);
        p.hit(r, h);
    }
    hint(p, w, scale(EXPANDED_H, s.dpi), "Колесо мыши — громкость · Нет плитки? Клик откроет настройки");
}
