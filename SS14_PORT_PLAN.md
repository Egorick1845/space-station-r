# SS14_PORT_PLAN.md — план портирования механик 1:1

Источник: `C:\ss14\mini-station-goob` (Goob/Corvax/Mini-форк, RobustToolbox
v289.0.3). Правило: только перенос из сборки, с указанием файла-источника в
комментарии кода; ничего не выдумывать; каждая механика проверяется скриншотом
или логом. Состояние выполнения — `PROGRESS.md` (там же история заходов).

Обозначения: ✅ сделано, 🔄 в работе, ⏳ запланировано.

---

## 0. Принципы (из уже сделанного)

- Числа берём ТОЛЬКО из прототипов и C#: таблицы (`item_size.yml`,
  `CollisionGroup.cs`), дефолты компонентов (`GunComponent`, `Verb`), формулы
  (`GetRecoilAngle`, `NearbyUnoccluded`).
- Данные прототипов — в `assets/prototypes_ss14.ron` (импортёр
  `crates/tools/src/bin/import-proto.rs`), резолв — сначала данные сборки, потом
  наш ручной каталог.
- Реплицируемые компоненты живут в `ssr_core`, регистрируются в
  `ssr_protocol::net::register_replication` (порядок обязан совпадать на обеих
  сторонах).
- Системы не должны превышать лимит параметров: вспомогательные — в
  `SystemParam` (`AuxParams`, `ActionQueries`).

---

## 1. Аура/видимость: стены и двери не видны сквозь стены ⏳

**Проблема владельца:** сквозь стены видны другие стены и двери.

Факты сборки: сущности рисуются с умножением на карту света, а зона вне FOV
становится чёрной; контекстное меню фильтруется `ExamineSystem.CanExamine`
(`Content.Client/Examine/ExamineSystem.cs`) и тегом `HideContextMenu`.

План:
1. Воспроизвести кадр: игрок у стены, за ней дверь/стена — сверить, что именно
   видно (тёмно-серое vs полная яркость).
2. Если виноват ambient/мягкая видимость — в `light_apply_fov_cs` при
   `occlusion < 1` доводить альфу до 1 (полная тьма вне FOV), как
   `fov-lighting.swsl` + `occludeColor = Color.Black`.
3. Если виноват z-порядок (двери 1.5, оверлей 2.01) — проверить, что оверлей
   рисуется после всех мировых спрайтов при любом рендер-графе.
4. Добавить фильтр сущностей по FOV для клика/контекстного меню (аналог
   `CanExamine`): сущности за стеной не должны попадать в меню вербов
   (`EntityMenuUIController.TryGetEntityMenuEntities`).

## 2. Режим призрака 1:1 🔄

Сборка: `Content.Shared/Ghost/GhostComponent.cs`, `Content.Server/Ghost/GhostSystem.cs`,
прототипы `Resources/Prototypes/Entities/Mobs/Player/observer.yml` (`Incorporeal`)
и `admin_ghost.yml` (`AdminObserver`).

Факты для переноса:
- Проход сквозь стены: у фикстуры `layer: GhostImpassable (=32)`, `mask` не задан
  ⇒ `CollisionMask = 0`; `bodyType: KinematicController`, `bodyStatus: InAir`,
  `MovementIgnoreGravity`, `CanMoveInAir`.
- Скорости: гост `baseSprintSpeed 12 / baseWalkSpeed 8`; aghost `30 / 12`.
- Видимость: `Eye { drawFov: false, visMask: [Ghost, Normal, ...] }`; живые не
  видят призрака вне PostRound (`OnGhostStartup`: слои `Ghost`/`Normal`).
- Взаимодействия запрещены (`SharedGhostSystem`: `UseAttemptEvent`,
  `InteractionAttemptEvent`, `DropAttemptEvent`, `PickupAttemptEvent`,
  `InteractionVerbAttemptEvent`), кроме UI при `CanGhostOpenUI = true`.
- HUD: `GhostGui` (Warp, ReturnToBody, GhostRoles, Thunderdome), окно
  `GhostTargetWindow`/`MiniGhostTargetWindow` (Players/Places/Antagonists).

Шаги:
1. ✅ Верб/команда: `Ghost` + `ColliderDisabled` (проход сквозь стены).
2. ⏳ Разделить состояния: живой / призрак (`Ghost`) / админ-призрак (`AdminObserver`):
   скорости 8/12 и 12/30, `drawFov: false` (снять затемнение FOV для призрака).
3. ⏳ Невидимость для живых: компонент `Spectral` + фильтр видимости на клиенте
   (не рисовать призраков, если у смотрящего нет `Ghost`/админ-прав), рассылка
   `system:` сообщения о входе/выходе из призрака.
4. ⏳ Запреты взаимодействий (подъём, применение, вербы) при `Ghost`.
5. ⏳ HUD призрака: кнопки Warp/ReturnToBody/GhostRoles + окно списка игроков
   (`MiniGhostTargetWindow`), варп — `WarpTo` (телепорт + обнуление скорости).
6. ⏳ `ghost`/`aghost`/`showghosts`/`toggleselfghost` — консольные команды.

## 3. Системные сообщения чата ⏳

Сборка: `Content.Server/Chat/Systems/ChatSystem.cs` (`SendEntitySystemMessage`),
`Content.Shared/CCVar` `chat.*`, локализация `Resources/Locale/*/chat/*.ftl`.

Шаги:
1. ✅ Клиент принимает `Event { kind: "system:<текст>" }` → строка в чате
   (`chat.system`).
2. ⏳ Сервер: рассылка системных сообщений на подключение/отключение
   («X подключился/отключился»), смерть, вход/выход из призрака, админ-действия
   (`spawn`, `kick`, `delete`), результат верба с `ConfirmationPopup`.
3. ⏳ Каналы: `ChatChannel::System` с префиксом и цветом как в
   `Resources/Locale/ru-RU/chat/managers/chat-manager.ftl`; отключение системных
   сообщений настройкой (`chat.system_messages`).

## 4. View Variables ⏳

Сборка: `RobustToolbox/Robust.Shared/ViewVariables/**` (домены `ioc`, `entity`,
`system`, `prototype`, `object`, `vvtest`), `Robust.Client/ViewVariables/**`
(окна/редакторы), `Robust.Server/ViewVariables/**` (сессии, права по команде
`vv`). Команды: `vv`, `vvread`, `vvwrite`, `vvinvoke`, `quickinspect`; хоткеи
`Alt+V/B/C`.

Шаги:
1. ✅ Верб `View Variables` (`VvVerb`, `i32::MAX`, всегда первый).
2. ⏳ Протокол: сессии (`MsgViewVariablesOpenSession/ReqData/RemoteData/
   ModifyRemote/CloseSession/DenySession`), блобы `Metadata`, `Members`,
   `EntityComponents`, `AllValidComponents`, `Enumerable`.
3. ⏳ Сервер: доступ к полям по атрибуту (`ReadOnly`/`ReadWrite`), домены путей,
   разбор `entity/<uid>/Компонент/Поле`, права (`CanViewVar` = права команды `vv`).
4. ⏳ Клиент: окно 640×420, вкладки client/server variables + components, кнопка
   «Add component» с поиском, редакторы bool/number/string/enum/Color/Angle/
   Vector2/TimeSpan/EntityUid/ProtoId, Refresh (ПКМ — авто 500 мс).
5. ⏳ Команды `vvread`/`vvwrite`/`vvinvoke` + `Alt+V/B/C` (`InspectEntity`).

## 5. Админ-панель и дебаг ⏳

Сборка: `Content.Client/Administration/UI/AdminMenuWindow.xaml` (вкладки Admin,
Adminbus, Atmos, Round, Server, PanicBunker, Players, Objects), `AdminFlags`
(u32, 25 флагов, `AdminFlags.cs`), `playerpanel` + EUI, спавнеры F5/F6/F8,
дебаг-мониторы F3 (`DebugMonitors`), `~` — консоль, F10 — escape-меню.

Шаги:
1. ⏳ `AdminFlags` в ядре (значения из `AdminFlags.cs`), гейт по правам.
2. ⏳ Окно админ-меню (F7) с вкладками; Players (ЛКМ → VV, ПКМ → вербы с
   `force: true`), Objects (Grids/Maps/Stations, ЛКМ → VV), Admin (кнопки команд).
3. ⏳ `playerpanel`: notes/ahelp/freeze/kick/ban/rejuvenate/follow/camera/logs.
4. ⏳ Дебаг-оверлеи и команды (`showmarkers`, `showsubfloor`, `toggleoutline`,
   `showhealthbars`, `showgunspread`, `nodevis`), мониторы F3.

## 6. Вербы: добить 1:1 ⏳

1. ✅ Модель (`ssr_core::verbs`), порядок `CompareTo`, категории, серверный сбор,
   меню 32 px с типами и категориями.
2. ⏳ PNG-иконки вербов (`Interface/VerbIcons/*.png`) — загрузчик PNG в клиенте
   (сейчас `RsiRegistry` умеет только RSI).
3. ⏳ Подменю категорий по наведению (задержка 0.2 с, позиция
   `parent + (width + 4, −4)`), ограничение «10 элементов до скролла».
4. ⏳ `ConfirmationPopup` (подменю «Подтвердить», `generic-confirm`).
5. ⏳ `ClientExclusive` вербы (экзамайн, VV) — исполнение на клиенте.
6. ⏳ Консольные `listverbs`, `invokeverb`, `runverbas` (форс-исполнение + лог).
7. ⏳ Вербы по объектам: `Vault` (Climbable), `Rotate` (Category Rotate,
   icons-only, приоритеты −2/−1/0, `CloseMenu = false`), `Dump`, режим огня
   (`VerbCategory.SelectType`, `Disabled` на текущем).

## 7. Столы и конструкции ⏳

1. ✅ Спрайты (4 угловых слоя `IconSmooth`), коллизия `Fixtures`, поверхность,
   z-порядок по `DrawDepth`.
2. ⏳ `Climbable`: верб «Vault» (AlternativeVerb), DoAfter 1.5 с с
   `BreakOnMove`/`BreakOnDamage`, транзит 5 ед./с, подмена фикстур (снять
   `TableLayer|LowImpassable`, сенсор r=0.35 `hard: false`).
3. ⏳ `Construction`: графы (`Resources/Prototypes/Recipes/Construction/Graphs/
   furniture/tables.yml`), инструменты (Wrench/Crowbar/Welder/Knife), времена
   1–3 с, `SnapToGrid`, стекло/картон → `TableFrame`.
4. ⏳ `Destructible` пороги (таблица из отчёта), `GlassTable` (масса > 60 ломает,
   стан 2 с), `FootstepModifier`, `Bonkable`.

## 8. Оружие и патроны ⏳

1. ✅ P0: данные из прототипов, формула разброса, патронник+магазин, снаряды,
   звуки, счётчик, тесты.
2. ⏳ P1: `FullAuto`/`Burst` (серверный AutoFire-тик, `BurstCooldown`),
   `ChamberMagazine` целиком (затвор, rack, lock слота), `AutoEject`,
   `RevolverAmmoProvider` (барабан, speedloader, Empty/Spin),
   `BatteryAmmoProvider` (shots = charge/FireCost), `BasicEntityAmmoProvider`,
   `RefreshModifiers`/`GunRefreshModifiersEvent`, `GunWieldBonus`, `Multishot`.
3. ⏳ P2: хитскан (`HitscanAmmo + HitscanBasicRaycast{20, Opaque} + Damage +
   Visuals + Effects`), прототипы `RedLaser 14`, `RedMediumLaser 17`,
   `XrayLaser`, `Pulse`; мазл-флеш/трейл/импакт (анимация 0.48 с), свет
   `radius 2, #cc8e2b, energy 5`.
4. ⏳ P3: проникание (`PenetrationThreshold = 10`, `Penetratable {15, 0.05}`),
   `RequireProjectileTarget` + crawl hitzone 0.85 м, `FlyBySound`, апгрейды,
   HUD по типу провайдера (`x{count:00}`, «No Magazine!», `#d7df60`).

## 9. Прототипы, карты, тайлы ⏳

1. ✅ Менеджер прототипов: YAML-мерж по ключам, мультинаследование, 19 421
   прототип, `HideSpawnMenu`, размеры/форма/спрайты/слоты/оружие.
2. ⏳ Спавн сущностей по прототипу с реальными компонентами
   (`EntityPrototype.LoadEntity`: Sprite/Storage/Clothing/Tool/Stack/Physics/
   PointLight) вместо «предмета с именем».
3. ⏳ Карты SS14 v6/v7: base64-чанки (7/6 байт на тайл), палитра `yamlId → proto`,
   сущности по группам с `Transform` (grid-local, центр тайла `x+0.5`).
4. ⏳ Тайлдефы `type: tile` (408, `Space` = 0 + порядок), спрайты по `variants`.
5. ⏳ Стаки (`StackComponent`: count/max, слияние при вставке и по клику, спрайт
   по числу, подпись «Count: N»).

---

## Порядок работ (по зависимостям и жалобам владельца)

1. **Аура/видимость** (п.1) — визуальный дефект, виден сразу.
2. **Призрак** (п.2) + **системный чат** (п.3) — быстрые, закрывают жалобы.
3. **Вербы добьём** (п.6: PNG-иконки, подменю, подтверждения, консольные команды).
4. **VV** (п.4) → **админ-панель** (п.5) — «с дебагом и админ панелью».
5. **Столы/конструкции** (п.7) → **оружие P1/P2** (п.8).
6. **Прототипы/карты/тайлы** (п.9) — самый крупный блок, идёт параллельно.

Каждая механика: перенос → тесты на формулы/данные → живая проверка
(`SSR_*_TEST` + скриншот/лог) → коммит по-русски → отметка здесь и в PROGRESS.md.
