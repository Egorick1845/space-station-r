//! Кровоток — перенос `BloodstreamComponent` + `SharedBloodstreamSystem`
//! (`Content.Shared/Body/{Components,Systems}`), числа из `Mobs/base.yml`
//! (`bloodlossDamage: Bloodloss 0.5`, `bloodlossHealDamage: Bloodloss -1`)
//! и `Damage/modifier_sets.yml` (`BloodlossHuman`).
//!
//! Кровь — один реагент `Blood` (у человека 300 ед.), поэтому раствор
//! сведён к объёму. Раны Shitmed (кровотечение по конечностям) не
//! перенесены: кровотечение копится сразу в `BleedAmount`, как у тел
//! `BodyType.Simple` и в апстриме SS14.

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

use crate::damage::{DamageSpecifier, DamageType};

/// `BloodReferenceSolution`: `Blood 300`.
pub const REFERENCE_VOLUME: f32 = 300.0;
/// `UpdateInterval` = 3 с.
pub const UPDATE_INTERVAL: f32 = 3.0;
/// `BleedReductionAmount` = 0.33 за такт.
pub const BLEED_REDUCTION: f32 = 0.33;
/// `MaxBleedAmount` = 10.
pub const MAX_BLEED: f32 = 10.0;
/// `BloodlossThreshold` = 0.9.
pub const BLOODLOSS_THRESHOLD: f32 = 0.9;
/// `BloodRefreshAmount` = 1.
pub const REFRESH_AMOUNT: f32 = 1.0;
/// `MaxVolumeModifier` = 2.
pub const MAX_VOLUME_MODIFIER: f32 = 2.0;
/// `bloodlossDamage` (`Mobs/base.yml`): `Bloodloss 0.5`.
pub const BLOODLOSS_DAMAGE: f32 = 0.5;
/// `bloodlossHealDamage` (`Mobs/base.yml`): `Bloodloss -1`.
pub const BLOODLOSS_HEAL: f32 = -1.0;
/// `GoobCVars.BleedMultiplier` (по умолчанию 4) — множитель восстановления.
pub const BLOODLOSS_HEAL_MULTIPLIER: f32 = 4.0;
/// `BleedPuddleThreshold` = 1: вытекло больше — на полу лужа.
pub const BLEED_PUDDLE_THRESHOLD: f32 = 1.0;

/// `BloodlossHuman` (`modifier_sets.yml`): сколько кровотечения даёт
/// единица урона. Жар отрицательный — прижигает рану.
pub fn bloodloss_coefficient(damage_type: DamageType) -> f32 {
    match damage_type {
        DamageType::Blunt => 0.08,
        DamageType::Heat => -0.5,
        DamageType::Piercing => 0.2,
        DamageType::Slash => 0.25,
        _ => 0.0,
    }
}

/// Кровоток моба. Реплицируется: уровень крови и кровотечение видят
/// анализатор здоровья и алерт «Кровотечение».
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Bloodstream {
    /// Объём крови (`Blood` в растворе `bloodstream`).
    pub volume: f32,
    /// `BleedAmount`: сколько крови теряется за такт.
    pub bleed: f32,
    /// Вытекшая, но ещё не пролитая кровь (`bloodstreamTemporary`).
    #[serde(skip)]
    pub temporary: f32,
}

impl Default for Bloodstream {
    fn default() -> Self {
        Self {
            volume: REFERENCE_VOLUME,
            bleed: 0.0,
            temporary: 0.0,
        }
    }
}

/// Итог такта кровотока.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BloodTick {
    /// Урон или лечение `Bloodloss`.
    pub damage: DamageSpecifier,
    /// Объём лужи, если её пора пролить.
    pub puddle: Option<f32>,
}

impl Bloodstream {
    /// `GetBloodLevel`: доля от нормального объёма (0…2).
    pub fn level(&self) -> f32 {
        (self.volume / REFERENCE_VOLUME).clamp(0.0, MAX_VOLUME_MODIFIER)
    }

    /// `TryRegulateBloodLevel`: сдвинуть объём к `reference × factor`, но не
    /// больше чем на `amount`.
    pub fn regulate(&mut self, amount: f32, factor: f32) {
        if amount == 0.0 {
            return;
        }
        let factor = factor.clamp(0.0, MAX_VOLUME_MODIFIER);
        let error = REFERENCE_VOLUME * factor - self.volume;
        if error > 0.0 {
            self.volume += error.min(amount);
        } else if error < 0.0 {
            self.volume -= (-error).min(amount);
        }
    }

    /// `TryModifyBloodLevel`: прибавить (к норме) или убрать (к нулю) кровь.
    pub fn modify_level(&mut self, amount: f32) {
        if amount < 0.0 {
            self.regulate(-amount, 0.0);
        } else {
            self.regulate(amount, 1.0);
        }
    }

    /// `TryModifyBleedAmount`: кровотечение в пределах `0…MaxBleedAmount`.
    pub fn modify_bleed(&mut self, amount: f32) {
        self.bleed = (self.bleed + amount).clamp(0.0, MAX_BLEED);
    }

    /// `TryBleedOut`: кровь уходит во временный раствор; набралось больше
    /// порога — возвращает объём лужи (плюс десятая часть оставшегося
    /// раствора, как в сборке).
    pub fn bleed_out(&mut self, amount: f32) -> Option<f32> {
        if amount <= 0.0 {
            return None;
        }
        let leaked = amount.min(self.volume);
        self.volume -= leaked;
        self.temporary += leaked;
        if self.temporary <= BLEED_PUDDLE_THRESHOLD {
            return None;
        }
        let extra = (self.temporary / 10.0).min(self.volume);
        self.volume -= extra;
        let puddle = self.temporary + extra;
        self.temporary = 0.0;
        Some(puddle)
    }

    /// `TickBleed`: потеря крови по кровотечению, затем рана подсыхает.
    pub fn tick_bleed(&mut self) -> Option<f32> {
        if self.bleed <= 0.0 {
            return None;
        }
        let puddle = self.bleed_out(self.bleed);
        self.modify_bleed(-BLEED_REDUCTION);
        puddle
    }

    /// Один такт `Update` (раз в 3 с). Живой: восполнение крови, рана,
    /// урон от кровопотери ниже порога или её лечение выше. Мёртвый —
    /// только кровотечение.
    pub fn cycle(&mut self, alive: bool) -> BloodTick {
        if !alive {
            return BloodTick {
                damage: DamageSpecifier::default(),
                puddle: self.tick_bleed(),
            };
        }
        self.regulate(REFRESH_AMOUNT, 1.0);
        let puddle = self.tick_bleed();
        let level = self.level();
        let damage = if level < BLOODLOSS_THRESHOLD {
            DamageSpecifier::of(DamageType::Bloodloss, BLOODLOSS_DAMAGE / (0.1 + level))
        } else {
            DamageSpecifier::of(
                DamageType::Bloodloss,
                BLOODLOSS_HEAL * level * BLOODLOSS_HEAL_MULTIPLIER,
            )
        };
        BloodTick { damage, puddle }
    }

    /// `OnDamageChanged`: полученный урон открывает кровотечение по
    /// `BloodlossHuman`; жар прижигает. С шансом `total / 25` («крит»)
    /// сразу теряется `total / 5` крови. `roll` — случайное число 0…1.
    pub fn on_damage(&mut self, delta: &DamageSpecifier, roll: f32) -> DamageOutcome {
        let mut total = 0.0;
        for (damage_type, value) in delta.entries() {
            if value > 0.0 {
                total += value * bloodloss_coefficient(damage_type);
            }
        }
        if total == 0.0 {
            return DamageOutcome::default();
        }
        let bleeding_before = self.bleed > 0.0;
        self.modify_bleed(total);
        let probability = (total / 25.0).clamp(0.0, 1.0);
        if total > 0.0 && roll < probability {
            return DamageOutcome {
                instant: true,
                puddle: self.bleed_out(total / 5.0),
                cauterized: false,
            };
        }
        DamageOutcome {
            instant: false,
            puddle: None,
            // `BloodHealedSoundThreshold` = −0.1.
            cauterized: total <= -0.1 && bleeding_before,
        }
    }

    /// `OnRejuvenate`: кровотечение остановлено, крови снова норма.
    pub fn rejuvenate(&mut self) {
        *self = Self::default();
    }

    /// Степень алерта `Bleed` (`bleed0…bleed10`): кровотечение с округлением
    /// к нулю. `None` — алерт снят.
    pub fn alert_severity(&self) -> Option<u8> {
        if self.bleed <= 0.0 {
            None
        } else {
            Some(self.bleed.trunc().clamp(0.0, 10.0) as u8)
        }
    }

    /// `OnHealthBeingExamined`: строки осмотра о кровотечении и бледности.
    pub fn examine_lines(&self, name: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if self.bleed > MAX_BLEED * 0.75 {
            lines.push(format!("{name} истекает кровью!"));
        } else if self.bleed > MAX_BLEED * 0.5 {
            lines.push(format!("{name} сильно кровоточит!"));
        } else if self.bleed > MAX_BLEED * 0.25 {
            lines.push(format!("{name} кровоточит."));
        } else if self.bleed > 0.0 {
            lines.push(format!("{name} слегка кровоточит."));
        }
        if self.level() < BLOODLOSS_THRESHOLD {
            lines.push(format!("{name} выглядит бледно."));
        }
        lines
    }
}

/// Итог полученного урона для кровотока.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DamageOutcome {
    /// «Критическое» ранение — кровь брызнула сразу.
    pub instant: bool,
    pub puddle: Option<f32>,
    /// Ожог прижёг кровоточащую рану.
    pub cauterized: bool,
}

/// Лужа крови (`Puddle` из `Entities/Effects/puddle.yml`, реагент `Blood`
/// цвета `#800000`). Растекание (`EdgeSpreader`) и скольжение не
/// перенесены — лужа копит объём на своём тайле.
#[derive(Component, Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Puddle {
    pub volume: f32,
    /// Зерно выбора спрайта и поворота (`HashCodeCombine(NetEntity, size)`).
    pub seed: u32,
}

/// `PuddleComponent.OverflowVolume` = 50.
pub const PUDDLE_OVERFLOW: f32 = 50.0;
/// `SharedPuddleSystem.LowThreshold` / `MediumThreshold`.
pub const PUDDLE_LOW: f32 = 0.3;
pub const PUDDLE_MEDIUM: f32 = 0.6;
/// Цвет реагента `Blood` (`Reagents/biological.yml`).
pub const BLOOD_COLOR: [f32; 3] = [128.0 / 255.0, 0.0, 0.0];

impl Puddle {
    /// Состояние `Fluids/puddle.rsi` (`UpdatePuddleSprite`). В этом RSI
    /// `splat0…splat15` — маски соединения с соседними лужами (как у
    /// `IconSmooth`), а `splata`/`splatb` — брызги. Лужи у нас не растекаются,
    /// поэтому одиночная лужа — `splat0` (состояние прототипа `Puddle`), а
    /// меньше `LowThreshold` — брызги `splatb`.
    pub fn sprite_state(&self) -> &'static str {
        if self.volume / PUDDLE_OVERFLOW < PUDDLE_LOW {
            "splatb"
        } else {
            "splat0"
        }
    }

    /// Поворот на кратный 90° угол (`NextAngle().RoundToCardinalAngle()`).
    pub fn rotation_quarters(&self) -> u32 {
        (self.seed / 7) % 4
    }
}

/// Наполнение аптечек (`Catalog/Fills/Items/firstaidkits.yml`,
/// `EntityTableContainerFill` → `storagebase`).
pub fn container_fill(id: &str) -> &'static [(&'static str, u32)] {
    match id {
        "MedkitFilled" => &[
            ("Brutepack", 1),
            ("Ointment", 1),
            ("Gauze", 1),
            ("PillCanisterTricordrazine", 1),
            ("PillCanisterIron", 1),
        ],
        "MedkitBurnFilled" => &[
            ("Ointment", 2),
            ("PillCanisterDermaline", 1),
            ("SprayBottleSilverSulfadiazine", 1),
        ],
        "MedkitBruteFilled" => &[
            ("Brutepack", 1),
            ("Gauze", 1),
            ("PillCanisterIron", 1),
            ("SprayBottleStypticPowder", 1),
        ],
        "MedkitAdvancedFilled" => &[
            ("MedicatedSuture", 1),
            ("RegenerativeMesh", 1),
            ("Bloodpack", 2),
            ("Tourniquet", 1),
        ],
        "MedkitCombatFilled" => &[
            ("MedicatedSuture", 1),
            ("RegenerativeMesh", 1),
            ("SyringeEphedrine", 1),
            ("SyringeSaline", 1),
            ("BruteAutoInjector", 1),
            ("BurnAutoInjector", 1),
        ],
        _ => &[],
    }
}

/// Анализатор здоровья (`HealthAnalyzerComponent`, `healthanalyzer.yml`).
pub mod analyzer {
    /// `ScanDelay` = 0.8 с.
    pub const SCAN_DELAY: f32 = 0.8;
    /// `MaxScanRange` = 2.5 (тайла).
    pub const MAX_SCAN_RANGE: f32 = 2.5;
    /// `UpdateInterval` = 1 с: окно обновляется, пока цель рядом.
    pub const UPDATE_INTERVAL: f32 = 1.0;

    /// Прототипы анализатора.
    pub fn is_analyzer(id: &str) -> bool {
        matches!(
            id,
            "HandheldHealthAnalyzer"
                | "HandheldHealthAnalyzerUnpowered"
                | "HandheldHealthAnalyzerEmpty"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::{Damageable, healing_for};

    #[test]
    fn slash_opens_bleeding_and_heat_cauterizes() {
        let mut blood = Bloodstream::default();
        // 20 Slash × 0.25 = 5; roll 1.0 — без «крита».
        let outcome = blood.on_damage(&DamageSpecifier::of(DamageType::Slash, 20.0), 1.0);
        assert!(!outcome.instant);
        assert!((blood.bleed - 5.0).abs() < 1e-4);
        // Жар: 4 × −0.5 = −2 — рана прижжена частично.
        let outcome = blood.on_damage(&DamageSpecifier::of(DamageType::Heat, 4.0), 1.0);
        assert!(outcome.cauterized);
        assert!((blood.bleed - 3.0).abs() < 1e-4);
        // Удушье кровотечения не даёт.
        let before = blood.bleed;
        blood.on_damage(&DamageSpecifier::of(DamageType::Asphyxiation, 50.0), 0.0);
        assert_eq!(blood.bleed, before);
    }

    #[test]
    fn bleeding_capped_and_crit_spills() {
        let mut blood = Bloodstream::default();
        // 100 Slash → 25 > MaxBleed 10; шанс крита 25/25 = 1.
        let outcome = blood.on_damage(&DamageSpecifier::of(DamageType::Slash, 100.0), 0.5);
        assert_eq!(blood.bleed, MAX_BLEED);
        assert!(outcome.instant);
        // Мгновенно вытекло 25/5 = 5 → лужа 5 + 0.5 (десятая часть).
        assert_eq!(outcome.puddle, Some(5.5));
        assert!((blood.volume - (300.0 - 5.5)).abs() < 1e-3);
    }

    #[test]
    fn tick_loses_blood_and_wound_dries() {
        let mut blood = Bloodstream {
            bleed: 2.0,
            ..Bloodstream::default()
        };
        let tick = blood.cycle(true);
        // Восполнение +1 упирается в норму, затем −2 крови и ещё 0.2 в лужу.
        assert!((blood.volume - 297.8).abs() < 1e-3);
        assert!((blood.bleed - (2.0 - BLEED_REDUCTION)).abs() < 1e-4);
        // 2 > порога лужи 1 → лужа 2 + 0.1·2 = 2.2.
        assert!((tick.puddle.unwrap() - 2.2).abs() < 1e-4);
        // Уровень ≥ 0.9 → лечение Bloodloss: −1 × level × 4.
        assert!(tick.damage.get(DamageType::Bloodloss) < -3.9);
    }

    #[test]
    fn low_blood_deals_bloodloss_damage() {
        let mut blood = Bloodstream {
            volume: 150.0,
            ..Bloodstream::default()
        };
        let tick = blood.cycle(true);
        // +1 восполнения: 151/300 ≈ 0.503 → 0.5 / 0.603 ≈ 0.83.
        let damage = tick.damage.get(DamageType::Bloodloss);
        assert!((damage - 0.5 / (0.1 + 151.0 / 300.0)).abs() < 1e-3);
        // Мёртвый не восполняет кровь.
        let mut dead = Bloodstream {
            volume: 150.0,
            ..Bloodstream::default()
        };
        assert_eq!(dead.cycle(false).damage.total(), 0.0);
        assert_eq!(dead.volume, 150.0);
    }

    #[test]
    fn gauze_stops_bleeding_and_bloodpack_refills() {
        let gauze = healing_for("Gauze").unwrap();
        let mob = Damageable::default();
        let mut blood = Bloodstream {
            bleed: 6.0,
            ..Bloodstream::default()
        };
        assert!(
            gauze.can_heal(&mob, Some(&blood)),
            "кровотечение лечится без урона"
        );
        assert!(gauze.apply_blood(&mut blood));
        assert_eq!(blood.bleed, 0.0);
        assert!(!gauze.can_heal(&mob, Some(&blood)));

        let pack = healing_for("Bloodpack").unwrap();
        let mut low = Bloodstream {
            volume: 200.0,
            ..Bloodstream::default()
        };
        assert!(pack.can_heal(&mob, Some(&low)));
        pack.apply_blood(&mut low);
        assert_eq!(low.volume, 230.0);
        // Выше нормы не наливает.
        let mut full = Bloodstream {
            volume: 290.0,
            ..Bloodstream::default()
        };
        pack.apply_blood(&mut full);
        assert_eq!(full.volume, REFERENCE_VOLUME);
        assert!(!pack.can_heal(&mob, Some(&full)));
    }

    #[test]
    fn alert_and_examine() {
        let mut blood = Bloodstream::default();
        assert_eq!(blood.alert_severity(), None);
        blood.bleed = 3.7;
        assert_eq!(blood.alert_severity(), Some(3));
        blood.bleed = 0.2;
        assert_eq!(blood.alert_severity(), Some(0));
        assert_eq!(blood.examine_lines("Боб"), vec!["Боб слегка кровоточит."]);
        blood.bleed = 9.0;
        blood.volume = 100.0;
        assert_eq!(
            blood.examine_lines("Боб"),
            vec!["Боб истекает кровью!", "Боб выглядит бледно."]
        );
        blood.rejuvenate();
        assert_eq!(blood.bleed, 0.0);
        assert_eq!(blood.volume, REFERENCE_VOLUME);
    }

    #[test]
    fn puddle_sprite_grows_with_volume() {
        let mut puddle = Puddle {
            volume: 2.0,
            seed: 3,
        };
        assert_eq!(puddle.sprite_state(), "splatb");
        puddle.volume = 20.0;
        assert_eq!(puddle.sprite_state(), "splat0");
        assert!(puddle.rotation_quarters() < 4);
    }

    #[test]
    fn medkit_fills_from_catalog() {
        assert_eq!(container_fill("MedkitFilled").len(), 5);
        assert!(container_fill("MedkitAdvancedFilled").contains(&("Bloodpack", 2)));
        assert!(
            container_fill("Medkit").is_empty(),
            "пустая аптечка без наполнения"
        );
    }
}
