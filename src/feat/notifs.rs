//! Уведомления других программ (Telegram, браузер, почта) в пилюле вместо баннеров Windows.
//!
//! Источник — центр уведомлений Windows (`sys::toasts`). События «пришло новое» у непакетного
//! приложения нет, поэтому раз в 2 с UI просит воркер перечитать список (`Job::Notifs`, 10–20 мс
//! в фоновом потоке). Новое уведомление — короткая надпись карточкой из чёлки, весь список — во
//! вкладке «События». Здесь логика без Windows, с тестами.

use std::sync::atomic::AtomicBool;
use std::sync::Mutex;
use std::time::Duration;

use crate::feat::agents::one_line;
use crate::sys::toasts::Toast;

/// Сколько уведомлений держим в списке.
const MAX_LIST: usize = 30;
/// Сколько висит карточка нового уведомления.
pub const PEEK_MS: u32 = 5000;

#[derive(Clone, PartialEq, Debug)]
pub struct Notif {
    pub id: u32,
    pub app: String,
    pub title: String,
    pub body: String,
    /// FILETIME создания (UTC, шаг 100 нс).
    pub created: i64,
}

impl Notif {
    pub fn from_toast(t: Toast) -> Notif {
        let mut texts = t.texts.iter().map(|s| one_line(s, 300)).filter(|s| !s.is_empty());
        let title = texts.next().unwrap_or_default();
        let body = texts.collect::<Vec<_>>().join(" ");
        Notif { id: t.id, app: one_line(&t.app, 40), title, body, created: t.created }
    }
}

pub struct Inbox {
    /// Новые сверху.
    pub list: Vec<Notif>,
    /// Самое свежее из только что пришедших (UI забирает по WM_APP_NOTIF и показывает карточкой).
    pub fresh: Option<Notif>,
    /// Сколько пришло с тех пор, как открывали «События» (счётчик в свёрнутой чёлке).
    pub unread: usize,
    /// Id из прошлого чтения. None — ещё не читали: то, что уже лежит в центре уведомлений, не «новое».
    known: Option<Vec<u32>>,
}

impl Inbox {
    pub const fn new() -> Inbox {
        Inbox { list: Vec::new(), fresh: None, unread: 0, known: None }
    }

    /// Положить свежепрочитанный список. true — список поменялся (надо перерисовать).
    pub fn update(&mut self, mut cur: Vec<Notif>) -> bool {
        cur.sort_by(|a, b| b.created.cmp(&a.created).then(b.id.cmp(&a.id)));
        cur.truncate(MAX_LIST);
        if let Some(known) = &self.known {
            // cur отсортирован: первый незнакомый — самый свежий.
            if let Some(n) = cur.iter().find(|n| !known.contains(&n.id)) {
                self.fresh = Some(n.clone());
            }
            self.unread += cur.iter().filter(|n| !known.contains(&n.id)).count();
        }
        self.known = Some(cur.iter().map(|n| n.id).collect());
        let changed = self.list != cur;
        self.list = cur;
        changed
    }

    /// Убрать из списка сразу, не дожидаясь следующего чтения.
    pub fn forget(&mut self, id: u32) {
        self.list.retain(|n| n.id != id);
    }
}

/// Пишет воркер, читает UI (вкладка «События» и WM_APP_NOTIF).
pub static INBOX: Mutex<Inbox> = Mutex::new(Inbox::new());
/// Доступа к уведомлениям нет: воркер выяснил это один раз, UI больше не просит читать.
pub static OFF: AtomicBool = AtomicBool::new(false);

/// Возраст уведомления по FILETIME.
pub fn age(created: i64, now: i64) -> Duration {
    Duration::from_secs(((now - created).max(0) / 10_000_000) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(id: u32, created: i64) -> Notif {
        Notif { id, app: "Telegram".into(), title: format!("Чат {}", id), body: "привет".into(), created }
    }

    #[test]
    fn unread_counts_only_new() {
        let mut b = Inbox::new();
        // Первое чтение: всё, что уже лежало в центре уведомлений, не считается.
        b.update(vec![n(1, 10), n(2, 20)]);
        assert_eq!(b.unread, 0);
        b.update(vec![n(1, 10), n(2, 20), n(3, 30), n(4, 40)]);
        assert_eq!(b.unread, 2);
        b.update(vec![n(3, 30), n(4, 40)]);
        assert_eq!(b.unread, 2);
    }

    #[test]
    fn texts() {
        let t = Toast { id: 7, app: "Telegram Desktop".into(), texts: vec!["Иван".into(), "привет,\nты где?".into(), " ".into(), "ау".into()], created: 5 };
        let x = Notif::from_toast(t);
        assert_eq!((x.title.as_str(), x.body.as_str()), ("Иван", "привет, ты где? ау"));
        assert_eq!(x.app, "Telegram Desktop");
    }

    #[test]
    fn inbox() {
        let mut b = Inbox::new();
        // Первое чтение: старые уведомления не мигают.
        assert!(b.update(vec![n(1, 10), n(2, 20)]));
        assert_eq!(b.fresh, None);
        assert_eq!(b.list[0].id, 2);
        // То же самое ещё раз — ничего не поменялось.
        assert!(!b.update(vec![n(2, 20), n(1, 10)]));
        // Пришли два новых: мигает самое свежее.
        assert!(b.update(vec![n(1, 10), n(2, 20), n(3, 30), n(4, 40)]));
        assert_eq!(b.fresh.take().map(|x| x.id), Some(4));
        // Одно убрали в центре уведомлений: список поменялся, надписи нет.
        assert!(b.update(vec![n(1, 10), n(3, 30), n(4, 40)]));
        assert_eq!(b.fresh, None);
        b.forget(3);
        assert_eq!(b.list.iter().map(|x| x.id).collect::<Vec<_>>(), vec![4, 1]);
        assert_eq!(age(10_000_000, 10_000_000 * 61), Duration::from_secs(60));
    }
}
