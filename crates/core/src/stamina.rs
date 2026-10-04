//! Выносливость — перенос из сборки (PORT_PLAN.md §2.3):
//! `Content.Shared/Damage/Components/StaminaComponent.cs` +
//! `Content.Shared/Damage/Systems/SharedStaminaSystem.cs` +
//! `Resources/Prototypes/Entities/Mobs/Species/human.yml` (`Sprinter.staminaDrainRate: 8`).
//!
//! Числа движка (все — из указанных файлов, не подбирались):
//! `CritThreshold = 100`, `Decay = 5` в секунду, `Cooldown = 5` с (пауза после
//! траты до начала восстановления), `StunTime = 6` с, буфер после крита `3` с,
//! `AfterCritDecayMultiplier = 5` (после крита реген 25/с), трата при беге
//! `8`/с (у человека), замедления от низкой стамины НЕТ
//! (`StunModifierThresholds = {0: 1}`) — «дебаффа» быть не должно.
//!
//! Ядро чистое: время передаётся аргументом, никаких Bevy-ресурсов.

use std::time::Duration;

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Порог крита (`StaminaComponent.CritThreshold`).
pub const CRIT_THRESHOLD: f32 = 100.0;
/// Урон стамины в секунду при беге (человек, `Sprinter.staminaDrainRate`).
pub const DRAIN_PER_SECOND: f32 = 8.0;
/// Восстановление в секунду (`Decay`).
pub const DECAY_PER_SECOND: f32 = 5.0;
/// Пауза после последней траты до начала восстановления (`Cooldown`), сек.
pub const REGEN_COOLDOWN: f32 = 5.0;
/// Сколько лежишь при крите (`StunTime`), сек.
pub const CRIT_STUN: f32 = 6.0;
/// Буфер после крита до начала восстановления (`StamCritBufferTime`), сек.
pub const CRIT_BUFFER: f32 = 3.0;
/// Множитель восстановления после крита (`AfterCritDecayMultiplier`): 25/с.
pub const AFTER_CRIT_DECAY_MULT: f32 = 5.0;
/// Множитель скорости спринта (`SprinterComponent.SprintSpeedMultiplier`).
pub const SPRINT_SPEED_MULT: f32 = 1.45;
/// Пауза между спринтами (`TimeBetweenSprints`), сек.
pub const TIME_BETWEEN_SPRINTS: f32 = 3.0;
/// Порог «тяжело дышит» (`AnimationThreshold`): выше него проигрывается дыхание.
pub const BREATHING_THRESHOLD: f32 = 50.0;
/// Урон от срыва спринта (`SprintDamageSpecifier`: Blunt 10).
pub const SPRINT_BREAK_DAMAGE: f32 = 10.0;

/// Состояние выносливости. Урон копится в `damage` (0 — полон сил, 100 — крит).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Stamina {
    /// Накопленный урон выносливости (0..[`CRIT_THRESHOLD`]).
    pub damage: f32,
    /// Секунды до момента, с которого можно восстанавливаться (пауза/буфер).
    pub regen_at: f32,
    /// Достигнут ли крит (до восстановления ниже порога).
    pub critical: bool,
}

impl Stamina {
    /// Остаток выносливости (100 — полон, 0 — крит).
    pub fn remaining(&self) -> f32 {
        (CRIT_THRESHOLD - self.damage).max(0.0)
    }

    /// Нужна ли анимация «тяжело дышит» (`StaminaDamage > AnimationThreshold`).
    pub fn breathing(&self) -> bool {
        self.damage > BREATHING_THRESHOLD
    }

    /// Уровень алерты (severity) — точный перенос `RoundToLevels`
    /// (`ContentHelpers.cs` сборки): actual = остаток выносливости, max =
    /// порог, levels = 7. При `actual >= max` — уровень 6, при `<= 0` — 0,
    /// иначе `ceil(actual / max × (levels − 2))`. Плюс в движке показывается
    /// и severity 0 (иконка stamina0 мерцает) — а не «алерта нет».
    pub fn alert_level(&self) -> u8 {
        let actual = self.remaining() as f64;
        let max = CRIT_THRESHOLD as f64;
        if actual >= max {
            return 6;
        }
        if actual <= 0.0 {
            return 0;
        }
        (actual / max * 5.0).ceil().clamp(0.0, 5.0) as u8
    }

    /// Наносит урон выносливости (удар, срыв спринта) и отодвигает реген.
    /// Возвращает `true`, если **достигнут порог крита** — решение о крите
    /// принимает вызывающий ([`Self::enter_crit`]), как `EnterStamCrit` в
    /// сборке (крит — отдельное событие, а не свойство накопления).
    pub fn add_damage(&mut self, amount: f32, now: f32) -> bool {
        self.damage = (self.damage + amount).min(CRIT_THRESHOLD);
        self.regen_at = now + REGEN_COOLDOWN;
        self.damage >= CRIT_THRESHOLD && !self.critical
    }

    /// Один шаг по времени: трата при беге либо восстановление по правилам
    /// движка. `moving` — игрок реально двигается (в сборке дренаж идёт, пока
    /// спринт активен; стоя на месте спринт не тратит).
    pub fn tick(&mut self, dt: f32, now: f32, sprinting: bool, moving: bool) -> bool {
        if sprinting && moving {
            return self.add_damage(DRAIN_PER_SECOND * dt, now);
        }
        if now < self.regen_at {
            return false; // ещё пауза после последней траты
        }
        // После крита восстанавливаемся быстрее, пока не вышли из крита.
        let decay = if self.critical {
            DECAY_PER_SECOND * AFTER_CRIT_DECAY_MULT
        } else {
            DECAY_PER_SECOND
        };
        self.damage = (self.damage - decay * dt).max(0.0);
        if self.critical && self.damage <= 0.0 {
            self.critical = false;
        }
        false
    }

    /// Полный станкрит: падение на [`CRIT_STUN`] и буфер до восстановления.
    /// Возвращает длительность стана — вызывающий ставит `KnockedDown`.
    pub fn enter_crit(&mut self, now: f32) -> Duration {
        self.damage = CRIT_THRESHOLD;
        self.critical = true;
        self.regen_at = now + CRIT_STUN + CRIT_BUFFER;
        Duration::from_secs_f32(CRIT_STUN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drain_and_regen_follow_engine_numbers() {
        let mut stamina = Stamina::default();
        // Бежим 4 секунды: 4 × 8 = 32 урона.
        for step in 0..40 {
            stamina.tick(0.1, step as f32 * 0.1, true, true);
        }
        assert!(
            (stamina.damage - 32.0).abs() < 0.5,
            "трата 8/с: {}",
            stamina.damage
        );
        // Стоим: первые 5 секунд — пауза, потом восстановление 5/с.
        let now = 4.0;
        stamina.tick(1.0, now + 1.0, false, false);
        assert!(
            (stamina.damage - 32.0).abs() < 0.5,
            "в паузе не восстанавливается"
        );
        for step in 0..40 {
            stamina.tick(0.1, now + 6.0 + step as f32 * 0.1, false, false);
        }
        assert!(stamina.damage < 32.0, "после паузы восстанавливается");
        assert!(stamina.damage > 0.0, "за 4 секунды не успел восстановиться");
    }

    #[test]
    fn crit_falls_for_six_seconds_and_recovers_fast() {
        let mut stamina = Stamina::default();
        let now = 0.0;
        // Догоняем до крита и входим в него.
        while stamina.damage < CRIT_THRESHOLD {
            stamina.tick(0.5, now, true, true);
        }
        let stun = stamina.enter_crit(now);
        assert_eq!(stun.as_secs_f32(), CRIT_STUN, "стан 6 с");
        assert!(stamina.critical, "крит выставлен");
        // Реген после крита — 25/с, но только после стана и буфера (9 с).
        stamina.tick(1.0, now + 8.0, false, false);
        assert!(
            stamina.damage >= CRIT_THRESHOLD,
            "до буфера не восстанавливается"
        );
        stamina.tick(1.0, now + 10.0 + REGEN_COOLDOWN, false, false);
        assert!(
            stamina.damage < CRIT_THRESHOLD * 0.8,
            "после буфера реген быстрый"
        );
    }

    #[test]
    fn sprint_break_damage_is_ten() {
        let mut stamina = Stamina::default();
        stamina.add_damage(SPRINT_BREAK_DAMAGE, 0.0);
        assert!(
            (stamina.damage - 10.0).abs() < 1e-3,
            "срыв спринта = Blunt 10"
        );
    }

    #[test]
    fn alert_levels_and_breathing() {
        let mut stamina = Stamina {
            damage: 0.0,
            ..Default::default()
        };
        // Полная выносливость: severity 6 (иконка stamina6 — самая спокойная),
        // но ИНДИКАТОР ПОКАЗЫВАЕТСЯ (в сборке алерт стамины виден всегда).
        assert_eq!(stamina.alert_level(), 6);
        assert!(!stamina.breathing());
        stamina.damage = BREATHING_THRESHOLD + 1.0;
        assert!(stamina.breathing(), "дыхание включается после порога 50");
        // Примеры `RoundToLevels`: 89.99 из 100 → ceil(0.8999×5)=5, 95/100 → 5.
        stamina.damage = 10.01;
        assert_eq!(stamina.alert_level(), 5);
        stamina.damage = 40.0; // остаток 60 → ceil(0.6×5)=3
        assert_eq!(stamina.alert_level(), 3);
        stamina.damage = CRIT_THRESHOLD - 0.01; // остаток 0.01 → ceil(0.0005)=1
        assert_eq!(stamina.alert_level(), 1, "почти крит — почти пустая иконка");
        stamina.damage = CRIT_THRESHOLD;
        assert_eq!(
            stamina.alert_level(),
            0,
            "на крите остаток 0 → уровень 0 (RoundToLevels: actual <= 0 → 0); \
             иконка stamina0 — самая тревожная, stamina6 — спокойная"
        );
    }
}
