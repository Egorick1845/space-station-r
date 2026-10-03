//! Headless-сервер Space Station R.
//!
//! T0.3: Bevy MinimalPlugins, фиксированный тик-рейт 20 TPS, лог тиков.
//! T1.2: lightyear raw-connection на 127.0.0.1:7777, рукопожатие Connect/Welcome.
//! T1.3: ввод клиента двигает сущность игрока; позиция реплицируется компонентом.
//! T1.4: interest management — видимость по чанкам (комнаты lightyear).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use lightyear::connection::server::Start;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use ssr_core::{CHUNK_SIZE, PLAYER_MOVE_SPEED, PlayerPosition, chunk_coords};
use ssr_protocol::net::GameChannel;
use ssr_protocol::{
    ClientMessage, DEFAULT_SERVER_PORT, PROTOCOL_VERSION, ProtocolPlugin, ServerMessage,
    is_compatible,
};
use tracing_subscriber::EnvFilter;

/// Тик-рейт сервера (PLAN.md T0.3).
const TPS: f64 = 20.0;

/// Радиус интереса в чанках (PLAN.md T1.4): клиент получает сущности в квадрате 5×5 чанков.
const INTEREST_RADIUS: i32 = 2;

/// Число сущностей нагрузочного теста (критерий T1.4: клиент получает < 100 из 1000).
const LOAD_TEST_ENTITIES: u32 = 1000;

/// Адрес, который слушает сервер.
const SERVER_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_SERVER_PORT);

fn main() {
    // Фильтр логов берётся из RUST_LOG (например, `RUST_LOG=debug`), по умолчанию info.
    // Внимание: bevy_log включает у tracing-subscriber фичу env-filter, из-за чего
    // `fmt::try_init()` без явного фильтра строит пустой EnvFilter и молчит.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if let Err(e) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("tracing init failed: {e}");
    }

    let mut app = App::new();
    app.add_plugins(
        MinimalPlugins
            .set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / TPS,
            )))
            // ServerPlugins использует init_state — нужен StateTransition-расписок.
            .add(bevy::state::app::StatesPlugin),
    );
    app.add_plugins(ServerPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / TPS),
    });
    app.add_plugins(ProtocolPlugin);
    // Interest management через комнаты (T1.4): сущность видна клиенту,
    // если они делят хотя бы одну комнату.
    app.add_plugins(RoomPlugin);
    app.init_resource::<Players>();
    app.init_resource::<TickState>();
    app.init_resource::<ChunkRooms>();
    app.add_systems(Startup, startup);
    app.add_systems(Startup, spawn_load_test);
    app.add_systems(
        Update,
        (
            tick_logger,
            handle_client_messages,
            movement,
            update_client_rooms,
        ),
    );
    app.add_observer(on_link_connected);
    app.add_observer(on_link_disconnected);
    // Регистрация реплицируемых компонентов — одинакова на сервере и клиенте (T1.3).
    app.component::<PlayerPosition>().replicate();
    app.run();
}

/// Сетевой сервер: одна сущность-линк на клиента.
fn startup(mut commands: Commands) -> Result {
    // RawServer: идентификация клиента по адресу (netcode/авторизация — позже, с привязкой к сайту).
    let server = commands
        .spawn((RawServer, LocalAddr(SERVER_ADDR), ServerUdpIo::default()))
        .id();
    commands.trigger(Start { entity: server });
    tracing::info!(%SERVER_ADDR, "server listening");
    Ok(())
}

/// Игрок на сервере: линк, имя, игровая сущность и текущий набор комнат интереса.
struct PlayerEntry {
    link: Entity,
    name: String,
    player: Entity,
    chunk: (i32, i32),
    rooms: Vec<RoomId>,
}

#[derive(Resource, Default)]
struct Players {
    entries: Vec<PlayerEntry>,
}

impl Players {
    fn entry_by_link_mut(&mut self, link_bits: u64) -> Option<&mut PlayerEntry> {
        self.entries
            .iter_mut()
            .find(|e| e.link.to_bits() == link_bits)
    }

    fn remove_by_link(&mut self, link_bits: u64) -> Option<(String, Entity)> {
        let index = self
            .entries
            .iter()
            .position(|e| e.link.to_bits() == link_bits)?;
        let entry = self.entries.remove(index);
        Some((entry.name, entry.player))
    }
}

/// Чанк → комната (T1.4). Комнаты выделяются лениво из RoomAllocator.
#[derive(Resource, Default)]
struct ChunkRooms {
    map: HashMap<(i32, i32), RoomId>,
}

impl ChunkRooms {
    fn room_for(&mut self, chunk: (i32, i32), allocator: &mut RoomAllocator) -> RoomId {
        *self
            .map
            .entry(chunk)
            .or_insert_with(|| allocator.allocate())
    }
}

/// Текущий ввод игрока; сервер применяет его каждый тик (ADR-3).
/// Не реплицируется — это серверная деталь применения ввода.
#[derive(Component, Default)]
struct PlayerInput([f32; 2]);

fn on_link_connected(trigger: On<Add, Connected>, mut commands: Commands) {
    // ReplicationSender — чтобы сервер слал компоненты этому клиенту (T1.3);
    // Rooms — фильтр видимости, наполняется системой update_client_rooms (T1.4).
    commands
        .entity(trigger.entity)
        .insert((ReplicationSender, Rooms::default()));
    tracing::debug!(link = ?trigger.entity, "link established");
}

fn on_link_disconnected(
    trigger: On<Add, Disconnected>,
    mut commands: Commands,
    mut players: ResMut<Players>,
) {
    let bits = trigger.entity.to_bits();
    if let Some((name, entity)) = players.remove_by_link(bits) {
        // despawn реплицируется: у клиентов сущность игрока удалится (T1.3).
        commands.entity(entity).despawn();
        tracing::info!(name, "Player disconnected");
    } else {
        tracing::debug!(link = ?trigger.entity, "link disconnected before handshake");
    }
}

/// Единая точка приёма сообщений клиента: рукопожатие (Connect/Welcome, T1.2)
/// и ввод (Input → PlayerInput, T1.3).
///
/// ВАЖНО: MessageReceiver осушается одним вызовом receive(), поэтому все
/// обработчики сообщений клиента живут в этой одной системе — иначе системы
/// конкурировали бы за один буфер и теряли сообщения.
fn handle_client_messages(
    mut commands: Commands,
    mut receivers: Query<(Entity, &RemoteId, &mut MessageReceiver<ClientMessage>), With<Connected>>,
    mut senders: Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
    mut inputs: Query<&mut PlayerInput>,
    mut players: ResMut<Players>,
) {
    let mut connected: Vec<(Entity, String)> = Vec::new();
    for (link_entity, remote_id, mut receiver) in receivers.iter_mut() {
        for message in receiver.receive() {
            match message {
                ClientMessage::Connect {
                    protocol_version,
                    name,
                } => {
                    if !is_compatible(protocol_version) {
                        // Несовпадение версии = отказ соединения (PLAN.md T1.1).
                        tracing::warn!(
                            client = ?remote_id,
                            peer_version = protocol_version,
                            "version mismatch, connection refused"
                        );
                        continue;
                    }
                    tracing::info!(client = ?remote_id, name, "Player connected");
                    connected.push((link_entity, name));
                }
                ClientMessage::Input { movement } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        tracing::warn!(link = ?link_entity, "input for unknown player");
                        continue;
                    };
                    match inputs.get_mut(entry.player) {
                        Ok(mut input) => {
                            input.0 = movement;
                            tracing::debug!(player = ?entry.player, input = ?movement, "input applied");
                        }
                        Err(e) => {
                            tracing::warn!(player = ?entry.player, error = %e, "no PlayerInput")
                        }
                    }
                }
                ClientMessage::Interact { .. } => {} // взаимодействие — T3.1
            }
        }
    }

    for (link_entity, name) in connected {
        // Игровая сущность игрока: позиция реплицируется клиентам из интереса (T1.3/T1.4).
        let player = commands
            .spawn((
                PlayerPosition::default(),
                PlayerInput::default(),
                Replicate::to_clients(NetworkTarget::All),
                Rooms::default(),
            ))
            .id();

        if let Ok((_, mut sender)) = senders.get_mut(link_entity) {
            sender.send::<GameChannel>(ServerMessage::Welcome {
                player_entity: player.to_bits(),
                protocol_version: PROTOCOL_VERSION,
            });
        }

        // Стартовый чанк (0,0): комнаты выделятся в update_client_rooms.
        players.entries.push(PlayerEntry {
            link: link_entity,
            name,
            player,
            chunk: (0, 0),
            rooms: Vec::new(),
        });
    }
}

/// Двигает игроков по их вводу; результат реплицируется компонентом PlayerPosition.
fn movement(time: Res<Time>, mut players: Query<(&PlayerInput, &mut PlayerPosition)>) {
    let dt = time.delta_secs();
    for (input, mut position) in players.iter_mut() {
        let dir = Vec2::from_array(input.0);
        if dir == Vec2::ZERO {
            continue;
        }
        let next = Vec2::from_array(position.0) + dir.normalize_or_zero() * PLAYER_MOVE_SPEED * dt;
        position.0 = next.to_array();
        tracing::debug!(position = ?position.0, "player moved");
    }
}

/// Interest management (T1.4): набор комнат клиента = чанки в радиусе
/// INTEREST_RADIUS от его игрока. Игрок и линк получают одинаковый набор.
///
/// ВАЖНО: Rooms — immutable-компонент (`&mut` не бывает), поэтому набор
/// меняется полной заменой через Commands, а не инкрементально.
fn update_client_rooms(
    mut commands: Commands,
    links: Query<Entity, With<Connected>>,
    positions: Query<&PlayerPosition>,
    mut players: ResMut<Players>,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
) {
    for entry in players.entries.iter_mut() {
        let Ok(position) = positions.get(entry.player) else {
            continue;
        };
        let chunk = chunk_coords(position.0[0], position.0[1]);
        let unchanged = chunk == entry.chunk && !entry.rooms.is_empty();
        if unchanged || !links.contains(entry.link) {
            continue;
        }

        let mut desired = Vec::with_capacity(25);
        for dx in -INTEREST_RADIUS..=INTEREST_RADIUS {
            for dy in -INTEREST_RADIUS..=INTEREST_RADIUS {
                desired.push(chunk_rooms.room_for((chunk.0 + dx, chunk.1 + dy), &mut allocator));
            }
        }

        // Комната игрока идентична комнате линка — обе стороны получают один набор.
        if let Ok(mut link_entity) = commands.get_entity(entry.link) {
            link_entity.try_insert(Rooms::from(desired.iter().copied()));
        }
        if let Ok(mut player_entity) = commands.get_entity(entry.player) {
            player_entity.try_insert(Rooms::from(desired.iter().copied()));
        }

        tracing::debug!(chunk = ?chunk, rooms = desired.len(), "interest updated");
        entry.chunk = chunk;
        entry.rooms = desired;
    }
}

/// Нагрузочный тест T1.4: SSR_LOAD_TEST=1 расставляет 1000 сущностей сеткой
/// 32 юнита (чанки −20..20 × −12..12). Критерий: клиент видит < 100 из 1000.
fn spawn_load_test(
    mut commands: Commands,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
) {
    if std::env::var_os("SSR_LOAD_TEST").is_none() {
        return;
    }
    let count = LOAD_TEST_ENTITIES;
    for i in 0..count {
        let x = ((i % 40) as f32 - 20.0) * CHUNK_SIZE + 16.0;
        let y = ((i / 40) as f32 - 12.0) * CHUNK_SIZE + 16.0;
        let chunk = chunk_coords(x, y);
        let room = chunk_rooms.room_for(chunk, &mut allocator);
        commands.spawn((
            PlayerPosition([x, y]),
            Replicate::to_clients(NetworkTarget::All),
            Rooms::single(room),
        ));
    }
    tracing::info!(count, "load test entities spawned");
}

/// Счётчик тиков сервера с момента запуска (T0.3).
#[derive(Resource, Default)]
struct TickState {
    tick: u64,
}

fn tick_logger(mut state: ResMut<TickState>) {
    state.tick += 1;
    tracing::debug!(tick = state.tick, "server tick");
}
