//! Модель свёрнутой пилюли: что в ней сейчас показывать. Рисует `ui::collapsed`.
//! Функции отдают факты (`Facts`), `assemble` раскладывает их по слотам:
//! [ значки слева ] [ текст по центру ] [ спектр справа ].
//! Приоритет центра (сверху важнее): тревога таймера → временная надпись (OSD) → обычный вид.
//! Пока центр занят тревогой или надписью, значки и спектр не показываются.

use crate::app::State;
use crate::bg::{watch, winrt};
use crate::feat::cmd::fmt_dur;
use crate::feat::osd::{battery_short, Flash};

/// Размеры значков (логические px).
pub const EQ_W: i32 = 18;
pub const DOT: i32 = 7;
/// Сторона мини-обложки.
pub const ART: i32 = 20;
/// Ширина маскота агента (10 пикселей по 2 px).
pub const MASCOT_W: i32 = 20;

/// Значок в левом слоте. Порядок в `View::lead` — порядок на экране.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Lead {
    Mic,
    Cam,
    /// Мини-обложка играющего трека.
    Art,
    /// Маскот главного агента; число рядом, если агентов больше одного.
    Mascot { count: usize },
}

impl Lead {
    /// Сколько места значок занимает вместе с отступом после себя.
    pub fn width(self) -> i32 {
        match self {
            Lead::Mic | Lead::Cam => DOT + 4,
            Lead::Art => ART + 10,
            Lead::Mascot { count } => MASCOT_W + 8 + if count > 1 { 12 } else { 0 },
        }
    }
}

/// Цвет текста в центре.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tone {
    Plain,
    Accent,
    Warn,
}

/// Что сейчас видно в свёрнутой пилюле.
#[derive(PartialEq, Debug)]
pub struct View {
    pub lead: Vec<Lead>,
    pub text: String,
    pub tone: Tone,
    /// Вместо текста полоска уровня (громкость, яркость): её рисует `ui::collapsed::level_bar`.
    pub level: bool,
    /// Справа полоски визуализатора.
    pub viz: bool,
    /// С какого байта текста начинается приглушённый хвост (заряд батареи). None — весь текст одним цветом.
    pub dim_from: Option<usize>,
    /// Обычный вид: можно рисовать полоску прогресса трека.
    pub plain: bool,
    /// Прогресс задачи скрипта, 0..=100: тонкая полоска по низу (важнее прогресса трека).
    pub task_bar: Option<u8>,
    /// Сколько чужих уведомлений пришло, пока «События» не открывали: синий счётчик справа от текста.
    pub unread: usize,
}

impl View {
    /// Ширина левого слота. После точек микрофона/камеры добавляется зазор.
    pub fn left_width(&self) -> i32 {
        let dots = self.lead.iter().any(|l| matches!(l, Lead::Mic | Lead::Cam));
        self.lead.iter().map(|l| l.width()).sum::<i32>() + if dots { DOT_GAP } else { 0 }
    }
    pub fn right_width(&self) -> i32 {
        (if self.unread > 0 { UNREAD_GAP + UNREAD_W } else { 0 }) + if self.viz { EQ_W + 10 } else { 0 }
    }
}

/// Зазор между точками микрофона/камеры и следующим значком.
pub const DOT_GAP: i32 = 4;
/// Счётчик пропущенных уведомлений: ширина кружка и отступ от текста.
pub const UNREAD_W: i32 = 18;
pub const UNREAD_GAP: i32 = 8;
/// Имя программы у точки микрофона/камеры не длиннее стольких символов.
const BUSY_APP_MAX: usize = 16;

/// Всё, от чего зависит свёрнутый вид. Собирается из `State` и статиков, без Win32.
#[derive(Default)]
pub struct Facts {
    pub alarm: Option<String>,
    pub flash: Option<Flash>,
    pub time: String,
    /// Задачи скриптов: «Рендер 45%».
    pub tasks: Vec<String>,
    /// Секунд до ближайшего таймера.
    pub timer_left: Option<u64>,
    pub battery: Option<String>,
    pub mic: bool,
    pub cam: bool,
    /// Кто держит микрофон (или камеру, если микрофон свободен).
    pub busy_app: Option<String>,
    /// Прогресс первой задачи скрипта, у которой он есть.
    pub task_bar: Option<u8>,
    /// Пропущенные чужие уведомления.
    pub unread: usize,
    /// Есть обложка играющего трека.
    pub art: bool,
    pub playing: bool,
    /// Сколько агентов работает или ждёт разрешения (None — никого).
    pub agents: Option<usize>,
}

pub fn assemble(f: Facts) -> View {
    let quiet = View { lead: Vec::new(), text: String::new(), tone: Tone::Plain, level: false, viz: false, dim_from: None, plain: false, task_bar: None, unread: 0 };
    if let Some(a) = f.alarm {
        return View { text: format!("⏰  {}", a), tone: Tone::Accent, ..quiet };
    }
    match f.flash {
        Some(Flash::Text(t, warn)) => return View { text: t, tone: if warn { Tone::Warn } else { Tone::Accent }, ..quiet },
        Some(Flash::Volume(..) | Flash::Brightness(_)) => return View { level: true, ..quiet },
        None => {}
    }
    let mut parts = Vec::new();
    // Рядом с точкой микрофона/камеры пишем, кто их держит.
    if f.mic || f.cam {
        parts.extend(f.busy_app.map(|a| a.chars().take(BUSY_APP_MAX).collect::<String>()).filter(|a| !a.is_empty()));
    }
    parts.push(f.time);
    parts.extend(f.tasks);
    if let Some(left) = f.timer_left {
        parts.push(format!("⏱ {}", fmt_dur(left)));
    }
    // Заряд батареи идёт последним и рисуется серым.
    let mut text = parts.join("   ");
    let mut dim_from = None;
    if let Some(b) = f.battery {
        text.push_str("  ");
        dim_from = Some(text.len());
        text.push_str(&b);
    }
    let mut lead = Vec::new();
    if f.mic {
        lead.push(Lead::Mic);
    }
    if f.cam {
        lead.push(Lead::Cam);
    }
    if f.art && f.playing {
        lead.push(Lead::Art);
    }
    if let Some(count) = f.agents {
        lead.push(Lead::Mascot { count });
    }
    View { lead, text, tone: Tone::Plain, level: false, viz: f.playing, dim_from, plain: true, task_bar: f.task_bar, unread: f.unread }
}

pub fn time_text() -> String {
    let (h, m) = crate::sys::local_hm();
    format!("{:02}:{:02}", h, m)
}

/// Вид по `State` и статикам фоновых потоков (микрофон/камера, плеер, уведомления).
/// Каждый мьютекс берётся и сразу отпускается.
pub fn view(s: &State) -> View {
    let mut f = Facts {
        alarm: s.timers.alarm.as_ref().map(|a| a.to_string()),
        flash: s.osd.flash.clone(),
        time: time_text(),
        tasks: s.tasks.badges(),
        timer_left: s.timers.nearest_left(),
        battery: battery_short(s.osd.battery),
        agents: s.agents.attention().map(|(_, n)| n),
        task_bar: s.tasks.bar(),
        ..Facts::default()
    };
    {
        let u = watch::IN_USE.lock().unwrap();
        f.mic = !u.mic.is_empty();
        f.cam = !u.cam.is_empty();
        f.busy_app = u.mic.first().or(u.cam.first()).cloned();
    }
    f.unread = crate::feat::notifs::INBOX.lock().unwrap().unread;
    {
        let m = winrt::MEDIA.lock().unwrap();
        f.playing = m.has && m.playing;
        f.art = m.art.is_some();
    }
    assemble(f)
}

/// Текст свёрнутой пилюли (раз в секунду сравнивается с прошлым).
pub fn text(s: &State) -> String {
    view(s).text
}

/// Сейчас показывается полоска уровня (громкость, яркость), а не текст.
pub fn shows_level(s: &State) -> bool {
    s.timers.alarm.is_none() && matches!(s.osd.flash, Some(Flash::Volume(..) | Flash::Brightness(_)))
}

/// Мини-обложка для отрисовки: пока трек играет и обложка есть.
pub fn mini_art() -> Option<std::sync::Arc<winrt::Art>> {
    let m = winrt::MEDIA.lock().unwrap();
    if m.has && m.playing {
        m.art.clone()
    } else {
        None
    }
}

/// Сколько ширины значки добавляют к тексту.
pub fn lead_width(s: &State) -> i32 {
    let v = view(s);
    v.left_width() + v.right_width()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts { time: "14:32".into(), ..Facts::default() }
    }

    #[test]
    fn plain_view_collects_slots() {
        let v = assemble(facts());
        assert_eq!((v.text.as_str(), v.lead.len(), v.viz, v.plain), ("14:32", 0, false, true));
        assert_eq!(v.left_width() + v.right_width(), 0);

        let v = assemble(Facts {
            tasks: vec!["Рендер 45%".into()],
            timer_left: Some(90),
            battery: Some("87%".into()),
            mic: true,
            art: true,
            playing: true,
            agents: Some(2),
            ..facts()
        });
        assert_eq!(v.text, format!("14:32   Рендер 45%   ⏱ {}  87%", fmt_dur(90)));
        assert_eq!(v.dim_from.map(|i| &v.text[i..]), Some("87%"));
        assert_eq!(v.lead, vec![Lead::Mic, Lead::Art, Lead::Mascot { count: 2 }]);
        // точка + зазор, обложка, маскот с числом
        assert_eq!(v.left_width(), (DOT + 4) + DOT_GAP + (ART + 10) + (MASCOT_W + 8 + 12));
        assert_eq!(v.right_width(), EQ_W + 10);
    }

    #[test]
    fn stage1_extras() {
        // Имя программы только пока микрофон или камера заняты, и не длиннее лимита.
        let v = assemble(Facts { mic: true, busy_app: Some("Очень длинное имя программы".into()), ..facts() });
        assert_eq!(v.text, "Очень длинное им   14:32");
        assert_eq!(assemble(Facts { busy_app: Some("Zoom".into()), ..facts() }).text, "14:32");
        // Счётчик и полоска задачи есть только в обычном виде.
        let v = assemble(Facts { unread: 3, task_bar: Some(45), playing: true, ..facts() });
        assert_eq!((v.unread, v.task_bar, v.right_width()), (3, Some(45), UNREAD_GAP + UNREAD_W + EQ_W + 10));
        let v = assemble(Facts { unread: 3, task_bar: Some(45), flash: Some(Flash::Text("x".into(), false)), ..facts() });
        assert_eq!((v.unread, v.task_bar, v.right_width()), (0, None, 0));
    }

    #[test]
    fn art_needs_playing() {
        let v = assemble(Facts { art: true, playing: false, ..facts() });
        assert!(v.lead.is_empty() && !v.viz);
    }

    #[test]
    fn alarm_and_flash_take_the_center() {
        let busy = || Facts { mic: true, playing: true, art: true, agents: Some(1), ..facts() };
        // Тревога важнее надписи, надпись важнее обычного вида; значки и спектр при этом убраны.
        let v = assemble(Facts { alarm: Some("Чай".into()), flash: Some(Flash::Text("x".into(), true)), ..busy() });
        assert_eq!((v.text.as_str(), v.tone, v.plain), ("⏰  Чай", Tone::Accent, false));
        assert!(v.lead.is_empty() && !v.viz);

        let v = assemble(Facts { flash: Some(Flash::Text("Батарея 10%".into(), true)), ..busy() });
        assert_eq!((v.text.as_str(), v.tone), ("Батарея 10%", Tone::Warn));
        let v = assemble(Facts { flash: Some(Flash::Text("Готово".into(), false)), ..busy() });
        assert_eq!(v.tone, Tone::Accent);

        let v = assemble(Facts { flash: Some(Flash::Volume(0.4, false)), ..busy() });
        assert!(v.level && v.text.is_empty() && v.left_width() + v.right_width() == 0);
    }
}
