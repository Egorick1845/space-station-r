# Интерфейс SS14: изученные факты (источник — сборка mini-station-goob)

Документ собран по исходникам `C:\ss14\mini-station-goob` (C# + XAML, RobustToolbox).
Это «источник правды» для порта интерфейса: прежде чем менять UI, сверяемся здесь.

## Слоты (общий класс `SlotControl.cs`)

- Сторона слота — `DefaultButtonSize = 64` px; сетка `GridContainer` с зазором **4 px**.
- Кнопка слота — `TextureRect` с масштабом **×2**; фон берётся из темы
  (`SS14DefaultTheme`, `path: /Textures/Interface/Default/`).
- Подсветка — `HighlightRect` с текстурой `Slots/slot_highlight` (32×32) поверх слота;
  блокировка — `Slots/blocked.png`.
- Предмет в слоте — `SpriteView { Scale = (2,2), SetSize = 64×64, OverrideDirection = South }`,
  то есть **мировой спрайт предмета**, а не `inhand`-состояние.
- Иконка хранилища в углу слота — `Slots/back` (масштаб 0.75).

## Панель рук (`Hands` + `HotbarGui.xaml`)

- `HandButton` использует текстуры `Slots/hand_l.png` / `Slots/hand_r.png` (с буквой L/R
  в углу; в русской локализации читается как Л/П) и `Slots/hand_m.png` (без буквы).
- Порядок в `HotbarGui.xaml`: `[SecondHotbar][ItemStatusPanel Right][HandsContainer][ItemStatusPanel Left][MainHotbar]`.
- «В руке пусто» — **не текст в слоте**, а строка `item-status-not-held` в отдельной
  панели `ItemStatusPanel` (ru-RU: «В руке пусто», en-US: «No held item»).
- `ItemStatusPanel`: 125×60, текстуры `item_status_left/right.png` (+`_highlight`),
  `TextureScale = 2`, patch margin top 6 / bottom 4, текст: имя предмета (шрифт 10) или
  пустая надпись (`ItemStatusNotHeld`: курсив 10, `Color.Gray`).
- Хотбар (колонка слотов с цифрами) живёт в `HotbarGui`, но **цифры рисует не он**:
  номер над слотом = подпись привязанной клавиши у кнопки действия
  (`ActionButton.cs: Label.Text = BoundKeyHelper.ShortKeyName(keybind)`), то есть это
  панель действий (`ActionsBar`), а не слоты инвентаря.

## Экран игры (`DefaultGameScreen.xaml`)

- Верхняя панель (`GameTopMenuBar`) — **слева вверху**, отступ 10; кнопки `MenuButton`
  42×64 (или 70×64 у первой): иконка сверху и **подпись горячей клавиши** снизу
  (`notoSansDisplayBold14`). Цвета иконок: normal `#99a7b3`, hover `#acbac6`, press `#75838e`.
- Под ней — колонка действий (`ActionsBar`): иконки действий 64×64, подпись клавиши
  в левом верхнем углу (margin 5).
- Панель призрака (`GhostGui`) — **снизу по центру**, `BottomWide`, отступ 80;
  кнопки: «Вернуться в тело», «Телепорт призрака», «Роли призраков (N)», «Антагонисты»,
  «Арена (N)». Видна только призраку.
- Чат (`ChatBox`) — **справа вверху**, отступ 10, `MinSize 465×225`, фон `#25252ADD`.

## Окно хранилища (`StorageWindow.cs`, строится кодом)

- Ячейка — `Storage/tile_empty.png` (16×16) с масштабом **×2 = 32×32**, сетка без зазоров
  (`HSeparationOverride = VSeparationOverride = 0`).
- Заголовка нет (cvar `control.storage_window_title` по умолчанию `false`).
- Сайдбар слева (`Storage/sidebar_top|mid|bottom|fat.png`), в нём красный крестик
  `Storage/exit.png` (16×16, ×2 = 32×32) — закрывает окно. Кнопка «назад» —
  `Storage/back.png` (для вложенных хранилищ).
- Таблица фона затемняется модуляцией `#222222` (`StorageWindow.cs:554`).
- Кусочки предметов — `Storage/piece_*.png` (8×8, ×2), спрайт предмета — масштаб ×2.

## Спавн-меню (`RobustToolbox/.../EntitySpawnWindow.xaml`)

- Окно **350×400** (минимум 350×200), якорь `CenterLeft`, при открытии фокус в поиске.
- Состав: строка поиска (`LineEdit` + кнопка «Очистить»), прокручиваемый список,
  снизу — «Заменить»/«Режим удаления»/меню режима размещения и подпись направления.
- Строка списка (`EntitySpawnButton`): **иконка прототипа 32×32** + имя (шрифт базовый),
  зазор между строками 2 px, одиночный клик — переход в **режим размещения**
  (сущность «висит» на курсоре, клик ставит); поля количества нет.
- Тексты (ru-RU): «Панель спавна сущностей», «поиск», «Очистить», «Заменить»,
  «Режим удаления».
- Хоткей открытия — **F5**.

## Стили (`StyleNano.cs`, `StyleBase.cs`, `themes.yml`)

- Кнопки: текстура `Nano/button.svg.96dpi.png`, patch margin 10, content margin V2/H14;
  modulate: normal `#464966`, hover `#575b7f`, press `#3e6c45`, disabled `#30313c`.
- Красные кнопки: `#D43B3B` / hover `#DF6B6B`.
- Окна: фон `Nano/window_background_bordered.png` (patch 2), шапка `Nano/window_header.png`
  (patch bottom 3), заголовок — `NanoGold #A88B5E`, шрифт display bold 14; крестик —
  `Nano/cross.svg.png` (22×22, modulate `#4B596A`, hover `#7F3636`).
- Поле ввода: `Nano/lineedit.png`, patch 3, content margin по горизонтали 5.
- Семантика: `#31843E` (хорошо), `#A5762F` (внимание), `#BB3232` (опасно),
  `#5A5A5A` (недоступно), `PanelDark #1E1E22`, текст `#FFF5EE`.
- Базовый шрифт игровых панелей — **13**; панель статуса — 10; заголовки — 14.

## Что у нас (client/ui_theme.rs, hud.rs, inventory_ui.rs, containers.rs)

Реализовано по этим фактам: кнопки-иконки верхней панели с подписью клавиши, колонка
действий, панель призрака снизу, слоты рук 64×64 с `slot_highlight`, панели статуса
125×60, окно хранилища с тайлами 32×32 и красным крестом, спавн-меню 350 px со списком
строк «иконка 32×32 + имя» и режимом размещения по клику.

Ещё не сделано (по этому же документу): панель чата справа вверху, окно ролей призрака,
прокрутка списка спавн-меню (сейчас список ограничен 11 строками + поиск), кусочки
предметов `Storage/piece_*.png` для многоклеточных предметов, вложенные хранилища.
