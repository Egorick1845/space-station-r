//! Bevy-клиент Space Station R.
//!
//! T0.2: окно 1280×720, тёмный фон, спрайт в центре, движение WASD/стрелками.
//! T1.2: lightyear raw-connection к серверу, рукопожатие Connect/Welcome.
//! T1.3: ввод шлётся на сервер; позиция игрока — реплицированная (ADR-3),
//!       без локального предсказания (запланировано после T2.2).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use bevy::asset::AssetId;
use bevy::prelude::*;
use bevy::text::Font;
use bevy::window::{Window, WindowPlugin, WindowResolution};
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::{GAME_NAME, PlayerPosition};

mod containers;
mod doors;
mod inventory_ui;
mod lobby;
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

/// Длительность серверного тика: интерполяция проигрывает путь между двумя
/// последними серверными позициями ровно за это время (без рывков).
const NET_TICK_SECS: f32 = 1.0 / NET_TPS as f32;

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
    app.init_resource::<tiles::ChunkRenderState>();
    app.init_resource::<lobby::LobbyState>();
    app.init_resource::<inventory_ui::OwnPlayerEntity>();
    app.init_resource::<inventory_ui::ActionMenu>();
    // RsiRegistry строится сразу после DefaultPlugins: нужен и игроку (обезьяна),
    // и дверям (closed/open) уже на Startup.
    let rsi_root = Path::new(&assets_file_path()).join("sprites/ss14");
    let registry = app
        .world_mut()
        .resource_scope(|world, mut images: Mut<Assets<Image>>| {
            let mut layouts = world.resource_mut::<Assets<TextureAtlasLayout>>();
            rsi::build_registry(&mut images, &mut layouts, &rsi_root)
        });
    app.insert_resource(registry);
    app.add_systems(Startup, (setup_camera, startup_map, install_default_font));
    app.add_systems(
        Update,
        (
            send_connect,
            send_input,
            receive_server,
            apply_player_state,
            count_replicated,
            tiles::render_map_chunks,
            tiles::despawn_orphan_chunks,
            camera_follow_player,
            doors::spawn_door_visuals,
            doors::update_door_visuals,
            doors::auto_interact,
            doors::hover_outline,
        )
            .run_if(in_game),
    );
    app.add_systems(
        Update,
        (
            inventory_ui::resolve_own_player,
            inventory_ui::spawn_remote_players,
            inventory_ui::sync_remote_players,
            inventory_ui::sync_inhand_items,
            inventory_ui::render_inventory_panel,
            inventory_ui::render_hands_panel,
            inventory_ui::render_action_menu,
            inventory_ui::inventory_slot_click,
            inventory_ui::hands_ui_click,
            inventory_ui::world_click,
            inventory_ui::action_menu_click,
            inventory_ui::inventory_test_mode,
            inventory_ui::build_test_mode,
            inventory_ui::attack_test_mode,
        )
            .run_if(in_game),
    );
    app.add_systems(
        Update,
        (
            containers::spawn_crate_visuals,
            containers::update_crate_visuals,
            containers::render_container_panel,
            containers::container_slot_click,
            containers::container_test_mode,
        )
            .run_if(in_game),
    );
    // Лобби — только в релизной сборке (в dev сразу в игру; для отладки
    // лобби в dev: SSR_LOBBY=1; тестовый обход лобби: SSR_AUTO_PLAY=1).
    let force_lobby = std::env::var_os("SSR_LOBBY").is_some();
    let auto_play = std::env::var_os("SSR_AUTO_PLAY").is_some();
    let show_lobby = !auto_play && (force_lobby || cfg!(not(debug_assertions)));
    if show_lobby {
        app.add_systems(Startup, lobby::spawn_lobby);
        app.add_systems(
            Update,
            (
                enter_game,
                lobby::lobby_button,
                lobby::animate_lobby_background,
            ),
        );
    } else {
        app.insert_resource(lobby::LobbyState::Playing);
        app.add_systems(Update, enter_game);
    }
    // Регистрация реплицируемых компонентов — одинакова на сервере и клиенте (T1.3).
    // ВАЖНО: порядок регистрации должен совпадать с сервером (иначе replicon
    // паникует «FnsId should be registered first»): PlayerPosition, TileChunkData, Door.
    app.component::<PlayerPosition>().replicate();
    // Чанки карты приходят с сервера (T2.3).
    app.component::<ssr_core::tiles::TileChunkData>()
        .replicate();
    // Двери: состояние реплицируется сервером (T3.1).
    app.component::<ssr_core::Door>().replicate();
    // Инвентарь и предметы (T3.2). Порядок обязан совпадать с сервером!
    app.component::<ssr_core::inventory::Inventory>()
        .replicate();
    app.component::<ssr_core::inventory::Item>().replicate();
    // Руки/здоровье/удержание (T3.3+). Тот же порядок, что у сервера!
    app.component::<ssr_core::inventory::Hands>().replicate();
    app.component::<ssr_core::inventory::Health>().replicate();
    app.component::<ssr_core::inventory::HeldBy>().replicate();
    // Контейнеры (T3.4). Тот же порядок, что у сервера!
    app.component::<ssr_core::inventory::Container>()
        .replicate();
    app.component::<ssr_core::inventory::ItemPosition>()
        .replicate();
    app.run();
}

/// Ставит Noto Sans (из сборки мини-станции, OFL) шрифтом по умолчанию:
/// встроенный шрифт Bevy — без кириллицы, из-за него в UI были «квадратики».
fn install_default_font(mut fonts: ResMut<Assets<Font>>) {
    let path = Path::new(&assets_file_path()).join("fonts/NotoSans-Regular.ttf");
    match std::fs::read(&path) {
        Ok(bytes) => {
            // Подмена дефолтного ассета: все тексты без явного шрифта
            // начинают использовать Noto Sans с кириллицей.
            let _ = fonts.insert(AssetId::default(), Font::from_bytes(bytes));
            tracing::info!("шрифт Noto Sans (кириллица) установлен");
        }
        Err(e) => tracing::warn!(path = %path.display(), error = %e, "шрифт не найден"),
    }
}

/// Игровые системы работают только вне лобби.
fn in_game(state: Res<lobby::LobbyState>) -> bool {
    *state == lobby::LobbyState::Playing
}

/// Вход в игру: после кнопки «Играть» создаём сетевой линк и спрайт игрока.
fn enter_game(
    mut commands: Commands,
    state: Res<lobby::LobbyState>,
    registry: Res<rsi::RsiRegistry>,
    mut entered: Local<bool>,
) {
    if *state != lobby::LobbyState::Playing || *entered {
        return;
    }
    *entered = true;
    // Сетевой линк клиента; идентификация по адресу (netcode — позже, T5.x).
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
    spawn_player_sprite(&mut commands, &registry);
    // Лом из SS14 рядом со стартовой точкой (демо IMP.1).
    let key = "sprites/ss14/Objects/Tools/crowbar.rsi#icon";
    rsi::spawn_rsi_sprite(
        &mut commands,
        &registry,
        key,
        0,
        Vec3::new(220.0, 120.0, 0.0),
    );
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
    player_entity: Res<PlayerEntity>,
    mut connected_elapsed: Local<f32>,
    connected: Query<(), With<Connected>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if !input_ready(&player_entity) {
        return;
    }
    if connected.iter().next().is_some() {
        *connected_elapsed += time.delta_secs();
    }
    let walk_secs = std::env::var("SSR_AUTO_WALK_MS")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map(|ms| ms / 1000.0)
        .unwrap_or(3.0);
    // SSR_LOCK_INPUT=1 — тестовый режим: клавиатура игнорируется
    // (ввод только автоходом), тесты детерминированы даже при чужом вводе.
    let locked = std::env::var_os("SSR_LOCK_INPUT").is_some();
    // SSR_AUTO_WALK_DIR="0,1" — направление автохода (по умолчанию вправо).
    let walk_dir = std::env::var("SSR_AUTO_WALK_DIR")
        .ok()
        .and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .unwrap_or(Vec2::X);
    // SSR_BUILD_TEST: с 9-й по 12-ю секунду клиент идёт на восток — в новую стену.
    let build_test_walk =
        std::env::var_os("SSR_BUILD_TEST").is_some() && (9.0..12.0).contains(&*connected_elapsed);
    let direction = if std::env::var_os("SSR_AUTO_WALK").is_some() && *connected_elapsed < walk_secs
    {
        walk_dir
    } else if build_test_walk {
        Vec2::X
    } else if locked {
        Vec2::ZERO
    } else {
        input_direction(&input)
    };
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Input {
            movement: direction.to_array(),
        });
    }
}

/// Готов ли клиент слать ввод: сервер создал игрока (Welcome получен).
/// До этого пакеты ввода бессмысленны и спамят лог сервера.
fn input_ready(player_entity: &PlayerEntity) -> bool {
    player_entity.0.is_some()
}

/// Принимает Welcome (и будущие серверные сообщения).
///
/// MessageReceiver появляется лениво, при первом входящем сообщении.
fn receive_server(
    mut receivers: Query<&mut MessageReceiver<ServerMessage>, With<Connected>>,
    mut player_entity: ResMut<PlayerEntity>,
    mut menu: ResMut<inventory_ui::ActionMenu>,
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
                ServerMessage::Actions { options } => {
                    // ВАЖНО: список действий обрабатывается ЗДЕСЬ же, потому что
                    // MessageReceiver осушается одним receive() — вторая система
                    // отбирала бы у этой сообщения (включая Welcome).
                    tracing::info!(count = options.len(), "actions received");
                    menu.options = options;
                }
                ServerMessage::EntityDelta { .. } => {} // дельты позиций — T1.3/T1.4
                ServerMessage::Event { kind } => tracing::info!(kind, "server event"),
            }
        }
    }
}

/// Клиентская сущность своего игрока: bits из Welcome → ServerEntityMap → клиент.
pub(crate) fn own_player_entity(
    player_entity: &PlayerEntity,
    entity_map: &Option<Res<ServerEntityMap>>,
) -> Option<Entity> {
    let server = Entity::try_from_bits(player_entity.0?)?;
    entity_map.as_deref()?.to_client().get(&server).copied()
}

/// Интерполяция собственной позиции между серверными снимками (T3.x):
/// держим предыдущую и целевую точки и проигрываем путь ровно за тик сети —
/// движение равномерное, без экспоненциального «догоняния» и дробления.
#[derive(Default)]
struct PositionInterp {
    prev: Vec2,
    target: Vec2,
    elapsed: f32,
    started: bool,
}

/// Двигает спрайт к реплицированной позиции СВОЕГО игрока (не «единственного»:
/// рядом бывают другие игроки) и выбирает сторону из 4 по движению.
fn apply_player_state(
    time: Res<Time>,
    player_entity: Res<PlayerEntity>,
    entity_map: Option<Res<ServerEntityMap>>,
    positions: Query<&PlayerPosition>,
    registry: Res<rsi::RsiRegistry>,
    mut interp: Local<PositionInterp>,
    mut sprites: Query<(&mut Transform, &mut Sprite, &mut PlayerFacing), With<Player>>,
) {
    const KEY: &str = "sprites/ss14/Mobs/Animals/monkey.rsi#monkey";
    let Some(own) = own_player_entity(&player_entity, &entity_map) else {
        return;
    };
    let Ok(target) = positions.get(own) else {
        return;
    };
    let target = Vec2::from_array(target.0);

    if !interp.started {
        interp.prev = target;
        interp.target = target;
        interp.started = true;
    } else if target != interp.target {
        // Новая серверная позиция: продолжаем путь от текущей отрисованной точки.
        interp.prev = interp
            .prev
            .lerp(interp.target, (interp.elapsed / NET_TICK_SECS).min(1.0));
        interp.target = target;
        interp.elapsed = 0.0;
    }
    interp.elapsed += time.delta_secs();
    let alpha = (interp.elapsed / NET_TICK_SECS).min(1.0);
    let current = interp.prev.lerp(interp.target, alpha);

    // Направление — по серверному смещению за последний снимок.
    // Порядок RsiDirection движка: South=0, North=1, East=2, West=3.
    let delta = interp.target - interp.prev;
    let direction = (delta.length() >= 0.5).then(|| {
        if delta.x.abs() > delta.y.abs() {
            if delta.x > 0.0 { 2 } else { 3 } // восток / запад
        } else if delta.y > 0.0 {
            1 // север
        } else {
            0 // юг
        }
    });

    let sprite_rsi = registry.get(KEY);
    for (mut transform, mut sprite, mut facing) in sprites.iter_mut() {
        transform.translation.x = current.x;
        transform.translation.y = current.y;
        if let Some(direction) = direction
            && facing.0 != direction
        {
            facing.0 = direction;
            if let (Some(rsi), Some(atlas)) = (sprite_rsi, sprite.texture_atlas.as_mut()) {
                atlas.index = rsi.index(direction, 0);
            }
        }
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

/// Спрайт игрока из RSI с атласом направлений (порядок движка: S,N,E,W).
fn spawn_player_sprite(commands: &mut Commands, registry: &rsi::RsiRegistry) {
    const KEY: &str = "sprites/ss14/Mobs/Animals/monkey.rsi#monkey";
    let Some(sprite) = registry.get(KEY) else {
        tracing::warn!(KEY, "player rsi missing");
        return;
    };
    let mut sprite_component = Sprite::from_image(sprite.image.clone());
    sprite_component.texture_atlas = Some(TextureAtlas {
        layout: sprite.layout.clone(),
        index: sprite.index(0, 0), // старт: смотрит на юг
    });
    // z = 1: игрок рисуется поверх тайлов карты.
    commands.spawn((
        Player,
        sprite_component,
        PlayerFacing(0),
        Transform::from_xyz(0.0, 0.0, 1.0),
    ));
}

/// Камера жёстко следует за отрисованной позицией игрока: никакого второго
/// сглаживания — иначе мир «дрожит» относительно персонажа.
fn camera_follow_player(
    player: Query<&Transform, (With<Player>, Without<Camera2d>)>,
    mut camera: Single<&mut Transform, With<Camera2d>>,
) {
    let Ok(target) = player.single() else {
        return;
    };
    camera.translation.x = target.translation.x;
    camera.translation.y = target.translation.y;
}

/// Текущее направление игрока (0 юг, 1 восток, 2 север, 3 запад).
#[derive(Component, Default, Clone, Copy, PartialEq)]
struct PlayerFacing(u32);

/// Грузит прототипы/спрайты тайлов; сами чанки приходят с сервера (T2.3).
fn startup_map(mut commands: Commands) {
    let root = Path::new(&assets_file_path()).to_path_buf();
    let visuals = tiles::load_tile_visuals(&root);
    commands.insert_resource(visuals);
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
