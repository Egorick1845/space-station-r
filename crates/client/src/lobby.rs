//! Лобби (стартовый экран) в духе сборки «Мини-станции»: фон-арт, название,
//! кнопка «Играть». Подключение к серверу происходит после нажатия.

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

/// Маркер кнопки «Играть».
#[derive(Component)]
pub struct PlayButton;

/// Фон лобби из сборки мини-станции (см. ASSETS_LICENSES.md).
const LOBBY_BACKGROUND: &str = "sprites/ss14/LobbyScreens/SpaceStation64.webp";

/// Строит стартовый экран.
pub fn spawn_lobby(mut commands: Commands, assets: Res<AssetServer>) {
    let background = assets.load(LOBBY_BACKGROUND);
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
            // Фон: арт станции 64.
            root.spawn((
                ImageNode::new(background),
                Node {
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
            ));
            // Затемнение поверх арта, чтобы текст читался.
            root.spawn((
                BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.45)),
                Node {
                    width: percent(100),
                    height: percent(100),
                    position_type: PositionType::Absolute,
                    ..default()
                },
            ));
            // Контент по центру.
            root.spawn(Node {
                width: percent(100),
                height: percent(100),
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: px(18),
                ..default()
            })
            .with_children(|content| {
                content.spawn((
                    Text::new("SPACE STATION R"),
                    TextFont::from_font_size(64.0),
                    TextColor(Color::srgb(1.0, 0.62, 0.15)),
                ));
                content.spawn((
                    Text::new("мини-станция на Rust · Bevy"),
                    TextFont::from_font_size(22.0),
                    TextColor(Color::srgb(0.82, 0.82, 0.86)),
                ));
                content
                    .spawn((
                        Button,
                        PlayButton,
                        Node {
                            padding: UiRect::axes(px(56), px(18)),
                            margin: UiRect::top(px(14)),
                            border: UiRect::all(px(2)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.92, 0.45, 0.08)),
                        BorderColor::from(Color::srgb(1.0, 0.75, 0.25)),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new("Играть"),
                            TextFont::from_font_size(30.0),
                            TextColor(Color::srgb(0.06, 0.05, 0.04)),
                        ));
                    });
                content.spawn((
                    Text::new("Сервер: 127.0.0.1:7777"),
                    TextFont::from_font_size(15.0),
                    TextColor(Color::srgb(0.62, 0.62, 0.66)),
                ));
            });
        });
}

/// Запрос кнопки «Играть» (алиас, чтобы не ловить clippy::type_complexity).
type PlayButtonQuery<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static mut BackgroundColor),
    (Changed<Interaction>, With<PlayButton>),
>;

/// Подсветка кнопки и переход в игру по нажатию.
pub fn lobby_button(
    mut interactions: PlayButtonQuery,
    mut state: ResMut<LobbyState>,
    mut commands: Commands,
    roots: Query<Entity, With<LobbyRoot>>,
) {
    for (interaction, mut color) in interactions.iter_mut() {
        match interaction {
            Interaction::Pressed => {
                if *state == LobbyState::Playing {
                    continue;
                }
                *state = LobbyState::Playing;
                for root in roots.iter() {
                    commands.entity(root).despawn();
                }
                tracing::info!("lobby: играть — подключение к серверу");
            }
            Interaction::Hovered => *color = BackgroundColor(Color::srgb(1.0, 0.6, 0.2)),
            Interaction::None => *color = BackgroundColor(Color::srgb(0.92, 0.45, 0.08)),
        }
    }
}
