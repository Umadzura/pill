//! Вкладка «Команды»: поле ввода и результаты.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;

use super::paint::*;
use crate::app::{Hit, State};
use crate::feat::cmd::{fmt_dur, results, CmdRes};

pub unsafe fn draw(p: &mut Painter, s: &State, w: i32) {
    p.input(w, s, "Нажми сюда или Alt+Space");
    let res = results(s);
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
        let h = Hit::Cmd(i);
        if p.hovered(h) || (s.input.active && s.input.sel == i) {
            p.fill(row, 10, HOVER);
        }
        p.hit(row, h);
        let (icon, label, right, color) = match r {
            CmdRes::Calc(v) => (I_CALC, format!("= {}", v), "Enter — скопировать", ACCENT),
            CmdRes::Timer(t) => (I_TIMER, format!("Таймер на {}", fmt_dur(*t)), "Enter — запустить", FG),
            CmdRes::App(a) => (I_APP, s.cmd.apps[*a].name.clone(), "программа", FG),
            CmdRes::Run(q) => (I_RUN, format!("Выполнить: {}", q), "команда", DIM),
            CmdRes::Active(t) => {
                let tm = &s.timers.list[*t];
                let left = tm.end.saturating_duration_since(now).as_secs();
                (I_TIMER, format!("Осталось {} из {}", fmt_dur(left), fmt_dur(tm.total)), "клик — отменить", FG)
            }
        };
        p.text(p.f.icon, icon, RECT { left: row.left + p.s(12), right: row.left + p.s(36), ..row }, DIM, DT_LEFT);
        p.text(p.f.normal, &label, RECT { left: row.left + p.s(40), right: row.right - p.s(150), ..row }, color, DT_LEFT | DT_END_ELLIPSIS);
        p.text(p.f.small, right, RECT { right: row.right - p.s(12), ..row }, DIM, DT_RIGHT);
    }
}
