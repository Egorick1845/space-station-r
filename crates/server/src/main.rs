//! Headless-сервер Space Station R.
//!
//! T0.3: Bevy MinimalPlugins, фиксированный тик-рейт 20 TPS, лог тиков.
//! T1.2: lightyear raw-connection на 127.0.0.1:7777, рукопожатие Connect/Welcome.
//! T1.3: ввод клиента двигает сущность игрока; позиция реплицируется компонентом.
//! T1.4: interest management — видимость по чанкам (комнаты lightyear).

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use avian2d::math::Vector;
use avian2d::prelude::{
    Collider, ColliderDisabled, Gravity, LinearVelocity, PhysicsPlugins, Position, RigidBody,
    Rotation,
};
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use lightyear::connection::server::Start;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use ssr_core::tiles::{MapFile, TileChunkData, TileType};
use ssr_core::{
    CHUNK_UNITS, Door, INTERACT_RANGE, PLAYER_MOVE_SPEED, PlayerPosition, TILE_SIZE, chunk_coords,
};
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
    // Физика только на сервере (ADR-3): стены — статические тела, игрок — динамическое.
    app.add_plugins(PhysicsPlugins::default());
    // Топ-даун вид: гравитация avian (−9.81 по Y) не нужна.
    app.insert_resource(Gravity(Vector::ZERO));
    app.init_resource::<Players>();
    app.init_resource::<TickState>();
    app.init_resource::<ChunkRooms>();
    app.init_resource::<SpawnCursor>();
    app.add_systems(Startup, startup);
    app.add_systems(
        Startup,
        (
            load_prototypes,
            load_map,
            spawn_walls,
            spawn_load_test,
            spawn_collision_test,
        )
            .chain(),
    );
    app.add_systems(
        Update,
        (
            tick_logger,
            handle_client_messages,
            movement,
            sync_replicated_position,
            update_client_rooms,
            log_player_position,
        ),
    );
    app.add_observer(on_link_connected);
    app.add_observer(on_link_disconnected);
    // Регистрация реплицируемых компонентов — одинакова на сервере и клиенте (T1.3).
    app.component::<PlayerPosition>().replicate();
    // Чанки карты реплицируются с учётом интереса (T2.3).
    app.component::<TileChunkData>().replicate();
    // Двери: сервер-авторитарное состояние (T3.1).
    app.component::<Door>().replicate();
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

/// Прототипы контента (портированы из SS14, задача IMP.2/IMP.3):
/// грузятся при старте, чтобы конвейер импорта проверялся в рантайме.
/// Использование в игровых системах — по мере появления фаз 4–5.
#[derive(Resource)]
struct Prototypes(ssr_core::prototypes::ProtoSet);

/// Грузит `assets/prototypes_ss14.ron` (9258 прототипов сущностей).
fn load_prototypes(mut commands: Commands) {
    let path = ssr_core::assets_root().join("prototypes_ss14.ron");
    match ssr_core::prototypes::ProtoSet::load(&path) {
        Ok(set) => {
            let with_sprite = set.protos.iter().filter(|p| p.sprite.is_some()).count();
            tracing::info!(
                protos = set.protos.len(),
                with_sprite,
                "content prototypes loaded"
            );
            commands.insert_resource(Prototypes(set));
        }
        Err(e) => tracing::warn!(error = %e, "content prototypes not loaded"),
    }
}

/// Загруженная карта (T2.3): чанки для репликации и точки спавна.
#[derive(Resource)]
struct GameMap {
    spawn_points: Vec<(f32, f32)>,
    chunks: Vec<TileChunkData>,
}

/// Курсор выдачи точек спавна (T2.4): каждый новый игрок получает следующую
/// точку по кругу, чтобы игроки не появлялись друг в друге.
#[derive(Resource, Default)]
struct SpawnCursor(usize);

/// Загрузка карты из `assets/maps/test.ron` (T2.3): правка файла + рестарт
/// сервера меняют мир без перекомпиляции. Чанки спавнятся сущностями
/// и реплицируются с учётом интереса (комната = чанк).
fn load_map(
    mut commands: Commands,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
) {
    // SSR_MAP=imported_aspid.ron — выбрать карту (файлы в assets/maps/).
    let map_name = std::env::var("SSR_MAP").unwrap_or_else(|_| "test.ron".to_string());
    let path = ssr_core::assets_root().join("maps").join(&map_name);
    let file = MapFile::load(&path).unwrap_or_else(|e| panic!("{e}"));
    let chunks: Vec<TileChunkData> = file
        .to_chunks()
        .unwrap_or_else(|e| panic!("{e}"))
        .iter()
        .map(Into::into)
        .collect();

    for data in &chunks {
        let room = chunk_rooms.room_for(data.coords, &mut allocator);
        commands.spawn((
            data.clone(),
            Replicate::to_clients(NetworkTarget::All),
            Rooms::single(room),
        ));
    }
    // Двери (T3.1): статичные тела, закрытые; состояние реплицируется клиентам.
    for &(x, y) in &file.doors {
        let room = chunk_rooms.room_for(chunk_coords(x, y), &mut allocator);
        commands.spawn((
            Door {
                open: false,
                position: [x, y],
            },
            Replicate::to_clients(NetworkTarget::All),
            Rooms::single(room),
            RigidBody::Static,
            Collider::rectangle(TILE_SIZE, TILE_SIZE),
            Position(Vector::new(x, y)),
            Rotation::default(),
        ));
    }
    tracing::info!(
        name = %file.name,
        chunks = chunks.len(),
        spawns = file.spawn_points.len(),
        doors = file.doors.len(),
        "map loaded"
    );
    commands.insert_resource(GameMap {
        spawn_points: file.spawn_points,
        chunks,
    });
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
#[allow(clippy::too_many_arguments)]
fn handle_client_messages(
    mut commands: Commands,
    map: Res<GameMap>,
    mut spawn_cursor: ResMut<SpawnCursor>,
    mut receivers: Query<(Entity, &RemoteId, &mut MessageReceiver<ClientMessage>), With<Connected>>,
    mut senders: Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
    mut inputs: Query<&mut PlayerInput>,
    positions: Query<&PlayerPosition>,
    mut doors: Query<&mut Door>,
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
                        // Штатный случай: ввод пришёл раньше Connect (спавна игрока).
                        tracing::debug!(link = ?link_entity, "input before handshake");
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
                ClientMessage::Interact {
                    entity: target_bits,
                } => {
                    // Взаимодействие (T3.1): цель должна быть дверью в радиусе
                    // INTERACT_RANGE (1.5 тайла) от игрока — сервер-авторитарно.
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player_entity = entry.player;
                    let Some(target) = Entity::try_from_bits(target_bits) else {
                        tracing::warn!(bits = target_bits, "interact: invalid entity bits");
                        continue;
                    };
                    let Ok(player_position) = positions.get(player_entity) else {
                        continue;
                    };
                    let Ok(mut door) = doors.get_mut(target) else {
                        tracing::debug!(target = ?target, "interact: target is not a door");
                        continue;
                    };
                    let dx = door.position[0] - player_position.0[0];
                    let dy = door.position[1] - player_position.0[1];
                    let distance = (dx * dx + dy * dy).sqrt();
                    if distance > INTERACT_RANGE {
                        tracing::warn!(
                            player = ?player_entity,
                            distance,
                            range = INTERACT_RANGE,
                            "interact: too far"
                        );
                        continue;
                    }
                    door.open = !door.open;
                    // Открытая дверь пропускает: коллайдер отключается маркером.
                    if door.open {
                        commands.entity(target).insert(ColliderDisabled);
                    } else {
                        commands.entity(target).remove::<ColliderDisabled>();
                    }
                    tracing::info!(door = ?target, open = door.open, "door toggled");
                }
            }
        }
    }

    for (link_entity, name) in connected {
        // Игрок появляется на следующей точке спавна из файла карты (T2.4).
        let spawn = if map.spawn_points.is_empty() {
            (0.0, 0.0)
        } else {
            let point = map.spawn_points[spawn_cursor.0 % map.spawn_points.len()];
            spawn_cursor.0 += 1;
            point
        };
        tracing::info!(name, spawn = ?spawn, "player spawned");
        // Игровая сущность игрока: динамическое тело (T2.2), позиция реплицируется
        // клиентам из интереса (T1.3/T1.4).
        let player = commands
            .spawn((
                PlayerPosition([spawn.0, spawn.1]),
                PlayerInput::default(),
                Replicate::to_clients(NetworkTarget::All),
                Rooms::default(),
                RigidBody::Dynamic,
                Collider::circle(16.0),
                Position(Vector::new(spawn.0, spawn.1)),
                Rotation::default(),
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

/// Ввод игрока превращается в скорость физического тела (ADR-3: физика на сервере).
/// Физический шаг двигает тело, стены останавливают его коллизией (T2.2).
fn movement(mut players: Query<(&PlayerInput, &mut LinearVelocity)>) {
    for (input, mut velocity) in players.iter_mut() {
        velocity.0 = Vec2::from_array(input.0).normalize_or_zero() * PLAYER_MOVE_SPEED;
    }
}

/// Копирует позицию физического тела в реплицируемый компонент (T2.2):
/// клиент получает подтверждённую сервером позицию.
fn sync_replicated_position(mut players: Query<(&Position, &mut PlayerPosition)>) {
    for (position, mut replicated) in players.iter_mut() {
        replicated.0 = [position.x, position.y];
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
    // По одной сущности в центр своего чанка (ADR-5): игрок видит только 5×5 чанков.
    for i in 0..count {
        let x = ((i % 40) as f32 - 20.0) * CHUNK_UNITS + CHUNK_UNITS / 2.0;
        let y = ((i / 40) as f32 - 12.0) * CHUNK_UNITS + CHUNK_UNITS / 2.0;
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

/// Стены карты (T2.2/T2.3): для каждого чанка — одно статическое тело
/// с составным коллайдером из тайлов-стен загруженной карты.
fn spawn_walls(mut commands: Commands, map: Res<GameMap>) {
    let tile = TILE_SIZE; // 32 юнита на тайл
    let mut total = 0usize;
    for data in &map.chunks {
        let mut shapes = Vec::new();
        for (index, tile_type) in data.tiles.iter().enumerate() {
            if *tile_type != TileType::Wall {
                continue;
            }
            let lx = index as i32 % 32;
            let ly = index as i32 / 32;
            let tx = data.coords.0 * 32 + lx;
            let ty = data.coords.1 * 32 + ly;
            // Центр тайла в мировых координатах (тело чанка стоит в origin).
            let offset = Vector::new((tx as f32 + 0.5) * tile, (ty as f32 + 0.5) * tile);
            shapes.push((offset, Rotation::default(), Collider::rectangle(tile, tile)));
            total += 1;
        }
        if shapes.is_empty() {
            continue;
        }
        commands.spawn((RigidBody::Static, Collider::compound(shapes)));
    }
    tracing::info!(walls = total, "map wall colliders spawned");
}

/// Тест коллизии T2.2: SSR_COLLISION_TEST=1 ставит стену 32×4096 с центром
/// x=704 (блокирует 688..720). Игрок (радиус 16) при автоходе вправо
/// останавливается на x=672 и не проходит сквозь.
fn spawn_collision_test(mut commands: Commands) {
    if std::env::var_os("SSR_COLLISION_TEST").is_none() {
        return;
    }
    commands.spawn((
        RigidBody::Static,
        Collider::rectangle(TILE_SIZE, 4096.0),
        Position(Vector::new(704.0, 0.0)),
        Rotation::default(),
    ));
    tracing::info!("collision test wall spawned at x=704 (blocks 688..720)");
}

/// Счётчик тиков сервера с момента запуска (T0.3).
#[derive(Resource, Default)]
struct TickState {
    tick: u64,
}

/// Раз в секунду пишет позицию игрока (для тестов коллизии, T2.2).
fn log_player_position(time: Res<Time>, mut next_log: Local<f32>, players: Query<&PlayerPosition>) {
    if std::env::var_os("SSR_COLLISION_TEST").is_none() {
        return;
    }
    *next_log += time.delta_secs();
    if *next_log < 1.0 {
        return;
    }
    *next_log = 0.0;
    for position in players.iter() {
        tracing::info!(position = ?position.0, "player position");
    }
}

fn tick_logger(mut state: ResMut<TickState>) {
    state.tick += 1;
    tracing::debug!(tick = state.tick, "server tick");
}
