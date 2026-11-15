//! Настройки клиента: меню по Esc, громкость, полный экран, показ FPS.
//! Сохраняются в `~/.ssr-settings.ron` и переживают перезапуск.

use bevy::audio::{GlobalVolume, Volume};
use bevy::diagnostic::DiagnosticsStore;
use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowMode};
use serde::{Deserialize, Serialize};

/// Настройки игрока.
#[derive(Resource, Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// LOOC над головами (`hud.show_looc_above_head`, default true).
    pub show_looc_overhead: bool,
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
            show_looc_overhead: true,
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

/// Переключаемая настройка (галочка).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Opt {
    Fullscreen,
    Fps,
    CombatIndicators,
    FootWalk,
    LoocOverhead,
}

impl Opt {
    fn get(self, s: &Settings) -> bool {
        match self {
            Opt::Fullscreen => s.fullscreen,
            Opt::Fps => s.show_fps,
            Opt::CombatIndicators => s.combat_indicators,
            Opt::FootWalk => s.foot_walk_animation,
            Opt::LoocOverhead => s.show_looc_overhead,
        }
    }

    fn flip(self, s: &mut Settings) {
        match self {
            Opt::Fullscreen => s.fullscreen = !s.fullscreen,
            Opt::Fps => s.show_fps = !s.show_fps,
            Opt::CombatIndicators => s.combat_indicators = !s.combat_indicators,
            Opt::FootWalk => s.foot_walk_animation = !s.foot_walk_animation,
            Opt::LoocOverhead => s.show_looc_overhead = !s.show_looc_overhead,
        }
    }
}

/// Кнопка окна настроек (`OptionsMenu.xaml`: вкладки, галочки и нижняя
/// панель «Сброс к настройкам по умолчанию / Сброс изменений / Сохранить и
/// применить»).
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum SettingsAction {
    Toggle(Opt),
    Close,
    ResetDefaults,
    ResetChanges,
    Apply,
}

/// Черновик настроек: правки окна копятся здесь и применяются кнопкой
/// «Сохранить и применить» (как `OptionsMenu` сборки).
#[derive(Resource, Default, Clone)]
pub struct SettingsDraft(pub Settings);

/// Текст значения громкости (обновляется при протяжке ползунка).
#[derive(Component)]
pub struct VolumeValue;

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
            // 2 — «Графика», 3 — «Управление», 4 — «Основные», 5 — «Аудио»,
            // 6 — «Доступность».
            settings.tab = match mode {
                3 => SettingsTab::Controls,
                4 => SettingsTab::Misc,
                5 => SettingsTab::Audio,
                6 => SettingsTab::Accessibility,
                _ => SettingsTab::Graphics,
            };
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

// Цвета окна настроек SS14 (тёмная тема `StyleNano`).
const WIN_BG: Color = Color::srgba(0.13, 0.13, 0.15, 0.97);
const WIN_TITLE: Color = Color::srgb(0.09, 0.09, 0.105);
const WIN_BORDER: Color = Color::srgb(0.26, 0.26, 0.30);
const TAB_STRIP: Color = Color::srgb(0.10, 0.10, 0.115);
const TAB_ACTIVE: Color = Color::srgb(0.29, 0.31, 0.37);
const SECTION: Color = Color::srgb(0.80, 0.66, 0.36);
const OPT_TEXT: Color = Color::srgb(0.88, 0.88, 0.90);
const OPT_DIM: Color = Color::srgb(0.58, 0.58, 0.63);
const BOX_BG: Color = Color::srgb(0.08, 0.08, 0.095);
const BOX_ON: Color = Color::srgb(0.62, 0.65, 0.74);
const BTN: Color = Color::srgb(0.20, 0.21, 0.25);
const BTN_PRIMARY: Color = Color::srgb(0.27, 0.36, 0.30);

/// Открывает окно настроек заново: черновик — копия текущих настроек.
fn spawn_menu(commands: &mut Commands, settings: &Settings) {
    commands.insert_resource(SettingsDraft(settings.clone()));
    build_menu(commands, settings, false);
}

/// Пересобирает окно по черновику (`dirty` — есть несохранённые правки).
fn build_menu(commands: &mut Commands, draft: &Settings, dirty: bool) {
    commands
        .spawn((
            SettingsMenu,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                width: px(720),
                height: px(500),
                margin: UiRect::new(px(-360), px(0), px(-250), px(0)),
                flex_direction: FlexDirection::Column,
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(WIN_BG),
            BorderColor::from(WIN_BORDER),
            GlobalZIndex(50),
        ))
        .with_children(|panel| {
            // Заголовок окна и крестик (`DefaultWindow`).
            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: px(30),
                        flex_shrink: 0.0,
                        align_items: AlignItems::Center,
                        padding: UiRect::horizontal(px(8)),
                        ..default()
                    },
                    BackgroundColor(WIN_TITLE),
                ))
                .with_children(|title| {
                    title.spawn((
                        Text::new("Игровые настройки"),
                        TextFont::from_font_size(16.0),
                        TextColor(OPT_TEXT),
                    ));
                    title.spawn(Node {
                        flex_grow: 1.0,
                        ..default()
                    });
                    title
                        .spawn((
                            SettingsAction::Close,
                            Button,
                            Node {
                                width: px(24),
                                height: px(24),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                        ))
                        .with_child((
                            Text::new("×"),
                            TextFont::from_font_size(20.0),
                            TextColor(OPT_DIM),
                        ));
                });
            tabs_row(panel, draft.tab);
            // Содержимое вкладки.
            panel
                .spawn(Node {
                    width: Val::Percent(100.0),
                    flex_grow: 1.0,
                    // Без min_height 0 длинный список (Управление) выдавливал
                    // нижнюю панель за край окна.
                    min_height: px(0),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    padding: UiRect::axes(px(12), px(8)),
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|page| match draft.tab {
                    SettingsTab::Misc => {
                        section(page, "Речь");
                        checkbox(page, draft, Opt::LoocOverhead, "Показывать LOOC-чат над головами персонажей");
                        section(page, "Курсор");
                        checkbox(page, draft, Opt::CombatIndicators, "Показать индикатор боевого режима рядом с курсором");
                    }
                    SettingsTab::Graphics => {
                        section(page, "Окно");
                        checkbox(page, draft, Opt::Fullscreen, "Полноэкранный режим");
                        section(page, "Интерфейс");
                        checkbox(page, draft, Opt::Fps, "Показывать счётчик FPS");
                    }
                    SettingsTab::Audio => {
                        section(page, "Громкость");
                        volume_slider(page, draft.volume);
                    }
                    SettingsTab::Controls => {
                        // Список привязок наполняет `keybinds::render_keybinds_panel`
                        // (`KeyRebindTab` в сборке); привязки применяются сразу.
                        page.spawn((
                            crate::keybinds::KeybindsPanel,
                            Node {
                                flex_direction: FlexDirection::Column,
                                row_gap: px(2),
                                ..default()
                            },
                        ));
                    }
                    SettingsTab::Accessibility => {
                        section(page, "Движение");
                        checkbox(page, draft, Opt::FootWalk, "Процедурная анимация шага");
                    }
                });
            // Нижняя панель.
            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: px(40),
                        flex_shrink: 0.0,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        padding: UiRect::horizontal(px(10)),
                        border: UiRect::top(px(1)),
                        ..default()
                    },
                    BorderColor::from(WIN_BORDER),
                ))
                .with_children(|bar| {
                    bottom_button(bar, "Сброс к настройкам по умолчанию", SettingsAction::ResetDefaults, BTN);
                    bar.spawn(Node {
                        flex_grow: 1.0,
                        ..default()
                    });
                    bottom_button(bar, "Сброс изменений", SettingsAction::ResetChanges, BTN);
                    bottom_button(
                        bar,
                        "Сохранить и применить",
                        SettingsAction::Apply,
                        if dirty { BTN_PRIMARY } else { BTN },
                    );
                });
        });
}

/// Золотой заголовок раздела.
fn section(page: &mut RelatedSpawnerCommands<'_, ChildOf>, title: &str) {
    page.spawn((
        Text::new(title.to_string()),
        TextFont::from_font_size(15.0),
        TextColor(SECTION),
        Node {
            margin: UiRect::top(px(4)),
            ..default()
        },
    ));
}

/// Строка-галочка (`CheckBox` в OptionsMenu).
fn checkbox(page: &mut RelatedSpawnerCommands<'_, ChildOf>, draft: &Settings, opt: Opt, label: &str) {
    let on = opt.get(draft);
    page.spawn((
        SettingsAction::Toggle(opt),
        Button,
        Node {
            align_items: AlignItems::Center,
            column_gap: px(8),
            height: px(24),
            ..default()
        },
        BackgroundColor(Color::NONE),
    ))
    .with_children(|row| {
        row.spawn((
            Node {
                width: px(16),
                height: px(16),
                border: UiRect::all(px(1)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BOX_BG),
            BorderColor::from(Color::srgb(0.40, 0.40, 0.46)),
        ))
        .with_children(|b| {
            if on {
                b.spawn((
                    Node {
                        width: px(10),
                        height: px(10),
                        ..default()
                    },
                    BackgroundColor(BOX_ON),
                ));
            }
        });
        row.spawn((
            Text::new(label.to_string()),
            TextFont::from_font_size(14.0),
            TextColor(OPT_TEXT),
        ));
    });
}

fn bottom_button(bar: &mut RelatedSpawnerCommands<'_, ChildOf>, label: &str, action: SettingsAction, bg: Color) {
    bar.spawn((
        action,
        Button,
        Node {
            height: px(28),
            padding: UiRect::horizontal(px(12)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
        BackgroundColor(bg),
    ))
    .with_child((
        Text::new(label.to_string()),
        TextFont::from_font_size(14.0),
        TextColor(OPT_TEXT),
    ));
}

/// Ряд вкладок окна настроек: порядок и подписи как в `OptionsMenu.xaml`,
/// плоские вкладки, активная — светлее.
fn tabs_row(panel: &mut RelatedSpawnerCommands<'_, ChildOf>, active: SettingsTab) {
    panel
        .spawn((
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                padding: UiRect::new(px(8), px(8), px(6), px(0)),
                column_gap: px(2),
                flex_shrink: 0.0,
                ..default()
            },
            BackgroundColor(TAB_STRIP),
        ))
        .with_children(|tabs| {
            for tab in SettingsTab::ALL {
                let selected = tab == active;
                tabs.spawn((
                    TabButton(tab),
                    Button,
                    Node {
                        height: px(26),
                        padding: UiRect::horizontal(px(8)),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(if selected { TAB_ACTIVE } else { Color::NONE }),
                ))
                .with_child((
                    Text::new(tab.title()),
                    TextFont::from_font_size(14.0),
                    TextColor(if selected { OPT_TEXT } else { OPT_DIM }),
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
        ))
        .with_children(|row| {
            row.spawn(Node {
                width: px(180),
                align_items: AlignItems::Center,
                ..default()
            })
            .with_child((
                Text::new("Общая громкость"),
                TextFont::from_font_size(14.0),
                TextColor(OPT_TEXT),
            ));
            row.spawn((
                VolumeSlider,
                Button,
                Node {
                    width: px(260),
                    height: px(14),
                    border: UiRect::all(px(2)),
                    ..default()
                },
                BackgroundColor(BOX_BG),
                BorderColor::from(Color::srgb(0.40, 0.40, 0.46)),
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
                BackgroundColor(BOX_ON),
            ));
            row.spawn((
                VolumeValue,
                Text::new(format!("{}%", (volume * 100.0).round())),
                TextFont::from_font_size(14.0),
                TextColor(OPT_TEXT),
            ));
        });
}

/// Клик или протяжка по дорожке задают громкость и двигают ручку.
pub fn volume_slider_drag(
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut draft: ResMut<SettingsDraft>,
    sliders: Query<(&Interaction, &ComputedNode, &GlobalTransform), With<VolumeSlider>>,
    mut knobs: Query<&mut Node, With<VolumeKnob>>,
    mut values: Query<&mut Text, With<VolumeValue>>,
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
        if (fraction - draft.0.volume).abs() < 0.001 {
            continue;
        }
        // Громкость — в черновик: применится «Сохранить и применить».
        draft.0.volume = fraction;
        for mut knob in knobs.iter_mut() {
            knob.left = Val::Percent(fraction * 100.0);
        }
        for mut text in values.iter_mut() {
            text.0 = format!("{}%", (fraction * 100.0).round());
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
    mut draft: ResMut<SettingsDraft>,
    root: Query<Entity, With<SettingsMenu>>,
    clicks: TabClicks,
) {
    for (interaction, tab) in clicks.iter() {
        if *interaction != Interaction::Pressed || draft.0.tab == tab.0 {
            continue;
        }
        // Вкладка не считается правкой: меняем её и в настройках, и в черновике.
        settings.bypass_change_detection().tab = tab.0;
        draft.0.tab = tab.0;
        for entity in root.iter() {
            commands.entity(entity).despawn();
        }
        let dirty = draft.0 != *settings;
        build_menu(&mut commands, &draft.0, dirty);
        tracing::info!(tab = ?tab.0, "settings tab changed");
    }
}

/// Клик по кнопке настроек.
type SettingsClicks<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static SettingsAction),
    (Changed<Interaction>, With<Button>),
>;

/// Клики окна настроек: галочки правят черновик, «Сохранить и применить»
/// переносит его в настройки (с полноэкранным режимом и сохранением),
/// «Сброс изменений» — назад к текущим, «Сброс к настройкам по умолчанию» —
/// значения по умолчанию (имя и серверы не трогаем).
pub fn settings_click(
    mut commands: Commands,
    mut settings: ResMut<Settings>,
    mut draft: ResMut<SettingsDraft>,
    mut windows: Query<&mut Window>,
    root: Query<Entity, With<SettingsMenu>>,
    clicks: SettingsClicks,
) {
    let mut rebuild = false;
    for (interaction, action) in clicks.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            SettingsAction::Toggle(opt) => opt.flip(&mut draft.0),
            SettingsAction::ResetChanges => draft.0 = settings.clone(),
            SettingsAction::ResetDefaults => {
                let keep = draft.0.clone();
                draft.0 = Settings {
                    player_name: keep.player_name,
                    server: keep.server,
                    servers: keep.servers,
                    tab: keep.tab,
                    ..Settings::default()
                };
            }
            SettingsAction::Apply => {
                if draft.0.fullscreen != settings.fullscreen
                    && let Ok(mut window) = windows.single_mut()
                {
                    window.mode = if draft.0.fullscreen {
                        WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
                    } else {
                        WindowMode::Windowed
                    };
                }
                *settings = draft.0.clone();
                settings.save();
                tracing::info!("settings applied");
            }
            SettingsAction::Close => {
                for entity in root.iter() {
                    commands.entity(entity).despawn();
                }
                return;
            }
        }
        rebuild = true;
    }
    if rebuild {
        for entity in root.iter() {
            commands.entity(entity).despawn();
        }
        let dirty = draft.0 != *settings;
        build_menu(&mut commands, &draft.0, dirty);
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
