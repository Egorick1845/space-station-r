//! Кровоток, лужи крови, наполнение аптечек и анализатор здоровья на
//! сервере. Источники в сборке mini-station-goob:
//! - `Content.Shared/Body/Systems/SharedBloodstreamSystem.cs` (`Update`,
//!   `OnDamageChanged`, `TickBleed`, `TryBleedOut`);
//! - `Content.Shared/Fluids/SharedPuddleSystem.cs` (лужа на тайле);
//! - `Content.Server/Medical/HealthAnalyzerSystem.cs` (`OnAfterInteract`,
//!   DoAfter 0.8 с, обновление раз в секунду, `MaxScanRange 2.5`);
//! - `Catalog/Fills/Items/firstaidkits.yml` (`EntityTableContainerFill`).

use bevy::prelude::*;
use lightyear::prelude::*;
use ssr_core::PlayerPosition;
use ssr_core::blood::{Bloodstream, Puddle, analyzer, container_fill};
use ssr_core::damage::{DO_AFTER_MOVEMENT_THRESHOLD, DamageSpecifier, Damageable, DoAfterBar};
use ssr_core::inventory::{Hands, HeldBy, Inventory, Item, ItemPosition};
use ssr_core::mechanics::Ghost;
use ssr_protocol::ServerMessage;
use ssr_protocol::net::GameChannel;

use crate::medical::{DoAfterIds, Senders, popup};
use crate::{DamageEvent, DamageSource, Players, TILE_SIZE};

/// Таймер такта кровотока (`NextUpdate`) — серверный, не реплицируется.
#[derive(Component, Clone, Copy, Debug)]
pub struct BloodTimer(pub f32);

/// Урон на прошлом кадре: по разнице видно полученный урон
/// (`DamageChangedEvent.DamageDelta`).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct LastDamage(pub DamageSpecifier);

/// Кровь, вытекшая за кадр: (позиция, объём) → лужи.
#[derive(Resource, Default)]
pub struct Spills(pub Vec<([f32; 2], f32)>);

/// Заявки на скан анализатором: (пользователь, цель, анализатор).
#[derive(Resource, Default)]
pub struct ScanRequests(pub Vec<(Entity, Entity, Entity)>);

/// Идущий скан (`HealthAnalyzerDoAfterEvent`) на пользователе.
#[derive(Component, Clone, Copy, Debug)]
pub struct ScanDoAfter {
    pub target: Entity,
    pub item: Entity,
    pub elapsed: f32,
    pub start: [f32; 2],
}

/// Анализатор следит за пациентом (`ScannedEntity`, `NextUpdate`).
#[derive(Component, Clone, Copy, Debug)]
pub struct AnalyzerScan {
    pub patient: Entity,
    pub user: Entity,
    pub next: f32,
}

/// Простой детерминированный ГПСЧ для «крита» кровотечения (в сборке
/// `System.Random` с зерном из тика и сущностей).
#[derive(Resource)]
pub struct BloodRng(pub u64);

impl Default for BloodRng {
    fn default() -> Self {
        Self(0x9E37_79B9_7F4A_7C15)
    }
}

impl BloodRng {
    pub fn next(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        ((x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32) / (1u64 << 24) as f32
    }
}

fn send_to(players: &Players, senders: &mut Senders, player: Entity, message: ServerMessage) {
    let Some(link) = players
        .entries
        .iter()
        .find(|entry| entry.player == player)
        .map(|entry| entry.link)
    else {
        return;
    };
    for (entity, mut sender) in senders.iter_mut() {
        if entity == link {
            sender.send::<GameChannel>(message.clone());
        }
    }
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Кровоток у каждого моба с уроном (`Bloodstream` в `Mobs/base.yml`).
#[allow(clippy::type_complexity)]
pub fn ensure_bloodstream(
    mut commands: Commands,
    added: Query<(Entity, &Damageable), (Added<Damageable>, Without<Bloodstream>, Without<Ghost>)>,
) {
    for (entity, damageable) in added.iter() {
        commands.entity(entity).insert((
            Bloodstream::default(),
            BloodTimer(ssr_core::blood::UPDATE_INTERVAL),
            LastDamage(damageable.damage),
        ));
    }
}

/// `OnDamageChanged`: по приросту урона открывается кровотечение.
pub fn bleed_on_damage(
    players: Res<Players>,
    mut senders: Senders,
    mut rng: ResMut<BloodRng>,
    mut spills: ResMut<Spills>,
    mut mobs: Query<
        (
            Entity,
            &Damageable,
            &mut Bloodstream,
            &mut LastDamage,
            &PlayerPosition,
        ),
        Changed<Damageable>,
    >,
) {
    for (entity, damageable, mut blood, mut last, position) in mobs.iter_mut() {
        let mut delta = DamageSpecifier::default();
        for (index, value) in damageable.damage.0.iter().enumerate() {
            delta.0[index] = value - last.0.0[index];
        }
        last.0 = damageable.damage;
        if !delta.0.iter().any(|value| *value > 0.0) {
            continue;
        }
        // Мутация только при реальном изменении: иначе репликация каждый кадр.
        let mut probe = *blood;
        let outcome = probe.on_damage(&delta, rng.next());
        if probe == *blood {
            continue;
        }
        *blood = probe;
        if let Some(volume) = outcome.puddle {
            spills.0.push((position.0, volume));
        }
        if outcome.cauterized {
            popup(&players, &mut senders, entity, "Ваши раны прижглись.");
        }
        tracing::info!(
            ?entity,
            bleed = blood.bleed,
            instant = outcome.instant,
            "bleeding from damage"
        );
    }
}

/// Такт кровотока раз в 3 с: восполнение, рана, урон или лечение
/// `Bloodloss` (через общий путь урона — пороги и подъём из крита).
pub fn tick_bloodstream(
    time: Res<Time>,
    mut spills: ResMut<Spills>,
    mut mobs: Query<(
        Entity,
        &Damageable,
        &mut Bloodstream,
        &mut BloodTimer,
        &PlayerPosition,
    )>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    let dt = time.delta_secs();
    for (entity, damageable, mut blood, mut timer, position) in mobs.iter_mut() {
        timer.0 -= dt;
        if timer.0 > 0.0 {
            continue;
        }
        timer.0 += ssr_core::blood::UPDATE_INTERVAL;
        let mut next = *blood;
        let tick = next.cycle(damageable.state != ssr_core::damage::MobState::Dead);
        if next != *blood {
            *blood = next;
        }
        if let Some(volume) = tick.puddle {
            spills.0.push((position.0, volume));
        }
        let bloodloss = tick.damage.get(ssr_core::damage::DamageType::Bloodloss);
        // Лечение без урона кровопотери пропускаем (Mini: «skip zero heals»).
        let current = damageable
            .damage
            .get(ssr_core::damage::DamageType::Bloodloss);
        if bloodloss > 0.0 || (bloodloss < 0.0 && current > 0.0) {
            damage_events.write(DamageEvent {
                target: entity,
                damage: tick.damage,
                source: DamageSource::Environment { cause: "bloodloss" },
            });
        }
    }
}

/// `TrySpillAt`: кровь на тайл — в существующую лужу или новой лужей.
pub fn spill_puddles(
    mut commands: Commands,
    mut spills: ResMut<Spills>,
    mut puddles: Query<(&mut Puddle, &ItemPosition)>,
    mut seed: Local<u32>,
) {
    // Сначала сводим кровь одного кадра по тайлам: новая лужа появится
    // только после применения команд, второй разлив её бы не нашёл.
    let mut by_tile: Vec<([f32; 2], f32)> = Vec::new();
    for (position, volume) in spills.0.drain(..) {
        let tile = (
            (position[0] / TILE_SIZE).floor(),
            (position[1] / TILE_SIZE).floor(),
        );
        let center = [(tile.0 + 0.5) * TILE_SIZE, (tile.1 + 0.5) * TILE_SIZE];
        match by_tile.iter_mut().find(|(at, _)| *at == center) {
            Some((_, total)) => *total += volume,
            None => by_tile.push((center, volume)),
        }
    }
    for (center, volume) in by_tile {
        if let Some((mut puddle, _)) = puddles
            .iter_mut()
            .find(|(_, at)| distance(at.0, center) < 1.0)
        {
            puddle.volume += volume;
            continue;
        }
        *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        commands.spawn((
            Puddle {
                volume,
                seed: *seed >> 8,
            },
            ItemPosition(center),
            Replicate::to_clients(NetworkTarget::All),
            Rooms::default(),
        ));
        tracing::info!(?center, volume, "blood puddle spawned");
    }
}

/// Хранилище и наполнение предмета при появлении (`StorageComponent` +
/// `EntityTableContainerFill`): аптечки сразу с содержимым. Содержимое
/// лежит в сетке, без позиции и носителя — видно через комнату аптечки.
#[allow(clippy::type_complexity)]
pub fn fill_containers(
    mut commands: Commands,
    world: crate::FillWorld,
    added: Query<(Entity, &Item), (Added<Item>, Without<Inventory>)>,
) {
    for (entity, item) in added.iter() {
        let Some(storage) = world.prototypes.storage_of(&item.name) else {
            continue;
        };
        let Some(&(x0, y0, x1, y1)) = storage.grid.first() else {
            continue;
        };
        let (lx, ly, hx, hy) = storage
            .grid
            .iter()
            .skip(1)
            .fold((x0, y0, x1, y1), |(lx, ly, hx, hy), (a, b, c, d)| {
                (lx.min(*a), ly.min(*b), hx.max(*c), hy.max(*d))
            });
        let cols = (hx - lx + 1).clamp(1, 16) as u8;
        let rows = (hy - ly + 1).clamp(1, 16) as u8;
        let mut inventory = Inventory::with_size(cols, rows);
        let mut filled = 0;
        for (id, amount) in container_fill(&item.name) {
            if !world.prototypes.contains(id) {
                tracing::warn!(container = %item.name, item = %id, "fill: unknown prototype");
                continue;
            }
            for _ in 0..*amount {
                let content = commands
                    .spawn((
                        Item {
                            name: (*id).to_string(),
                        },
                        HeldBy { player: 0 },
                        Replicate::to_clients(NetworkTarget::All),
                        Rooms::default(),
                    ))
                    .id();
                let (w, h) = world
                    .prototypes
                    .size_cells(id)
                    .unwrap_or_else(|| world.catalogs.items.size_of(id));
                if inventory.put_first_fit(content.to_bits(), w, h).is_none() {
                    commands.entity(content).despawn();
                    tracing::warn!(container = %item.name, item = %id, "fill: no room");
                } else {
                    filled += 1;
                }
            }
        }
        commands
            .entity(entity)
            .insert((ssr_core::inventory::ItemStorage { open: false }, inventory));
        if filled > 0 {
            tracing::info!(container = %item.name, filled, "container filled");
        }
    }
}

/// `OnAfterInteract`: DoAfter скана (`NeedHand`, `BreakOnMove`), цели —
/// всплывашка «вас сканируют».
#[allow(clippy::too_many_arguments)]
pub fn start_scan(
    mut commands: Commands,
    mut requests: ResMut<ScanRequests>,
    mut ids: ResMut<DoAfterIds>,
    players: Res<Players>,
    names: Query<&ssr_core::mechanics::PlayerName>,
    positions: Query<&PlayerPosition>,
    damageables: Query<&Damageable>,
    mut senders: Senders,
) {
    for (user, target, item) in std::mem::take(&mut requests.0) {
        if damageables.get(user).is_ok_and(|d| d.incapacitated()) {
            continue;
        }
        // Сканировать можно только мобов (`HasComp<MobStateComponent>`).
        if damageables.get(target).is_err() {
            continue;
        }
        let (Ok(user_pos), Ok(target_pos)) = (positions.get(user), positions.get(target)) else {
            continue;
        };
        if user != target && distance(user_pos.0, target_pos.0) > crate::INTERACT_RANGE + 32.0 {
            continue;
        }
        ids.0 = ids.0.wrapping_add(1);
        commands.entity(user).insert((
            ScanDoAfter {
                target,
                item,
                elapsed: 0.0,
                start: user_pos.0,
            },
            DoAfterBar {
                delay: analyzer::SCAN_DELAY,
                id: ids.0,
            },
        ));
        if user != target {
            let name = names
                .get(user)
                .map(|n| n.0.clone())
                .unwrap_or_else(|_| "Кто-то".to_string());
            popup(
                &players,
                &mut senders,
                target,
                &format!("{name} сканирует вас!"),
            );
        }
        tracing::info!(?user, ?target, "health scan started");
    }
}

/// Тик DoAfter скана: прерывания, затем анализатор привязывается к цели и
/// у пользователя открывается окно.
#[allow(clippy::too_many_arguments)]
pub fn tick_scan(
    mut commands: Commands,
    time: Res<Time>,
    players: Res<Players>,
    mut doers: Query<(Entity, &mut ScanDoAfter)>,
    positions: Query<&PlayerPosition>,
    hands: Query<&Hands>,
    mut senders: Senders,
) {
    let dt = time.delta_secs();
    for (user, mut scan) in doers.iter_mut() {
        let cancel = |commands: &mut Commands| {
            commands.entity(user).remove::<(ScanDoAfter, DoAfterBar)>();
        };
        let Ok(user_pos) = positions.get(user) else {
            cancel(&mut commands);
            continue;
        };
        if distance(user_pos.0, scan.start) > DO_AFTER_MOVEMENT_THRESHOLD * TILE_SIZE {
            cancel(&mut commands);
            tracing::info!(?user, "health scan cancelled: moved");
            continue;
        }
        let in_hand = hands
            .get(user)
            .ok()
            .and_then(|h| h.active_item())
            .is_some_and(|bits| bits == scan.item.to_bits());
        if !in_hand {
            cancel(&mut commands);
            continue;
        }
        scan.elapsed += dt;
        if scan.elapsed < analyzer::SCAN_DELAY {
            continue;
        }
        cancel(&mut commands);
        commands.entity(scan.item).insert(AnalyzerScan {
            patient: scan.target,
            user,
            next: analyzer::UPDATE_INTERVAL,
        });
        send_to(
            &players,
            &mut senders,
            user,
            ServerMessage::HealthScan {
                target: scan.target.to_bits(),
                active: true,
                opened: true,
            },
        );
        tracing::info!(?user, patient = ?scan.target, "health scan done");
    }
}

/// `HealthAnalyzerSystem.Update`: раз в секунду — пациент ещё в радиусе
/// 2.5 тайла? Нет — слежение снимается (`StopAnalyzingEntity`). Анализатор
/// убран из руки — тоже (`OnInsertedIntoContainer` выключает его).
pub fn update_scans(
    mut commands: Commands,
    time: Res<Time>,
    players: Res<Players>,
    mut analyzers: Query<(Entity, &mut AnalyzerScan)>,
    positions: Query<&PlayerPosition>,
    hands: Query<&Hands>,
    mut senders: Senders,
) {
    let dt = time.delta_secs();
    for (item, mut scan) in analyzers.iter_mut() {
        scan.next -= dt;
        if scan.next > 0.0 {
            continue;
        }
        scan.next = analyzer::UPDATE_INTERVAL;
        let held = hands
            .get(scan.user)
            .is_ok_and(|h| h.slots.iter().flatten().any(|bits| *bits == item.to_bits()));
        let in_range = match (positions.get(scan.user), positions.get(scan.patient)) {
            (Ok(a), Ok(b)) => distance(a.0, b.0) <= analyzer::MAX_SCAN_RANGE * TILE_SIZE,
            _ => false,
        };
        let active = held && in_range;
        send_to(
            &players,
            &mut senders,
            scan.user,
            ServerMessage::HealthScan {
                target: scan.patient.to_bits(),
                active,
                opened: false,
            },
        );
        if !active {
            commands.entity(item).remove::<AnalyzerScan>();
            tracing::info!(?item, held, in_range, "health scan stopped");
        }
    }
}

/// Тест `SSR_BLOOD_TEST=1`: на 3-й секунде первому игроку Slash 40
/// (кровотечение 10), в руку — марлевый бинт; на 12-й — самолечение
/// бинтом (кровотечение остановлено), на 18–19-й — анализатор и скан себя,
/// на 23–24-й — продвинутая аптечка в руку и открыть её (Z).
#[allow(clippy::too_many_arguments)]
pub fn blood_test(
    mut commands: Commands,
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut stage: Local<u8>,
    players: Res<Players>,
    mut hands: Query<&mut Hands>,
    mut heal: ResMut<crate::medical::HealRequests>,
    mut interactions: ResMut<crate::interaction::InteractionQueue>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    if std::env::var_os("SSR_BLOOD_TEST").is_none() {
        return;
    }
    let Some(entry) = players.entries.first() else {
        return;
    };
    *elapsed += time.delta_secs();
    let player = entry.player;
    let mut give = |commands: &mut Commands, name: &str| {
        let item = commands
            .spawn((
                Item {
                    name: name.to_string(),
                },
                HeldBy {
                    player: player.to_bits(),
                },
                Replicate::to_clients(NetworkTarget::All),
                Rooms::default(),
            ))
            .id();
        if let Ok(mut hand) = hands.get_mut(player) {
            let active = hand.active as usize;
            if let Some(slot) = hand.slots.get_mut(active) {
                if let Some(old) = slot.take().and_then(Entity::try_from_bits) {
                    commands.entity(old).despawn();
                }
                *slot = Some(item.to_bits());
            }
        }
    };
    match *stage {
        0 if *elapsed >= 3.0 => {
            damage_events.write(DamageEvent {
                target: player,
                damage: DamageSpecifier::of(ssr_core::damage::DamageType::Slash, 40.0),
                source: DamageSource::Environment {
                    cause: "blood test",
                },
            });
            give(&mut commands, "Gauze");
            tracing::info!("blood-test: slash 40, gauze in hand");
            *stage = 1;
        }
        1 if *elapsed >= 12.0 => {
            heal.0.push((player, player));
            tracing::info!("blood-test: gauze self-heal requested");
            *stage = 2;
        }
        2 if *elapsed >= 18.0 => {
            give(&mut commands, "HandheldHealthAnalyzer");
            *stage = 3;
        }
        3 if *elapsed >= 19.0 => {
            heal.0.push((player, player));
            tracing::info!("blood-test: self scan requested");
            *stage = 4;
        }
        4 if *elapsed >= 23.0 => {
            give(&mut commands, "MedkitAdvancedFilled");
            tracing::info!("blood-test: advanced medkit in hand");
            *stage = 5;
        }
        5 if *elapsed >= 24.0 => {
            interactions
                .0
                .push((player, crate::interaction::Request::UseInHand));
            tracing::info!("blood-test: medkit opened");
            *stage = 6;
        }
        _ => {}
    }
}
