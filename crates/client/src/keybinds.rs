//! Переназначаемые клавиши — по образцу SS14 (`Resources/keybinds.yml`,
//! `ContentKeyFunctions`, вкладка «Управление» `KeyRebindTab`): у каждой
//! функции две привязки (основная и запасная), клик по привязке — «нажмите
//! клавишу», ПКМ — снять, «Сбросить всё» — значения по умолчанию.
//!
//! Модификаторы сравниваются точно: Q (`Drop`) не срабатывает при зажатом
//! Ctrl, где Ctrl+Q — бросок (`ThrowItemInHand`). Удержание (движение, ходьба)
//! проверяется без модификаторов. Сохраняется в `~/.ssr-keybinds.ron`.

use std::collections::HashMap;

use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Функция управления (имена — как `ContentKeyFunctions` / `EngineKeyFunctions`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyFunction {
    MoveUp,
    MoveDown,
    MoveLeft,
    MoveRight,
    Walk,
    Sprint,
    SwapHands,
    Drop,
    ThrowItemInHand,
    ActivateItemInHand,
    ActivateItemInWorld,
    SmartEquipBelt,
    SmartEquipBackpack,
    ReleasePulledObject,
    Reload,
    EditorCopyObject,
    ToggleCombatMode,
    Hotbar1,
    Hotbar2,
    Hotbar3,
    FocusChat,
    OpenCraftingMenu,
    OpenCharacterMenu,
    OpenBackpack,
    OpenEmotesMenu,
    OpenGuidebook,
    OpenLanguageMenu,
    OpenAHelp,
    OpenSandboxWindow,
    OpenAdminMenu,
    ShowDebugConsole,
}

/// Привязка: клавиша и требуемые модификаторы.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub key: KeyCode,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

impl Binding {
    pub const fn key(key: KeyCode) -> Self {
        Self {
            key,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    const fn ctrl(key: KeyCode) -> Self {
        Self {
            key,
            ctrl: true,
            shift: false,
            alt: false,
        }
    }

    const fn shift(key: KeyCode) -> Self {
        Self {
            key,
            ctrl: false,
            shift: true,
            alt: false,
        }
    }

    /// Подпись для интерфейса: «Ctrl+Q», «Shift+E», «Пробел».
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.shift {
            parts.push("Shift".to_string());
        }
        if self.alt {
            parts.push("Alt".to_string());
        }
        parts.push(key_label(self.key).to_string());
        parts.join("+")
    }
}

/// Раздел списка и подпись функции (русские названия как в `keybinds.ftl`).
impl KeyFunction {
    pub const ALL: [KeyFunction; 31] = [
        KeyFunction::MoveUp,
        KeyFunction::MoveDown,
        KeyFunction::MoveLeft,
        KeyFunction::MoveRight,
        KeyFunction::Walk,
        KeyFunction::Sprint,
        KeyFunction::SwapHands,
        KeyFunction::Drop,
        KeyFunction::ThrowItemInHand,
        KeyFunction::ActivateItemInHand,
        KeyFunction::ActivateItemInWorld,
        KeyFunction::SmartEquipBelt,
        KeyFunction::SmartEquipBackpack,
        KeyFunction::ReleasePulledObject,
        KeyFunction::Reload,
        KeyFunction::EditorCopyObject,
        KeyFunction::ToggleCombatMode,
        KeyFunction::Hotbar1,
        KeyFunction::Hotbar2,
        KeyFunction::Hotbar3,
        KeyFunction::FocusChat,
        KeyFunction::OpenCraftingMenu,
        KeyFunction::OpenCharacterMenu,
        KeyFunction::OpenBackpack,
        KeyFunction::OpenEmotesMenu,
        KeyFunction::OpenGuidebook,
        KeyFunction::OpenLanguageMenu,
        KeyFunction::OpenAHelp,
        KeyFunction::OpenSandboxWindow,
        KeyFunction::OpenAdminMenu,
        KeyFunction::ShowDebugConsole,
    ];

    pub fn group(self) -> &'static str {
        use KeyFunction::*;
        match self {
            MoveUp | MoveDown | MoveLeft | MoveRight | Walk | Sprint => "Движение",
            SwapHands | Drop | ThrowItemInHand | ActivateItemInHand | ActivateItemInWorld
            | SmartEquipBelt | SmartEquipBackpack | ReleasePulledObject | Reload
            | EditorCopyObject => "Предметы",
            ToggleCombatMode | Hotbar1 | Hotbar2 | Hotbar3 => "Бой и действия",
            _ => "Интерфейс",
        }
    }

    pub fn title(self) -> &'static str {
        use KeyFunction::*;
        match self {
            MoveUp => "Идти вверх",
            MoveDown => "Идти вниз",
            MoveLeft => "Идти влево",
            MoveRight => "Идти вправо",
            Walk => "Идти шагом (удерживать)",
            Sprint => "Спринт",
            SwapHands => "Сменить руку",
            Drop => "Положить предмет",
            ThrowItemInHand => "Бросить предмет",
            ActivateItemInHand => "Использовать предмет в руке",
            ActivateItemInWorld => "Использовать предмет в мире",
            SmartEquipBelt => "Быстрая экипировка: пояс",
            SmartEquipBackpack => "Быстрая экипировка: рюкзак",
            ReleasePulledObject => "Отпустить",
            Reload => "Перезарядить",
            EditorCopyObject => "Копировать сущность",
            ToggleCombatMode => "Боевой режим",
            Hotbar1 => "Действие 1",
            Hotbar2 => "Действие 2",
            Hotbar3 => "Действие 3",
            FocusChat => "Открыть чат",
            OpenCraftingMenu => "Меню крафта",
            OpenCharacterMenu => "Персонаж",
            OpenBackpack => "Рюкзак",
            OpenEmotesMenu => "Эмоции",
            OpenGuidebook => "Руководство",
            OpenLanguageMenu => "Языки",
            OpenAHelp => "Админ-помощь",
            OpenSandboxWindow => "Меню спавна",
            OpenAdminMenu => "Админ-меню",
            ShowDebugConsole => "Консоль",
        }
    }

    /// Привязки по умолчанию (основная, запасная) — как было в игре.
    pub fn defaults(self) -> [Option<Binding>; 2] {
        use KeyCode as K;
        use KeyFunction::*;
        let one = |b: Binding| [Some(b), None];
        match self {
            MoveUp => [Some(Binding::key(K::KeyW)), Some(Binding::key(K::ArrowUp))],
            MoveDown => [Some(Binding::key(K::KeyS)), Some(Binding::key(K::ArrowDown))],
            MoveLeft => [Some(Binding::key(K::KeyA)), Some(Binding::key(K::ArrowLeft))],
            MoveRight => [Some(Binding::key(K::KeyD)), Some(Binding::key(K::ArrowRight))],
            Walk => [
                Some(Binding::key(K::ShiftLeft)),
                Some(Binding::key(K::ShiftRight)),
            ],
            Sprint => one(Binding::key(K::Space)),
            SwapHands => one(Binding::key(K::KeyX)),
            Drop => one(Binding::key(K::KeyQ)),
            ThrowItemInHand => one(Binding::ctrl(K::KeyQ)),
            ActivateItemInHand => one(Binding::key(K::KeyZ)),
            ActivateItemInWorld => one(Binding::key(K::KeyE)),
            SmartEquipBelt => one(Binding::shift(K::KeyE)),
            SmartEquipBackpack => one(Binding::shift(K::KeyB)),
            ReleasePulledObject => one(Binding::key(K::KeyH)),
            Reload => one(Binding::key(K::KeyR)),
            EditorCopyObject => one(Binding::key(K::KeyP)),
            ToggleCombatMode => one(Binding::key(K::KeyF)),
            Hotbar1 => one(Binding::key(K::Digit1)),
            Hotbar2 => one(Binding::key(K::Digit2)),
            Hotbar3 => one(Binding::key(K::Digit3)),
            FocusChat => one(Binding::key(K::KeyT)),
            OpenCraftingMenu => one(Binding::key(K::KeyG)),
            OpenCharacterMenu => one(Binding::key(K::KeyI)),
            OpenBackpack => one(Binding::key(K::KeyV)),
            OpenEmotesMenu => one(Binding::key(K::KeyY)),
            OpenGuidebook => one(Binding::key(K::Numpad0)),
            OpenLanguageMenu => one(Binding::key(K::KeyL)),
            OpenAHelp => one(Binding::key(K::F1)),
            OpenSandboxWindow => one(Binding::key(K::F5)),
            OpenAdminMenu => one(Binding::key(K::F7)),
            ShowDebugConsole => one(Binding::key(K::Backquote)),
        }
    }

    /// Удерживаемая функция: срабатывает при любых модификаторах.
    fn held(self) -> bool {
        matches!(
            self,
            KeyFunction::MoveUp
                | KeyFunction::MoveDown
                | KeyFunction::MoveLeft
                | KeyFunction::MoveRight
                | KeyFunction::Walk
        )
    }
}

/// Таблица клавиш: код, имя в файле, подпись в интерфейсе.
const KEYS: &[(KeyCode, &str, &str)] = &[
    (KeyCode::KeyA, "A", "A"),
    (KeyCode::KeyB, "B", "B"),
    (KeyCode::KeyC, "C", "C"),
    (KeyCode::KeyD, "D", "D"),
    (KeyCode::KeyE, "E", "E"),
    (KeyCode::KeyF, "F", "F"),
    (KeyCode::KeyG, "G", "G"),
    (KeyCode::KeyH, "H", "H"),
    (KeyCode::KeyI, "I", "I"),
    (KeyCode::KeyJ, "J", "J"),
    (KeyCode::KeyK, "K", "K"),
    (KeyCode::KeyL, "L", "L"),
    (KeyCode::KeyM, "M", "M"),
    (KeyCode::KeyN, "N", "N"),
    (KeyCode::KeyO, "O", "O"),
    (KeyCode::KeyP, "P", "P"),
    (KeyCode::KeyQ, "Q", "Q"),
    (KeyCode::KeyR, "R", "R"),
    (KeyCode::KeyS, "S", "S"),
    (KeyCode::KeyT, "T", "T"),
    (KeyCode::KeyU, "U", "U"),
    (KeyCode::KeyV, "V", "V"),
    (KeyCode::KeyW, "W", "W"),
    (KeyCode::KeyX, "X", "X"),
    (KeyCode::KeyY, "Y", "Y"),
    (KeyCode::KeyZ, "Z", "Z"),
    (KeyCode::Digit0, "0", "0"),
    (KeyCode::Digit1, "1", "1"),
    (KeyCode::Digit2, "2", "2"),
    (KeyCode::Digit3, "3", "3"),
    (KeyCode::Digit4, "4", "4"),
    (KeyCode::Digit5, "5", "5"),
    (KeyCode::Digit6, "6", "6"),
    (KeyCode::Digit7, "7", "7"),
    (KeyCode::Digit8, "8", "8"),
    (KeyCode::Digit9, "9", "9"),
    (KeyCode::Numpad0, "Num0", "Num0"),
    (KeyCode::Numpad1, "Num1", "Num1"),
    (KeyCode::Numpad2, "Num2", "Num2"),
    (KeyCode::Numpad3, "Num3", "Num3"),
    (KeyCode::Numpad4, "Num4", "Num4"),
    (KeyCode::Numpad5, "Num5", "Num5"),
    (KeyCode::Numpad6, "Num6", "Num6"),
    (KeyCode::Numpad7, "Num7", "Num7"),
    (KeyCode::Numpad8, "Num8", "Num8"),
    (KeyCode::Numpad9, "Num9", "Num9"),
    (KeyCode::F1, "F1", "F1"),
    (KeyCode::F2, "F2", "F2"),
    (KeyCode::F3, "F3", "F3"),
    (KeyCode::F4, "F4", "F4"),
    (KeyCode::F5, "F5", "F5"),
    (KeyCode::F6, "F6", "F6"),
    (KeyCode::F7, "F7", "F7"),
    (KeyCode::F8, "F8", "F8"),
    (KeyCode::F9, "F9", "F9"),
    (KeyCode::F10, "F10", "F10"),
    (KeyCode::F11, "F11", "F11"),
    (KeyCode::F12, "F12", "F12"),
    (KeyCode::ArrowUp, "Up", "↑"),
    (KeyCode::ArrowDown, "Down", "↓"),
    (KeyCode::ArrowLeft, "Left", "←"),
    (KeyCode::ArrowRight, "Right", "→"),
    (KeyCode::Space, "Space", "Пробел"),
    (KeyCode::Tab, "Tab", "Tab"),
    (KeyCode::Enter, "Enter", "Enter"),
    (KeyCode::Backquote, "Backquote", "`"),
    (KeyCode::Minus, "Minus", "-"),
    (KeyCode::Equal, "Equal", "="),
    (KeyCode::BracketLeft, "BracketLeft", "["),
    (KeyCode::BracketRight, "BracketRight", "]"),
    (KeyCode::Semicolon, "Semicolon", ";"),
    (KeyCode::Quote, "Quote", "'"),
    (KeyCode::Comma, "Comma", ","),
    (KeyCode::Period, "Period", "."),
    (KeyCode::Slash, "Slash", "/"),
    (KeyCode::Backslash, "Backslash", "\\"),
    (KeyCode::Insert, "Insert", "Insert"),
    (KeyCode::Delete, "Delete", "Delete"),
    (KeyCode::Home, "Home", "Home"),
    (KeyCode::End, "End", "End"),
    (KeyCode::PageUp, "PageUp", "PageUp"),
    (KeyCode::PageDown, "PageDown", "PageDown"),
    (KeyCode::ShiftLeft, "ShiftLeft", "Левый Shift"),
    (KeyCode::ShiftRight, "ShiftRight", "Правый Shift"),
    (KeyCode::ControlLeft, "ControlLeft", "Левый Ctrl"),
    (KeyCode::ControlRight, "ControlRight", "Правый Ctrl"),
    (KeyCode::AltLeft, "AltLeft", "Левый Alt"),
    (KeyCode::AltRight, "AltRight", "Правый Alt"),
    (KeyCode::CapsLock, "CapsLock", "CapsLock"),
];

pub fn key_label(key: KeyCode) -> &'static str {
    KEYS.iter()
        .find(|(code, _, _)| *code == key)
        .map(|(_, _, label)| *label)
        .unwrap_or("?")
}

fn key_name(key: KeyCode) -> Option<&'static str> {
    KEYS.iter().find(|(code, _, _)| *code == key).map(|(_, name, _)| *name)
}

fn key_from_name(name: &str) -> Option<KeyCode> {
    KEYS.iter().find(|(_, n, _)| *n == name).map(|(code, _, _)| *code)
}

fn is_modifier(key: KeyCode) -> bool {
    matches!(
        key,
        KeyCode::ShiftLeft
            | KeyCode::ShiftRight
            | KeyCode::ControlLeft
            | KeyCode::ControlRight
            | KeyCode::AltLeft
            | KeyCode::AltRight
    )
}

/// Привязка в файле: «Ctrl+Q», «Shift+E», «Space».
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct SavedBinds(Vec<(KeyFunction, Vec<String>)>);

fn binding_to_string(binding: &Binding) -> Option<String> {
    let mut parts = Vec::new();
    if binding.ctrl {
        parts.push("Ctrl");
    }
    if binding.shift {
        parts.push("Shift");
    }
    if binding.alt {
        parts.push("Alt");
    }
    parts.push(key_name(binding.key)?);
    Some(parts.join("+"))
}

fn binding_from_string(text: &str) -> Option<Binding> {
    let mut binding = Binding::key(KeyCode::KeyA);
    let mut key = None;
    for part in text.split('+') {
        match part {
            "Ctrl" => binding.ctrl = true,
            "Shift" => binding.shift = true,
            "Alt" => binding.alt = true,
            name => key = key_from_name(name),
        }
    }
    binding.key = key?;
    Some(binding)
}

/// Текущие привязки.
#[derive(Resource, Clone, Debug, PartialEq)]
pub struct Keybinds {
    binds: HashMap<KeyFunction, [Option<Binding>; 2]>,
    /// Идёт захват клавиши в настройках — игровые хоткеи молчат.
    pub capturing: bool,
}

impl Default for Keybinds {
    fn default() -> Self {
        Self {
            binds: KeyFunction::ALL
                .into_iter()
                .map(|f| (f, f.defaults()))
                .collect(),
            capturing: false,
        }
    }
}

fn keybinds_path() -> std::path::PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".ssr-keybinds.ron")
}

impl Keybinds {
    pub fn get(&self, function: KeyFunction) -> [Option<Binding>; 2] {
        self.binds.get(&function).copied().unwrap_or([None, None])
    }

    pub fn set(&mut self, function: KeyFunction, slot: usize, binding: Option<Binding>) {
        let entry = self.binds.entry(function).or_insert([None, None]);
        entry[slot.min(1)] = binding;
    }

    /// Нажата в этом кадре (модификаторы — точно как в привязке).
    pub fn just_pressed(&self, function: KeyFunction, keys: &ButtonInput<KeyCode>) -> bool {
        if self.capturing {
            return false;
        }
        let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
        self.get(function).into_iter().flatten().any(|b| {
            keys.just_pressed(b.key)
                && (function.held() || (b.ctrl == ctrl && b.shift == shift && b.alt == alt))
        })
    }

    /// Удерживается (для движения и ходьбы — модификаторы не важны).
    pub fn pressed(&self, function: KeyFunction, keys: &ButtonInput<KeyCode>) -> bool {
        if self.capturing {
            return false;
        }
        self.get(function)
            .into_iter()
            .flatten()
            .any(|b| keys.pressed(b.key))
    }

    /// Подпись основной привязки (для подсказок на кнопках HUD).
    pub fn short_label(&self, function: KeyFunction) -> String {
        self.get(function)[0].map(|b| b.label()).unwrap_or_default()
    }

    /// Функции с той же привязкой (подсветка конфликтов).
    pub fn conflicts(&self, function: KeyFunction, binding: &Binding) -> Vec<KeyFunction> {
        KeyFunction::ALL
            .into_iter()
            .filter(|other| *other != function)
            .filter(|other| self.get(*other).iter().flatten().any(|b| b == binding))
            .collect()
    }

    pub fn load() -> Self {
        let mut binds = Self::default();
        let Ok(text) = std::fs::read_to_string(keybinds_path()) else {
            return binds;
        };
        match ron::from_str::<SavedBinds>(&text) {
            Ok(saved) => {
                for (function, list) in saved.0 {
                    let mut slots = [None, None];
                    for (slot, text) in list.iter().take(2).enumerate() {
                        slots[slot] = binding_from_string(text);
                    }
                    binds.binds.insert(function, slots);
                }
            }
            Err(error) => tracing::warn!(%error, "keybinds: файл не прочитан, по умолчанию"),
        }
        binds
    }

    pub fn save(&self) {
        let saved = SavedBinds(
            KeyFunction::ALL
                .into_iter()
                .map(|f| {
                    let list = self
                        .get(f)
                        .iter()
                        .map(|b| b.and_then(|b| binding_to_string(&b)).unwrap_or_default())
                        .collect();
                    (f, list)
                })
                .collect(),
        );
        match ron::ser::to_string_pretty(&saved, ron::ser::PrettyConfig::default()) {
            Ok(text) => {
                if let Err(error) = std::fs::write(keybinds_path(), text) {
                    tracing::warn!(%error, "keybinds: не сохранено");
                }
            }
            Err(error) => tracing::warn!(%error, "keybinds: не сериализовано"),
        }
    }
}

// ------------------------------------------------- вкладка «Управление»

/// Контейнер списка привязок во вкладке «Управление» (наполняет система).
#[derive(Component)]
pub struct KeybindsPanel;

/// Кнопка привязки: функция и слот (0 — основная, 1 — запасная).
#[derive(Component, Clone, Copy)]
pub struct BindButton {
    function: KeyFunction,
    slot: usize,
}

/// Кнопка «Сбросить всё».
#[derive(Component)]
pub struct ResetBindsButton;

/// Какая привязка ждёт нажатия; прокрутка списка.
#[derive(Resource, Default)]
pub struct RebindState {
    pub capture: Option<(KeyFunction, usize)>,
    pub scroll: usize,
}

/// Строк списка в окне (дальше — колесо мыши).
const VISIBLE: usize = 12;

/// Содержимое вкладки: строки «функция | основная | запасная».
#[allow(clippy::type_complexity)]
pub fn render_keybinds_panel(
    mut commands: Commands,
    binds: Res<Keybinds>,
    state: Res<RebindState>,
    panels: Query<(Entity, Option<&Children>), With<KeybindsPanel>>,
    added: Query<(), Added<KeybindsPanel>>,
) {
    if !binds.is_changed() && !state.is_changed() && added.is_empty() {
        return;
    }
    for (panel, children) in panels.iter() {
        if let Some(children) = children {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        commands.entity(panel).with_children(|list| {
            list.spawn((
                Text::new(
                    "Клик — назначить, ПКМ — снять, Esc — отмена. Мышь: Shift+ЛКМ — осмотр, \
                     Ctrl+ЛКМ — тянуть, Alt+ЛКМ — альтернативное действие, ПКМ — меню.",
                ),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgb(0.62, 0.62, 0.66)),
            ));
            let mut group = "";
            for function in KeyFunction::ALL.into_iter().skip(state.scroll).take(VISIBLE) {
                if function.group() != group {
                    group = function.group();
                    list.spawn((
                        Text::new(group),
                        TextFont::from_font_size(14.0),
                        TextColor(Color::srgb(1.0, 0.75, 0.25)),
                        Node {
                            margin: UiRect::top(px(4)),
                            ..default()
                        },
                    ));
                }
                list.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                    height: px(26),
                    ..default()
                })
                .with_children(|row| {
                    // Подпись в своей ячейке фиксированной ширины: у самого
                    // текста ширина ломала выравнивание строки.
                    row.spawn(Node {
                        width: px(250),
                        height: px(26),
                        align_items: AlignItems::Center,
                        overflow: Overflow::clip(),
                        ..default()
                    })
                    .with_child((
                        Text::new(function.title()),
                        TextFont::from_font_size(13.0),
                        TextColor(Color::srgb(0.88, 0.88, 0.90)),
                    ));
                    for (slot, binding) in binds.get(function).into_iter().enumerate() {
                        let capturing = state.capture == Some((function, slot));
                        let conflict = binding.is_some_and(|b| !binds.conflicts(function, &b).is_empty());
                        let label = if capturing {
                            "Нажмите клавишу…".to_string()
                        } else {
                            binding.map(|b| b.label()).unwrap_or_else(|| "—".to_string())
                        };
                        let color = if conflict {
                            Color::srgb(0.95, 0.45, 0.35)
                        } else {
                            Color::srgb(0.88, 0.88, 0.90)
                        };
                        row.spawn((
                            BindButton { function, slot },
                            Button,
                            Node {
                                width: px(100),
                                height: px(24),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                border: UiRect::all(px(1)),
                                ..default()
                            },
                            BackgroundColor(if capturing {
                                Color::srgb(0.30, 0.26, 0.12)
                            } else {
                                Color::srgb(0.13, 0.13, 0.16)
                            }),
                            BorderColor::from(Color::srgb(0.30, 0.30, 0.35)),
                        ))
                        .with_child((Text::new(label), TextFont::from_font_size(12.0), TextColor(color)));
                    }
                });
            }
            list.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: px(8),
                align_items: AlignItems::Center,
                margin: UiRect::top(px(6)),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    ResetBindsButton,
                    Button,
                    Node {
                        padding: UiRect::axes(px(10), px(4)),
                        border: UiRect::all(px(1)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.42, 0.13, 0.13)),
                    BorderColor::from(Color::srgb(0.30, 0.30, 0.35)),
                ))
                .with_child((
                    Text::new("Сбросить всё"),
                    TextFont::from_font_size(13.0),
                    TextColor(Color::srgb(0.88, 0.88, 0.90)),
                ));
                let total = KeyFunction::ALL.len();
                row.spawn((
                    Text::new(format!(
                        "Строки {}–{} из {} (колесо мыши)",
                        state.scroll + 1,
                        (state.scroll + VISIBLE).min(total),
                        total
                    )),
                    TextFont::from_font_size(12.0),
                    TextColor(Color::srgb(0.62, 0.62, 0.66)),
                ));
            });
        });
    }
}

/// Клики по привязкам: ЛКМ — ждать клавишу, ПКМ — снять; «Сбросить всё».
pub fn keybind_clicks(
    mouse: Res<ButtonInput<MouseButton>>,
    mut binds: ResMut<Keybinds>,
    mut state: ResMut<RebindState>,
    buttons: Query<(&Interaction, &BindButton)>,
    reset: Query<&Interaction, (Changed<Interaction>, With<ResetBindsButton>)>,
) {
    for (interaction, button) in buttons.iter() {
        if *interaction == Interaction::Pressed && mouse.just_pressed(MouseButton::Left) {
            state.capture = Some((button.function, button.slot));
            binds.capturing = true;
        }
        if *interaction != Interaction::None && mouse.just_pressed(MouseButton::Right) {
            binds.set(button.function, button.slot, None);
            binds.save();
        }
    }
    if reset.iter().any(|i| *i == Interaction::Pressed) {
        *binds = Keybinds::default();
        binds.save();
        state.capture = None;
    }
}

/// Захват клавиши: первая немодификаторная клавиша с зажатыми модификаторами
/// (или сам модификатор для «Идти шагом»); Esc — отмена.
pub fn capture_keybind(
    keys: Res<ButtonInput<KeyCode>>,
    mut binds: ResMut<Keybinds>,
    mut state: ResMut<RebindState>,
) {
    let Some((function, slot)) = state.capture else {
        return;
    };
    if keys.just_pressed(KeyCode::Escape) {
        state.capture = None;
        binds.capturing = false;
        return;
    }
    let ctrl = keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]);
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    let pressed = keys.get_just_pressed().copied().find(|key| {
        key_name(*key).is_some() && (!is_modifier(*key) || function.held())
    });
    let Some(key) = pressed else {
        return;
    };
    let binding = if is_modifier(key) {
        Binding::key(key)
    } else {
        Binding {
            key,
            ctrl,
            shift,
            alt,
        }
    };
    binds.set(function, slot, Some(binding));
    binds.capturing = false;
    binds.save();
    state.capture = None;
    tracing::info!(?function, slot, binding = %binding.label(), "keybind changed");
}

/// Колесо мыши над окном настроек — прокрутка списка привязок.
pub fn scroll_keybinds(
    mut wheel: MessageReader<MouseWheel>,
    panels: Query<(), With<KeybindsPanel>>,
    mut state: ResMut<RebindState>,
) {
    if panels.is_empty() {
        return;
    }
    let delta: f32 = wheel.read().map(|e| e.y).sum();
    if delta == 0.0 {
        return;
    }
    let max = KeyFunction::ALL.len().saturating_sub(VISIBLE);
    let next = (state.scroll as i64 - delta.signum() as i64 * 2).clamp(0, max as i64);
    state.scroll = next as usize;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_modifiers_separate_drop_and_throw() {
        let binds = Keybinds::default();
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::KeyQ);
        assert!(binds.just_pressed(KeyFunction::Drop, &keys));
        assert!(!binds.just_pressed(KeyFunction::ThrowItemInHand, &keys));
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::ControlLeft);
        keys.press(KeyCode::KeyQ);
        assert!(!binds.just_pressed(KeyFunction::Drop, &keys));
        assert!(binds.just_pressed(KeyFunction::ThrowItemInHand, &keys));
    }

    #[test]
    fn movement_ignores_modifiers() {
        let binds = Keybinds::default();
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::ShiftLeft);
        keys.press(KeyCode::KeyW);
        assert!(binds.pressed(KeyFunction::MoveUp, &keys));
        assert!(binds.pressed(KeyFunction::Walk, &keys));
    }

    #[test]
    fn bindings_roundtrip_through_text() {
        for function in KeyFunction::ALL {
            for binding in function.defaults().into_iter().flatten() {
                let text = binding_to_string(&binding).unwrap();
                assert_eq!(binding_from_string(&text), Some(binding), "{text}");
            }
        }
        assert_eq!(Binding::shift(KeyCode::KeyE).label(), "Shift+E");
    }

    #[test]
    fn defaults_have_no_conflicts() {
        let binds = Keybinds::default();
        for function in KeyFunction::ALL {
            for binding in binds.get(function).into_iter().flatten() {
                assert!(
                    binds.conflicts(function, &binding).is_empty(),
                    "{function:?}: {}",
                    binding.label()
                );
            }
        }
    }
}
