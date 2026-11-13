//! Чужие уведомления (тосты Telegram, браузера и других программ) через `UserNotificationListener`.
//! Только из фонового потока: вызовы блокирующие (10–20 мс).
//! Событие `NotificationChanged` непакетному приложению недоступно (0x80070490, проверено),
//! поэтому список читает по запросу тот, кто нас зовёт.

use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};
use windows::Win32::System::SystemInformation::GetSystemTimeAsFileTime;

#[derive(Clone)]
pub struct Toast {
    pub id: u32,
    /// Имя программы, как в центре уведомлений.
    pub app: String,
    /// Строки текста: заголовок и дальше тело.
    pub texts: Vec<String>,
    /// Когда создано: FILETIME (UTC, шаг 100 нс).
    pub created: i64,
}

pub struct Listener {
    api: UserNotificationListener,
    /// Уже прочитанные уведомления по id: имя программы и тексты достаются десятком вызовов COM,
    /// а список перечитывается каждые 2 с. В простое сверяем только id.
    cache: Vec<Toast>,
}

impl Listener {
    /// None — доступа нет (запрещён в «Параметры → Конфиденциальность → Уведомления») или система слишком старая.
    pub fn open() -> Option<Listener> {
        let l = UserNotificationListener::Current().ok()?;
        let mut status = l.GetAccessStatus().ok()?;
        if status == UserNotificationListenerAccessStatus::Unspecified {
            status = l.RequestAccessAsync().ok()?.get().ok()?;
        }
        (status == UserNotificationListenerAccessStatus::Allowed).then_some(Listener { api: l, cache: Vec::new() })
    }

    /// Все уведомления из центра уведомлений. None — чтение не удалось.
    pub fn read(&mut self) -> Option<Vec<Toast>> {
        let list = self.api.GetNotificationsAsync(NotificationKinds::Toast).ok()?.get().ok()?;
        let mut out = Vec::new();
        for n in list {
            let Ok(id) = n.Id() else { continue };
            match self.cache.iter().find(|t| t.id == id) {
                Some(t) => out.push(t.clone()),
                None => out.push(toast(id, &n)),
            }
        }
        self.cache = out.clone();
        Some(out)
    }

    /// Убрать уведомление из центра уведомлений Windows.
    pub fn remove(&self, id: u32) {
        let _ = self.api.RemoveNotification(id);
    }
}

fn toast(id: u32, n: &UserNotification) -> Toast {
    let app = n.AppInfo().and_then(|a| a.DisplayInfo()).and_then(|d| d.DisplayName()).map(|h| h.to_string()).unwrap_or_default();
    let mut texts = Vec::new();
    let binding = KnownNotificationBindings::ToastGeneric().and_then(|k| n.Notification()?.Visual()?.GetBinding(&k));
    if let Ok(list) = binding.and_then(|b| b.GetTextElements()) {
        for t in list {
            if let Ok(s) = t.Text() {
                texts.push(s.to_string());
            }
        }
    }
    let created = n.CreationTime().map(|t| t.UniversalTime).unwrap_or(0);
    Toast { id, app, texts, created }
}

/// Сейчас в тех же единицах, что `Toast::created`.
pub fn now_filetime() -> i64 {
    let ft = unsafe { GetSystemTimeAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}
