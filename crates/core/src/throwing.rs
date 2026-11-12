//! Бросок предметов — перенос из сборки mini-station-goob:
//! - `Content.Shared/_Trauma/Hands/PredictedHandsSystem.cs` (`ThrowHeldItem`:
//!   дальность зажимается в `[0.1, ThrowRange = 8]` тайлов, пауза
//!   `ThrowCooldown = 0.5` с, скорость `BaseThrowspeed = 15` тайлов/с (Goob),
//!   `compensateFriction = true` — предмет останавливается у курсора);
//! - `Content.Shared/Throwing/ThrowingSystem.cs` (`TryThrow`: время полёта
//!   `длина / скорость × FlyTimePercentage 0.8`, трение тайла
//!   `physics.tile_friction 8 × TileFrictionMod 1.5 = 12`, скорость
//!   `длина / (полёт + 1 / трение)`);
//! - `Content.Shared/Throwing/ThrownItemSystem.cs` (в полёте предмет бьётся
//!   о твёрдое и о мобов, кроме бросившего; после `LandTime` — на полу и
//!   скользит с трением тайла);
//! - `Content.Shared/CCVar/CCVars.Physics.cs` (`air_friction 0.2`,
//!   `stop_speed 0.1`).
//!
//! Всё в тайлах и секундах; перевод в юниты мира — у вызывающего.

/// `HandsComponent.ThrowRange`, тайлы.
pub const THROW_RANGE: f32 = 8.0;
/// Минимальная дальность (`minDistance` в `ThrowHeldItem`), тайлы.
pub const MIN_THROW_DISTANCE: f32 = 0.1;
/// `HandsComponent.BaseThrowspeed` (Goob), тайлов/с.
pub const BASE_THROW_SPEED: f32 = 15.0;
/// `HandsComponent.ThrowCooldown`, с.
pub const THROW_COOLDOWN: f32 = 0.5;
/// `ThrowingSystem.FlyTimePercentage`.
pub const FLY_TIME_PERCENTAGE: f32 = 0.8;
/// Трение тайла при броске: `physics.tile_friction (8) × TileFrictionMod (1.5)`.
pub const TILE_FRICTION: f32 = 8.0 * 1.5;
/// `physics.air_friction` — затухание скорости в полёте.
pub const AIR_FRICTION: f32 = 0.2;
/// `physics.stop_speed`, тайлов/с: медленнее — предмет остановился.
pub const STOP_SPEED: f32 = 0.1;

/// Параметры броска: направление (единичное), скорость (тайлов/с) и время
/// полёта (с). `None` — бросать некуда (нулевой вектор).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Throw {
    pub dir: [f32; 2],
    pub speed: f32,
    pub fly_time: f32,
}

/// `ThrowHeldItem` + `TryThrow(compensateFriction: true)` для вектора от
/// бросающего к курсору (в тайлах).
pub fn plan(offset: [f32; 2]) -> Option<Throw> {
    let length = (offset[0] * offset[0] + offset[1] * offset[1]).sqrt();
    if !length.is_finite() || length == 0.0 {
        return None;
    }
    let dir = [offset[0] / length, offset[1] / length];
    let distance = length.clamp(MIN_THROW_DISTANCE, THROW_RANGE);
    let fly_time = distance / BASE_THROW_SPEED * FLY_TIME_PERCENTAGE;
    let speed = distance / (fly_time + 1.0 / TILE_FRICTION);
    Some(Throw {
        dir,
        speed,
        fly_time,
    })
}

/// Скорость после шага `dt`: в полёте — воздушное затухание, на полу —
/// трение тайла (`TileFrictionController`: `drop = max(stop, v) × f × dt`).
/// Возвращает 0, когда предмет остановился.
pub fn damp(speed: f32, landed: bool, dt: f32) -> f32 {
    let friction = if landed { TILE_FRICTION } else { AIR_FRICTION };
    let control = speed.max(STOP_SPEED);
    let next = (speed - control * friction * dt).max(0.0);
    if landed && next < STOP_SPEED { 0.0 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Сумма пути при моделировании с мелким шагом.
    fn simulate(offset: [f32; 2]) -> f32 {
        let throw = plan(offset).unwrap();
        let dt = 1.0 / 240.0;
        let (mut t, mut speed, mut travelled) = (0.0, throw.speed, 0.0);
        while speed > 0.0 && t < 10.0 {
            let landed = t >= throw.fly_time;
            travelled += speed * dt;
            speed = damp(speed, landed, dt);
            t += dt;
        }
        travelled
    }

    #[test]
    fn item_stops_near_the_cursor() {
        // compensateFriction: полёт + скольжение ≈ расстояние до курсора.
        for distance in [1.0, 3.0, 5.0, 8.0] {
            let travelled = simulate([distance, 0.0]);
            assert!(
                (travelled - distance).abs() < distance * 0.1 + 0.15,
                "{distance} тайлов → {travelled}"
            );
        }
    }

    #[test]
    fn range_is_clamped_to_eight_tiles() {
        let far = plan([30.0, 0.0]).unwrap();
        let max = plan([8.0, 0.0]).unwrap();
        assert_eq!(far.speed, max.speed);
        assert!(simulate([30.0, 0.0]) < 9.0);
        assert_eq!(plan([0.0, 0.0]), None);
        let diag = plan([3.0, 4.0]).unwrap();
        assert!((diag.dir[0] - 0.6).abs() < 1e-6 && (diag.dir[1] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn fly_time_follows_throwing_system() {
        let throw = plan([7.5, 0.0]).unwrap();
        assert!((throw.fly_time - 7.5 / 15.0 * 0.8).abs() < 1e-6);
        assert!((throw.speed - 7.5 / (0.4 + 1.0 / 12.0)).abs() < 1e-4);
    }
}
