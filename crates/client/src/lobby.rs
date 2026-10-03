//! Лобби по образцу мини-станции (`LobbyGui.xaml` + их `AnimatedBackgroundControl`):
//! полноэкранный анимированный фон (`_Mini/Lobby/mars.rsi`, 64 кадра), верхняя
//! панель-«страйп» с кнопками «Правила»/«Вики» и логотипом, внизу слева —
//! информация о сервере, по центру — статус смены и кнопки «Готов»/«Выход».
//! Показывается ТОЛЬКО в релизной сборке (в dev сразу игра; SSR_LOBBY=1 — отладка).

use bevy::prelude::*;
use bevy::ui::widget::Button;

use crate::rsi::RsiRegistry;

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

/// Фон лобби: анимированный ImageNode.
#[derive(Component)]
pub struct LobbyBackground;

/// Анимация фона (кадр + таймер по delays RSI).
#[derive(Default)]
pub struct BackgroundAnim {
    frame: u32,
    elapsed: f32,
}

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

/// Анимированный фон — RSI из сборки мини-станции (см. ASSETS_LICENSES.md).
const LOBBY_BACKGROUND_RSI: &str = "sprites/ss14/_Mini/Lobby/mars.rsi#1";
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
pub fn spawn_lobby(mut commands: Commands, assets: Res<AssetServer>, registry: Res<RsiRegistry>) {
    let logo = assets.load(LOBBY_LOGO);

    // Фон: первый кадр mars.rsi, дальше система крутит анимацию.
    let background = registry.get(LOBBY_BACKGROUND_RSI).map(|rsi| {
        let mut node = ImageNode::new(rsi.image.clone());
        node.texture_atlas = Some(TextureAtlas {
            layout: rsi.layout.clone(),
            index: rsi.index(0, 0),
        });
        node
    });

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
            // Полноэкранный анимированный фон (аналог AnimatedBackgroundControl).
            if let Some(mut node) = background {
                node.image_mode = NodeImageMode::Stretch;
                root.spawn((
                    LobbyBackground,
                    node,
                    Node {
                        width: percent(100),
                        height: percent(100),
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                ));
            }

            // Верхняя панель-«страйп»: кнопки и логотип справа (TopPanel из XAML).
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    right: px(0),
                    top: px(0),
                    height: px(56),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    padding: UiRect::horizontal(px(12)),
                    column_gap: px(8),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
            ))
            .with_children(|top| {
                top_button(top, "Правила", LobbyAction::Url("https://ministation.ru"));
                top_button(top, "Вики", LobbyAction::Url("https://wiki.ministation.ru"));
                // Пружина: логотип уходит вправо.
                top.spawn(Node {
                    flex_grow: 1.0,
                    ..default()
                });
                top.spawn((
                    ImageNode::new(logo),
                    Node {
                        width: px(40),
                        height: px(40),
                        ..default()
                    },
                ));
            });

            // Низ слева: ServerInfo (название сервера, адрес, режим) — как левая панель XAML.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(12),
                    bottom: px(12),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(px(12)),
                    row_gap: px(4),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
            ))
            .with_children(|info| {
                info.spawn((
                    Text::new("Мини-станция · Space Station R"),
                    TextFont::from_font_size(18.0),
                    TextColor(ACCENT),
                ));
                info.spawn((
                    Text::new("Сервер: 127.0.0.1:7777"),
                    TextFont::from_font_size(14.0),
                    TextColor(TEXT),
                ));
                info.spawn((
                    Text::new("Режим: песочница · сборка на Rust"),
                    TextFont::from_font_size(13.0),
                    TextColor(TEXT_DIM),
                ));
            });

            // Центральная колонка: статус смены, «Готов», «Выход» (CenterPanel из XAML).
            root.spawn(Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                top: px(56),
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
                            margin: UiRect::bottom(px(10)),
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

/// Кнопка верхней панели (125×36, как кнопки Rules/Guidebook в XAML).
fn top_button(parent: &mut ChildSpawnerCommands, label: &str, action: LobbyAction) {
    parent
        .spawn((
            Button,
            action,
            Node {
                width: px(125),
                height: px(36),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(BUTTON_BG),
            BorderColor::from(BUTTON_BORDER),
        ))
        .with_child((
            Text::new(label),
            TextFont::from_font_size(16.0),
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

/// Крутит кадры анимированного фона по `delays` из RSI (аналог
/// AnimatedBackgroundControl.FrameUpdate в сборке мини-станции).
pub fn animate_lobby_background(
    time: Res<Time>,
    registry: Res<RsiRegistry>,
    mut anim: Local<BackgroundAnim>,
    mut backgrounds: Query<&mut ImageNode, With<LobbyBackground>>,
) {
    let Some(rsi) = registry.get(LOBBY_BACKGROUND_RSI) else {
        return;
    };
    let frames = rsi.frames_per_direction.first().copied().unwrap_or(1);
    if frames <= 1 {
        return;
    }
    let delays = rsi.delays.first().cloned().unwrap_or_default();

    anim.elapsed += time.delta_secs();
    let current = (anim.frame % frames) as usize;
    let delay = delays.get(current).copied().unwrap_or(0.4).max(0.001);
    if anim.elapsed < delay {
        return;
    }
    anim.elapsed -= delay;
    anim.frame = (anim.frame + 1) % frames;

    for mut node in backgrounds.iter_mut() {
        if let Some(atlas) = node.texture_atlas.as_mut() {
            atlas.index = rsi.index(0, anim.frame);
        }
    }
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
