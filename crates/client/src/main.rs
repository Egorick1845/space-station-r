//! Bevy-клиент Space Station R.
//!
//! T0.2: окно 1280×720, тёмный фон, спрайт в центре, движение WASD/стрелки.

use bevy::prelude::*;
use bevy::window::{Window, WindowPlugin, WindowResolution};

use ssr_core::GAME_NAME;

/// Скорость тестового персонажа, пикселей в секунду.
const MOVE_SPEED: f32 = 300.0;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: format!("{GAME_NAME} — dev client"),
                resolution: WindowResolution::new(1280, 720),
                ..default()
            }),
            ..default()
        }))
        .add_systems(Startup, (setup_camera, spawn_player))
        .add_systems(Update, player_movement)
        .run();
}

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(16, 18, 24)),
            ..default()
        },
    ));
}

fn spawn_player(mut commands: Commands, assets: Res<AssetServer>) {
    // Временный спрайт из сборки мини-станции (assets/sprites/ss14/ATTRIBUTION.md).
    let texture: Handle<Image> = assets.load("sprites/ss14/Mobs/Animals/monkey.rsi/monkey.png");
    commands.spawn((Player, Sprite::from_image(texture)));
}

#[derive(Component)]
struct Player;

fn player_movement(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut players: Query<&mut Transform, With<Player>>,
) {
    let mut direction = Vec2::ZERO;
    if input.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        direction.y += 1.0;
    }
    if input.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        direction.y -= 1.0;
    }
    if input.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        direction.x -= 1.0;
    }
    if input.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        direction.x += 1.0;
    }
    if direction == Vec2::ZERO {
        return;
    }

    let step = direction.normalize_or_zero() * MOVE_SPEED * time.delta_secs();
    for mut transform in players.iter_mut() {
        transform.translation += step.extend(0.0);
    }
}
