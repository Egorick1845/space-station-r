//! Оружие, патроны и снаряды: числа и формулы из сборки (W-план).
//!
//! Источники: `Content.Shared/Weapons/Ranged/Components/GunComponent.cs`
//! (дефолты), `Content.Server/Weapons/Ranged/Systems/GunSystem.cs:355-376`
//! (формула разброса и угла выстрела), `Content.Shared/Weapons/Ranged/Systems/
//! SharedGunSystem.cs:81` (`ProjectileSpeed`).

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Скорость снаряда по умолчанию, тайлов/с (`SharedGunSystem.ProjectileSpeed = 40`).
pub const PROJECTILE_SPEED: f32 = 40.0;
/// `GunComponent.AngleIncrease` — прирост угла разброса за выстрел, градусы.
pub const ANGLE_INCREASE_DEG: f32 = 0.5;
/// `GunComponent.AngleDecay` — спад угла разброса, градусов в секунду.
pub const ANGLE_DECAY_DEG: f32 = 4.0;
/// `GunComponent.MinAngle` — минимальный угол разброса, градусы.
pub const MIN_ANGLE_DEG: f32 = 1.0;
/// `GunComponent.MaxAngle` — максимальный угол разброса, градусы.
pub const MAX_ANGLE_DEG: f32 = 2.0;
/// `GunComponent.FireRate` по умолчанию — выстрелов в секунду.
pub const FIRE_RATE: f32 = 8.0;
/// `GunComponent.ShotsPerBurst` / `BurstCooldown` / `BurstFireRate`.
pub const SHOTS_PER_BURST: u32 = 3;
pub const BURST_COOLDOWN: f32 = 0.25;
pub const BURST_FIRE_RATE: f32 = 8.0;
/// `MeleeWeaponComponent` — дальность удара и ударов в секунду по умолчанию.
pub const MELEE_RANGE: f32 = 1.5;
pub const MELEE_ATTACK_RATE: f32 = 1.5;

/// Обновление текущего угла разброса — дословно `GunSystem.GetAngle`
/// (`Content.Server/Weapons/Ranged/Systems/GunSystem.cs:358-363`):
/// `clamp(CurrentAngle + AngleIncrease − AngleDecay * времяСПрошлогоВыстрела,
///  min(MinAngle, MaxAngle), max(MinAngle, MaxAngle))`.
pub fn update_spread(
    current: f32,
    seconds_since_last_fire: f32,
    increase: f32,
    decay: f32,
    min_angle: f32,
    max_angle: f32,
) -> f32 {
    let low = min_angle.min(max_angle);
    let high = min_angle.max(max_angle);
    (current + increase - decay * seconds_since_last_fire).clamp(low, high)
}

/// Итоговый угол выстрела: `direction + CurrentAngle * random(−0.5…0.5)`
/// (`GunSystem.cs:369-375`; `random` уже умножен на модификатор отдачи).
pub fn shot_angle(direction: f32, spread: f32, random: f32) -> f32 {
    direction + spread * random
}

/// Псевдослучайное число из диапазона `[−0.5, 0.5)` — детерминированный аналог
/// `IRobustRandom.NextFloat(-0.5f, 0.5f)` (сервер использует своё зерно).
pub fn spread_random(seed: u64) -> f32 {
    // xorshift64*: быстрый и без внешних зависимостей.
    let mut x = seed | 1;
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    let value = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
    ((value >> 11) as f64 / (1u64 << 53) as f64) as f32 - 0.5
}

/// Состояние оружия в руке/на полу. Модель сборки: в оружии есть **патронник**
/// (`gun_chamber`, один патрон) и **магазин** (`gun_magazine`, отдельная
/// сущность-предмет со своим `BallisticAmmoProvider`).
#[derive(Component, Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Gun {
    /// Прототип оружия (`WeaponPistolMk58`) — по нему берутся спрайт и звук.
    pub proto: String,
    /// Выстрелов в секунду (`Gun.FireRate`, у пистолета 5).
    pub fire_rate: f32,
    /// Выбранный режим (`SemiAuto`/`FullAuto`/`Burst`).
    pub selected_mode: String,
    /// Доступные режимы.
    pub modes: Vec<String>,
    /// Звук выстрела (`soundGunshot`).
    pub sound: Option<String>,
    /// Звук пустого магазина (`SoundEmpty`).
    pub sound_empty: Option<String>,
    /// Текущий угол разброса в градусах (`GunComponent.CurrentAngle`).
    pub spread: f32,
    /// Прирост/спад/границы угла (градусы) — из прототипа или дефолты движка.
    pub angle_increase: f32,
    pub angle_decay: f32,
    pub min_angle: f32,
    pub max_angle: f32,
    /// Патрон в патроннике (прототип, например `CartridgePistol`).
    pub chamber: Option<String>,
    /// Магазин — bits сущности-предмета (если вставлен).
    pub magazine: Option<u64>,
    /// Время последнего выстрела (секунды сервера) — для спада разброса.
    pub last_fire: f64,
    /// Время, раньше которого стрелять нельзя (`NextFire`).
    pub next_fire: f64,
}

/// Ёмкость патронов (`BallisticAmmoProvider`): магазин, коробка, лента.
#[derive(Component, Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct AmmoProvider {
    /// Ёмкость (`capacity`, у `MagazinePistol` — 12).
    pub capacity: u32,
    /// Патроны в ёмкости (прототипы; в сборке берутся С КОНЦА — LIFO).
    pub rounds: Vec<String>,
}

impl AmmoProvider {
    pub fn is_empty(&self) -> bool {
        self.rounds.is_empty()
    }

    /// Вынуть верхний патрон (в сборке `BallisticAmmoProvider` берёт `Entities[^1]`).
    pub fn take_round(&mut self) -> Option<String> {
        self.rounds.pop()
    }
}

/// Патрон-предмет (`CartridgeAmmo`): какой снаряд порождает и стрелян ли он.
#[derive(Component, Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Cartridge {
    /// Прототип снаряда (`proto: BulletPistol`).
    pub proto: String,
    /// Стреляная гильза (`spent`).
    pub spent: bool,
}

/// Снаряд в полёте (`ProjectileComponent` + скорость из `GunComponent`).
#[derive(Component, Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Projectile {
    /// Прототип снаряда (`BulletPistol`) — спрайт и урон.
    pub proto: String,
    /// Урон по типам (`[("Piercing", 16)]`).
    pub damage: Vec<(String, f32)>,
    /// Скорость, единиц/с (`ProjectileSpeed = 40` тайлов/с).
    pub velocity: [f32; 2],
    /// Остаток времени жизни, с (`TimedDespawn.lifetime = 10`).
    pub lifetime: f32,
    /// Кто выстрелил (bits) — по нему не бьём своего стрелка (`IgnoreShooter`).
    pub shooter: u64,
    /// Эффект попадания (`impactEffect`).
    pub impact_effect: Option<String>,
    /// Звук попадания (`soundHit`).
    pub sound_hit: Option<String>,
}

/// Звук, который нужно проиграть рядом с точкой мира (выстрел, попадание,
/// пустой магазин): короткоживущая сущность, которую видит клиент.
#[derive(Component, Clone, Debug, Serialize, Deserialize)]
pub struct WorldSound {
    /// Путь к файлу (`/Audio/Weapons/Guns/...` → `sounds/ss14/...`).
    pub path: String,
    /// Позиция в мире.
    pub position: [f32; 2],
    /// Остаток времени жизни, с.
    pub lifetime: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Формула разброса движка: рост за выстрел, спад по времени, зажим в границы.
    #[test]
    fn spread_follows_engine_formula() {
        // Первый выстрел: угол 0 → зажим снизу даёт MinAngle (1°).
        assert!((update_spread(0.0, 0.0, 0.5, 4.0, 1.0, 2.0) - 1.0).abs() < 1e-6);
        // Долгая пауза: 1° + 0.5° − 4°*1 с < 1° → снова MinAngle.
        assert!((update_spread(1.0, 1.0, 0.5, 4.0, 1.0, 2.0) - 1.0).abs() < 1e-6);
        // Быстрая стрельба: 1° + 0.5° − 0 = 1.5°.
        assert!((update_spread(1.0, 0.0, 0.5, 4.0, 1.0, 2.0) - 1.5).abs() < 1e-6);
        // Зажим сверху — 2°.
        assert!((update_spread(2.0, 0.0, 0.5, 4.0, 1.0, 2.0) - 2.0).abs() < 1e-6);
        // Границы не зависят от порядка min/max (goob-правка движка).
        assert!((update_spread(1.0, 0.0, 0.5, 4.0, 2.0, 1.0) - 1.5).abs() < 1e-6);
    }

    /// Угол выстрела: разброс умножается на случайное из [−0.5, 0.5).
    #[test]
    fn shot_angle_adds_half_spread_at_most() {
        let direction = 1.25f32;
        assert!((shot_angle(direction, 2.0, 0.0) - direction).abs() < 1e-6);
        assert!((shot_angle(direction, 2.0, 0.5) - (direction + 1.0)).abs() < 1e-6);
        assert!((shot_angle(direction, 2.0, -0.5) - (direction - 1.0)).abs() < 1e-6);
    }

    /// Дефолты движка (`GunComponent`) — чтобы не «изобретать» числа.
    #[test]
    fn defaults_match_engine() {
        assert_eq!(PROJECTILE_SPEED, 40.0);
        assert_eq!(ANGLE_INCREASE_DEG, 0.5);
        assert_eq!(ANGLE_DECAY_DEG, 4.0);
        assert_eq!(MIN_ANGLE_DEG, 1.0);
        assert_eq!(MAX_ANGLE_DEG, 2.0);
        assert_eq!(FIRE_RATE, 8.0);
        assert_eq!(SHOTS_PER_BURST, 3);
        assert_eq!(BURST_COOLDOWN, 0.25);
    }

    /// `spread_random` укладывается в [−0.5, 0.5) и не липнет к нулю.
    #[test]
    fn spread_random_is_in_range() {
        let mut min = f32::MAX;
        let mut max = f32::MIN;
        for seed in 0..1000u64 {
            let value = spread_random(seed);
            assert!(
                (-0.5..0.5).contains(&value),
                "значение {value} вне диапазона"
            );
            min = min.min(value);
            max = max.max(value);
        }
        assert!(
            min < -0.4 && max > 0.4,
            "диапазон слишком узкий: {min}…{max}"
        );
    }
}
