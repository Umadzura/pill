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
/// Ширина маскота агента (9 пикселей по 2 px).
pub const MASCOT_W: i32 = 18;

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
    /// Обычный вид: можно рисовать полоску прогресса трека.
    pub plain: bool,
}

impl View {
    /// Ширина левого слота. После точек микрофона/камеры добавляется зазор.
    pub fn left_width(&self) -> i32 {
        let dots = self.lead.iter().any(|l| matches!(l, Lead::Mic | Lead::Cam));
        self.lead.iter().map(|l| l.width()).sum::<i32>() + if dots { DOT_GAP } else { 0 }
    }
    pub fn right_width(&self) -> i32 {
        if self.viz {
            EQ_W + 10
        } else {
            0
        }
    }
}

/// Зазор между точками микрофона/камеры и следующим значком.
pub const DOT_GAP: i32 = 4;

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
    /// Есть обложка играющего трека.
    pub art: bool,
    pub playing: bool,
    /// Сколько агентов работает или ждёт разрешения (None — никого).
    pub agents: Option<usize>,
}

pub fn assemble(f: Facts) -> View {
    let quiet = View { lead: Vec::new(), text: String::new(), tone: Tone::Plain, level: false, viz: false, plain: false };
    if let Some(a) = f.alarm {
        return View { text: format!("⏰  {}", a), tone: Tone::Accent, ..quiet };
    }
    match f.flash {
        Some(Flash::Text(t, warn)) => return View { text: t, tone: if warn { Tone::Warn } else { Tone::Accent }, ..quiet },
        Some(Flash::Volume(..) | Flash::Brightness(_)) => return View { level: true, ..quiet },
        None => {}
    }
    let mut parts = vec![f.time];
    parts.extend(f.tasks);
    if let Some(left) = f.timer_left {
        parts.push(format!("⏱ {}", fmt_dur(left)));
    }
    parts.extend(f.battery);
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
    View { lead, text: parts.join("   "), tone: Tone::Plain, level: false, viz: f.playing, plain: true }
}

pub fn time_text() -> String {
    let (h, m) = crate::sys::local_hm();
    format!("{:02}:{:02}", h, m)
}

/// Факты из `State`, без чужих мьютексов: хватает для текста и его ширины.
fn base(s: &State) -> Facts {
    Facts {
        alarm: s.timers.alarm.as_ref().map(|a| a.to_string()),
        flash: s.osd.flash.clone(),
        time: time_text(),
        tasks: s.tasks.badges(),
        timer_left: s.timers.nearest_left(),
        battery: battery_short(s.osd.battery),
        agents: s.agents.attention().map(|(_, n)| n),
        ..Facts::default()
    }
}

/// Полный вид: добавляет то, что живёт в статиках фоновых потоков (микрофон/камера, плеер).
pub fn view(s: &State) -> View {
    let mut f = base(s);
    {
        let u = watch::IN_USE.lock().unwrap();
        f.mic = !u.mic.is_empty();
        f.cam = !u.cam.is_empty();
    }
    {
        let m = winrt::MEDIA.lock().unwrap();
        f.playing = m.has && m.playing;
        f.art = m.art.is_some();
    }
    assemble(f)
}

/// Текст свёрнутой пилюли (раз в секунду сравнивается с прошлым, поэтому без мьютексов).
pub fn text(s: &State) -> String {
    assemble(base(s)).text
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
        assert_eq!(v.text, format!("14:32   Рендер 45%   ⏱ {}   87%", fmt_dur(90)));
        assert_eq!(v.lead, vec![Lead::Mic, Lead::Art, Lead::Mascot { count: 2 }]);
        // точка + зазор, обложка, маскот с числом
        assert_eq!(v.left_width(), (DOT + 4) + DOT_GAP + (ART + 10) + (MASCOT_W + 8 + 12));
        assert_eq!(v.right_width(), EQ_W + 10);
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
