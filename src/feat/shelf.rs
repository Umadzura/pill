//! Полка: файлы, брошенные на пилюлю. Сами файлы не копируем, храним пути в `%APPDATA%\Pill\shelf.txt`.
//! Картинкам миниатюру декодирует воркер (`winrt::Job::Thumb`), остальным подпись по расширению.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Input::KeyboardAndMouse::DragDetect;

use crate::app::{st, Redraw, Tab};
use crate::bg::winrt::{self, Art};
use crate::sys;

/// Сколько файлов держит полка (две строки по четыре).
pub const MAX: usize = 8;

pub struct Item {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub dir: bool,
    pub image: bool,
    /// Миниатюра и размер картинки в пикселях (когда воркер её декодировал).
    pub thumb: Option<Arc<Art>>,
    pub dims: Option<(u32, u32)>,
    /// Миниатюру уже просили (чтобы не просить повторно, даже если не вышло).
    pub asked: bool,
}

impl Item {
    /// Описать файл по пути (читает метаданные: вне `st()`). None — файла нет.
    pub fn probe(path: &str) -> Option<Item> {
        let md = std::fs::metadata(path).ok()?;
        let name = Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string());
        let dir = md.is_dir();
        Some(Item { path: path.to_string(), image: !dir && is_image(&name), name, size: md.len(), dir, thumb: None, dims: None, asked: false })
    }

    /// Подпись под именем: размер картинки, иначе размер файла.
    pub fn caption(&self) -> String {
        match (self.dir, self.dims) {
            (true, _) => "папка".into(),
            (_, Some((w, h))) => format!("{}×{}", w, h),
            _ => size_text(self.size),
        }
    }

    /// Надпись на плитке без миниатюры: расширение крупно.
    pub fn label(&self) -> String {
        if self.dir {
            return "ПАПКА".into();
        }
        match Path::new(&self.name).extension() {
            Some(e) => e.to_string_lossy().chars().take(5).collect::<String>().to_uppercase(),
            None => "ФАЙЛ".into(),
        }
    }
}

#[derive(Default)]
pub struct Shelf {
    pub items: Vec<Item>,
    /// Над окном держат файл: свёрнутая пилюля говорит «Отпусти: на полку».
    pub over: bool,
}

impl Shelf {
    pub fn load() -> Self {
        let text = file().and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
        let items = text.lines().filter(|l| !l.trim().is_empty()).filter_map(Item::probe).take(MAX).collect();
        Shelf { items, over: false }
    }

    /// Положить новые файлы в начало (повторы поднимаются наверх). Возвращает, сколько положено.
    pub fn add(&mut self, new: Vec<Item>) -> usize {
        let n = new.len();
        for it in new.into_iter().rev() {
            if let Some(i) = self.items.iter().position(|x| x.path.eq_ignore_ascii_case(&it.path)) {
                let old = self.items.remove(i);
                self.items.insert(0, old);
            } else {
                self.items.insert(0, it);
            }
        }
        self.items.truncate(MAX);
        n
    }

    pub fn remove(&mut self, i: usize) {
        if i < self.items.len() {
            self.items.remove(i);
        }
    }

    pub fn serialize(&self) -> String {
        self.items.iter().map(|i| format!("{}\n", i.path)).collect()
    }
}

fn file() -> Option<PathBuf> {
    std::env::var("APPDATA").ok().map(|d| PathBuf::from(d).join("Pill").join("shelf.txt"))
}

/// Записать список (вне `st()`).
fn save(text: String) {
    if let Some(p) = file() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, text);
    }
}

pub fn is_image(name: &str) -> bool {
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "bmp" | "gif" | "tif" | "tiff" | "ico" | "webp" | "heic" | "jxr")
}

/// «840 КБ», «2,1 МБ», «12 Б».
pub fn size_text(b: u64) -> String {
    const K: f64 = 1024.0;
    let f = b as f64;
    let one = |v: f64, unit: &str| {
        if v < 10.0 {
            format!("{:.1} {}", v, unit).replace('.', ",")
        } else {
            format!("{:.0} {}", v, unit)
        }
    };
    if f < K {
        format!("{} Б", b)
    } else if f < K * K {
        one(f / K, "КБ")
    } else if f < K * K * K {
        one(f / K / K, "МБ")
    } else {
        one(f / K / K / K, "ГБ")
    }
}

/// Попросить воркер сделать миниатюры картинкам, у которых их нет.
pub fn request_thumbs() {
    let want: Vec<String> = st(|s| {
        s.shelf
            .items
            .iter_mut()
            .filter(|i| i.image && !i.asked)
            .map(|i| {
                i.asked = true;
                i.path.clone()
            })
            .collect()
    });
    for p in want {
        winrt::send(winrt::Job::Thumb(p));
    }
}

/// Воркер прислал миниатюры (`WM_APP_THUMB`).
pub unsafe fn on_thumbs(hwnd: HWND) {
    let got = std::mem::take(&mut *winrt::THUMBS.lock().unwrap());
    let shown = st(|s| {
        for (path, t) in got {
            if let Some(it) = s.shelf.items.iter_mut().find(|i| i.path == path) {
                if let Some((art, w, h)) = t {
                    it.thumb = Some(Arc::new(art));
                    it.dims = Some((w, h));
                }
            }
        }
        s.shell.expanded && s.shell.tab == Tab::Shelf
    });
    if shown {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(hwnd, None, false);
    }
}

/// Файл над окном, ушёл или брошен (`WM_APP_DROP`, см. `sys::dragdrop`).
pub unsafe fn on_drop(hwnd: HWND, what: usize) {
    match what {
        sys::dragdrop::ENTER | sys::dragdrop::LEAVE => {
            let over = what == sys::dragdrop::ENTER;
            if st(|s| std::mem::replace(&mut s.shelf.over, over) != over) {
                crate::shell::layout(hwnd);
            }
        }
        _ => {
            let paths = std::mem::take(&mut *sys::dragdrop::DROPPED.lock().unwrap());
            st(|s| s.shelf.over = false);
            add_paths(hwnd, paths);
        }
    }
}

/// Положить файлы на полку (бросок, кнопка «На полку» у снимка): записать список, попросить
/// миниатюры; свёрнутой пилюлей сказать «На полке: имя». Читает метаданные: вне `st()`.
pub unsafe fn add_paths(hwnd: HWND, paths: Vec<String>) {
    let items: Vec<Item> = paths.iter().filter_map(|p| Item::probe(p)).collect();
    let first = items.first().map(|i| i.name.clone());
    let (n, text, expanded) = st(|s| {
        let n = s.shelf.add(items);
        // Следующее раскрытие покажет полку (если не идёт поиск).
        if n > 0 && !s.input.active {
            s.shell.tab = Tab::Shelf;
        }
        (n, s.shelf.serialize(), s.shell.expanded)
    });
    if n == 0 {
        crate::shell::layout(hwnd);
        return;
    }
    save(text);
    request_thumbs();
    if expanded {
        crate::shell::layout(hwnd);
    } else {
        let msg = match (n, first) {
            (1, Some(name)) => format!("На полке: {}", name),
            _ => format!("На полке: {} файла", n),
        };
        crate::feat::osd::show(hwnd, crate::feat::osd::Flash::Text(msg, false), 2000);
    }
}

/// Нажали на плитку: потащили — отдаём файл в другое окно, просто щёлкнули — открываем.
pub unsafe fn on_press(hwnd: HWND, i: usize, x: i32, y: i32) -> Redraw {
    let Some(path) = st(|s| s.shelf.items.get(i).map(|it| it.path.clone())) else { return Redraw::None };
    let mut pt = POINT { x, y };
    let _ = ClientToScreen(hwnd, &mut pt);
    if DragDetect(hwnd, pt).as_bool() {
        sys::dragdrop::drag_out(hwnd, &path);
        // Курсор ушёл в другое окно, «ушёл» от системы могло не прийти.
        crate::shell::schedule_collapse(hwnd);
        return Redraw::Content;
    }
    if Path::new(&path).exists() {
        crate::input::close_panel(hwnd, false);
        sys::launch::open(&path, None);
        Redraw::None
    } else {
        // Файл удалили или перенесли: с полки тоже убираем.
        remove(i);
        Redraw::Content
    }
}

/// Убрать с полки (крестик). Сам файл не трогаем.
pub fn remove(i: usize) {
    let text = st(|s| {
        s.shelf.remove(i);
        s.shelf.serialize()
    });
    save(text);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(p: &str) -> Item {
        Item { path: p.into(), name: p.into(), size: 0, dir: false, image: false, thumb: None, dims: None, asked: false }
    }

    #[test]
    fn shelf_order_and_text() {
        let mut s = Shelf::default();
        s.add(vec![item("a"), item("b")]);
        assert_eq!(s.serialize(), "a\nb\n");
        // Повтор поднимается наверх, а не дублируется.
        s.add(vec![item("B")]);
        assert_eq!(s.serialize(), "b\na\n");
        s.add((0..10).map(|i| item(&format!("f{}", i))).collect());
        assert_eq!(s.items.len(), MAX);
        assert_eq!(s.items[0].path, "f0");
        assert_eq!((size_text(12), size_text(860_160), size_text(2_202_009), size_text(193_000_000)), ("12 Б".into(), "840 КБ".into(), "2,1 МБ".into(), "184 МБ".into()));
        assert!(is_image("Снимок.PNG") && !is_image("отчёт.pdf"));
        let mut pdf = item("C:/x/отчёт.pdf");
        pdf.name = "отчёт.pdf".into();
        assert_eq!(pdf.label(), "PDF");
    }
}
