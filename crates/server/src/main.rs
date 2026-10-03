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
use ssr_core::inventory::{Hands, Health, HeldBy, Inventory, Item, SLOT_ANY};
use ssr_core::tiles::{MapFile, TileChunkData, TileType};
use ssr_core::{
    CHUNK_UNITS, Door, INTERACT_RANGE, PLAYER_MOVE_SPEED, PlayerPosition, TILE_SIZE, chunk_coords,
};
use ssr_protocol::net::GameChannel;
use ssr_protocol::{
    ActionKind, ActionOption, ClientMessage, DEFAULT_SERVER_PORT, PROTOCOL_VERSION, ProtocolPlugin,
    ServerMessage, is_compatible,
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
    app.init_resource::<MapIndex>();
    app.init_resource::<ActionQueue>();
    app.add_systems(Startup, startup);
    app.add_systems(
        Startup,
        (
            check_prototypes,
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
            process_actions,
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
    // Инвентарь и предметы (T3.2). Порядок регистрации обязан совпадать с клиентом!
    app.component::<Inventory>().replicate();
    app.component::<Item>().replicate();
    // Руки/здоровье/удержание (T3.3+). Тот же порядок, что у клиента!
    app.component::<Hands>().replicate();
    app.component::<Health>().replicate();
    app.component::<HeldBy>().replicate();
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

/// Проверяет портированные прототипы (`assets/prototypes_ss14.ron`, IMP.2/IMP.3)
/// при старте — конвейер импорта валидируется в рантайме. Ресурс с прототипами
/// добавится, когда появятся игровые системы фаз 4–5, которые их читают.
fn check_prototypes() {
    let path = ssr_core::assets_root().join("prototypes_ss14.ron");
    match ssr_core::prototypes::ProtoSet::load(&path) {
        Ok(set) => {
            let with_sprite = set.protos.iter().filter(|p| p.sprite.is_some()).count();
            tracing::info!(
                protos = set.protos.len(),
                with_sprite,
                "content prototypes loaded"
            );
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
    mut map_index: ResMut<MapIndex>,
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
        let entity = commands
            .spawn((
                data.clone(),
                Replicate::to_clients(NetworkTarget::All),
                Rooms::single(room),
            ))
            .id();
        map_index.chunks.insert(data.coords, entity);
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

/// Индекс чанков карты: координаты → сущность (для правки тайлов, T3.3)
/// и координаты → сущность коллайдеров чанка (пересобирается при изменениях).
#[derive(Resource, Default)]
struct MapIndex {
    chunks: HashMap<(i32, i32), Entity>,
    colliders: HashMap<(i32, i32), Entity>,
}

/// Очередь действий игроков (T3.3+): и прямые сообщения (Interact/UseItem/Attack),
/// и выбранные из меню verbs. Обрабатывается одной системой, чтобы не нарушать
/// единственную точку чтения сообщений (handle_client_messages).
#[derive(Resource, Default)]
struct ActionQueue(Vec<(Entity, QueuedAction)>);

/// Элемент очереди: выполнить действие или прислать список доступных (verbs).
enum QueuedAction {
    Do(ActionKind),
    RequestActions { entity: u64, tx: i32, ty: i32 },
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
/// Приём сообщений клиента (единственная точка чтения) + операции над руками
/// (SS14-модель): переключить руку, взять из рюкзака, убрать в рюкзак. Всё
/// исполняемое (атака, двери, применение предметов, verbs) уходит в очередь
/// [`ActionQueue`] и обрабатывается одной системой [`process_actions`].
#[allow(clippy::too_many_arguments)]
fn handle_client_messages(
    mut commands: Commands,
    map: Res<GameMap>,
    mut spawn_cursor: ResMut<SpawnCursor>,
    mut receivers: Query<(Entity, &RemoteId, &mut MessageReceiver<ClientMessage>), With<Connected>>,
    mut senders: Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
    mut inputs: Query<&mut PlayerInput>,
    positions: Query<&PlayerPosition>,
    mut inventories: Query<&mut Inventory>,
    mut hands: Query<&mut Hands>,
    mut actions: ResMut<ActionQueue>,
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
                        tracing::debug!(link = ?link_entity, "input before handshake");
                        continue;
                    };
                    match inputs.get_mut(entry.player) {
                        Ok(mut input) => input.0 = movement,
                        Err(e) => tracing::warn!(error = %e, "no PlayerInput"),
                    }
                }
                ClientMessage::TransferItem {
                    item,
                    to_slot,
                    target_player,
                } => {
                    let Some(sender_entry) = players.entry_by_link_mut(link_entity.to_bits())
                    else {
                        continue;
                    };
                    let sender_player = sender_entry.player;
                    let Ok(sender_position) = positions.get(sender_player) else {
                        continue;
                    };
                    let receiver_player = if target_player == 0 {
                        sender_player
                    } else {
                        let Some(target) = Entity::try_from_bits(target_player) else {
                            tracing::warn!(bits = target_player, "transfer: invalid target bits");
                            continue;
                        };
                        if inventories.get(target).is_err() {
                            tracing::warn!(target = ?target, "transfer: target has no inventory");
                            continue;
                        }
                        let Ok(target_position) = positions.get(target) else {
                            continue;
                        };
                        let dx = target_position.0[0] - sender_position.0[0];
                        let dy = target_position.0[1] - sender_position.0[1];
                        if (dx * dx + dy * dy).sqrt() > INTERACT_RANGE + 32.0 {
                            tracing::warn!(target = ?target, "transfer: too far");
                            continue;
                        }
                        target
                    };
                    if receiver_player == sender_player {
                        let Ok(mut inventory) = inventories.get_mut(sender_player) else {
                            continue;
                        };
                        if !inventory.contains(item) {
                            tracing::warn!(item, "transfer: item is not in sender inventory");
                            continue;
                        }
                        let Some(from) = inventory.slot_of(item) else {
                            continue;
                        };
                        if to_slot == SLOT_ANY || to_slot == from {
                            continue;
                        }
                        inventory.take(item);
                        if !inventory.put(to_slot, item) {
                            inventory.put(from, item);
                            tracing::warn!(slot = to_slot, "transfer: slot busy");
                            continue;
                        }
                        tracing::info!(item, from, to = to_slot, "item moved");
                        continue;
                    }
                    let Ok([mut sender_inventory, mut receiver_inventory]) =
                        inventories.get_many_mut([sender_player, receiver_player])
                    else {
                        continue;
                    };
                    if !sender_inventory.contains(item) {
                        tracing::warn!(item, "transfer: item is not in sender inventory");
                        continue;
                    }
                    let slot = if to_slot == SLOT_ANY {
                        receiver_inventory.first_empty()
                    } else {
                        Some(to_slot)
                    };
                    let Some(slot) = slot else {
                        tracing::warn!(target = ?receiver_player, "transfer: no free slot");
                        continue;
                    };
                    if !receiver_inventory.put(slot, item) {
                        tracing::warn!(slot, "transfer: slot busy");
                        continue;
                    }
                    sender_inventory.take(item);
                    tracing::info!(item, slot, from = ?sender_player, to = ?receiver_player, "item transferred");
                }
                ClientMessage::SwitchHand => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    if let Ok(mut hand) = hands.get_mut(entry.player) {
                        hand.switch();
                        tracing::info!(active = hand.active, "hand switched");
                    }
                }
                ClientMessage::TakeInHand { slot } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let Ok(mut inventory) = inventories.get_mut(player) else {
                        continue;
                    };
                    let Ok(mut hand) = hands.get_mut(player) else {
                        continue;
                    };
                    let Some(item) = inventory.slots.get(slot as usize).copied().flatten() else {
                        continue;
                    };
                    inventory.take(item);
                    if !hand.take_in_active(item) {
                        inventory.put(slot, item);
                        tracing::warn!(slot, "take in hand: active hand busy");
                        continue;
                    }
                    tracing::info!(item, hand = hand.active, "item taken in hand");
                }
                ClientMessage::MoveHandToInventory { item } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let Ok(mut inventory) = inventories.get_mut(player) else {
                        continue;
                    };
                    let Ok(mut hand) = hands.get_mut(player) else {
                        continue;
                    };
                    if !hand.take(item) {
                        continue;
                    }
                    match inventory.put_first_empty(item) {
                        Some(slot) => tracing::info!(item, slot, "item stowed"),
                        None => {
                            hand.take_in_active(item);
                            tracing::warn!(item, "stow: inventory full");
                        }
                    }
                }
                ClientMessage::DropHand => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let Ok(mut hand) = hands.get_mut(player) else {
                        continue;
                    };
                    let Some(item) = hand.active_item() else {
                        continue;
                    };
                    hand.take(item);
                    let Ok(mut inventory) = inventories.get_mut(player) else {
                        hand.take_in_active(item);
                        continue;
                    };
                    match inventory.put_first_empty(item) {
                        Some(slot) => tracing::info!(item, slot, "item stowed from hand"),
                        None => {
                            hand.take_in_active(item);
                            tracing::warn!(item, "stow: inventory full");
                        }
                    }
                }
                ClientMessage::Attack { target } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    actions.0.push((
                        entry.player,
                        QueuedAction::Do(ActionKind::Attack { target }),
                    ));
                }
                ClientMessage::UseItem { item, tx, ty } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    actions.0.push((
                        entry.player,
                        QueuedAction::Do(ActionKind::UseItem { item, tx, ty }),
                    ));
                }
                ClientMessage::Interact { entity } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    actions.0.push((
                        entry.player,
                        QueuedAction::Do(ActionKind::Interact { entity }),
                    ));
                }
                ClientMessage::RequestActions { entity, tx, ty } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    actions.0.push((
                        entry.player,
                        QueuedAction::RequestActions { entity, tx, ty },
                    ));
                }
                ClientMessage::PerformAction { action } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    actions.0.push((entry.player, QueuedAction::Do(action)));
                }
            }
        }
    }

    for (link_entity, name) in connected {
        // Точка спавна — до создания сущности (id игрока сразу известен и нужен
        // демо-предметам: commands отложены, но id уже зарезервирован).
        let spawn = if map.spawn_points.is_empty() {
            (0.0, 0.0)
        } else {
            let point = map.spawn_points[spawn_cursor.0 % map.spawn_points.len()];
            spawn_cursor.0 += 1;
            point
        };
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
                Hands::default(),
                Health::default(),
            ))
            .id();
        let player_bits = player.to_bits();
        tracing::info!(name, spawn = ?spawn, "player spawned");

        // Демо-предметы: лом + два стальных листа; HeldBy — на игрока (SS14-модель).
        let mut inventory = Inventory::default();
        for item_name in ["Crowbar", "SteelSheet", "SteelSheet"] {
            let item = commands
                .spawn((
                    Item {
                        name: item_name.to_string(),
                    },
                    HeldBy {
                        player: player_bits,
                    },
                    Replicate::to_clients(NetworkTarget::All),
                    Rooms::default(),
                ))
                .id();
            inventory.put_first_empty(item.to_bits());
        }
        commands.entity(player).insert(inventory);
        players.entries.push(PlayerEntry {
            link: link_entity,
            name,
            player,
            chunk: (0, 0),
            rooms: Vec::new(),
        });

        if let Ok((_, mut sender)) = senders.get_mut(link_entity) {
            sender.send::<GameChannel>(ServerMessage::Welcome {
                player_entity: player_bits,
                protocol_version: PROTOCOL_VERSION,
            });
        }
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

/// Статическое тело-коллайдер одного чанка (стены тайлами, T2.2/T3.3).
/// Возвращает None, если в чанке нет стен.
fn spawn_chunk_collider(commands: &mut Commands, data: &TileChunkData) -> Option<Entity> {
    let tile = TILE_SIZE; // 32 юнита на тайл
    let mut shapes = Vec::new();
    for (index, tile_type) in data.tiles.iter().enumerate() {
        if *tile_type != TileType::Wall {
            continue;
        }
        let lx = index as i32 % 32;
        let ly = index as i32 / 32;
        let tx = data.coords.0 * 32 + lx;
        let ty = data.coords.1 * 32 + ly;
        let offset = Vector::new((tx as f32 + 0.5) * tile, (ty as f32 + 0.5) * tile);
        shapes.push((offset, Rotation::default(), Collider::rectangle(tile, tile)));
    }
    if shapes.is_empty() {
        return None;
    }
    Some(
        commands
            .spawn((RigidBody::Static, Collider::compound(shapes)))
            .id(),
    )
}

/// Стены карты (T2.2/T2.3/T3.3): по телу на чанк, индекс — для пересборки при стройке.
fn spawn_walls(mut commands: Commands, map: Res<GameMap>, mut index: ResMut<MapIndex>) {
    let mut total = 0usize;
    for data in &map.chunks {
        if let Some(entity) = spawn_chunk_collider(&mut commands, data) {
            index.colliders.insert(data.coords, entity);
        }
        total += data.tiles.iter().filter(|t| **t == TileType::Wall).count();
    }
    tracing::info!(walls = total, "map wall colliders spawned");
}

/// Обрабатывает очередь действий (T3.3+): атака, двери, применение предметов
/// из активной руки и выдача списка контекстных действий (verbs).
#[allow(clippy::too_many_arguments)]
fn process_actions(
    mut commands: Commands,
    mut queue: ResMut<ActionQueue>,
    mut index: ResMut<MapIndex>,
    mut chunks: Query<&mut TileChunkData>,
    mut bodies: Query<&mut Position>,
    positions: Query<&PlayerPosition>,
    mut inventories: Query<&mut Inventory>,
    mut hands: Query<&mut Hands>,
    mut healths: Query<&mut Health>,
    mut doors: Query<&mut Door>,
    items: Query<&Item>,
    mut senders: Query<&mut MessageSender<ServerMessage>, With<Connected>>,
    map: Res<GameMap>,
    players: Res<Players>,
) {
    if queue.0.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut queue.0);
    for (player, queued) in pending {
        let action = match queued {
            QueuedAction::Do(action) => action,
            QueuedAction::RequestActions { entity, tx, ty } => {
                let mut options: Vec<ActionOption> = Vec::new();
                if entity != 0 {
                    if let Some(target) = Entity::try_from_bits(entity) {
                        if healths.get(target).is_ok() {
                            options.push(ActionOption {
                                label: "Ударить".into(),
                                action: ActionKind::Attack { target: entity },
                            });
                        }
                        if let Ok(door) = doors.get_mut(target) {
                            options.push(ActionOption {
                                label: if door.open {
                                    "Закрыть"
                                } else {
                                    "Открыть"
                                }
                                .into(),
                                action: ActionKind::Interact { entity },
                            });
                        }
                    }
                } else {
                    let hand_item = hands.get(player).ok().and_then(|h| h.active_item());
                    let item_name = hand_item
                        .and_then(Entity::try_from_bits)
                        .and_then(|e| items.get(e).ok())
                        .map(|i| i.name.clone())
                        .unwrap_or_default();
                    let chunk_tiles = ssr_core::tiles::CHUNK_TILES as i32;
                    let tile = index
                        .chunks
                        .get(&(tx.div_euclid(chunk_tiles), ty.div_euclid(chunk_tiles)))
                        .and_then(|e| chunks.get(*e).ok())
                        .map(|c| {
                            let lx = (tx - tx.div_euclid(chunk_tiles) * chunk_tiles) as usize;
                            let ly = (ty - ty.div_euclid(chunk_tiles) * chunk_tiles) as usize;
                            c.tiles[ly * ssr_core::tiles::CHUNK_TILES as usize + lx]
                        });
                    if let (Some(item), Some(tile)) = (hand_item, tile) {
                        if item_name == "SteelSheet" && tile == TileType::Floor {
                            options.push(ActionOption {
                                label: "Построить стену".into(),
                                action: ActionKind::UseItem { item, tx, ty },
                            });
                        }
                        if item_name == "Crowbar" && tile == TileType::Wall {
                            options.push(ActionOption {
                                label: "Разобрать стену".into(),
                                action: ActionKind::UseItem { item, tx, ty },
                            });
                        }
                    }
                }
                if !options.is_empty()
                    && let Some(link) = players
                        .entries
                        .iter()
                        .find(|e| e.player == player)
                        .map(|e| e.link)
                    && let Ok(mut sender) = senders.get_mut(link)
                {
                    sender.send::<GameChannel>(ServerMessage::Actions { options });
                }
                continue;
            }
        };
        match action {
            ActionKind::Attack { target } => {
                let Some(target_entity) = Entity::try_from_bits(target) else {
                    continue;
                };
                let Ok(player_position) = positions.get(player) else {
                    continue;
                };
                let Ok(target_position) = positions.get(target_entity) else {
                    tracing::warn!(?target_entity, "attack: target is not a player");
                    continue;
                };
                let dx = target_position.0[0] - player_position.0[0];
                let dy = target_position.0[1] - player_position.0[1];
                if (dx * dx + dy * dy).sqrt() > INTERACT_RANGE + 32.0 {
                    tracing::warn!(?target_entity, "attack: too far");
                    continue;
                }
                let Ok(mut health) = healths.get_mut(target_entity) else {
                    tracing::warn!(?target_entity, "attack: target has no health");
                    continue;
                };
                // Урон по предмету в активной руке: лом — 15, кулак — 5 (T4.1-мини).
                let weapon = hands
                    .get(player)
                    .ok()
                    .and_then(|h| h.active_item())
                    .and_then(Entity::try_from_bits)
                    .and_then(|e| items.get(e).ok())
                    .map(|i| i.name.clone());
                let damage = if weapon.as_deref() == Some("Crowbar") {
                    15
                } else {
                    5
                };
                let dead = health.damage(damage);
                tracing::info!(
                    ?player,
                    ?target_entity,
                    damage,
                    hp = health.current,
                    "attack hit"
                );
                if dead {
                    health.current = health.max;
                    let spawn = map.spawn_points.first().copied().unwrap_or((0.0, 0.0));
                    if let Ok(mut body) = bodies.get_mut(target_entity) {
                        body.0 = Vector::new(spawn.0, spawn.1);
                    }
                    tracing::info!(?target_entity, "player died and respawned");
                }
            }
            ActionKind::Interact { entity } => {
                let Some(target) = Entity::try_from_bits(entity) else {
                    tracing::warn!(bits = entity, "interact: invalid entity bits");
                    continue;
                };
                let Ok(player_position) = positions.get(player) else {
                    continue;
                };
                let Ok(mut door) = doors.get_mut(target) else {
                    tracing::debug!(target = ?target, "interact: target is not a door");
                    continue;
                };
                let dx = door.position[0] - player_position.0[0];
                let dy = door.position[1] - player_position.0[1];
                if (dx * dx + dy * dy).sqrt() > INTERACT_RANGE {
                    tracing::warn!(?player, "interact: too far");
                    continue;
                }
                door.open = !door.open;
                if door.open {
                    commands.entity(target).insert(ColliderDisabled);
                } else {
                    commands.entity(target).remove::<ColliderDisabled>();
                }
                tracing::info!(door = ?target, open = door.open, "door toggled");
            }
            ActionKind::UseItem { item, tx, ty } => {
                let Ok(player_position) = positions.get(player) else {
                    continue;
                };
                let center =
                    Vec2::new((tx as f32 + 0.5) * TILE_SIZE, (ty as f32 + 0.5) * TILE_SIZE);
                let player_pos = Vec2::from_array(player_position.0);
                if center.distance(player_pos) > INTERACT_RANGE + TILE_SIZE {
                    tracing::warn!(tx, ty, "use item: too far");
                    continue;
                }
                let chunk_tiles = ssr_core::tiles::CHUNK_TILES as i32;
                let chunk_coords = (tx.div_euclid(chunk_tiles), ty.div_euclid(chunk_tiles));
                let Some(&chunk_entity) = index.chunks.get(&chunk_coords) else {
                    tracing::warn!(?chunk_coords, "use item: no chunk");
                    continue;
                };
                let Ok(mut chunk) = chunks.get_mut(chunk_entity) else {
                    continue;
                };
                let lx = (tx - chunk_coords.0 * chunk_tiles) as usize;
                let ly = (ty - chunk_coords.1 * chunk_tiles) as usize;
                let cell = ly * ssr_core::tiles::CHUNK_TILES as usize + lx;

                // Предмет обязан быть в АКТИВНОЙ руке (SS14-модель).
                let in_hand = hands
                    .get(player)
                    .ok()
                    .and_then(|h| h.active_item())
                    .is_some_and(|hand_item| hand_item == item);
                if !in_hand {
                    tracing::warn!(item, "use item: not in active hand");
                    continue;
                }
                let item_name = Entity::try_from_bits(item)
                    .and_then(|e| items.get(e).ok())
                    .map(|i| i.name.clone())
                    .unwrap_or_default();

                if item_name == "SteelSheet" {
                    if chunk.tiles[cell] != TileType::Floor {
                        tracing::warn!(tx, ty, "build: tile is not empty floor");
                        continue;
                    }
                    // Запрет стройки на тайле, где стоит любой игрок (в т.ч. сам).
                    let occupied = positions.iter().any(|p| {
                        (
                            (p.0[0] / TILE_SIZE).floor() as i32,
                            (p.0[1] / TILE_SIZE).floor() as i32,
                        ) == (tx, ty)
                    });
                    if occupied {
                        tracing::warn!(tx, ty, "build: tile occupied by a player");
                        continue;
                    }
                    let Ok(mut hand) = hands.get_mut(player) else {
                        continue;
                    };
                    hand.take(item);
                    if let Some(entity) = Entity::try_from_bits(item) {
                        commands.entity(entity).despawn();
                    }
                    chunk.tiles[cell] = TileType::Wall;
                    tracing::info!(tx, ty, "tile built");
                } else if item_name == "Crowbar" {
                    if chunk.tiles[cell] != TileType::Wall {
                        tracing::warn!(tx, ty, "deconstruct: tile is not a wall");
                        continue;
                    }
                    chunk.tiles[cell] = TileType::Floor;
                    if let Ok(mut inventory) = inventories.get_mut(player) {
                        let sheet = commands
                            .spawn((
                                Item {
                                    name: "SteelSheet".to_string(),
                                },
                                HeldBy {
                                    player: player.to_bits(),
                                },
                                Replicate::to_clients(NetworkTarget::All),
                                Rooms::default(),
                            ))
                            .id();
                        let sheet_bits = sheet.to_bits();
                        if let Some(slot) = inventory.put_first_empty(sheet_bits) {
                            tracing::info!(slot, "deconstruct: material returned");
                        }
                    }
                    tracing::info!(tx, ty, "tile destroyed");
                } else {
                    tracing::warn!(item, name = %item_name, "use item: unknown item");
                    continue;
                }

                if let Some(old) = index.colliders.remove(&chunk_coords) {
                    commands.entity(old).despawn();
                }
                if let Some(new_entity) = spawn_chunk_collider(&mut commands, &chunk) {
                    index.colliders.insert(chunk_coords, new_entity);
                }
            }
        }
    }
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
