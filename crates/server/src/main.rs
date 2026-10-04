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
    Collider, ColliderDisabled, Friction, Gravity, LinearVelocity, PhysicsPlugins, Position,
    RigidBody, Rotation,
};
use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use lightyear::connection::server::Start;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use ssr_core::atmosphere::Gas;
use ssr_core::clothing::{Clothing, ClothingSlot};
use ssr_core::inventory::{
    Container, Hands, Health, HeldBy, Inventory, Item, ItemPosition, SLOT_ANY,
};
use ssr_core::mechanics::{FacialHair, Hair, Sex, facial_hair_style_names, hair_style_names};
use ssr_core::mechanics::{Ghost, KNOCKDOWN_SECS, KnockedDown};
use ssr_core::power::{Cable, Consumer, Generator, Light, Powered};
use ssr_core::roles::{Access, PlayerRole, RoleSet};
use ssr_core::tiles::{MapFile, TileChunkData, TileType};
use ssr_core::{
    CHUNK_UNITS, Door, INTERACT_RANGE, PLAYER_ACCEL, PLAYER_FRICTION_IDLE, PLAYER_MOVE_SPEED,
    PLAYER_WALK_SPEED, PlayerPosition, Species, TILE_SIZE, chunk_coords,
};
use ssr_protocol::net::GameChannel;
use ssr_protocol::{
    ActionKind, ActionOption, ChatChannel, ClientMessage, DEFAULT_SERVER_PORT, PROTOCOL_VERSION,
    ProtocolPlugin, ServerMessage, is_compatible,
};
use tracing_subscriber::EnvFilter;

/// Тик-рейт сервера (PLAN.md T0.3).
const TPS: f64 = 20.0;

/// Радиус интереса в чанках (PLAN.md T1.4): клиент получает сущности в квадрате 5×5 чанков.
const INTEREST_RADIUS: i32 = 2;

/// Число сущностей нагрузочного теста (критерий T1.4: клиент получает < 100 из 1000).
const LOAD_TEST_ENTITIES: u32 = 1000;

/// Радиус коллайдера игрока: чуть меньше половины тайла, чтобы проход шириной
/// в 1 тайл не тёрся о стены (из-за этого было замедление в коридоре).
const PLAYER_RADIUS: f32 = 14.0;

/// Адрес, который слушает сервер. Порт переопределяется `SSR_PORT` — тестовые
/// прогоны идут на отдельном порту и не перехватывают живую игру в 7777.
fn server_addr() -> SocketAddr {
    let port = std::env::var("SSR_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_SERVER_PORT);
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

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
    // Метрики транспорта (T6.1): копим байты/пакеты для нагрузочного лога.
    // Регистр — глобальный GLOBAL_RECORDER, из него читает perf_logger.
    app.add_plugins(lightyear::metrics::prelude::MetricsPlugin::with_registry(
        lightyear::metrics::prelude::GLOBAL_RECORDER.clone(),
    ));
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
    app.init_resource::<ChatCooldowns>();
    app.init_resource::<SpawnCursor>();
    app.init_resource::<MapIndex>();
    app.init_resource::<ActionQueue>();
    app.init_resource::<GameRoles>();
    app.init_resource::<RoleCursor>();
    app.init_resource::<Atmospheres>();
    app.init_resource::<ContentCatalog>();
    app.init_resource::<NetStats>();
    app.add_systems(Startup, startup);
    app.add_systems(
        Startup,
        (
            check_prototypes,
            load_roles,
            load_content,
            load_map,
            spawn_power,
            init_atmosphere,
            spawn_walls,
            spawn_containers,
            spawn_load_test,
            spawn_collision_test,
        )
            .chain(),
    );
    app.add_message::<DamageEvent>();
    app.add_message::<DeathEvent>();
    app.add_systems(
        Update,
        (
            tick_logger,
            handle_client_messages,
            // Урон применяется сразу после разбора очереди действий (T4.1).
            (process_actions, apply_damage, respawn_dead).chain(),
            movement,
            sync_replicated_position,
            update_client_rooms,
            sync_item_rooms,
            log_player_position,
            // Атмосфера (T4.3): диффузия, урон от разгерметизации.
            simulate_atmosphere,
            suffocation,
            auto_doors,
            knockdown_tick,
            power_grid,
        ),
    );
    // Тест-режимы и нагрузочный лог (SSR_*_TEST / SSR_PERF_LOG) — отдельно.
    app.add_systems(
        Update,
        (
            damage_test,
            power_test,
            breach_test,
            vacuum_test,
            log_atmosphere,
            perf_logger,
        ),
    );
    app.add_observer(teleport_to_player);
    app.add_observer(on_link_connected);
    app.add_observer(on_link_disconnected);
    // Регистрация реплицируемых компонентов — общий список в ssr_protocol::net:
    // порядок обязан совпадать с клиентом, иначе «Hit the end of buffer».
    ssr_protocol::net::register_replication(&mut app);
    app.run();
}

/// Сетевой сервер: одна сущность-линк на клиента.
fn startup(mut commands: Commands) -> Result {
    // RawServer: идентификация клиента по адресу (netcode/авторизация — позже, с привязкой к сайту).
    let addr = server_addr();
    let server = commands
        .spawn((RawServer, LocalAddr(addr), ServerUdpIo::default()))
        .id();
    commands.trigger(Start { entity: server });
    tracing::info!(%addr, "server listening");
    Ok(())
}

/// Ящики (T3.4): пара контейнеров рядом со спавн-точками с запасом предметов.
/// Стоят в мире (Container + Inventory + ItemPosition), состояние — серверное.
fn spawn_containers(
    mut commands: Commands,
    map: Res<GameMap>,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
) {
    let Some(&(sx, sy)) = map.spawn_points.first() else {
        return;
    };
    // Смещения в тайлах от первой точки спавна (пол в стартовом зале).
    let spots = [(sx + 64.0, sy), (sx + 64.0, sy + 64.0)];
    for (index, (x, y)) in spots.iter().enumerate() {
        let mut inventory = Inventory::default();
        // Разное содержимое: лом и листы (индекс — для разнообразия).
        let names: [&str; 2] = if index == 0 {
            ["Crowbar", "SteelSheet"]
        } else {
            ["SteelSheet", "SteelSheet"]
        };
        let mut items = Vec::new();
        for name in names {
            items.push((
                commands
                    .spawn((
                        Item {
                            name: name.to_string(),
                        },
                        HeldBy { player: 0 },
                        Replicate::to_clients(NetworkTarget::All),
                        Rooms::default(),
                    ))
                    .id()
                    .to_bits(),
                ssr_core::inventory::item_size(name),
            ));
        }
        for (item, (w, h)) in items {
            if inventory.put_first_fit(item, w, h).is_none() {
                tracing::warn!("container: no room for item");
            }
        }
        let room = chunk_rooms.room_for(chunk_coords(*x, *y), &mut allocator);
        commands.spawn((
            Container {
                open: false,
                name: format!("Ящик {}", index + 1),
            },
            inventory,
            ItemPosition([*x, *y]),
            Replicate::to_clients(NetworkTarget::All),
            ItemRoom(room),
            Rooms::single(room),
        ));
    }
    tracing::info!(count = spots.len(), "containers spawned");
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
    /// Электрика из файла карты (T4.4).
    cables: Vec<(f32, f32)>,
    generators: Vec<(f32, f32, f32)>,
    lights: Vec<(f32, f32)>,
}

/// Курсор выдачи точек спавна (T2.4): каждый новый игрок получает следующую
/// точку по кругу, чтобы игроки не появлялись друг в друге.
#[derive(Resource, Default)]
struct SpawnCursor(usize);

/// Роли (T4.2): прототипы из `assets/prototypes/roles.ron`.
#[derive(Resource, Default)]
struct GameRoles(RoleSet);

/// Ресурсы контента и статистики для обработки сообщений (T5.2/T6.1):
/// одним параметром — у функций-систем лимит 16 параметров.
#[derive(bevy::ecs::system::SystemParam)]
struct ServerContent<'w> {
    roles: Res<'w, GameRoles>,
    role_cursor: ResMut<'w, RoleCursor>,
    catalogs: Res<'w, ContentCatalog>,
    stats: ResMut<'w, NetStats>,
}

/// Запрос телепорта к другому игроку (админ-команда tpto, T-мех).
#[derive(Event)]
struct TeleportToPlayer {
    player: Entity,
    target: Entity,
}

/// Исполняет телепорт к игроку (позицию цели знает система с доступом к телам).
fn teleport_to_player(
    trigger: On<TeleportToPlayer>,
    positions: Query<&PlayerPosition>,
    mut bodies: Query<(&mut Position, &mut LinearVelocity)>,
) {
    let event = trigger.event();
    let Ok(target_position) = positions.get(event.target) else {
        return;
    };
    if let Ok((mut body, mut velocity)) = bodies.get_mut(event.player) {
        body.0 = Vector::new(target_position.0[0], target_position.0[1]);
        velocity.0 = Vector::ZERO;
        tracing::info!(
            player = ?event.player,
            target = ?event.target,
            "admin tpto applied"
        );
    }
}

/// Отложенный респавн: игрок лежит и ждёт возврата на спавн (механики).
#[derive(Component, Default)]
struct RespawnPending;

/// Счётчики входящих сообщений (T6.1): сколько клиентских сообщений приняли.
#[derive(Resource, Default)]
struct NetStats {
    messages_in: u64,
}

/// Админы сервера (T5.5): имена через запятую в `SSR_ADMINS`
/// (пусто — админ-команд нет; `SSR_OPEN_ADMIN=1` — все админы, для тестов).
fn is_admin(name: &str) -> bool {
    if std::env::var_os("SSR_OPEN_ADMIN").is_some() {
        return true;
    }
    let Ok(list) = std::env::var("SSR_ADMINS") else {
        return false;
    };
    list.split(',')
        .map(|entry| entry.trim())
        .any(|entry| !entry.is_empty() && entry == name)
}

/// Каталоги контента (T5.2): предметы и рецепты из assets/prototypes.
#[derive(Resource, Default)]
struct ContentCatalog {
    items: ssr_core::items::ItemSet,
    recipes: ssr_core::recipes::RecipeSet,
}

/// Загружает каталоги предметов и рецептов (T5.2).
fn load_content(mut commands: Commands) {
    let root = ssr_core::assets_root().join("prototypes");
    let items = ssr_core::items::ItemSet::load(&root.join("items.ron"));
    let recipes = ssr_core::recipes::RecipeSet::load(&root.join("recipes.ron"));
    match (items, recipes) {
        (Ok(items), Ok(recipes)) => {
            tracing::info!(
                items = items.items.len(),
                recipes = recipes.recipes.len(),
                "content catalog loaded"
            );
            commands.insert_resource(ContentCatalog { items, recipes });
        }
        (items, recipes) => {
            if let Err(e) = items {
                tracing::error!(error = %e, "items.ron not loaded");
            }
            if let Err(e) = recipes {
                tracing::error!(error = %e, "recipes.ron not loaded");
            }
        }
    }
}

/// Шаг симуляции атмосферы, секунды (T4.3): 5 раз в секунду достаточно.
const ATMOS_STEP_SECS: f32 = 0.2;
/// Доля выравнивания давления между соседними тайлами за шаг.
/// Внимание: явная схема с 4 соседями устойчива только при K <= 0.25.
const DIFFUSION_K: f32 = 0.2;

/// Автоматика двери (как в SS14): открывается при подходе игрока с доступом,
/// закрывается через [`AUTO_CLOSE_SECS`] после того, как рядом никого не осталось.
const AUTO_DOOR_RANGE: f32 = 44.0;
const AUTO_CLOSE_SECS: f32 = 4.0;

/// Таймер автозакрытия двери (только сервер, не реплицируется).
#[derive(Component)]
struct DoorAuto {
    close_in: f32,
}

/// Атмосфера мира (T4.3, lite): единая сетка газа по тайлам карты.
/// Хранится плоско (мир маленький), а клиенту отдаётся по чанкам
/// реплицируемыми компонентами [`ChunkAtmosphere`].
#[derive(Resource, Default)]
struct Atmospheres {
    /// Левый нижний тайл сетки.
    min: (i32, i32),
    /// Размер сетки в тайлах.
    size: (usize, usize),
    /// Газ по тайлам: индекс = y * width + x.
    gas: Vec<Gas>,
    /// Типы тайлов (правила обмена: стена не пропускает, космос — сток).
    tiles: Vec<TileType>,
    /// (координаты чанка) → сущность реплицируемой атмосферы.
    entities: HashMap<(i32, i32), Entity>,
}

impl Atmospheres {
    fn index(&self, tx: i32, ty: i32) -> Option<usize> {
        let (w, h) = self.size;
        let x = tx - self.min.0;
        let y = ty - self.min.1;
        if x < 0 || y < 0 || x >= w as i32 || y >= h as i32 {
            return None;
        }
        Some(y as usize * w + x as usize)
    }

    /// Газ тайла по мировым координатам тайла.
    fn gas_at_tile(&self, tx: i32, ty: i32) -> Option<Gas> {
        self.index(tx, ty).and_then(|i| self.gas.get(i)).copied()
    }

    /// Газ по мировым координатам в юнитах (для урона и HUD).
    fn gas_at_units(&self, x: f32, y: f32) -> Option<Gas> {
        self.gas_at_tile(
            (x / TILE_SIZE).floor() as i32,
            (y / TILE_SIZE).floor() as i32,
        )
    }
}

/// Курсор выдачи ролей (T4.2): round-robin, чтобы в раунде были разные роли.
#[derive(Resource, Default)]
struct RoleCursor(usize);

/// Потребители сети: позиция, потребление, питание.
type GridConsumers<'w, 's> = Query<
    'w,
    's,
    (
        &'static ItemPosition,
        &'static Consumer,
        &'static mut Powered,
    ),
    (Without<Generator>, Without<Cable>),
>;
/// Генераторы сети.
type GridGenerators<'w, 's> = Query<
    'w,
    's,
    (
        &'static ItemPosition,
        &'static Generator,
        &'static mut Powered,
    ),
    (Without<Consumer>, Without<Cable>),
>;

/// Спавнит электрику (T4.4): кабели, генераторы, лампы из файла карты.
fn spawn_power(
    mut commands: Commands,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
    map: Res<GameMap>,
) {
    for &(x, y) in &map.cables {
        let room = chunk_rooms.room_for(chunk_coords(x, y), &mut allocator);
        commands.spawn((
            Cable,
            ItemPosition([x, y]),
            Replicate::to_clients(NetworkTarget::All),
            Rooms::single(room),
        ));
    }
    for &(x, y, power_kw) in &map.generators {
        let room = chunk_rooms.room_for(chunk_coords(x, y), &mut allocator);
        commands.spawn((
            Generator { power_kw },
            Powered(true),
            ItemPosition([x, y]),
            Replicate::to_clients(NetworkTarget::All),
            Rooms::single(room),
        ));
    }
    for &(x, y) in &map.lights {
        let room = chunk_rooms.room_for(chunk_coords(x, y), &mut allocator);
        commands.spawn((
            Light::default(),
            Consumer {
                draw_kw: ssr_core::power::LIGHT_DRAW_KW,
            },
            Powered(false),
            ItemPosition([x, y]),
            Replicate::to_clients(NetworkTarget::All),
            Rooms::single(room),
        ));
    }
    tracing::info!(
        cables = map.cables.len(),
        generators = map.generators.len(),
        lights = map.lights.len(),
        "power spawned"
    );
}

/// Считает энергобаланс сетей (T4.4): кабели, соединённые по 4 сторонам,
/// образуют сеть; питание есть, если выработка покрывает потребление.
/// Карты без генераторов считаются запитанными (страховка для импорта).
fn power_grid(
    time: Res<Time>,
    mut next_step: Local<f32>,
    mut last_summary: Local<String>,
    mut consumers: GridConsumers,
    mut generators: GridGenerators,
    cables: Query<&ItemPosition, With<Cable>>,
) {
    *next_step += time.delta_secs();
    if *next_step < 1.0 {
        return;
    }
    *next_step = 0.0;

    let tile_of = |position: &[f32; 2]| {
        (
            (position[0] / TILE_SIZE) as i32,
            (position[1] / TILE_SIZE) as i32,
        )
    };
    let cable_tiles: HashMap<(i32, i32), ()> = cables
        .iter()
        .map(|position| (tile_of(&position.0), ()))
        .collect();

    if generators.iter().next().is_none() {
        for (_, _, mut powered) in consumers.iter_mut() {
            powered.0 = true;
        }
        return;
    }

    // Сети: BFS по кабельным тайлам.
    let mut visited: HashMap<(i32, i32), usize> = HashMap::new();
    let mut networks: Vec<Vec<(i32, i32)>> = Vec::new();
    for &tile in cable_tiles.keys() {
        if visited.contains_key(&tile) {
            continue;
        }
        let index = networks.len();
        let mut stack = vec![tile];
        let mut network = Vec::new();
        while let Some(current) = stack.pop() {
            if visited.insert(current, index).is_some() {
                continue;
            }
            network.push(current);
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let next = (current.0 + dx, current.1 + dy);
                if cable_tiles.contains_key(&next) && !visited.contains_key(&next) {
                    stack.push(next);
                }
            }
        }
        networks.push(network);
    }

    let mut supply: Vec<f32> = vec![0.0; networks.len()];
    let mut demand: Vec<f32> = vec![0.0; networks.len()];
    for (position, generator, _) in generators.iter() {
        if let Some(&index) = visited.get(&tile_of(&position.0)) {
            supply[index] += generator.power_kw;
        }
    }
    for (position, consumer, _) in consumers.iter() {
        if let Some(&index) = visited.get(&tile_of(&position.0)) {
            demand[index] += consumer.draw_kw;
        }
    }

    let mut summary = String::new();
    for (index, network) in networks.iter().enumerate() {
        let powered = supply[index] >= demand[index] && supply[index] > 0.0;
        summary.push_str(&format!(
            "[{} кабелей: {:.0}/{:.0} кВт{}] ",
            network.len(),
            supply[index],
            demand[index],
            if powered { "" } else { " ОБЕСТОЧЕНО" }
        ));
        for (position, _, mut state) in generators.iter_mut() {
            if visited.get(&tile_of(&position.0)) == Some(&index) {
                state.0 = powered;
            }
        }
        for (position, _, mut state) in consumers.iter_mut() {
            if visited.get(&tile_of(&position.0)) == Some(&index) {
                state.0 = powered;
            }
        }
    }
    for (position, _, mut state) in consumers.iter_mut() {
        if !visited.contains_key(&tile_of(&position.0)) {
            state.0 = false;
        }
    }
    if *last_summary != summary {
        *last_summary = summary.clone();
        tracing::info!(grid = %summary, "power grid");
    }
}

/// Тест T4.4: SSR_POWER_TEST=1 — через 5 секунд обесточивает генераторы.
fn power_test(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut done: Local<bool>,
    mut generators: Query<&mut Generator>,
) {
    if std::env::var_os("SSR_POWER_TEST").is_none() || *done {
        return;
    }
    *elapsed += time.delta_secs();
    if *elapsed < 5.0 {
        return;
    }
    *done = true;
    for mut generator in generators.iter_mut() {
        generator.power_kw = 0.0;
    }
    tracing::info!("power test: generators shut down");
}

/// Инициализирует атмосферу мира (T4.3): пол — воздух станции, стены и космос —
/// вакуум; по чанкам спавнятся реплицируемые [`ChunkAtmosphere`] для клиента.
fn init_atmosphere(
    mut commands: Commands,
    mut atmospheres: ResMut<Atmospheres>,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
    map: Res<GameMap>,
) {
    let Some(first) = map.chunks.first() else {
        return;
    };
    let size = ssr_core::tiles::CHUNK_TILES as i32;
    let mut min = (first.coords.0 * size, first.coords.1 * size);
    let mut max = min;
    for chunk in &map.chunks {
        min = (
            min.0.min(chunk.coords.0 * size),
            min.1.min(chunk.coords.1 * size),
        );
        max = (
            max.0.max((chunk.coords.0 + 1) * size),
            max.1.max((chunk.coords.1 + 1) * size),
        );
    }
    let width = (max.0 - min.0) as usize;
    let height = (max.1 - min.1) as usize;
    let mut tiles = vec![TileType::Space; width * height];
    let mut gas = vec![Gas::VACUUM; width * height];
    for chunk in &map.chunks {
        for ly in 0..size as usize {
            for lx in 0..size as usize {
                let tx = chunk.coords.0 * size + lx as i32 - min.0;
                let ty = chunk.coords.1 * size + ly as i32 - min.1;
                let index = ty as usize * width + tx as usize;
                let tile = chunk.tiles[ly * size as usize + lx];
                tiles[index] = tile;
                if tile.is_walkable() {
                    gas[index] = Gas::STATION;
                }
            }
        }
    }

    for chunk in &map.chunks {
        let room = chunk_rooms.room_for(chunk.coords, &mut allocator);
        let entity = commands
            .spawn((
                ssr_core::atmosphere::ChunkAtmosphere::pack(
                    chunk.coords,
                    &chunk
                        .tiles
                        .iter()
                        .map(|tile| {
                            if *tile == TileType::Floor {
                                Gas::STATION
                            } else {
                                Gas::VACUUM
                            }
                        })
                        .collect::<Vec<Gas>>(),
                ),
                Replicate::to_clients(NetworkTarget::All),
                Rooms::single(room),
            ))
            .id();
        atmospheres.entities.insert(chunk.coords, entity);
    }
    atmospheres.min = min;
    atmospheres.size = (width, height);
    atmospheres.tiles = tiles;
    atmospheres.gas = gas;
    tracing::info!(
        width,
        height,
        chunks = map.chunks.len(),
        "atmosphere initialized"
    );
}

/// Диффузия газа между тайлами (T4.3): обмен по 4 соседям, космос — сток,
/// стены не пропускают. Клиентские компоненты обновляются при изменениях.
fn simulate_atmosphere(
    mut atmospheres: ResMut<Atmospheres>,
    mut components: Query<&mut ssr_core::atmosphere::ChunkAtmosphere>,
    time: Res<Time>,
    mut next_step: Local<f32>,
) {
    *next_step += time.delta_secs();
    if *next_step < ATMOS_STEP_SECS {
        return;
    }
    *next_step = 0.0;
    let (width, height) = atmospheres.size;
    if width == 0 || height == 0 {
        return;
    }

    let current = atmospheres.gas.clone();
    let mut next = current.clone();
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let index = y as usize * width + x as usize;
            if !atmospheres.tiles[index].is_walkable() {
                continue; // стены и космос газ не держат
            }
            // Пары (восток, север) — каждая пара обрабатывается один раз.
            for (dx, dy) in [(1i32, 0i32), (0, 1)] {
                let index_b =
                    atmospheres.index(atmospheres.min.0 + x + dx, atmospheres.min.1 + y + dy);
                let (pressure_b, oxygen_b, exchange) = match index_b {
                    // Сосед-пол: обмен с переносом газа (с сохранением).
                    Some(b) if atmospheres.tiles[b].is_walkable() => {
                        (current[b].pressure, current[b].oxygen, true)
                    }
                    // Стена газ не пропускает и не впитывает: пары нет вовсе.
                    Some(b) if atmospheres.tiles[b] == TileType::Wall => continue,
                    // Космос (в т.ч. за краем карты): сток — газ уходит.
                    Some(_) | None => (0.0, 0.0, false),
                };
                let a = current[index];
                let flow = (a.pressure - pressure_b) * DIFFUSION_K;
                if flow.abs() < 0.01 {
                    continue;
                }
                if flow > 0.0 {
                    // Из текущего тайла в соседа.
                    let moved_o2 = flow * a.oxygen;
                    let a_next = &mut next[index];
                    a_next.pressure = (a_next.pressure - flow).max(0.0);
                    if a_next.pressure > 0.001 {
                        a_next.oxygen =
                            ((a.pressure * a.oxygen) - moved_o2).max(0.0) / a_next.pressure;
                    } else {
                        a_next.oxygen = 0.0;
                    }
                    if exchange && let Some(b) = index_b {
                        let b_current = current[b];
                        let b_next = &mut next[b];
                        b_next.pressure += flow;
                        if b_next.pressure > 0.001 {
                            b_next.oxygen = (b_current.pressure * b_current.oxygen + moved_o2)
                                / b_next.pressure;
                        }
                    }
                } else {
                    // Из соседа в текущий.
                    let moved = -flow;
                    let moved_o2 = moved * oxygen_b;
                    let a_next = &mut next[index];
                    a_next.pressure += moved;
                    if a_next.pressure > 0.001 {
                        a_next.oxygen = (a.pressure * a.oxygen + moved_o2) / a_next.pressure;
                    }
                    if exchange && let Some(b) = index_b {
                        let b_next = &mut next[b];
                        b_next.pressure = (b_next.pressure - moved).max(0.0);
                        if b_next.pressure > 0.001 {
                            b_next.oxygen =
                                ((pressure_b * oxygen_b) - moved_o2).max(0.0) / b_next.pressure;
                        } else {
                            b_next.oxygen = 0.0;
                        }
                    }
                }
            }
        }
    }
    atmospheres.gas = next;

    // Обновляем реплицируемые компоненты чанков, где значения изменились.
    let chunk_tiles = ssr_core::tiles::CHUNK_TILES as i32;
    for (coords, entity) in &atmospheres.entities {
        let Ok(mut component) = components.get_mut(*entity) else {
            continue;
        };
        let packed = ssr_core::atmosphere::ChunkAtmosphere::pack(
            *coords,
            &chunk_slice(&atmospheres, *coords, chunk_tiles),
        );
        if *component != packed {
            *component = packed;
        }
    }
}

/// Срез газа чанка в порядке тайлов [`TileChunkData`].
fn chunk_slice(atmospheres: &Atmospheres, coords: (i32, i32), chunk_tiles: i32) -> Vec<Gas> {
    let mut gas = vec![Gas::VACUUM; (chunk_tiles * chunk_tiles) as usize];
    for ly in 0..chunk_tiles {
        for lx in 0..chunk_tiles {
            let tx = coords.0 * chunk_tiles + lx;
            let ty = coords.1 * chunk_tiles + ly;
            if let Some(value) = atmospheres.gas_at_tile(tx, ty) {
                gas[(ly * chunk_tiles + lx) as usize] = value;
            }
        }
    }
    gas
}

/// Урон от разгерметизации (T4.3): каждую секунду в негодной атмосфере.
fn suffocation(
    time: Res<Time>,
    mut next_tick: Local<f32>,
    players: Res<Players>,
    positions: Query<&PlayerPosition>,
    ghosts: Query<&Ghost>,
    atmospheres: Res<Atmospheres>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    *next_tick += time.delta_secs();
    if *next_tick < 1.0 {
        return;
    }
    *next_tick = 0.0;
    for entry in &players.entries {
        // Призрак летает в вакууме без вреда (механики владельца).
        if ghosts.get(entry.player).is_ok() {
            continue;
        }
        let Ok(position) = positions.get(entry.player) else {
            continue;
        };
        let Some(gas) = atmospheres.gas_at_units(position.0[0], position.0[1]) else {
            continue;
        };
        if gas.is_breathable() {
            continue;
        }
        let cause = if gas.pressure < ssr_core::atmosphere::LOW_PRESSURE_KPA {
            "vacuum"
        } else {
            "no_oxygen"
        };
        damage_events.write(DamageEvent {
            target: entry.player,
            amount: ssr_core::atmosphere::VACUUM_DAMAGE_PER_SECOND,
            source: DamageSource::Environment { cause },
        });
        tracing::info!(player = ?entry.player, pressure = gas.pressure, oxygen = gas.oxygen, cause, "suffocation damage");
    }
}

/// Тик лежачего состояния: игрок лежит KNOCKDOWN_SECS, затем встаёт и
/// возвращается на точку спавна (падение после смерти, механики владельца).
fn knockdown_tick(
    time: Res<Time>,
    mut commands: Commands,
    mut knocked: Query<(Entity, &mut KnockedDown, Option<&RespawnPending>)>,
    mut bodies: Query<&mut Position>,
    map: Res<GameMap>,
) {
    for (entity, mut state, respawn) in knocked.iter_mut() {
        if state.seconds <= 0.0 {
            continue;
        }
        state.seconds -= time.delta_secs();
        if state.seconds > 0.0 {
            continue;
        }
        commands.entity(entity).remove::<KnockedDown>();
        if respawn.is_some() {
            let spawn = map.spawn_points.first().copied().unwrap_or((0.0, 0.0));
            if let Ok(mut body) = bodies.get_mut(entity) {
                body.0 = Vector::new(spawn.0, spawn.1);
            }
            commands.entity(entity).remove::<RespawnPending>();
            tracing::info!(?entity, spawn = ?spawn, "player got up and respawned");
        }
    }
}

/// Автоматика дверей (как в SS14): подошёл игрок с доступом — дверь
/// открывается; ушёл — закрывается через [`AUTO_CLOSE_SECS`].
fn auto_doors(
    mut commands: Commands,
    time: Res<Time>,
    mut doors: Query<(Entity, &mut Door, &mut DoorAuto)>,
    positions: Query<&PlayerPosition>,
    access: Query<&Access>,
    powered: Query<&Powered>,
    players: Res<Players>,
) {
    for (entity, mut door, mut auto) in doors.iter_mut() {
        // Без питания дверь не работает (T4.4) и закрывается, если была открыта.
        if !powered.get(entity).map(|state| state.0).unwrap_or(true) {
            if door.open {
                door.open = false;
                commands.entity(entity).remove::<ColliderDisabled>();
                tracing::info!(door = ?entity, "door closed (unpowered)");
            }
            continue;
        }
        let center = Vec2::from_array(door.position);
        let mut nearby = false;
        for entry in &players.entries {
            let Ok(position) = positions.get(entry.player) else {
                continue;
            };
            if Vec2::from_array(position.0).distance(center) > AUTO_DOOR_RANGE {
                continue;
            }
            let allowed = match door.access.as_deref() {
                None => true,
                Some(required) => access
                    .get(entry.player)
                    .map(|keys| keys.list.iter().any(|key| key == required))
                    .unwrap_or(false),
            };
            if allowed {
                nearby = true;
                break;
            }
        }
        if nearby {
            auto.close_in = AUTO_CLOSE_SECS;
            if !door.open {
                door.open = true;
                commands.entity(entity).insert(ColliderDisabled);
                tracing::info!(door = ?entity, "door auto-opened");
            }
        } else if door.open {
            auto.close_in -= time.delta_secs();
            if auto.close_in <= 0.0 {
                door.open = false;
                commands.entity(entity).remove::<ColliderDisabled>();
                tracing::info!(door = ?entity, "door auto-closed");
            }
        }
    }
}

/// Тест T4.3: SSR_BREACH_TEST=1 — через 3 секунды пробивает стену у космоса
/// (тайлы (62,0..2) в тестовой карте): комната разгерметизируется.
fn breach_test(
    mut commands: Commands,
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut done: Local<bool>,
    mut atmospheres: ResMut<Atmospheres>,
    mut chunks: Query<&mut TileChunkData>,
    mut index: ResMut<MapIndex>,
) {
    if std::env::var_os("SSR_BREACH_TEST").is_none() || *done {
        return;
    }
    *elapsed += time.delta_secs();
    if *elapsed < 3.0 {
        return;
    }
    *done = true;
    let chunk_tiles = ssr_core::tiles::CHUNK_TILES as i32;
    let tx = 62i32;
    for ty in 0..3i32 {
        let coords = (tx.div_euclid(chunk_tiles), ty.div_euclid(chunk_tiles));
        let Some(&entity) = index.chunks.get(&coords) else {
            continue;
        };
        let Ok(mut chunk) = chunks.get_mut(entity) else {
            continue;
        };
        let lx = (tx - coords.0 * chunk_tiles) as usize;
        let ly = (ty - coords.1 * chunk_tiles) as usize;
        chunk.tiles[ly * chunk_tiles as usize + lx] = TileType::Floor;
        if let Some(atmosphere_index) = atmospheres.index(tx, ty) {
            atmospheres.tiles[atmosphere_index] = TileType::Floor;
            atmospheres.gas[atmosphere_index] = Gas::STATION;
        }
        if let Some(old) = index.colliders.remove(&coords) {
            commands.entity(old).despawn();
        }
        if let Some(new_entity) = spawn_chunk_collider(&mut commands, &chunk) {
            index.colliders.insert(coords, new_entity);
        }
        tracing::info!(tx, ty, "breach: wall opened to space");
    }
}

/// Нагрузочный лог (T6.1): раз в секунду TPS, интервал тика, игроки и трафик
/// (байты отправлено/получено из метрик транспорта lightyear).
/// Предыдущий срез нагрузочного лога: время, тики, байты, сообщения.
type PerfSample = (std::time::Instant, u64, f64, f64, u64);

fn perf_logger(
    tick_state: Res<TickState>,
    stats: Res<NetStats>,
    players: Res<Players>,
    replicated: Query<(), With<Replicate>>,
    mut last: Local<Option<PerfSample>>,
) {
    if std::env::var_os("SSR_PERF_LOG").is_none() {
        return;
    }
    let now = std::time::Instant::now();
    let ticks = tick_state.tick;
    let metric = |name: &'static str| {
        lightyear::metrics::prelude::GLOBAL_RECORDER
            .get_gauge_value(&lightyear::metrics::metrics::Key::from_name(name))
            .unwrap_or(0.0)
    };
    let sent = metric("transport/send_bytes");
    let recv = metric("transport/recv_bytes");
    let messages = stats.messages_in;
    let Some((previous_time, previous_ticks, previous_sent, previous_recv, previous_messages)) =
        *last
    else {
        *last = Some((now, ticks, sent, recv, messages));
        return;
    };
    let elapsed = now.duration_since(previous_time).as_secs_f64();
    if elapsed < 1.0 {
        return;
    }
    *last = Some((now, ticks, sent, recv, messages));
    let tps = (ticks - previous_ticks) as f64 / elapsed;
    let tick_ms = elapsed * 1000.0 / (ticks - previous_ticks).max(1) as f64;
    tracing::info!(
        tps = format!("{tps:.1}"),
        tick_ms = format!("{tick_ms:.2}"),
        players = players.entries.len(),
        replicated = replicated.iter().count(),
        sent_kbps = format!("{:.1}", (sent - previous_sent) / 1024.0 / elapsed),
        recv_kbps = format!("{:.1}", (recv - previous_recv) / 1024.0 / elapsed),
        msgs_in = messages - previous_messages,
        "perf"
    );
    // Тики в TickState растут: сбрасываем локальный счётчик через разницу ✓.
}

/// Раз в секунду логирует давление у спавна (для проверки T4.3 без клиента).
fn log_atmosphere(time: Res<Time>, mut next_log: Local<f32>, atmospheres: Res<Atmospheres>) {
    if std::env::var_os("SSR_ATMOS_LOG").is_none() {
        return;
    }
    *next_log += time.delta_secs();
    if *next_log < 1.0 {
        return;
    }
    *next_log = 0.0;
    // Спавн и окрестности пробоя (для проверки утечки в космос).
    for (tx, ty) in [(0, 0), (61, 0), (60, 0)] {
        if let Some(gas) = atmospheres.gas_at_tile(tx, ty) {
            tracing::info!(
                tx,
                ty,
                pressure = gas.pressure,
                oxygen = gas.oxygen,
                "atmosphere"
            );
        }
    }
}

/// Тест T4.3: SSR_VACUUM_TEST=1 — через 3 секунды опустошает тайлы вокруг
/// спавна: игрок задыхается (проверка урона и HUD без долгой утечки).
fn vacuum_test(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut done: Local<bool>,
    mut atmospheres: ResMut<Atmospheres>,
    map: Res<GameMap>,
) {
    if std::env::var_os("SSR_VACUUM_TEST").is_none() || *done {
        return;
    }
    *elapsed += time.delta_secs();
    if *elapsed < 3.0 {
        return;
    }
    *done = true;
    let Some(&(sx, sy)) = map.spawn_points.first() else {
        return;
    };
    let center = (
        (sx / TILE_SIZE).floor() as i32,
        (sy / TILE_SIZE).floor() as i32,
    );
    let mut cleared = 0;
    for dx in -3..=3 {
        for dy in -3..=3 {
            if let Some(index) = atmospheres.index(center.0 + dx, center.1 + dy)
                && atmospheres.tiles[index].is_walkable()
            {
                atmospheres.gas[index] = Gas::VACUUM;
                cleared += 1;
            }
        }
    }
    tracing::info!(tiles = cleared, center = ?center, "vacuum test: area depressurized");
}

/// Загружает роли (T4.2). Ошибка — пустой набор: сервер не падает, игроки
/// получают стандартный набор предметов без роли (в логе — error).
fn load_roles(mut commands: Commands) {
    let path = ssr_core::assets_root().join("prototypes/roles.ron");
    match RoleSet::load(&path) {
        Ok(set) => {
            tracing::info!(roles = set.roles.len(), "roles loaded");
            commands.insert_resource(GameRoles(set));
        }
        Err(e) => tracing::error!(error = %e, "roles not loaded"),
    }
}

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
    let map_name = std::env::var("SSR_MAP").unwrap_or_else(|_| "station.ron".to_string());
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
    // Доступ (T4.2): ключ берётся из door_access карты, иначе дверь открыта всем.
    for &(x, y) in &file.doors {
        let access = file
            .door_access
            .iter()
            .find(|entry| entry.position == (x, y))
            .map(|entry| entry.access.clone());
        let room = chunk_rooms.room_for(chunk_coords(x, y), &mut allocator);
        commands.spawn((
            Door {
                open: false,
                position: [x, y],
                access,
            },
            DoorAuto {
                close_in: AUTO_CLOSE_SECS,
            },
            Consumer {
                draw_kw: ssr_core::power::DOOR_DRAW_KW,
            },
            Powered(true),
            // Позиция нужна энергобалансу (T4.4) и UI.
            ItemPosition([x, y]),
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
        cables: file.cables,
        generators: file.generators,
        lights: file.lights,
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
    RequestActions {
        entity: u64,
        tx: i32,
        ty: i32,
    },
    /// Осмотр объекта или тайла (механики владельца): сервер отвечает описанием.
    Examine {
        entity: u64,
        tx: i32,
        ty: i32,
    },
}

/// Источник урона (T4.1): кто и чем нанёс удар. Новые источники (среда,
/// удушье, электричество) добавляются сюда по мере задач T4.3/T4.4.
#[derive(Debug, Clone)]
enum DamageSource {
    /// Ближний бой: атакующий и предмет в его активной руке.
    Melee {
        attacker: Entity,
        weapon: Option<String>,
    },
    /// Среда (T4.3): разгерметизация, нехватка кислорода, урон без убийцы.
    Environment { cause: &'static str },
}

/// Урон, нанесённый за кадр (T4.1): пишется атакой, применяется apply_damage.
#[derive(Message, Debug)]
struct DamageEvent {
    target: Entity,
    amount: i32,
    source: DamageSource,
}

/// Смерть игрока — Health дошёл до нуля (T4.1): лог и возврат на спавн.
#[derive(Message, Debug)]
struct DeathEvent {
    target: Entity,
    killer: Option<Entity>,
}

/// Предел длины сообщения чата и антиспам-пауза между репликами (сек).
const CHAT_MAX_LEN: usize = 200;
const CHAT_COOLDOWN: f32 = 0.6;
/// Дальность слышимости локального чата (LOOC), юнитов.
const CHAT_LOCAL_RANGE: f32 = 320.0;

/// Время последнего сообщения игрока (антиспам).
#[derive(Resource, Default)]
struct ChatCooldowns(HashMap<u64, f32>);

/// Вспомогательные параметры одним SystemParam: у функций-систем Bevy лимит
/// 16 SystemParam, а handle_client_messages уже на пределе.
#[derive(bevy::ecs::system::SystemParam)]
struct AuxParams<'w, 's> {
    time: Res<'w, Time>,
    cooldowns: ResMut<'w, ChatCooldowns>,
    clothings: Query<'w, 's, &'static mut Clothing>,
}

/// Индекс из времени: без внешних RNG-зависимостей.
fn rand_index(bound: usize) -> usize {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    if bound == 0 { 0 } else { nanos % bound }
}

/// Простой «бросок монетки» для пола на спавне (без выбора игрока — как в SS14).
fn rand_bool() -> bool {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() % 2 == 0)
        .unwrap_or(true)
}

/// Имя игрока по его сущности (для чата): из реестра подключений.
fn items_name(players: &Players, player: Entity) -> String {
    players
        .entries
        .iter()
        .find(|entry| entry.player == player)
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| "Кто-то".to_string())
}

/// Комната, назначенная сущности для репликации предметов: предмет виден
/// клиенту, когда наборы комнат пересекаются (`Rooms` пустой = невидим всем).
#[derive(Component, Clone, Copy, PartialEq, Debug)]
struct ItemRoom(RoomId);

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
#[derive(Component)]
struct PlayerInput {
    direction: [f32; 2],
    /// Бег (Shift): множитель скорости (T-мех).
    running: bool,
    /// Боевой режим: клики бьют, а не используют (T-мех).
    combat: bool,
}

impl Default for PlayerInput {
    fn default() -> Self {
        Self {
            direction: [0.0, 0.0],
            running: false,
            combat: false,
        }
    }
}

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
/// Описание объекта или тайла для осмотра (механики владельца).
#[allow(clippy::too_many_arguments)]
fn describe_target(
    entity_bits: u64,
    tx: i32,
    ty: i32,
    items: &Query<&Item>,
    containers: &Query<&mut Container>,
    container_positions: &Query<&ItemPosition>,
    doors: &Query<&mut Door>,
    atmospheres: &Atmospheres,
    catalogs: &ContentCatalog,
) -> String {
    // Объект под курсором: предмет, ящик, дверь или генератор.
    if entity_bits != 0
        && let Some(entity) = Entity::try_from_bits(entity_bits)
    {
        if let Ok(item) = items.get(entity) {
            let proto = catalogs.items.by_id(&item.name);
            let name = proto
                .map(|item| item.name.clone())
                .unwrap_or_else(|| item.name.clone());
            let tags = proto
                .map(|item| item.tags.join(", "))
                .filter(|tags| !tags.is_empty())
                .map(|tags| format!(" [{tags}]"))
                .unwrap_or_default();
            return format!("{name}{tags}");
        }
        if let Ok(container) = containers.get(entity) {
            let count = container_positions
                .get(entity)
                .map(|_| "с предметами")
                .unwrap_or("");
            return format!(
                "{}: {} {count}",
                container.name,
                if container.open {
                    "открыт"
                } else {
                    "закрыт"
                }
            );
        }
        if let Ok(door) = doors.get(entity) {
            let access = door.access.clone().unwrap_or_else(|| "общий".to_string());
            return format!(
                "Дверь: {}, доступ: {access}",
                if door.open {
                    "открыта"
                } else {
                    "закрыта"
                }
            );
        }
    }
    // Тайл: пол/техпол/стена, атмосфера, провода, игроки.
    let text = match atmospheres.gas_at_tile(tx, ty) {
        Some(gas)
            if atmospheres
                .gas_at_tile(tx, ty)
                .map(|_| true)
                .unwrap_or(false) =>
        {
            format!(
                "Тайл ({tx}, {ty}): давление {:.0} кПа, O₂ {:.0}%",
                gas.pressure,
                gas.oxygen * 100.0
            )
        }
        _ => format!("Тайл ({tx}, {ty}): вне карты"),
    };
    // Игроков на тайле показывает система с доступом к их позициям;
    // здесь (в очереди действий) доступны только имена.
    text
}

/// Выполняет админ-команду (T5.5) и возвращает текст ответа.
/// Команды: `tp <x> <y>`, `spawn <предмет> [кол-во]`, `kick <имя> [причина]`,
/// `heal`.
#[allow(clippy::too_many_arguments)]
fn run_admin_command(
    command: &str,
    player: Entity,
    link: Entity,
    players: &Players,
    commands: &mut Commands,
    inventories: &mut Query<&mut Inventory>,
    positions: &Query<&PlayerPosition>,
    catalogs: &ContentCatalog,
) -> String {
    let mut parts = command.split_whitespace();
    let name = parts.next().unwrap_or_default();
    match name {
        "tp" => {
            let (Some(x), Some(y)) = (
                parts.next().and_then(|v| v.parse::<f32>().ok()),
                parts.next().and_then(|v| v.parse::<f32>().ok()),
            ) else {
                return "использование: tp <x> <y>".to_string();
            };
            // Позиция тела: PlayerPosition едет следом (sync_replicated_position).
            commands.entity(player).insert((
                Position(Vector::new(x, y)),
                LinearVelocity(Vector::ZERO),
                PlayerPosition([x, y]),
            ));
            format!("телепорт в {x:.0}, {y:.0}")
        }
        "spawn" => {
            let Some(item_id) = parts.next() else {
                return "использование: spawn <предмет> [кол-во] [floor]".to_string();
            };
            if catalogs.items.by_id(item_id).is_none() {
                return format!("неизвестный предмет: {item_id}");
            }
            let count: u32 = parts
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1)
                .min(20);
            // Второй режим: положить предмет на пол у ног (спавн-меню, «разместить»).
            let on_floor = parts.next() == Some("floor");
            let (w, h) = catalogs.items.size_of(item_id);
            let Some(base) = positions.get(player).ok().map(|p| p.0) else {
                return "нет позиции игрока".to_string();
            };
            // Координаты размещения (режим размещения спавн-меню): spawn <id> 1 floor x y
            let explicit = match (parts.next(), parts.next()) {
                (Some(x), Some(y)) => match (x.parse::<f32>(), y.parse::<f32>()) {
                    (Ok(x), Ok(y)) => Some((x, y)),
                    _ => None,
                },
                _ => None,
            };
            if on_floor {
                let mut produced = 0;
                for index in 0..count {
                    let spread = (index as f32) * TILE_SIZE * 0.6;
                    let x = explicit.map_or(base[0] + spread, |(x, _)| x + spread);
                    let y = explicit.map_or(base[1] - TILE_SIZE * 0.8, |(_, y)| y);
                    commands.spawn((
                        Item {
                            name: item_id.to_string(),
                        },
                        HeldBy { player: 0 },
                        ItemPosition([x, y]),
                        Replicate::to_clients(NetworkTarget::All),
                        Rooms::default(),
                    ));
                    produced += 1;
                }
                return format!("размещено на полу {produced}× {item_id}");
            }
            let Ok(mut inventory) = inventories.get_mut(player) else {
                return "нет рюкзака".to_string();
            };
            let mut produced = 0;
            for _ in 0..count {
                let entity = commands
                    .spawn((
                        Item {
                            name: item_id.to_string(),
                        },
                        HeldBy {
                            player: player.to_bits(),
                        },
                        Replicate::to_clients(NetworkTarget::All),
                        Rooms::default(),
                    ))
                    .id();
                if inventory.put_first_fit(entity.to_bits(), w, h).is_none() {
                    commands.entity(entity).despawn();
                    break;
                }
                produced += 1;
            }
            format!("выдано {produced}× {item_id}")
        }
        "kick" => {
            let Some(target_name) = parts.next() else {
                return "использование: kick <имя> [причина]".to_string();
            };
            let reason: String = parts.collect::<Vec<_>>().join(" ");
            let Some(target) = players
                .entries
                .iter()
                .find(|entry| entry.name == target_name)
            else {
                return format!("игрок не найден: {target_name}");
            };
            if target.link == link {
                return "нельзя кикнуть себя".to_string();
            }
            // Отключение: компонент Disconnecting → сервер закроет линк,
            // наблюдатель разошлёт Disconnected и уберёт игрока (T5.5).
            commands
                .entity(target.link)
                .insert(lightyear::connection::client::Disconnecting);
            tracing::info!(target = %target_name, reason = %reason, "admin kick");
            format!("кикнут {target_name} ({reason})")
        }
        "heal" => {
            commands.entity(player).insert(Health::default());
            "здоровье восстановлено".to_string()
        }
        "tpto" => {
            // Телепорт к игроку по имени (админ-меню, T-мех).
            let Some(target_name) = parts.next() else {
                return "использование: tpto <имя>".to_string();
            };
            let Some(target) = players
                .entries
                .iter()
                .find(|entry| entry.name == target_name)
            else {
                return format!("игрок не найден: {target_name}");
            };
            if target.player == player {
                return "это вы и есть".to_string();
            }
            // Позицию цели берём из её PlayerPosition (реплицируется).
            commands.trigger(TeleportToPlayer {
                player,
                target: target.player,
            });
            format!("телепорт к {target_name}")
        }
        "ghost" => {
            // Призрак: летает сквозь стены, без коллизии (механики владельца).
            commands
                .entity(player)
                .insert((Ghost, ColliderDisabled, KnockedDown { seconds: 0.0 }));
            "режим призрака включён (полёт сквозь стены)".to_string()
        }
        "unghost" => {
            commands
                .entity(player)
                .remove::<(Ghost, ColliderDisabled, KnockedDown)>();
            "режим призрака выключен".to_string()
        }
        other => format!("неизвестная команда: {other} (tp/spawn/kick/heal/ghost/unghost)"),
    }
    .to_string()
}

/// Единая точка приёма сообщений клиента: рукопожатие (Connect/Welcome, T1.2),
/// операции над руками и админ-команды; исполняемое уходит в [`ActionQueue`].
/// [`ActionQueue`] и обрабатывается одной системой [`process_actions`].
/// Размер предмета по bits сущности (из имени, `item_size`): (ширина, высота).
fn item_size_of(catalogs: &ContentCatalog, items: &Query<&Item>, bits: u64) -> (u8, u8) {
    Entity::try_from_bits(bits)
        .and_then(|entity| items.get(entity).ok())
        .map(|item| catalogs.items.size_of(&item.name))
        .unwrap_or((1, 1))
}

/// Обработчик всех сообщений клиентов: одна точка приёма (MessageReceiver
/// осушается `receive()`), поэтому аргументов много.
#[allow(clippy::too_many_arguments)]
fn handle_client_messages(
    mut commands: Commands,
    map: Res<GameMap>,
    mut content: ServerContent,
    mut spawn_cursor: ResMut<SpawnCursor>,
    mut receivers: Query<(Entity, &RemoteId, &mut MessageReceiver<ClientMessage>), With<Connected>>,
    mut senders: Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
    mut inputs: Query<&mut PlayerInput>,
    positions: Query<&PlayerPosition>,
    mut inventories: Query<&mut Inventory>,
    mut hands: Query<&mut Hands>,
    containers: Query<(Entity, &Container, &ItemPosition)>,
    item_positions: Query<&ItemPosition>,
    items: Query<&Item>,
    mut actions: ResMut<ActionQueue>,
    mut players: ResMut<Players>,
    mut aux: AuxParams,
) {
    // Новый раунд (никого нет): выдача ролей с начала списка — первый игрок
    // сессии снова получает инженера (и его доступы к дверям).
    if players.entries.is_empty() {
        content.role_cursor.0 = 0;
    }
    let mut connected: Vec<(Entity, String)> = Vec::new();
    for (link_entity, remote_id, mut receiver) in receivers.iter_mut() {
        for message in receiver.receive() {
            content.stats.messages_in += 1;
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
                ClientMessage::Examine { entity, tx, ty } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    actions
                        .0
                        .push((entry.player, QueuedAction::Examine { entity, tx, ty }));
                }
                ClientMessage::SetCombat { combat } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    if let Ok(mut input) = inputs.get_mut(entry.player) {
                        input.combat = combat;
                    }
                }
                ClientMessage::Input {
                    movement,
                    running,
                    combat,
                } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        tracing::debug!(link = ?link_entity, "input before handshake");
                        continue;
                    };
                    match inputs.get_mut(entry.player) {
                        Ok(mut input) => {
                            input.direction = movement;
                            input.running = running;
                            input.combat = combat;
                        }
                        Err(e) => tracing::warn!(error = %e, "no PlayerInput"),
                    }
                }
                ClientMessage::TransferItem {
                    item,
                    to_slot,
                    target_player,
                } => {
                    // Перенос предмета (T3.2/T3.4): источник — рюкзак или руки
                    // отправителя ЛИБО открытый контейнер рядом; приёмник —
                    // игрок (0 = сам) либо открытый контейнер.
                    let Some(sender_entry) = players.entry_by_link_mut(link_entity.to_bits())
                    else {
                        continue;
                    };
                    let sender_player = sender_entry.player;
                    let Ok(sender_position) = positions.get(sender_player) else {
                        continue;
                    };
                    let in_range = |point: [f32; 2]| {
                        let dx = point[0] - sender_position.0[0];
                        let dy = point[1] - sender_position.0[1];
                        (dx * dx + dy * dy).sqrt() <= INTERACT_RANGE + TILE_SIZE
                    };

                    // Приёмник: свой/чужой рюкзак или контейнер.
                    let receiver_player = if target_player == 0 {
                        Some(sender_player)
                    } else {
                        let Some(target) = Entity::try_from_bits(target_player) else {
                            tracing::warn!(bits = target_player, "transfer: invalid target bits");
                            continue;
                        };
                        if containers.get(target).is_ok() {
                            // В контейнер: только открытый и только рядом.
                            let Ok((_, container, item_position)) = containers.get(target) else {
                                continue;
                            };
                            if !container.open {
                                tracing::warn!(?target, "transfer: container is closed");
                                continue;
                            }
                            if !in_range(item_position.0) {
                                tracing::warn!(?target, "transfer: container too far");
                                continue;
                            }
                            None
                        } else if inventories.contains(target) {
                            let Ok(target_position) = positions.get(target) else {
                                continue;
                            };
                            let dx = target_position.0[0] - sender_position.0[0];
                            let dy = target_position.0[1] - sender_position.0[1];
                            if (dx * dx + dy * dy).sqrt() > INTERACT_RANGE + 32.0 {
                                tracing::warn!(target = ?target, "transfer: too far");
                                continue;
                            }
                            Some(target)
                        } else {
                            tracing::warn!(target = ?target, "transfer: target has no inventory");
                            continue;
                        }
                    };
                    let receiver_entity = Entity::try_from_bits(target_player);

                    // Источник: рюкзак/руки отправителя или открытый контейнер рядом.
                    let mut source_container: Option<Entity> = None;
                    let in_own = inventories
                        .get(sender_player)
                        .ok()
                        .is_some_and(|inv| inv.contains(item));
                    let in_hands = hands.get(sender_player).ok().is_some_and(|h| h.has(item));
                    if !in_own && !in_hands {
                        // Ищем предмет в открытых контейнерах рядом.
                        for (container_entity, container, item_position) in containers.iter() {
                            if !container.open {
                                continue;
                            }
                            if !in_range(item_position.0) {
                                continue;
                            }
                            let Ok(inv) = inventories.get(container_entity) else {
                                continue;
                            };
                            if inv.contains(item) {
                                source_container = Some(container_entity);
                                break;
                            }
                        }
                        if source_container.is_none() {
                            tracing::warn!(item, "transfer: item is not available to sender");
                            continue;
                        }
                    }

                    // Куда класть: указанный слот или первый свободный.
                    let dest_entity = receiver_player.or(receiver_entity);
                    let Some(dest_entity) = dest_entity else {
                        continue;
                    };
                    let Ok(mut dest_inventory) = inventories.get_mut(dest_entity) else {
                        continue;
                    };
                    let (w, h) = item_size_of(&content.catalogs, &items, item);
                    let anchor = (to_slot != SLOT_ANY).then_some(to_slot);
                    let Some(index) = dest_inventory.find_place(w, h, anchor) else {
                        tracing::warn!(?dest_entity, w, h, "transfer: no room for item");
                        continue;
                    };
                    if !dest_inventory.place(item, w, h, index) {
                        tracing::warn!(index, "transfer: placement failed");
                        continue;
                    }

                    // Изъять из источника.
                    if let Some(container_entity) = source_container {
                        if let Ok(mut source) = inventories.get_mut(container_entity) {
                            source.take(item);
                        }
                    } else if in_hands && let Ok(mut hand) = hands.get_mut(sender_player) {
                        hand.take(item);
                    } else if let Ok(mut inventory) = inventories.get_mut(sender_player) {
                        inventory.take(item);
                    }
                    tracing::info!(item, index, from = ?source_container, to = ?dest_entity, "item transferred");
                    continue;
                }
                ClientMessage::Craft { recipe } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let Some(recipe) = content.catalogs.recipes.by_id(&recipe).cloned() else {
                        tracing::warn!(recipe, "craft: unknown recipe");
                        continue;
                    };
                    let Ok(mut inventory) = inventories.get_mut(player) else {
                        continue;
                    };
                    // Материалы ищем в рюкзаке игрока.
                    let have: Vec<String> = inventory
                        .cells
                        .iter()
                        .flatten()
                        .copied()
                        .filter_map(Entity::try_from_bits)
                        .filter_map(|entity| items.get(entity).ok())
                        .map(|item| item.name.clone())
                        .collect();
                    if !ssr_core::recipes::can_craft(&recipe, &have) {
                        tracing::warn!(recipe = %recipe.id, "craft: not enough materials");
                        continue;
                    }
                    // Списываем вход.
                    let mut consumed = 0usize;
                    for (id, count) in &recipe.inputs {
                        let mut left = *count;
                        while left > 0 {
                            let Some(bits) =
                                inventory.cells.iter().flatten().copied().find(|bits| {
                                    Entity::try_from_bits(*bits)
                                        .and_then(|entity| items.get(entity).ok())
                                        .map(|item| item.name == *id)
                                        .unwrap_or(false)
                                })
                            else {
                                break;
                            };
                            inventory.take(bits);
                            if let Some(entity) = Entity::try_from_bits(bits) {
                                commands.entity(entity).despawn();
                            }
                            consumed += 1;
                            left -= 1;
                        }
                    }
                    // Выдаём результат (по размеру из каталога, тетрис).
                    let (output_id, count) = recipe.output.clone();
                    let (w, h) = content.catalogs.items.size_of(&output_id);
                    let mut produced = 0usize;
                    for _ in 0..count {
                        let entity = commands
                            .spawn((
                                Item {
                                    name: output_id.clone(),
                                },
                                HeldBy {
                                    player: player.to_bits(),
                                },
                                Replicate::to_clients(NetworkTarget::All),
                                Rooms::default(),
                            ))
                            .id();
                        if inventory.put_first_fit(entity.to_bits(), w, h).is_none() {
                            tracing::warn!(item = %output_id, "craft: no room in inventory");
                            commands.entity(entity).despawn();
                            break;
                        }
                        produced += 1;
                    }
                    tracing::info!(
                        recipe = %recipe.id,
                        consumed,
                        produced,
                        output = %output_id,
                        "crafted"
                    );
                }
                ClientMessage::Admin { command } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let name = entry.name.clone();
                    if !is_admin(&name) {
                        tracing::warn!(%name, command = %command, "admin: отказ в правах");
                        continue;
                    }
                    let reply = run_admin_command(
                        &command,
                        player,
                        link_entity,
                        &players,
                        &mut commands,
                        &mut inventories,
                        &positions,
                        &content.catalogs,
                    );
                    tracing::info!(%name, command = %command, reply = %reply, "admin command");
                    if let Ok((_, mut sender)) = senders.get_mut(link_entity) {
                        sender.send::<GameChannel>(ServerMessage::Event {
                            kind: format!("admin:{reply}"),
                        });
                    }
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
                    let Some(item) = inventory.cells.get(slot as usize).copied().flatten() else {
                        continue;
                    };
                    let anchor = inventory.anchor_of(item).unwrap_or(slot);
                    let (w, h) = item_size_of(&content.catalogs, &items, item);
                    inventory.take(item);
                    if !hand.take_in_active(item) {
                        inventory.place(item, w, h, anchor);
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
                    let (w, h) = item_size_of(&content.catalogs, &items, item);
                    match inventory.put_first_fit(item, w, h) {
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
                    // Выброс на пол (механики владельца): предмет остаётся сущностью,
                    // но теряет владельца и получает мировую позицию.
                    let Ok(position) = positions.get(player) else {
                        continue;
                    };
                    if let Some(entity) = Entity::try_from_bits(item) {
                        commands
                            .entity(entity)
                            .insert((HeldBy { player: 0 }, ItemPosition(position.0)));
                        tracing::info!(item, position = ?position.0, "item dropped on floor");
                    }
                }
                ClientMessage::Pickup { item } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let Ok(player_position) = positions.get(player) else {
                        continue;
                    };
                    let Some(entity) = Entity::try_from_bits(item) else {
                        continue;
                    };
                    // Поднять можно только предмет с пола (ItemPosition) и без владельца.
                    let Ok(item_position) = item_positions.get(entity) else {
                        tracing::warn!(item, "pickup: item is not on the floor");
                        continue;
                    };
                    let dx = item_position.0[0] - player_position.0[0];
                    let dy = item_position.0[1] - player_position.0[1];
                    if (dx * dx + dy * dy).sqrt() > INTERACT_RANGE + TILE_SIZE {
                        tracing::warn!(item, "pickup: too far");
                        continue;
                    }
                    let name = items
                        .get(entity)
                        .map(|item| item.name.clone())
                        .unwrap_or_default();
                    let (w, h) = content.catalogs.items.size_of(&name);
                    // Свободная активная рука — приоритет (как в SS14), иначе рюкзак.
                    let mut taken = false;
                    if let Ok(mut hand) = hands.get_mut(player)
                        && hand.active_item().is_none()
                    {
                        taken = hand.take_in_active(item);
                    }
                    if !taken {
                        let Ok(mut inventory) = inventories.get_mut(player) else {
                            continue;
                        };
                        if inventory.put_first_fit(item, w, h).is_none() {
                            tracing::warn!(name, "pickup: no room");
                            continue;
                        }
                    }
                    commands
                        .entity(entity)
                        .remove::<ItemPosition>()
                        .insert(HeldBy {
                            player: player.to_bits(),
                        });
                    tracing::info!(item, name, "item picked up");
                }
                ClientMessage::DropItem { item } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let Ok(position) = positions.get(player) else {
                        continue;
                    };
                    // Предмет должен лежать в рюкзаке этого игрока.
                    let Ok(mut inventory) = inventories.get_mut(player) else {
                        continue;
                    };
                    if inventory.anchor_of(item).is_none() {
                        tracing::warn!(item, "drop item: not in inventory");
                        continue;
                    }
                    inventory.take(item);
                    if let Some(entity) = Entity::try_from_bits(item) {
                        commands.entity(entity).insert((
                            HeldBy { player: 0 },
                            ItemPosition([position.0[0], position.0[1] - TILE_SIZE * 0.8]),
                        ));
                        tracing::info!(item, "item dropped from inventory to floor");
                    }
                }
                ClientMessage::SetAppearance {
                    sex,
                    hair,
                    beard,
                    hair_color,
                } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    if let Some(sex) = sex
                        && let Some(sex) = Sex::from_id(&sex)
                    {
                        commands.entity(player).insert(sex);
                    }
                    if let Some(hair) = hair {
                        if hair_style_names().iter().any(|name| name == &hair) {
                            let color = hair_color.unwrap_or([0x6b, 0x4a, 0x2f]);
                            commands.entity(player).insert(Hair {
                                style: hair.clone(),
                                color,
                            });
                            tracing::info!(%hair, "appearance: hair set");
                        } else {
                            tracing::warn!(%hair, "appearance: unknown hair style");
                        }
                    }
                    if let Some(beard) = beard {
                        if beard.is_empty() {
                            commands.entity(player).remove::<FacialHair>();
                        } else if facial_hair_style_names().iter().any(|name| name == &beard) {
                            let color = hair_color.unwrap_or([0x6b, 0x4a, 0x2f]);
                            commands.entity(player).insert(FacialHair {
                                style: beard,
                                color,
                            });
                        } else {
                            tracing::warn!(%beard, "appearance: unknown beard style");
                        }
                    }
                }
                ClientMessage::Equip { item, slot } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let name = items
                        .get(Entity::try_from_bits(item).unwrap_or(player))
                        .map(|item| item.name.clone())
                        .unwrap_or_default();
                    let Some(slot) = ClothingSlot::from_id(&slot) else {
                        tracing::warn!(slot, "equip: unknown slot");
                        continue;
                    };
                    // Одежда надевается только в свой слот; в карманы/разгрузку
                    // можно класть любые подходящие по размеру предметы.
                    let slot_kind = if slot.is_pocket() || slot.needs_outer() {
                        None
                    } else {
                        match content.catalogs.items.slot_of(&name) {
                            Some(kind) if kind == slot.id() => Some(kind.to_string()),
                            _ => {
                                tracing::warn!(%name, slot = slot.id(), "equip: wrong slot");
                                continue;
                            }
                        }
                    };
                    let _ = slot_kind;
                    // Карман: только мелкие предметы (SS14 PocketableItemSize = Small).
                    if slot.is_pocket() {
                        let (w, h) = content.catalogs.items.size_of(&name);
                        if w > 1 || h > 1 {
                            tracing::warn!(%name, "equip: item too big for pocket");
                            continue;
                        }
                    }
                    // Разгрузка: нужна верхняя одежда (`dependsOn: outerClothing`).
                    if slot.needs_outer()
                        && aux
                            .clothings
                            .get(player)
                            .map(|clothing| clothing.get(ClothingSlot::OuterClothing).is_none())
                            .unwrap_or(true)
                    {
                        tracing::warn!(%name, "equip: no outer clothing for suit storage");
                        continue;
                    }
                    // Предмет должен лежать в рюкзаке: забираем и надеваем.
                    let Ok(mut inventory) = inventories.get_mut(player) else {
                        continue;
                    };
                    if inventory.anchor_of(item).is_none() {
                        tracing::warn!(%name, "equip: item not in inventory");
                        continue;
                    }
                    inventory.take(item);
                    let Ok(mut clothing) = aux.clothings.get_mut(player) else {
                        continue;
                    };
                    if let Some(previous) = clothing.equip(slot, item) {
                        // Прежняя вещь из слота возвращается в рюкзак.
                        let (w, h) = item_size_of(&content.catalogs, &items, previous);
                        inventory.put_first_fit(previous, w, h);
                    }
                    tracing::info!(%name, slot = slot.id(), "clothing equipped");
                }
                ClientMessage::Unequip { slot } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let Some(slot) = ClothingSlot::from_id(&slot) else {
                        continue;
                    };
                    let Ok(mut clothing) = aux.clothings.get_mut(player) else {
                        continue;
                    };
                    let Some(item) = clothing.unequip(slot) else {
                        continue;
                    };
                    let (w, h) = item_size_of(&content.catalogs, &items, item);
                    if let Ok(mut inventory) = inventories.get_mut(player)
                        && inventory.put_first_fit(item, w, h).is_none()
                    {
                        // Рюкзак полон — вещь падает под ноги.
                        if let (Ok(position), Some(entity)) =
                            (positions.get(player), Entity::try_from_bits(item))
                        {
                            commands.entity(entity).insert((
                                HeldBy { player: 0 },
                                ItemPosition([position.0[0], position.0[1] - TILE_SIZE * 0.8]),
                            ));
                        }
                    }
                    tracing::info!(slot = slot.id(), item, "clothing unequipped");
                }
                ClientMessage::Chat { channel, text } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let name = items_name(&players, player);
                    let text: String = text.trim().chars().take(CHAT_MAX_LEN).collect();
                    if text.is_empty() {
                        continue;
                    }
                    // Антиспам: не чаще одной реплики в CHAT_COOLDOWN секунд.
                    let now = aux.time.elapsed_secs();
                    if let Some(last) = aux.cooldowns.0.get(&player.to_bits())
                        && now - last < CHAT_COOLDOWN
                    {
                        tracing::debug!(%name, "chat: rate limited");
                        continue;
                    }
                    aux.cooldowns.0.insert(player.to_bits(), now);
                    let from = positions.get(player).map(|p| p.0).unwrap_or_default();
                    let mut recipients = 0;
                    for (link, mut sender) in senders.iter_mut() {
                        // LOOC слышат только те, кто рядом; OOC — все.
                        if channel == ChatChannel::Looc {
                            let Some(target) = players
                                .entries
                                .iter()
                                .find(|entry| entry.link == link)
                                .map(|entry| entry.player)
                            else {
                                continue;
                            };
                            let Ok(target_position) = positions.get(target) else {
                                continue;
                            };
                            let dx = target_position.0[0] - from[0];
                            let dy = target_position.0[1] - from[1];
                            if (dx * dx + dy * dy).sqrt() > CHAT_LOCAL_RANGE {
                                continue;
                            }
                        }
                        sender.send::<GameChannel>(ServerMessage::Chat {
                            channel,
                            from: name.clone(),
                            text: text.clone(),
                        });
                        recipients += 1;
                    }
                    tracing::info!(%name, ?channel, recipients, %text, "chat message");
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
                MoveVel::default(),
                Replicate::to_clients(NetworkTarget::All),
                Rooms::default(),
                RigidBody::Dynamic,
                Collider::circle(PLAYER_RADIUS),
                // Трение о стены в узком проходе тормозило игрока — выключаем.
                Friction::ZERO,
                Position(Vector::new(spawn.0, spawn.1)),
                Rotation::default(),
                Hands::default(),
                Health::default(),
            ))
            .id();
        let player_bits = player.to_bits();
        // Раса (T5.3): SSR_SPECIES=<id>, по умолчанию человек.
        let species = std::env::var("SSR_SPECIES").unwrap_or_else(|_| "Human".to_string());
        commands.entity(player).insert(Species {
            id: species.clone(),
        });
        tracing::info!(name, spawn = ?spawn, species = %species, "player spawned");

        // Роль (T4.2): SSR_ROLE=<id> — фиксированная (тесты/отладка), иначе
        // выдача по кругу, чтобы в раунде были разные роли.
        let role = std::env::var("SSR_ROLE")
            .ok()
            .and_then(|id| content.roles.0.by_id(&id).cloned())
            .or_else(|| {
                let list = &content.roles.0.roles;
                if list.is_empty() {
                    return None;
                }
                let role = list[content.role_cursor.0 % list.len()].clone();
                content.role_cursor.0 += 1;
                Some(role)
            });

        // Стартовый инвентарь — из роли (T4.2); без ролей — прежний демо-набор.
        let item_names: Vec<String> =
            role.as_ref()
                .map(|role| role.items.clone())
                .unwrap_or_else(|| {
                    ["Crowbar", "SteelSheet", "SteelSheet"]
                        .iter()
                        .map(|name| (*name).to_string())
                        .collect()
                });
        let mut inventory = Inventory::default();
        for item_name in &item_names {
            let item = commands
                .spawn((
                    Item {
                        name: item_name.clone(),
                    },
                    HeldBy {
                        player: player_bits,
                    },
                    Replicate::to_clients(NetworkTarget::All),
                    Rooms::default(),
                ))
                .id();
            let (w, h) = content.catalogs.items.size_of(item_name);
            if inventory.put_first_fit(item.to_bits(), w, h).is_none() {
                tracing::warn!(item = %item_name, "inventory: no room for starting item");
            }
        }
        commands.entity(player).insert(inventory);
        // Одежда: стартовый комплект как у ассистента/инженера в SS14 —
        // рюкзак, комбинезон и ботинки (без рюкзака окно инвентаря не открыть).
        let mut clothing = Clothing::default();
        for worn_name in ["Backpack", "JumpsuitEngineering", "ShoesBlack"] {
            let item = commands
                .spawn((
                    Item {
                        name: worn_name.to_string(),
                    },
                    HeldBy {
                        player: player_bits,
                    },
                    Replicate::to_clients(NetworkTarget::All),
                    Rooms::default(),
                ))
                .id();
            let Some(slot) = content
                .catalogs
                .items
                .slot_of(worn_name)
                .and_then(ClothingSlot::from_id)
            else {
                continue;
            };
            clothing.equip(slot, item.to_bits());
        }
        commands.entity(player).insert(clothing);
        // Пол: варианты есть только у head/chest/groin (SS14 HasSexMorph).
        let sex = std::env::var("SSR_SEX")
            .ok()
            .and_then(|value| Sex::from_id(&value))
            .unwrap_or_else(|| if rand_bool() { Sex::Female } else { Sex::Male });
        commands.entity(player).insert(sex);
        // Причёска: случайный стиль и цвет, как выбор внешности в лобби SS14.
        let hair_options = hair_style_names();
        let hair = Hair {
            style: hair_options[rand_index(hair_options.len())].to_string(),
            color: [
                80 + rand_index(176) as u8,
                50 + rand_index(120) as u8,
                30 + rand_index(100) as u8,
            ],
        };
        // Борода: в SS14 это отдельный маркинг; выпадает не всем (примерно половине).
        if rand_index(2) == 0 {
            let beard_options = facial_hair_style_names();
            commands.entity(player).insert(FacialHair {
                style: beard_options[rand_index(beard_options.len())].to_string(),
                color: hair.color,
            });
        }
        commands.entity(player).insert(hair);
        if let Some(role) = role {
            commands.entity(player).insert((
                PlayerRole {
                    id: role.id.clone(),
                    name: role.name.clone(),
                    antagonist: role.antagonist,
                    goal: role.goal.clone(),
                },
                Access {
                    list: role.access.clone(),
                },
            ));
            tracing::info!(
                name,
                role = %role.id,
                role_name = %role.name,
                antagonist = role.antagonist,
                items = item_names.len(),
                "role assigned"
            );
        }
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
/// Скорость движения, накопленная по Quake-модели (как `MoverController` в SS14):
/// серверная часть сглаживания — без неё позиция меняется скачком за тик.
#[derive(Component, Default)]
struct MoveVel(Vec2);

/// Движение по модели SS14 (`SharedMoverController`): friction → accelerate.
/// Спринт включён по умолчанию (4.5 м/с), Shift = ходьба 2.5 м/с; разгон
/// 20 м/с², торможение почти мгновенное (25/с). Это убирает рывки на сервере,
/// клиентская интерполяция сглаживает оставшееся.
fn movement(
    time: Res<Time>,
    mut players: Query<(
        &PlayerInput,
        &mut LinearVelocity,
        &mut MoveVel,
        Option<&KnockedDown>,
    )>,
) {
    let dt = time.delta_secs().min(0.1);
    for (input, mut velocity, mut move_vel, knocked) in players.iter_mut() {
        // Лежачего не двигаем (падение/стан, T-мех).
        if knocked.is_some() {
            move_vel.0 = Vector::ZERO;
            velocity.0 = Vector::ZERO;
            continue;
        }
        let wish = Vec2::from_array(input.direction).normalize_or_zero();
        // В SS14 спринт по умолчанию: Shift включает ХОДЬБУ, а не бег.
        let wish_speed = if input.running {
            PLAYER_WALK_SPEED
        } else {
            PLAYER_MOVE_SPEED
        };
        // Quake: friction (при движении клампится до accel = 20/с), затем accelerate.
        let friction = if wish != Vec2::ZERO {
            PLAYER_ACCEL / 32.0 // 20/с, как min(friction, accel) в SS14
        } else {
            PLAYER_FRICTION_IDLE
        };
        move_vel.0 *= (1.0 - dt * friction).max(0.0);
        if wish != Vec2::ZERO {
            let add_speed = wish_speed - move_vel.0.dot(wish);
            // accel(20 м/с²) * dt * wishSpeed(м/с) → в юнитах/с.
            let accel_speed = (PLAYER_ACCEL / 32.0 * dt * wish_speed).min(add_speed.max(0.0));
            move_vel.0 += wish * accel_speed;
        }
        velocity.0 = move_vel.0;
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
            // Комната чанка под игроком — якорь для его предметов (руки/рюкзак).
            let own_room = chunk_rooms.room_for(chunk, &mut allocator);
            player_entity.try_insert((Rooms::from(desired.iter().copied()), ItemRoom(own_room)));
        }

        tracing::debug!(chunk = ?chunk, rooms = desired.len(), "interest updated");
        entry.chunk = chunk;
        entry.rooms = desired;
    }
}

/// Выдаёт предметам комнату якоря: держателя (руки/рюкзак), ящика или чанка
/// под лежащим предметом. Иначе `Rooms::default()` (пустой набор) делает предмет
/// невидимым всем клиентам — иконки в UI и модель в руке не приходят (T-мех).
fn sync_item_rooms(
    mut commands: Commands,
    items: Query<(Entity, &HeldBy, Option<&ItemPosition>, Option<&ItemRoom>)>,
    holders: Query<&ItemRoom, With<PlayerPosition>>,
    containers: Query<(&Inventory, &ItemRoom), With<Container>>,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
) {
    for (entity, held, position, current) in items.iter() {
        let room = if held.player != 0 {
            Entity::try_from_bits(held.player)
                .and_then(|holder| holders.get(holder).ok())
                .copied()
        } else if let Some(position) = position {
            Some(ItemRoom(chunk_rooms.room_for(
                chunk_coords(position.0[0], position.0[1]),
                &mut allocator,
            )))
        } else {
            containers
                .iter()
                .find(|(inventory, _)| {
                    inventory
                        .cells
                        .iter()
                        .flatten()
                        .any(|bits| *bits == entity.to_bits())
                })
                .map(|(_, room)| *room)
        };
        let Some(room) = room else {
            continue;
        };
        if current == Some(&room) {
            continue;
        }
        commands
            .entity(entity)
            .insert((room, Rooms::single(room.0)));
        tracing::info!(item = ?entity, room = ?room.0, "item room assigned");
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

/// Запросы здоровья/доступа/питания для обработки действий (сокращает
/// число аргументов системы: у функций-систем лимит 16 параметров).
#[derive(bevy::ecs::system::SystemParam)]
struct ActionQueries<'w, 's> {
    healths: Query<'w, 's, &'static Health>,
    access: Query<'w, 's, &'static Access>,
    powered: Query<'w, 's, &'static Powered>,
    atmospheres: Res<'w, Atmospheres>,
    catalogs: Res<'w, ContentCatalog>,
}

/// Доступ к двери (T4.2): дверь без ключа открыта всем, с ключом — только
/// ролям, у которых этот ключ есть; игрок без роли получает отказ.
fn has_door_access(access: &Query<&Access>, player: Entity, door: &Door) -> bool {
    let Some(required) = door.access.as_deref() else {
        return true;
    };
    access
        .get(player)
        .map(|access| access.list.iter().any(|key| key == required))
        .unwrap_or(false)
}

/// Обрабатывает очередь действий (T3.3+): атака, двери, применение предметов
/// из активной руки и выдача списка контекстных действий (verbs).
#[allow(clippy::too_many_arguments)]
fn process_actions(
    mut commands: Commands,
    mut queue: ResMut<ActionQueue>,
    mut index: ResMut<MapIndex>,
    mut chunks: Query<&mut TileChunkData>,
    positions: Query<&PlayerPosition>,
    mut inventories: Query<&mut Inventory>,
    mut hands: Query<&mut Hands>,
    world: ActionQueries,
    mut doors: Query<&mut Door>,
    mut containers: Query<&mut Container>,
    container_positions: Query<&ItemPosition>,
    items: Query<&Item>,
    mut senders: Query<&mut MessageSender<ServerMessage>, With<Connected>>,
    players: Res<Players>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    if queue.0.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut queue.0);
    for (player, queued) in pending {
        let action = match queued {
            QueuedAction::Do(action) => action,
            // Осмотр (механики владельца): описание объекта или тайла игроку.
            QueuedAction::Examine { entity, tx, ty } => {
                let text = describe_target(
                    entity,
                    tx,
                    ty,
                    &items,
                    &containers,
                    &container_positions,
                    &doors,
                    &world.atmospheres,
                    &world.catalogs,
                );
                tracing::info!(?player, %text, "examine");
                if let Some(link) = players
                    .entries
                    .iter()
                    .find(|entry| entry.player == player)
                    .map(|entry| entry.link)
                    && let Ok(mut sender) = senders.get_mut(link)
                {
                    sender.send::<GameChannel>(ServerMessage::Event {
                        kind: format!("examine:{text}"),
                    });
                }
                continue;
            }
            QueuedAction::RequestActions { entity, tx, ty } => {
                let mut options: Vec<ActionOption> = Vec::new();
                if entity != 0 {
                    if let Some(target) = Entity::try_from_bits(entity) {
                        if world.healths.get(target).is_ok() {
                            options.push(ActionOption {
                                label: "Ударить".into(),
                                action: ActionKind::Attack { target: entity },
                            });
                        }
                        if let Ok(door) = doors.get(target)
                            && has_door_access(&world.access, player, door)
                        {
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
                        if item_name == "SteelSheet" && tile.is_walkable() {
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
                if world.healths.get(target_entity).is_err() {
                    tracing::warn!(?target_entity, "attack: target has no health");
                    continue;
                }
                // Урон по предмету в активной руке: лом — 15, кулак — 5.
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
                damage_events.write(DamageEvent {
                    target: target_entity,
                    amount: damage,
                    source: DamageSource::Melee {
                        attacker: player,
                        weapon,
                    },
                });
                // Атакующему — подтверждение удара (звук попадания, T5.3).
                if let Some(link) = players
                    .entries
                    .iter()
                    .find(|entry| entry.player == player)
                    .map(|entry| entry.link)
                    && let Ok(mut sender) = senders.get_mut(link)
                {
                    sender.send::<GameChannel>(ServerMessage::Event {
                        kind: "hit".to_string(),
                    });
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
                    // Не дверь — возможно, контейнер (T3.4): открыть/закрыть.
                    if let Ok(mut container) = containers.get_mut(target) {
                        let dx = container_positions
                            .get(target)
                            .map(|p| p.0[0] - player_position.0[0])
                            .unwrap_or(0.0);
                        let dy = container_positions
                            .get(target)
                            .map(|p| p.0[1] - player_position.0[1])
                            .unwrap_or(0.0);
                        if (dx * dx + dy * dy).sqrt() > INTERACT_RANGE + TILE_SIZE {
                            tracing::warn!(?player, "interact container: too far");
                            continue;
                        }
                        container.open = !container.open;
                        tracing::info!(
                            container = ?target, open = container.open,
                            "container toggled"
                        );
                    } else {
                        tracing::debug!(target = ?target, "interact: target is not interactable");
                    }
                    continue;
                };
                let dx = door.position[0] - player_position.0[0];
                let dy = door.position[1] - player_position.0[1];
                if (dx * dx + dy * dy).sqrt() > INTERACT_RANGE {
                    tracing::warn!(?player, "interact: too far");
                    continue;
                }
                // Питание (T4.4): обесточенная дверь не открывается.
                if !world
                    .powered
                    .get(target)
                    .map(|state| state.0)
                    .unwrap_or(true)
                {
                    tracing::warn!(door = ?target, "door is unpowered");
                    continue;
                }
                // Доступ (T4.2): дверь с ключом открывают только роли с этим ключом.
                if !has_door_access(&world.access, player, &door) {
                    tracing::warn!(
                        ?player,
                        door = ?target,
                        required = door.access.as_deref().unwrap_or(""),
                        "door access denied"
                    );
                    // Клиенту — сигнал красной лампы (как deny в SS14).
                    if let Some(link) = players
                        .entries
                        .iter()
                        .find(|entry| entry.player == player)
                        .map(|entry| entry.link)
                        && let Ok(mut sender) = senders.get_mut(link)
                    {
                        sender.send::<GameChannel>(ServerMessage::Event {
                            kind: format!("door_denied:{}", target.to_bits()),
                        });
                    }
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
                    if !chunk.tiles[cell].is_walkable() {
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
                    // Разбор стены открывает техпол: на нём видно проводку.
                    chunk.tiles[cell] = TileType::Plating;
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
                        let (w, h) = ssr_core::inventory::item_size("SteelSheet");
                        if let Some(slot) = inventory.put_first_fit(sheet_bits, w, h) {
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

/// Применяет урон из событий (T4.1): Health − amount, лог каждого попадания
/// с источником, при нуле — событие смерти.
fn apply_damage(
    mut damage_events: MessageReader<DamageEvent>,
    mut death_events: MessageWriter<DeathEvent>,
    mut healths: Query<&mut Health>,
) {
    for event in damage_events.read() {
        let Ok(mut health) = healths.get_mut(event.target) else {
            tracing::warn!(target = ?event.target, "damage: target has no health");
            continue;
        };
        let dead = health.damage(event.amount);
        let (killer, weapon) = match &event.source {
            DamageSource::Melee { attacker, weapon } => {
                (Some(*attacker), weapon.as_deref().unwrap_or("fist"))
            }
            // Среда: убийцы нет, «оружие» — причина (vacuum/no_oxygen).
            DamageSource::Environment { cause } => (None, *cause),
        };
        tracing::info!(
            target = ?event.target,
            amount = event.amount,
            killer = ?killer,
            weapon,
            hp = health.current,
            "damage applied"
        );
        if dead {
            death_events.write(DeathEvent {
                target: event.target,
                killer,
            });
        }
    }
}

/// Смерть (T4.1): Health обратно к максимуму, тело на точку спавна, лог.
fn respawn_dead(
    mut commands: Commands,
    mut death_events: MessageReader<DeathEvent>,
    mut healths: Query<&mut Health>,
) {
    for event in death_events.read() {
        if let Ok(mut health) = healths.get_mut(event.target) {
            health.current = health.max;
        }
        // «Падение»: игрок лежит KNOCKDOWN_SECS, потом возвращается на спавн
        // (телепорт — в knockdown_tick, чтобы было видно падение).
        commands.entity(event.target).insert((
            KnockedDown {
                seconds: KNOCKDOWN_SECS,
            },
            RespawnPending,
        ));
        tracing::info!(
            target = ?event.target,
            killer = ?event.killer,
            "player died and fell down"
        );
    }
}

/// Тест T4.1: SSR_DAMAGE_TEST=1 — раз в секунду 25 урона первому игроку
/// (4 удара → смерть → респавн; весь цикл проверяется одним клиентом).
fn damage_test(
    time: Res<Time>,
    players: Res<Players>,
    mut next_hit: Local<f32>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    if std::env::var_os("SSR_DAMAGE_TEST").is_none() {
        return;
    }
    *next_hit += time.delta_secs();
    if *next_hit < 1.0 {
        return;
    }
    *next_hit = 0.0;
    let Some(entry) = players.entries.first() else {
        return;
    };
    damage_events.write(DamageEvent {
        target: entry.player,
        amount: 25,
        source: DamageSource::Melee {
            attacker: entry.player,
            weapon: None,
        },
    });
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
