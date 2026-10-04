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

/// Кнопка меню настроек.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    VolumeDown,
    VolumeUp,
    ToggleFullscreen,
    ToggleFps,
    Close,
}

/// Текст значения настройки (обновляется при изменении).
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum SettingsValue {
    Volume,
    Fullscreen,
    Fps,
}

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

/// Применяет громкость к общему миксу.
pub fn apply_volume(settings: Res<Settings>, mut volume: ResMut<GlobalVolume>) {
    if settings.is_changed() {
        volume.volume = Volume::Linear(settings.volume.clamp(0.0, 1.0));
    }
}

/// Esc открывает и закрывает меню настроек (когда консоль закрыта).
pub fn toggle_settings_menu(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<crate::console::Console>,
    settings: Res<Settings>,
    root: Query<Entity, With<SettingsMenu>>,
) {
    if !keys.just_pressed(KeyCode::Escape) || console.open {
        return;
    }
    let existing: Vec<Entity> = root.iter().collect();
    for &entity in &existing {
        commands.entity(entity).despawn();
    }
    if existing.is_empty() {
        spawn_menu(&mut commands, &settings);
    }
}

/// Строит окно настроек по центру экрана.
fn spawn_menu(commands: &mut Commands, settings: &Settings) {
    let fullscreen = if settings.fullscreen {
        "Вкл"
    } else {
        "Выкл"
    };
    let fps = if settings.show_fps {
        "Вкл"
    } else {
        "Выкл"
    };
    commands
        .spawn((
            SettingsMenu,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(35.0),
                top: Val::Percent(18.0),
                width: px(420),
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                padding: UiRect::all(px(14)),
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.95)),
            BorderColor::from(Color::srgb(0.30, 0.30, 0.36)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Настройки"),
                TextFont::from_font_size(18.0),
                TextColor(Color::srgb(1.0, 0.75, 0.25)),
            ));
            settings_row(
                panel,
                "Громкость",
                &format!("{}%", (settings.volume * 100.0).round()),
                SettingsValue::Volume,
                SettingsAction::VolumeDown,
                SettingsAction::VolumeUp,
            );
            settings_row(
                panel,
                "Полный экран",
                fullscreen,
                SettingsValue::Fullscreen,
                SettingsAction::ToggleFullscreen,
                SettingsAction::ToggleFullscreen,
            );
            settings_row(
                panel,
                "Показывать FPS",
                fps,
                SettingsValue::Fps,
                SettingsAction::ToggleFps,
                SettingsAction::ToggleFps,
            );
            panel.spawn((
                Text::new(
                    "Управление:\n  WASD — движение\n  ЛКМ — использовать/атака, Ctrl+ЛКМ — передать\n  \
                     ПКМ — действия по объекту\n  X — сменить руку\n  ` — консоль\n  Esc — это меню",
                ),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgb(0.75, 0.75, 0.80)),
            ));
            panel
                .spawn((
                    SettingsAction::Close,
                    Button,
                    Node {
                        height: px(34),
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
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(8),
            ..default()
        })
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
            SettingsAction::VolumeDown => settings.volume = (settings.volume - 0.1).max(0.0),
            SettingsAction::VolumeUp => settings.volume = (settings.volume + 0.1).min(1.0),
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
