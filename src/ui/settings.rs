//! Окно настроек: обычное окно Windows (можно двигать, есть на панели задач) в том же стиле, что
//! и пилюля, нарисованное тем же `Painter`. Слева разделы, справа строки «название — переключатель».
//! Источник правды — `%APPDATA%\Pill\settings.txt`, окно только меняет его и сразу сохраняет.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT};
use windows::Win32::UI::WindowsAndMessaging::*;

use super::paint::*;
use crate::app::{scale, st, Hit};
use crate::feat::hooks::{self, HookState};
use crate::feat::{notifs, settings::Settings};
use crate::ids::WM_MOUSELEAVE;
use crate::sys;

/// Размер содержимого окна (логические px).
const W: i32 = 740;
const H: i32 = 500;
const NAV_W: i32 = 190;
const ROW: i32 = 58;
const PAGES: [&str; 6] = ["Панель", "Агенты", "Уведомления", "Звуки", "Клавиши", "О программе"];
const DELAYS: [(u32, &str); 4] = [(200, "0,2 с"), (350, "0,35 с"), (600, "0,6 с"), (1000, "1 с")];
/// Фон окна чуть светлее чистого чёрного: отличается от рабочего стола и пилюли.
const BG_WIN: COLORREF = rgb(11, 11, 12);

/// Зоны клика окна настроек.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SetHit {
    Page(u8),
    /// Переключатель: 0 «просто точка», 1 автозапуск, 2 «не мешать в окне агента», 3 тихие часы, 4 звуки,
    /// 5 история буфера только в памяти.
    Toggle(u8),
    /// Задержка сворачивания (номер в DELAYS).
    Delay(u8),
    /// Кнопка: 0 поставить хуки, 1 убрать хуки, 2 доступ к уведомлениям, 3 баннеры Windows, 4 папка данных,
    /// 5 скопировать ключ API.
    Action(u8),
}

/// Открытое окно настроек.
pub struct SetWin {
    pub hwnd: isize,
    pub page: u8,
    pub hits: Vec<(RECT, Hit)>,
    pub hover: Option<Hit>,
    tracking: bool,
    /// Стоят ли хуки Claude Code (перечитывается при открытии раздела «Агенты» и после кнопок).
    hooks: Result<HookState, String>,
    /// Ключ API только что скопирован (надпись на кнопке, до смены раздела).
    copied: bool,
}

/// Состояние хуков с нашим ключом. Читает файл: звать вне `st()`.
fn hooks_status() -> Result<HookState, String> {
    let key = st(|s| s.settings.api_key.clone());
    hooks::status(&key)
}

static CLASS: AtomicBool = AtomicBool::new(false);

fn hwnd_of(s: &SetWin) -> HWND {
    HWND(s.hwnd as *mut _)
}

/// Открыть окно настроек (или поднять, если уже открыто).
pub unsafe fn open(main: HWND) {
    if let Some(h) = st(|s| s.setwin.as_ref().map(hwnd_of)) {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        sys::launch::force_foreground(h);
        return;
    }
    let inst = GetModuleHandleW(None).unwrap_or_default();
    let class = w!("PillSettings");
    if !CLASS.swap(true, Ordering::Relaxed) {
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: inst.into(),
            lpszClassName: class,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            hIcon: st(|s| s.shell.icon),
            ..Default::default()
        };
        RegisterClassW(&wc);
    }
    let dpi = st(|s| s.shell.dpi);
    let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
    let mut rc = RECT { left: 0, top: 0, right: scale(W, dpi), bottom: scale(H, dpi) };
    let _ = AdjustWindowRectEx(&mut rc, style, false, WINDOW_EX_STYLE(0));
    let (ow, oh) = (rc.right - rc.left, rc.bottom - rc.top);
    let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
    let Ok(h) = CreateWindowExW(WINDOW_EX_STYLE(0), class, w!("Pill — настройки"), style, (sw - ow) / 2, (sh - oh) / 2, ow, oh, None, None, inst, None) else {
        return;
    };
    // Тёмный заголовок окна, как у системных приложений в тёмной теме.
    let dark: BOOL = TRUE;
    let _ = DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const _ as *const _, std::mem::size_of::<BOOL>() as u32);
    let hooks = hooks_status();
    st(|s| s.setwin = Some(SetWin { hwnd: h.0 as isize, page: 0, hits: Vec::new(), hover: None, tracking: false, hooks, copied: false }));
    let _ = ShowWindow(h, SW_SHOW);
    sys::launch::force_foreground(h);
    let _ = main;
}

/// Открыть раздел (номер в PAGES) в уже открытом окне.
pub unsafe fn show_page(page: u8) {
    let h = st(|s| s.setwin.as_mut().map(|w| {
        w.page = page.min(PAGES.len() as u8 - 1);
        hwnd_of(w)
    }));
    if page == 1 {
        let hk = hooks_status();
        st(|s| {
            if let Some(w) = s.setwin.as_mut() {
                w.hooks = hk;
            }
        });
    }
    if let Some(h) = h {
        let _ = InvalidateRect(h, None, false);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            render(hwnd, hdc);
            let _ = EndPaint(hwnd, &ps);
        }
        WM_ERASEBKGND => return LRESULT(1),
        WM_MOUSEMOVE => {
            let (x, y) = ((lparam.0 & 0xFFFF) as i16 as i32, ((lparam.0 >> 16) & 0xFFFF) as i16 as i32);
            let (changed, arm) = st(|s| {
                let Some(w) = s.setwin.as_mut() else { return (false, false) };
                let h = w.hits.iter().rev().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|h| h.1);
                (std::mem::replace(&mut w.hover, h) != h, !std::mem::replace(&mut w.tracking, true))
            });
            if arm {
                let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                let _ = TrackMouseEvent(&mut tme);
            }
            if changed {
                let _ = InvalidateRect(hwnd, None, false);
            }
        }
        WM_MOUSELEAVE => {
            st(|s| {
                if let Some(w) = s.setwin.as_mut() {
                    w.hover = None;
                    w.tracking = false;
                }
            });
            let _ = InvalidateRect(hwnd, None, false);
        }
        WM_LBUTTONDOWN => {
            let (x, y) = ((lparam.0 & 0xFFFF) as i16 as i32, ((lparam.0 >> 16) & 0xFFFF) as i16 as i32);
            let hit = st(|s| s.setwin.as_ref().and_then(|w| w.hits.iter().rev().find(|(r, _)| x >= r.left && x < r.right && y >= r.top && y < r.bottom).map(|h| h.1)));
            if let Some(Hit::Set(h)) = hit {
                on_click(hwnd, h);
            }
        }
        WM_DESTROY => st(|s| s.setwin = None),
        _ => return DefWindowProcW(hwnd, msg, wparam, lparam),
    }
    LRESULT(0)
}

unsafe fn on_click(hwnd: HWND, h: SetHit) {
    let mut cfg = st(|s| s.settings.clone());
    let mut save = true;
    match h {
        SetHit::Page(i) => {
            save = false;
            let hooks = if i == 1 { Some(hooks_status()) } else { None };
            st(|s| {
                if let Some(w) = s.setwin.as_mut() {
                    w.page = i;
                    w.copied = false;
                    if let Some(hk) = hooks {
                        w.hooks = hk;
                    }
                }
            });
        }
        SetHit::Toggle(0) => cfg.dot_mode = !cfg.dot_mode,
        SetHit::Toggle(1) => {
            save = false;
            sys::autostart::set(!sys::autostart::enabled());
        }
        SetHit::Toggle(2) => cfg.skip_focused = !cfg.skip_focused,
        SetHit::Toggle(3) => cfg.quiet = if cfg.quiet.is_some() { None } else { Some((23, 8)) },
        SetHit::Toggle(5) => {
            // Файл удаляется и пишется здесь, вне `st()`; в `st()` только флаг и снимок истории.
            cfg.clip_memory = !cfg.clip_memory;
            let disk = !cfg.clip_memory;
            let text = st(|s| {
                s.clip.disk = disk;
                disk.then(|| s.clip.serialize())
            });
            match text {
                // Вернули историю на диск: то, что накопилось в памяти, не теряется.
                Some(t) => crate::feat::clip::write_file(&t),
                None => crate::feat::clip::remove_file(),
            }
        }
        SetHit::Toggle(_) => cfg.sound = !cfg.sound,
        SetHit::Delay(i) => cfg.collapse_ms = DELAYS[(i as usize).min(DELAYS.len() - 1)].0,
        SetHit::Action(a) => {
            save = false;
            match a {
                0 | 1 => {
                    let key = st(|s| s.settings.api_key.clone());
                    let res = hooks::apply(a == 0, &key).and_then(|_| hooks::status(&key));
                    st(|s| {
                        if let Some(w) = s.setwin.as_mut() {
                            w.hooks = res;
                        }
                    });
                }
                5 => {
                    // Ключ для своих скриптов: curl -H "X-Pill-Key: …" или параметр key=.
                    let key = st(|s| s.settings.api_key.clone());
                    let ok = !key.is_empty() && sys::clipboard::set_secret(hwnd, &key);
                    st(|s| {
                        if let Some(w) = s.setwin.as_mut() {
                            w.copied = ok;
                        }
                    });
                }
                2 => {
                    sys::launch::open("ms-settings:privacy-notifications", None);
                }
                3 => {
                    sys::launch::open("ms-settings:notifications", None);
                }
                _ => {
                    if let Ok(dir) = std::env::var("APPDATA") {
                        let p = std::path::Path::new(&dir).join("Pill");
                        let _ = std::fs::create_dir_all(&p);
                        sys::launch::open(&p.to_string_lossy(), None);
                    }
                }
            }
        }
    }
    if save {
        st(|s| s.settings = cfg.clone());
        cfg.save();
    }
    let _ = InvalidateRect(hwnd, None, false);
}

// ---------------- отрисовка ----------------

unsafe fn render(hwnd: HWND, hdc: HDC) {
    let mut rc = RECT::default();
    let _ = GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);
    if w <= 0 || h <= 0 {
        return;
    }
    let dpi = GetDpiForWindow(hwnd).max(96);
    super::surface::offscreen(w, h, |sf| {
        let mem = sf.dc;
        let cv = sf.canvas();
        cv.clear(BG_WIN);
        SetBkMode(mem, TRANSPARENT);
        let fonts = Fonts::new(dpi);
        let old_font = GetCurrentObject(mem, OBJ_FONT);
        st(|s| {
            let Some(win) = s.setwin.as_ref() else { return };
            let mut p = Painter { hdc: mem, cv, oy: 0, ox: 0, dpi, f: &fonts, hits: Vec::new(), hover: win.hover, bottom: 0, viz: None };
            nav(&mut p, win.page, h);
            match win.page {
                0 => page_panel(&mut p, &s.settings),
                1 => page_agents(&mut p, &s.settings, &win.hooks, win.copied),
                2 => page_notifs(&mut p, &s.settings),
                3 => page_sounds(&mut p, &s.settings),
                4 => page_keys(&mut p),
                _ => page_about(&mut p),
            }
            let hits = std::mem::take(&mut p.hits);
            if let Some(win) = s.setwin.as_mut() {
                win.hits = hits;
            }
        });
        SelectObject(mem, old_font);
        drop(fonts);
        let _ = GdiFlush();
        let _ = BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);
    });
}

/// Левая колонка: заголовок и разделы; справа от неё тонкая черта.
unsafe fn nav(p: &mut Painter, page: u8, h: i32) {
    p.text(p.f.title, "Настройки", p.r(26, 22, NAV_W, 50), FG, DT_LEFT);
    for (i, name) in PAGES.iter().enumerate() {
        let y = 66 + i as i32 * 36;
        let r = p.r(14, y, NAV_W - 14, y + 32);
        let hit = Hit::Set(SetHit::Page(i as u8));
        let on = page == i as u8;
        if on {
            p.fill(r, R_ROW, HOVER);
        } else if p.hovered(hit) {
            p.fill(r, R_ROW, PANEL);
        }
        p.text(p.f.normal, name, RECT { left: r.left + p.s(12), ..r }, if on { FG } else { DIM }, DT_LEFT);
        p.hit(r, hit);
    }
    p.fill(RECT { left: p.s(NAV_W), top: 0, right: p.s(NAV_W) + p.s(1).max(1), bottom: h }, 0, HOVER);
}

/// Заголовок раздела. Возвращает y первой строки.
unsafe fn title(p: &Painter, t: &str) -> i32 {
    p.text(p.f.hero, t, p.r(NAV_W + 28, 20, W - 28, 52), FG, DT_LEFT);
    64
}

/// Строка: название и пояснение слева, черта снизу. `reserve` — сколько места справа занимает
/// управление (логические px). Возвращает прямоугольник для управления справа.
unsafe fn row(p: &Painter, y: i32, label: &str, sub: &str, reserve: i32) -> RECT {
    let (l, r) = (NAV_W + 28, W - 28);
    p.text(p.f.normal, label, p.r(l, y + 9, r - reserve - 16, y + 29), FG, DT_LEFT | DT_END_ELLIPSIS);
    p.text(p.f.small, sub, p.r(l, y + 29, r - reserve - 16, y + 47), DIM, DT_LEFT | DT_END_ELLIPSIS);
    p.fill(RECT { left: p.s(l), top: p.s(y + ROW - 1), right: p.s(r), bottom: p.s(y + ROW - 1) + p.s(1).max(1) }, 0, PANEL);
    p.r(r - 330, y, r, y + ROW - 1)
}

/// Переключатель у правого края области.
unsafe fn toggle(p: &mut Painter, area: RECT, on: bool, id: u8) {
    let (tw, th) = (p.s(38), p.s(22));
    let mid = (area.top + area.bottom) / 2;
    let r = RECT { left: area.right - tw, top: mid - th / 2, right: area.right, bottom: mid - th / 2 + th };
    p.fill(r, R_PILL, if on { FG } else { TRACK });
    let k = th - p.s(4);
    let kx = if on { r.right - p.s(2) - k } else { r.left + p.s(2) };
    p.fill(RECT { left: kx, top: r.top + p.s(2), right: kx + k, bottom: r.top + p.s(2) + k }, R_PILL, if on { INK } else { rgb(199, 199, 204) });
    let hit = Hit::Set(SetHit::Toggle(id));
    p.hit(RECT { left: r.left - p.s(6), right: r.right + p.s(4), top: area.top, bottom: area.bottom }, hit);
}

/// Кнопка-капсула, правым краем к `right`. Возвращает левый край.
unsafe fn button(p: &mut Painter, right: i32, area: RECT, label: &str, hit: SetHit, primary: bool) -> i32 {
    let w = p.measure(p.f.bold, label) + p.s(28);
    let mid = (area.top + area.bottom) / 2;
    let r = RECT { left: right - w, top: mid - p.s(15), right, bottom: mid + p.s(15) };
    let h = Hit::Set(hit);
    let bg = match (primary, p.hovered(h)) {
        (true, false) => FG,
        (true, true) => rgb(220, 220, 224),
        (false, false) => HOVER,
        (false, true) => TRACK,
    };
    p.fill(r, R_PILL, bg);
    p.text(p.f.bold, label, r, if primary { INK } else { FG }, DT_CENTER);
    p.hit(r, h);
    r.left
}

/// Клавиши справа: «Ctrl» «Alt» «H» в рамках.
unsafe fn keys(p: &Painter, area: RECT, ks: &[&str]) {
    let mid = (area.top + area.bottom) / 2;
    let mut x = area.right;
    for k in ks.iter().rev() {
        let w = p.measure(p.f.small, k) + p.s(16);
        let r = RECT { left: x - w, top: mid - p.s(12), right: x, bottom: mid + p.s(12) };
        p.fill(r, 12, HOVER);
        p.text(p.f.small, k, r, FG, DT_CENTER);
        x = r.left - p.s(6);
    }
}

unsafe fn page_panel(p: &mut Painter, cfg: &Settings) {
    let mut y = title(p, "Панель");
    let cur = DELAYS.iter().find(|d| d.0 == cfg.collapse_ms).map(|d| d.1.to_string()).unwrap_or_else(|| format!("{} мс", cfg.collapse_ms));
    let area = row(p, y, "Сворачиваться после ухода курсора", &format!("сейчас {}", cur), 200);
    let mut x = area.right;
    for (i, (ms, label)) in DELAYS.iter().enumerate().rev() {
        let w = p.measure(p.f.normal, label);
        let r = RECT { left: x - w, right: x, ..area };
        let hit = Hit::Set(SetHit::Delay(i as u8));
        let on = cfg.collapse_ms == *ms;
        p.text(p.f.normal, label, r, if on { FG } else if p.hovered(hit) { DIM } else { FAINT }, DT_LEFT);
        p.hit(RECT { left: r.left - p.s(5), right: r.right + p.s(5), ..r }, hit);
        x = r.left - p.s(16);
    }
    y += ROW;
    let area = row(p, y, "Просто точка", "карточки не выезжают, о событиях говорит свёрнутая чёлка", 50);
    toggle(p, area, cfg.dot_mode, 0);
    y += ROW;
    let area = row(p, y, "Запускать вместе с Windows", "Pill стартует сам после входа в систему", 50);
    toggle(p, area, sys::autostart::enabled(), 1);
    y += ROW;
    let area = row(p, y, "Не хранить историю буфера на диске", "только в памяти до выхода, clipboard.txt удаляется", 50);
    toggle(p, area, cfg.clip_memory, 5);
    y += ROW;
    let area = row(p, y, "Спрятать и вернуть пилюлю", "чтобы нажать то, что под ней", 130);
    keys(p, area, &["Ctrl", "Alt", "H"]);
}

unsafe fn page_agents(p: &mut Painter, cfg: &Settings, hooks_state: &Result<HookState, String>, copied: bool) {
    let mut y = title(p, "Агенты");
    let (sub, state) = match hooks_state {
        Ok(HookState::On) => ("установлены".to_string(), HookState::On),
        // Стоят от прежней версии (без ключа API или с другим): /hook им отвечает 403.
        Ok(HookState::Stale) => ("нужно переустановить: стоят от старой версии".to_string(), HookState::Stale),
        Ok(HookState::Off) => ("не установлены — без них агенты не появятся в Pill".to_string(), HookState::Off),
        Err(e) => (e.clone(), HookState::Off),
    };
    let area = row(p, y, "Хуки Claude Code", &sub, 250);
    if state == HookState::Off {
        button(p, area.right, area, "Установить", SetHit::Action(0), true);
    } else {
        let l = button(p, area.right, area, "Убрать", SetHit::Action(1), false);
        button(p, l - p.s(8), area, "Переустановить", SetHit::Action(0), state == HookState::Stale);
    }
    let note = p.r(NAV_W + 28, y + ROW + 4, W - 28, y + ROW + 22);
    p.text(p.f.small, "Пишет в ~/.claude/settings.json, прежний файл сохраняется рядом как settings.json.pill-backup.", note, FAINT, DT_LEFT | DT_END_ELLIPSIS);
    let note2 = RECT { top: note.bottom, bottom: note.bottom + p.s(18), ..note };
    p.text(p.f.small, "Свои записи Pill узнаёт по адресу 127.0.0.1:47821, чужие хуки не трогает.", note2, FAINT, DT_LEFT | DT_END_ELLIPSIS);
    y += ROW + 48;
    // Порт занят (второй Pill или чужая программа): ни хуки, ни скрипты до Pill не дойдут.
    let (label, sub) = if crate::bg::api::DOWN.load(Ordering::Relaxed) {
        ("API не поднялся: порт 47821 занят", "агенты и скрипты не дойдут до Pill; освободи порт и перезапусти Pill")
    } else {
        ("Локальный API", "без ключа хуки и скрипты получают 403: заголовок X-Pill-Key или key=")
    };
    let area = row(p, y, label, sub, 170);
    if !cfg.api_key.is_empty() {
        button(p, area.right, area, if copied { "Скопирован" } else { "Скопировать ключ" }, SetHit::Action(5), false);
    }
    y += ROW;
    let area = row(p, y, "Не мешать в окне агента", "без карточек, если терминал или редактор этого проекта уже активен", 50);
    toggle(p, area, cfg.skip_focused, 2);
    y += ROW;
    row(p, y, "Ответ на просьбу ждёт 30 секунд", "потом вопрос уходит в окно агента, как будто Pill нет", 0);
}

unsafe fn page_notifs(p: &mut Painter, cfg: &Settings) {
    let mut y = title(p, "Уведомления");
    let off = notifs::OFF.load(Ordering::Relaxed);
    let area = row(p, y, "Доступ к уведомлениям Windows", if off { "нет доступа — сообщения не появятся в пилюле" } else { "есть: Pill читает центр уведомлений" }, 170);
    button(p, area.right, area, "Открыть параметры", SetHit::Action(2), off);
    y += ROW;
    let area = row(p, y, "Баннеры Windows", "выключи их, чтобы сообщения не дублировались", 200);
    button(p, area.right, area, "Параметры уведомлений", SetHit::Action(3), false);
    y += ROW;
    let area = row(p, y, "Тихие часы 23:00–8:00", "ни звуков, ни карточек; счётчик пропущенных растёт как обычно", 50);
    toggle(p, area, cfg.quiet.is_some(), 3);
}

unsafe fn page_sounds(p: &mut Painter, cfg: &Settings) {
    let mut y = title(p, "Звуки");
    let area = row(p, y, "Звуки", "таймер, просьба агента, «агент готов» — системными звуками Windows", 50);
    toggle(p, area, cfg.sound, 4);
    y += ROW;
    row(p, y, "Тихие часы", if cfg.quiet.is_some() { "включены: 23:00–8:00 без звуков" } else { "выключены, включаются в разделе «Уведомления»" }, 0);
}

unsafe fn page_keys(p: &mut Painter) {
    let mut y = title(p, "Клавиши");
    for (label, sub, ks) in [
        ("Поиск", "программы, буфер, расчёт, таймеры", &["Alt", "Space"][..]),
        ("Буфер обмена", "сразу режим «Буфер»", &["Ctrl", "Alt", "V"][..]),
        ("Спрятать пилюлю", "и вернуть тем же сочетанием", &["Ctrl", "Alt", "H"][..]),
        ("Ответ агенту", "разрешить, нет, всегда, в терминал — пока на экране просьба", &["Alt Y", "Alt N", "Alt A", "Alt T"][..]),
        ("Вариант ответа", "пока на экране вопрос агента", &["Alt 1…4"][..]),
    ] {
        let area = row(p, y, label, sub, if ks.len() > 2 { 190 } else { 120 });
        keys(p, area, ks);
        y += ROW;
    }
}

unsafe fn page_about(p: &mut Painter) {
    let mut y = title(p, &format!("Pill {}", env!("CARGO_PKG_VERSION")));
    p.text(p.f.normal, "Лёгкая «островная» панель для Windows: музыка, буфер, агенты, уведомления.", p.r(NAV_W + 28, y, W - 28, y + 22), DIM, DT_LEFT | DT_END_ELLIPSIS);
    y += 36;
    let data = std::env::var("APPDATA").map(|a| format!("{}\\Pill", a)).unwrap_or_default();
    let area = row(p, y, "Папка с данными", &data, 100);
    button(p, area.right, area, "Открыть", SetHit::Action(4), false);
    y += ROW;
    let size = std::env::current_exe().ok().and_then(|e| std::fs::metadata(e).ok()).map(|m| format!("{} КБ, один файл без установщика", m.len() / 1024)).unwrap_or_default();
    row(p, y, "Размер программы", &size, 0);
}
