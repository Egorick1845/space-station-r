//! Базовое взаимодействие с предметами — перенос из сборки mini-station-goob:
//! - `Content.Client/Hands/Systems/HandsSystem.cs` (`UIHandClick`) и
//!   `Content.Shared/Hands/EntitySystems/SharedHandsSystem.Interactions.cs`
//!   (`UseItemInHand` на Z, `TryMoveHeldEntityToActiveHand`,
//!   `TryInteractHandWithActiveHand`);
//! - `Content.Shared/Interaction/SharedInteractionSystem.cs` (`UserInteraction`:
//!   пустая рука берёт предмет, занятая — `InteractUsing`; `AltInteract` —
//!   первый альтернативный верб);
//! - `Content.Shared/Interaction/SmartEquipSystem.cs` (Shift+E пояс, Shift+B
//!   рюкзак) и `ru-RU/interaction/smart-equip-system.ftl`;
//! - `Content.Shared/Stacks/SharedStackSystem*.cs` (слияние по `InteractUsing`,
//!   `UserSplit`, «Стопка теперь заполнена.»).

use bevy::prelude::*;
use lightyear::prelude::*;
use ssr_core::PlayerPosition;
use ssr_core::clothing::{Clothing, ClothingSlot};
use ssr_core::inventory::{Hands, HeldBy, Inventory, Item, ItemPosition, ItemStorage};
use ssr_core::stack::{self, Stack};

use crate::medical::{HealRequests, Senders, popup};
use crate::{ContentCatalog, INTERACT_RANGE, Players, ProtoCatalog, PulledBy, Pulling, TILE_SIZE};

/// Запрос взаимодействия от игрока.
#[derive(Debug, Clone)]
pub enum Request {
    /// Z / клик по активной руке с предметом (`UseInHandEvent`).
    UseInHand,
    /// Клик по слоту руки (`UIHandClick`).
    HandClick(u8),
    /// Предмет активной руки применён к предмету (`InteractUsing`).
    InteractUsing(Entity),
    /// Shift+E / Shift+B (`SmartEquip`).
    SmartEquip(ClothingSlot),
    /// Alt+клик (`AltInteract`).
    AltInteract(Entity),
    /// Разделить стопку (`UserSplit`).
    Split(Entity, u32),
    /// Передать предмет активной руки игроку.
    Give(Entity),
    /// H — отпустить (`ReleasePulledObject`).
    ReleasePull,
}

/// Очередь запросов: наполняют обработчик сообщений и `process_actions`.
#[derive(Resource, Default)]
pub struct InteractionQueue(pub Vec<(Entity, Request)>);

/// Данные мира для обработки запросов (иначе у системы больше 16 параметров).
#[derive(bevy::ecs::system::SystemParam)]
pub struct World<'w, 's> {
    hands: Query<'w, 's, &'static mut Hands>,
    inventories: Query<'w, 's, &'static mut Inventory>,
    clothings: Query<'w, 's, &'static mut Clothing>,
    items: Query<'w, 's, &'static Item>,
    stacks: Query<'w, 's, &'static mut Stack>,
    storages: Query<'w, 's, &'static mut ItemStorage>,
    floor: Query<'w, 's, &'static ItemPosition>,
    positions: Query<'w, 's, &'static PlayerPosition>,
    pulling: Query<'w, 's, &'static Pulling>,
    catalog: Res<'w, ContentCatalog>,
    prototypes: Res<'w, ProtoCatalog>,
}

/// Количество в стопке (у не-стопки — 1).
pub fn stack_count(stacks: &Query<&mut Stack>, entity: Entity) -> u32 {
    stacks.get(entity).map(|stack| stack.count).unwrap_or(1)
}

/// Новый стопочный предмет получает полную стопку (`Stack.count` у прототипа
/// без суффикса `1` в сборке — полный, напр. `SheetSteel` = 30), если при
/// спавне не задано количество.
#[allow(clippy::type_complexity)]
pub fn ensure_stacks(
    mut commands: Commands,
    catalog: Res<ContentCatalog>,
    added: Query<(Entity, &Item), (Added<Item>, Without<Stack>)>,
) {
    for (entity, item) in added.iter() {
        if let Some(max) = catalog.items.max_stack(&item.name) {
            commands.entity(entity).insert(Stack { count: max });
        }
    }
}

/// Убрать `count` единиц стопки (или сам предмет): `ReduceCount`, пустая
/// стопка удаляется. Возвращает, удалён ли предмет (тогда снять его с рук).
pub fn reduce_stack(
    commands: &mut Commands,
    stacks: &mut Query<&mut Stack>,
    entity: Entity,
    count: u32,
) -> bool {
    if let Ok(mut stack) = stacks.get_mut(entity) {
        stack.count = stack.count.saturating_sub(count);
        if stack.count > 0 {
            return false;
        }
    }
    commands.entity(entity).despawn();
    true
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Предмет доступен игроку: в руках, в рюкзаке, в надетом хранилище или на
/// полу рядом (`InRangeUnobstructed`, без проверки стен).
fn reachable(world: &World, player: Entity, item: Entity) -> bool {
    let bits = item.to_bits();
    if world.hands.get(player).is_ok_and(|h| h.has(bits)) {
        return true;
    }
    if world.inventories.get(player).is_ok_and(|i| i.contains(bits)) {
        return true;
    }
    if let Ok(clothing) = world.clothings.get(player) {
        for (_, worn) in &clothing.slots {
            if let Some(worn) = Entity::try_from_bits(*worn)
                && world.inventories.get(worn).is_ok_and(|i| i.contains(bits))
            {
                return true;
            }
        }
    }
    let (Ok(at), Ok(me)) = (world.floor.get(item), world.positions.get(player)) else {
        return false;
    };
    distance(at.0, me.0) <= INTERACT_RANGE + TILE_SIZE
}

/// Убрать предмет оттуда, где он лежит у игрока (рука/рюкзак/хранилище).
fn detach(world: &mut World, player: Entity, item: Entity) {
    let bits = item.to_bits();
    if let Ok(mut hands) = world.hands.get_mut(player) {
        hands.take(bits);
    }
    if let Ok(mut inventory) = world.inventories.get_mut(player) {
        inventory.take(bits);
    }
    let worn: Vec<u64> = world
        .clothings
        .get(player)
        .map(|c| c.slots.iter().map(|(_, b)| *b).collect())
        .unwrap_or_default();
    for worn in worn.into_iter().filter_map(Entity::try_from_bits) {
        if let Ok(mut inventory) = world.inventories.get_mut(worn) {
            inventory.take(bits);
        }
    }
}

/// `Hands.PickupOrDrop`: в активную руку, иначе в любую свободную, иначе под
/// ноги.
fn pickup_or_drop(commands: &mut Commands, world: &mut World, player: Entity, item: Entity) {
    let bits = item.to_bits();
    let placed = world.hands.get_mut(player).is_ok_and(|mut hands| {
        if hands.take_in_active(bits) {
            return true;
        }
        match hands.slots.iter().position(Option::is_none) {
            Some(free) => {
                hands.slots[free] = Some(bits);
                true
            }
            None => false,
        }
    });
    if placed {
        commands
            .entity(item)
            .remove::<ItemPosition>()
            .insert(HeldBy {
                player: player.to_bits(),
            });
        return;
    }
    let at = world.positions.get(player).map(|p| p.0).unwrap_or_default();
    commands
        .entity(item)
        .insert((HeldBy { player: 0 }, ItemPosition(at)));
}

#[allow(clippy::too_many_arguments)]
pub fn process_interactions(
    mut commands: Commands,
    mut queue: ResMut<InteractionQueue>,
    mut world: World,
    mut heal: ResMut<HealRequests>,
    players: Res<Players>,
    mut senders: Senders,
) {
    for (player, request) in std::mem::take(&mut queue.0) {
        match request {
            Request::UseInHand => use_in_hand(&mut world, &mut heal, player),
            Request::HandClick(hand) => {
                hand_click(&mut commands, &mut world, &players, &mut senders, &mut heal, player, hand)
            }
            Request::InteractUsing(target) => {
                interact_using(&mut commands, &mut world, &players, &mut senders, player, target)
            }
            Request::SmartEquip(slot) => {
                smart_equip(&mut commands, &mut world, &players, &mut senders, player, slot)
            }
            Request::AltInteract(target) => {
                // Единственный альтернативный верб у нас — разделение стопки;
                // первый по сортировке — «Разделить пополам» (приоритет 1).
                let count = stack_count(&world.stacks, target);
                if count > 1 && world.stacks.get(target).is_ok() {
                    split(&mut commands, &mut world, &players, &mut senders, player, target, count / 2);
                }
            }
            Request::Split(item, amount) => {
                split(&mut commands, &mut world, &players, &mut senders, player, item, amount)
            }
            Request::Give(target) => give(&mut commands, &mut world, player, target),
            Request::ReleasePull => {
                if let Ok(pulling) = world.pulling.get(player) {
                    commands.entity(pulling.target).remove::<PulledBy>();
                    commands.entity(player).remove::<Pulling>();
                    tracing::info!(?player, "pull released (H)");
                }
            }
        }
    }
}

/// `UseInHandEvent` предмета активной руки: лечащий — на себя
/// (`HealingSystem.OnHealingUse`), с хранилищем — открыть/закрыть окно
/// (`StorageComponent` открывается активацией в руке).
fn use_in_hand(world: &mut World, heal: &mut HealRequests, player: Entity) {
    let Some(item) = world
        .hands
        .get(player)
        .ok()
        .and_then(|h| h.active_item())
        .and_then(Entity::try_from_bits)
    else {
        return;
    };
    let name = world.items.get(item).map(|i| i.name.clone()).unwrap_or_default();
    if ssr_core::damage::healing_for(&name).is_some() {
        heal.0.push((player, player));
        tracing::info!(?player, item = %name, "use in hand: heal self");
        return;
    }
    if let Ok(mut storage) = world.storages.get_mut(item) {
        storage.open = !storage.open;
        tracing::info!(?player, item = %name, open = storage.open, "use in hand: storage");
        return;
    }
    tracing::debug!(?player, item = %name, "use in hand: nothing to do");
}

/// `HandsSystem.UIHandClick`.
#[allow(clippy::too_many_arguments)]
fn hand_click(
    commands: &mut Commands,
    world: &mut World,
    players: &Players,
    senders: &mut Senders,
    heal: &mut HealRequests,
    player: Entity,
    hand: u8,
) {
    let Ok(hands) = world.hands.get(player) else {
        return;
    };
    if hand as usize >= hands.slots.len() {
        return;
    }
    let active = hands.active;
    let pressed = hands.item_in_hand(hand);
    let held = hands.active_item();
    match (hand == active, pressed, held) {
        // Активная рука с предметом — использовать (`RequestUseInHandEvent`).
        (true, _, Some(_)) => use_in_hand(world, heal, player),
        // Пустая неактивная рука — сделать активной (`RequestSetHandEvent`).
        (false, None, _) => {
            if let Ok(mut hands) = world.hands.get_mut(player) {
                hands.active = hand;
            }
        }
        // Предмет в обеих — применить активный к нажатому (`InteractUsing`).
        (false, Some(target), Some(_)) => {
            if let Some(target) = Entity::try_from_bits(target) {
                interact_using(commands, world, players, senders, player, target);
            }
        }
        // Активная пуста — переложить предмет в неё
        // (`TryMoveHeldEntityToActiveHand`).
        (false, Some(item), None) => {
            if let Ok(mut hands) = world.hands.get_mut(player) {
                hands.slots[hand as usize] = None;
                hands.slots[active as usize] = Some(item);
            }
        }
        (true, _, None) => {}
    }
}

/// `InteractUsing` предмета активной руки по предмету `target`: слияние
/// стопок (`OnStackInteractUsing`: донор — цель, получатель — предмет в руке)
/// или вложение в хранилище цели (`StorageComponent.OnInteractUsing`).
fn interact_using(
    commands: &mut Commands,
    world: &mut World,
    players: &Players,
    senders: &mut Senders,
    player: Entity,
    target: Entity,
) {
    let Some(used) = world
        .hands
        .get(player)
        .ok()
        .and_then(|h| h.active_item())
        .and_then(Entity::try_from_bits)
    else {
        return;
    };
    if used == target || !reachable(world, player, target) {
        return;
    }
    let (Ok(used_item), Ok(target_item)) = (world.items.get(used), world.items.get(target)) else {
        return;
    };
    let used_name = used_item.name.clone();
    if used_name == target_item.name
        && let Some(max) = world.catalog.items.max_stack(&used_name)
    {
        let donor = stack_count(&world.stacks, target);
        let recipient = stack_count(&world.stacks, used);
        let Some(merge) = stack::merge(donor, recipient, max) else {
            if recipient >= max {
                popup(players, senders, player, "Стопка уже заполнена.");
            }
            return;
        };
        if let Ok(mut stack) = world.stacks.get_mut(used) {
            stack.count = merge.recipient;
        }
        if merge.donor == 0 {
            detach(world, player, target);
            commands.entity(target).despawn();
        } else if let Ok(mut stack) = world.stacks.get_mut(target) {
            stack.count = merge.donor;
        }
        popup(players, senders, player, &format!("+{}", merge.transferred));
        if merge.recipient >= max {
            popup(players, senders, player, "Стопка теперь заполнена.");
        }
        tracing::info!(?player, item = %used_name, transferred = merge.transferred, "stacks merged");
        return;
    }
    // Хранилище цели (сумка/ящик для инструментов в руке или на полу).
    if world.storages.get(target).is_ok() {
        let (w, h) = crate::item_size_of(&world.catalog, &world.prototypes, &world.items, used.to_bits());
        let placed = world
            .inventories
            .get_mut(target)
            .ok()
            .and_then(|mut inventory| inventory.put_first_fit(used.to_bits(), w, h));
        if placed.is_some() {
            if let Ok(mut hands) = world.hands.get_mut(player) {
                hands.take(used.to_bits());
            }
            commands.entity(used).remove::<ItemPosition>().insert(HeldBy {
                player: player.to_bits(),
            });
            tracing::info!(?player, item = %used_name, ?target, "inserted into storage");
        }
    }
}

/// `SmartEquipSystem.HandleSmartEquip` для пояса и рюкзака. Хранилище рюкзака
/// у нас — сетка на самом игроке (окно V), у пояса — на предмете.
fn smart_equip(
    commands: &mut Commands,
    world: &mut World,
    players: &Players,
    senders: &mut Senders,
    player: Entity,
    slot: ClothingSlot,
) {
    let Ok(hands) = world.hands.get(player) else {
        return;
    };
    let hand_item = hands.active_item();
    let slot_name = slot.id();
    let slot_item = world.clothings.get(player).ok().and_then(|c| c.get(slot));
    let Some(slot_item) = slot_item else {
        // Пустой слот: надеть предмет из руки (`TryEquip`), если подходит.
        let Some(item) = hand_item else {
            popup(
                players,
                senders,
                player,
                &format!("В вашем слоту {slot_name} нет ничего, что можно было бы вынуть!"),
            );
            return;
        };
        let name = Entity::try_from_bits(item)
            .and_then(|e| world.items.get(e).ok())
            .map(|i| i.name.clone())
            .unwrap_or_default();
        if world.catalog.items.slot_of(&name) != Some(slot_name) {
            return;
        }
        if let Ok(mut hands) = world.hands.get_mut(player) {
            hands.take(item);
        }
        if let Ok(mut clothing) = world.clothings.get_mut(player) {
            clothing.equip(slot, item);
        }
        if let Some(entity) = Entity::try_from_bits(item) {
            crate::attach_item_storage(
                commands,
                &world.prototypes,
                &world.inventories,
                &name,
                entity,
            );
        }
        tracing::info!(?player, item = %name, slot = slot_name, "smart equip: equipped");
        return;
    };
    let Some(slot_entity) = Entity::try_from_bits(slot_item) else {
        return;
    };
    // Сетка хранилища слота.
    let storage_entity = if slot == ClothingSlot::Back {
        player
    } else {
        slot_entity
    };
    if world.inventories.get(storage_entity).is_err() {
        // Не хранилище: с пустой рукой — снять в руку.
        if hand_item.is_none() {
            if let Ok(mut clothing) = world.clothings.get_mut(player) {
                clothing.unequip(slot);
            }
            if let Ok(mut hands) = world.hands.get_mut(player) {
                hands.take_in_active(slot_item);
            }
            tracing::info!(?player, slot = slot_name, "smart equip: unequipped to hand");
        }
        return;
    }
    match hand_item {
        // Пустая рука: достать последний положенный предмет.
        None => {
            let last = world
                .inventories
                .get(storage_entity)
                .ok()
                .and_then(|inventory| inventory.cells.iter().flatten().copied().last());
            let Some(item) = last else {
                popup(
                    players,
                    senders,
                    player,
                    &format!("В вашем слоту {slot_name} нет ничего, что можно было бы вынуть!"),
                );
                return;
            };
            if let Ok(mut inventory) = world.inventories.get_mut(storage_entity) {
                inventory.take(item);
            }
            if let Some(entity) = Entity::try_from_bits(item) {
                pickup_or_drop(commands, world, player, entity);
            }
            tracing::info!(?player, slot = slot_name, item, "smart equip: taken out");
        }
        // Предмет в руке: положить в хранилище слота.
        Some(item) => {
            let (w, h) = crate::item_size_of(&world.catalog, &world.prototypes, &world.items, item);
            let placed = world
                .inventories
                .get_mut(storage_entity)
                .ok()
                .and_then(|mut inventory| inventory.put_first_fit(item, w, h));
            if placed.is_some() {
                if let Ok(mut hands) = world.hands.get_mut(player) {
                    hands.take(item);
                }
                tracing::info!(?player, slot = slot_name, item, "smart equip: stored");
            }
        }
    }
}

/// `SharedStackSystem.UserSplit`: отделить `amount` в новую стопку и отдать
/// игроку в руку (`PickupOrDrop`).
#[allow(clippy::too_many_arguments)]
fn split(
    commands: &mut Commands,
    world: &mut World,
    players: &Players,
    senders: &mut Senders,
    player: Entity,
    item: Entity,
    amount: u32,
) {
    if !reachable(world, player, item) {
        return;
    }
    let Ok(name) = world.items.get(item).map(|i| i.name.clone()) else {
        return;
    };
    let Ok(mut stack) = world.stacks.get_mut(item) else {
        return;
    };
    if amount == 0 || amount >= stack.count {
        popup(players, senders, player, "Стопка слишком мала для разделения.");
        return;
    }
    stack.count -= amount;
    let new = commands
        .spawn((
            Item { name: name.clone() },
            Stack { count: amount },
            HeldBy {
                player: player.to_bits(),
            },
            Replicate::to_clients(NetworkTarget::All),
            Rooms::default(),
        ))
        .id();
    pickup_or_drop(commands, world, player, new);
    popup(players, senders, player, "Вы разделили стопку.");
    tracing::info!(?player, item = %name, amount, "stack split");
}

/// Передать предмет активной руки игроку: в его свободную руку, иначе в
/// рюкзак (как прежний Ctrl+клик, теперь вербом «Передать»).
fn give(commands: &mut Commands, world: &mut World, player: Entity, target: Entity) {
    if player == target {
        return;
    }
    let (Ok(me), Ok(them)) = (world.positions.get(player), world.positions.get(target)) else {
        return;
    };
    if distance(me.0, them.0) > INTERACT_RANGE + 32.0 {
        return;
    }
    let Some(item) = world.hands.get(player).ok().and_then(|h| h.active_item()) else {
        return;
    };
    let placed_in_hand = world.hands.get_mut(target).is_ok_and(|mut hands| {
        match hands.slots.iter().position(Option::is_none) {
            Some(free) => {
                hands.slots[free] = Some(item);
                true
            }
            None => false,
        }
    });
    let placed = placed_in_hand || {
        let (w, h) = crate::item_size_of(&world.catalog, &world.prototypes, &world.items, item);
        world
            .inventories
            .get_mut(target)
            .ok()
            .and_then(|mut inventory| inventory.put_first_fit(item, w, h))
            .is_some()
    };
    if !placed {
        return;
    }
    if let Ok(mut hands) = world.hands.get_mut(player) {
        hands.take(item);
    }
    if let Some(entity) = Entity::try_from_bits(item) {
        commands.entity(entity).insert(HeldBy {
            player: target.to_bits(),
        });
    }
    tracing::info!(?player, ?target, item, "item given");
}

/// Тест `SSR_ITEMS_TEST=1` (первый игрок): стопка стали 30 в активную руку →
/// разделить 10 (новая стопка во вторую руку) → клик по второй руке (слияние
/// обратно в активную: 30) → Shift+E убрать в пояс → Shift+E достать →
/// Alt-клик (пополам: 15 + 15).
pub fn items_test(
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut stage: Local<u8>,
    mut commands: Commands,
    players: Res<Players>,
    mut hands: Query<&mut Hands>,
    mut queue: ResMut<InteractionQueue>,
) {
    if std::env::var_os("SSR_ITEMS_TEST").is_none() {
        return;
    }
    let Some(player) = players.entries.first().map(|entry| entry.player) else {
        return;
    };
    *elapsed += time.delta_secs();
    let step = |at: f32| *elapsed >= at;
    let held = hands.get(player).ok().and_then(|h| h.active_item());
    match *stage {
        0 if step(3.0) => {
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
            if let Ok(mut hands) = hands.get_mut(player) {
                hands.active = 0;
                hands.slots = vec![Some(sheet.to_bits()), None];
            }
            tracing::info!("items-test: steel stack in hand");
            *stage = 1;
        }
        1 if step(5.0) => {
            if let Some(item) = held.and_then(Entity::try_from_bits) {
                queue.0.push((player, Request::Split(item, 10)));
            }
            *stage = 2;
        }
        2 if step(8.0) => {
            queue.0.push((player, Request::HandClick(1)));
            *stage = 3;
        }
        3 if step(11.0) => {
            queue.0.push((player, Request::SmartEquip(ClothingSlot::Belt)));
            *stage = 4;
        }
        4 if step(13.0) => {
            queue.0.push((player, Request::SmartEquip(ClothingSlot::Belt)));
            *stage = 5;
        }
        5 if step(15.0) => {
            if let Some(item) = held.and_then(Entity::try_from_bits) {
                queue.0.push((player, Request::AltInteract(item)));
            }
            *stage = 6;
        }
        _ => {}
    }
}

/// Лог состояния рук для теста (раз в секунду): предметы и количества.
pub fn log_hands_test(
    time: Res<Time>,
    mut next: Local<f32>,
    players: Res<Players>,
    hands: Query<&Hands>,
    items: Query<&Item>,
    stacks: Query<&Stack>,
) {
    if std::env::var_os("SSR_ITEMS_TEST").is_none() {
        return;
    }
    *next += time.delta_secs();
    if *next < 1.0 {
        return;
    }
    *next = 0.0;
    let Some(player) = players.entries.first().map(|entry| entry.player) else {
        return;
    };
    let Ok(hands) = hands.get(player) else {
        return;
    };
    let describe: Vec<String> = hands
        .slots
        .iter()
        .map(|slot| {
            slot.and_then(Entity::try_from_bits)
                .and_then(|e| {
                    let name = items.get(e).ok()?.name.clone();
                    let count = stacks.get(e).map(|s| s.count).unwrap_or(1);
                    Some(format!("{name}x{count}"))
                })
                .unwrap_or_else(|| "-".to_string())
        })
        .collect();
    tracing::info!(active = hands.active, hands = ?describe, "items-test: hands");
}
