//! Bevy-клиент Space Station R.
//!
//! T0.2: окно 1280×720, тёмный фон, спрайт в центре, движение WASD/стрелки.
//! T1.2: lightyear raw-connection к серверу, рукопожатие Connect/Welcome.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use bevy::prelude::*;
use bevy::window::{Window, WindowPlugin, WindowResolution};
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_protocol::net::GameChannel;
use ssr_protocol::{
    ClientMessage, DEFAULT_SERVER_PORT, PROTOCOL_VERSION, ProtocolPlugin, ServerMessage,
    is_compatible,
};

use ssr_core::GAME_NAME;

/// Скорость тестового персонажа, пикселей в секунду.
const MOVE_SPEED: f32 = 300.0;

/// Тик-рейт сети (совпадает с сервером, T0.3).
const NET_TPS: f64 = 20.0;

/// Адрес сервера (T5.4 добавит экран подключения).
const SERVER_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_SERVER_PORT);

/// Локальный конец линка (порт 0 — любой свободный).
const CLIENT_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

/// Имя игрока до появления экрана входа (T5.4).
const DEV_PLAYER_NAME: &str = "SSR-dev";

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
        .add_plugins(ClientPlugins {
            tick_duration: std::time::Duration::from_secs_f64(1.0 / NET_TPS),
        })
        .add_plugins(ProtocolPlugin)
        .init_resource::<Handshake>()
        .init_resource::<PlayerEntity>()
        .add_systems(Startup, (setup_camera, spawn_player, startup_connection))
        .add_systems(Update, (player_movement, send_connect, receive_server))
        .run();
}

/// Сетевой линк клиента; идентификация по адресу (netcode — позже, T5.x).
fn startup_connection(mut commands: Commands) {
    commands
        .spawn((
            Client,
            LocalAddr(CLIENT_ADDR),
            PeerAddr(SERVER_ADDR),
            Link::default(),
            RawClient,
            UdpIo::default(),
        ))
        .trigger(Connect::from);
}

/// Состояние рукопожатия.
#[derive(Resource, Default)]
struct Handshake {
    connect_sent: bool,
}

/// Серверная сущность игрока из Welcome (ADR-7).
#[derive(Resource, Default)]
struct PlayerEntity(Option<u64>);

/// Шлёт Connect, как только линк подключился.
///
/// MessageSender авто-добавляется required-компонентом на линк клиента.
fn send_connect(
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut handshake: ResMut<Handshake>,
) {
    if handshake.connect_sent {
        return;
    }
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Connect {
            protocol_version: PROTOCOL_VERSION,
            name: DEV_PLAYER_NAME.into(),
        });
        handshake.connect_sent = true;
        tracing::info!(name = DEV_PLAYER_NAME, "Connect sent");
    }
}

/// Принимает Welcome (и будущие серверные сообщения).
///
/// MessageReceiver появляется лениво, при первом входящем сообщении.
fn receive_server(
    mut receivers: Query<&mut MessageReceiver<ServerMessage>, With<Connected>>,
    mut player_entity: ResMut<PlayerEntity>,
) {
    for mut receiver in receivers.iter_mut() {
        for message in receiver.receive() {
            match message {
                ServerMessage::Welcome {
                    player_entity: entity,
                    protocol_version,
                } => {
                    if !is_compatible(protocol_version) {
                        tracing::error!(
                            server_version = protocol_version,
                            "server version mismatch"
                        );
                        continue;
                    }
                    player_entity.0 = Some(entity);
                    tracing::info!(player_entity = entity, "Welcome accepted");
                }
                ServerMessage::WorldState { .. } => {} // репликация мира — T1.3
                ServerMessage::EntityDelta { .. } => {} // дельты позиций — T1.3
                ServerMessage::Event { kind } => tracing::info!(kind, "server event"),
            }
        }
    }
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
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR")
        && let Some(root) = Path::new(&manifest_dir).ancestors().nth(2)
    {
        return root.join("assets").to_string_lossy().into_owned();
    }
    // Прямой запуск: <repo>/target/<profile>/ssr-client.exe → корень через три уровня.
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(root) = exe_path.ancestors().nth(3)
    {
        return root.join("assets").to_string_lossy().into_owned();
    }
    "assets".into()
}
