//! Модель урона и состояния моба — перенос из сборки mini-station-goob:
//! - `Resources/Prototypes/Damage/types.yml`, `groups.yml`, `containers.yml`
//!   (14 типов, 6 групп, контейнер `Biological`);
//! - `Content.Shared/Damage/DamageSpecifier.cs` (набор «тип → значение»,
//!   равное деление группы между её типами);
//! - `Content.Shared/Mobs/MobState.cs` + `Mobs/base.yml` (`MobThresholds`:
//!   0 Alive, 100 Critical, 200 Dead; `AllowRevives` по умолчанию false);
//! - `Content.Shared/Mobs/Systems/MobThresholdSystem.cs` (`UpdateAlerts`:
//!   severity HumanHealth = round(lerp(0, 4, урон / порог крита)));
//! - `Species/base.yml` (`SlowOnDamage` 60 → ×0.7, 80 → ×0.5;
//!   `PassiveDamage`: Heat −0.07 и Brute −0.07 в секунду, только Alive,
//!   `damageCap: 20`);
//! - `Content.Server/Body/Systems/RespiratorSystem.cs` (в крите вдоха нет:
//!   насыщение падает на 2 за цикл в 2 с, ниже нуля — Asphyxiation 1.0).

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Тип урона (`damageType`, порядок как в `types.yml`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DamageType {
    Asphyxiation,
    Bloodloss,
    Blunt,
    Cellular,
    Caustic,
    Cold,
    Heat,
    Piercing,
    Poison,
    Radiation,
    Shock,
    Slash,
    Structural,
    Holy,
}

/// Число типов урона.
pub const DAMAGE_TYPE_COUNT: usize = 14;

impl DamageType {
    pub const ALL: [DamageType; DAMAGE_TYPE_COUNT] = [
        DamageType::Asphyxiation,
        DamageType::Bloodloss,
        DamageType::Blunt,
        DamageType::Cellular,
        DamageType::Caustic,
        DamageType::Cold,
        DamageType::Heat,
        DamageType::Piercing,
        DamageType::Poison,
        DamageType::Radiation,
        DamageType::Shock,
        DamageType::Slash,
        DamageType::Structural,
        DamageType::Holy,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    /// Id прототипа (`Blunt`, `Heat`, ...).
    pub fn id(self) -> &'static str {
        match self {
            DamageType::Asphyxiation => "Asphyxiation",
            DamageType::Bloodloss => "Bloodloss",
            DamageType::Blunt => "Blunt",
            DamageType::Cellular => "Cellular",
            DamageType::Caustic => "Caustic",
            DamageType::Cold => "Cold",
            DamageType::Heat => "Heat",
            DamageType::Piercing => "Piercing",
            DamageType::Poison => "Poison",
            DamageType::Radiation => "Radiation",
            DamageType::Shock => "Shock",
            DamageType::Slash => "Slash",
            DamageType::Structural => "Structural",
            DamageType::Holy => "Holy",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.id() == id)
    }

    /// Русское название (`ru-RU/damage/damage-types.ftl`).
    pub fn name_ru(self) -> &'static str {
        match self {
            DamageType::Asphyxiation => "Удушение",
            DamageType::Bloodloss => "Кровопотеря",
            DamageType::Blunt => "Ушибы",
            DamageType::Cellular => "Клеточные",
            DamageType::Caustic => "Кислотные",
            DamageType::Cold => "Обморожение",
            DamageType::Heat => "Термические",
            DamageType::Piercing => "Уколы",
            DamageType::Poison => "Яды",
            DamageType::Radiation => "Радиация",
            DamageType::Shock => "Электрические",
            DamageType::Slash => "Порезы",
            DamageType::Structural => "Структурные",
            DamageType::Holy => "Святые",
        }
    }
}

/// Группа урона (`damageGroup`, `groups.yml`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DamageGroup {
    Brute,
    Burn,
    Airloss,
    Toxin,
    Genetic,
    Metaphysical,
}

impl DamageGroup {
    pub const ALL: [DamageGroup; 6] = [
        DamageGroup::Brute,
        DamageGroup::Burn,
        DamageGroup::Airloss,
        DamageGroup::Toxin,
        DamageGroup::Genetic,
        DamageGroup::Metaphysical,
    ];

    /// Типы группы в порядке `damageTypes` из `groups.yml` (остаток от
    /// деления уходит последним типам).
    pub fn types(self) -> &'static [DamageType] {
        use DamageType::*;
        match self {
            DamageGroup::Brute => &[Blunt, Slash, Piercing],
            DamageGroup::Burn => &[Heat, Shock, Cold, Caustic],
            DamageGroup::Airloss => &[Asphyxiation, Bloodloss],
            DamageGroup::Toxin => &[Poison, Radiation],
            DamageGroup::Genetic => &[Cellular],
            DamageGroup::Metaphysical => &[Holy],
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            DamageGroup::Brute => "Brute",
            DamageGroup::Burn => "Burn",
            DamageGroup::Airloss => "Airloss",
            DamageGroup::Toxin => "Toxin",
            DamageGroup::Genetic => "Genetic",
            DamageGroup::Metaphysical => "Metaphysical",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|g| g.id() == id)
    }

    /// Русское название (`ru-RU/damage/damage-groups.ftl`).
    pub fn name_ru(self) -> &'static str {
        match self {
            DamageGroup::Brute => "Механические",
            DamageGroup::Burn => "Физические",
            DamageGroup::Airloss => "Нехватка воздуха",
            DamageGroup::Toxin => "Токсины",
            DamageGroup::Genetic => "Генетические",
            DamageGroup::Metaphysical => "Метафизические",
        }
    }
}

/// Шаг `FixedPoint2` (две цифры после запятой).
fn fixed2(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}

/// Набор урона (`DamageSpecifier`); отрицательные значения — лечение.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DamageSpecifier(pub [f32; DAMAGE_TYPE_COUNT]);

impl DamageSpecifier {
    /// Один тип.
    pub fn of(damage_type: DamageType, value: f32) -> Self {
        let mut spec = Self::default();
        spec.0[damage_type.index()] = value;
        spec
    }

    /// Группа: делится поровну, остаток — последним типам
    /// (конструктор `DamageSpecifier(DamageGroupPrototype, FixedPoint2)`).
    pub fn group(group: DamageGroup, value: f32) -> Self {
        let mut spec = Self::default();
        let mut remaining_types = group.types().len() as f32;
        let mut remaining = value;
        for damage_type in group.types() {
            let part = fixed2(remaining / remaining_types);
            spec.0[damage_type.index()] += part;
            remaining = fixed2(remaining - part);
            remaining_types -= 1.0;
        }
        spec
    }

    /// Из пар «id типа или группы → значение» (`damage: types/groups` в YAML,
    /// `Projectile.damage` у нас). Неизвестные id пропускаются.
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, f32)>) -> Self {
        let mut spec = Self::default();
        for (id, value) in pairs {
            if let Some(damage_type) = DamageType::from_id(id) {
                spec.0[damage_type.index()] += value;
            } else if let Some(group) = DamageGroup::from_id(id) {
                spec = spec + Self::group(group, value);
            }
        }
        spec
    }

    pub fn get(&self, damage_type: DamageType) -> f32 {
        self.0[damage_type.index()]
    }

    /// Сумма значений (`GetTotal`).
    pub fn total(&self) -> f32 {
        self.0.iter().sum()
    }

    /// Сумма по группе (`TryGetDamageInGroup`).
    pub fn group_total(&self, group: DamageGroup) -> f32 {
        group.types().iter().map(|t| self.get(*t)).sum()
    }

    /// Типы с ненулевым значением.
    pub fn entries(&self) -> impl Iterator<Item = (DamageType, f32)> + '_ {
        DamageType::ALL
            .into_iter()
            .map(|t| (t, self.get(t)))
            .filter(|(_, v)| *v != 0.0)
    }

    pub fn scaled(&self, factor: f32) -> Self {
        let mut spec = *self;
        for value in spec.0.iter_mut() {
            *value *= factor;
        }
        spec
    }
}

impl std::ops::Add for DamageSpecifier {
    type Output = Self;
    fn add(mut self, other: Self) -> Self {
        for (a, b) in self.0.iter_mut().zip(other.0) {
            *a += b;
        }
        self
    }
}

/// Состояние моба (`MobState`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum MobState {
    #[default]
    Alive,
    Critical,
    Dead,
}

/// Порог крита (`MobThresholds`: `100: Critical`).
pub const CRIT_THRESHOLD: f32 = 100.0;
/// Порог смерти (`200: Dead`).
pub const DEAD_THRESHOLD: f32 = 200.0;

/// Урон и состояние моба (`DamageableComponent` с контейнером `Biological`
/// + `MobStateComponent`/`MobThresholdsComponent`). Реплицируется.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Damageable {
    pub damage: DamageSpecifier,
    pub state: MobState,
}

/// Контейнер `Biological` (`containers.yml`): Brute, Burn, Toxin, Airloss,
/// Genetic и Holy; `Structural` к живым не применяется.
pub fn biological_supports(damage_type: DamageType) -> bool {
    damage_type != DamageType::Structural
}

impl Damageable {
    /// Суммарный урон (`TotalDamage`; без конечностей он же «vital»).
    pub fn total(&self) -> f32 {
        self.damage.total()
    }

    pub fn is_alive(&self) -> bool {
        self.state == MobState::Alive
    }

    /// Крит или смерть (`MobStateSystem.IsIncapacitated`).
    pub fn incapacitated(&self) -> bool {
        self.state != MobState::Alive
    }

    /// `DamageableSystem.ChangeDamage`: неподдерживаемые типы отбрасываются,
    /// итог по типу не ниже нуля. Возвращает реально применённую разницу.
    pub fn change(&mut self, delta: &DamageSpecifier) -> DamageSpecifier {
        let mut applied = DamageSpecifier::default();
        for damage_type in DamageType::ALL {
            let value = delta.get(damage_type);
            if value == 0.0 || !biological_supports(damage_type) {
                continue;
            }
            let index = damage_type.index();
            let before = self.damage.0[index];
            let after = fixed2((before + value).max(0.0));
            self.damage.0[index] = after;
            applied.0[index] = after - before;
        }
        applied
    }

    /// Состояние по порогам (`MobThresholdSystem.CheckThresholds`): мёртвого
    /// без `AllowRevives` пороги не поднимают. (старое, новое) при смене.
    pub fn update_state(&mut self) -> Option<(MobState, MobState)> {
        if self.state == MobState::Dead {
            return None;
        }
        let total = self.total();
        let new = if total >= DEAD_THRESHOLD {
            MobState::Dead
        } else if total >= CRIT_THRESHOLD {
            MobState::Critical
        } else {
            MobState::Alive
        };
        if new == self.state {
            return None;
        }
        let old = self.state;
        self.state = new;
        Some((old, new))
    }

    /// Полное лечение и подъём (`rejuvenate`).
    pub fn rejuvenate(&mut self) {
        *self = Self::default();
    }

    /// Алерт категории Health (`MobThresholdSystem.UpdateAlerts`).
    pub fn alert(&self) -> HealthAlert {
        match self.state {
            MobState::Alive => {
                let fraction = (self.total() / CRIT_THRESHOLD).clamp(0.0, 1.0);
                HealthAlert::Health((4.0 * fraction).round() as u8)
            }
            MobState::Critical => HealthAlert::Crit,
            MobState::Dead => HealthAlert::Dead,
        }
    }

    /// Пассивная регенерация за секунду (`PassiveDamage`, `Species/base.yml`).
    /// `SplitEnsureAllDamagedAndOrganic`: групповое лечение делится только
    /// между повреждёнными типами. Пусто, если не Alive или урон ≥ 20.
    pub fn passive_regen(&self) -> DamageSpecifier {
        let mut spec = DamageSpecifier::default();
        if self.state != MobState::Alive || self.total() >= PASSIVE_DAMAGE_CAP {
            return spec;
        }
        spec.0[DamageType::Heat.index()] = PASSIVE_HEAT;
        let damaged: Vec<DamageType> = DamageGroup::Brute
            .types()
            .iter()
            .copied()
            .filter(|t| self.damage.get(*t) > 0.0)
            .collect();
        for damage_type in &damaged {
            spec.0[damage_type.index()] += PASSIVE_BRUTE / damaged.len() as f32;
        }
        spec
    }
}

/// `PassiveDamage.damageCap`.
pub const PASSIVE_DAMAGE_CAP: f32 = 20.0;
/// `PassiveDamage.damage.types.Heat`.
pub const PASSIVE_HEAT: f32 = -0.07;
/// `PassiveDamage.damage.groups.Brute`.
pub const PASSIVE_BRUTE: f32 = -0.07;

/// Иконка категории Health (`alerts.yml`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HealthAlert {
    /// `HumanHealth`, severity 0..4.
    Health(u8),
    /// `HumanCrit`.
    Crit,
    /// `HumanDead`.
    Dead,
}

impl HealthAlert {
    /// Ключ спрайта в реестре RSI клиента.
    pub fn sprite_key(self) -> String {
        match self {
            HealthAlert::Health(level) => {
                format!("sprites/ss14/Interface/Alerts/human_alive.rsi#health{level}")
            }
            HealthAlert::Crit => {
                "sprites/ss14/Interface/Alerts/human_critical.rsi#critical".to_string()
            }
            HealthAlert::Dead => "sprites/ss14/Interface/Alerts/human_dead.rsi#dead".to_string(),
        }
    }
}

/// Замедление от урона (`SlowOnDamage.speedModifierThresholds`).
pub fn slow_on_damage(total: f32) -> f32 {
    if total >= 80.0 {
        0.5
    } else if total >= 60.0 {
        0.7
    } else {
        1.0
    }
}

/// Дыхание (`RespiratorComponent` + `Species/base.yml`): цикл 2 с,
/// насыщение 5 (мин. −2), порог удушья 0, удушье 1.0 и восстановление −1.0.
pub mod respirator {
    pub const UPDATE_INTERVAL: f32 = 2.0;
    pub const MAX_SATURATION: f32 = 5.0;
    pub const MIN_SATURATION: f32 = -2.0;
    pub const SUFFOCATION_THRESHOLD: f32 = 0.0;
    pub const SUFFOCATION_DAMAGE: f32 = 1.0;
    pub const RECOVERY: f32 = -1.0;

    /// Один цикл: `breathing` — может вдохнуть (не в крите, воздух годен).
    /// Возвращает новое насыщение и Asphyxiation (+ удушье, − восстановление).
    pub fn cycle(saturation: f32, breathing: bool) -> (f32, f32) {
        if breathing {
            return (MAX_SATURATION, RECOVERY);
        }
        let saturation = (saturation - UPDATE_INTERVAL).max(MIN_SATURATION);
        if saturation > SUFFOCATION_THRESHOLD {
            (saturation, 0.0)
        } else {
            (saturation, SUFFOCATION_DAMAGE)
        }
    }
}

/// Лечащий предмет (`HealingComponent` + `healing.yml`).
#[derive(Clone, Debug, PartialEq)]
pub struct Healing {
    pub damage: DamageSpecifier,
    /// `bloodlossModifier` (кровотечения пока нет — только запись).
    pub bloodloss_modifier: f32,
    /// `Delay` = 2 с (Goob).
    pub delay: f32,
    /// `SelfHealPenaltyMultiplier` = 2 (Goob).
    pub self_heal_penalty: f32,
    /// Размер стопки (`Stack.count` у варианта Full).
    pub stack: u32,
    pub begin_sound: &'static str,
    pub end_sound: &'static str,
}

const BRUTE_BEGIN: &str = "/Audio/Items/Medical/brutepack_begin.ogg";
const BRUTE_END: &str = "/Audio/Items/Medical/brutepack_end.ogg";
const OINTMENT_BEGIN: &str = "/Audio/Items/Medical/ointment_begin.ogg";
const OINTMENT_END: &str = "/Audio/Items/Medical/ointment_end.ogg";

/// `HealingComponent.Delay`.
pub const HEALING_DELAY: f32 = 2.0;
/// `HealingComponent.SelfHealPenaltyMultiplier`.
pub const SELF_HEAL_PENALTY: f32 = 2.0;

/// Лечение по id прототипа (`Objects/Specific/Medical/healing.yml`).
pub fn healing_for(id: &str) -> Option<Healing> {
    use DamageType::*;
    let (damage, bloodloss_modifier, begin_sound, end_sound) = match id {
        "Ointment" => (
            DamageSpecifier::from_pairs([
                ("Heat", -10.0),
                ("Cold", -10.0),
                ("Shock", -10.0),
                ("Caustic", -5.0),
            ]),
            0.0,
            OINTMENT_BEGIN,
            OINTMENT_END,
        ),
        "RegenerativeMesh" => (
            DamageSpecifier::from_pairs([
                ("Heat", -15.0),
                ("Cold", -15.0),
                ("Shock", -15.0),
                ("Caustic", -15.0),
            ]),
            0.0,
            OINTMENT_BEGIN,
            OINTMENT_END,
        ),
        "Brutepack" => (
            DamageSpecifier::group(DamageGroup::Brute, -30.0),
            0.0,
            BRUTE_BEGIN,
            BRUTE_END,
        ),
        "MedicatedSuture" => (
            DamageSpecifier::group(DamageGroup::Brute, -45.0),
            -10.0,
            BRUTE_BEGIN,
            BRUTE_END,
        ),
        "Gauze" => (
            DamageSpecifier::of(Slash, -10.0) + DamageSpecifier::of(Piercing, -15.0),
            -10.0,
            BRUTE_BEGIN,
            BRUTE_END,
        ),
        "Bloodpack" => (
            DamageSpecifier::of(Bloodloss, -2.5),
            0.0,
            BRUTE_BEGIN,
            BRUTE_END,
        ),
        _ => return None,
    };
    Some(Healing {
        damage,
        bloodloss_modifier,
        delay: HEALING_DELAY,
        self_heal_penalty: SELF_HEAL_PENALTY,
        stack: 15,
        begin_sound,
        end_sound,
    })
}

impl Healing {
    /// `HealingSystem.HasDamage`: хотя бы один тип предмета повреждён у цели.
    pub fn has_damage(&self, target: &Damageable) -> bool {
        self.damage
            .entries()
            .any(|(damage_type, _)| target.damage.get(damage_type) > 0.0)
    }

    /// Задержка (`TryHeal`): на себя — со штрафом `GetScaledHealingPenalty`.
    pub fn delay_for(&self, self_heal: bool, target: &Damageable) -> f32 {
        if self_heal {
            self.delay * scaled_healing_penalty(target.total(), self.self_heal_penalty)
        } else {
            self.delay
        }
    }
}

/// `HealingSystem.GetScaledHealingPenalty`: доля урона до крита × (mod − 1)
/// + 1, не меньше 1.
pub fn scaled_healing_penalty(total: f32, modifier: f32) -> f32 {
    let percent = total / CRIT_THRESHOLD;
    (percent * (modifier - 1.0) + 1.0).max(1.0)
}

/// `DoAfterArgs.MovementThreshold` (тайлы): сдвиг исполнителя прерывает.
pub const DO_AFTER_MOVEMENT_THRESHOLD: f32 = 0.3;
/// `DoAfterArgs.DistanceThreshold` (тайлы): цель дальше — прерывание.
pub const DO_AFTER_DISTANCE_THRESHOLD: f32 = 1.5;

/// Полоска DoAfter над исполнителем (реплицируется). `id` меняется на каждом
/// цикле (повтор лечения) — клиент по нему сбрасывает отсчёт.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DoAfterBar {
    pub delay: f32,
    pub id: u32,
}

/// Цвет полоски (`ProgressColorSystem.GetProgressColor`): HSV(5/18·p, 1,
/// 0.75), по завершении — чистый зелёный. RGB 0..1.
pub fn progress_color(progress: f32) -> [f32; 3] {
    if progress >= 1.0 {
        return [0.0, 1.0, 0.0];
    }
    let hue = 5.0 / 18.0 * progress.clamp(0.0, 1.0);
    hsv_to_rgb(hue, 1.0, 0.75)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [f32; 3] {
    let h6 = (h * 6.0).rem_euclid(6.0);
    let c = v * s;
    let x = c * (1.0 - (h6 % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h6 as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [r + m, g + m, b + m]
}

#[cfg(test)]
#[path = "damage_tests.rs"]
mod tests;
