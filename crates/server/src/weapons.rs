//! Оружие, патроны и снаряды (W-план) — перенос механики SS14.
//!
//! Источники:
//! * `Content.Shared/Weapons/Ranged/Components/GunComponent.cs` — дефолты
//!   (`FireRate`, `AngleIncrease/Decay`, `MinAngle/MaxAngle`, `ProjectileSpeed`);
//! * `Content.Server/Weapons/Ranged/Systems/GunSystem.cs:355-376` — формула
//!   разброса и угла выстрела (перенесена в [`ssr_core::weapons`]);
//! * `Content.Shared/Weapons/Ranged/Systems/SharedGunSystem.ChamberMagazine.cs` —
//!   порядок «патронник + магазин» (выстрел, досыл патрона, перезарядка);
//! * `Content.Shared/Projectiles/ProjectileComponent.cs` +
//!   `Resources/Prototypes/Entities/Objects/Weapons/Guns/Projectiles/projectiles.yml`
//!   — параметры снаряда (`damage`, `impactEffect`, `soundHit`,
//!   `TimedDespawn.lifetime = 10`, фикстура `projectile` `-0.1,-0.10,0.1,0.30`).

use bevy::prelude::*;
use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::Rooms;
pub use ssr_core::weapons::{AmmoProvider, Cartridge, Gun, Projectile, WorldSound};

/// Очередь выстрелов: сообщения клиентов разбираются в одном месте, а стрельбу
/// (с проверками и тратой патрона) делает отдельная система.
#[derive(Resource, Default)]
pub struct ShootQueue(pub Vec<(Entity, [f32; 2])>);

/// Очередь перезарядок (R): система сама решает, что вынуть/вставить.
#[derive(Resource, Default)]
pub struct ReloadQueue(pub Vec<Entity>);

/// Просьба проиграть звук в мире (наполняется стрельбой/попаданиями).
#[derive(Resource, Default)]
pub struct SoundQueue(pub Vec<([f32; 2], String)>);

/// Навешивает на сущность-оружие состояние `Gun` и слоты (магазин + патронник).
/// В сборке `ChamberMagazineAmmoProvider` держит патрон в `gun_chamber`, а
/// магазин — отдельная сущность в `gun_magazine` (`ItemSlots`), поэтому магазин
/// спавним как предмет и запоминаем его bits.
pub fn attach_gun_with_slots(
    commands: &mut Commands,
    catalogs: &crate::ProtoCatalog,
    item_entity: Entity,
    proto: &str,
) {
    attach_gun(commands, catalogs, item_entity, proto);
    let Some(magazine_proto) = catalogs.slot_starting_item(proto, "gun_magazine") else {
        return;
    };
    let Some(magazine) = spawn_magazine(commands, catalogs, &magazine_proto, item_entity) else {
        return;
    };
    commands.entity(item_entity).insert(MagazineRef(magazine));
}

/// Ссылка на сущность-магазин, вставленный в оружие (`gun_magazine`).
#[derive(Component, Clone, Copy, Debug, Serialize, Deserialize)]
pub struct MagazineRef(pub Entity);

/// Смещение снаряда вперёд от стрелка (в сборке пуля появляется из дула,
/// `GunSystem` берёт `ShootCoordinates` с офсетом ~0.5 тайла).
pub const MUZZLE_OFFSET_TILES: f32 = 0.5;

/// Навешивает на сущность-оружие состояние `Gun` по прототипу и заполняет
/// патронник (`gun_chamber.startingItem`) — как `ItemSlots` в сборке.
pub fn attach_gun(
    commands: &mut Commands,
    catalogs: &crate::ProtoCatalog,
    item_entity: Entity,
    proto: &str,
) {
    let Some(gun) = catalogs.gun(proto) else {
        return;
    };
    let chamber = catalogs.slot_starting_item(proto, "gun_chamber");
    let spread = 0.0;
    commands.entity(item_entity).insert(Gun {
        proto: proto.to_string(),
        fire_rate: gun.fire_rate,
        selected_mode: gun.selected_mode.clone(),
        modes: gun.modes.clone(),
        sound: gun.sound.clone(),
        sound_empty: Some("/Audio/Weapons/Guns/Empty/empty.ogg".to_string()),
        spread,
        angle_increase: gun
            .angle_increase
            .unwrap_or(ssr_core::weapons::ANGLE_INCREASE_DEG),
        angle_decay: gun
            .angle_decay
            .unwrap_or(ssr_core::weapons::ANGLE_DECAY_DEG),
        min_angle: gun.min_angle.unwrap_or(ssr_core::weapons::MIN_ANGLE_DEG),
        max_angle: gun.max_angle.unwrap_or(ssr_core::weapons::MAX_ANGLE_DEG),
        chamber,
        magazine: None,
        last_fire: 0.0,
        next_fire: 0.0,
    });
}

/// Спавнит магазин по прототипу слота. Важно: в сборке магазин наполняется
/// ТОЛЬКО при заданном `BallisticAmmoProvider.proto` (`SharedGunSystem.Ballistic.cs:223-233`,
/// `OnBallisticMapInit`); у `BaseMagazinePistol`/`BaseMagazineShotgun`/`BaseSpeedLoaderMagnum`
/// `proto` нет — такие магазины рождаются ПУСТЫМИ.
pub fn spawn_magazine(
    commands: &mut Commands,
    catalogs: &crate::ProtoCatalog,
    proto: &str,
    owner: Entity,
) -> Option<Entity> {
    let provider = catalogs.ammo_provider(proto)?;
    let rounds = match &provider.proto {
        Some(round) => vec![round.clone(); provider.capacity as usize],
        None => Vec::new(),
    };
    let item = commands
        .spawn((
            ssr_core::inventory::Item {
                name: proto.to_string(),
            },
            ssr_core::inventory::HeldBy {
                player: owner.to_bits(),
            },
            Replicate::to_clients(NetworkTarget::All),
            Rooms::default(),
            AmmoProvider {
                capacity: provider.capacity,
                rounds,
            },
        ))
        .id();
    Some(item)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Патроны расходуются сверху стека — как `TakeRound` в сборке.
    #[test]
    fn ammo_provider_takes_from_top() {
        let mut provider = AmmoProvider {
            capacity: 3,
            rounds: vec!["a".into(), "b".into(), "c".into()],
        };
        assert_eq!(provider.take_round().as_deref(), Some("c"));
        assert_eq!(provider.take_round().as_deref(), Some("b"));
        assert!(!provider.is_empty());
        assert_eq!(provider.take_round().as_deref(), Some("a"));
        assert!(provider.is_empty());
        assert_eq!(provider.take_round(), None);
    }
}
