//! Bevy-клиент Space Station R.
//!
//! T0.2: окно 1280×720, тёмный фон, спрайт в центре, движение WASD/стрелки.

use std::path::Path;

use bevy::prelude::*;
use bevy::window::{Window, WindowPlugin, WindowResolution};

use ssr_core::GAME_NAME;

/// Скорость тестового персонажа, пикселей в секунду.
const MOVE_SPEED: f32 = 300.0;

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: format!("{GAME_NAME} — dev client"),
                        resolution: WindowResolution::new(1280, 720),
                        ..default()
                    }),
                    ..default()
                })
                .set(asset_plugin()),
        )
        .add_systems(Startup, (setup_camera, spawn_player))
        .add_systems(Update, player_movement)
        .run();
}

/// Плагин ассетов с явным путём к каталогу `assets/` в корне репозитория.
///
/// Bevy по умолчанию ищет ассеты относительно BEVY_ASSET_ROOT / CARGO_MANIFEST_DIR
/// (при `cargo run` это `crates/client` — мимо корня репо) или каталога exe,
/// поэтому путь задаём явно. Если BEVY_ASSET_ROOT выставлен вручную — не мешаем.
fn asset_plugin() -> AssetPlugin {
    if std::env::var_os("BEVY_ASSET_ROOT").is_some() {
        return AssetPlugin::default();
    }
    AssetPlugin {
        file_path: assets_file_path(),
        ..default()
    }
}

/// Абсолютный путь к `<repo>/assets/`, вычисленный из окружения запуска.
fn assets_file_path() -> String {
    // `cargo run`: CARGO_MANIFEST_DIR = <repo>/crates/client → корень через два уровня.
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        if let Some(root) = Path::new(&manifest_dir).ancestors().nth(2) {
            return root.join("assets").to_string_lossy().into_owned();
        }
    }
    // Прямой запуск: <repo>/target/<profile>/ssr-client.exe → корень через три уровня.
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(root) = exe_path.ancestors().nth(3) {
            return root.join("assets").to_string_lossy().into_owned();
        }
    }
    "assets".into()
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
