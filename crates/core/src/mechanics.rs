//! Компоненты игровых механик (запросы владельца): призрак админа, лежачий
//! после смерти, бег и боевой режим — состояния, которые видят обе стороны.

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Множитель скорости бега (Shift).
pub const RUN_SPEED_MULT: f32 = 1.6;
/// Сколько секунд игрок лежит после смерти.
pub const KNOCKDOWN_SECS: f32 = 3.0;

/// Лежачий игрок: не двигается (после смерти или сильного удара),
/// клиент рисует тело повёрнутым.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct KnockedDown {
    pub seconds: f32,
}

/// Имя игрока (реплицируется для админ-меню и осмотра).
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PlayerName(pub String);

/// Призрак админа: летает сквозь стены, без коллизии и урона.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Ghost;

/// Пол персонажа: влияет только на части тела `head/chest/groin` — как
/// `HumanoidVisualLayersExtension.HasSexMorph` в SS14 (только эти три слоя
/// имеют варианты `_m`/`_f`; у Unsexed остаётся мужской арт).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, bevy::prelude::Component,
)]
pub enum Sex {
    #[default]
    Male,
    Female,
    Unsexed,
}

impl Sex {
    /// Суффикс состояния RSI для слоя с половым вариантом (`_m`/`_f`).
    pub fn part_suffix(self) -> &'static str {
        match self {
            Sex::Female => "_f",
            _ => "_m",
        }
    }

    /// Разбор строки (`SSR_SEX`).
    pub fn from_id(id: &str) -> Option<Self> {
        match id.to_ascii_lowercase().as_str() {
            "male" | "м" | "m" => Some(Self::Male),
            "female" | "ж" | "f" => Some(Self::Female),
            "unsexed" => Some(Self::Unsexed),
            _ => None,
        }
    }
}

/// Причёска: стиль (состояние `Mobs/Customization/human_hair.rsi`) и цвет.
/// В SS14 это маркинг слоя `HumanoidVisualLayers.Hair`; человек скрывает волосы
/// под шлемом (`hideLayersOnEquip: [Hair, Snout]` в `human.yml`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, bevy::prelude::Component)]
pub struct Hair {
    /// Имя состояния RSI (например "80s", "afro", "bedhead").
    pub style: String,
    /// Цвет волос (RGB).
    pub color: [u8; 3],
}

/// Причёски, которые могут выпасть на спавне (проверены по meta.json сборки).
pub const HAIR_STYLES: [&str; 8] = [
    "80s", "afro", "antenna", "b", "baby", "bedhead", "baldfade", "a",
];

/// Растительность на лице (борода/усы): отдельный маркинг слоя `FacialHair`
/// в SS14 (`human_facial_hair.rsi`, 38 стилей).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, bevy::prelude::Component)]
pub struct FacialHair {
    /// Имя состояния RSI (например "3oclock", "brokenman", "chaplin").
    pub style: String,
    /// Цвет (обычно совпадает с цветом волос).
    pub color: [u8; 3],
}

/// Стили бороды, которые могут выпасть на спавне (проверены по meta.json).
pub const FACIAL_HAIR_STYLES: [&str; 6] = [
    "3oclock",
    "5oclockmoustache",
    "brokenman",
    "chaplin",
    "chin",
    "abe",
];
