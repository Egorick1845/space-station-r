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
