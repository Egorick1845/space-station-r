//! Компоненты игровых механик (запросы владельца): призрак админа, лежачий
//! после смерти, бег и боевой режим — состояния, которые видят обе стороны.

use std::sync::OnceLock;

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

/// Спринт-тоггл (`InputMoverComponent.Sprinting` + `SprinterComponent` в сборке):
/// у человека включается Space, множитель скорости ×1.45, пауза между спринтами
/// 3 с. Реплицируется — клиенту нужен флаг для анимации шага и HUD.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Sprinting(pub bool);

/// Имя игрока (реплицируется для админ-меню и осмотра).
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PlayerName(pub String);

/// Призрак админа: летает сквозь стены, без коллизии и урона.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Ghost;

/// «Спит» (SSDIndicator в сборке, спрайт `Effects/ssd.rsi#default0`): тело
/// без управления — призрак ушёл или клиент отключился. Иконку рисует клиент.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Ssd;

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

/// Полный список стилей причёсок из `human_hair.rsi` (в сборке 203 состояния).
/// В SS14 его отдают прототипы маркингов (`HairStyles.cs` перечисляет лишь
/// дефолты); у нас источник — сам RSI, поэтому добавление стиля в ассеты сразу
/// доступно и клиенту (окно внешности), и серверу (проверка и спавн).
/// Если RSI не прочитался — откат на короткий список [`HAIR_STYLES`].
pub fn hair_style_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| style_names_from_rsi("Mobs/Customization/human_hair.rsi", &HAIR_STYLES))
}

/// Полный список стилей бороды из `human_facial_hair.rsi` (38 состояний).
pub fn facial_hair_style_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        style_names_from_rsi(
            "Mobs/Customization/human_facial_hair.rsi",
            &FACIAL_HAIR_STYLES,
        )
    })
}

/// Имена состояний RSI отсортированными по алфавиту (как список ItemList в
/// SS14, который сортирует маркинги по локализованному имени).
fn style_names_from_rsi(rel: &str, fallback: &[&str]) -> Vec<String> {
    let path = crate::assets_root().join("sprites/ss14").join(rel);
    let from_rsi = crate::rsi::state_names(&path)
        .ok()
        .filter(|names| !names.is_empty());
    match from_rsi {
        Some(mut names) => {
            names.sort();
            names
        }
        None => fallback.iter().map(|name| name.to_string()).collect(),
    }
}

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
