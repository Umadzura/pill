# Архитектура Pill

Читать перед тем, как добавлять функцию. Здесь описано, как всё устроено, какие правила нельзя нарушать и куда класть новый код.
Состояние на 2026-11-12 (после визуализатора музыки). Если код разошёлся с этим файлом, правится файл.

---

## 1. Бюджет лёгкости (жёсткие правила)

Pill существует ради того, чтобы быть лёгким. Функция, которая нарушает эти правила, не мёржится, пока её не переделают.

| Правило | Почему | Как проверить |
|---|---|---|
| **В простое 0% CPU.** Никаких опросов чаще 1 раза в секунду. Единственный постоянный таймер `TIMER_CLOCK` (1 с). **Исключение (согласовано с пользователем): визуализатор музыки**, ~30 кадров/с, только пока играет музыка и пилюлю видно (не в полноэкранном режиме). Замер: играет, свёрнуто, 5 с — 0,016 с CPU (~0,3% ядра, на границе точности счётчика 15,6 мс); простой — 0,000 с. Других исключений нет. | Ради этого всё и затевалось (Island брал 16%). | Диспетчер задач, 5 минут в простое. |
| **Всё по событиям.** Сначала ищем системное уведомление (WM_*, COM-callback, WinRT-event, `RegNotifyChangeKeyValue`), опрос только если события нет. Опрашивать можно только пока панель раскрыта. | Опрос в свёрнутом виде съедает батарею ноутбука. | `grep SetTimer`: каждый новый таймер должен быть одноразовым или работать только в раскрытом виде. |
| **Не больше 4 своих потоков**: UI, WinRT-воркер, наблюдатель (будущий), сеть (будущий). | Каждый поток стоит ~1 МБ стека и усложняет синхронизацию. | Новый поток добавляется только с записью в раздел 3. |
| **Working set ≤ 30 МБ, приватная память ≤ 10 МБ.** Сейчас 23,2 / 3,4 (после раскрытия «Управления» и смены яркости 25,1 / 3,9: dxva2; во время игры музыки с визуализатором 26,1 / 4,5). | | Process Explorer после часа работы. |
| **Exe ≤ 1,5 МБ (сейчас 629 248 байт), один файл, без ресурсов и DLL рядом.** | Portable: скопировал и запустил. | `ls target/release/pill.exe` |
| **Перерисовка только того, что поменялось.** `InvalidateRect` зовём только если видимое изменилось (сравнение со старым значением). | GDI-перерисовка 560×340 недорогая, но 60 раз в секунду уже заметно. | |
| **UI-поток не блокируется дольше ~50 мс.** Долгое (WinRT `.get()`, сеть, `rasdial`, WMI, DDC/CI) уходит в фоновый поток. | Иначе пилюля подвисает при наведении. | |

Новые зависимости кроме `windows` не добавляем. Нужна ещё одна фича `windows`, добавляй её в `Cargo.toml` с комментарием, для чего она.

---

## 2. Карта модулей (как есть)

```
main.rs        110 строк  вход: panic hook, мьютекс, COM, DPI, класс окна, цикл сообщений, регистрация хоткеев
ids.rs          55        все WM_APP_*, TIMER_*, HOTKEY_*, MENU_*
app.rs         355        State из частей, st(), Hit/Tab/Redraw, wndproc-диспетчер, on_click, on_tick
shell.rs       283        окно: layout, раскрытие/сворачивание, полноэкран (AppBar), трей, DPI
input.rs       217        клавиатура, поле ввода, вставка из буфера, close_panel
pill.rs         67        модель свёрнутой пилюли: text, lead_width, shows_level (бейджей пока нет)

ui/            ~820       GDI, State только читает
  mod.rs        103       paint(): кадр, вкладки
  paint.rs      223       цвета, значки MDL2, Fonts, Painter, виджеты slider_row (volume_row через него)/input/hint/icon_button
  collapsed.rs   98       свёрнутый вид (level_bar: громкость и яркость; 4 столбика визуализатора)
  viz.rs         63       VizArea, draw(), on_frame(): перерисовка только прямоугольника полосок
  music.rs 115  clip.rs 59 (ROWS = 9)  cmd.rs 50  ctl.rs 96 (строка яркости, если BRIGHTNESS = Some)

feat/          ~1330      логика функций, без GDI
  clip.rs       201       History, ClipHit, on_hit/on_update
  cmd.rs        384       бывший launcher.rs: поиск, калькулятор, CmdRes, results, run_index
  ctl.rs        168       Ctl, Pw, CtlHit, Snapshot, CtlState, refresh, volume_at/volume_wheel, brightness_at/brightness_wheel
  music.rs       29       MusicHit
  osd.rs        213       Flash (Volume/Brightness/Text), Osd (кэш батареи), on_volume, on_audio_device, on_power, battery_event (тест)
  timers.rs      77       Timers: tick/add/cancel/dismiss (тест)
  viz.rs        259       DSP без Win32: FFT 1024, окно Ханна, 24 полосы 40 Гц–16 кГц, group(); static SPECTRUM, WANT/WAKE/PENDING (4 теста)

sys/           ~800       тонкие обёртки над Windows, без State
  audio.rs 143 (громкость + Watch с колбэками)  autostart.rs 23  clipboard.rs 69 (read_text/set_text)
  icon.rs 47  launch.rs 84 (open, run_line, Ctrl+V, force_foreground, hidden)  net.rs 63 (VPN)
  display.rs 213 (яркость: WMI на ноутбуке, DDC/CI на ПК, struct Displays для воркера, to_pct/from_pct с тестом)
  loopback.rs 104 (WASAPI loopback устройства вывода: shared, буфер 200 мс, float32 и PCM16 -> моно; Err при пропаже устройства)
  power.rs 80 (battery() -> Battery, register_notify/parse_notify -> PowerEvent, lock/sleep/restart/shutdown)  theme.rs 43

bg/            ~680       фоновые потоки
  winrt.rs      423       бывший media.rs: GSMTC, обложки, радио, яркость (Displays)
  watch.rs      258       бывший privacy.rs: микрофон/камера (RegNotifyChangeKeyValue) и захват звука для визуализатора
```

Всего ~4825 строк. Правила зависимостей:
- `sys` и `bg` не знают `State`.
- `feat` может звать слой приложения: `app::st`, `input::close_panel`/`paste`, `shell::layout`.
- `ui` читает данные `feat` и `State`, но feat-логику не вызывает.
- `app` знает всех и маршрутизирует.

State: `{ shell, input, hits, hover, clip, cmd, ctl, timers, osd, viz_area }`.
Hit: `Tab, Input, Volume, Brightness, Music(MusicHit), Clip(ClipHit), Cmd(usize), Ctl(CtlHit), Alarm`. `enum Redraw { None, Content, Layout }`.

---

## 3. Потоки и владение данными

```
┌──────────────── UI-поток (STA) ────────────────┐        ┌──── WinRT-воркер (MTA) ────┐
│ GetMessage → wndproc → обработчик → State      │ Job →  │ mpsc::Receiver<Job>         │
│ thread_local STATE: RefCell<Option<State>>     │ ─────> │ владеет GSMTC-сессией,      │
│ paint() читает State + статики media           │        │ подписками, Radio-объектами │
│                                                │ <───── │ пишет в static MEDIA/RADIOS │
│                                                │PostMsg │ PostMessage(WM_APP_MEDIA..) │
└────────────────────────────────────────────────┘        └─────────────────────────────┘
        ↑ WM_* от Windows: таймеры, мышь, буфер, хоткеи, трей, AppBar, DPI, тема
```

Правила:

1. **`State` живёт только в UI-потоке** (`thread_local`). Другие потоки его не видят и не трогают.
2. **Фоновый поток → UI**: кладёт результат в свой `static Mutex<...>` и шлёт `PostMessageW(hwnd, WM_APP_..., 0, 0)`. Никогда `SendMessage`: он заблокирует воркер, а при обратном вызове получится дедлок.
3. **UI → фоновый поток**: `winrt::send(Job::...)`. Задачи в очереди схлопываются (одинаковые подряд выполняются один раз), поэтому `Job` должен быть `PartialEq` и идемпотентным.
4. **Мьютексы держим коротко**: залочил, скопировал или прочитал, отпустил. В `paint` лок `MEDIA` отпускается (`drop(m)`) до рисования кнопок. Никогда не держать два лока сразу.
5. **COM-callback'и** (`IAudioEndpointVolumeCallback` и смена устройства, `sys/audio.rs`) приходят в чужом потоке. Внутри только `PostMessage`, никакой логики.
6. `panic = "abort"`: паника в любом потоке убивает процесс (след остаётся в `crash.log`). Поэтому в фоновых потоках не используем `unwrap()` на результатах Windows API, только `.ok()` / `let _ =`. `lock().unwrap()` допустим: отравления не бывает, процесс уже умер бы.

Свои потоки сейчас:
1. UI.
2. WinRT-воркер (`bg/winrt.rs`): GSMTC, радио и яркость (`Job::Brightness`, `Job::SetBrightness(u8)`, в пачке выполняется только последний; результат в `static BRIGHTNESS: Mutex<Option<u8>>`, сообщение `WM_APP_BRIGHT`). `Displays` (WMI/DDC) живёт в этом потоке.
3. Наблюдатель (`bg/watch.rs`, MTA, инициализирует COM): один `WaitForMultipleObjects` по `[реестр микрофона, реестр камеры (если открылись), событие пробуждения визуализатора]`. Микрофон/камера: `WM_APP_PRIVACY`, результат в `static IN_USE`. Визуализатор: пока `viz::want()` — тайм-аут 33 мс, на тайм-ауте читает WASAPI loopback (`sys/loopback.rs`), считает спектр (`feat/viz.rs`), кладёт в `static SPECTRUM: Mutex<[f32;24]>` и шлёт `WM_APP_VIZ`; иначе `INFINITE`. `PENDING` (AtomicBool): кадр отправлен, но UI его ещё не нарисовал, новые не шлются. UI будит наблюдателя через `SetEvent(WAKE)` при смене `WANT` (`set_want`). Устройство пропало — переоткрытие не чаще раза в секунду. В тишине, когда полоски уже на нуле, UI не будится. Тему пока не слушает.
4. Короткоживущий: `sys/theme.rs::set_dark` на время рассылки `WM_SETTINGCHANGE` (может подвиснуть).

События питания (`WM_POWERBROADCAST`) приходят в UI-поток, своего потока не требуют.

Статики для обмена между потоками: `MEDIA`, `RADIOS`, `BRIGHTNESS`, `IN_USE`, `SPECTRUM` (мьютексы), `WANT`, `WAKE`, `PENDING` (атомики).

Резерв: сеть (`events.rs`, `127.0.0.1`, партия 2). COM-колбэки громкости приходят в чужом потоке COM, внутри только `PostMessage`.

---

## 4. Жизненный цикл события

Всё в Pill идёт по одной схеме:

```
событие (WM_*, WM_APP_* от воркера, таймер)
  → обработчик on_xxx(hwnd, ...)
      → st(|s| ...) меняет State, возвращает «что сделать»
      → вне st(): вызовы Win32 (SetWindowPos, SetForegroundWindow, ShellExecute...)
  → layout(hwnd)        если поменялся размер/вид пилюли
    или InvalidateRect  если поменялось только содержимое
  → WM_PAINT → ui::paint → рисует и заново заполняет State::hits
```

### Правило `st()`: самое важное в коде

`st(|s| ...)` берёт `borrow_mut` у `RefCell`. **Внутри замыкания нельзя звать Win32, которые синхронно шлют сообщения нашему окну** (`SetWindowPos`, `ShowWindow`, `SetFocus`, `SetForegroundWindow`, `DestroyWindow`, `TrackPopupMenu`, `MessageBox`, `UpdateWindow`). Сообщение войдёт в `wndproc`, тот позовёт `st()` второй раз, будет `BorrowMutError` и падение.

Шаблон:
```rust
let (need_layout, path) = st(|s| { /* только данные */ (true, s.apps[i].path.clone()) });
if need_layout { layout(hwnd); }   // Win32 снаружи
```

Также внутри `st()` не делаем ничего долгого (файлы, процессы), если можно вынести наружу. Исключение: `clip.add()` пишет файл, это мелочь, но новые сохранения лучше выносить.

### Hit-test

Кликабельные зоны не хранятся отдельно от отрисовки. `paint` рисует элемент и тут же вызывает `p.hit(rect, Hit::...)`. Клик и наведение ищут зону в `State::hits` (последняя нарисованная выигрывает). Отсюда следствия:
- Элемент, который не нарисован, нельзя нажать. Отдельно синхронизировать нечего.
- `Hit` несёт всё нужное для обработки (индекс записи, тип плитки), чтобы `on_click` не пересчитывал раскладку.
- Индексы в `Hit` (`ClipRow(idx)`) — это индексы в данных, а не номера строк на экране. Между отрисовкой и кликом данные могут поменяться (пришла новая запись буфера), поэтому обработчик всегда делает `.get(i)` и молча выходит, если элемента нет.

---

## 5. Реестр номеров и ресурсов

Все «магические» номера в одном месте. Добавил новый, впиши сюда. Источник правды: `src/ids.rs`.

**Сообщения окна**

| Константа | Значение | Где | Кто шлёт |
|---|---|---|---|
| `WM_APP_MEDIA` | `WM_APP+1` | bg/winrt.rs | воркер: трек/статус изменились |
| `WM_APP_RADIO` | `WM_APP+2` | bg/winrt.rs | воркер: Wi-Fi/BT прочитаны |
| `WM_APP_TRAY` | `WM_APP+3` | shell.rs | иконка трея |
| `WM_APP_BAR` | `WM_APP+4` | shell.rs | Explorer: `ABN_FULLSCREENAPP` |
| `WM_APP_VOLUME` | `WM_APP+5` | sys/audio.rs | COM-колбэк громкости (wparam = громкость×1000, lparam = 1 при mute) |
| `WM_APP_AUDIODEV` | `WM_APP+6` | sys/audio.rs | сменилось устройство вывода |
| `WM_APP_PRIVACY` | `WM_APP+7` | bg/watch.rs | микрофон/камера: поменялось, кто их держит |
| `WM_APP_BRIGHT` | `WM_APP+8` | bg/winrt.rs | воркер: яркость прочитана или поставлена (значение в `static BRIGHTNESS`) |
| `WM_APP_VIZ` | `WM_APP+9` | bg/watch.rs | наблюдатель: новый кадр спектра (`static SPECTRUM`); не шлётся, пока не сброшен `PENDING` |
| `WM_POWERBROADCAST` / `PBT_POWERSETTINGCHANGE` | из крейта | sys/power.rs | система: `GUID_ACDC_POWER_SOURCE`, `GUID_BATTERY_PERCENTAGE_REMAINING`, `GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS` (`RegisterPowerSettingNotification`) |
| `WM_MOUSELEAVE` | `0x02A3` | система | нет в крейте, объявлен в ids.rs |
| `WM_CLIPBOARDUPDATE` | из крейта | система | `AddClipboardFormatListener` |
| `TaskbarCreated` | `RegisterWindowMessage` | shell.rs | Explorer перезапустился |

Резерв под роадмап: `+10` events.rs. Питание и яркость ноутбука своего `WM_APP_*` не имеют: это системный `WM_POWERBROADCAST` (раздел 7).

**Таймеры** (`SetTimer`, id в `wparam`)

| Id | Имя | Период | Когда работает |
|---|---|---|---|
| 1 | `TIMER_CLOCK` | 1000 мс | всегда (единственный постоянный) |
| 2 | `TIMER_COLLAPSE` | 350 мс, разовый | после ухода курсора |
| 3 | `TIMER_PASTE` | 80 мс, разовый | после выбора записи буфера |
| 4 | `TIMER_CTL` | 3000 мс, разовый | после переключения VPN |
| 5 | `TIMER_FLASH` | разовый, длительность задаёт вызов | убрать временную надпись: громкость, яркость, зарядка, предупреждения |
| 6 | `TIMER_SHOT` | 250 мс, разовый | открыть выделение области, когда панель уже свернулась |

**Прочее**

| Что | Значение |
|---|---|
| Хоткеи | 1 `HOTKEY_CMD` Alt+Space (запасной Ctrl+Alt+Space), 2 `HOTKEY_CLIP` Ctrl+Alt+V |
| Пункты меню трея | 10 автозапуск, 11 выход |
| Мьютекс | `Local\PillSingleInstance` |
| Класс окна | `PillWindow` |
| Реестр (пишем) | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` → `Pill`; `...\Themes\Personalize` (`AppsUseLightTheme`, `SystemUsesLightTheme`) |
| Реестр (читаем) | `HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{microphone,webcam}` (+ `NonPackaged`), наблюдает `bg/watch.rs` |
| Файлы | `%APPDATA%\Pill\clipboard.txt` (история), `%APPDATA%\Pill\crash.log` (последняя паника) |
| Только читаем | `%APPDATA%` и `%ProgramData%` `\Microsoft\Network\Connections\Pbk\rasphone.pbk` (VPN-профили), меню Пуск (ярлыки) |

---

## 6. Соглашения

- **`Option` = «недоступно».** `Option<bool>` у плитки: `None` — устройства или функции нет (тусклая плитка, клик открывает `ms-settings:`), `Some(on)` — есть, вот состояние. Так же `battery_text() -> None` на ПК. Новые функции делаем так же: если у машины нет батареи, яркости или камеры, функция просто не показывается, без ошибок и заглушек.
- **Ошибки Windows не пробрасываем.** `let _ =` и `.ok()?` нормальны. Функция в `sys` возвращает `Option`/`bool`, а UI показывает «—» или прячет элемент.
- **Размеры в логических px** при 96 DPI, через `scale(v, dpi)` или `p.s(v)`. Прямых пикселей в коде нет.
- **`DrawTextW` с пустой строкой падает.** Проверка в `Painter::text`, не убирать. Весь текст рисуется только через `Painter::text`.
- **GDI-объекты**: создал, используй и удали в той же функции (`DeleteObject`). Утечка GDI-хендлов через сутки приводит к чёрной пилюле. Шрифты пока создаются на каждый `paint` (дёшево); если станет заметно, кэшировать в `State` с пересозданием на `WM_DPICHANGED`.
- **Логика без Win32 живёт в отдельных функциях и покрыта тестами** (`feat::cmd::calc`, `parse_timer`, `clip::escape`). Новый парсер или ранжирование пишется так же: чистая функция плюс `#[test]`.
- **Внешние программы** (`cmd`, `shutdown`, `rasdial`) запускаем через `sys::hidden` / `sys::open`. Долг из CHANGES.md: перевести на полный путь `%SystemRoot%\System32\...`.
- Комментарии и строки интерфейса по-русски. Комментарий объясняет «почему», а не «что».

---

## 7. Куда растём: целевая структура

### Проблема

(Исторически, до переезда.) Каждая функция из ROADMAP (OSD громкости, яркость, зарядка, микрофон/камера, мини-обложка, скриншот, файловая полка) сейчас потребует правок в пяти местах: поля в `State`, варианты в `Hit`, ветки в `on_click`/`wndproc`, отрисовка в `ui.rs`, текст в `collapsed_text`. `main.rs` уже 1000 строк, после партии 1 станет 2000, и тогда и начнётся «ахуеть».

### Цель

Функция = папка ответственности, которую можно читать отдельно. Слой приложения только маршрутизирует.

```
src/
  main.rs            ~80 строк: panic hook, мьютекс, COM, DPI, класс окна, цикл сообщений
  ids.rs             все WM_APP_*, TIMER_*, HOTKEY_*, MENU_* (раздел 5)
  app.rs             State (из частей), st(), wndproc как чистый диспетчер
  shell.rs           окно: layout, раскрытие/сворачивание, полноэкран (AppBar), трей, DPI
  input.rs           клавиатура и поле ввода: activate/deactivate, on_char, on_key, хоткеи
  pill.rs            свёрнутая пилюля: модель «бейджей» и OSD (см. ниже)

  ui/
    mod.rs           paint(): кадр, вкладки, двойной буфер, диспатч по вкладке
    paint.rs         Painter, Fonts, цвета, значки MDL2, общие виджеты (кнопка, ползунок, поле ввода)
    collapsed.rs     отрисовка свёрнутого вида
    music.rs  clip.rs  cmd.rs  ctl.rs  shelf.rs

  feat/              логика функций: свои данные + обработка своих Hit, без GDI
    music.rs         MusicState, on_hit(MusicHit)
    clip.rs          (бывший clip.rs) History + on_hit(ClipHit)
    cmd.rs           (бывший launcher.rs) + run_cmd, cmd_results
    ctl.rs           плитки, питание, подтверждение перезагрузки
    timers.rs        таймеры и тревога
    osd.rs           громкость/яркость всплывашкой
    shelf.rs         файловая полка (партия 2)

  sys/               тонкие обёртки над Windows, без State
    audio.rs  display.rs (яркость WMI + DDC/CI)  power.rs  net.rs (VPN)
    theme.rs  launch.rs (ShellExecute, Ctrl+V, foreground)  autostart.rs  icon.rs

  bg/                фоновые потоки
    winrt.rs         (бывший media.rs) GSMTC + Radios
    watch.rs         наблюдатель реестра/событий (партия 1: микрофон/камера)
```

### Контракт функции

Каждая функция в `feat/` даёт одно и то же:

```rust
pub struct XxxState { ... }                       // своя часть State
pub enum XxxHit { ... }                           // свои зоны клика
pub unsafe fn on_hit(hwnd, s: XxxHit, ...) -> Redraw   // что перерисовать
pub fn badge(&self) -> Option<Badge>              // что показать в свёрнутой пилюле (если есть)
```

А `app.rs` склеивает:

```rust
pub struct State { pub shell: ShellState, pub input: InputState,
                   pub music: MusicState, pub clip: ClipState, pub cmd: CmdState,
                   pub ctl: CtlState, pub timers: TimersState, pub osd: OsdState, ... }

pub enum Hit { Tab(Tab), Input, Music(MusicHit), Clip(ClipHit), Cmd(CmdHit), Ctl(CtlHit), Alarm, ... }

enum Redraw { None, Content /* InvalidateRect */, Layout /* layout() */ }
```

`on_click` превращается в `match hit { Hit::Music(h) => feat::music::on_hit(...), ... }` на десяток строк. Новая функция добавляет одну ветку, а не размазывается по файлу.

### Свёрнутая пилюля: бейджи и OSD

Партия 1 почти целиком про свёрнутый вид, поэтому его модель надо задать заранее, иначе `collapsed_text` превратится в лапшу из `if`.

- **Бейдж** — маленький кусок свёрнутой пилюли: `{ slot, text/icon, color, priority }`. Функции отдают свои бейджи, `pill.rs` собирает их по слотам:
  ```
  [ левый слот ]   [ центр: время ]   [ правый слот ]
   мини-обложка/♪                        таймер · ⚡64% · ● мик/кам
  ```
- **OSD (временный режим)** перекрывает обычный вид на ~1,5 с: «громкость 40% ▬▬▬▭▭», «яркость», «зарядка подключена», «батарея 10%». В `State` одно поле `osd: Option<Osd { kind, value, until }>` и одноразовый `TIMER_OSD`. Новое событие перезаписывает старое, таймер перезапускается.
- **Приоритет** (сверху важнее): тревога таймера → OSD → предупреждение батареи → обычный вид. Наведение мышью всегда раскрывает панель, поверх всего.
- **Ширина** свёрнутой пилюли пересчитывается только при смене набора бейджей (уже так: сравнение с `last_collapsed`), а не каждую секунду.
- В полноэкранном режиме пилюля скрыта целиком, OSD тоже не показываем (там свой системный).

### Источники событий под роадмап

Решение для каждой функции, чтобы не изобретать заново:

| Функция | Источник события | Поток | Опрос? |
|---|---|---|---|
| Полноэкран | `SHAppBarMessage` → `ABN_FULLSCREENAPP` + проверка на секундном тике | UI | нет (тик уже есть) |
| OSD громкости (сделано) | `IAudioEndpointVolumeCallback::OnNotify` → `WM_APP_VOLUME`, смена устройства → `WM_APP_AUDIODEV` (`sys/audio.rs::Watch`) | COM-поток → UI | нет |
| Яркость ноутбука (сделано иначе) | Событие: `RegisterPowerSettingNotification(GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS)` → `WM_POWERBROADCAST` → `osd::on_power` (WMI-sink не нужен). Чтение и запись: WMI `ROOT\WMI` (`WmiMonitorBrightness`, `WmiSetBrightness`), подключается лениво и только если есть батарея (на ПК DLL WMI дают ~3 МБ) | события в UI, WMI в `bg/winrt.rs` | нет |
| Яркость монитора ПК (сделано) | DDC/CI (`dxva2`: `EnumDisplayMonitors` → `GetPhysicalMonitorsFromHMONITOR` → `Get/SetMonitorBrightness`); ставим на все мониторы, читаем первый | `bg/winrt.rs`: DDC медленный, 40–100 мс | события нет; читаем при раскрытии «Управления» (`ctl::refresh` → `Job::Brightness`) |
| Зарядка/батарея (сделано) | `RegisterPowerSettingNotification` на `GUID_ACDC_POWER_SOURCE` и `GUID_BATTERY_PERCENTAGE_REMAINING` → `WM_POWERBROADCAST` → `osd::on_power`; `GetSystemPowerStatus` читается только в событии, результат кэшируется в `Osd.battery` | UI | нет |
| Микрофон/камера (сделано) | `RegNotifyChangeKeyValue` на `HKCU\...\CapabilityAccessManager\ConsentStore\{microphone,webcam}` (дерево) → `WM_APP_PRIVACY` | `bg/watch.rs` | нет |
| Визуализатор музыки (сделано) | WASAPI loopback устройства вывода по умолчанию (`sys/loopback.rs`). События нет: тайм-аут 33 мс в `WaitForMultipleObjects` наблюдателя, только пока музыка играет и пилюлю видно (`viz::refresh_want` при `WM_APP_MEDIA`, смене раскрытия и полноэкрана) → `WM_APP_VIZ` | `bg/watch.rs` → UI | да, ~30 Гц, осознанное исключение (раздел 1) |
| Прогресс трека | `GetTimelineProperties` + `TimelinePropertiesChanged`, позицию между событиями досчитываем сами от `LastUpdatedTime` | WinRT-воркер | секундный тик только перерисовывает полоску и только если играет |
| Скриншот (сделано) | `ShellExecute("ms-screenclip:")` через `TIMER_SHOT` (250 мс после сворачивания) | UI | — |
| Файловая полка | `IDropTarget` (приём) + `DoDragDrop` с `IDataObject` (отдача), нужен `OleInitialize` | UI | — |

---

## 8. План переезда

Каждый шаг — отдельный коммит, после каждого `cargo build --release` проходит и поведение не меняется. Делаем **до партии 1**, иначе потом переносить в два раза больше.

1. [x] `ids.rs`: вынести все константы номеров (WM_APP из `media.rs` тоже).
2. [x] `sys.rs` → `sys/` по файлам.
3. [x] `ui.rs` → `ui/mod.rs` + `ui/paint.rs` + по файлу на вкладку.
4. [x] `shell.rs` (layout, раскрытие, полноэкран, трей) и `input.rs` (клавиатура) из `main.rs`.
5. [x] Разбить `State` на части, а `Hit` на вложенные enum'ы. `on_click` стал диспетчером.
6. [~] `pill.rs`: `text`/`lead_width`/`shows_level` есть, OSD работает через `feat/osd.rs` и `TIMER_FLASH`. Бейджей (слоты, приоритеты) пока нет.
7. [x] `media.rs` → `bg/winrt.rs` (и `privacy.rs` → `bg/watch.rs`).

Разделы 2 и 5 обновлены после шагов 1–5 и 7 и после партии «яркость + батарея событиями» и после визуализатора.

---

## 9. Чеклист: добавляю функцию

1. Есть ли системное событие? Если нет, можно ли опрашивать только в раскрытом виде? (раздел 1)
2. Будет ли блокировка дольше 50 мс? Значит, фоновый поток: существующий воркер или `watch.rs`, новый поток не заводим.
3. Новые номера (WM_APP, таймер, хоткей) вписать в `ids.rs` и в раздел 5.
4. Логика в `feat/xxx.rs`, обёртки Windows в `sys/xxx.rs`, рисование в `ui/xxx.rs`. `feat` и `sys` не зависят от GDI, `sys` не зависит от `State`.
5. Нет железа (батареи, камеры, DDC)? Значит `None`, и функция не видна.
6. Чистую логику покрыть `#[test]`.
7. Замер после: CPU в простое 0%, working set, размер exe.
8. Запустить в фоне агента `docs-writer` (Sonnet): он обновит этот файл, CLAUDE.md, ROADMAP.md и CHANGES.md и сверит код с правилами отсюда. Передать ему, что сделано, что проверено руками и замеры из п. 7.

---

## 10. Решения и почему (не пересматривать без причины)

| Решение | Почему | Что потеряли |
|---|---|---|
| Win32 + GDI, без фреймворков | RAM и CPU. WebView2 дал Island 950 МБ | Нет сглаживания и альфы. Если понадобится, точечно `UpdateLayeredWindow` с 32-битным DIB, но не Direct2D на всё |
| Тулчейн `x86_64-pc-windows-gnu` | VS Build Tools не ставятся (ошибка 5008) | Нет C-компилятора, значит нет SQLite и других крейтов с C-кодом |
| История буфера в текстовом файле | См. выше. Сотня записей, база не нужна | Нет индекса. При 100 записях не важно |
| `State` в `thread_local` + `RefCell` | Однопоточный UI, без `Mutex` и `unsafe static mut` | Правило `st()` из раздела 4 |
| Hit-зоны записываются при отрисовке | Раскладка и клики не расходятся | Клик до первого `paint` ничего не находит (не страшно) |
| `SetWindowRgn` для формы | Просто и дёшево | Угловатые края (в планах) |
| Один WinRT-воркер на всё WinRT | Блокирующие `.get()` не трогают UI, а MTA нужен для событий | Медленная операция (обложка) задерживает остальные задачи. Пока терпимо |
| WMI для яркости только при наличии батареи, на ПК DDC/CI | DLL WMI добавляют ~3 МБ WS, на ПК WMI не нужен | Ноутбук без батареи (редкость) получит яркость только по DDC/CI |
| Кадр визуализатора перерисовывает только свой прямоугольник (`ui/viz.rs::on_frame`: `GetDC`, маленький буфер, `BitBlt`), а не `InvalidateRect` | 30 раз в секунду перерисовывать всё окно со шрифтами и обложкой заметно по CPU | Кадр рисуется мимо `paint`; прямоугольник берётся из `State.viz_area`, который обновляет каждый `paint` |
| Захват звука и расчёт кадров в `bg/watch.rs`, без нового потока | Лимит потоков, наблюдатель и так ждёт на `WaitForMultipleObjects` | Наблюдатель перестал быть чисто событийным: пока играет музыка, просыпается 30 раз в секунду |
| `panic = "abort"`, `strip`, LTO | Размер exe | Нет бэктрейса, только сообщение в `crash.log` |

---

## Сборка и проверка

```
taskkill /IM pill.exe /F
%USERPROFILE%\.cargo\bin\cargo build --release
%USERPROFILE%\.cargo\bin\cargo test --release
```
Debug-сборка не линкуется (кириллица в пути пользователя), поэтому всё через `--release`.
