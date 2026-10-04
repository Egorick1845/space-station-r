# PORT_PLAN.md — план полного порта механик SS14 в Space Station R

Правило (фаза 8 PLAN.md): **только перенос из сборки** `C:\ss14\mini-station-goob`
(контент) и её движка `RobustToolbox`. В коде переноса указывается файл-источник.
Свои решения не допускаются; если механизм сборки требует возможности, которой
у нас нет — делаем ровно эту возможность (пример: GPU-конвейер света).

Метод каждого пункта: **(1)** изучить субагентами по конкретным файлам →
**(2)** перенести 1:1 с источником в комментарии → **(3)** сверить скриншотом или
логом против сборки → **(4)** тест на числа/инвариант → **(5)** отметка в PROGRESS.md.

---

## M1. Визуальный двойник (сначала то, что видно)

| # | Механика | Источники в сборке | У нас сейчас | Что сделать |
|---|---|---|---|---|
| 1.1 | Свет и FOV (GPU-конвейер) | `Robust.Client/Graphics/Clyde/Clyde.LightRendering.cs`, `Resources/Shaders/Internal/{light_shared,light-soft,light-hard,shadow_cast_shared,light-blur,wall-bleed-blur,wall-merge,fov-lighting,fov_shared,fov}.swsl`, `Shaders/shadow-depth.{vert,frag}` | **готово**: GPU-путь подключён — `extract_light` (ExtractSchedule) → буферы/пайплайны в `RenderSystems::PrepareResources` → цепочка проходов в `Core2d`/`Core2dSystems::Prepass` (тени лучами, FOV, карта света, маска стен-стенсил, блюр, просачивание), оверлей мира — материал-умножение (`COLOR * LIGHT`). CPU-путь (`update_lighting`) — фолбэк под `SSR_LIGHT_CPU=1`, признан владельцем годным | Осталось: карта света пока 960×540 (подогнать под окно, как `light.resolution_scale = 0.5`) и профилирование (T7.5) |
| 1.2 | Спрайты, слои, порядок отрисовки | `Robust.Client/GameObjects/Systems/SpriteSystem.cs` (слои, `SpriteComponent.Layer`, `DrawDepth`), `SpriteOrdering` | частичная (тело, одежда, предметы) | Перенести модель слоёв и `DrawDepth` (порядок мира), выравнивание по Y как в движке |
| 1.3 | Тайлы и IconSmooth | `IconSmoothSystem`/`IconSmoothComponent`, `SetCornerLayers` | своя логика (уже сверена: SE→0, NE→2, NW→1, SW→3) | Перенести полностью из компонента, включая диагональные варианты и «слои углов»; тест на эталонные тайлы |
| 1.4 | Окна интерфейса по XAML | `Content.Client/**/*.xaml` + `Stylesheets/StyleNano.cs` | стекло-тема, слоты, хотбар, инвентарь, спавн-меню (факты в SS14_UI.md); **меню спавна — все сущности сборки** (5130 со спрайтом из `prototypes_ss14.ron`); **игровое меню (`EscapeMenu.xaml`) и окно настроек (`OptionsMenu.xaml`, 5 вкладок)** | Довести каждое окно 1:1: размеры, отступы, шрифты, цвета, позиции (HotbarGui, InventoryGui, StorageWindow, EntitySpawnWindow, VerbMenu, Chat, Admin, Options — 7 вкладок и ~40 контролов) |
| 1.5 | Алерты и статусы | `Content.Client/UserInterface/Systems/Alerts/*`, `Content.Shared/Alert/*`, `Resources/Prototypes/Alerts/*` | **готово**: колонка `AlertsUI` 64×64 справа под чатом (`SetMarginTop(Alerts, chatHeight)`), Health 5 иконок `human_alive` (`severity = round(lerp(0,4,доля урона))`), Stamina 7 иконок (`RoundToLevels(остаток, 100, 7)`), давление `pressure.rsi` только в опасной зоне (`BarotraumaSystem`: 50/385 предупреждение, 20/550 урон); **иконки анимированы** — флипбук по `delays` RSI (`hud::animate_alerts`, у `health0` 28 кадров × 0.05 с) | — |
| 1.6 | Инвентарь: размеры и формы | `Resources/Prototypes/item_size.yml`, `Content.Shared/Item/ItemComponent.cs`, `Content.Client/.../Storage/{StorageWindow.cs,ItemGridPiece.cs}` | сетка 7×4, размеры в прототипах, иконка ×2 | Рамка формы из `piece_*`, фантом предмета при переносе, подсветка клеток (`#1E8000`/`#B40046`), поворот (СКМ), сохранение позиции |
| 1.7 | Хранилища: ящики = EntityStorage | `Content.Shared/Storage/Components/EntityStorageComponent.cs`, `Content.Shared/Storage/EntitySystems/SharedEntityStorageSystem.cs`, `Resources/Prototypes/Entities/Structures/Storage/Crates/*` | **готово**: у ящиков нет сеточного окна; открытие высыпает содержимое (`OpenStorage` → `EmptyContents`), закрытие всасывает только предметы НА ящике (`EnteringRange`), класть — перетаскиванием на открытый ящик (аналог `PlaceableSurface`); звуки `closetopen/closetclose.ogg`; сетка осталась только у `StorageComponent` (рюкзак, пояс) | — |
| 1.8 | Позиционирование HUD | `HotbarGui.xaml`, `InventoryGui.xaml`, `ChatBox.xaml`, `ActionsBar`, `GhostBar` | позиции по SS14_UI.md | Сверить каждую панель с XAML (пиксельно по скриншоту) |

## M2. «Штурвал в руках» (движение и взаимодействие)

| # | Механика | Источники | У нас | Что сделать |
|---|---|---|---|---|
| 2.1 | Движение (Quake-модель) | `Content.Shared/Movement/Systems/SharedMoverController.cs`, `Components/{InputMoverComponent,MovementSpeedModifierComponent}` | своя реализация (friction/accel сверены) | Перенести множители состояний и скорости точно (walk 2.5 / sprint 4.5; лежачий ×0.45, урон ×0.7/0.5, голод 0.75 и т.д.) |
| 2.2 | Бег/ходьба/спринт | `InputMoverComponent.Sprinting`, `keybinds.yml` (Shift = Walk, Space = Sprint), `_Goobstation/Sprinting/*` | бег по умолчанию, Shift — ходьба | Добавить спринт-тоггл (Space, ×1.45, кулдаун 3 с, запреты) и состояния |
| 2.3 | Выносливость и станкрит | `Content.Shared/Damage/Components/StaminaComponent.cs`, `SharedStaminaSystem.cs` | нет | 100 единиц, трата 8/с, реген 5/с через 5 с, крит → 6 с нокдауна + Blunt 10 + звёзды, буфер 3 с |
| 2.4 | Анимация ног (процедурная) | `Content.Client/_Mini/FootWalk/FootWalkAnimationSystem.cs`, `FootWalkAnimationComponent.cs` | нет | Фаза `dt·9·rate`, rate 0.6375/1.2025, амплитуда 2.5/32 только ноги/стопы, боковая дальняя ×0.4, отключение при |v|²<0.04 |
| 2.5 | Лежание и вставание | `StandingStateSystem`, `CrawlerComponent`, `KnockedDownComponent`, `RotationVisuals` | поворот куклы есть | Поворот 90° за 0.125 с, скорость ×0.45, трение ×0.65, вставание 1.2 с, форс-вставание за 10 стамины |
| 2.6 | Тянуть за собой (Pull) | `Content.Shared/Pulling/*` (`PullerComponent`, `PullableComponent`), верб Pull | **готово**: верб «Тянуть»/«Отпустить» + Ctrl+ЛКМ, верёвка = расстояние захвата + 0.15 м (4.8 юнита), следом за игроком, скорость ×0.95, разрыв при стане/пропаже/телепорте; `ssr_core::pull` с тестами, `SSR_PULL_TEST` | Осталось: алерт «Pulling» в колонке (нужна репликация состояния захвата) |
| 2.7 | Вербы (ПКМ-меню) | `Content.Shared/Verbs/*`, `Content.Client/Verbs/{VerbSystem.cs,UI/VerbMenuUIController.cs,UI/VerbMenuElement.cs}`, `ContextMenu/UI/*` | «Осмотреть» и «Взять» | Таблица вербов, порядок по TypePriority, строка 32 px, цвета `#2d3341d9/#3d4556/#404352`, шрифты Noto 12 с вариантами, иконки вербов |
| 2.8 | Взаимодействие | `SharedInteractionSystem` (1.5 м, `CanAccess/CanInteract/CanComplexInteract`), `InteractionSystem` | радиус 1.5 тайла, часть действий | Перенести правила доступа (контейнеры, открытые хранилища) и Alt-активацию |
| 2.9 | Слоты и крепления | `ItemSlots/*`, `Buckle/*`, `Storage` | экипировка по клику/драгу | Перенести `ItemSlots` (вставка/извлечение) и пристёгивание |
| 2.10 | Бой и урон | `Content.Shared/Damage/*` (DamageSpecifier, Damageable), `Weapons/Melee/*`, `GunSystem`, `Stamina` | урон 15/5, смерть | Перенести типы урона, броню, стан, оружие (затвор/магазин) |

## M3. Станция живая (атмосфера, энергия, производство)

| # | Механика | Источники | У нас | Что сделать |
|---|---|---|---|---|
| 3.1 | Атмосфера | `Content.Server/Atmos/*` (GasMixture, GridAtmosphere, трубы) | lite (давление/O₂ по тайлам) | Перенести модель газов (моли, тепло, диффузия, разгерметизация, огонь) |
| 3.2 | Электрика | `Content.Server/Power/*` (PowerNet, АПК, батареи, генераторы) | lite | Перенести сеть питания, приоритеты, разряды, свет от питания |
| 3.3 | Строительство | `Content.Shared/Construction/*` (гайдбуки, шаги) | стройка листом, разбор ломом | Перенести шаги строительства и рецепты из прототипов |
| 3.4 | Крафт | `Content.Shared/Crafting/*` | простой крафт | Перенести рецепты и категории из прототипов |
| 3.5 | Медицина и химия | `Content.Shared/Chemistry/*`, `Content.Server/Chemistry/*`, тело/раны | нет | Реагенты, метаболизм, шприцы, части тела, ранения |
| 3.6 | Логистика | `Storage`, конвейеры, `Cargo` | нет | По мере надобности, после 3.1–3.5 |

## M4. Раунд и роли

| # | Механика | Источники | Что сделать |
|---|---|---|---|
| 4.1 | Таймер и ход раунда | `Content.Server/GameTicking/*` | Перенести состояния раунда (лобби, старт, конец, отсчёт) |
| 4.2 | Роли и работы | `Content.Shared/Roles/*`, `Prototypes/Roles/*`, `Jobs/*` | Перенести роли из прототипов (у нас три роли вручную) |
| 4.3 | Антагонисты и цели | `Content.Server/GameTicking/Rules/*`, `Objectives/*` | Перенести выбор антагов и цели (T4.5 запрещался ранее — уточнить у владельца, нужен ли) |
| 4.4 | Лобби/меню | `Content.Client/Lobby/*` | Перенести экран входа и выбор персонажа (у нас своё меню) |

## M5. Сеть, контент, локализация

| # | Механика | Источники | Что сделать |
|---|---|---|---|
| 5.1 | Предсказание и интерполяция | `Robust.Shared/GameStates/*`, `Prediction/*` | Перенести модель (у нас интерполяция есть, предсказания нет) |
| 5.2 | PVS/интерес | `Robust.Server/GameStates/PVSSystem.cs` | Перенести правила видимости (у нас `Rooms`) |
| 5.3 | Все прототипы | `Resources/Prototypes/**` (21 358 шт.) | Спавн-меню и серверный спавн по всем сущностям; конвертер компонентов (IMP.3) |
| 5.4 | Карты станции | `Resources/Maps/**` | Перенос через экспортёр (IMP.4) |
| 5.5 | Локализация | `Resources/Locale/*.ftl` | Перенести строки (у нас русские подписи захардкожены) |
| 5.6 | Админка | `Content.Server/Administration/*` (команды, вербы, лог) | Перенести список команд (таблица в отчёте) и админ-вербы |
| 5.7 | Ассеты (звук/спрайты) | `Resources/Audio/**`, `Resources/Textures/**` | Догружать по мере переноса механик (RSI-загрузчик готов) |

---

## Порядок и критерии приёмки

1. **M1 закрывается**, когда скриншоты нашей станции совпадают с кадрами сборки по
   свету, теням, слотам, окнам и инвентарю (сверка глазами владельца).
2. **M2** — когда бег/ходьба/стамина/лежание/тянуть и вербы ведут себя как в сборке
   (числа из таблиц выше закреплены тестами).
3. **M3** — когда станция «живёт»: воздух уходит через пробой, свет гаснет без питания,
   вещи строятся по шагам гайдбука.
4. **M4/M5** — раунд, роли, антаги, сеть и полный контент прототипов/карт.

## Ближайшие три работы (в порядке старта)

1. **1.1** — проводка GPU-конвейера света: **сделана** (сверка с CPU-путём A/B на
   одном спавне, `target/ab_light.ps1`); остались размер карты под окно и
   профилирование (T7.5).
2. **1.6** — фантом формы предмета при переносе и подсветка клеток (в сетке
   рюкзака сделано; ждёт окна хранилищ).
3. **Вербы предмета** — остаток по отчёту о `VerbSystem` (взять в руку, надеть,
   снять, убрать в рюкзак) и админ-команды из сборки (пункт 3 очереди запросов).
