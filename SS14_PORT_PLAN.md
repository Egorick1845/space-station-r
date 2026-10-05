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

## 1. Аура/видимость: стены и двери не видны сквозь стены 🔄 (hard FOV готов)

**Проблема владельца:** сквозь стены видны другие стены и двери.

Причина (найдена в нашем коде, заход 2026-10-06):
1. `assets/shaders/light.wgsl`, `light_apply_fov_cs` (строка `occlusion = 1.0;`
   в ветке стен): стеновым текселям маска видимости глаза НЕ применяется —
   стена/дверь за другой стеной не затемняется вовсе, её яркость определяет
   только wall-bleed от соседей.
2. Там же: `alpha = max(1 − luminance, 1 − occlusion)` — ambient (0.082) входит
   в карту света, поэтому «тьма» вне FOV полупрозрачна и за ней проступает
   геометрия.
3. Мягкая видимость VSM (Chebyshev с дисперсией) даёт частичную видимость за
   краем окклюдера; на CPU стеновые тексели берут максимум видимости 4 соседей
   (`lighting.rs`, «просачивание видимости») — стена за стеной подсвечивается.

Факты сборки (как делает SS14, `Robust.Client/Graphics/Clyde/Clyde.LightRendering.cs`,
шейдеры `fov-lighting.swsl`, `fov.swsl`):
- **Два прохода FOV.** (а) `ApplyLightingFovToBuffer` — по световому буферу:
  пиксели за стеной заливаются `occludeColor = Color.Black` с альфой
  `1 − occlusion`, полностью видимые — discard; стенсил помечает затенённые
  пиксели, и лампы рисуются ТОЛЬКО в видимой области (за стеной света нет
  вообще). (б) **Hard FOV** `ApplyFovToBuffer` — по финальному фреймбуферу
  (`Clyde.HLR.cs:740-745`): шейдер `fov.swsl` заливает всё, что за первой
  стеной, непрозрачным чёрным (`alpha = 1.0`), карта глубины рендерится второй
  раз с front-face culling — «смотрим внутрь стен» (bias −0.75/32).
- `LightManager.DrawHardFov = true` по умолчанию ⇒ стена за стеной в SS14
  полностью чёрная. BlurOntoWalls/MergeWallLayer дают «дыхание света» только на
  стенах в поле зрения (их свет-буфер перезаписывается размытым светом ×1.1).
- Сервер может перекрасить «туман войны» через реплицируемый CVar
  `render.fov_color` (по умолчанию чёрный).
- Контекстное меню фильтруется `ExamineSystem.CanExamine`
  (`Content.Client/Examine/ExamineSystem.cs`) и тегом `HideContextMenu`.

План фикса (1:1):
1. ✅ Дальняя граница тела стены: в `fov_map_cs` карта `fov_far` (2048 бинов,
   `ssr_core::light::first_body_exit` — slab-тесты по тайлам стен, кластер
   сомкнутых тайлов расширяется) — аналог front-culled карты движка.
2. ✅ `light_apply_fov_cs`: hard FOV — точка за выходом из первого тела стены
   (`dist > far − 0.75`) получает rgb = 0, alpha = 1 — непрозрачный чёрный
   (оверлей-умножение даёт чистый чёрный); alpha только от итогового цвета.
   Проверено в игре: за стеной и слева от стены — чистый чёрный, пол виден
   только через дверной проём; свет на видимых стенах (wall-bleed) остался.
3. ✅ CPU-путь `lighting.rs`: те же правила (карта `far_map` по тайлам стен +
   закрытым дверям; скрытые hard FOV тексели = 0, просачивание видимости на
   них не действует; у призрака FOV по-прежнему выключен).
4. ⏳ Серверный CVar `render.fov_color` (реплицируемый) — цвет тумана войны.
5. ⏳ Фильтр сущностей по FOV для клика/контекстного меню (аналог
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

## 10. Крафт-меню 1:1 (ConstructionMenu) 🔄

Сборка: `Content.Client/Construction/UI/ConstructionMenu.xaml(.cs)`,
`ConstructionMenuPresenter.cs` (667 строк), контроллер
`Content.Client/UserInterface/Systems/Crafting/CraftingUIController.cs`,
стили `Content.Client/Stylesheets/StyleNano.cs` (строки 238-269, 686-721),
`Content.Client/Stylesheets/Sheetlets/ListContainerSheetlet.cs`,
локаль `Resources/Locale/en-US/construction/ui/construction-menu.ftl`.

В этой сборке (Goobstation-форк) окно называется **ConstructionMenu** — нового
WizDen-CraftingMenu здесь нет.

Спецификация окна:
- `DefaultWindow` 560×450, MinSize 560×320, заголовок «Construction».
- Левая колонка (MinWidth 243, margin 0 0 5 0): `LineEdit SearchBar`
  (placeholder «Search») + `OptionButton OptionCategories` (MinSize 130×0) +
  `ListContainer Recipes` (Group, Toggle) — список рецептов. Альтернативный
  вид (скрыт по умолчанию): Grid 5 колонок.
- RecipeRow: `EntityPrototypeView` 32×32 (Stretch Fill, Scale 2, margin 0 2) +
  `Label` с именем (margin 5 0), tooltip — описание; стиль
  `list-container-button`: фон #373744, hover/pressed #4B4B56, disabled
  #0A0A0C; фон списка Color(55,55,68), выбранная строка Color(75,75,86).
- Рецепты НЕ фильтруются по крафтабельности; сортировка по алфавиту.
- Категории: OptionButton = «All» + «Favorites» (если есть) + уникальные
  категории, отсортированные по локали; смена категории очищает поиск; поиск
  фильтрует по имени без учёта регистра. В Goob: CVar `AutoFocusSearchOnBuildMenu`.
- Правая колонка: ряд `MenuGridViewButton` («Grid View») + `FavoriteButton`;
  шапка — иконка рецепта (Stretch Fill, margin 0 0 10 0) + `TargetName` +
  `TargetDesc` (RichTextLabel); `RecipeStepList` (ItemList) — шаги с иконками,
  отступы `PadLeft`; внизу `BuildButton` (toggle, VerticalExpand, ratio 0.5) и
  ряд `EraseButton` (0.7) + `ClearButton` (0.3) — «Eraser Mode»/«Clear All»
  относятся к строительным призракам.
- Цвета StyleNano: NanoGold #A88B5E, PanelDark #1E1E22, GoodGreenFore #31843E,
  DisabledFore #5A5A5A, ButtonColorDefault #464966, Hovered #575b7f, Pressed
  #3e6c45, Disabled #30313c.

У нас: рецепты из `assets/prototypes/recipes.ron` (input→output, система
`ssr_core::recipes`), клиентское окно `crates/client/src/crafting.rs` — сейчас
плоский текстовый список без иконок/категорий/поиска.

Шаги:
1. ✅ Окно DefaultWindow 560×450 (`hud::window_header`/`window_body`), по центру
   экрана, перетаскивание (`WindowKind::Craft`, позиция запоминается).
2. ✅ Поле категории `category: Option<String>` в `Recipe` (+ recipes.ron:
   «Материалы»/«Инструменты»/«Разное»).
3. ✅ Левая колонка: поиск (клик включает ввод) + выпадающий список категорий
   («Все» + уникальные, смена категории очищает поиск) + список рецептов с
   иконками RSI 32×32 и именем по алфавиту; фон строк #373744, hover/выбор
   #4B4B56; колёсо и перетаскиваемый граббер скроллбара.
4. ✅ Правая колонка: иконка+имя (NanoGold)+«Результат: …», шаги (входы →
   «Получится: …») с иконками предметов и отступом результата, кнопка
   «Создать» (Enabled при крафтабельности, шлёт `ClientMessage::Craft`) +
   «Режим ластика»/«Очистить всё» disabled-заглушки + «Сеткой» заглушка.
5. ⏳ Grid view 5 колонок как рабочий toggle (низкий приоритет).
6. ⏳ Tooltip строки = описание рецепта (в рецептах пока нет поля description).

## 11. Верхняя панель 1:1 (GameTopMenuBar) 🔄

Сборка: `Content.Client/UserInterface/Systems/MenuBar/Widgets/GameTopMenuBar.xaml(.cs)`,
`GameTopMenuBarUIController.cs` (10 контроллеров LoadButtons/UnloadButtons),
`Content.Client/UserInterface/Controls/MenuButton.cs`,
`Content.Client/Stylesheets/Sheetlets/MenuButtonSheetlet.cs`,
`Content.Client/UserInterface/Screens/DefaultGameScreen.xaml`.

Спецификация:
- Anchor TopLeft, margin 10; `HorizontalContainer` SeparationOverride 5.
- Кнопки по порядку (все ToggleMode): EscapeButton — hamburger.svg.192dpi.png,
  MinSize 70×64, ButtonOpenRight; GuidebookButton — information.svg.192dpi.png;
  CharacterButton — character.svg.192dpi.png; EmotesButton — emotes.svg;
  CraftingButton — hammer.svg; ActionButton — fist.svg; LanguageButton —
  `_EinsteinEngines/Interface/language.png`; AdminButton — gavel.svg;
  SandboxButton — sandbox.svg; AHelpButton — info.svg, ButtonOpenLeft; все
  остальные MinSize 42×64.
- Кнопка = ContainerButton с вертикальным BoxContainer: иконка (TextureScale
  0.5, VertPad 4, центр) + Label с горячей клавишей (`BoundKeyHelper.
  ShortKeyName`, шрифт notoSansDisplayBold14, стиль topButtonLabel).
- Цвета иконки/текста (MenuButton.cs:18-20): Normal #99a7b3, Hover #acbac6,
  Pressed #75838e. Патч текстуры кнопки margin 10 (атлас-полосы
  ButtonOpenRight/ButtonOpenLeft/ButtonSquare).
- Каждая кнопка напрямую открывает своё окно (выпадающих меню нет): Escape →
  EscapeMenu, Character → окно персонажа, Crafting → ConstructionMenu,
  Admin → админ-меню, Sandbox → окно сандбокса; Action → меню действий,
  Emotes → меню эмоций, Language → меню языка, Guidebook → гайдбук,
  AHelp → окно обращений к админам.
- В DefaultGameScreen чат — TopRight margin 10 (MinSize 465×225), алерты —
  TopRight под чатом.

У нас: `crates/client/src/hud.rs` `spawn_hud` — 4 кнопки (Esc/G/F5/F7) без
порядка и размеров сборки; ActionsBar (боевой режим/осмотр/выбросить) уже как
в сборке.

Шаги:
1. ✅ 10 кнопок в порядке сборки с иконками из `Interface/*.svg.192dpi.png`
   (language.png импортирован из `_EinsteinEngines/Interface` сборки), размеры
   70×64/42×64, gap 5, anchor TopLeft margin 10, подписи клавиш.
2. ✅ Клавиши: сверены с `Resources/keybinds.yml` сборки (Character U→у нас I,
   Emotes Y, Language L, Guidebook Numpad0→«Num0», AHelp F1, Sandbox B→у нас
   F5-спавн); у нас Escape → Escape-меню, Character → окно персонажа (I),
   Crafting → G, Admin → F7, Sandbox → F5 (спавн-меню).
3. ✅ Guidebook/Emotes/Action/Language/AHelp — кнопки тогглятся и открывают
   окно-заглушку «В разработке» до появления настоящих окон (Esc их закрывает).
4. ✅ Toggle-подсветка кнопки, пока её окно открыто (`topbar_toggle_tint`);
   hover/press — через `hud_button_tint`.
5. ⏳ Bold-шрифт подписей (NotoSans-Bold из сборки, если есть в Resources/Fonts).

---

## Порядок работ (по зависимостям и жалобам владельца)

1. **Аура/видимость** (п.1) — визуальный дефект, виден сразу.
2. **Крафт-меню** (п.10) и **верхняя панель** (п.11) — выглядят не как в SS14
   (жалоба владельца 2026-10-06).
3. **Призрак** (п.2) + **системный чат** (п.3) — быстрые, закрывают жалобы.
4. **Вербы добьём** (п.6: PNG-иконки, подменю, подтверждения, консольные команды).
5. **VV** (п.4) → **админ-панель** (п.5) — «с дебагом и админ панелью».
6. **Столы/конструкции** (п.7) → **оружие P1/P2** (п.8).
7. **Прототипы/карты/тайлы** (п.9) — самый крупный блок, идёт параллельно.

Каждая механика: перенос → тесты на формулы/данные → живая проверка
(`SSR_*_TEST` + скриншот/лог) → коммит по-русски → отметка здесь и в PROGRESS.md.

## 10. Новые пункты владельца (2026-10-05, второй список)

Источник жалоб: «Полёт через гост сейчас работает не так как в сс14, летать должен
именно призрак, у игрока должен быть афк индикатор (ssd), нет анимации бега, нет
анимации ударов, все окна со скроллом мигают при прокрутке. Не работает поиск в
спавн меню, спавн меню в целом работает очень плохо. Сделай начальную карту Dev
как в самой сс14, сделай роли как в сс14, там капитан и т.д, сделай систему
визоров должности, здоровья и т.д.»

| № | Пункт | Факты сборки | Статус |
|---|---|---|---|
| 10.1 | Призрак — отдельная сущность `MobObserver`, а не игрок: `GhostSystem.SpawnGhost` спавнит `GameTicker.ObserverPrototypeName = "MobObserver"`, майнд переезжает (`_minds.Visit/TransferTo`), у обсервера свои компоненты (`Physics: KinematicController`, `bodyStatus: InAir`, `MovementIgnoreGravity`, `CanMoveInAir`, фикстура с `layer: GhostImpassable`, `mask` не задан; `Eye { drawFov: false }`, `Examiner`, `CollectiveMind`, `Speech: Dead`) | `Content.Server/Ghost/GhostSystem.cs:488-540`, `Entities/Mobs/Player/observer.yml` | ⏳ |
| 10.2 | Индикатор AFK/SSD у игрока: в сборке `SSDIndicator`-оверлей — если игрок не двигался дольше `SSDThreshold`, над спрайтом рисуется значок; сервер помечает состояние | `Content.Client/Overlays/SSDIndicatorSystem.cs`, `Content.Shared/CCVar` | ⏳ |
| 10.3 | Анимация бега: `FootWalkAnimation` (у нас есть) + спринт-модификатор; в сборке ещё `SpriteMovementAnimation`/`Wielded`-поза; проверить, что фаза шага ускоряется при Shift и виден бег | `Content.Client/Footsteps`? `humanoid` (`foot_walk_animation`) | 🔄 есть скелет, нужна проверка/ускорение |
| 10.4 | Анимация ударов: `MeleeWeaponComponent.Animation = "WeaponArcThrust"/"WeaponArcSlash"`, `MissAnimation`, `WideAnimation`, `DisarmAnimation`; на клиенте `MeleeWeaponSystem` рисует дугу поверх персонажа (`Effects/arcs.rsi`) на время удара | `Content.Shared/Weapons/Melee/MeleeWeaponComponent.cs:112-126`, `Content.Client/Weapons/Melee/MeleeWeaponSystem.cs` | ⏳ |
| 10.5 | Мигание окон со скроллом: список пересобирался на каждую прокрученную долю пикселя. Исправлено: подпись окна — НОМЕР первой строки, дробный сдвиг делает отдельная система `spawn_scroll_offset` через `UiTransform` | — | ✅ (клиент) |
| 10.6 | Поиск в спавн-меню: фильтр работает, но счётчик совпадений и предел прокрутки считали только 42 предмета каталога, а не 14 тысяч строк списка — прокрутка «упиралась». Исправлено: `spawn_matched_count` считает то же, что отрисовка | — | ✅ (клиент) |
| 10.7 | Спавн-меню в целом: список, категории, вкладки, иконки состояния, избранное, режим размещения по клику — как `EntitySpawningUIController` (`F5`), тайлы `F6`, декали `F8`, sandbox `B` | `Content.Client/UserInterface/Systems/Sandbox/SandboxUIController.cs` | 🔄 |
| 10.8 | Начальная карта Dev как в сборке: `Resources/Maps/Dev/*.yml` (v7) — нужен парсер карт SS14 (п.9.3) и тайлдефы (п.9.4) | `Resources/Maps/Dev/`, `MapLoader.cs` | ⏳ (зависит от 9.3/9.4) |
| 10.9 | Роли как в сборке: прототипы `type: job` (капитан, инженер, СБ…): `id`, `name`, `icon`, `access`, `startingGear`, `weight`, `departments`; импортёр сейчас `job` не читает | `Resources/Prototypes/Roles/Jobs/**` | ⏳ |
| 10.10 | Визоры/индикаторы должности и здоровья: иконка должности на ID и в HUD, полоски здоровья над персонажем (`HealthBars`), статус-иконки (кровотечение, стан, крит), как `JobIcon`, `HealthIcon`, `StatusIcon` в сборке | `Content.Client/Overlays/StatusEffects`, `Content.Shared/StatusEffect` | ⏳ |
