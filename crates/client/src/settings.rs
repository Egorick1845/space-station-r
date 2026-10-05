//! Настройки клиента: меню по Esc, громкость, полный экран, показ FPS.
//! Сохраняются в `~/.ssr-settings.ron` и переживают перезапуск.

use bevy::audio::{GlobalVolume, Volume};
use bevy::diagnostic::DiagnosticsStore;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowMode};
use serde::{Deserialize, Serialize};

/// Настройки игрока.
#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub volume: f32,
    pub fullscreen: bool,
    pub show_fps: bool,
    /// Имя игрока (T5.4): уходит в Connect.
    pub player_name: String,
    /// Адрес сервера "host:port" (T5.4); пусто — локальный из SSR_PORT.
    pub server: String,
    /// Сохранённый список серверов (T5.4).
    pub servers: Vec<String>,
    /// Активная вкладка окна настроек (не сохраняется между запусками).
    #[serde(skip)]
    pub tab: SettingsTab,
    /// Показывать прицел боевого режима у курсора
    /// (`hud.combat_mode_indicators_point_show` в сборке, default true).
    pub combat_indicators: bool,
    /// Процедурная анимация шага (`accessibility.foot_walk_animation`, default true).
    pub foot_walk_animation: bool,
    /// Показывать призраков живым (команда `showghosts` в сборке).
    #[serde(default)]
    pub show_ghosts: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            volume: 0.8,
            fullscreen: false,
            show_fps: false,
            player_name: "Игрок".to_string(),
            server: String::new(),
            servers: vec!["127.0.0.1:7777".to_string()],
            tab: SettingsTab::default(),
            // Как CVar-ы сборки: `hud.combat_mode_indicators_point_show = true`,
            // `accessibility.foot_walk_animation = true`.
            combat_indicators: true,
            foot_walk_animation: true,
            show_ghosts: false,
        }
    }
}

/// Файл настроек в домашнем каталоге.
fn settings_path() -> std::path::PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".ssr-settings.ron")
}

impl Settings {
    pub fn load() -> Self {
        let path = settings_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| match ron::from_str(&text) {
                Ok(settings) => Some(settings),
                Err(e) => {
                    tracing::warn!(error = %e, "settings parse failed, defaults used");
                    None
                }
            })
            .unwrap_or_default()
    }

    pub fn save(&self) {
        match ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()) {
            Ok(text) => {
                if let Err(e) = std::fs::write(settings_path(), text) {
                    tracing::warn!(error = %e, "settings save failed");
                }
            }
            Err(e) => tracing::warn!(error = %e, "settings serialize failed"),
        }
    }
}

/// Корень окна настроек.
#[derive(Component)]
pub struct SettingsMenu;

/// Корень игрового меню (Escape) — как `EscapeMenu.xaml` в сборке.
#[derive(Component)]
pub struct EscapeMenuRoot;

/// Кнопка игрового меню.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum EscapeAction {
    /// «Продолжить» — закрыть меню.
    Resume,
    /// «Настройки» — открыть окно настроек (`ui-escape-options`).
    Options,
    /// «Управление» — окно настроек на вкладке «Управление».
    Controls,
    /// «Выйти» — завершить клиент (`ui-escape-quit`).
    Quit,
}

/// Кнопка меню настроек.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    ToggleFullscreen,
    ToggleFps,
    ToggleCombatIndicators,
    ToggleFootWalk,
    Close,
}

/// Текст значения настройки (обновляется при изменении).
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum SettingsValue {
    Volume,
    Fullscreen,
    Fps,
    CombatIndicators,
    FootWalk,
}

/// Активная вкладка окна настроек — порядок и названия как у вкладок
/// `OptionsMenu.xaml` сборки (Основные, Графика, Управление, Аудио, Доступность).
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum SettingsTab {
    Misc,
    #[default]
    Graphics,
    Controls,
    Audio,
    Accessibility,
}

impl SettingsTab {
    /// Все вкладки в порядке XAML сборки (без «Админ» и «Сеть»: таких настроек
    /// у нас пока нет).
    pub const ALL: [SettingsTab; 5] = [
        SettingsTab::Misc,
        SettingsTab::Graphics,
        SettingsTab::Controls,
        SettingsTab::Audio,
        SettingsTab::Accessibility,
    ];

    /// Заголовок вкладки (`ui-options-tab-*` из options-menu.ftl).
    pub fn title(self) -> &'static str {
        match self {
            SettingsTab::Misc => "Основные",
            SettingsTab::Graphics => "Графика",
            SettingsTab::Controls => "Управление",
            SettingsTab::Audio => "Аудио",
            SettingsTab::Accessibility => "Доступность",
        }
    }
}

/// Кнопка вкладки.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub struct TabButton(pub SettingsTab);

/// Дорожка ползунка громкости (клик и протяжка задают значение).
#[derive(Component)]
pub struct VolumeSlider;

/// Ручка ползунка.
#[derive(Component)]
pub struct VolumeKnob;

/// Текст FPS в углу экрана.
#[derive(Component)]
pub struct FpsText;

/// Загружает настройки при старте и создаёт строку FPS.
pub fn load_settings(mut commands: Commands) {
    let settings = Settings::load();
    tracing::info!(
        volume = settings.volume,
        fullscreen = settings.fullscreen,
        show_fps = settings.show_fps,
        "settings loaded"
    );
    commands.insert_resource(settings);
}

/// Создаёт текст FPS (показывается по настройке).
pub fn spawn_fps_text(mut commands: Commands) {
    commands.spawn((
        FpsText,
        Text::new("FPS —"),
        TextFont::from_font_size(13.0),
        TextColor(Color::srgb(0.7, 0.85, 0.7)),
        Node {
            position_type: PositionType::Absolute,
            right: px(10),
            top: px(8),
            ..default()
        },
    ));
}

/// Применяет сохранённый режим окна при старте.
pub fn apply_saved_window_mode(settings: Res<Settings>, mut windows: Query<&mut Window>) {
    if !settings.fullscreen {
        return;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.mode = WindowMode::BorderlessFullscreen(MonitorSelection::Primary);
    }
}

/// Тест интерфейса (SSR_MENU_TEST): `1` — через 2 с открыть игровое меню,
/// `2` — игровое меню, через 5 с окно настроек. Для скриншот-проверки.
pub fn menu_test(
    mut commands: Commands,
    mut settings: ResMut<Settings>,
    mut state: Local<(u8, f32)>,
    time: Res<Time>,
    escape_root: Query<Entity, With<EscapeMenuRoot>>,
    settings_root: Query<Entity, With<SettingsMenu>>,
) {
    let Ok(mode) = std::env::var("SSR_MENU_TEST") else {
        return;
    };
    let mode: u8 = mode.parse().unwrap_or(1);
    if state.0 == 0 {
        state.1 += time.delta_secs();
        if state.1 >= 2.0 {
            state.0 = 1;
            if escape_root.is_empty() {
                spawn_escape_menu(&mut commands, &settings);
            }
        }
        return;
    }
    if mode >= 2 && state.0 == 1 {
        state.1 += time.delta_secs();
        if state.1 >= 7.0 {
            state.0 = 2;
            for entity in escape_root.iter() {
                commands.entity(entity).despawn();
            }
            for entity in settings_root.iter() {
                commands.entity(entity).despawn();
            }
            settings.tab = SettingsTab::Graphics;
            spawn_menu(&mut commands, &settings);
        }
    }
}

/// Применяет громкость к общему миксу.
pub fn apply_volume(settings: Res<Settings>, mut volume: ResMut<GlobalVolume>) {
    if settings.is_changed() {
        volume.volume = Volume::Linear(settings.volume.clamp(0.0, 1.0));
    }
}

/// Esc: закрыть верхнее окно, иначе открыть/закрыть игровое меню
/// (`EscapeContextUIController` в сборке: `CloseMostRecentWindow` →
/// `ToggleWindow`).
pub fn toggle_escape_menu(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<crate::console::Console>,
    settings: Res<Settings>,
    escape_root: Query<Entity, With<EscapeMenuRoot>>,
    settings_root: Query<Entity, With<SettingsMenu>>,
) {
    if !keys.just_pressed(KeyCode::Escape) || console.open {
        return;
    }
    // Открыто окно настроек — Escape закрывает именно его.
    let settings_open: Vec<Entity> = settings_root.iter().collect();
    if !settings_open.is_empty() {
        for entity in settings_open {
            commands.entity(entity).despawn();
        }
        return;
    }
    let existing: Vec<Entity> = escape_root.iter().collect();
    for &entity in &existing {
        commands.entity(entity).despawn();
    }
    if existing.is_empty() {
        spawn_escape_menu(&mut commands, &settings);
    }
}

/// Игровое меню (`EscapeMenu.xaml`): заголовок окна, список кнопок в
/// `BoxContainer` с разделителями `LowDivider`, «Выйти» красной кнопкой.
fn spawn_escape_menu(commands: &mut Commands, settings: &Settings) {
    let _ = settings;
    commands
        .spawn((
            EscapeMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(38.0),
                top: Val::Percent(28.0),
                width: px(220),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                padding: UiRect::all(px(10)),
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.96)),
            BorderColor::from(Color::srgb(0.30, 0.30, 0.36)),
        ))
        .with_children(|window| {
            window
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        padding: UiRect::axes(px(8), px(3)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.133, 0.133, 0.165, 0.85)),
                ))
                .with_child((
                    Text::new("Игровое меню"),
                    TextFont::from_font_size(16.0),
                    TextColor(Color::srgb(0.92, 0.95, 1.0)),
                ));
            escape_button(window, "Продолжить", EscapeAction::Resume, false);
            divider(window);
            escape_button(window, "Настройки", EscapeAction::Options, false);
            escape_button(window, "Управление", EscapeAction::Controls, false);
            divider(window);
            escape_button(window, "Выйти", EscapeAction::Quit, true);
        });
}

/// Кнопка игрового меню (`MinWidth 150`, ContentMargin 14/2 в StyleNano).
fn escape_button(
    window: &mut RelatedSpawnerCommands<'_, ChildOf>,
    label: &str,
    action: EscapeAction,
    danger: bool,
) {
    window
        .spawn((
            action,
            Button,
            Node {
                width: Val::Percent(100.0),
                height: px(28),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(if danger {
                Color::srgba(0.831, 0.231, 0.231, 0.85)
            } else {
                Color::srgba(0.275, 0.286, 0.400, 0.85)
            }),
            BorderColor::from(Color::srgb(0.30, 0.30, 0.35)),
        ))
        .with_child((
            Text::new(label.to_string()),
            TextFont::from_font_size(14.0),
            TextColor(Color::srgb(0.92, 0.92, 0.94)),
        ));
}

/// Разделитель `LowDivider` (`Margin="0 2.5"`, цвет `#444`).
fn divider(window: &mut RelatedSpawnerCommands<'_, ChildOf>) {
    window.spawn((
        Node {
            width: Val::Percent(100.0),
            height: px(2),
            margin: UiRect::vertical(px(2)),
            ..default()
        },
        BackgroundColor(Color::srgb(0.267, 0.267, 0.267)),
    ));
}

/// Клики по кнопкам игрового меню.
type EscapeClicks<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static EscapeAction),
    (Changed<Interaction>, With<Button>),
>;

/// Обработка игрового меню: продолжить, настройки, управление, выход.
pub fn escape_click(
    mut commands: Commands,
    mut settings: ResMut<Settings>,
    mut exit: MessageWriter<AppExit>,
    escape_root: Query<Entity, With<EscapeMenuRoot>>,
    settings_root: Query<Entity, With<SettingsMenu>>,
    clicks: EscapeClicks,
) {
    for (interaction, action) in clicks.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            EscapeAction::Resume => {
                for entity in escape_root.iter() {
                    commands.entity(entity).despawn();
                }
            }
            EscapeAction::Options | EscapeAction::Controls => {
                settings.tab = if matches!(action, EscapeAction::Controls) {
                    SettingsTab::Controls
                } else {
                    settings.tab
                };
                for entity in escape_root.iter() {
                    commands.entity(entity).despawn();
                }
                for entity in settings_root.iter() {
                    commands.entity(entity).despawn();
                }
                spawn_menu(&mut commands, &settings);
            }
            EscapeAction::Quit => {
                tracing::info!("escape menu: quit");
                exit.write(AppExit::Success);
            }
        }
    }
}

/// Открывает меню настроек (кнопка HUD делает то же, что Esc).
pub fn open_menu(
    commands: &mut Commands,
    settings: &Settings,
    root: &Query<Entity, With<SettingsMenu>>,
) {
    let existing: Vec<Entity> = root.iter().collect();
    for &entity in &existing {
        commands.entity(entity).despawn();
    }
    if existing.is_empty() {
        spawn_menu(commands, settings);
    }
}

/// Строит окно настроек: заголовок, вкладки и содержимое активной вкладки.
fn spawn_menu(commands: &mut Commands, settings: &Settings) {
    commands
        .spawn((
            SettingsMenu,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(33.0),
                top: Val::Percent(14.0),
                width: px(470),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(12)),
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.96)),
            BorderColor::from(Color::srgb(0.30, 0.30, 0.36)),
        ))
        .with_children(|panel| {
            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        padding: UiRect::axes(px(8), px(3)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.10, 0.10, 0.13)),
                ))
                .with_child((
                    Text::new("Игровые настройки"),
                    TextFont::from_font_size(16.0),
                    TextColor(Color::srgb(1.0, 0.75, 0.25)),
                ));
            tabs_row(panel, settings.tab);
            match settings.tab {
                SettingsTab::Misc => {
                    settings_row(
                        panel,
                        "Показывать счётчик FPS",
                        if settings.show_fps {
                            "Вкл"
                        } else {
                            "Выкл"
                        },
                        SettingsValue::Fps,
                        SettingsAction::ToggleFps,
                        SettingsAction::ToggleFps,
                    );
                    settings_row(
                        panel,
                        "Прицел боевого режима у курсора",
                        if settings.combat_indicators {
                            "Вкл"
                        } else {
                            "Выкл"
                        },
                        SettingsValue::CombatIndicators,
                        SettingsAction::ToggleCombatIndicators,
                        SettingsAction::ToggleCombatIndicators,
                    );
                }
                SettingsTab::Graphics => {
                    settings_row(
                        panel,
                        "Полный экран",
                        if settings.fullscreen {
                            "Вкл"
                        } else {
                            "Выкл"
                        },
                        SettingsValue::Fullscreen,
                        SettingsAction::ToggleFullscreen,
                        SettingsAction::ToggleFullscreen,
                    );
                    settings_row(
                        panel,
                        "Показывать счётчик FPS",
                        if settings.show_fps {
                            "Вкл"
                        } else {
                            "Выкл"
                        },
                        SettingsValue::Fps,
                        SettingsAction::ToggleFps,
                        SettingsAction::ToggleFps,
                    );
                }
                SettingsTab::Audio => volume_slider(panel, settings.volume),
                SettingsTab::Controls => {
                    panel.spawn((
                        Text::new(KEYBINDS_HELP),
                        TextFont::from_font_size(12.0),
                        TextColor(Color::srgb(0.78, 0.78, 0.82)),
                    ));
                }
                SettingsTab::Accessibility => {
                    settings_row(
                        panel,
                        "Анимация шага",
                        if settings.foot_walk_animation {
                            "Вкл"
                        } else {
                            "Выкл"
                        },
                        SettingsValue::FootWalk,
                        SettingsAction::ToggleFootWalk,
                        SettingsAction::ToggleFootWalk,
                    );
                }
            }
            panel
                .spawn((
                    SettingsAction::Close,
                    Button,
                    Node {
                        height: px(32),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(px(2)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.13, 0.13, 0.16)),
                    BorderColor::from(Color::srgb(0.30, 0.30, 0.35)),
                ))
                .with_child((
                    Text::new("Закрыть"),
                    TextFont::from_font_size(14.0),
                    TextColor(Color::srgb(0.88, 0.88, 0.90)),
                ));
        });
}

/// Справка по управлению (вкладка «Управление»).
pub const KEYBINDS_HELP: &str = "WASD — движение, Shift — ходьба, Space — спринт
ЛКМ — использовать (в бою — удар), Ctrl+ЛКМ — тянуть/передать предмет
ПКМ — действия по объекту, E — действие, Shift+E — осмотр
1/2 — руки, X — сменить руку, Q — бросить предмет
F — боевой режим, C — крафт, I — персонаж, F5 — спавн-меню, F7 — админ-меню
` — консоль, Esc — закрыть окно/игровое меню";

/// Ряд вкладок окна настроек: порядок и подписи как в `OptionsMenu.xaml`.
fn tabs_row(panel: &mut RelatedSpawnerCommands<'_, ChildOf>, active: SettingsTab) {
    panel
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: px(4),
            ..default()
        })
        .with_children(|tabs| {
            for tab in SettingsTab::ALL {
                let selected = tab == active;
                tabs.spawn((
                    TabButton(tab),
                    Button,
                    Node {
                        height: px(26),
                        padding: UiRect::horizontal(px(12)),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(px(2)),
                        ..default()
                    },
                    // `glassTabActive` / `glassTabInactive` из StyleNano:
                    // активная #323240D9, неактивная #1A1A24B3.
                    BackgroundColor(if selected {
                        Color::srgba(0.196, 0.196, 0.251, 0.85)
                    } else {
                        Color::srgba(0.102, 0.102, 0.141, 0.70)
                    }),
                    BorderColor::from(if selected {
                        Color::srgb(1.0, 0.75, 0.25)
                    } else {
                        Color::srgb(0.30, 0.30, 0.35)
                    }),
                ))
                .with_child((
                    Text::new(tab.title()),
                    TextFont::from_font_size(13.0),
                    TextColor(Color::srgb(0.88, 0.88, 0.90)),
                ));
            }
        });
}

/// Ползунок громкости: дорожка и ручка (как слайдеры в опциях SS14).
fn volume_slider(panel: &mut RelatedSpawnerCommands<'_, ChildOf>, volume: f32) {
    panel
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(8),
                padding: UiRect::axes(px(6), px(3)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.12, 0.12, 0.15, 0.7)),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new("Громкость"),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.88, 0.88, 0.90)),
                Node {
                    width: px(110),
                    ..default()
                },
            ));
            row.spawn((
                VolumeSlider,
                Button,
                Node {
                    width: px(230),
                    height: px(16),
                    border: UiRect::all(px(2)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.16, 0.16, 0.20)),
                BorderColor::from(Color::srgb(0.34, 0.34, 0.40)),
            ))
            .with_child((
                VolumeKnob,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Percent(volume * 100.0),
                    top: px(-3),
                    width: px(10),
                    height: px(18),
                    ..default()
                },
                BackgroundColor(Color::srgb(1.0, 0.75, 0.25)),
            ));
            row.spawn((
                SettingsValue::Volume,
                Text::new(format!("{}%", (volume * 100.0).round())),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(1.0, 0.75, 0.25)),
            ));
        });
}

/// Клик или протяжка по дорожке задают громкость и двигают ручку.
pub fn volume_slider_drag(
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut settings: ResMut<Settings>,
    sliders: Query<(&Interaction, &ComputedNode, &GlobalTransform), With<VolumeSlider>>,
    mut knobs: Query<&mut Node, With<VolumeKnob>>,
) {
    if !mouse.pressed(MouseButton::Left) {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    for (interaction, computed, transform) in sliders.iter() {
        // Пока ЛКМ нажата на дорожке — значение следует за курсором.
        if !matches!(interaction, Interaction::Pressed | Interaction::Hovered) {
            continue;
        }
        let size = computed.size() * computed.inverse_scale_factor();
        let left = transform.translation().x - size.x / 2.0;
        let fraction = ((cursor.x - left) / size.x.max(1.0)).clamp(0.0, 1.0);
        if (fraction - settings.volume).abs() < 0.001 {
            continue;
        }
        settings.volume = fraction;
        settings.save();
        for mut knob in knobs.iter_mut() {
            knob.left = Val::Percent(fraction * 100.0);
        }
    }
}

/// Клики по вкладкам настроек.
type TabClicks<'w, 's> =
    Query<'w, 's, (&'static Interaction, &'static TabButton), (Changed<Interaction>, With<Button>)>;

/// Клик по вкладке переключает содержимое окна настроек.
pub fn settings_tab_click(
    mut commands: Commands,
    mut settings: ResMut<Settings>,
    root: Query<Entity, With<SettingsMenu>>,
    clicks: TabClicks,
) {
    for (interaction, tab) in clicks.iter() {
        if *interaction != Interaction::Pressed || settings.tab == tab.0 {
            continue;
        }
        settings.tab = tab.0;
        // Пересобираем окно с новой вкладкой.
        for entity in root.iter() {
            commands.entity(entity).despawn();
        }
        spawn_menu(&mut commands, &settings);
        tracing::info!(tab = ?tab.0, "settings tab changed");
    }
}

/// Одна строка настройки: подпись, значение и кнопки изменения.
fn settings_row(
    panel: &mut RelatedSpawnerCommands<'_, ChildOf>,
    label: &str,
    value: &str,
    marker: SettingsValue,
    left: SettingsAction,
    right: SettingsAction,
) {
    panel
        .spawn((
            Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: px(8),
                padding: UiRect::axes(px(6), px(2)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.12, 0.12, 0.15, 0.7)),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(label.to_string()),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.88, 0.88, 0.90)),
                Node {
                    width: px(180),
                    ..default()
                },
            ));
            for action in [left, right] {
                row.spawn((
                    action,
                    Button,
                    Node {
                        width: px(34),
                        height: px(28),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(px(2)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.13, 0.13, 0.16)),
                    BorderColor::from(Color::srgb(0.30, 0.30, 0.35)),
                ))
                .with_child((
                    Text::new(if action == left { "−" } else { "+" }),
                    TextFont::from_font_size(15.0),
                    TextColor(Color::srgb(0.9, 0.9, 0.92)),
                ));
            }
            row.spawn((
                marker,
                Text::new(value.to_string()),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(1.0, 0.75, 0.25)),
            ));
        });
}

/// Клик по кнопке настроек.
type SettingsClicks<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static SettingsAction),
    (Changed<Interaction>, With<Button>),
>;

/// Обрабатывает клики по кнопкам настроек.
pub fn settings_click(
    mut commands: Commands,
    mut settings: ResMut<Settings>,
    mut windows: Query<&mut Window>,
    root: Query<Entity, With<SettingsMenu>>,
    clicks: SettingsClicks,
) {
    let mut save = false;
    for (interaction, action) in clicks.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        save = true;
        match action {
            SettingsAction::ToggleFullscreen => {
                settings.fullscreen = !settings.fullscreen;
                if let Ok(mut window) = windows.single_mut() {
                    window.mode = if settings.fullscreen {
                        WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
                    } else {
                        WindowMode::Windowed
                    };
                }
            }
            SettingsAction::ToggleFps => settings.show_fps = !settings.show_fps,
            SettingsAction::ToggleCombatIndicators => {
                settings.combat_indicators = !settings.combat_indicators;
            }
            SettingsAction::ToggleFootWalk => {
                settings.foot_walk_animation = !settings.foot_walk_animation;
            }
            SettingsAction::Close => {
                for entity in root.iter() {
                    commands.entity(entity).despawn();
                }
                save = false;
            }
        }
    }
    if save {
        settings.save();
    }
}

/// Обновляет тексты значений при изменении настроек.
pub fn update_settings_text(
    settings: Res<Settings>,
    mut texts: Query<(&mut Text, &SettingsValue)>,
) {
    if !settings.is_changed() {
        return;
    }
    for (mut text, marker) in texts.iter_mut() {
        let value = match marker {
            SettingsValue::Volume => format!("{}%", (settings.volume * 100.0).round()),
            SettingsValue::Fullscreen => (if settings.fullscreen {
                "Вкл"
            } else {
                "Выкл"
            })
            .into(),
            SettingsValue::Fps => (if settings.show_fps {
                "Вкл"
            } else {
                "Выкл"
            })
            .into(),
            SettingsValue::CombatIndicators => (if settings.combat_indicators {
                "Вкл"
            } else {
                "Выкл"
            })
            .into(),
            SettingsValue::FootWalk => (if settings.foot_walk_animation {
                "Вкл"
            } else {
                "Выкл"
            })
            .into(),
        };
        if text.0 != value {
            text.0 = value;
        }
    }
}

/// Показывает FPS, когда включено в настройках.
pub fn update_fps(
    settings: Res<Settings>,
    diagnostics: Res<DiagnosticsStore>,
    mut nodes: Query<&mut Node, With<FpsText>>,
    mut texts: Query<&mut Text, With<FpsText>>,
) {
    let Ok(mut node) = nodes.single_mut() else {
        return;
    };
    node.display = if settings.show_fps {
        Display::Flex
    } else {
        Display::None
    };
    if !settings.show_fps {
        return;
    }
    let fps = diagnostics
        .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed());
    if let Some(fps) = fps
        && let Ok(mut text) = texts.single_mut()
    {
        let value = format!("FPS {fps:.0}");
        if text.0 != value {
            text.0 = value;
        }
    }
}
