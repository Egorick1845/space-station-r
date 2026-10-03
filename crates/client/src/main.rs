//! Bevy-клиент Space Station R.
//!
//! T0.2: окно 1280×720, тёмный фон, спрайт в центре, движение WASD/стрелками.
//! T1.2: lightyear raw-connection к серверу, рукопожатие Connect/Welcome.
//! T1.3: ввод шлётся на сервер; позиция игрока — реплицированная (ADR-3),
//!       без локального предсказания (запланировано после T2.2).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use bevy::prelude::*;
use bevy::window::{Window, WindowPlugin, WindowResolution};
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::{GAME_NAME, PlayerPosition};

mod doors;
mod rsi;
mod tiles;
use ssr_protocol::net::GameChannel;
use ssr_protocol::{
    ClientMessage, DEFAULT_SERVER_PORT, PROTOCOL_VERSION, ProtocolPlugin, ServerMessage,
    is_compatible,
};

/// Тик-рейт сети (совпадает с сервером, T0.3).
const NET_TPS: f64 = 20.0;

/// Адрес сервера (T5.4 добавит экран подключения).
const SERVER_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_SERVER_PORT);

/// Локальный конец линка (порт 0 — любой свободный).
const CLIENT_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

/// Имя игрока до появления экрана входа (T5.4).
const DEV_PLAYER_NAME: &str = "SSR-dev";

/// Скорость сглаживания реплицированной позиции (экспоненциальный lerp).
const POSITION_SMOOTHING: f32 = 12.0;

fn main() {
    let mut app = App::new();
    app.add_plugins(
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
    );
    app.add_plugins(ClientPlugins {
        tick_duration: std::time::Duration::from_secs_f64(1.0 / NET_TPS),
    });
    app.add_plugins(ProtocolPlugin);
    app.init_resource::<Handshake>();
    app.init_resource::<PlayerEntity>();
    app.add_systems(
        Startup,
        (
            setup_camera,
            spawn_player,
            startup_connection,
            startup_rsi,
            startup_map,
        ),
    );
    app.add_systems(
        Update,
        (
            send_connect,
            send_input,
            receive_server,
            apply_position,
            count_replicated,
            tiles::render_map_chunks,
            tiles::camera_edge_scroll,
            doors::spawn_door_visuals,
            doors::update_door_visuals,
            doors::click_interact,
            doors::auto_interact,
        ),
    );
    // Регистрация реплицируемых компонентов — одинакова на сервере и клиенте (T1.3).
    // ВАЖНО: порядок регистрации должен совпадать с сервером (иначе replicon
    // паникует «FnsId should be registered first»): PlayerPosition, TileChunkData, Door.
    app.component::<PlayerPosition>().replicate();
    // Чанки карты приходят с сервера (T2.3).
    app.component::<ssr_core::tiles::TileChunkData>()
        .replicate();
    // Двери: состояние реплицируется сервером (T3.1).
    app.component::<ssr_core::Door>().replicate();
    app.run();
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
            // принимает реплицируемые компоненты сервера (T1.3)
            ReplicationReceiver,
        ))
        .trigger(Connect::from);
}

/// Состояние рукопожатия.
#[derive(Resource, Default)]
struct Handshake {
    connect_sent: bool,
}

/// Серверная сущность игрока из Welcome (ADR-7: клиент хранит маппинг серверных ID).
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

/// Направление ввода WASD/стрелок.
fn input_direction(input: &ButtonInput<KeyCode>) -> Vec2 {
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
    direction.normalize_or_zero()
}

/// Шлёт серверу текущее направление ввода (сервер сам двигает игрока, ADR-3).
///
/// Состояние шлётся каждый кадр (а не только при изменении): первый пакет может
/// прийти до создания сущности игрока на сервере — актуальный ввод самолечится.
/// Сжатие до тик-рейта сети сделаем в T1.4.
///
/// SSR_AUTO_WALK=1 — тестовый режим: после подключения клиент «держит вправо»
/// SSR_AUTO_WALK_MS миллисекунд (по умолчанию 3000; короткое значение оставляет
/// игрока у двери для тестов взаимодействия). Пригодится ботам в T6.1.
fn send_input(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut connected_elapsed: Local<f32>,
    connected: Query<(), With<Connected>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if connected.iter().next().is_some() {
        *connected_elapsed += time.delta_secs();
    }
    let walk_secs = std::env::var("SSR_AUTO_WALK_MS")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map(|ms| ms / 1000.0)
        .unwrap_or(3.0);
    let direction = if std::env::var_os("SSR_AUTO_WALK").is_some() && *connected_elapsed < walk_secs
    {
        Vec2::X
    } else {
        input_direction(&input)
    };
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Input {
            movement: direction.to_array(),
        });
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
                ServerMessage::WorldState { .. } => {} // полный снимок — T1.4
                ServerMessage::EntityDelta { .. } => {} // дельты позиций — T1.3/T1.4
                ServerMessage::Event { kind } => tracing::info!(kind, "server event"),
            }
        }
    }
}

/// Двигает спрайт к реплицированной позиции сервера с экспоненциальным сглаживанием.
/// Отставание при локальной игре заведомо меньше 200 мс (критерий T1.3).
fn apply_position(
    time: Res<Time>,
    mut render_pos: Local<Option<Vec2>>,
    positions: Query<&PlayerPosition>,
    mut sprites: Query<&mut Transform, With<Player>>,
) {
    let Ok(target) = positions.single() else {
        return;
    };
    let target = Vec2::from_array(target.0);

    let current = render_pos.unwrap_or(target);
    let alpha = 1.0 - (-POSITION_SMOOTHING * time.delta_secs()).exp();
    let current = current.lerp(target, alpha);
    *render_pos = Some(current);

    for mut transform in sprites.iter_mut() {
        transform.translation.x = current.x;
        transform.translation.y = current.y;
    }
    tracing::debug!(target = ?target.to_array(), render = ?current.to_array(), "position applied");
}

/// Счётчик и позиции видимых игроков (T1.4/T2.4): лог раз в 2 секунды.
/// Позиции меняются на глазах — видно, что реплицируется движение.
fn count_replicated(
    time: Res<Time>,
    mut next_log: Local<f32>,
    replicated: Query<&PlayerPosition, With<Remote>>,
) {
    *next_log += time.delta_secs();
    if *next_log < 2.0 {
        return;
    }
    *next_log = 0.0;
    let mut positions: Vec<[f32; 2]> = replicated.iter().map(|p| p.0).collect();
    if positions.is_empty() {
        return;
    }
    // Постоянный порядок в логе: сортировка по x, затем по y.
    positions.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    tracing::info!(count = positions.len(), positions = ?positions, "visible players");
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
    // z = 1: игрок рисуется поверх тайлов карты.
    commands.spawn((
        Player,
        Sprite::from_image(texture),
        Transform::from_xyz(0.0, 0.0, 1.0),
    ));
}

/// Грузит прототипы/спрайты тайлов; сами чанки приходят с сервера (T2.3).
fn startup_map(mut commands: Commands) {
    let root = Path::new(&assets_file_path()).to_path_buf();
    let visuals = tiles::load_tile_visuals(&root);
    commands.insert_resource(visuals);
}

/// Строит RsiRegistry и спавнит лом из SS14 (критерий IMP.1: лом виден в окне).
fn startup_rsi(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let root = Path::new(&assets_file_path()).join("sprites/ss14");
    let registry = rsi::build_registry(&mut images, &mut layouts, &root);
    let key = "sprites/ss14/Objects/Tools/crowbar.rsi#icon";
    match rsi::spawn_rsi_sprite(
        &mut commands,
        &registry,
        key,
        0,
        Vec3::new(220.0, 120.0, 0.0),
    ) {
        Some(_) => tracing::info!(key, "rsi sprite spawned"),
        None => tracing::warn!(key, "rsi sprite not found in registry"),
    }
    commands.insert_resource(registry);
}

#[derive(Component)]
struct Player;

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
