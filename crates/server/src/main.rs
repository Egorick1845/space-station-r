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

mod weapons;
use lightyear::connection::server::Start;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use ssr_core::atmosphere::Gas;
use ssr_core::clothing::{Clothing, ClothingSlot};
use ssr_core::inventory::{
    Container, Hands, Health, HeldBy, Inventory, Item, ItemPosition, ItemStorage, SLOT_ANY,
};
use ssr_core::mechanics::{
        FacialHair, Hair, PlayerName, Sex, facial_hair_style_names, hair_style_names,
    };
use ssr_core::mechanics::{Ghost, KnockedDown, Sprinting};
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

/// Ящик как `EntityStorage` (PORT_PLAN 1.7): радиус всасывания при закрытии —
/// только предметы, лежащие НА ящике (его собственный тайл). В сборке это
/// `EnteringRange = 0.18` тайла вокруг центра ящика: предмет кладут на открытую
/// крышку, и он лежит в центре. У нас «на ящике» = половина тайла от центра,
/// поэтому вещь, брошенная рядом (падение — 0.8 тайла от игрока), останется на
/// полу, если игрок не стоит у самого ящика.
const CONTAINER_ENTERING_RANGE: f32 = TILE_SIZE * 0.5;
/// На сколько далеко от ящика раскладывается высыпанное содержимое. В движке
/// вещи ложатся в центр (`worldPos + EnteringOffset`), но у нас предмет на полу
/// рисуется ПОД спрайтом ящика (пол 0.45 против ящика 0.7) — в центре их не
/// видно, поэтому раскладываем кольцом вокруг ящика.
const CONTAINER_SPILL_RADIUS: f32 = TILE_SIZE * 1.05;

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
    app.init_resource::<StaminaClock>();
    app.init_resource::<SprintCooldowns>();
    app.init_resource::<GameRoles>();
    app.init_resource::<RoleCursor>();
    app.init_resource::<Atmospheres>();
    app.init_resource::<ContentCatalog>();
    app.init_resource::<NetStats>();
    // Оружие и патроны (W-план): очереди выстрелов, перезарядок и звуков.
    app.init_resource::<weapons::ShootQueue>();
    app.init_resource::<weapons::ReloadQueue>();
    app.init_resource::<weapons::SoundQueue>();
    // Системные сообщения чата (вход/выход, смерть, призрак).
    app.init_resource::<SystemChatQueue>();
    app.add_systems(Startup, startup);
    app.add_systems(
        Startup,
        (
            load_prototypes,
            load_roles,
            load_content,
            load_map,
            spawn_power,
            init_atmosphere,
            spawn_walls,
            spawn_containers,
            spawn_map_entities,
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
            flush_system_chat,
            movement,
            // Тянуть за собой (Pull): цель идёт за игроком после движения.
            pull_follow,
            sync_replicated_position,
            update_client_rooms,
            sync_item_rooms,
            sync_ssd_body_rooms,
            // Оружие (W-план): состояние из прототипа, стрельба, полёт снарядов.
            // Вложенная группа — у кортежа Bevy лимит 20 систем.
            (
                equip_spawned_guns,
                fire_weapons,
                move_projectiles,
                move_thrown,
                flush_world_sounds,
                flush_world_effects,
                reload_weapons,
                gun_test,
            ),
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
            stamina_crit_test,
            drop_hands_on_stam_crit,
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
            // Ящик — `EntityStorage` в сборке: у него есть коллизия, пока крышка
            // закрыта (`IsCollidableWhenOpen = false`), и фикстуры
            // `-0.4,-0.4,0.4,0.29` тайла. Открытый ящик проходим.
            RigidBody::Static,
            Collider::rectangle(TILE_SIZE * 0.8, TILE_SIZE * 0.7),
            Position(Vector::new(*x, *y)),
            Rotation::default(),
        ));
    }
    tracing::info!(count = spots.len(), "containers spawned");
}

/// Спавнимые прототипы сборки (IMP.2/IMP.3): id → размер предмета (`itemSize`).
/// Из этого набора работает команда `spawn` и меню спавна — «как можно больше
/// сущностей», а не только наш ручной каталог предметов.
#[derive(Resource, Default)]
struct ProtoCatalog {
    ids: std::collections::HashSet<String>,
    /// Размер предмета: id → id размера из `item_size.yml` (`Normal`, `Small`, …).
    sizes: std::collections::HashMap<String, String>,
    /// Габариты ЯВНОЙ формы предмета (Item.shape): у лома 1×2 при размере Normal.
    shape_cells: std::collections::HashMap<String, (u8, u8)>,
    /// Структуры (не-предметы): коллизия, поверхность, соединение спрайтов.
    structures: std::collections::HashMap<String, StructureInfo>,
    /// Оружие (`Gun`), патроны (`CartridgeAmmo`), снаряды (`Projectile`),
    /// ёмкости (`BallisticAmmoProvider`), слоты (`ItemSlots`) — W-план.
    guns: std::collections::HashMap<String, ssr_core::prototypes::ProtoGun>,
    cartridges: std::collections::HashMap<String, ssr_core::prototypes::ProtoCartridge>,
    projectiles: std::collections::HashMap<String, ssr_core::prototypes::ProtoProjectile>,
    ammo: std::collections::HashMap<String, ssr_core::prototypes::ProtoAmmoProvider>,
    slots: std::collections::HashMap<String, Vec<ssr_core::prototypes::ProtoItemSlot>>,
    /// Теги прототипов — по ним магазин находит подходящий патрон.
    item_tags: std::collections::HashMap<String, Vec<String>>,
    /// Storage прототипов (StorageComponent): пояс, сумка, коробка.
    storages: std::collections::HashMap<String, ssr_core::prototypes::ProtoStorage>,
}

/// Данные структуры из прототипа (стол, машина, шкаф).
#[derive(Clone, Debug, Default)]
struct StructureInfo {
    /// `PlaceableSurface` — можно класть предметы.
    surface: bool,
    /// `Fixtures` с `hard: true` — есть коллизия.
    solid: bool,
    /// Ключ `IconSmooth` — соседние структуры соединяются спрайтами.
    smooth: Option<String>,
    /// Полуразмеры коллизии в мировых единицах (из `PhysShapeAabb.bounds`).
    half: (f32, f32),
}

impl ProtoCatalog {
    fn contains(&self, id: &str) -> bool {
        self.ids.contains(id)
    }

    /// Габариты предмета: явная форма (`Item.shape`) перекрывает `defaultShape`
    /// размера (`item_size.yml`) — у лома `Normal` + `[0,0,0,1]` = 1×2, у стали
    /// `Normal` без формы = 2×2.
    fn size_cells(&self, id: &str) -> Option<(u8, u8)> {
        if let Some(cells) = self.shape_cells.get(id) {
            return Some(*cells);
        }
        let size_id = self.sizes.get(id)?;
        ssr_core::item_size::cells_of(size_id)
    }

    /// Данные структуры (`None` — прототип не структура).
    fn structure(&self, id: &str) -> Option<&StructureInfo> {
        self.structures.get(id)
    }

    /// `Gun` прототипа (оружие), `None` — прототип не оружие.
    fn gun(&self, id: &str) -> Option<&ssr_core::prototypes::ProtoGun> {
        self.guns.get(id)
    }

    /// `BallisticAmmoProvider` прототипа (магазин/коробка).
    fn ammo_provider(&self, id: &str) -> Option<&ssr_core::prototypes::ProtoAmmoProvider> {
        self.ammo.get(id)
    }

    /// `CartridgeAmmo` прототипа (патрон).
    fn cartridge(&self, id: &str) -> Option<&ssr_core::prototypes::ProtoCartridge> {
        self.cartridges.get(id)
    }

    /// `Projectile` прототипа (снаряд).
    fn projectile(&self, id: &str) -> Option<&ssr_core::prototypes::ProtoProjectile> {
        self.projectiles.get(id)
    }

    /// `ItemSlots` прототипа: стартовый предмет слота (`gun_chamber`).
    fn slot_starting_item(&self, id: &str, slot: &str) -> Option<String> {
        self.slots
            .get(id)?
            .iter()
            .find(|entry| entry.id == slot)?
            .starting_item
            .clone()
    }

    /// `Storage` прототипа (`StorageComponent`): сетка и максимальный размер.
    fn storage_of(&self, id: &str) -> Option<&ssr_core::prototypes::ProtoStorage> {
        self.storages.get(id)
    }

    /// Патрон для магазина по его тегам: в сборке `BallisticAmmoProvider.whitelist`
    /// принимает `CartridgePistol`; ищем первый патрон с общим тегом (нужно для
    /// ручного снаряжения магазинов — P1 `MayTransfer`).
    #[allow(dead_code)]
    fn ammo_tag_round(&self, magazine: &str) -> Option<String> {
        let tags = self.item_tags.get(magazine)?;
        self.cartridges
            .iter()
            .find(|(id, _)| {
                self.item_tags
                    .get(*id)
                    .is_some_and(|round_tags| round_tags.iter().any(|tag| tags.contains(tag)))
            })
            .map(|(id, _)| id.clone())
    }
}

/// Проверяет портированные прототипы (`assets/prototypes_ss14.ron`, IMP.2/IMP.3)
/// при старте и запоминает спавнимые (не abstract, без `HideSpawnMenu`, entity)
/// со спрайтом — те же условия, что в `EntitySpawningUIController.BuildEntityList`.
fn load_prototypes(mut commands: Commands) {
    let path = ssr_core::assets_root().join("prototypes_ss14.ron");
    match ssr_core::prototypes::ProtoSet::load(&path) {
        Ok(set) => {
            let mut catalog = ProtoCatalog::default();
            for proto in &set.protos {
                if proto.abstract_ || proto.kind != "entity" {
                    continue;
                }
                // Данные оружия/патронов/снарядов/слотов — для ВСЕХ прототипов:
                // пули и патроны помечены `HideSpawnMenu` (их нельзя поставить из
                // меню, но стрельба обязана их находить).
                if let Some(size) = &proto.size {
                    catalog.sizes.insert(proto.id.clone(), size.clone());
                }
                // Явная форма предмета (`Item.shape`): у лома 1×2, у стали нет.
                if let Some(cells) = ssr_core::item_size::cells_of_shape(&proto.shape) {
                    catalog.shape_cells.insert(proto.id.clone(), cells);
                }
                if let Some(gun) = &proto.gun {
                    catalog.guns.insert(proto.id.clone(), gun.clone());
                }
                if let Some(cartridge) = &proto.cartridge {
                    catalog
                        .cartridges
                        .insert(proto.id.clone(), cartridge.clone());
                }
                if let Some(projectile) = &proto.projectile {
                    catalog
                        .projectiles
                        .insert(proto.id.clone(), projectile.clone());
                }
                if let Some(provider) = &proto.ammo_provider {
                    catalog.ammo.insert(proto.id.clone(), provider.clone());
                }
                if !proto.item_slots.is_empty() {
                    catalog
                        .slots
                        .insert(proto.id.clone(), proto.item_slots.clone());
                }
                // `Storage` (пояс/сумка/коробка): сетка и максимальный размер.
                if let Some(storage) = &proto.storage {
                    catalog.storages.insert(proto.id.clone(), storage.clone());
                }
                if !proto.tags.is_empty() {
                    catalog
                        .item_tags
                        .insert(proto.id.clone(), proto.tags.clone());
                }
                // Дальше — только спавнимые из меню (не abstract, со спрайтом,
                // без `HideSpawnMenu`) — как `EntitySpawningUIController.BuildEntityList`.
                if proto.sprite.is_none()
                    || proto
                        .categories
                        .iter()
                        .any(|category| category == "HideSpawnMenu")
                {
                    continue;
                }
                // Структура (не предмет): коллизия из `Fixtures`, поверхность из
                // `PlaceableSurface`, соединение из `IconSmooth` — как у столов
                // (`bounds: "-0.45,-0.45,0.45,0.45"`, `hard: true`).
                if !proto.is_item {
                    let fixture = proto
                        .fixtures
                        .iter()
                        .find(|fixture| fixture.hard && fixture.bounds != (-0.5, -0.5, 0.5, 0.5))
                        .or_else(|| proto.fixtures.iter().find(|fixture| fixture.hard));
                    let half = fixture
                        .map(|fixture| {
                            let (left, bottom, right, top) = fixture.bounds;
                            (
                                (right - left).abs() * 0.5 * TILE_SIZE,
                                (top - bottom).abs() * 0.5 * TILE_SIZE,
                            )
                        })
                        .unwrap_or((TILE_SIZE * 0.45, TILE_SIZE * 0.45));
                    catalog.structures.insert(
                        proto.id.clone(),
                        StructureInfo {
                            surface: proto.surface,
                            solid: fixture.is_some(),
                            smooth: proto.smooth.as_ref().map(|smooth| smooth.key.clone()),
                            half,
                        },
                    );
                }
                catalog.ids.insert(proto.id.clone());
            }
            tracing::info!(
                protos = set.protos.len(),
                spawnable = catalog.ids.len(),
                with_size = catalog.sizes.len(),
                "content prototypes loaded"
            );
            commands.insert_resource(catalog);
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
    /// Сущности карты из сборки: лампы, мебель, шкафы, предметы.
    entities: Vec<(String, f32, f32)>,
    /// Двери карты (юниты) — герметичные тайлы атмосферы.
    doors: Vec<(f32, f32)>,
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
    /// Спавнимые прототипы сборки (меню спавна и команда `spawn`).
    prototypes: Res<'w, ProtoCatalog>,
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
/// (`SSR_OPEN_ADMIN=0` — админ-команд нет, по умолчанию админ есть у всех;
/// `SSR_ADMINS` — список имён, если нужно ограничить).
fn is_admin(name: &str) -> bool {
    // На дев-сборке админ есть по умолчанию: владелец запускает игру без флагов
    // и не должен оставаться без спавн-меню и админ-команд. Выключается
    // `SSR_OPEN_ADMIN=0` (прежнее `=1` — тоже «включено»).
    if std::env::var("SSR_OPEN_ADMIN")
        .map(|value| value != "0")
        .unwrap_or(true)
    {
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

/// Автоматика двери (как в SS14): дверь открывается НЕ по близости, а когда
/// игрок с доступом идёт вплотную (толчок, `DoorSystem` в сборке реагирует на
/// столкновение тела с дверью); закрывается через [`AUTO_CLOSE_SECS`], пока в
/// проёме никого нет (`Safety`).
const AUTO_CLOSE_SECS: f32 = 5.0;

/// Таймер автозакрытия двери (только сервер, не реплицируется).
#[derive(Component)]
struct DoorAuto {
    close_in: f32,
}

/// Скорость брошенного предмета, юнит/с: в сборке бросок ~10 тайлов/с
/// (`SharedHandsSystem.TryThrow`, тайл 1 м → у нас 32 юнита).
const THROW_SPEED: f32 = 320.0;
/// Время полёта брошенного предмета, с (дальше гаснет скорость — как трение).
const THROW_LIFETIME: f32 = 0.5;
/// Дальность аккуратного дропа Q: InteractionRange (1.5 тайла) в сборке.
const DROP_RANGE: f32 = INTERACT_RANGE;

/// Брошенный предмет (`ThrownItemComponent` в сборке): летит по прямой,
/// гасится о стену или игрока, затем остаётся лежать. Только сервер: клиент
/// видит полёт по обновлениям `ItemPosition`.
#[derive(Component)]
struct Thrown {
    velocity: [f32; 2],
    lifetime: f32,
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
    /// Тайлы дверей (герметичны, как закрытый шлюз в SS14 — lite-атмосфера
    /// считает их всегда закрытыми).
    doors: std::collections::HashSet<(i32, i32)>,
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
    // Двери — герметичные тайлы (закрытый шлюз не пропускает атмосферу).
    atmospheres.doors = map
        .doors
        .iter()
        .map(|(x, y)| {
            (
                (*x / TILE_SIZE).floor() as i32,
                (*y / TILE_SIZE).floor() as i32,
            )
        })
        .collect();
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
                // Дверь герметична: через дверной тайл обмена нет (lite-модель
                // считает шлюзы закрытыми).
                if atmospheres.doors.contains(&(atmospheres.min.0 + x, atmospheres.min.1 + y))
                    || atmospheres
                        .doors
                        .contains(&(atmospheres.min.0 + x + dx, atmospheres.min.1 + y + dy))
                {
                    continue;
                }
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

/// Двери по модели SS14: НЕ открываются от близости, а только от ТОЛЧКА
/// (игрок идёт вплотную в дверь — аналог bump-open через `PhysicsController`)
/// или от руки (Interact). Открытая дверь закрывается через
/// [`AUTO_CLOSE_SECS`] (у шлюза в сборке `secondsUntilAutoclose = 5`), пока в
/// проёме кто-то стоит — не закрывается (`Safety`). Раньше дверь открывалась
/// от близости 44 юнита и тут же захлопывалась после ручного открытия из
/// «мёртвой зоны» 44–48, а ручное закрытие рядом мгновенно отменялось.
#[allow(clippy::too_many_arguments)]
fn auto_doors(
    mut commands: Commands,
    time: Res<Time>,
    mut doors: Query<(Entity, &mut Door, &mut DoorAuto)>,
    positions: Query<&PlayerPosition>,
    inputs: Query<&PlayerInput>,
    access: Query<&Access>,
    powered: Query<&Powered>,
    players: Res<Players>,
) {
    // Радиус толчка: центр игрока вплотную к двери (дальше половины тайла
    // плюс радиус тела коллизия ещё не подпускает).
    const BUMP_RANGE: f32 = 30.0;
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
        let mut bumping = false; // допущенный игрок идёт вплотную в дверь
        let mut blocked = false; // кто-то стоит в проёме — не закрывать
        for entry in &players.entries {
            let Ok(position) = positions.get(entry.player) else {
                continue;
            };
            let distance = Vec2::from_array(position.0).distance(center);
            if distance > BUMP_RANGE {
                continue;
            }
            let allowed = match door.access.as_deref() {
                None => true,
                Some(required) => access
                    .get(entry.player)
                    .map(|keys| keys.list.iter().any(|key| key == required))
                    .unwrap_or(false),
            };
            if !allowed {
                continue;
            }
            blocked = true;
            let moving = inputs
                .get(entry.player)
                .map(|input| input.direction != [0.0, 0.0])
                .unwrap_or(false);
            if moving {
                bumping = true;
            }
        }
        if bumping && !door.open {
            door.open = true;
            commands.entity(entity).insert(ColliderDisabled);
            auto.close_in = AUTO_CLOSE_SECS;
            tracing::info!(door = ?entity, "door bumped open");
        } else if door.open {
            if blocked {
                auto.close_in = AUTO_CLOSE_SECS;
                continue;
            }
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
    // SSR_MAP=имя файла в assets/maps — RON или YAML КАРТЫ SS14 НАПРЯМУЮ
    // (ssr_core::ss14map: грид-локальные координаты + смещение грида — иначе
    // двери/спавны/сущности попадали в космос и комнаты «разгерметизировались»).
    // По умолчанию — Dev-карта из сборки (SS14 стартует на Dev).
    let map_name = std::env::var("SSR_MAP").unwrap_or_else(|_| "dev_map.yml".to_string());
    let path = ssr_core::assets_root().join("maps").join(&map_name);
    let file = if map_name.ends_with(".yml") || map_name.ends_with(".yaml") {
        // Карта SS14 читается НАПРЯМУЮ (ss14map: грид-смещения, сущности).
        ssr_core::ss14map::load(&path).unwrap_or_else(|e| panic!("{e}"))
    } else {
        MapFile::load(&path).unwrap_or_else(|e| panic!("{e}"))
    };
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
        entities: file.entities,
        doors: file.doors,
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
    /// Сущность ПОД УПРАВЛЕНИЕМ (призрак-наблюдатель или тело).
    player: Entity,
    /// Тело игрока (при ghost остаётся на месте со спрайтом SSD).
    body: Entity,
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

    fn remove_by_link(&mut self, link_bits: u64) -> Option<PlayerEntry> {
        let index = self
            .entries
            .iter()
            .position(|e| e.link.to_bits() == link_bits)?;
        Some(self.entries.remove(index))
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
    /// Снаряд (W-план): стрелявший и прототип снаряда/оружия.
    Projectile {
        shooter: Entity,
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
    spawn_cursor: ResMut<'w, SpawnCursor>,
    /// Заявки на выстрел (W-план) — здесь, чтобы не превысить лимит 16
    /// параметров у `handle_client_messages`.
    shoot_queue: ResMut<'w, weapons::ShootQueue>,
    reload_queue: ResMut<'w, weapons::ReloadQueue>,
    /// Системные сообщения чата (вход/выход игрока, призрак).
    system_chat: ResMut<'w, SystemChatQueue>,
    /// Хранилища предметов (пояса/сумки) — проверка доступа из сообщений.
    item_storages: Query<'w, 's, &'static ItemStorage>,
    /// Наблюдатели-призраки (возврат в тело по команде unghost).
    observers: Query<'w, 's, &'static GhostObserver>,
    /// Спящие тела (Ssd) — кандидаты на переподключение.
    sleeping: Query<'w, 's, (Entity, &'static PlayerName), With<ssr_core::mechanics::Ssd>>,
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
    mut system_chat: ResMut<SystemChatQueue>,
) {
    let bits = trigger.entity.to_bits();
    if let Some(entry) = players.remove_by_link(bits) {
        // Наблюдатель (призрак) под управлением — деспавн.
        if entry.player != entry.body {
            commands.entity(entry.player).despawn();
        }
        // ТЕЛО ОСТАЁТСЯ (SS14: отключившийся игрок «спит» с иконкой SSD):
        // ввод снят, управление никому не принадлежит до переподключения.
        commands
            .entity(entry.body)
            .remove::<PlayerInput>()
            .insert(ssr_core::mechanics::Ssd);
        system_chat.0.push(format!("{} отключился", entry.name));
        tracing::info!(name = entry.name, body = ?entry.body, "Player disconnected");
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
    players: &mut Players,
    commands: &mut Commands,
    inventories: &mut Query<&mut Inventory>,
    positions: &Query<&PlayerPosition>,
    observers: &Query<&GhostObserver>,
    senders: &mut Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
    catalogs: &ContentCatalog,
    prototypes: &ProtoCatalog,
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
            if catalogs.items.by_id(item_id).is_none() && !prototypes.contains(item_id) {
                return format!("неизвестный предмет: {item_id}");
            }
            let count: u32 = parts
                .next()
                .and_then(|v| v.parse().ok())
                .unwrap_or(1)
                .min(20);
            // Второй режим: положить предмет на пол у ног (спавн-меню, «разместить»).
            let on_floor = parts.next() == Some("floor");
            // Размер: сначала данные сборки (форма/размер прототипа), затем наш
            // каталог — у стали `Normal` = 2×2, у лома `Normal` + `shape` = 1×2.
            let (w, h) = prototypes
                .size_cells(item_id)
                .unwrap_or_else(|| catalogs.items.size_of(item_id));
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
            // Структура (стол, машина, шкаф): в сборке это сущность с `Fixtures`
            // (коллизия `bounds: "-0.45,-0.45,0.45,0.45"`), `PlaceableSurface`
            // (на неё кладут предметы) и `IconSmooth` (соединение соседей) —
            // спавним её как структуру, а не как «предмет с именем».
            if catalogs.items.by_id(item_id).is_none()
                && let Some(info) = prototypes.structure(item_id).cloned()
            {
                let mut produced = 0;
                for index in 0..count {
                    let spread = (index as f32) * TILE_SIZE * 0.6;
                    let x = explicit.map_or(base[0] + spread, |(x, _)| x + spread);
                    let y = explicit.map_or(base[1] - TILE_SIZE * 0.8, |(_, y)| y);
                    let mut entity = commands.spawn((
                        ssr_core::structures::Structure {
                            proto: item_id.to_string(),
                            smooth: info.smooth.clone(),
                            surface: info.surface,
                            solid: info.solid,
                        },
                        ItemPosition([x, y]),
                        Replicate::to_clients(NetworkTarget::All),
                        Rooms::default(),
                    ));
                    if info.solid {
                        entity.insert((
                            RigidBody::Static,
                            Collider::rectangle(info.half.0 * 2.0, info.half.1 * 2.0),
                            Position(Vector::new(x, y)),
                            Rotation::default(),
                        ));
                    }
                    produced += 1;
                }
                return format!("размещено структур {produced}× {item_id}");
            }
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
            // Оживление: снятое лежание (смерть теперь оставляет тело лежать).
            commands.entity(player).remove::<KnockedDown>();
            commands.entity(player).remove::<RespawnPending>();
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
            // Управление переходит наблюдателю, тело остаётся (Ssd).
            if let Some(entry) = players.entry_by_link_mut(link.to_bits()) {
                let observer = become_ghost(commands, entry, positions);
                send_welcome(entry.link, observer, senders);
                "режим призрака включён (полёт сквозь стены)".to_string()
            } else {
                "игрок не найден".to_string()
            }
        }
        "unghost" => {
            if let Some(entry) = players.entry_by_link_mut(link.to_bits()) {
                if return_to_body(commands, entry, observers) {
                    send_welcome(entry.link, entry.player, senders);
                    "режим призрака выключен".to_string()
                } else {
                    "призрак не активен".to_string()
                }
            } else {
                "игрок не найден".to_string()
            }
        }
        other => format!("неизвестная команда: {other} (tp/spawn/kick/heal/ghost/unghost)"),
    }
    .to_string()
}

/// Единая точка приёма сообщений клиента: рукопожатие (Connect/Welcome, T1.2),
/// операции над руками и админ-команды; исполняемое уходит в [`ActionQueue`].
/// [`ActionQueue`] и обрабатывается одной системой [`process_actions`].
/// Размер предмета по bits сущности: сначала данные СБОРКИ (явная форма
/// `Item.shape`, затем размер `Item.size` → `item_size.yml`), потом наш каталог.
fn item_size_of(
    catalogs: &ContentCatalog,
    prototypes: &ProtoCatalog,
    items: &Query<&Item>,
    bits: u64,
) -> (u8, u8) {
    let Some(item) = Entity::try_from_bits(bits).and_then(|entity| items.get(entity).ok()) else {
        return (1, 1);
    };
    if let Some(cells) = prototypes.size_cells(&item.name) {
        return cells;
    }
    catalogs.items.size_of(&item.name)
}

/// Призрак-наблюдатель (SS14 `MobObserver`, `observer.yml`): ОТДЕЛЬНАЯ
/// сущность, на которую переносится управление. Тело остаётся на месте и
/// «спит» (Ssd). Хранит биты тела для возврата.
#[derive(Component)]
struct GhostObserver {
    body: Entity,
}

/// Стать призраком: спавн наблюдателя + перенос управления, тело «спит».
/// Единая точка для команды `ghost` и верба `aghost` (раньше верб снимал
/// Ghost без ColliderDisabled — тело продолжало летать сквозь стены).
fn become_ghost(
    commands: &mut Commands,
    entry: &mut PlayerEntry,
    positions: &Query<&PlayerPosition>,
) -> Entity {
    let Ok(position) = positions.get(entry.player) else {
        return entry.player;
    };
    let body = entry.body;
    let spawn = position.0;
    let observer = commands
        .spawn((
            Ghost,
            // Спрайт призрака: клиент выбирает расу по компоненту Species —
            // без неё наблюдатель рисовался обычным игроком (жалоба владельца).
            Species {
                id: "Ghost".to_string(),
            },
            GhostObserver { body },
            // Наблюдатель летает (Kinematic без коллайдера — сквозь стены),
            // скорости 8/12 из observer.yml читает система движения.
            PlayerPosition([spawn[0], spawn[1]]),
            PlayerInput::default(),
            MoveVel::default(),
            RigidBody::Kinematic,
            Position(Vector::new(spawn[0], spawn[1])),
            Rotation::default(),
            LinearVelocity(Vector::ZERO),
            Replicate::to_clients(NetworkTarget::All),
            Rooms::default(),
        ))
        .id();
    // Тело остаётся: ввод снят (не двигается), SSD включён.
    commands
        .entity(body)
        .remove::<PlayerInput>()
        .insert(ssr_core::mechanics::Ssd);
    entry.player = observer;
    tracing::info!(name = entry.name, body = ?body, observer = ?observer, "ghost observer spawned");
    observer
}

/// Вернуться в тело: управление обратно, наблюдатель деспавнится, SSD снят.
fn return_to_body(
    commands: &mut Commands,
    entry: &mut PlayerEntry,
    observers: &Query<&GhostObserver>,
) -> bool {
    if entry.player == entry.body {
        return false;
    }
    let Ok(observer) = observers.get(entry.player) else {
        return false;
    };
    let body = observer.body;
    commands.entity(entry.player).despawn();
    commands
        .entity(body)
        .remove::<ssr_core::mechanics::Ssd>()
        .insert(PlayerInput::default());
    entry.player = body;
    // Welcome с битами тела шлёт вызывающий (у него своя форма запроса senders).
    tracing::info!(name = entry.name, body = ?body, "returned to body");
    true
}

/// Welcome с новой сущностью под управлением (OwnPlayerEntity клиента
/// переключится): общий для unghost/aghost/переподключения.
fn send_welcome(
    link: Entity,
    entity: Entity,
    senders: &mut Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
) {
    if let Ok((_, mut sender)) = senders.get_mut(link) {
        sender.send::<GameChannel>(ServerMessage::Welcome {
            player_entity: entity.to_bits(),
            protocol_version: PROTOCOL_VERSION,
        });
    }
}

/// Обработчик всех сообщений клиентов: одна точка приёма (MessageReceiver
/// осушается `receive()`), поэтому аргументов много.
/// Прямое системное сообщение одному клиенту (отказ в канале чата и т.п.):
/// в сборке ответ приходит `ChatMessageToOne` с `ChatChannel.Server`.
fn send_system_to(
    link_entity: Entity,
    senders: &mut Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
    text: &str,
) {
    for (link, mut sender) in senders.iter_mut() {
        if link == link_entity {
            sender.send::<GameChannel>(ServerMessage::Chat {
                channel: ChatChannel::System,
                from: String::new(),
                text: text.to_string(),
            });
        }
    }
}

/// Снимает слот с игрока и каскадно зависимые слоты (`dependsOn` шаблона
/// человека, как `TryUnequip` в Equip.cs:458): снятое — в свободную руку,
/// иначе под ноги (`HandsSystem.PickupOrDrop`). Общая для ClientMessage::Unequip
/// и верба «Снять» (ActionKind::Unequip).
fn unequip_slot(
    commands: &mut Commands,
    clothings: &mut Query<&mut Clothing>,
    hands: &mut Query<&mut Hands>,
    positions: &Query<&PlayerPosition>,
    player: Entity,
    slot: ClothingSlot,
) {
    let Ok(mut clothing) = clothings.get_mut(player) else {
        return;
    };
    let Some(item) = clothing.unequip(slot) else {
        return;
    };
    let mut removed = vec![item];
    for dependent in slot.dependents() {
        if let Some(extra) = clothing.unequip(*dependent) {
            removed.push(extra);
        }
    }
    for item in removed {
        // Приёмник — как в сборке: `TryUnequip` (Equip.cs:473-475) кладёт снятое
        // `DropNextTo`, а `OnUseSlot` — через `HandsSystem.PickupOrDrop`
        // (Equip.cs:102): свободная рука, иначе пол. В РЮКЗАК НЕ КЛАДЁМ: снятый
        // рюкзак попадал в собственную сетку и исчезал (владелец: «сняв его
        // кликом он тупо исчезает»).
        let mut placed = false;
        if let Ok(mut hand) = hands.get_mut(player) {
            placed = hand.take_in_active(item);
        }
        if !placed
            && let (Ok(position), Some(entity)) =
                (positions.get(player), Entity::try_from_bits(item))
        {
            commands.entity(entity).insert((
                HeldBy { player: 0 },
                ItemPosition([position.0[0], position.0[1] - TILE_SIZE * 0.8]),
            ));
        }
    }
}

/// Даёт предмету хранилище из прототипа (`StorageComponent`): сетка —
/// ограничивающий прямоугольник `Storage.grid` (у пояса `0,0,7,1` → 8×2).
/// Вызывается при надевании и при спавне экипировки; при снятии компоненты
/// остаются — содержимое пояса сохраняется, как в сборке.
fn attach_item_storage(
    commands: &mut Commands,
    prototypes: &ProtoCatalog,
    inventories: &Query<&mut Inventory>,
    name: &str,
    item: Entity,
) {
    let Some(storage) = prototypes.storage_of(name) else {
        return;
    };
    if inventories.get(item).is_ok() {
        return; // сетка уже есть — содержимое не сбрасываем
    }
    // Ограничивающий прямоугольник всех боксов сетки.
    let Some(&(x0, y0, x1, y1)) = storage.grid.first() else {
        return;
    };
    let bounds = storage
        .grid
        .iter()
        .skip(1)
        .fold((x0, y0, x1, y1), |(lx, ly, hx, hy), (a, b, c, d)| {
            (lx.min(*a), ly.min(*b), hx.max(*c), hy.max(*d))
        });
    let cols = (bounds.2 - bounds.0 + 1).clamp(1, 16) as u8;
    let rows = (bounds.3 - bounds.1 + 1).clamp(1, 16) as u8;
    commands
        .entity(item)
        .insert((ItemStorage { open: false }, Inventory::with_size(cols, rows)));
    tracing::info!(name, cols, rows, "item storage attached");
}

/// Доступно ли хранилище предмета: открыто И (носитель — это сам игрок ИЛИ
/// предмет лежит рядом). Для надетого пояса позиции нет — доступ по одежде.
fn storage_accessible(
    item_storages: &Query<&ItemStorage>,
    item_positions: &Query<&ItemPosition>,
    clothings: &Query<&mut Clothing>,
    player: Entity,
    container: Entity,
    player_position: [f32; 2],
) -> bool {
    if !item_storages.get(container).is_ok_and(|storage| storage.open) {
        return false;
    }
    if clothings
        .get(player)
        .map(|clothing| clothing.contains(container.to_bits()))
        .unwrap_or(false)
    {
        return true;
    }
    item_positions
        .get(container)
        .map(|position| {
            let dx = position.0[0] - player_position[0];
            let dy = position.0[1] - player_position[1];
            dx * dx + dy * dy <= INTERACT_RANGE * INTERACT_RANGE
        })
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn handle_client_messages(
    mut commands: Commands,
    map: Res<GameMap>,
    mut content: ServerContent,
    mut sprint_state: SprintState,
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
                    // Системное сообщение в чат: как строка входа игрока в сборке.
                    aux.system_chat.0.push(format!("{name} подключился"));
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
                ClientMessage::ToggleSprint { sprint } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let now = sprint_state.clock.seconds;
                    // Запреты из сборки (`SprintAttemptEvent`): лежание и призрак.
                    // Невесомость у нас пока не моделируется по игроку — отметить
                    // как отклонение при переносе атмосферы (PORT_PLAN 3.1).
                    let blocked = sprint_state.knockeds.get(player).is_ok()
                        || sprint_state.ghosts.get(player).is_ok();
                    // Пауза между спринтами (`TimeBetweenSprints = 3` с).
                    let cooling = sprint_state
                        .cooldowns
                        .0
                        .get(&player)
                        .is_some_and(|until| now < *until);
                    if sprint && (blocked || cooling) {
                        tracing::info!(?player, blocked, cooling, "sprint denied");
                        continue;
                    }
                    commands.entity(player).insert(Sprinting(sprint));
                    if !sprint {
                        sprint_state
                            .cooldowns
                            .0
                            .insert(player, now + ssr_core::stamina::TIME_BETWEEN_SPRINTS);
                    }
                    tracing::info!(?player, sprint, "sprint toggled");
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
                    let (w, h) = item_size_of(&content.catalogs, &content.prototypes, &items, item);
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
                        &mut players,
                        &mut commands,
                        &mut inventories,
                        &positions,
                        &aux.observers,
                        &mut senders,
                        &content.catalogs,
                        &content.prototypes,
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
                    let (w, h) = item_size_of(&content.catalogs, &content.prototypes, &items, item);
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
                    let (w, h) = item_size_of(&content.catalogs, &content.prototypes, &items, item);
                    match inventory.put_first_fit(item, w, h) {
                        Some(slot) => tracing::info!(item, slot, "item stowed"),
                        None => {
                            hand.take_in_active(item);
                            tracing::warn!(item, "stow: inventory full");
                        }
                    }
                }
                ClientMessage::DropHand { target, throw } => {
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
                    let Some(entity) = Entity::try_from_bits(item) else {
                        continue;
                    };
                    // SS14 (`SharedHandsSystem`): Q кладёт предмет К КУРСОРУ,
                    // но не дальше InteractionRange — дальше зажимаем к игроку;
                    // Ctrl+Q (`ThrowItemInHand`) — бросок: предмет летит к
                    // курсору и гасится о стены.
                    let Ok(position) = positions.get(player) else {
                        continue;
                    };
                    let from = position.0;
                    let mut offset = [target[0] - from[0], target[1] - from[1]];
                    let distance = (offset[0] * offset[0] + offset[1] * offset[1]).sqrt();
                    if throw && distance > 8.0 {
                        // Бросок: скорость из сборки ~10 тайлов/с (TILE 32 → 320
                        // юнит/с), время полёта ограничено — как дальность руки.
                        let speed = THROW_SPEED;
                        let dir = [offset[0] / distance, offset[1] / distance];
                        commands.entity(entity).insert((
                            HeldBy { player: 0 },
                            ItemPosition(from),
                            Thrown {
                                velocity: [dir[0] * speed, dir[1] * speed],
                                lifetime: THROW_LIFETIME,
                            },
                        ));
                        tracing::info!(item, target = ?target, "item thrown");
                        continue;
                    }
                    if distance > DROP_RANGE {
                        offset = [
                            offset[0] / distance * DROP_RANGE,
                            offset[1] / distance * DROP_RANGE,
                        ];
                    }
                    let at = [from[0] + offset[0], from[1] + offset[1]];
                    commands
                        .entity(entity)
                        .insert((HeldBy { player: 0 }, ItemPosition(at)));
                    tracing::info!(item, at = ?at, "item dropped on floor");
                }
                ClientMessage::Shoot { dir } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    // Стрельбу исполняет `fire_weapons`: здесь только ставим заявку
                    // (клиент сообщает направление прицела, разброс считает сервер).
                    tracing::info!(player = ?entry.player, ?dir, "shoot request");
                    aux.shoot_queue.0.push((entry.player, dir));
                }
                ClientMessage::Reload => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    aux.reload_queue.0.push(entry.player);
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
                    // Карман: только мелкие предметы. В сборке сравнение идёт
                    // по ВЕСУ размера (`InventorySystem.Equip.cs:262-270`:
                    // `GetSizePrototype(item.Size) <= GetSizePrototype("Small")`),
                    // а не по клеткам: Tiny(1) и Small(2) влезают, Normal(4) —
                    // уже нет (стальной лист 2×2 в карман не лезет).
                    if slot.is_pocket() && !content.catalogs.items.pocketable(&name) {
                        tracing::warn!(%name, "equip: item too big for pocket");
                        continue;
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
                    // Предмет должен быть в руке или в рюкзаке. В SS14 надеть вещь
                    // можно прямо из руки (`InventorySystem` берёт её из слота-источника),
                    // поэтому клик по слоту с вещью в руке работает.
                    let in_inventory = inventories
                        .get(player)
                        .map(|inventory| inventory.anchor_of(item).is_some())
                        .unwrap_or(false);
                    let in_hand = hands
                        .get(player)
                        .map(|hand| hand.active_item() == Some(item))
                        .unwrap_or(false);
                    if !in_inventory && !in_hand {
                        tracing::warn!(%name, "equip: item is neither in a hand nor in the backpack");
                        continue;
                    }
                    if in_hand {
                        if let Ok(mut hand) = hands.get_mut(player) {
                            hand.take(item);
                        }
                    } else if let Ok(mut inventory) = inventories.get_mut(player) {
                        inventory.take(item);
                    }
                    let Ok(mut clothing) = aux.clothings.get_mut(player) else {
                        continue;
                    };
                    if let Some(previous) = clothing.equip(slot, item) {
                        // Прежняя вещь из слота возвращается туда, откуда пришла
                        // новая: в руку (если надевали из руки) или в рюкзак.
                        let (w, h) =
                            item_size_of(&content.catalogs, &content.prototypes, &items, previous);
                        let mut placed = false;
                        if in_hand && let Ok(mut hand) = hands.get_mut(player) {
                            placed = hand.take_in_active(previous);
                        }
                        if !placed
                            && let Ok(mut inventory) = inventories.get_mut(player)
                            && inventory.put_first_fit(previous, w, h).is_some()
                        {
                            placed = true;
                        }
                        if !placed
                            && let (Ok(position), Some(entity)) =
                                (positions.get(player), Entity::try_from_bits(previous))
                        {
                            // Ни руки, ни места — вещь падает под ноги.
                            commands.entity(entity).insert((
                                HeldBy { player: 0 },
                                ItemPosition([position.0[0], position.0[1] - TILE_SIZE * 0.8]),
                            ));
                        }
                    }
                    if let Some(entity) = Entity::try_from_bits(item) {
                        attach_item_storage(
                            &mut commands,
                            &content.prototypes,
                            &inventories,
                            &name,
                            entity,
                        );
                    }
                    tracing::info!(%name, slot = slot.id(), from_hand = in_hand, "clothing equipped");
                }
                ClientMessage::Unequip { slot } => {
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let Some(slot) = ClothingSlot::from_id(&slot) else {
                        continue;
                    };
                    let player = entry.player;
                    unequip_slot(
                        &mut commands,
                        &mut aux.clothings,
                        &mut hands,
                        &positions,
                        player,
                        slot,
                    );
                    tracing::info!(slot = slot.id(), "clothing unequipped");
                }
                ClientMessage::StoragePut { container, item } => {
                    // Положить предмет из активной руки в хранилище предмета
                    // (окно пояса в сборке: перетаскивание в StorageWindow).
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let Some(container_entity) = Entity::try_from_bits(container) else {
                        continue;
                    };
                    // Хранилище открыто и доступно (носитель или рядом).
                    if !storage_accessible(
                        &aux.item_storages,
                        &item_positions,
                        &aux.clothings,
                        player,
                        container_entity,
                        positions.get(player).map(|p| p.0).unwrap_or_default(),
                    ) {
                        continue;
                    }
                    let Ok(mut hand) = hands.get_mut(player) else {
                        continue;
                    };
                    if hand.active_item() != Some(item) {
                        tracing::warn!(item, "storage put: item is not in the active hand");
                        continue;
                    }
                    let (w, h) = item_size_of(&content.catalogs, &content.prototypes, &items, item);
                    let Ok(mut dest) = inventories.get_mut(container_entity) else {
                        continue;
                    };
                    let Some(index) = dest.put_first_fit(item, w, h) else {
                        tracing::warn!(item, "storage put: no room");
                        continue;
                    };
                    hand.take(item);
                    tracing::info!(item, container, index, "item stored");
                }
                ClientMessage::StorageTake { container, slot } => {
                    // Взять из хранилища в активную руку (клик по ячейке окна;
                    // рука занята — оставляем в хранилище, как отмена перетаскивания).
                    let Some(entry) = players.entry_by_link_mut(link_entity.to_bits()) else {
                        continue;
                    };
                    let player = entry.player;
                    let Some(container_entity) = Entity::try_from_bits(container) else {
                        continue;
                    };
                    if !storage_accessible(
                        &aux.item_storages,
                        &item_positions,
                        &aux.clothings,
                        player,
                        container_entity,
                        positions.get(player).map(|p| p.0).unwrap_or_default(),
                    ) {
                        continue;
                    }
                    let Ok(mut source) = inventories.get_mut(container_entity) else {
                        continue;
                    };
                    let Some(item) = source.cells.get(slot as usize).copied().flatten() else {
                        continue;
                    };
                    let to_hand = match hands.get_mut(player) {
                        Ok(mut hand) => hand.take_in_active(item),
                        Err(_) => false,
                    };
                    if !to_hand {
                        tracing::warn!(item, "storage take: active hand is busy");
                        continue;
                    }
                    source.take(item);
                    if let Some(entity) = Entity::try_from_bits(item) {
                        commands.entity(entity).insert(HeldBy {
                            player: player.to_bits(),
                        });
                    }
                    tracing::info!(item, container, slot, "item taken from storage");
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
                    // Права каналов из сборки (`ChatSystem.cs`): чат мёртвых
                    // пишут только призраки (SendDeadChat), админ-чат — только
                    // админы (`ChatManager.SendAdminChat`). Отказ — прямое
                    // системное сообщение этому клиенту.
                    if channel == ChatChannel::Dead && sprint_state.ghosts.get(player).is_err() {
                        send_system_to(
                            link_entity,
                            &mut senders,
                            "Чат мёртвых доступен только призракам",
                        );
                        continue;
                    }
                    if channel == ChatChannel::AdminChat && !is_admin(&name) {
                        send_system_to(
                            link_entity,
                            &mut senders,
                            "Админ-чат доступен только администрации",
                        );
                        continue;
                    }
                    let from = positions.get(player).map(|p| p.0).unwrap_or_default();
                    let mut recipients = 0;
                    for (link, mut sender) in senders.iter_mut() {
                        let entry = players
                            .entries
                            .iter()
                            .find(|entry| entry.link == link);
                        // Адресация из сборки: LOOC слышат только рядом (OOC —
                        // все), чат мёртвых — призраки и админы
                        // (`GetDeadChatClients`), админ-чат — только админы.
                        let audible = match channel {
                            ChatChannel::Looc => {
                                let Some(target) = entry.map(|entry| entry.player) else {
                                    continue;
                                };
                                let Ok(target_position) = positions.get(target) else {
                                    continue;
                                };
                                let dx = target_position.0[0] - from[0];
                                let dy = target_position.0[1] - from[1];
                                (dx * dx + dy * dy).sqrt() <= CHAT_LOCAL_RANGE
                            }
                            ChatChannel::Dead => entry
                                .map(|entry| {
                                    sprint_state.ghosts.get(entry.player).is_ok()
                                        || is_admin(&entry.name)
                                })
                                .unwrap_or(false),
                            ChatChannel::AdminChat => entry
                                .map(|entry| is_admin(&entry.name))
                                .unwrap_or(false),
                            _ => true,
                        };
                        if !audible {
                            continue;
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
        // Переподключение: если тело этого имени «спит» (Ssd) — возвращаем
        // управление В НЕГО (SS14: Mind переносится, тело не деспавнится).
        let reused = aux
            .sleeping
            .iter()
            .find(|(entity, player_name)| {
                player_name.0 == name
                    // Тело не под управлением другого наблюдателя.
                    && !players.entries.iter().any(|entry| entry.body == *entity)
            })
            .map(|(entity, _)| entity);
        if let Some(body) = reused {
            commands
                .entity(body)
                .remove::<ssr_core::mechanics::Ssd>()
                .insert(PlayerInput::default());
            players.entries.push(PlayerEntry {
                link: link_entity,
                name: name.clone(),
                player: body,
                body,
                chunk: (0, 0),
                rooms: Vec::new(),
            });
            if let Ok((_, mut sender)) = senders.get_mut(link_entity) {
                sender.send::<GameChannel>(ServerMessage::Welcome {
                    player_entity: body.to_bits(),
                    protocol_version: PROTOCOL_VERSION,
                });
            }
            tracing::info!(name, body = ?body, "player reconnected into body");
            continue;
        }
        // Точка спавна — до создания сущности (id игрока сразу известен и нужен
        // демо-предметам: commands отложены, но id уже зарезервирован).
        let spawn = if map.spawn_points.is_empty() {
            (0.0, 0.0)
        } else {
            let point = map.spawn_points[aux.spawn_cursor.0 % map.spawn_points.len()];
            aux.spawn_cursor.0 += 1;
            point
        };
        let player = commands
            .spawn((
                PlayerPosition([spawn.0, spawn.1]),
                PlayerInput::default(),
                MoveVel::default(),
                ssr_core::stamina::Stamina::default(),
                Sprinting(false),
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
        // Имя на теле — переподключение находит СВОЁ тело (SS14: Mind).
        commands.entity(player).insert(PlayerName(name.clone()));
        tracing::info!(name, spawn = ?spawn, species = %species, "player spawned");

        // Роль (T4.2): SSR_ROLE=<id> — фиксированная (тесты/отладка), иначе
        // выдача по кругу, чтобы в раунде были разные роли.
        let role = std::env::var("SSR_ROLE")
            .ok()
            .and_then(|id| content.roles.0.by_id(&id).cloned())
            .or_else(|| {
                // Round-robin по весу профессий (JobPrototype.weight: больше —
                // раньше в раздаче).
                let mut list = content.roles.0.roles.clone();
                list.sort_by_key(|role| -role.weight);
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
        // Одежда — из роли (startingGear.equipment в сборке): слоты → предметы.
        // Пояс с хранилищем получает ItemStorage из прототипа автоматически.
        let gear: Vec<(String, String)> = role
            .as_ref()
            .map(|role| role.gear.clone())
            .unwrap_or_else(|| {
                [
                    ("back", "Backpack"),
                    ("jumpsuit", "JumpsuitEngineering"),
                    ("shoes", "ShoesBlack"),
                    ("gloves", "GlovesYellow"),
                    ("head", "HardhatWhite"),
                    ("ears", "Headset"),
                    ("belt", "ClothingBeltUtility"),
                    ("eyes", "ClothingEyesGlasses"),
                    ("id", "IDCardEngineer"),
                ]
                .iter()
                .map(|(slot, item)| ((*slot).to_string(), (*item).to_string()))
                .collect()
            });
        let mut clothing = Clothing::default();
        for (slot_id, worn_name) in &gear {
            let item = commands
                .spawn((
                    Item {
                        name: worn_name.clone(),
                    },
                    HeldBy {
                        player: player_bits,
                    },
                    Replicate::to_clients(NetworkTarget::All),
                    Rooms::default(),
                ))
                .id();
            // Слот — из роли, иначе из каталога предметов.
            let Some(slot) = ClothingSlot::from_id(slot_id)
                .or_else(|| {
                    content
                        .catalogs
                        .items
                        .slot_of(worn_name)
                        .and_then(ClothingSlot::from_id)
                })
            else {
                tracing::warn!(item = %worn_name, slot = %slot_id, "gear: unknown slot");
                continue;
            };
            clothing.equip(slot, item.to_bits());
            attach_item_storage(
                &mut commands,
                &content.prototypes,
                &inventories,
                worn_name,
                item,
            );
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
                    icon: role.icon.clone(),
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
            body: player,
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

/// Игрок тянет сущность за собой (верб «Тянуть»): в сборке это
/// `PullerComponent.Pulling` плюс distance-сустав до цели.
#[derive(Component)]
struct Pulling {
    target: Entity,
    /// Длина «верёвки»: расстояние между центрами в момент захвата.
    length: f32,
}

/// Эту сущность тянут (`PullableComponent.Puller`).
#[derive(Component)]
struct PulledBy {
    puller: Entity,
}

/// Движение по модели SS14 (`SharedMoverController`): friction → accelerate.
/// Спринт включён по умолчанию (4.5 м/с), Shift = ходьба 2.5 м/с; разгон
/// 20 м/с², торможение почти мгновенное (25/с). Это убирает рывки на сервере,
/// клиентская интерполяция сглаживает оставшееся.
fn movement(
    time: Res<Time>,
    mut clock: ResMut<StaminaClock>,
    mut players: MovingPlayers,
    mut commands: Commands,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    let dt = time.delta_secs().min(0.1);
    clock.seconds += dt;
    let now = clock.seconds;
    for (
        player,
        input,
        mut velocity,
        mut move_vel,
        knocked,
        stamina,
        sprinting_flag,
        health,
        pulling,
        ghost,
    ) in players.iter_mut()
    {
        // Лежачего не двигаем (падение/стан, T-мех).
        if knocked.is_some() {
            move_vel.0 = Vector::ZERO;
            velocity.0 = Vector::ZERO;
            continue;
        }
        let wish = Vec2::from_array(input.direction).normalize_or_zero();
        // Спринт-тоггл (`SprinterComponent.Sprinting`) умножает обе скорости
        // на ×1.45 — как в сборке.
        let sprinting = sprinting_flag.is_some_and(|flag| flag.0);
        // Выносливость (PORT_PLAN 2.3, числа из StaminaComponent/SharedStaminaSystem):
        // бег тратит 8/с, восстановление 5/с и только через 5 с после траты,
        // крит — падение на 6 с и Blunt 10.
        if let Some(mut stamina) = stamina {
            // Трата идёт от спринт-тоггла (`SprinterComponent`), а не от бега
            // по умолчанию: бег у человека бесплатный, платит он за спринт.
            let crit = stamina.tick(dt, now, sprinting, wish != Vec2::ZERO);
            if crit {
                let stun = stamina.enter_crit(now);
                tracing::info!(player = ?player, "stamina crit: knockdown");
                commands.entity(player).insert(KnockedDown {
                    seconds: stun.as_secs_f32(),
                });
                damage_events.write(DamageEvent {
                    target: player,
                    amount: ssr_core::stamina::SPRINT_BREAK_DAMAGE as i32,
                    source: DamageSource::Environment {
                        cause: "sprint break",
                    },
                });
            }
        }
        // В SS14 спринт по умолчанию: Shift включает ХОДЬБУ, а не бег.
        // Призрак летает по `observer.yml`: ходьба 8, бег 12 тайлов/с
        // (`baseWalkSpeed`/`baseSprintSpeed` у `Incorporeal`).
        let base_speed = if ghost.is_some() {
            if input.running {
                OBSERVER_WALK_SPEED * TILE_SIZE
            } else {
                OBSERVER_SPRINT_SPEED * TILE_SIZE
            }
        } else if input.running {
            PLAYER_WALK_SPEED
        } else {
            PLAYER_MOVE_SPEED
        };
        // Замедление от урона (`SlowOnDamage` в `Species/base.yml`):
        // урон ≥60 → ×0.7, ≥80 → ×0.5 (числа из сборки).
        let damage = health
            .map(|health| (health.max - health.current).max(0) as f32)
            .unwrap_or(0.0);
        let damage_mult = if damage >= SLOW_ON_DAMAGE_HIGH {
            SLOW_ON_DAMAGE_HIGH_MULT
        } else if damage >= SLOW_ON_DAMAGE_LOW {
            SLOW_ON_DAMAGE_LOW_MULT
        } else {
            1.0
        };
        let wish_speed = base_speed
            * if sprinting {
                ssr_core::stamina::SPRINT_SPEED_MULT
            } else {
                1.0
            }
            // Тянуть тяжело и ходить, и бежать: ×0.95 (`PullerComponent`).
            * if pulling.is_some() {
                ssr_core::pull::PULL_SPEED_MODIFIER
            } else {
                1.0
            }
            * damage_mult;
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

/// Ведёт тянумую сущность за игроком (`PullingSystem` сборки): сустав длиной
/// `length` с запасом 0.15 м — внутри диапазона объект стоит, за границей
/// подтягивается. Разрыв: стан тянущего, пропажа цели, слишком большое
/// расхождение (телепорт).
fn pull_follow(
    mut commands: Commands,
    mut pullers: Query<(Entity, &mut Pulling, &PlayerPosition, Option<&KnockedDown>)>,
    mut targets: Query<(&mut ItemPosition, Option<&mut Position>), With<PulledBy>>,
) {
    for (player, pulling, player_position, knocked) in pullers.iter_mut() {
        let Ok((mut item_position, body)) = targets.get_mut(pulling.target) else {
            commands.entity(player).remove::<Pulling>();
            continue;
        };
        if knocked.is_some() {
            commands.entity(pulling.target).remove::<PulledBy>();
            commands.entity(player).remove::<Pulling>();
            continue;
        }
        let player_position = Vec2::from_array(player_position.0);
        let target_position = Vec2::from_array(item_position.0);
        let diff = player_position - target_position;
        let distance = diff.length();
        if distance > ssr_core::pull::PULL_BREAK_UNITS {
            commands.entity(pulling.target).remove::<PulledBy>();
            commands.entity(player).remove::<Pulling>();
            continue;
        }
        let limit = pulling.length + ssr_core::pull::PULL_SLACK_UNITS;
        if distance > limit {
            let direction = diff / distance.max(1e-4);
            let next = player_position - direction * pulling.length;
            item_position.0 = [next.x, next.y];
            if let Some(mut body) = body {
                body.0 = Vector::new(next.x, next.y);
            }
        }
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

/// Кому `sync_item_rooms` выдаёт комнату видимости: предметам, структурам
/// (столам), снарядам и звукам — у остальных сущностей комнату ставит свой код.
type RoomAssignable = Or<(
    With<Item>,
    With<ssr_core::structures::Structure>,
    With<weapons::Projectile>,
    With<weapons::WorldSound>,
    With<weapons::WorldEffect>,
)>;

/// Выдаёт предметам комнату якоря: держателя (руки/рюкзак), ящика или чанка
/// под лежащим предметом. Иначе `Rooms::default()` (пустой набор) делает предмет
/// невидимым всем клиентам — иконки в UI и модель в руке не приходят (T-мех).
/// Структуры (столы) идут тем же путём: без комнаты они не реплицируются.
#[allow(clippy::type_complexity)]
fn sync_item_rooms(
    mut commands: Commands,
    items: Query<
        (
            Entity,
            Option<&HeldBy>,
            Option<&ItemPosition>,
            Option<&ItemRoom>,
        ),
        RoomAssignable,
    >,
    holders: Query<&ItemRoom, With<PlayerPosition>>,
    containers: Query<(&Inventory, &ItemRoom), With<Container>>,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
) {
    for (entity, held, position, current) in items.iter() {
        let holder = held.filter(|held| held.player != 0).map(|held| held.player);
        let room = if let Some(holder_bits) = holder {
            Entity::try_from_bits(holder_bits)
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

/// Спящие тела (Ssd) остаются видимыми: их комнаты больше не обновляет
/// `update_client_rooms` (управление ушло к наблюдателю), поэтому комната
/// интереса держится по СВОЕМУ чанку — как у предметов.
fn sync_ssd_body_rooms(
    mut commands: Commands,
    bodies: Query<
        (
            Entity,
            &PlayerPosition,
            Option<&ItemRoom>,
        ),
        With<ssr_core::mechanics::Ssd>,
    >,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
) {
    for (entity, position, current) in bodies.iter() {
        let room = ItemRoom(chunk_rooms.room_for(
            chunk_coords(position.0[0], position.0[1]),
            &mut allocator,
        ));
        if current == Some(&room) {
            continue;
        }
        commands
            .entity(entity)
            .insert((room, Rooms::single(room.0)));
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

/// Спавн сущностей карты (лампы, мебель, шкафы, предметы) тем же правилом,
/// что и команда `spawn`: структура из ProtoCatalog — Structure, прототип
/// есть — предмет на полу, `light` — лампа энергосистемы.
fn spawn_map_entities(
    mut commands: Commands,
    map: Res<GameMap>,
    prototypes: Res<ProtoCatalog>,
    mut chunk_rooms: ResMut<ChunkRooms>,
    mut allocator: ResMut<RoomAllocator>,
) {
    let mut structures = 0usize;
    let mut items = 0usize;
    let mut lights = 0usize;
    for (id, x, y) in &map.entities {
        let room = ItemRoom(chunk_rooms.room_for(chunk_coords(*x, *y), &mut allocator));
        if id == "light" {
            commands.spawn((
                Light::default(),
                Consumer {
                    draw_kw: ssr_core::power::LIGHT_DRAW_KW,
                },
                Powered(true),
                ItemPosition([*x, *y]),
                Replicate::to_clients(NetworkTarget::All),
                Rooms::single(room.0),
            ));
            lights += 1;
            continue;
        }
        if let Some(info) = prototypes.structure(id).cloned() {
            let mut entity = commands.spawn((
                ssr_core::structures::Structure {
                    proto: id.clone(),
                    smooth: info.smooth.clone(),
                    surface: info.surface,
                    solid: info.solid,
                },
                ItemPosition([*x, *y]),
                Replicate::to_clients(NetworkTarget::All),
                Rooms::single(room.0),
            ));
            if info.solid {
                entity.insert((
                    RigidBody::Static,
                    Collider::rectangle(info.half.0 * 2.0, info.half.1 * 2.0),
                    Position(Vector::new(*x, *y)),
                    Rotation::default(),
                ));
            }
            structures += 1;
            continue;
        }
        if prototypes.contains(id) {
            commands.spawn((
                Item {
                    name: id.clone(),
                },
                HeldBy { player: 0 },
                ItemPosition([*x, *y]),
                Replicate::to_clients(NetworkTarget::All),
                Rooms::single(room.0),
            ));
            items += 1;
        }
    }
    tracing::info!(structures, items, lights, "map entities spawned");
}

/// Запросы здоровья/доступа/питания для обработки действий (сокращает
/// число аргументов системы: у функций-систем лимит 16 параметров).
#[derive(bevy::ecs::system::SystemParam)]
struct ActionQueries<'w, 's> {
    healths: Query<'w, 's, &'static mut Health>,
    clothings: Query<'w, 's, &'static mut Clothing>,
    /// Хранилища предметов (пояса/сумки) и авто-закрытие дверей — здесь, чтобы
    /// у `process_actions` не превышался предел числа параметров (16).
    item_storages: Query<'w, 's, &'static mut ItemStorage>,
    door_autos: Query<'w, 's, &'static mut DoorAuto>,
    access: Query<'w, 's, &'static Access>,
    powered: Query<'w, 's, &'static Powered>,
    pulling: Query<'w, 's, &'static Pulling>,
    pulled_by: Query<'w, 's, &'static PulledBy>,
    atmospheres: Res<'w, Atmospheres>,
    catalogs: Res<'w, ContentCatalog>,
    /// Размеры предметов из прототипов сборки (`item_size.yml`).
    prototypes: Res<'w, ProtoCatalog>,
    /// Призраки — проверка состояния для админ-верба `aghost`.
    ghosts: Query<'w, 's, &'static Ghost>,
    /// Наблюдатели — возврат в тело (верб aghost).
    observers: Query<'w, 's, &'static GhostObserver>,
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

/// Игроки для системы движения: ввод, скорость, выносливость, лежание.
type MovingPlayers<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static PlayerInput,
        &'static mut LinearVelocity,
        &'static mut MoveVel,
        Option<&'static KnockedDown>,
        Option<&'static mut ssr_core::stamina::Stamina>,
        Option<&'static Sprinting>,
        Option<&'static Health>,
        Option<&'static Pulling>,
        // Призрак (Content.Shared/Ghost/GhostComponent.cs): скорости берутся
        // из observer.yml (8 ходьба / 12 бег тайлов в секунду).
        Option<&'static Ghost>,
    ),
>;

/// Состояние спринта для обработчика сообщений: часы, кулдауны и блокирующие
/// состояния (лежание, призрак) — одним `SystemParam`, иначе у системы
/// превышается предел числа параметров.
#[derive(bevy::ecs::system::SystemParam)]
struct SprintState<'w, 's> {
    clock: Res<'w, StaminaClock>,
    cooldowns: ResMut<'w, SprintCooldowns>,
    knockeds: Query<'w, 's, &'static KnockedDown>,
    ghosts: Query<'w, 's, &'static Ghost>,
}

/// Пауза между спринтами (`SprinterComponent.TimeBetweenSprints = 3` с):
/// когда игроку снова можно включить спринт.
#[derive(Resource, Default)]
struct SprintCooldowns(std::collections::HashMap<Entity, f32>);

// Замедление от урона — пороги и множители из `Species/base.yml` (`SlowOnDamage`).
const SLOW_ON_DAMAGE_LOW: f32 = 60.0;
const SLOW_ON_DAMAGE_HIGH: f32 = 80.0;
const SLOW_ON_DAMAGE_LOW_MULT: f32 = 0.7;
const SLOW_ON_DAMAGE_HIGH_MULT: f32 = 0.5;

/// Часы выносливости: единая шкала времени для трат, пауз и буферов
/// (`SharedStaminaSystem` работает по `Timing.CurTime`).
#[derive(Resource, Default)]
struct StaminaClock {
    seconds: f32,
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
    mut world: ActionQueries,
    mut doors: Query<&mut Door>,
    mut containers: Query<&mut Container>,
    container_positions: Query<&ItemPosition>,
    // То же, но с сущностями — для всасывания предметов в ящик (EntityStorage).
    floor_positions: Query<(Entity, &ItemPosition)>,
    items: Query<&Item>,
    mut senders: Query<&mut MessageSender<ServerMessage>, With<Connected>>,
    mut players: ResMut<Players>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    if queue.0.is_empty() {
        return;
    }
    let pending = std::mem::take(&mut queue.0);
    for (player, queued) in pending {
        let action = match queued {
            QueuedAction::Do(action) => {
                // Призрак не взаимодействует: в сборке `SharedGhostSystem`
                // отменяет `UseAttemptEvent`, `InteractionAttemptEvent`,
                // `DropAttemptEvent`, `PickupAttemptEvent`,
                // `InteractionVerbAttemptEvent`, если `!CanGhostInteract`
                // (у `MobObserver` он `false`). Осмотр и VV разрешены.
                if world.ghosts.get(player).is_ok()
                    && !matches!(
                        action,
                        ActionKind::Examine { .. } | ActionKind::ViewVariables { .. }
                    )
                {
                    tracing::debug!(?player, "ghost: взаимодействие запрещено");
                    continue;
                }
                action
            }
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
                // Сбор вербов — по правилам `SharedVerbSystem.GetLocalVerbs`:
                // `CanAccess` (дистанция `InteractionRange = 1.5` тайла) и
                // `CanInteract`, `Using` — предмет в активной руке.
                let player_position = positions.get(player).ok().map(|p| p.0);
                let hand_item = hands.get(player).ok().and_then(|h| h.active_item());
                // Тайл занят стеной (для проверки видимости) — по индексу чанков.
                let solid_at = |x: f32, y: f32| -> bool {
                    let chunk_tiles = ssr_core::tiles::CHUNK_TILES as i32;
                    let tx = (x / TILE_SIZE).floor() as i32;
                    let ty = (y / TILE_SIZE).floor() as i32;
                    let coords = (tx.div_euclid(chunk_tiles), ty.div_euclid(chunk_tiles));
                    let Some(entity) = index.chunks.get(&coords).copied() else {
                        return true;
                    };
                    let Ok(chunk) = chunks.get(entity) else {
                        return true;
                    };
                    let lx = tx.rem_euclid(chunk_tiles) as usize;
                    let ly = ty.rem_euclid(chunk_tiles) as usize;
                    ssr_core::occluders::is_solid(chunk.tiles[ly * chunk_tiles as usize + lx])
                };
                // `InRangeUnobstructed` (`SharedInteractionSystem.cs:651-693`):
                // цель должна быть в 1.5 тайла И не за стеной.
                let in_range = |target: Entity| {
                    let (Some(user), Ok(target_position)) =
                        (player_position, container_positions.get(target))
                    else {
                        return false;
                    };
                    let dx = target_position.0[0] - user[0];
                    let dy = target_position.0[1] - user[1];
                    if dx * dx + dy * dy > (INTERACT_RANGE * 1.5) * (INTERACT_RANGE * 1.5) {
                        return false;
                    }
                    let from = [user[0], user[1]];
                    let to = target_position.0;
                    let steps = (((to[0] - from[0]).abs().max((to[1] - from[1]).abs()))
                        / (TILE_SIZE * 0.5))
                        .ceil()
                        .max(1.0) as u32;
                    let target_tile = (
                        (to[0] / TILE_SIZE).floor() as i32,
                        (to[1] / TILE_SIZE).floor() as i32,
                    );
                    let mut last_tile = (
                        (from[0] / TILE_SIZE).floor() as i32,
                        (from[1] / TILE_SIZE).floor() as i32,
                    );
                    for step in 1..steps {
                        let t = step as f32 / steps as f32;
                        let x = from[0] + (to[0] - from[0]) * t;
                        let y = from[1] + (to[1] - from[1]) * t;
                        let tile = (
                            (x / TILE_SIZE).floor() as i32,
                            (y / TILE_SIZE).floor() as i32,
                        );
                        if tile == last_tile {
                            continue;
                        }
                        last_tile = tile;
                        if tile == target_tile {
                            break;
                        }
                        if solid_at(x, y) {
                            return false;
                        }
                    }
                    true
                };
                if entity != 0 {
                    if let Some(target) = Entity::try_from_bits(entity) {
                        // Health → верб «Ударить» (в сборке это боевой режим, у нас
                        // дублируем вербом: `InteractionVerb`).
                        if world.healths.get(target).is_ok() && in_range(target) {
                            options.push(ActionOption {
                                label: "Ударить".into(),
                                action: ActionKind::Attack { target: entity },
                                ..verb_default(ssr_core::verbs::VerbType::Interaction)
                            });
                        }
                        // Экипировка цели (SS14: ПКМ по игроку показывает
                        // вербы его одежды — `StrippingSystem`/InventorySystem):
                        // «Снять» (только с себя), «Осмотреть» каждый предмет.
                        if let Ok(clothing) = world.clothings.get(target) {
                            for (slot, item_bits) in &clothing.slots {
                                let Some(item_entity) = Entity::try_from_bits(*item_bits) else {
                                    continue;
                                };
                                let item_name = items
                                    .get(item_entity)
                                    .map(|item| item.name.clone())
                                    .unwrap_or_default();
                                let mut unequip = ActionOption {
                                    label: format!("Снять: {item_name}"),
                                    action: ActionKind::Unequip {
                                        slot: slot.id().to_string(),
                                    },
                                    icon: Some(
                                        "Interface/VerbIcons/equip.svg.192dpi.png".to_string(),
                                    ),
                                    ..verb_default(ssr_core::verbs::VerbType::Interaction)
                                };
                                if target != player {
                                    // Стриппинг чужой одежды — StrippingComponent,
                                    // в порте ещё нет: верб виден, но отключён.
                                    unequip.disabled = true;
                                    unequip.message = Some("Можно снимать только с себя".into());
                                }
                                options.push(unequip);
                                options.push(ActionOption {
                                    label: format!("Осмотреть: {item_name}"),
                                    action: ActionKind::Examine {
                                        entity: *item_bits,
                                    },
                                    icon: Some(
                                        "Interface/VerbIcons/examine.svg.192dpi.png".into(),
                                    ),
                                    priority: 10,
                                    close_menu: Some(false),
                                    ..verb_default(ssr_core::verbs::VerbType::Examine)
                                });
                            }
                        }
                        // Предмет: «Взять» (`AddPickupVerb`) и «Осмотреть»
                        // (`AddExamineVerb`, категория Examine, приоритет 10).
                        if let Ok(item) = items.get(target) {
                            let held = hand_item.is_some();
                            let mut pickup = ActionOption {
                                label: "Взять".into(),
                                action: ActionKind::Pickup { item: entity },
                                icon: Some("Interface/VerbIcons/pickup.svg.192dpi.png".to_string()),
                                ..verb_default(ssr_core::verbs::VerbType::Interaction)
                            };
                            // В сборке верб пропадает, если руки заняты или
                            // предмет уже в руке (`args.Using != null`).
                            if held {
                                pickup.disabled = true;
                                pickup.message = Some("Руки заняты".into());
                            }
                            if !in_range(target) {
                                pickup.disabled = true;
                                pickup.message = Some("Слишком далеко".into());
                            }
                            options.push(pickup);
                            options.push(ActionOption {
                                label: "Осмотреть".into(),
                                action: ActionKind::Examine { entity },
                                icon: Some("Interface/VerbIcons/examine.svg.192dpi.png".into()),
                                priority: 10,
                                close_menu: Some(false),
                                ..verb_default(ssr_core::verbs::VerbType::Examine)
                            });
                            let _ = item;
                        }
                        // Тянуть: верб ставит `PullingSystem.AddPullVerbs` —
                        // обычный `Verb` БЕЗ категории и иконки, поэтому в меню
                        // он всплывает выше категоризированных.
                        let pullable = container_positions.get(target).is_ok()
                            && (containers.get(target).is_ok()
                                || items
                                    .get(target)
                                    .map(|item| world.catalogs.items.pullable(&item.name))
                                    .unwrap_or(false));
                        if pullable {
                            let currently = world
                                .pulling
                                .get(player)
                                .ok()
                                .is_some_and(|pulling| pulling.target == target);
                            options.push(ActionOption {
                                label: if currently {
                                    "Отпустить"
                                } else {
                                    "Тянуть"
                                }
                                .into(),
                                action: ActionKind::Pull { target: entity },
                                ..verb_default(ssr_core::verbs::VerbType::Verb)
                            });
                        }
                        // Дверь: открыть/закрыть (`AddToggleOpenVerb`, иконки
                        // `open.svg`/`close.svg`).
                        if let Ok(door) = doors.get(target)
                            && has_door_access(&world.access, player, door)
                        {
                            let (text, icon) = if door.open {
                                ("Закрыть", "Interface/VerbIcons/close.svg.192dpi.png")
                            } else {
                                ("Открыть", "Interface/VerbIcons/open.svg.192dpi.png")
                            };
                            options.push(ActionOption {
                                label: text.into(),
                                action: ActionKind::Interact { entity },
                                icon: Some(icon.into()),
                                ..verb_default(ssr_core::verbs::VerbType::Interaction)
                            });
                        }
                        // Админ-верб «Стать призраком» (`aghost` в сборке —
                        // кнопка админ-меню; у нас ещё и вербом по себе).
                        if target == player {
                            options.push(
                                ActionOption {
                                    label: "Стать призраком".into(),
                                    action: ActionKind::AdminGhost,
                                    icon: Some(
                                        "Interface/VerbIcons/sentient.svg.192dpi.png".into(),
                                    ),
                                    ..verb_default(ssr_core::verbs::VerbType::Verb)
                                }
                                .in_category(ssr_core::verbs::VerbCategory::Admin),
                            );
                        }
                        // Админ/дебаг-вербы — как `AdminVerbSystem` (категории
                        // Admin/Debug) и `VvVerb` (всегда первый в меню).
                        options.push(ActionOption {
                            label: "View Variables".into(),
                            action: ActionKind::ViewVariables { entity },
                            icon: Some("Interface/VerbIcons/vv.svg.192dpi.png".into()),
                            client_exclusive: false,
                            ..verb_default(ssr_core::verbs::VerbType::ViewVariables)
                        });
                        options.push(
                            ActionOption {
                                label: "Удалить".into(),
                                action: ActionKind::Delete { entity },
                                icon: Some(
                                    "Interface/VerbIcons/delete_transparent.svg.192dpi.png".into(),
                                ),
                                confirmation_popup: true,
                                ..verb_default(ssr_core::verbs::VerbType::Verb)
                            }
                            .in_category(ssr_core::verbs::VerbCategory::Debug),
                        );
                        options.push(
                            ActionOption {
                                label: "Оживить".into(),
                                action: ActionKind::Rejuvenate { entity },
                                icon: Some("Interface/VerbIcons/rejuvenate.svg.192dpi.png".into()),
                                ..verb_default(ssr_core::verbs::VerbType::Verb)
                            }
                            .in_category(ssr_core::verbs::VerbCategory::Debug),
                        );
                    }
                } else {
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
                    if let (Some(item), Some(tile)) = (hand_item, tile)
                        && item_name == "SteelSheet"
                        && tile.is_walkable()
                    {
                        options.push(ActionOption {
                            label: "Построить стену".into(),
                            action: ActionKind::UseItem { item, tx, ty },
                            ..verb_default(ssr_core::verbs::VerbType::Interaction)
                        });
                        // Лом НЕ разбирает стены: в сборке стена разбирается
                        // строительством (`Construction` с инструментами), а
                        // `Crowbar` умеет только `Prying` (двери и половые плитки).
                    }
                }
                // Порядок — как `SortedSet<Verb>` в сборке (`Verb.CompareTo`).
                options.sort_by(|left, right| {
                    left.sort_key()
                        .cmp(&right.sort_key())
                        .then_with(|| left.label.cmp(&right.label))
                });
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
            // Верб «Тянуть»/«Отпустить» (Pull): цель следует за игроком на
            // длине «верёвки» — как distance-сустав в `PullingSystem` сборки.
            ActionKind::Pull { target } => {
                let Some(entity) = Entity::try_from_bits(target) else {
                    continue;
                };
                // Повторное действие по той же цели — отпустить.
                if world
                    .pulling
                    .get(player)
                    .ok()
                    .is_some_and(|pulling| pulling.target == entity)
                {
                    commands.entity(entity).remove::<PulledBy>();
                    commands.entity(player).remove::<Pulling>();
                    tracing::info!(?player, ?entity, "pull released (verb)");
                    continue;
                }
                // Тянул что-то другое — прежнюю цель отпускаем.
                if let Ok(pulling) = world.pulling.get(player) {
                    commands.entity(pulling.target).remove::<PulledBy>();
                }
                let (Ok(target_position), Ok(player_position)) =
                    (container_positions.get(entity), positions.get(player))
                else {
                    continue;
                };
                let diff =
                    Vec2::from_array(target_position.0) - Vec2::from_array(player_position.0);
                let distance = diff.length();
                if distance > INTERACT_RANGE + TILE_SIZE {
                    tracing::warn!(?player, ?entity, "pull: too far");
                    continue;
                }
                // Цель уже кто-то тянет — прежний тянущий отпускает (перехват
                // в сборке разрешён, `TryStartPull` меняет владельца).
                if let Ok(previous) = world.pulled_by.get(entity) {
                    commands.entity(previous.puller).remove::<Pulling>();
                }
                commands.entity(entity).insert(PulledBy { puller: player });
                commands.entity(player).insert(Pulling {
                    target: entity,
                    length: distance.max(ssr_core::pull::PULL_MIN_LENGTH_UNITS),
                });
                tracing::info!(?player, ?entity, distance, "pull started");
                continue;
            }
            // Верб «Осмотреть»: описание объекта, как у очереди Examine.
            ActionKind::Examine { entity } => {
                let text = describe_target(
                    entity,
                    0,
                    0,
                    &items,
                    &containers,
                    &container_positions,
                    &doors,
                    &world.atmospheres,
                    &world.catalogs,
                );
                tracing::info!(?player, %text, "examine (verb)");
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
            // Верб «Взять»: поднять предмет с пола (та же проверка, что у
            // ClientMessage::Pickup — дистанция и «лежит на полу»).
            ActionKind::Pickup { item } => {
                let Some(entity) = Entity::try_from_bits(item) else {
                    continue;
                };
                let Ok(player_position) = positions.get(player) else {
                    continue;
                };
                let Ok(item_position) = container_positions.get(entity) else {
                    tracing::warn!(item, "verb pickup: item is not on the floor");
                    continue;
                };
                let dx = item_position.0[0] - player_position.0[0];
                let dy = item_position.0[1] - player_position.0[1];
                if (dx * dx + dy * dy).sqrt() > INTERACT_RANGE + TILE_SIZE {
                    tracing::warn!(item, "verb pickup: too far");
                    continue;
                }
                let name = items
                    .get(entity)
                    .map(|i| i.name.clone())
                    .unwrap_or_default();
                let (w, h) = world.catalogs.items.size_of(&name);
                let mut taken = false;
                if let Ok(mut hand) = hands.get_mut(player)
                    && hand.active_item().is_none()
                {
                    taken = hand.take_in_active(item);
                }
                if !taken && let Ok(mut inventory) = inventories.get_mut(player) {
                    taken = inventory.put_first_fit(item, w, h).is_some();
                }
                if taken {
                    commands
                        .entity(entity)
                        .remove::<ssr_core::inventory::ItemPosition>()
                        .insert(ssr_core::inventory::HeldBy {
                            player: player.to_bits(),
                        });
                    tracing::info!(item, "verb pickup: taken");
                }
            }
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
                        weapon: weapon.clone(),
                    },
                });
                // Дуга удара в точке цели (`WeaponArc` в сборке: fist — кулак,
                // claw — предмет). Видна всем, деспавнится по lifetime.
                let arc_state = if weapon.is_some() { "claw" } else { "fist" };
                commands.spawn((
                    ssr_core::weapons::WorldEffect {
                        position: target_position.0,
                        key: format!("sprites/ss14/Effects/arcs.rsi#{arc_state}"),
                        lifetime: 0.4,
                    },
                    Replicate::to_clients(NetworkTarget::All),
                    Rooms::default(),
                ));
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
            // Админ-верб «Удалить» (`delete-verb-get-data-text`, категория Debug):
            // удаляем сущность (в сборке — с подтверждением).
            ActionKind::Delete { entity } => {
                let Some(target) = Entity::try_from_bits(entity) else {
                    continue;
                };
                tracing::info!(?player, ?target, "admin verb: delete");
                commands.entity(target).despawn();
            }
            // Дебаг-верб «Оживить»: полное лечение (`rejuvenate` в сборке лечит,
            // снимает станы и чинит — у нас лечим и поднимаем).
            ActionKind::Rejuvenate { entity } => {
                let Some(target) = Entity::try_from_bits(entity) else {
                    continue;
                };
                if let Ok(mut health) = world.healths.get_mut(target) {
                    tracing::info!(?player, ?target, "debug verb: rejuvenate");
                    // `rejuvenate` в сборке лечит полностью и снимает станы.
                    health.heal();
                }
                commands.entity(target).remove::<KnockedDown>();
            }
            // Админ-верб «Стать призраком» (`aghost`): переводим игрока в
            // состояние призрака — сквозь стены и невидимым для живых.
            ActionKind::AdminGhost => {
                // Единая модель с командой ghost: наблюдатель + тело в SSD.
                // Раньше верб снимал только Ghost — ColliderDisabled оставался
                // и «вернувшееся» тело продолжало летать сквозь стены.
                let Some(entry) = players.entries.iter_mut().find(|entry| entry.player == player)
                else {
                    continue;
                };
                if world.ghosts.get(player).is_ok() {
                    if return_to_body(&mut commands, entry, &world.observers) {
                        for mut sender in senders.iter_mut() {
                            sender.send::<GameChannel>(ServerMessage::Welcome {
                                player_entity: entry.player.to_bits(),
                                protocol_version: PROTOCOL_VERSION,
                            });
                        }
                    }
                    tracing::info!(?player, "aghost: вернулся в тело");
                } else {
                    let observer = become_ghost(&mut commands, entry, &positions);
                    for mut sender in senders.iter_mut() {
                        sender.send::<GameChannel>(ServerMessage::Welcome {
                            player_entity: observer.to_bits(),
                            protocol_version: PROTOCOL_VERSION,
                        });
                    }
                    tracing::info!(?player, "aghost: стал призраком");
                }
            }
            // Верб «View Variables»: отдаём клиенту снимок компонентов и полей
            // сущности (упрощённый `ViewVariablesBlobMembers` сборки).
            ActionKind::ViewVariables { entity } => {
                let Some(target) = Entity::try_from_bits(entity) else {
                    continue;
                };
                let dump = describe_components(target, &items, &containers, &doors);
                if let Some(link) = players
                    .entries
                    .iter()
                    .find(|entry| entry.player == player)
                    .map(|entry| entry.link)
                    && let Ok(mut sender) = senders.get_mut(link)
                {
                    sender.send::<GameChannel>(ServerMessage::Event {
                        kind: format!("vv:{entity}:{dump}"),
                    });
                }
                tracing::info!(?player, ?target, "vv opened");
            }
            // Двери и контейнеры: открыть/закрыть (`Interact`).
            ActionKind::Interact { entity } => {
                let Some(target) = Entity::try_from_bits(entity) else {
                    tracing::warn!(bits = entity, "interact: invalid entity bits");
                    continue;
                };
                let Ok(player_position) = positions.get(player) else {
                    continue;
                };
                let Ok(mut door) = doors.get_mut(target) else {
                    // Хранилище предмета (`StorageComponent`): пояс/сумка —
                    // окно StorageWindow, содержимое НЕ высыпается. Открыть
                    // может носитель (предмет надет) — позиций у него нет.
                    if let Ok(mut storage) = world.item_storages.get_mut(target) {
                        storage.open = !storage.open;
                        tracing::info!(item = ?target, open = storage.open, "item storage toggled");
                        continue;
                    }
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
                        let open = container.open;
                        // Коллизия: закрытый ящик не проходим, открытый — проходим
                        // (`IsCollidableWhenOpen = false` в `EntityStorageComponent`).
                        if open {
                            commands.entity(target).insert(ColliderDisabled);
                        } else {
                            commands.entity(target).remove::<ColliderDisabled>();
                        }
                        // Ящик — `EntityStorage` в сборке: у него НЕТ сеточного
                        // окна. Открытие ВЫСЫПАЕТ содержимое на пол
                        // (`OpenStorage` → `EmptyContents`), закрытие ВСАСЫВАЕТ
                        // предметы, лежащие рядом (`CloseStorage` →
                        // `GetEntitiesInRange(EnteringOffset, EnteringRange)`).
                        // Предметы кладут в открытый ящик перетаскиванием на него
                        // (наш аналог `PlaceableSurface` у открытой крышки).
                        let items_inside: Vec<u64> = inventories
                            .get(target)
                            .map(|inventory| inventory.cells.iter().flatten().copied().collect())
                            .unwrap_or_default();
                        if open {
                            if let Ok(mut inventory) = inventories.get_mut(target) {
                                let origin = container_positions
                                    .get(target)
                                    .map(|position| position.0)
                                    .unwrap_or([0.0, 0.0]);
                                for (index, item) in items_inside.iter().enumerate() {
                                    inventory.take(*item);
                                    let Some(entity) = Entity::try_from_bits(*item) else {
                                        continue;
                                    };
                                    // Раскладываем вокруг ящика, чтобы вещи не
                                    // слиплись в одну точку (в движке они ложатся
                                    // в `worldPos + EnteringOffset`).
                                    let angle = index as f32 * 0.7;
                                    let offset = [
                                        origin[0] + angle.cos() * CONTAINER_SPILL_RADIUS,
                                        origin[1] + angle.sin() * CONTAINER_SPILL_RADIUS,
                                    ];
                                    commands
                                        .entity(entity)
                                        // `HeldBy { player: 0 }` — «ничей»: клиент
                                        // рисует предмет на полу только при
                                        // наличии `HeldBy` (иначе он не попадает
                                        // в его запрос и вещь не видно).
                                        .insert((HeldBy { player: 0 }, ItemPosition(offset)));
                                }
                                tracing::info!(
                                    container = ?target,
                                    spilled = items_inside.len(),
                                    "container opened: contents spilled"
                                );
                            }
                        } else {
                            // Всасывание: предметы с пола в радиусе вокруг ящика.
                            // `ItemPosition` есть только у лежащего в мире
                            // (взятую вещь `Pickup` его лишает), поэтому «на полу»
                            // = есть `ItemPosition` и есть `Item`, а сам ящик
                            // отсеивается проверкой `Item`.
                            let Ok(origin) =
                                container_positions.get(target).map(|position| position.0)
                            else {
                                continue;
                            };
                            let mut nearby: Vec<u64> = Vec::new();
                            let mut floor_count = 0usize;
                            let mut min_dist = f32::MAX;
                            for (entity, position) in floor_positions.iter() {
                                floor_count += 1;
                                if entity == target || items.get(entity).is_err() {
                                    continue;
                                }
                                let dx = position.0[0] - origin[0];
                                let dy = position.0[1] - origin[1];
                                let dist = (dx * dx + dy * dy).sqrt();
                                min_dist = min_dist.min(dist);
                                if dist <= CONTAINER_ENTERING_RANGE {
                                    nearby.push(entity.to_bits());
                                }
                            }
                            tracing::debug!(
                                floor = floor_count,
                                min_dist,
                                range = CONTAINER_ENTERING_RANGE,
                                candidates = nearby.len(),
                                "container absorb scan"
                            );
                            let mut absorbed = 0usize;
                            if let Ok(mut inventory) = inventories.get_mut(target) {
                                for item in nearby {
                                    let (w, h) = item_size_of(
                                        &world.catalogs,
                                        &world.prototypes,
                                        &items,
                                        item,
                                    );
                                    if inventory.put_first_fit(item, w, h).is_none() {
                                        break; // ящик полон (Capacity в движке)
                                    }
                                    if let Some(entity) = Entity::try_from_bits(item) {
                                        commands.entity(entity).remove::<ItemPosition>();
                                    }
                                    absorbed += 1;
                                }
                            }
                            tracing::info!(
                                container = ?target, absorbed, "container closed: nearby items stored"
                            );
                        }
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
                    // Ручное открытие тоже запускает авто-закрытие (`DoorSystem`:
                    // secondsUntilAutoclose), иначе дверь стояла бы открытой.
                    if let Ok(mut auto) = world.door_autos.get_mut(target) {
                        auto.close_in = AUTO_CLOSE_SECS;
                    }
                } else {
                    commands.entity(target).remove::<ColliderDisabled>();
                }
                tracing::info!(door = ?target, open = door.open, "door toggled");
            }
            ActionKind::Unequip { slot } => {
                // Верб «Снять» (ПКМ по себе → вербы экипировки): тот же каскад,
                // что и ClientMessage::Unequip (`TryUnequip` в сборке).
                let Some(slot) = ClothingSlot::from_id(&slot) else {
                    continue;
                };
                unequip_slot(
                    &mut commands,
                    &mut world.clothings,
                    &mut hands,
                    &positions,
                    player,
                    slot,
                );
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
                    // Лом (`BaseCrowbar`, качество `Prying`) стен НЕ разбирает:
                    // в сборке обычную стену снимают сваркой (10 с) → ключом с
                    // якоря → отвёрткой (2 с). Лом срывает только ПОЛОВЫЕ ПЛИТКИ
                    // (`BaseStationTile.deconstructTools: [Prying]`,
                    // `baseTurf: Plating`); обшивку (Plating) он не берёт — там
                    // качество `Axing` (топор).
                    if chunk.tiles[cell] != TileType::Floor {
                        tracing::warn!(
                            tx, ty, tile = ?chunk.tiles[cell],
                            "crowbar: снимаются только половые плитки (Prying)"
                        );
                        continue;
                    }
                    chunk.tiles[cell] = TileType::Plating;
                    tracing::info!(tx, ty, "floor tile pried (baseTurf: plating)");
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
            // Снаряд: убийца — стрелявший, «оружие» — прототип снаряда.
            DamageSource::Projectile { shooter, weapon } => {
                (Some(*shooter), weapon.as_deref().unwrap_or("projectile"))
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

/// Смерть (T4.1) как в SS14: тело ОСТАЁТСЯ лежать (никакого респавна и
/// телепорта на спавн — жалоба владельца), управление переходит призраку
/// (наблюдатель, как `GhostSystem.OnGhostStartup`).
#[allow(clippy::too_many_arguments)]
fn respawn_dead(
    mut commands: Commands,
    mut death_events: MessageReader<DeathEvent>,
    positions: Query<&PlayerPosition>,
    mut players: ResMut<Players>,
    names: Query<&ssr_core::mechanics::PlayerName>,
    mut senders: Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
    mut system_chat: ResMut<SystemChatQueue>,
) {
    for event in death_events.read() {
        // Системное сообщение в чат, как строка смерти в сборке
        // (`ChatSystem.SendEntitySystemMessage`, локаль `chat/*.ftl`).
        let victim = names
            .get(event.target)
            .ok()
            .map(|name| name.0.clone())
            .or_else(|| {
                players
                    .entries
                    .iter()
                    .find(|entry| entry.player == event.target)
                    .map(|entry| entry.name.clone())
            })
            .unwrap_or_else(|| "Игрок".to_string());
        let killer = event.killer.and_then(|killer| {
            names
                .get(killer)
                .ok()
                .map(|name| name.0.clone())
                .or_else(|| {
                    players
                        .entries
                        .iter()
                        .find(|entry| entry.player == killer)
                        .map(|entry| entry.name.clone())
                })
        });
        match killer {
            Some(killer) => system_chat
                .0
                .push(format!("{victim} погиб от рук {killer}")),
            None => system_chat.0.push(format!("{victim} погиб")),
        }
        // Тело остаётся на месте и лежит навсегда (мёртвое): поднять может
        // только админский heal/rejuvenate.
        commands.entity(event.target).insert(KnockedDown {
            seconds: f32::MAX,
        });
        // Управление переходит призраку-наблюдателю (OnGhostStartup в сборке).
        if let Some(entry) = players
            .entries
            .iter_mut()
            .find(|entry| entry.player == event.target)
        {
            let observer = become_ghost(&mut commands, entry, &positions);
            send_welcome(entry.link, observer, &mut senders);
        }
        tracing::info!(
            target = ?event.target,
            killer = ?event.killer,
            "player died: body stays, control moved to ghost"
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

/// Падение от стамина-крита выбрасывает предметы из рук: в сборке
/// `EnterStamCrit` → `TryUpdateParalyzeDuration` → `OnStunnedSuccessfully`
/// (Goobstation) → `DropHandItemsEvent`, а `HandsSystem.OnDropHandItems` бросает
/// предметы с разбросом 45° и скоростью `BaseThrowspeed 15 × [0.45..0.55]` =
/// 6.75..8.25 м/с. У нас у лежащих предметов нет физики, поэтому разброс
/// задаём смещением по направлению броска (как «улетел на полтайла»).
fn drop_hands_on_stam_crit(
    mut commands: Commands,
    mut hands: Query<(&mut Hands, &Position, Option<&ssr_core::stamina::Stamina>)>,
    items: Query<&Item>,
    knocked: Query<Entity, Added<KnockedDown>>,
) {
    for player in knocked.iter() {
        let Ok((mut hand, position, stamina)) = hands.get_mut(player) else {
            continue;
        };
        // Только стамина-крит: при обычном нокдауне от урона руки не пустеют.
        if !stamina.is_some_and(|stamina| stamina.critical) {
            continue;
        }
        let mut dropped = 0;
        for slot in 0..hand.slots.len() {
            let Some(item) = hand.slots.get(slot).copied().flatten() else {
                continue;
            };
            hand.slots[slot] = None;
            let Some(entity) = Entity::try_from_bits(item) else {
                continue;
            };
            if items.get(entity).is_err() {
                continue;
            }
            // Разброс ±45° вокруг направления «от игрока вбок»: у нас нет
            // вектора броска, поэтому берём фиксированные стороны (левая/правая)
            // с дистанцией из скорости броска за один тик.
            let angle = if slot == 0 { 0.0 } else { std::f32::consts::PI };
            let spread = angle + 0.25 * (slot as f32 - 0.5);
            let distance = THROW_DISTANCE;
            let offset = Vec2::new(spread.cos(), spread.sin()) * distance;
            let world = position.0 + offset;
            commands
                .entity(entity)
                .insert((HeldBy { player: 0 }, ItemPosition([world.x, world.y])));
            dropped += 1;
            tracing::info!(item, slot, "stamina crit: item dropped from hand");
        }
        if dropped > 0 {
            commands.entity(player).insert(Hands {
                active: hand.active,
                slots: hand.slots.clone(),
            });
        }
    }
}

/// Насколько далеко улетает выроненный предмет (полтайла).
const THROW_DISTANCE: f32 = crate::TILE_SIZE * 0.6;

/// Тест стамина-крита (`SSR_CRIT_TEST=1`): через 3 с выставляет стамину на/// порог, чтобы проверить падение персонажа (нокдаун 6 с, поворот на 90°).
fn stamina_crit_test(
    time: Res<Time>,
    players: Res<Players>,
    mut staminas: Query<&mut ssr_core::stamina::Stamina>,
    mut commands: Commands,
    mut done: Local<bool>,
    mut wait: Local<f32>,
) {
    if std::env::var_os("SSR_CRIT_TEST").is_none() || *done {
        return;
    }
    // Ждём подключения игрока: до него критить некого. Значение переменной —
    // задержка в секундах после подключения (по умолчанию 3).
    let delay: f32 = std::env::var("SSR_CRIT_TEST")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3.0);
    let Some(entry) = players.entries.first() else {
        *wait = 0.0;
        return;
    };
    *wait += time.delta_secs();
    if *wait < delay {
        return;
    }
    *done = true;
    if let Ok(mut stamina) = staminas.get_mut(entry.player) {
        let stun = stamina.enter_crit(time.elapsed_secs());
        commands.entity(entry.player).insert(KnockedDown {
            seconds: stun.as_secs_f32(),
        });
        tracing::info!(player = ?entry.player, "crit-test: stamina crit forced");
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

/// Снимок компонентов и полей сущности для окна View Variables — упрощённый
/// `ViewVariablesBlobMembers` из сборки: имя компонента и его поля.
fn describe_components(
    target: Entity,
    items: &Query<&Item>,
    containers: &Query<&mut Container>,
    doors: &Query<&mut Door>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Ok(item) = items.get(target) {
        parts.push(format!("Item {{ name: {} }}", item.name));
    }
    if let Ok(container) = containers.get(target) {
        parts.push(format!("Container {{ open: {} }}", container.open));
    }
    if let Ok(door) = doors.get(target) {
        parts.push(format!(
            "Door {{ open: {}, access: {:?} }}",
            door.open, door.access
        ));
    }
    if parts.is_empty() {
        parts.push("(нет известных компонентов)".to_string());
    }
    parts.join(" | ")
}

/// Верб со значениями по умолчанию (`Verb.cs`): текст пустой, приоритет 0,
/// категории и иконки нет — заполняют точечно.
fn verb_default(kind: ssr_core::verbs::VerbType) -> ActionOption {
    ActionOption {
        label: String::new(),
        action: ActionKind::Examine { entity: 0 },
        kind,
        category: None,
        icon: None,
        priority: 0,
        disabled: false,
        message: None,
        close_menu: None,
        client_exclusive: false,
        confirmation_popup: false,
    }
}

/// Скорости призрака, тайлов/с: `Resources/Prototypes/Entities/Mobs/Player/observer.yml`
/// (`Incorporeal`: `baseWalkSpeed: 8`, `baseSprintSpeed: 12`).
const OBSERVER_WALK_SPEED: f32 = 8.0;
const OBSERVER_SPRINT_SPEED: f32 = 12.0;

/// Тайл занят стеной? Снаряд в сборке сталкивается со слоями
/// `Impassable | BulletImpassable` (`ProjectileSystem.OnStartCollide` требует
/// hard-фикстуру) — у нас стены это тайлы, проверяем по чанкам карты.
fn wall_at(map: &GameMap, x: f32, y: f32) -> bool {
    let tx = (x / TILE_SIZE).floor() as i32;
    let ty = (y / TILE_SIZE).floor() as i32;
    let chunk_tiles = ssr_core::tiles::CHUNK_TILES as i32;
    let coords = (tx.div_euclid(chunk_tiles), ty.div_euclid(chunk_tiles));
    let Some(chunk) = map.chunks.iter().find(|chunk| chunk.coords == coords) else {
        return true; // за картой — считаем препятствием
    };
    let lx = tx.rem_euclid(chunk_tiles) as u32;
    let ly = ty.rem_euclid(chunk_tiles) as u32;
    ssr_core::occluders::is_solid(chunk.get_local(lx, ly))
}

/// Навешивает оружию состояние `Gun` и магазин, а патронам — `Cartridge`, как
/// только предмет появился из прототипа (`Added<Item>`). Одна точка — покрывает
/// меню спавна, админ-команду и стартовое снаряжение.
fn equip_spawned_guns(
    mut commands: Commands,
    catalogs: Res<ProtoCatalog>,
    items: Query<(Entity, &Item), Added<Item>>,
) {
    for (entity, item) in items.iter() {
        if catalogs.gun(&item.name).is_some() {
            weapons::attach_gun_with_slots(&mut commands, &catalogs, entity, &item.name);
        } else if let Some(cartridge) = catalogs.cartridge(&item.name) {
            commands.entity(entity).insert(weapons::Cartridge {
                proto: cartridge.proto.clone(),
                spent: cartridge.spent,
            });
        }
    }
}

/// Стрельба (W-план) — `AttemptShoot` + `GunSystem.Shoot` сборки в объёме
/// P0: кулдаун, патронник + магазин (`AutoCycle`), разброс `CurrentAngle`,
/// спавн снаряда, звук выстрела.
#[allow(clippy::too_many_arguments)]
fn fire_weapons(
    mut commands: Commands,
    time: Res<Time>,
    mut queue: ResMut<weapons::ShootQueue>,
    mut sounds: ResMut<weapons::SoundQueue>,
    catalogs: Res<ProtoCatalog>,
    hands: Query<&Hands>,
    mut guns: Query<&mut weapons::Gun>,
    mut providers: Query<&mut weapons::AmmoProvider>,
    players: Query<&PlayerPosition>,
) {
    tracing::debug!(queued = queue.0.len(), "fire_weapons tick");
    if queue.0.is_empty() {
        return;
    }
    let now = time.elapsed_secs_f64();
    for (player, dir) in std::mem::take(&mut queue.0) {
        let Ok(player_position) = players.get(player) else {
            tracing::warn!(?player, "fire: нет позиции игрока");
            continue;
        };
        // Оружие — предмет в АКТИВНОЙ руке (как `HandsSystem` в сборке).
        let Some(gun_entity) = hands
            .get(player)
            .ok()
            .and_then(|hands| hands.active_item())
            .and_then(Entity::try_from_bits)
        else {
            tracing::warn!(?player, "fire: рука пуста");
            continue;
        };
        let Ok(mut gun) = guns.get_mut(gun_entity) else {
            tracing::warn!(?gun_entity, "fire: в руке не оружие");
            continue;
        };
        if gun.next_fire > now {
            tracing::info!(?gun_entity, next_fire = gun.next_fire, now, "fire: кулдаун");
            continue;
        }
        tracing::info!(
            ?gun_entity,
            chamber = ?gun.chamber,
            magazine = ?gun.magazine,
            fire_rate = gun.fire_rate,
            "fire: состояние оружия"
        );
        // Патронник, затем досыл из магазина (`ChamberMagazineAmmoProvider`).
        let round = gun.chamber.take();
        if let Some(magazine) = gun.magazine.and_then(Entity::try_from_bits)
            && let Ok(mut provider) = providers.get_mut(magazine)
        {
            gun.chamber = provider.take_round();
        }
        let Some(round) = round else {
            // Пусто: в сборке popup «No ammo left!» + `SoundEmpty`, кулдаун 0.5 с.
            sounds.0.push((
                player_position.0,
                "/Audio/Weapons/Guns/Empty/empty.ogg".to_string(),
            ));
            gun.next_fire = now + 0.5;
            continue;
        };
        let Some(cartridge) = catalogs.cartridge(&round).cloned() else {
            continue;
        };
        let Some(projectile) = catalogs.projectile(&cartridge.proto).cloned() else {
            continue;
        };
        // Разброс: формула `GunSystem.GetRecoilAngle` (`ssr_core::weapons`).
        let spread = ssr_core::weapons::update_spread(
            gun.spread,
            (now - gun.last_fire) as f32,
            gun.angle_increase,
            gun.angle_decay,
            gun.min_angle,
            gun.max_angle,
        );
        let seed = ((now * 1000.0) as u64) ^ player.to_bits();
        let random = ssr_core::weapons::spread_random(seed);
        let length = (dir[0] * dir[0] + dir[1] * dir[1]).sqrt().max(1e-4);
        let aim = (dir[1] / length).atan2(dir[0] / length);
        let angle = ssr_core::weapons::shot_angle(aim, spread, random);
        gun.spread = spread;
        gun.last_fire = gun.next_fire;
        gun.next_fire = now + 1.0 / gun.fire_rate.max(0.01) as f64;
        // Снаряд: скорость `GunComponent.ProjectileSpeed` (40 тайлов/с).
        let speed = ssr_core::weapons::PROJECTILE_SPEED * TILE_SIZE;
        let (sx, sy) = (player_position.0[0], player_position.0[1]);
        let dx = sx + angle.cos() * TILE_SIZE * weapons::MUZZLE_OFFSET_TILES;
        let dy = sy + angle.sin() * TILE_SIZE * weapons::MUZZLE_OFFSET_TILES;
        commands.spawn((
            weapons::Projectile {
                proto: cartridge.proto.clone(),
                damage: projectile.damage.clone(),
                velocity: [angle.cos() * speed, angle.sin() * speed],
                lifetime: if projectile.lifetime > 0.0 {
                    projectile.lifetime
                } else {
                    10.0
                },
                shooter: player.to_bits(),
                impact_effect: projectile.impact_effect.clone(),
                sound_hit: projectile.sound_hit.clone(),
            },
            ItemPosition([dx, dy]),
            Replicate::to_clients(NetworkTarget::All),
            Rooms::default(),
        ));
        if let Some(sound) = &gun.sound {
            sounds.0.push(([dx, dy], sound.clone()));
        }
        tracing::info!(player = ?player, round = %round, projectile = %cartridge.proto, spread, angle, "shot fired");
    }
}

/// Полёт снарядов: стены (тайлы), игроки, время жизни (`TimedDespawn.lifetime`).
/// Урон идёт тем же путём, что и ближний бой (`DamageEvent`).
fn move_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    mut projectiles: Query<(Entity, &mut weapons::Projectile, &mut ItemPosition)>,
    map: Res<GameMap>,
    players: Query<(Entity, &PlayerPosition, &Health)>,
    mut damage_events: MessageWriter<DamageEvent>,
    mut sounds: ResMut<weapons::SoundQueue>,
) {
    let dt = time.delta_secs();
    for (entity, mut projectile, mut position) in projectiles.iter_mut() {
        position.0[0] += projectile.velocity[0] * dt;
        position.0[1] += projectile.velocity[1] * dt;
        projectile.lifetime -= dt;
        let (x, y) = (position.0[0], position.0[1]);
        let mut hit = false;
        for (player, player_position, _) in players.iter() {
            if player.to_bits() == projectile.shooter {
                continue; // `IgnoreShooter = true`
            }
            let dx = player_position.0[0] - x;
            let dy = player_position.0[1] - y;
            if dx * dx + dy * dy < (TILE_SIZE * 0.5) * (TILE_SIZE * 0.5) {
                let amount: f32 = projectile.damage.iter().map(|(_, value)| value).sum();
                damage_events.write(DamageEvent {
                    target: player,
                    amount: amount.round() as i32,
                    source: DamageSource::Projectile {
                        shooter: Entity::try_from_bits(projectile.shooter)
                            .unwrap_or(Entity::PLACEHOLDER),
                        weapon: Some(projectile.proto.clone()),
                    },
                });
                hit = true;
                break;
            }
        }
        if !hit && wall_at(&map, x, y) {
            hit = true;
        }
        if hit || projectile.lifetime <= 0.0 {
            if hit {
                if let Some(sound) = &projectile.sound_hit {
                    sounds.0.push(([x, y], sound.clone()));
                }
                tracing::info!(proto = %projectile.proto, x, y, "projectile hit");
            }
            commands.entity(entity).despawn();
        }
    }
}

/// Полёт брошенного предмета (Ctrl+Q, `ThrownItemComponent` в сборке):
/// летит по прямой, гасится о стену или игрока, затем просто лежит.
/// Урон от броска в сборке считается по массе и скорости (`ThrowRegion`) —
/// добавим при переносе масс предметов (PORT_PLAN 2.x).
fn move_thrown(
    mut commands: Commands,
    time: Res<Time>,
    mut thrown: Query<(Entity, &mut Thrown, &mut ItemPosition)>,
    map: Res<GameMap>,
    players: Query<&PlayerPosition>,
) {
    let dt = time.delta_secs();
    for (entity, mut item, mut position) in thrown.iter_mut() {
        position.0[0] += item.velocity[0] * dt;
        position.0[1] += item.velocity[1] * dt;
        item.lifetime -= dt;
        let (x, y) = (position.0[0], position.0[1]);
        let stopped = item.lifetime <= 0.0
            || wall_at(&map, x, y)
            || players.iter().any(|player_position| {
                let dx = player_position.0[0] - x;
                let dy = player_position.0[1] - y;
                dx * dx + dy * dy < (TILE_SIZE * 0.5) * (TILE_SIZE * 0.5)
            });
        if stopped {
            commands.entity(entity).remove::<Thrown>();
            tracing::info!(item = ?entity, x, y, "thrown item landed");
        }
    }
}

/// Звуки в мире: короткоживущие сущности, которые видит клиент (выстрел,
/// попадание, пустой магазин).
fn flush_world_sounds(
    mut commands: Commands,
    mut queue: ResMut<weapons::SoundQueue>,
    time: Res<Time>,
    mut sounds: Query<(Entity, &mut weapons::WorldSound)>,
) {
    for (position, path) in std::mem::take(&mut queue.0) {
        commands.spawn((
            weapons::WorldSound {
                path,
                position,
                lifetime: 0.25,
            },
            Replicate::to_clients(NetworkTarget::All),
            Rooms::default(),
        ));
    }
    for (entity, mut sound) in sounds.iter_mut() {
        sound.lifetime -= time.delta_secs();
        if sound.lifetime <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

/// Деспавн дуг удара по времени жизни (сущность реплицируется — клиенты
/// уберут спрайт вместе с ней).
fn flush_world_effects(
    mut commands: Commands,
    time: Res<Time>,
    mut effects: Query<(Entity, &mut weapons::WorldEffect)>,
) {
    for (entity, mut effect) in effects.iter_mut() {
        effect.lifetime -= time.delta_secs();
        if effect.lifetime <= 0.0 {
            commands.entity(entity).despawn();
        }
    }
}

/// Перезарядка (R): в сборке `ItemSlots` оружия принимают магазин/патрон из
/// активной руки, а `EjectMagazine` отдаёт вставленный магазин в руки
/// (`SharedGunSystem.Magazine.cs`). У нас: магазин в руке → вставить,
/// иначе вставленный магазин → вынуть в руку.
#[allow(clippy::too_many_arguments)]
fn reload_weapons(
    mut commands: Commands,
    mut queue: ResMut<weapons::ReloadQueue>,
    catalogs: Res<ProtoCatalog>,
    hands: Query<&Hands>,
    mut guns: Query<(&mut weapons::Gun, &ItemPosition)>,
    mut providers: Query<&mut weapons::AmmoProvider>,
    mut sounds: ResMut<weapons::SoundQueue>,
    positions: Query<&PlayerPosition>,
) {
    if queue.0.is_empty() {
        return;
    }
    for player in std::mem::take(&mut queue.0) {
        let Ok(hand) = hands.get(player) else {
            continue;
        };
        let Some(gun_entity) = hand.active_item().and_then(Entity::try_from_bits) else {
            continue;
        };
        let Ok((mut gun, _)) = guns.get_mut(gun_entity) else {
            continue;
        };
        let position = positions
            .get(player)
            .map(|position| position.0)
            .unwrap_or([0.0, 0.0]);
        // 1) Вынуть магазин (если он есть) — он падает игроку под ноги.
        if let Some(magazine) = gun.magazine.and_then(Entity::try_from_bits) {
            if let Ok(mut provider) = providers.get_mut(magazine) {
                // Магазин сохраняет патроны — просто отпускаем предмет.
                let _ = &mut provider;
            }
            commands
                .entity(magazine)
                .insert((HeldBy { player: 0 }, ItemPosition(position)));
            gun.magazine = None;
            sounds.0.push((
                position,
                "/Audio/Weapons/Guns/MagOut/pistol_magout.ogg".into(),
            ));
            tracing::info!(player = ?player, "magazine ejected");
            continue;
        }
        // 2) Вставить магазин из руки в оружие нельзя (рука занята оружием) —
        // поэтому ищем магазин среди предметов, лежащих рядом (упрощение P0).
        let Some(magazine_proto) = catalogs.slot_starting_item(&gun.proto, "gun_magazine") else {
            continue;
        };
        if let Some(entity) =
            weapons::spawn_magazine(&mut commands, &catalogs, &magazine_proto, player)
        {
            gun.magazine = Some(entity.to_bits());
            if let Ok(mut provider) = providers.get_mut(entity) {
                // В сборке магазин приходит пустым, если у `BallisticAmmoProvider`
                // нет `proto` — патроны пересыпаются вручную (`MayTransfer`).
                let _ = &mut provider;
            }
            sounds.0.push((
                position,
                "/Audio/Weapons/Guns/MagIn/pistol_magin.ogg".into(),
            ));
            tracing::info!(player = ?player, magazine = %magazine_proto, "magazine inserted");
        }
    }
}

/// Тест-режим `SSR_GUN_TEST=1`: выдаёт первому игроку пистолет MK58 с магазином
/// (магазин наполняем патронами — в сборке `MagazinePistol` рождается пустым, но
/// для проверки выстрела нужны патроны) и кладёт оружие в активную руку.
fn gun_test(
    mut commands: Commands,
    catalogs: Res<ProtoCatalog>,
    players: Res<Players>,
    mut hands: Query<&mut Hands>,
    mut done: Local<bool>,
) {
    if *done || std::env::var_os("SSR_GUN_TEST").is_none() {
        return;
    }
    let Some(entry) = players.entries.first() else {
        return;
    };
    let player = entry.player;
    *done = true;
    let item = commands
        .spawn((
            Item {
                name: "WeaponPistolMk58".to_string(),
            },
            HeldBy {
                player: player.to_bits(),
            },
            Replicate::to_clients(NetworkTarget::All),
            Rooms::default(),
        ))
        .id();
    weapons::attach_gun_with_slots(&mut commands, &catalogs, item, "WeaponPistolMk58");
    if let Ok(mut hand) = hands.get_mut(player) {
        let _ = hand.take_in_active(item.to_bits());
    }
    tracing::info!(player = ?player, "gun test: пистолет выдан");
}

/// Очередь системных сообщений чата: наполняется событиями (вход/выход игрока,
/// смерть, призрак), рассылается всем клиентам одной системой.
/// В сборке это `ChatSystem.SendEntitySystemMessage` / системные строки
/// (`Resources/Locale/*/chat/*.ftl`).
#[derive(Resource, Default)]
struct SystemChatQueue(Vec<String>);

/// Рассылает системные сообщения всем подключённым клиентам:
/// `ServerMessage::Event { kind: "system:<текст>" }` → строка в чате.
fn flush_system_chat(
    mut queue: ResMut<SystemChatQueue>,
    mut senders: Query<&mut MessageSender<ServerMessage>, With<Connected>>,
) {
    if queue.0.is_empty() {
        return;
    }
    for text in std::mem::take(&mut queue.0) {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ServerMessage::Event {
                kind: format!("system:{text}"),
            });
        }
        tracing::info!(%text, "system chat message");
    }
}
