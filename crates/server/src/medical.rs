//! Медицина на сервере: лечение предметами с DoAfter, дыхание в крите и
//! пассивная регенерация. Источники в сборке mini-station-goob:
//! - `Content.Shared/Medical/Healing/HealingSystem.cs` (`TryHeal`, `OnDoAfter`,
//!   повтор, расход стопки, штраф самолечения);
//! - `Content.Shared/DoAfter/DoAfterArgs.cs` (`BreakOnMove`,
//!   `MovementThreshold 0.3`, `DistanceThreshold 1.5`, `NeedHand`);
//! - `Content.Server/Body/Systems/RespiratorSystem.cs` (в крите вдоха нет);
//! - `Content.Shared/Damage/Systems/PassiveDamageSystem.cs` (раз в секунду).
//!
//! Тексты — `ru-RU/medical/components/healing-component.ftl`.

use bevy::prelude::*;
use lightyear::prelude::*;
use ssr_core::PlayerPosition;
use ssr_core::damage::{
    DO_AFTER_DISTANCE_THRESHOLD, DO_AFTER_MOVEMENT_THRESHOLD, DamageSpecifier, DamageType,
    Damageable, DoAfterBar, MobState, healing_for, respirator,
};
use ssr_core::inventory::{Hands, Item};
use ssr_core::mechanics::Ghost;
use ssr_protocol::ServerMessage;
use ssr_protocol::net::GameChannel;

use crate::{DamageEvent, DamageSource, Players, TILE_SIZE};

/// Заявки «лечить» из обработки действий: (лекарь, цель).
#[derive(Resource, Default)]
pub struct HealRequests(pub Vec<(Entity, Entity)>);

/// Остаток стопки лечащего предмета (`StackComponent.Count`). Нет
/// компонента — стопка полная (`healing.yml`: Full = 15).
#[derive(Component, Clone, Copy, Debug)]
pub struct Stack(pub u32);

/// Идущее лечение (DoAfter с `HealingDoAfterEvent`) на лекаре.
#[derive(Component, Clone, Copy, Debug)]
pub struct HealingDoAfter {
    pub target: Entity,
    pub item: Entity,
    pub elapsed: f32,
    pub delay: f32,
    pub start: [f32; 2],
}

/// Насыщение дыхания (`RespiratorComponent.Saturation`) и таймер цикла.
#[derive(Component, Clone, Copy, Debug)]
pub struct Breath {
    pub saturation: f32,
    pub timer: f32,
}

impl Default for Breath {
    fn default() -> Self {
        Self {
            saturation: respirator::MAX_SATURATION,
            timer: 0.0,
        }
    }
}

/// Счётчик id полосок DoAfter (клиент сбрасывает отсчёт по смене id).
#[derive(Resource, Default)]
pub struct DoAfterIds(pub u32);

type Senders<'w, 's> = Query<
    'w,
    's,
    (Entity, &'static mut MessageSender<ServerMessage>),
    With<Connected>,
>;

/// Всплывашка игроку (`PopupClient`) — у нас строка системного чата.
fn popup(players: &Players, senders: &mut Senders, player: Entity, text: &str) {
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
            sender.send::<GameChannel>(ServerMessage::Event {
                kind: format!("system:{text}"),
            });
        }
    }
}

fn name_of(names: &Query<&ssr_core::mechanics::PlayerName>, players: &Players, e: Entity) -> String {
    names
        .get(e)
        .ok()
        .map(|name| name.0.clone())
        .or_else(|| {
            players
                .entries
                .iter()
                .find(|entry| entry.player == e)
                .map(|entry| entry.name.clone())
        })
        .unwrap_or_else(|| "Кто-то".to_string())
}

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Лечащий предмет в активной руке: (сущность, id прототипа).
fn healing_in_hand(hands: &Hands, items: &Query<&Item>) -> Option<(Entity, String)> {
    let item = hands.active_item().and_then(Entity::try_from_bits)?;
    let name = items.get(item).ok()?.name.clone();
    healing_for(&name)?;
    Some((item, name))
}

/// `HealingSystem.TryHeal`: проверки и запуск DoAfter.
#[allow(clippy::too_many_arguments)]
pub fn start_healing(
    mut commands: Commands,
    mut requests: ResMut<HealRequests>,
    mut ids: ResMut<DoAfterIds>,
    players: Res<Players>,
    catalog: Res<crate::ContentCatalog>,
    positions: Query<&PlayerPosition>,
    damageables: Query<&Damageable>,
    hands: Query<&Hands>,
    items: Query<&Item>,
    stacks: Query<&Stack>,
    names: Query<&ssr_core::mechanics::PlayerName>,
    mut senders: Senders,
    mut sounds: ResMut<crate::weapons::SoundQueue>,
) {
    for (user, target) in std::mem::take(&mut requests.0) {
        // Лекарь в крите/мёртв — действовать не может (`CheckAct`).
        if damageables.get(user).is_ok_and(|d| d.incapacitated()) {
            continue;
        }
        let Ok(target_damage) = damageables.get(target) else {
            continue;
        };
        let Some((item, proto)) = hands.get(user).ok().and_then(|h| healing_in_hand(h, &items))
        else {
            continue;
        };
        let Some(healing) = healing_for(&proto) else {
            continue;
        };
        let item_name = catalog.items.name_of(&proto).to_lowercase();
        let (Ok(user_pos), Ok(target_pos)) = (positions.get(user), positions.get(target)) else {
            continue;
        };
        // `InRangeUnobstructed` (только для чужой цели).
        if user != target && distance(user_pos.0, target_pos.0) > crate::INTERACT_RANGE + 32.0 {
            continue;
        }
        if stacks.get(item).is_ok_and(|stack| stack.0 < 1) {
            continue;
        }
        if !healing.has_damage(target_damage) {
            popup(
                &players,
                &mut senders,
                user,
                &format!("Нет повреждений, которые можно вылечить при помощи {item_name}"),
            );
            continue;
        }
        let self_heal = user == target;
        if !self_heal {
            let user_name = name_of(&names, &players, user);
            popup(
                &players,
                &mut senders,
                target,
                &format!("{user_name} пытается лечить вас при помощи {item_name}!"),
            );
        }
        let delay = healing.delay_for(self_heal, target_damage);
        sounds.0.push((target_pos.0, healing.begin_sound.to_string()));
        ids.0 = ids.0.wrapping_add(1);
        commands.entity(user).insert((
            HealingDoAfter {
                target,
                item,
                elapsed: 0.0,
                delay,
                start: user_pos.0,
            },
            DoAfterBar { delay, id: ids.0 },
        ));
        tracing::info!(?user, ?target, item = %proto, delay, "healing started");
    }
}

/// Тик DoAfter лечения: прерывания, применение, расход стопки и повтор.
#[allow(clippy::too_many_arguments)]
pub fn tick_healing(
    mut commands: Commands,
    time: Res<Time>,
    mut ids: ResMut<DoAfterIds>,
    players: Res<Players>,
    catalog: Res<crate::ContentCatalog>,
    mut doers: Query<(Entity, &mut HealingDoAfter)>,
    positions: Query<&PlayerPosition>,
    damageables: Query<&Damageable>,
    mut hands: Query<&mut Hands>,
    items: Query<&Item>,
    stacks: Query<&Stack>,
    mut senders: Senders,
    mut sounds: ResMut<crate::weapons::SoundQueue>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    let dt = time.delta_secs();
    for (user, mut doafter) in doers.iter_mut() {
        let cancel = |commands: &mut Commands, reason: &str| {
            commands
                .entity(user)
                .remove::<(HealingDoAfter, DoAfterBar)>();
            tracing::info!(?user, reason, "healing cancelled");
        };
        let (Ok(user_pos), Ok(target_pos)) =
            (positions.get(user), positions.get(doafter.target))
        else {
            cancel(&mut commands, "gone");
            continue;
        };
        // `BreakOnMove` + `MovementThreshold`, `DistanceThreshold`.
        if distance(user_pos.0, doafter.start) > DO_AFTER_MOVEMENT_THRESHOLD * TILE_SIZE {
            cancel(&mut commands, "moved");
            continue;
        }
        if doafter.target != user
            && distance(user_pos.0, target_pos.0) > DO_AFTER_DISTANCE_THRESHOLD * TILE_SIZE
        {
            cancel(&mut commands, "target too far");
            continue;
        }
        if damageables.get(user).is_ok_and(|d| d.incapacitated()) {
            cancel(&mut commands, "incapacitated");
            continue;
        }
        // `NeedHand`: предмет должен остаться в активной руке.
        let in_hand = hands
            .get(user)
            .ok()
            .and_then(|h| h.active_item())
            .is_some_and(|bits| bits == doafter.item.to_bits());
        if !in_hand {
            cancel(&mut commands, "item left hand");
            continue;
        }
        doafter.elapsed += dt;
        if doafter.elapsed < doafter.delay {
            continue;
        }

        // --- `HealingSystem.OnDoAfter` ---
        let Ok(proto) = items.get(doafter.item).map(|item| item.name.clone()) else {
            cancel(&mut commands, "no item");
            continue;
        };
        let (Some(healing), Ok(target_damage)) =
            (healing_for(&proto), damageables.get(doafter.target))
        else {
            cancel(&mut commands, "not healable");
            continue;
        };
        let item_name = catalog.items.name_of(&proto).to_lowercase();
        // Лечение идёт через общий путь урона (пороги, подъём из крита).
        let mut after = *target_damage;
        let healed = after.change(&healing.damage);
        damage_events.write(DamageEvent {
            target: doafter.target,
            damage: healing.damage,
            source: DamageSource::Healing { healer: user },
        });
        // Расход стопки (`ReduceCount(1)`), пустая — удаляется.
        let left = stacks.get(doafter.item).map(|s| s.0).unwrap_or(healing.stack) - 1;
        let empty = left == 0;
        if empty {
            if let Ok(mut hand) = hands.get_mut(user) {
                for slot in hand.slots.iter_mut() {
                    if *slot == Some(doafter.item.to_bits()) {
                        *slot = None;
                    }
                }
            }
            commands.entity(doafter.item).despawn();
        } else {
            commands.entity(doafter.item).insert(Stack(left));
        }
        sounds.0.push((target_pos.0, healing.end_sound.to_string()));
        tracing::info!(
            ?user,
            target = ?doafter.target,
            item = %proto,
            healed = -healed.total(),
            stack_left = left,
            "healing applied"
        );
        // `args.Repeat = HasDamage && !dontRepeat`.
        let repeat = healing.has_damage(&after) && !empty;
        if !repeat {
            commands
                .entity(user)
                .remove::<(HealingDoAfter, DoAfterBar)>();
            popup(
                &players,
                &mut senders,
                user,
                &format!("Вы закончили лечить при помощи {item_name}"),
            );
            continue;
        }
        // Повтор: на себя задержка пересчитывается со штрафом.
        let delay = healing.delay_for(doafter.target == user, &after);
        doafter.elapsed = 0.0;
        doafter.delay = delay;
        ids.0 = ids.0.wrapping_add(1);
        commands.entity(user).insert(DoAfterBar { delay, id: ids.0 });
    }
}

/// Дыхание (`RespiratorSystem.Update`, цикл 2 с): в крите вдоха нет —
/// насыщение падает и после порога идёт Asphyxiation (медленная смерть в
/// крите). Живой и дышащий восстанавливает удушье (`DamageRecovery`).
/// Вакуум по-прежнему считает `suffocation` (T4.3).
pub fn breathing(
    mut commands: Commands,
    time: Res<Time>,
    atmospheres: Res<crate::Atmospheres>,
    mut mobs: Query<(Entity, &Damageable, Option<&mut Breath>, &PlayerPosition), Without<Ghost>>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    let dt = time.delta_secs();
    for (entity, damageable, breath, position) in mobs.iter_mut() {
        let Some(mut breath) = breath else {
            commands.entity(entity).insert(Breath::default());
            continue;
        };
        breath.timer += dt;
        if breath.timer < respirator::UPDATE_INTERVAL {
            continue;
        }
        breath.timer = 0.0;
        if damageable.state == MobState::Dead {
            continue;
        }
        let air = atmospheres
            .gas_at_units(position.0[0], position.0[1])
            .is_none_or(|gas| gas.is_breathable());
        let can_breathe = damageable.state == MobState::Alive && air;
        let (saturation, asphyxiation) = respirator::cycle(breath.saturation, can_breathe);
        breath.saturation = saturation;
        if asphyxiation > 0.0 || damageable.damage.get(DamageType::Asphyxiation) > 0.0 {
            damage_events.write(DamageEvent {
                target: entity,
                damage: DamageSpecifier::of(DamageType::Asphyxiation, asphyxiation),
                source: DamageSource::Environment {
                    cause: if asphyxiation > 0.0 { "suffocation" } else { "breath recovery" },
                },
            });
        }
    }
}

/// Пассивная регенерация раз в секунду (`PassiveDamageSystem`). Только
/// живые и ниже `damageCap` — состояние не меняется, пишем напрямую.
pub fn passive_regen(
    time: Res<Time>,
    mut next: Local<f32>,
    mut mobs: Query<&mut Damageable, Without<Ghost>>,
) {
    *next += time.delta_secs();
    if *next < 1.0 {
        return;
    }
    *next = 0.0;
    for mut damageable in mobs.iter_mut() {
        let regen = damageable.passive_regen();
        if regen.entries().next().is_some() && damageable.total() > 0.0 {
            damageable.change(&regen);
        }
    }
}

/// Тест `SSR_MEDICAL_TEST=1`: на 3-й секунде первому игроку Blunt 45 и
/// набор для ушибов в активную руку, затем самолечение (повторы DoAfter);
/// на 25-й — Blunt 110 (крит: лежит, роняет вещи, задыхается).
#[allow(clippy::too_many_arguments)]
pub fn medical_test(
    mut commands: Commands,
    time: Res<Time>,
    mut elapsed: Local<f32>,
    mut stage: Local<u8>,
    players: Res<Players>,
    mut hands: Query<&mut Hands>,
    mut requests: ResMut<HealRequests>,
    mut damage_events: MessageWriter<DamageEvent>,
) {
    if std::env::var_os("SSR_MEDICAL_TEST").is_none() {
        return;
    }
    let Some(entry) = players.entries.first() else {
        return;
    };
    *elapsed += time.delta_secs();
    let player = entry.player;
    match *stage {
        0 if *elapsed >= 3.0 => {
            damage_events.write(DamageEvent {
                target: player,
                damage: DamageSpecifier::of(DamageType::Blunt, 45.0),
                source: DamageSource::Environment {
                    cause: "medical test",
                },
            });
            let item = commands
                .spawn((
                    Item {
                        name: "Brutepack".to_string(),
                    },
                    ssr_core::inventory::HeldBy {
                        player: player.to_bits(),
                    },
                    Replicate::to_clients(NetworkTarget::All),
                    Rooms::default(),
                ))
                .id();
            if let Ok(mut hand) = hands.get_mut(player) {
                let active = hand.active as usize;
                if let Some(slot) = hand.slots.get_mut(active) {
                    *slot = Some(item.to_bits());
                }
            }
            tracing::info!("medical-test: blunt 45, brutepack in hand");
            *stage = 1;
        }
        1 if *elapsed >= 4.0 => {
            requests.0.push((player, player));
            tracing::info!("medical-test: self-heal requested");
            *stage = 2;
        }
        2 if *elapsed >= 25.0 => {
            damage_events.write(DamageEvent {
                target: player,
                damage: DamageSpecifier::of(DamageType::Blunt, 110.0),
                source: DamageSource::Environment {
                    cause: "medical test",
                },
            });
            tracing::info!("medical-test: blunt 110 (crit)");
            *stage = 3;
        }
        _ => {}
    }
}
