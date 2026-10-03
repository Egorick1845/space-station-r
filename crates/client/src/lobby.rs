//! Лобби в духе сборки «Мини-станции» (их `LobbyGui.xaml`): слева — панель
//! с логотипом и кнопками «Сайт»/«Вики», в центре — строка статуса, кнопка
//! «Готов» (подключение к серверу) и «Выход». Показывается ТОЛЬКО в релизной
//! сборке — в dev сразу игра (принудительно включить: SSR_LOBBY=1).

use bevy::prelude::*;
use bevy::ui::widget::Button;

/// Состояние клиента: лобби или игра.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub enum LobbyState {
    #[default]
    Lobby,
    Playing,
}

/// Корень UI лобби (удаляется при входе в игру).
#[derive(Component)]
pub struct LobbyRoot;

/// Действие кнопки лобби.
#[derive(Component, Clone, Copy, PartialEq)]
pub enum LobbyAction {
    /// Подключиться к серверу и начать игру.
    Ready,
    /// Открыть ссылку во внешнем браузере.
    Url(&'static str),
    /// Закрыть игру.
    Quit,
}

/// Фон лобби — арт станции из сборки мини-станции (см. ASSETS_LICENSES.md).
const LOBBY_BACKGROUND: &str = "sprites/ss14/LobbyScreens/SpaceStation64.webp";
/// Логотип мини-станции (их `Textures/Interface/main_logo.png`).
const LOBBY_LOGO: &str = "sprites/ss14/Interface/main_logo.png";

/// Цвета в стиле SS14: тёмные панели и кнопки с рамкой.
const PANEL_BG: Color = Color::srgba(0.06, 0.06, 0.08, 0.72);
const BUTTON_BG: Color = Color::srgb(0.13, 0.13, 0.16);
const BUTTON_BG_HOVER: Color = Color::srgb(0.20, 0.20, 0.25);
const BUTTON_BORDER: Color = Color::srgb(0.30, 0.30, 0.35);
const TEXT: Color = Color::srgb(0.88, 0.88, 0.90);
const TEXT_DIM: Color = Color::srgb(0.62, 0.62, 0.66);
const ACCENT: Color = Color::srgb(1.0, 0.62, 0.15);
const DANGER_BG: Color = Color::srgb(0.42, 0.13, 0.13);
const DANGER_BG_HOVER: Color = Color::srgb(0.55, 0.17, 0.17);

/// Строит стартовый экран по образцу лобби мини-станции.
pub fn spawn_lobby(mut commands: Commands, assets: Res<AssetServer>) {
    let background = assets.load(LOBBY_BACKGROUND);
    let logo = assets.load(LOBBY_LOGO);

    commands
        .spawn((
            LobbyRoot,
            Node {
                width: percent(100),
                height: percent(100),
                position_type: PositionType::Absolute,
                ..default()
            },
        ))
        .with_children(|root| {
            // Фон: арт станции.
            root.spawn((
                ImageNode::new(background),
                Node {
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
            ));

            // Левая панель: логотип, информация о сервере и кнопки —
            // как левая панель лобби мини-станции (main_logo + Сайт/Дискорд/Телеграм).
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    bottom: px(0),
                    width: px(300),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::all(px(18)),
                    row_gap: px(14),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
            ))
            .with_children(|panel| {
                panel.spawn((
                    ImageNode::new(logo),
                    Node {
                        width: px(220),
                        height: px(220),
                        ..default()
                    },
                ));
                panel.spawn((
                    Text::new("SPACE STATION R"),
                    TextFont::from_font_size(22.0),
                    TextColor(ACCENT),
                ));
                panel.spawn((
                    Text::new("Мини-станция · 127.0.0.1:7777"),
                    TextFont::from_font_size(13.0),
                    TextColor(TEXT_DIM),
                ));
                panel.spawn((
                    Node {
                        width: percent(100),
                        height: px(1),
                        margin: UiRect::vertical(px(6)),
                        ..default()
                    },
                    BackgroundColor(BUTTON_BORDER),
                ));
                menu_button(
                    panel,
                    "Сайт",
                    LobbyAction::Url("https://ministation.ru"),
                    false,
                );
                menu_button(
                    panel,
                    "Вики",
                    LobbyAction::Url("https://wiki.ministation.ru"),
                    false,
                );
            });

            // Центральная колонка: строка статуса, «Готов», «Выход».
            root.spawn(Node {
                position_type: PositionType::Absolute,
                left: px(300),
                right: px(0),
                top: px(0),
                bottom: px(0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(10),
                ..default()
            })
            .with_children(|center| {
                center
                    .spawn((
                        Node {
                            padding: UiRect::axes(px(18), px(6)),
                            margin: UiRect::bottom(px(8)),
                            ..default()
                        },
                        BackgroundColor(PANEL_BG),
                    ))
                    .with_child((
                        Text::new("Смена ещё не началась"),
                        TextFont::from_font_size(22.0),
                        TextColor(TEXT),
                    ));
                wide_button(center, "Готов", LobbyAction::Ready, false);
                wide_button(center, "Выход", LobbyAction::Quit, true);
            });
        });
}

/// Кнопка шириной панели (левая колонка).
fn menu_button(parent: &mut ChildSpawnerCommands, label: &str, action: LobbyAction, danger: bool) {
    parent
        .spawn((
            Button,
            action,
            Node {
                width: percent(100),
                height: px(40),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(if danger { DANGER_BG } else { BUTTON_BG }),
            BorderColor::from(BUTTON_BORDER),
        ))
        .with_child((
            Text::new(label),
            TextFont::from_font_size(18.0),
            TextColor(TEXT),
        ));
}

/// Широкая кнопка центральной колонки (280×48, как ReadyButton в SS14).
fn wide_button(parent: &mut ChildSpawnerCommands, label: &str, action: LobbyAction, danger: bool) {
    parent
        .spawn((
            Button,
            action,
            Node {
                width: px(280),
                height: px(48),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(if danger { DANGER_BG } else { BUTTON_BG }),
            BorderColor::from(BUTTON_BORDER),
        ))
        .with_child((
            Text::new(label),
            TextFont::from_font_size(22.0),
            TextColor(TEXT),
        ));
}

/// Запрос кнопок лобби (алиас против clippy::type_complexity).
type LobbyButtonQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        &'static LobbyAction,
        &'static mut BackgroundColor,
    ),
    (Changed<Interaction>, With<Button>),
>;

/// Подсветка кнопок и действия: «Готов» — в игру, ссылки — в браузер, «Выход» — закрыть.
pub fn lobby_button(
    mut interactions: LobbyButtonQuery,
    mut state: ResMut<LobbyState>,
    mut commands: Commands,
    roots: Query<Entity, With<LobbyRoot>>,
    mut app_exit: MessageWriter<AppExit>,
) {
    for (interaction, action, mut color) in interactions.iter_mut() {
        let danger = matches!(action, LobbyAction::Quit);
        match interaction {
            Interaction::Pressed => match action {
                LobbyAction::Ready => {
                    if *state == LobbyState::Playing {
                        continue;
                    }
                    *state = LobbyState::Playing;
                    for root in roots.iter() {
                        commands.entity(root).despawn();
                    }
                    tracing::info!("lobby: готов — подключение к серверу");
                }
                LobbyAction::Url(url) => open_url(url),
                LobbyAction::Quit => {
                    app_exit.write(AppExit::Success);
                }
            },
            Interaction::Hovered => {
                *color = BackgroundColor(if danger {
                    DANGER_BG_HOVER
                } else {
                    BUTTON_BG_HOVER
                });
            }
            Interaction::None => {
                *color = BackgroundColor(if danger { DANGER_BG } else { BUTTON_BG });
            }
        }
    }
}

/// Открывает ссылку в системном браузере (как кнопки «Сайт/Дискорд/Телеграм»
/// в лобби мини-станции — они ведут наружу).
fn open_url(url: &str) {
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    match result {
        Ok(_) => tracing::info!(url, "lobby: открываю ссылку"),
        Err(e) => tracing::warn!(url, error = %e, "lobby: не удалось открыть ссылку"),
    }
}
