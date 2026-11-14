# Архитектура Pill

Читать перед тем, как добавлять функцию. Здесь описано, как всё устроено, какие правила нельзя нарушать и куда класть новый код.
Состояние на 2026-11-15 (после агентов, уведомлений, слоёного окна, вкладок-групп, ушек, анимации раскрытия, карточки из чёлки, маскотов, настроек, карточек сессий агентов, поиска вместо вкладок «Буфер» и «Команды» и уведомлений карточкой по чатам, этап 5, просьбы агентов карточкой с ответом, этап 6, спрятать пилюлю по Ctrl+Alt+H, ползунки перетаскиванием, этап 11, редизайн 13.2: высота панели по содержимому вкладки). Если код разошёлся с этим файлом, правится файл.

---

## 1. Бюджет лёгкости (жёсткие правила)

Pill существует ради того, чтобы быть лёгким. Функция, которая нарушает эти правила, не мёржится, пока её не переделают.

| Правило | Почему | Как проверить |
|---|---|---|
| **В простое 0% CPU.** Никаких опросов чаще 1 раза в секунду. Единственный постоянный таймер `TIMER_CLOCK` (1 с). **Короткоживущий `TIMER_ANIM` (15 мс) живёт только ~200 мс во время раскрытия или сворачивания и сразу гасится `KillTimer`; в простое его нет.** **Исключение (согласовано с пользователем): визуализатор музыки**, ~30 кадров/с, только пока играет музыка и пилюлю видно (не в полноэкранном режиме). Замер: играет, свёрнуто, 5 с — 0,016 с CPU (~0,3% ядра, на границе точности счётчика 15,6 мс); простой — 0,000 с. **Второе исключение (согласовано): `s.load.tick()` на секундном тике, в том числе свёрнутым** (`GetSystemTimes` + `GlobalMemoryStatusEx`, предупреждение о нагрузке). Монитор системы (снимки процессов, GPU, диск, сеть) снимается только пока открыта вкладка «Система». **Третье исключение (ЖДЁТ СОГЛАСОВАНИЯ с пользователем): опрос чужих уведомлений `GetNotificationsAsync` раз в 2 с (`Job::Notifs` с секундного тика через раз), в том числе свёрнутым, пока пилюлю видно (не в полноэкранном режиме); пока нет доступа (`notifs::OFF`), не шлётся.** Замер: простой 60 с, свёрнуто — 47 мс CPU (~0,08% ядра; 78 мс до перехода на слоёное окно, 250 мс до кэша прочитанных по id), до этого 0,000 с. Других исключений нет. | Ради этого всё и затевалось (Island брал 16%). | Диспетчер задач, 5 минут в простое. |
| **Всё по событиям.** Сначала ищем системное уведомление (WM_*, COM-callback, WinRT-event, `RegNotifyChangeKeyValue`), опрос только если события нет. Опрашивать можно только пока панель раскрыта. | Опрос в свёрнутом виде съедает батарею ноутбука. | `grep SetTimer`: каждый новый таймер должен быть одноразовым, короткоживущим (как `TIMER_ANIM`) или работать только в раскрытом виде. |
| **Не больше 4 своих потоков**: UI, WinRT-воркер, наблюдатель, локальный API (все четыре заняты, пятого нет). | Каждый поток стоит ~1 МБ стека и усложняет синхронизацию. | Новый поток добавляется только с записью в раздел 3. |
| **Working set ≤ 30 МБ, приватная память ≤ 10 МБ.** **ПРЕВЫШЕНО (14.11, коммиты d815ee0 и 20efa8a): рабочий набор около 28 МБ сразу после запуска и около 32 МБ после открытия вкладок, потолок 30 МБ пробит при открытых вкладках. Причина не выяснена, подозрение на фичу `Networking_Connectivity` (имя Wi-Fi). Требуется замер (с фичой и без неё, приватная память тоже), пока не мёржить как «в бюджете».** До этих коммитов после этапов 1 и 3 рабочий набор был около 25,7 МБ (приватная не замерялась); раньше 26,5 / 4,8 (простой 30 с, свёрнуто, после ушек и анимации; 26,8 / 4,9 после слоёного окна, после открытия вкладок; до него 25,3 / 4,3 в простое, замер после агентов и уведомлений; вкладка «События» открыта 28,5–28,8 / 5,2–5,4; до них 20,9 / 3,2; раньше 22,6 / 3,6; после раскрытия «Управления» и смены яркости 25,1 / 3,9: dxva2; во время игры музыки с визуализатором 26,1 / 4,5; вкладка «Система» открыта 23,2 / 4,2). **PDH не использовать** (см. раздел 10). | | Process Explorer после часа работы. |
| **Exe ≤ 1,5 МБ (сейчас 895 488 байт), один файл, без ресурсов и DLL рядом.** | Portable: скопировал и запустил. | `ls target/release/pill.exe` |
| **Перерисовка только того, что поменялось.** `InvalidateRect` зовём только если видимое изменилось (сравнение со старым значением). | Кадр раскрытой панели (буфер ~3,8 МБ на 200%) недорогой, но 60 раз в секунду уже заметно. | |
| **UI-поток не блокируется дольше ~50 мс.** Долгое (WinRT `.get()`, сеть, `rasdial`, WMI, DDC/CI) уходит в фоновый поток. | Иначе пилюля подвисает при наведении. | Нарушение: `feat/ctl.rs::refresh` зовёт COM (`audio::output_name`, `mic_muted`, плюс `volume`, `dark`) в UI-потоке, в том числе на каждом шаге колеса громкости (`volume_wheel`, `volume_at`), а с 14.11 и на каждом `WM_MOUSEMOVE` при перетаскивании ползунка громкости. Время не замерено. |

Новые зависимости кроме `windows` не добавляем. Нужна ещё одна фича `windows`, добавляй её в `Cargo.toml` с комментарием, для чего она.

---

## 2. Карта модулей (как есть)

```
main.rs        112 строк  вход: panic hook, мьютекс, COM, DPI, класс окна (WS_EX_LAYERED), цикл сообщений, регистрация хоткеев (1, 2, 7 Ctrl+Alt+H; хоткеи ответа 3..6 регистрирует `feat/peek.rs`)
ids.rs          81        все WM_APP_*, TIMER_*, HOTKEY_*, MENU_*
app.rs         588        State из частей (в том числе `peek`, `asks` (очередь просьб), `drag`, `settings`, `quiet_until`; `WM_HOTKEY` с id 3..6 уходит в `peek::on_hotkey`, id 7 в `shell::toggle_hidden`; `on_click` берёт ползунок в `drag` и зовёт `SetCapture`, `on_mouse_move` тащит его, `on_mouse_up` и `WM_CAPTURECHANGED` сбрасывают), st(), Hit/Tab/Redraw, wndproc-диспетчер, on_click, on_tick (счётчики нагрузки читает вне st(); сигнал таймера только при `settings.sound`; `bg::api::expire` отпускает хуки), on_notifs (карточка из группы чата; при «тишине» карточку не показывает, счётчик растёт), on_mouse_move/leave (над карточкой панель не раскрывается, `peek::set_hover`, `collapse_ms` из настроек), on_wheel (на «Событиях» листает карточки агентов: `feed_scroll`, в поиске в режиме «Буфер» — `input::scroll_list`), sys_sampling_tick, `WM_APP_LAYOUT` -> `shell::layout` (высота панели по содержимому вкладки)
shell.rs       441        окно: три вида (`MODE_COLLAPSED/PEEK/EXPANDED`, `mode(s)`), layout (после SetWindowPos сразу ui::render; размер карточки `Peek::width()` × `Peek::height()`: 440 или 600 у просьбы), свернувшись из поиска (`Tab::Commands`), панель возвращается на `input.back` (`set_expanded`); смена вида с анимацией (Anim, place, on_anim_timer, TIMER_ANIM), спрятать (`hidden`, `toggle_hidden`: Ctrl+Alt+H, окна нет вовсе), полноэкран (AppBar), трей (`tray_menu`: автозапуск, настройки, выход), DPI, форма (TOP_MARGIN, BOTTOM_R_*, EAR = 9: окно шире тела на 2×EAR; `COLLAPSED_MIN_W` 104, поля свёрнутой чёлки по 28 px), sys_page, `panel_h` (высота раскрытой панели по содержимому вкладки: `layout` берёт её, смена высоты идёт тем же `Anim`)
input.rs       261        строка поиска: `Input { mode, back, ... }`, `activate(hwnd, mode)`, `switch_mode`, `on_hotkey(hwnd, mode)`, Alt+1…9 (выполнить по номеру), Tab/Shift+Tab (режим), Del (убрать запись буфера), Ctrl+P (закрепить), `scroll_list`, вставка из буфера, close_panel
pill.rs        287        модель свёрнутой пилюли: `Facts` (всё, от чего зависит вид; в том числе `task_bar`, `unread`, `busy_app`) -> чистая `assemble(Facts) -> View { lead: Vec<Lead>, text, tone: Tone, level, viz, dim_from: Option<usize> (с какого байта текста серый хвост: заряд батареи, цвет `DIM`), plain, task_bar: Option<u8>, unread: usize }`; `Lead` = Mic | Cam | Mascot { count } | Art с `width()` (агент левее обложки); `View::left_width/right_width` (справа ещё счётчик: `UNREAD_GAP` + `UNREAD_W`), `DOT_GAP`; `view(s)` (факты из State, затем по очереди IN_USE, INBOX, MEDIA, каждый лок отдельно), `text(s)` = `view(s).text`; `shows_level`, `lead_width`, `mini_art`, `time_text` (4 теста, в том числе `stage1_extras`)

ui/            ~1950       GDI для текста + свой растеризатор, State только читает
  mod.rs        194       paint() (WM_PAINT: BeginPaint/EndPaint + render), render(): кадр в Surface (вид MODE_PEEK рисует `peek::draw`; во время анимации раскрытая панель в полной раскладке, по центру, обрезана размером окна), вкладки словами без капсул (TABS = 3: Музыка, Система, События; активная FG, наведённая DIM, остальные FAINT; в шапке шестерёнка `Hit::Settings`, пока открывает `shell::tray_menu`; «Система» — группа Controls + System с переключателем subtabs, SUB_H = 30; справа лупа `Hit::Mode(0)`, у поиска своя шапка `search::header`; на «Событиях» оранжевый счётчик агентов, ждущих разрешения, `agents.summary().0`; в шапке справа время и короткая батарея `osd::battery_short`). Высота раскрытой панели по содержимому вкладки: `Painter.bottom` (`reach`) + 20 в пределах 200..`EXPANDED_H`, пишется в `Shell.panel_h`; при смене `PostMessageW(WM_APP_LAYOUT)` вне `st()`
  surface.rs    279       Surface: постоянный 32-бит DIB + memory DC (thread_local UI-потока, пересоздаётся при смене размера), present() = UpdateLayeredWindowIndirect (ULW_ALPHA, prcDirty); Canvas: fill_round, round_off, clear, finish(rad_px, ear) (непрозрачность, вырез ушек и нижних углов, премультипликация)
  paint.rs      301       палитра «чёрное стекло» (ACCENT = FG белый, FAINT, INK: чёрный текст на белом, I_GEAR), R_TILE/R_ROW/R_PILL, face() (Segoe UI Variable Text, если есть SegUIVar.ttf), значки MDL2, Fonts, Painter (cv: Canvas, oy: сдвиг вниз, ox: сдвиг вправо на ухо, off() сдвигает по обеим осям, `bottom` и `reach(y)` (нижний край содержимого для высоты панели), art(): обложка со сглаженными углами, `text_wrap`: перенос текста для предпросмотра), виджеты slider_row (линия 3 px, число без «%»; volume_row через него)/input/icon_button
  collapsed.rs  153       свёрнутый вид по `pill::View` (цвет текста из `Tone`; level_bar: громкость и яркость; draw_left идёт по `View::lead`: точки, мини-обложка, маскот; 4 столбика визуализатора справа от текста; синий счётчик пропущенных уведомлений справа от текста; `progress(p, rc, task)` рисует полоску задачи скрипта вместо прогресса трека)
  viz.rs         52       VizArea, draw() через Canvas, on_frame(): обновляет только прямоугольник полосок в постоянном буфере (present(Some(rect))); полоски светлые (212,212,216), растут от нижнего края, скругление 4 px
  music.rs  93 (имя устройства вывода плашкой; обложка 148 px, спектр на всю ширину под перемоткой y 248..288, громкость y 300)
  search.rs 283       поиск (`Tab::Commands`): header с режимами (`Hit::Mode`), общий список с группами и номерами Alt (`list_mode`), режим «Буфер» с предпросмотром выбранной записи (`clip_mode`, `CLIP_ROWS`), у выбранной строки «↵», у остальных номер Alt, без строки подсказок, плитки частых программ (`Hit::AppTile`) при пустой строке. Бывшие `ui/clip.rs` и `ui/cmd.rs` удалены
  ctl.rs    163       «Управление»: плитки 3×2 (`TILE_H` 52, значок слева, название и состояние словами; у Wi-Fi имя сети), строка яркости, если BRIGHTNESS = Some; кнопка «Выключить / включить микрофон» (`CtlHit::MicMute`), пока микрофон кто-то держит или он выключен; питание четырьмя круглыми кнопками 34 px справа (контурные; значок выключения красный, подтверждение красной заливкой; включённая плитка белая с чёрным текстом), слева батарея без плашки, под курсором название кнопки, при подтверждении «…: нажми ещё раз» красным
  sys.rs        117       вкладка «Монитор»: карточки 2×2 (144×96) с историей за минуту ступенчатой линией (`spark`), строка диска под ними, топ-5 приложений в правой колонке; WARN при ≥85%. Отдельного графика ЦП нет
  mascot.rs     122       маскот агента: свой персонаж 10×9 с ножками и белыми глазами из прямоугольников, цвет тела по хешу имени (шесть цветов, без синего, оранжевого, зелёного и красного: они заняты под состояния), лицо по состоянию; «кадр» берётся из `State.notif_flip`, своего таймера нет
  peek.rs       199       карточка из чёлки (ширина по `Peek::width()`; карточка просьбы: маскот, проект, ветка, «ты: запрос», инструмент, блок строк кода («−» красным, «+» зелёным), кнопки с клавишами Alt+Y/A/N/T, «ещё N в очереди», подсказка; клик мимо кнопок у просьбы ничего не делает): у уведомления кружок с буквой отправителя (цвет по имени), имя, счётчик, программа, текст до двух строк (`Painter::text_wrap`) или до трёх последних сообщений чата, кнопки «Открыть / Скрыть / Тишина на час» под курсором; у просьбы агента «Отклонить»/«Разрешить» (если `answerable`), `Hit::Peek`. Значка программы на кружке нет
  feed.rs       213       вкладка «События»: переключатель «Агенты / Уведомления» (`Hit::FeedPage`) и сводка; «Агенты» — карточки сессий 50 px (`CARD_H`, шаг 56, до `CARDS` = 5; маскот, имя, ветка git, инструмент, последний ответ, возраст, крестик; сначала ждущие разрешения, `Agents::order`), прокрутка колесом (`State.feed_scroll`); «Уведомления» (карточки чатов 46 px, шаг 50, `notifs::groups`; подпись программы с числом уведомлений, кружок с буквой, имя, счётчик, последнее сообщение, возраст; клик по карточке убирает весь чат, `Hit::NotifGroup`)

feat/          ~3300      логика функций, без GDI
  api.rs        485       локальный API (`sound()`: `MessageBeep` MB_ICONQUESTION на просьбу, MB_ICONASTERISK на «закончил», если `settings.sound`; карточка просьбы собирается из данных агента): handle(buf) -> (Reply, Option<Event>), expected_len, parse_form, url_decode, Tasks (apply/expire/badges/bar: прогресс задачи для полоски), static QUEUE, on_events; эндпоинты /hook, /agent и /open (`Event::Open(tab, query)`: раскрыть панель на вкладке `music|ctl|mon|feed|notifs` или открыть поиск в режиме `cmd|apps|clip|act` с запросом `q`, другое значение сворачивает; для поиска обработчик внутри `st()` зовёт `cmd::scan()`: нарушение, см. отчёт), Event::Agent; после `agents.apply` `on_events` вне `st()` читает ветку git (`agents::read_branch`); on_events превращает `agents::Notice` в карточки (`peek::show/resolve`) и отпускает хуки (`bg::api::answer`) (5 тестов)
  agents.rs     608       агенты: AgentState (Work/Ask/Wait/Done/Gone), Update (`answerable`: хук ждёт решения; `extra: Extra { tool, cwd, answer, ask_lines, ask_stat, suggestions }`), Agent (+ `tool`, `answer`, `branch`, `cwd`, `ask_lines`, `ask_stat`, `ask_always`), `ask_detail()` (чистая: строки кода просьбы и счётчики, тест `ask_details`; PermissionRequest не затирает `note`, запрос человека сохраняется; `json_fields` берёт и вложенные массивы), Notice (Ask/Done/Resolved), Agents (apply/expire/badges/set_state/attention, wants_branch/set_branch, summary, order), branch_from_head (чистая), read_branch (читает `.git/HEAD`, только вне `st()`), from_hook (JSON хука Claude Code -> событие), json_strings/json_fields (свой разбор строковых полей верхнего уровня, терпит обрыв), one_line, fmt_age (5 тестов, в том числе `branch_and_session_details`)
  notifs.rs     198       чужие уведомления: Notif (+ `aumid`), Group и `groups()` (чат = программа + заголовок; до трёх последних строк, счётчик, `newest`), Inbox (update/`forget_group`; `forget` только для тестов; `unread` растёт в `update` на число незнакомых id, первое чтение не считается; сбрасывается в `app::on_tab` и `shell::set_expanded` на «Событиях»), static INBOX, static OFF (AtomicBool: доступа нет), flash_text, age, PEEK_MS = 5000 (4 теста, в том числе `groups_by_chat`)
  clip.rs       277       History, `Entry.time` (секунды Unix, 0 = неизвестно), `Kind`/`kind()` (текст, ссылка, путь, цвет), `now_secs`, `age_text`, `parse_line` (читает старый формат «флаг TAB текст» и новый «флаг TAB время TAB текст»), ClipHit (без `Row`: вставка идёт из поиска), on_hit/on_update (тест `file_formats_and_kinds`)
  cmd.rs        465       бывший launcher.rs: поиск, калькулятор, `MODES`/`MODE_ALL|APPS|CLIP|ACT`, CmdRes (+ `Clip(usize)`), `results(s)` по режиму, run_index, `frequent()`, счётчики запусков `CmdState.freq` (`load_freq`/`parse_freq`/`save_freq` -> `%APPDATA%\Pill\launches.txt`, запись вне `st()`), `run_app`
  ctl.rs        184       Ctl, Pw, CtlHit (+ `MicMute`), Snapshot (+ `out_dev`, `mic_muted`), CtlState, refresh (COM в UI-потоке, см. раздел 1), volume_at/volume_wheel, brightness_at/brightness_wheel
  music.rs       29       MusicHit
  osd.rs        213       Flash (Volume/Brightness/Text), Osd (кэш батареи), on_volume, on_audio_device, on_power, battery_event (тест)
  timers.rs      77       Timers: tick/add/cancel/dismiss (тест)
  peek.rs       244       карточка из чёлки: Peek (`Peek::new`, поля `lines`, `count`, `aumid`, `hover`, у просьбы `tool`, `branch`, `stat`, `can_always`), PeekKind (Notif/Ask{id, answerable}/Done), PeekHit (Allow/Always/Deny/Dismiss/Open/Quiet), show/hide/resolve/on_hit/on_hotkey, `set_hover` (зовёт layout), W = 440, W_ASK = 600, `width()`, `code_height()` (18 на строку + 16), height(): уведомление по содержимому (38 + строки + 12, с кнопками + 50), просьба 88 + код + 10 + 28 + 10 + 18 + 12, «закончил» 86; очередь просьб `State.asks` (`show` при занятой карточке-просьбе ставит просьбу в очередь, `hide` показывает следующую, `resolve` убирает и из очереди); глобальные хоткеи 3..6 (Alt+Y/A/N/T) `hotkeys()` регистрирует в `show` и снимает в `hide`, только пока просьба с ответом на экране; Open = `shell:AppsFolder\AUMID`, Quiet = `State.quiet_until` на час; `TIMER_PEEK`. В «просто точке» show ничего не рисует и отпускает ждущий хук без решения, после отказа от карточки зовёт layout (ширина чёлки меняется из-за счётчика)
  settings.rs   122       настройки: Settings { collapse_ms (зажат 100..5000), dot_mode, sound, extra }, parse/serialize (чистые, 2 теста), load/save; файл `%APPDATA%\Pill\settings.txt`
  viz.rs        259       DSP без Win32: FFT 1024, окно Ханна, 24 полосы 40 Гц–16 кГц, group(); static SPECTRUM, WANT/WAKE/PENDING (4 теста)
  sysmon.rs     501       монитор системы: cpu_pct, parse_processes (смещения SYSTEM_PROCESS_INFORMATION x64), apps() (сложение по имени), top_by_cpu/top_by_ram, gpu_busy, Snapshot (+ `ram_hist`, `gpu_hist`, `push_hist`) + static SNAPSHOT, Sampler (живёт в winrt-воркере, пока вкладка открыта), WarnProbe, LoadWatch (≥90% 30 с подряд -> Warn; за 2 с до порога Probe; после предупреждения молчит до 20 с спокойствия) + static LOAD_WARN, fmt_bytes/fmt_rate (6 тестов)

sys/           ~1125       тонкие обёртки над Windows, без State
  audio.rs 177 (громкость, `output_name` (PKEY_Device_FriendlyName), `mic_muted`/`toggle_mic` + Watch с колбэками)  autostart.rs 23  clipboard.rs 69 (read_text/set_text)
  icon.rs 47  launch.rs 84 (open, run_line, Ctrl+V, force_foreground, hidden)  net.rs 62 (VPN, только из воркера)
  display.rs 213 (яркость: WMI на ноутбуке, DDC/CI на ПК, struct Displays для воркера, to_pct/from_pct с тестом)
  loopback.rs 104 (WASAPI loopback устройства вывода: shared, буфер 200 мс, float32 и PCM16 -> моно; Err при пропаже устройства)
  power.rs 80 (battery() -> Battery, register_notify/parse_notify -> PowerEvent, lock/sleep/restart/shutdown)  theme.rs 48 (set_dark пишет реестр, broadcast() рассылает WM_SETTINGCHANGE, только из воркера)
  toasts.rs 82 (Listener поверх UserNotificationListener: open/read/remove, `Toast.aumid`, кэш прочитанных по id, now_filetime; только из воркера)
  sysinfo.rs 210 (cpu_times, memory, cpu_count, processes(buf) через NtQuerySystemInformation, net_totals через GetIfTable2 (только железо), system_disk, struct Disks (IOCTL_DISK_PERFORMANCE на \\.\PhysicalDrive0..15, доступ 0), struct Gpu (D3DKMTEnumAdapters2 + D3DKMTQueryStatistics))

bg/            ~890       фоновые потоки
  winrt.rs      517       бывший media.rs: GSMTC, обложки, радио (и имя Wi-Fi: `static WIFI_NAME`, `wifi_name()` перебирает все профили `NetworkInformation`), VPN, рассылка темы, яркость (Displays), снимки системы (Sampler), чужие уведомления (Listener)
  api.rs        135       локальный HTTP-сервер: TcpListener 127.0.0.1:47821, спит в accept, по одному соединению; `static HELD` (хуки PermissionRequest, которым ещё не ответили: id, соединение, время, правила `permission_suggestions` как строка JSON), `enum Answer { Allow, Always, Deny }`, `answer(id, Option<Answer>)` пишет ответ в сокет из UI-потока (`Always` отвечает allow + `updatedPermissions` = присланные правила, без правил это обычное «разрешить»), `expire()` отпускает через `HOLD` = 30 с; ответы ALLOW/DENY, `respond_text`
  watch.rs      258       бывший privacy.rs: микрофон/камера (RegNotifyChangeKeyValue) и захват звука для визуализатора
```

Всего 9737 строк `src/**/*.rs` (`wc -l`, коммит 5d7b66d). Правила зависимостей:
- `sys` и `bg` не знают `State`.
- `feat` может звать слой приложения: `app::st`, `input::close_panel`/`paste`, `shell::layout`.
- `ui` читает данные `feat` и `State`, но feat-логику не вызывает.
- `app` знает всех и маршрутизирует.

State: `{ shell, input (с `mode`, `back`), hits, hover, clip, cmd, ctl, timers, osd, viz_area, load, sys_sampling, tasks, agents, notif_flip, peek, drag (ползунок, который тащат: `Hit` и его полоска), asks (очередь просьб агентов, на экране одна), quiet_until, feed_notifs, feed_scroll, settings }`. `Tab`: Music, Commands, Controls, System, Feed (`Clipboard` удалён; `Commands` — строка поиска, в `TABS` её нет; в шапке 3 вкладки: Музыка, Система, События; Controls и System — страницы группы «Система», `Shell.sys_page` помнит последнюю).
Hit: `Tab, Input, Volume, Brightness, Music(MusicHit), Clip(ClipHit), Cmd(usize) (номер в `cmd::results`), Mode(u8), Settings, AppTile(usize), Ctl(CtlHit), FeedPage(bool), AgentDrop(usize), NotifGroup(u32) (id самого свежего уведомления чата), Peek(PeekHit) (PeekHit: Allow, Always, Deny, Dismiss, Open, Quiet), Alarm`. `enum Redraw { None, Content, Layout }`.

---

## 3. Потоки и владение данными

```
┌──────────────── UI-поток (STA) ────────────────┐        ┌──── WinRT-воркер (MTA) ────┐
│ GetMessage → wndproc → обработчик → State      │ Job →  │ mpsc::Receiver<Job>         │
│ thread_local STATE: RefCell<Option<State>>     │ ─────> │ владеет GSMTC-сессией,      │
│ render() читает State + статики media           │        │ подписками, Radio-объектами │
│                                                │ <───── │ пишет в static MEDIA/RADIOS │
│                                                │PostMsg │ PostMessage(WM_APP_MEDIA..) │
└────────────────────────────────────────────────┘        └─────────────────────────────┘
        ↑ ещё два потока шлют PostMessage: наблюдатель (bg/watch.rs) и API (bg/api.rs)
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
2. WinRT-воркер (`bg/winrt.rs`): GSMTC, радио, VPN (`Job::Vpn` читает pbk/RAS в `static VPN` + `WM_APP_RADIO`, `Job::ToggleVpn`; `Job::Radios` читает радио в `static RADIOS` и имя Wi-Fi в `static WIFI_NAME` (`wifi_name()`, `Networking_Connectivity`), оба под `WM_APP_RADIO` запускает rasdial), рассылка смены темы (`Job::ThemeBroadcast`: `sys::theme::broadcast`, а `set_dark` только пишет реестр в UI) и яркость (`Job::Brightness`, `Job::SetBrightness(u8)`, в пачке выполняется только последний; результат в `static BRIGHTNESS: Mutex<Option<u8>>`, сообщение `WM_APP_BRIGHT`). `Displays` (WMI/DDC) живёт в этом потоке. Монитор системы: `Job::SysSample` (UI шлёт на секундном тике и при клике по вкладке, пока открыта «Система»; воркер держит `Sampler`, кладёт `Snapshot` в `static sysmon::SNAPSHOT`, `WM_APP_SYS`), `Job::SysStop` (вкладку закрыли: `Sampler` и снимок сбрасываются). Предупреждение: `Job::LoadProbe` (за 2 с до порога снять процессы), `Job::LoadWarn(Load)` (собрать текст с именем процесса в `static LOAD_WARN`, `WM_APP_SYS`). Снимок ~5 мс, из них ~4,5 мс список процессов; на это время воркер занят (медиа-события ждут). Чужие уведомления: `Job::Notifs` (UI шлёт на каждом втором секундном тике, пока пилюлю видно, и при открытии «Событий»; воркер один раз открывает `toasts::Listener`, читает `GetNotificationsAsync`, кладёт список в `static notifs::INBOX`, `WM_APP_NOTIF`), `Job::NotifDrop(id)` (убрать и из центра уведомлений Windows, перечитать). Нет доступа — `static notifs::OFF`, и UI больше не шлёт. **Это опрос раз в 2 с в обход правила «только в раскрытом виде»: ждёт согласования (раздел 1).**
3. Наблюдатель (`bg/watch.rs`, MTA, инициализирует COM): один `WaitForMultipleObjects` по `[реестр микрофона, реестр камеры (если открылись), событие пробуждения визуализатора]`. Микрофон/камера: `WM_APP_PRIVACY`, результат в `static IN_USE`. Визуализатор: пока `viz::want()` — тайм-аут 33 мс, на тайм-ауте читает WASAPI loopback (`sys/loopback.rs`), считает спектр (`feat/viz.rs`), кладёт в `static SPECTRUM: Mutex<[f32;24]>` и шлёт `WM_APP_VIZ`; иначе `INFINITE`. `PENDING` (AtomicBool): кадр отправлен, но UI его ещё не нарисовал, новые не шлются. UI будит наблюдателя через `SetEvent(WAKE)` при смене `WANT` (`set_want`). Устройство пропало — переоткрытие не чаще раза в секунду. В тишине, когда полоски уже на нуле, UI не будится. Тему пока не слушает.
4. Локальный API (`bg/api.rs`, без COM): `TcpListener` на `127.0.0.1:47821`, в простое спит в `accept` (0% CPU). Соединения по одному, таймаут чтения/записи 2 с, запрос ≤ 16 КБ. Хук `PermissionRequest` (`Update.answerable`) соединение не закрывает: оно кладётся в `static HELD` (id, `TcpStream`, время, правила `permission_suggestions` для «Всегда»), поток идёт дальше в `accept`; ответ пишет UI-поток через `bg::api::answer(id, Some(Answer::Allow/Always/Deny)/None)` (кнопка в карточке или Alt+Y/A/N/T, `Notice::Resolved`, `expire()` на секундном тике через `HOLD` = 30 с). Запись в локальный сокет мгновенная (таймаут записи 2 с). Разбор и ответ — `feat/api.rs::handle` (чистая функция), событие кладётся в `static feat::api::QUEUE` и шлётся `WM_APP_API`. Через него же ходят агенты: `POST /hook` (JSON хука Claude Code, ответ 200 с пустым телом: stdout хука попадает в контекст Claude; тело длиннее `MAX_REQUEST` разбирается обрезанным) и `/agent`. Порт занят — API молча выключен, остальное работает.

События питания (`WM_POWERBROADCAST`) приходят в UI-поток, своего потока не требуют.

Буфер окна `ui::surface::SURFACE` — `thread_local` UI-потока, другим потокам недоступен (визуализатор рисует в него в UI по `WM_APP_VIZ`). `UpdateLayeredWindowIndirect` и `Surface::present` зовутся вне `st()`.

Статики для обмена между потоками: `MEDIA`, `RADIOS`, `WIFI_NAME`, `BRIGHTNESS`, `IN_USE`, `SPECTRUM`, `SNAPSHOT`, `LOAD_WARN`, `QUEUE` (API), `INBOX` (уведомления), `HELD` (ждущие хуки, `bg/api.rs`) (мьютексы), `WANT`, `WAKE`, `PENDING`, `OFF` (уведомления недоступны) (атомики).

COM-колбэки громкости приходят в чужом потоке COM, внутри только `PostMessage`.

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
  → WM_PAINT → ui::paint → ui::render: рисует в Surface, заново заполняет State::hits, выводит UpdateLayeredWindow
    (layout после SetWindowPos зовёт ui::render сам)
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
| `WM_APP_SYS` | `WM_APP+10` | bg/winrt.rs | воркер: снимок системы (`static SNAPSHOT`) или предупреждение о нагрузке (`static LOAD_WARN`) готово |
| `WM_APP_API` | `WM_APP+11` | bg/api.rs | поток API: в `static QUEUE` лежат события (уведомление, задача) |
| `WM_APP_NOTIF` | `WM_APP+12` | bg/winrt.rs | воркер: список чужих уведомлений поменялся или пришло новое (`static notifs::INBOX`) |
| `WM_APP_LAYOUT` | `WM_APP+13` | ui/mod.rs | отрисовка: высота раскрытой панели по содержимому вкладки поменялась (`Shell.panel_h`); шлётся `PostMessageW` вне `st()`, обработчик в `app.rs` зовёт `shell::layout` |
| `WM_POWERBROADCAST` / `PBT_POWERSETTINGCHANGE` | из крейта | sys/power.rs | система: `GUID_ACDC_POWER_SOURCE`, `GUID_BATTERY_PERCENTAGE_REMAINING`, `GUID_VIDEO_CURRENT_MONITOR_BRIGHTNESS` (`RegisterPowerSettingNotification`) |
| `WM_MOUSELEAVE` | `0x02A3` | система | нет в крейте, объявлен в ids.rs |
| `WM_CLIPBOARDUPDATE` | из крейта | система | `AddClipboardFormatListener` |
| `TaskbarCreated` | `RegisterWindowMessage` | shell.rs | Explorer перезапустился |

Свободный номер: `+14`. Карточке из чёлки и настройкам своих `WM_APP_*` не потребовалось: карточку показывают `WM_APP_API` и `WM_APP_NOTIF`, настройки меняются из меню трея. Питание и яркость ноутбука своего `WM_APP_*` не имеют: это системный `WM_POWERBROADCAST` (раздел 7).

**Таймеры** (`SetTimer`, id в `wparam`)

| Id | Имя | Период | Когда работает |
|---|---|---|---|
| 1 | `TIMER_CLOCK` | 1000 мс | всегда (единственный постоянный) |
| 2 | `TIMER_COLLAPSE` | `settings.collapse_ms` (по умолчанию 350 мс, 200/350/600/1000 из трея), разовый | после ухода курсора |
| 3 | `TIMER_PASTE` | 80 мс, разовый | после выбора записи буфера |
| 4 | `TIMER_CTL` | 3000 мс, разовый | после переключения VPN |
| 5 | `TIMER_FLASH` | разовый, длительность задаёт вызов | убрать временную надпись: громкость, яркость, зарядка, предупреждения |
| 6 | `TIMER_SHOT` | 250 мс, разовый | открыть выделение области, когда панель уже свернулась |
| 7 | `TIMER_ANIM` | 15 мс, ~200 мс (`ANIM_MS`) | **короткоживущий, не постоянный**: только пока идёт смена вида (свёрнуто, карточка, раскрыто), по окончании `KillTimer` (`shell::on_anim_timer`) |
| 8 | `TIMER_PEEK` | разовый: `ms` из вызова `peek::show` (уведомление 5 с, «закончил» 4 с, просьба без кнопок 8 с), 2,5 с после ухода курсора с карточки | спрятать карточку из чёлки; просьба с кнопками (`answerable`) таймера не имеет, ждёт ответа |

**Прочее**

| Что | Значение |
|---|---|
| Хоткеи | 1 `HOTKEY_CMD` Alt+Space (запасной Ctrl+Alt+Space) открывает поиск в режиме «Всё», 2 `HOTKEY_CLIP` Ctrl+Alt+V: в режиме «Буфер» (`cmd::MODE_CLIP`); повтор в том же режиме закрывает панель. 3 `HOTKEY_ALLOW` Alt+Y, 4 `HOTKEY_ALWAYS` Alt+A, 5 `HOTKEY_DENY` Alt+N, 6 `HOTKEY_TERM` Alt+T (в терминал: хук отпускается без решения), `MOD_NOREPEAT`: **только пока просьба агента с ответом на экране** (`peek::hotkeys` в `show`/`hide`; Pill не берёт фокус, поэтому клавиши глобальные и на это время перехватывают Alt+Y/A/N/T у других программ). Внутри поиска (не глобальные): Alt+1…9, Tab/Shift+Tab, Del, Ctrl+P. 7 `HOTKEY_HIDE` Ctrl+Alt+H (`MOD_NOREPEAT`): прячет пилюлю целиком (окна нет, клики идут под ней), повтор возвращает; хоткей поиска, Ctrl+Alt+V и просьба агента тоже возвращают |
| Пункты меню трея | 10 `MENU_AUTOSTART`, 11 `MENU_EXIT`, 12 `MENU_DOT` («Просто точка»), 13 `MENU_SOUND` («Звуки: таймер, просьбы агентов, «закончил»»), 20..=23 `MENU_DELAY` (подменю «Сворачиваться после ухода курсора»: 200, 350, 600, 1000 мс) |
| Права Windows | Доступ к чужим уведомлениям (`UserNotificationListener`), запрашивается из воркера; нет доступа — `notifs::OFF` |
| Сеть | `127.0.0.1:47821` (TCP, только loopback; `PORT` в `feat/api.rs`). Адреса: `/ping`, `/notify`, `/task`, `/agent`, `/hook`, `/open?tab=music\|ctl\|mon\|feed\|notifs\|cmd\|apps\|clip\|act[&q=запрос]` (раскрыть панель на вкладке или открыть поиск в режиме «Всё / Программы / Буфер / Действия» с запросом `q`, другое значение сворачивает; для скриптов и сверки вида по снимку экрана) |
| Мьютекс | `Local\PillSingleInstance` |
| Класс окна | `PillWindow` |
| Реестр (пишем) | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` → `Pill`; `...\Themes\Personalize` (`AppsUseLightTheme`, `SystemUsesLightTheme`) |
| Реестр (читаем) | `HKCU\Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\{microphone,webcam}` (+ `NonPackaged`), наблюдает `bg/watch.rs` |
| Файлы | `%APPDATA%\Pill\clipboard.txt` (история; строка «флаг TAB время TAB текст», старый формат без времени читается, пишется всегда новый), `%APPDATA%\Pill\launches.txt` (счётчики запусков программ из поиска, строки «число TAB имя в нижнем регистре»; читается при старте, пишется в `cmd::run_app` вне `st()`), `%APPDATA%\Pill\crash.log` (последняя паника), `%APPDATA%\Pill\settings.txt` (настройки: `collapse_ms`, `dot_mode`, `sound`; читается при старте, пишется из меню трея вне `st()`) |
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

- **Модель (сделана, `pill.rs`)**: функции отдают факты (`Facts`), чистая `assemble` раскладывает их в `View` по трём слотам. Слотов-объектов с приоритетом нет: приоритет только у центра.
  ```
  [ левый слот: Lead ]   [ центр: text + Tone ]   [ правый слот: viz, счётчик ]
   Mic · Cam · Mascot · Art   [программа] время · задачи · ⏱ · батарея   4 столбика · (N)
  ```
  Порядок значков слева фиксирован: микрофон, камера, маскот агента (с числом, если агентов больше одного), обложка (только пока играет и обложка есть). Справа спектр, пока играет музыка, и синий счётчик пропущенных чужих уведомлений (`View.unread`, пока больше 0; `UNREAD_GAP` + `UNREAD_W`, «9+» от 10). Ширина = сумма `Lead::width()` + `DOT_GAP` после точек.
  Имя программы, державшей микрофон или камеру (`Facts.busy_app`, до 16 символов), пишется в центре перед временем. Полоска задачи скрипта (`View.task_bar` = `Tasks::bar()`) рисуется вместо полоски прогресса трека (`collapsed::progress`). Счётчик растёт на новые уведомления и обнуляется при открытии «Событий». Всё это только в обычном виде: при тревоге и `Flash::Text` `task_bar` и `unread` пустые (тест `stage1_extras`). Выключение микрофона кликом по свёрнутой пилюле не сделано: наведение сразу раскрывает панель.
- **Приоритет центра** (сверху важнее): тревога таймера (`Tone::Accent`) -> временная надпись `Flash::Text` (Accent или Warn) -> полоска уровня `Flash::Volume/Brightness` -> обычный вид. Пока центр занят первыми тремя, значки и спектр не показываются.
- **OSD (временный режим)** перекрывает обычный вид на ~1,5 с: «громкость 40% ▬▬▬▭▭», «яркость», «зарядка подключена», «батарея 10%». В `State` одно поле `osd: Option<Osd { kind, value, until }>` и одноразовый `TIMER_OSD`. Новое событие перезаписывает старое, таймер перезапускается.
- Наведение мышью всегда раскрывает панель, поверх всего.
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
| Монитор системы (сделано) | События нет. Раз в секунду, только пока открыта вкладка «Система»: `Job::SysSample` в воркер (`GetSystemTimes`, `GlobalMemoryStatusEx`, `NtQuerySystemInformation`, `GetIfTable2`, `GetDiskFreeSpaceExW`, `IOCTL_DISK_PERFORMANCE`, `D3DKMTQueryStatistics`) → `WM_APP_SYS`. PDH не используем (раздел 10) | UI → `bg/winrt.rs` → UI | да, 1 Гц, только пока вкладка открыта |
| Предупреждение о нагрузке (сделано) | `LoadWatch::tick` на секундном тике UI (два дешёвых вызова: `GetSystemTimes`, `GlobalMemoryStatusEx`), ≥90% 30 с подряд → `Job::LoadWarn` → `WM_APP_SYS` → `osd::Flash::Text` на 8 с | UI + `bg/winrt.rs` | да, 1 Гц, в том числе свёрнутым: согласованное исключение (раздел 1) |
| Локальный API для скриптов (сделано) | `TcpListener` на `127.0.0.1:47821` (std, без крейтов): `accept` блокирует поток → `WM_APP_API` → `api::on_events` (`osd::Flash::Text` и `State.tasks`) | `bg/api.rs` → UI | нет, поток спит в `accept` |
| Агенты в чёлке (сделано) | События нет, их шлют сами: хуки Claude Code (`curl.exe`, `async: true`, кроме `PermissionRequest`: он синхронный, `timeout` 45; `UserPromptSubmit`, `PostToolUse`, `PermissionRequest`, `Notification`, `Stop`, `SessionEnd`) и скрипты через `POST /hook`, `/agent` → `WM_APP_API` → `agents.apply` → `Notice` → карточка (`peek::show`). Хук `PermissionRequest` ждёт: соединение лежит в `HELD`, ответ (разрешить/всегда/отклонить/без решения) пишет UI-поток по кнопке карточки или по Alt+Y/A/N/T (`WM_HOTKEY`, только пока просьба на экране) | `bg/api.rs` → UI → `bg/api.rs` | нет, поток спит в `accept`. Протухание на секундном тике (`agents.expire`, `bg::api::expire`, 30 с) |
| Чужие уведомления (сделано иначе) | Событие `NotificationChanged` непакетному приложению недоступно (0x80070490, проверено); файл `wpndatabase.db*` при новом тосте событий каталога не даёт (проверено `FileSystemWatcher`); Bot API Telegram не видит личные сообщения. Поэтому `UserNotificationListener::GetNotificationsAsync` раз в 2 с (`Job::Notifs`) → `WM_APP_NOTIF` → карточка из чёлки на 5 с (`peek::show`, `notifs::PEEK_MS`); карточка собирается из группы чата (`notifs::groups`: те же программа и заголовок), пока курсор на ней, не прячется; «Тишина на час» (`State.quiet_until`) карточки не показывает, счётчик пропущенных растёт | UI → `bg/winrt.rs` → UI | **да, 2 с, в том числе свёрнутым: ждёт согласования (раздел 1)** |
| Настройки (сделано) | Файл `%APPDATA%\Pill\settings.txt`, читается один раз при старте (`State.settings`), пишется из меню трея; следить за файлом не нужно | UI | нет |
| Файловая полка | `IDropTarget` (приём) + `DoDragDrop` с `IDataObject` (отдача), нужен `OleInitialize` | UI | — |

---

## 8. План переезда

Каждый шаг — отдельный коммит, после каждого `cargo build --release` проходит и поведение не меняется. Делаем **до партии 1**, иначе потом переносить в два раза больше.

1. [x] `ids.rs`: вынести все константы номеров (WM_APP из `media.rs` тоже).
2. [x] `sys.rs` → `sys/` по файлам.
3. [x] `ui.rs` → `ui/mod.rs` + `ui/paint.rs` + по файлу на вкладку.
4. [x] `shell.rs` (layout, раскрытие, полноэкран, трей) и `input.rs` (клавиатура) из `main.rs`.
5. [x] Разбить `State` на части, а `Hit` на вложенные enum'ы. `on_click` стал диспетчером.
6. [x] `pill.rs`: `Facts` -> `assemble` -> `View` (слоты, приоритет центра), OSD работает через `feat/osd.rs` и `TIMER_FLASH`. 36 тестов.
7. [x] `media.rs` → `bg/winrt.rs` (и `privacy.rs` → `bg/watch.rs`).

Разделы 2 и 5 обновлены после шагов 1–5 и 7 и после партии «яркость + батарея событиями» , после визуализатора, монитора системы, локального API, агентов, уведомлений слоёного окна, ушек, анимации раскрытия, карточки из чёлки, маскотов и настроек (кусок 0.1 из PLAN.md), этапов 1 и 3 (свёрнутая пилюля: счётчик, полоска задачи, имя программы; перерисовка «Музыки», «Управления», «Монитора»). Реестр номеров (раздел 5) не менялся, кроме адреса `/open`: новых `WM_APP_*`, таймеров, хоткеев и файлов нет. 14.11 добавлены имя Wi-Fi, устройство вывода, кнопка микрофона и карточки сессий агентов (этап 4 из PLAN.md). Позже 14.11: этап 5 (уведомления карточкой по чатам: `notifs::groups`, `Peek::new`, `set_hover`, `quiet_until`, `Hit::NotifGroup` вместо `NotifDrop`). Позже 14.11: этап 2 (куски 2.1–2.3): поиск вместо вкладок «Буфер» и «Команды»; разделы 2 и 5 обновлены (хоткеи открывают режимы, файл `launches.txt`, новый формат `clipboard.txt`, `/open` с режимами и `q`). Новых `WM_APP_*`, таймеров и потоков нет (этап 5 тоже). Позже 14.11: этап 6 (куски 6.0–6.2 и звуки): хоткеи 3..6 в разделе 5 (регистрируются только пока просьба на экране), `State.asks`, `PeekHit::Always`, `bg::api::Answer`; новых `WM_APP_*`, таймеров и потоков нет. Позже 14.11: этап 11 (Ctrl+Alt+H: хоткей 7 в разделе 5, `State.drag`, порядок маскота и обложки в разделе 7); новых `WM_APP_*`, таймеров и потоков нет.

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
| Win32 + GDI, без фреймворков | RAM и CPU. WebView2 дал Island 950 МБ | Нет сглаживания и альфы у GDI: решено через `UpdateLayeredWindow` + свой растеризатор скруглений (см. ниже), Direct2D не нужен. Текст остался GDI |
| Тулчейн `x86_64-pc-windows-gnu` | VS Build Tools не ставятся (ошибка 5008) | Нет C-компилятора, значит нет SQLite и других крейтов с C-кодом |
| История буфера в текстовом файле | См. выше. Сотня записей, база не нужна | Нет индекса. При 100 записях не важно |
| `State` в `thread_local` + `RefCell` | Однопоточный UI, без `Mutex` и `unsafe static mut` | Правило `st()` из раздела 4 |
| Hit-зоны записываются при отрисовке | Раскладка и клики не расходятся | Клик до первого `paint` ничего не находит (не страшно) |
| Окно `WS_EX_LAYERED` + `UpdateLayeredWindowIndirect`, форма и скругления рисуются в своём 32-битном буфере (`ui/surface.rs`: `fill_round`, `finish`), GDI только для текста. `SetWindowRgn`/`CreateRoundRectRgn` убраны. Прежнее решение (регион) заменено | Регион давал лесенку; слоёное окно даёт сглаженные края. GDI обнуляет альфу под текстом, поэтому `finish` делает кадр непрозрачным. Постоянный буфер: ~3,8 МБ на раскрытой панели при 200% (в свёрнутом виде маленький). Сам кадр вне `WM_PAINT` (`layout` зовёт `render`) |
| Один WinRT-воркер на всё WinRT | Блокирующие `.get()` не трогают UI, а MTA нужен для событий | Медленная операция (обложка) задерживает остальные задачи. Пока терпимо |
| WMI для яркости только при наличии батареи, на ПК DDC/CI | DLL WMI добавляют ~3 МБ WS, на ПК WMI не нужен | Ноутбук без батареи (редкость) получит яркость только по DDC/CI |
| Кадр визуализатора перерисовывает только свой прямоугольник в постоянном буфере (`ui/viz.rs::on_frame`: `Canvas::fill_round` фона и полосок, `present(Some(rect))` с `prcDirty`), а не `InvalidateRect` (раньше `GetDC` + `BitBlt`) | 30 раз в секунду перерисовывать всё окно со шрифтами и обложкой заметно по CPU | Кадр рисуется мимо `render`; прямоугольник берётся из `State.viz_area`, который обновляет каждый `render`. Буфер должен жить между кадрами (пересоздаётся при смене размера, `layout` сразу зовёт `render`) |
| Захват звука и расчёт кадров в `bg/watch.rs`, без нового потока | Лимит потоков, наблюдатель и так ждёт на `WaitForMultipleObjects` | Наблюдатель перестал быть чисто событийным: пока играет музыка, просыпается 30 раз в секунду |
| Монитор системы без PDH: `GetSystemTimes`, `NtQuerySystemInformation`, `GetIfTable2`, `IOCTL_DISK_PERFORMANCE` (`\\.\PhysicalDriveN`, доступ 0, без админа), `D3DKMTQueryStatistics` (GPU) | Замер: счётчик «GPU Engine(*)» добавлял ~7 МБ WS / ~6 МБ приватной, даже один `PhysicalDisk(_Total)` ~5 МБ, и после `PdhCloseQuery` память не возвращалась. D3DKMT и IOCTL почти ничего не стоят | `NtQuerySystemInformation` и D3DKMT недокументированы/полудокументированы: смещения `SYSTEM_PROCESS_INFORMATION` захардкожены под x64, GPU берётся как самый занятый движок, без разбивки по процессам |
| Предупреждение о нагрузке считается на секундном тике и в свёрнутом виде | Нужно ловить нагрузку, когда вкладка закрыта; цена: два вызова в секунду, нового таймера и потока нет | Исключение из правила «опрос только в раскрытом виде» (раздел 1) |
| Локальный API на `std::net` (`TcpListener`), без HTTP-крейтов. Защита: запрос с `Origin` или `Sec-Fetch-*` -> 403 (это браузер), `Host` не `127.0.0.1`/`localhost`/`[::1]` -> 403 (DNS rebinding); слушаем только `127.0.0.1` | Крейты (hyper, tiny_http) раздули бы exe, а запросов всего три вида | Нет авторизации: любая локальная программа пользователя может слать надписи. Соединения по одному (медленный клиент держит поток до 2 с) |
| Всё Win32 и медленное вне `st()` и вне UI-потока: счётчики нагрузки читает `app::on_tick` до `st()`, VPN и `WM_SETTINGCHANGE` идут в winrt-воркер | Закрыт долг: Win32 внутри `st()`, rasdial/pbk в UI, короткоживущий поток темы | Статус VPN приходит с задержкой; воркер чуть чаще занят |
| Чужие уведомления: опрос `GetNotificationsAsync` раз в 2 с в winrt-воркере с кэшем прочитанных по id (`sys/toasts.rs`). Баннеры самой Windows программно не убрать: пользователь выключает их в Параметрах, уведомления остаются в центре уведомлений, откуда Pill их читает | Событие `NotificationChanged` недоступно непакетному приложению (0x80070490); файл `wpndatabase.db*` событий каталога не даёт; Bot API Telegram не видит личные сообщения. Кэш по id снизил CPU с ~0,4% до ~0,13% ядра | Нарушает правила «0% CPU в простое» и «опрос только в раскрытом виде»: ждёт согласования. С выключенными баннерами вживую не проверено |
| Агентов ведёт хук Claude Code через локальный API, имя агента = папка проекта (cwd). Хук: `curl.exe -s -m 2 --connect-timeout 1 --data-binary @- http://127.0.0.1:47821/hook`, `async: true` (кроме `PermissionRequest`: синхронный, `timeout` 45, иначе ответ не дойдёт до Claude Code); `/hook` отвечает пустым телом, кроме ответа на просьбу | Stdout хука попадает в контекст Claude, текст в ответе засорил бы его; `async` не тормозит Claude Code. Свой разбор JSON (`json_strings`) терпит обрыв: тело длиннее 16 КБ разбирается обрезанным | Две сессии в одной папке получают одно имя (id разные); работающий без событий 30 мин и остальные 12 ч удаляются |
| Вкладки: 3 в шапке (Музыка, Система, События) и поиск, «Система» — группа из двух страниц (`Tab::Controls` «Управление», `Tab::System` «Монитор») с сегментом `subtabs` под шапкой; страницы сдвигаются через `Painter.oy` (`SUB_H = 30`), `Shell.sys_page` помнит последнюю | Плоский ряд из шести вкладок разрастался; сдвиг через `oy` не требует правки координат страниц | Страницы группы на 30 px ниже; высота панели теперь по содержимому вкладки (13.2), `EXPANDED_H` 400 стал верхним пределом; `Tab` из 5 значений, в шапке 3: `Tab::Commands` — поиск, открывается хоткеем или лупой (`Hit::Mode(0)`), вкладки «Буфер» и «Команды» убраны |
| «Ушки» чёлки (`shell::EAR = 9`): окно шире тела на 2×EAR, вогнутые переходы в верхний край вырезает `Canvas::finish(rad_px, ear)`, содержимое сдвинуто `Painter.ox` | Вид как у чёлки в маках, края гладкие (слоёное окно) | Окно шире видимого тела; весь код раскладки должен помнить про `ox` |
| Анимация раскрытия: `Shell.anim {from, to, start}`, `TIMER_ANIM` 15 мс только ~200 мс, кривая 1-(1-t)^3. Во время анимации `render` рисует раскрытую панель в полной раскладке по центру и обрезает размером окна. Анимируется смена вида (свёрнуто, карточка, раскрыто), смена ширины свёрнутой чёлки мгновенная | Перерисовка не меняет раскладку, нет «прыжков» содержимого; в простое таймера нет | Кадров ~13 на переход, каждый целиком рендерится; клики и DPI во время анимации не проверены |
| Карточка из чёлки: третий вид окна `MODE_PEEK` (ширина 440 px, у просьбы с ответом 600; высота по содержимому: уведомление 38 + текст (40 или 20 на строку, до трёх) + 12, с кнопками под курсором +50; просьба с кнопками 88 + блок кода (18 на строку + 16) + 10 + 28 + 10 + 18 + 12, «закончил» 86), `State.peek`, `TIMER_PEEK`; чужое уведомление, «агент закончил» и просьба о разрешении показываются ею, а не надписью в свёрнутой чёлке. Над карточкой панель не раскрывается, пока курсор на ней, таймер не идёт; просьба с кнопками не прячется сама | Нужно нажимать кнопки без случайного раскрытия панели; надпись `osd::Flash::Text` осталась для API и нагрузки | Размер окна зависит от `peek`; клик по карточке без кнопок её убирает; высота меняется от наведения (`set_hover` -> `layout` на каждый вход и выход курсора); уведомления одного чата в одной карточке (до трёх строк). Кнопки под курсором и настоящий Telegram вживую не проверены |
| Ответ на просьбу о разрешении: хук `PermissionRequest` держит соединение (`bg/api.rs::HELD`), ответ пишет UI-поток (`answer`). Дольше `HOLD` = 30 с не держим, без решения Claude Code спросит в своём окне. Тот же отпуск при «просто точке», при клике по самой карточке, при смене состояния агента | Решить из чёлки, не переключаясь в терминал; stdout хука `/hook` всё равно пустой во всех остальных случаях | Соединение живёт до 30 с, а таймаут хука в `settings.json` пользователя (синхронный, 45 с) больше; запись в сокет идёт под локом `HELD` из UI-потока (мгновенно, но формально блокирует) |
| Карточка просьбы 600 px с кодом и очередью: `Peek.tool/branch/lines/stat/can_always` собираются из данных агента (`agents::ask_detail`), вторая просьба ждёт в `State.asks` и показывается после ответа на первую (`hide` -> `show`); PermissionRequest больше не затирает `note`, чтобы остался запрос человека | Решать по сути (что именно правит агент), а не вслепую; две просьбы подряд не теряются | Ширина окна зависит от вида карточки; просьбы в очереди тоже живут не дольше `HOLD` = 30 с (потом `resolve` убирает их); блок кода не прокручивается |
| Ответ на просьбу клавишами Alt+Y/A/N/T: глобальные хоткеи 3..6, регистрируются в `peek::show` и снимаются в `peek::hide`, только пока просьба с ответом на экране. «Всегда» отвечает `decision.behavior = allow` и `updatedPermissions` = присланные Claude Code `permission_suggestions` (формат сверен с документацией хуков); если правил нет, кнопка действует как «Разрешить». Alt+T = «в терминал»: хук отпускается с пустым телом | Pill не берёт фокус, обычные клавиши до него не дойдут; пустой stdout = обычный диалог в терминале | На время просьбы Alt+Y/A/N/T перехватываются у других программ (меню «Правка/Таблица» и т. п.); занятое чужой программой сочетание тихо не регистрируется (`let _ =`), останется только мышь. Клики мышью по кнопкам вживую не проверены |
| Маскот агента (`ui/mascot.rs`): спрайт 10×9 прямоугольниками, цвет по хешу имени, лицо по состоянию; «кадр» берёт `State.notif_flip`, и пока агент работает или просит разрешение, свёрнутая чёлка перерисовывается каждую секунду (`on_tick`, `pulse`) | Нет ни ресурсов, ни нового таймера | Лишняя перерисовка раз в секунду без сравнения со старым значением: мелкое отступление от правила «только изменившееся» (раздел 1) |
| Настройки в `settings.txt` строками «ключ=значение»; незнакомые строки сохраняются при записи, кривые значения знакомых ключей заменяются умолчанием, `collapse_ms` зажат в 100..5000. Меню трея вместо окна настроек | Ни зависимостей, ни нового окна; чужие и будущие ключи не теряются | Правка вручную подхватывается только после перезапуска. «Просто точка» вживую не проверена |
| Палитра «чёрное стекло»: чёрный, белый и серые, без синего; главное и «включено» белое с чёрным текстом, цвет только у состояний и маскотов (макеты: https://claude.ai/artifact/MdqtziwcURTAj2viKMo39q); `face()` = Segoe UI Variable Text при наличии `SegUIVar.ttf` | Редизайн 13.1 (PLAN.md); проверка файла дешёвая, без реестра | Синий акцент убран совсем; Windows 10 (без Variable) вживую не проверено |
| Имя Wi-Fi: `wifi_name()` перебирает все профили `GetConnectionProfiles` (WLAN и связь не `None`), читается в воркере вместе с радио | При VPN `GetInternetConnectionProfile` возвращает туннель, а не Wi-Fi; фича `Networking_Connectivity` | Возможная причина роста рабочего набора до ~32 МБ (раздел 1), не подтверждено. Кабель или нет сети: имени нет |
| Ветка git агента: `agents::read_branch` читает `<cwd>\.git\HEAD` один раз на агента (`wants_branch`/`set_branch`), в UI-потоке вне `st()` из `api::on_events` | Не запускать `git` и не добавлять поток; файл крошечный | Файловый ввод-вывод в UI-потоке (пока мелкий, но не в воркере); `.git` как файл (worktree, submodule) ветку не даст |
| `GET /open?tab=...`: раскрыть панель на вкладке, без `tab` или с неизвестным значением сворачивает | Сверять вид по снимкам экрана без наведения мышью | Любая локальная программа может открыть панель (как и остальной API, без авторизации) |
| Поиск вместо вкладок «Буфер» и «Команды»: один `Tab::Commands`, режимы `cmd::MODE_*` переключаются Tab/Shift+Tab и кликом, при сворачивании возврат на `Input.back`. Счётчики запусков в `launches.txt` (ключ = имя ярлыка в нижнем регистре) для плиток частых программ | Одна строка вместо двух вкладок, буфер и программы в одном списке; текстовый файл вместо базы (как у истории буфера) | `cmd::scan()` (обход меню Пуск) на каждом открытии поиска, в том числе по Ctrl+Alt+V, в UI-потоке; из `/open` даже внутри `st()`. Магазинные приложения (2.4) не искались. Набор текста, хоткеи, Alt+цифра, вставка, счётчики и прокрутка вживую не проверены |
| Ctrl+Alt+H прячет окно целиком (`Shell.hidden`, `visible()` false, `layout` скрывает окно), а не сворачивает пилюлю в точку | Нужно нажать то, что под пилюлей (вкладку браузера), а полоска под курсором не должна ловить клики | Окна нет, значит и подсказки нет: вернуть можно только хоткеем, поиском или просьбой агента; автовозврата по таймеру нет |
| Ползунок тащится через `SetCapture` (захват на время зажатой кнопки; `WM_LBUTTONUP` и `WM_CAPTURECHANGED` снимают его), значение считается по положению курсора за пределами окна | Курсор может уйти за окно, а ползунок должен слушаться до отпускания | Пока тащим, мышь не у других программ; громкость ставится на каждом движении в UI-потоке (раздел 1) |
| Высота раскрытой панели по содержимому открытой вкладки: `Painter::reach(y)` отмечает нижний край (`Painter.bottom`), после отрисовки высота = `bottom` + 20 в пределах 200..`EXPANDED_H` (400) и пишется в `Shell.panel_h`. Смена высоты шлёт `PostMessageW(WM_APP_LAYOUT)` вне `st()`, окно переложится тем же `Anim`. Поиск всегда полной высоты | Пустая «Музыка» (271 px) не такая высокая, как «Монитор» (375 px), и под содержимым нет пустого хвоста | Вкладка без `reach` не пересчитывает высоту и держит прежнюю. При уменьшении панели под курсором курсор может выйти за окно, и панель свернётся через 350 мс (клик по вкладке в шапке этого не даёт) |
| `panic = "abort"`, `strip`, LTO | Размер exe | Нет бэктрейса, только сообщение в `crash.log` |

---

## Сборка и проверка

```
taskkill /IM pill.exe /F
%USERPROFILE%\.cargo\bin\cargo build --release
%USERPROFILE%\.cargo\bin\cargo test --release
```
Debug-сборка не линкуется (кириллица в пути пользователя), поэтому всё через `--release`.
