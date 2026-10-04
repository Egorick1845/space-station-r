//! Электрика lite (T4.4): генератор → кабели по тайлам → потребители
//! (двери, лампы). Энергобаланс считается по сетям: кабели, соединённые
//! в 4 стороны, образуют сеть; если суммарная выработка меньше потребления,
//! сеть обесточена и потребители не работают.
//!
//! Числа в кВт на тайл — намеренно просто: цель lite-модели — аптайм и
//! понятный энергобаланс, а не симуляция электрических цепей.

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Кабель (T4.4): соединения по 4 сторонам — биты N=1, E=2, S=4, W=8.
/// Индекс стейта `lvcable_N` в RSI равен этой маске.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Cable;

/// Генератор (T4.4): сколько кВт даёт в сеть.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Generator {
    pub power_kw: f32,
}

/// Потребитель (T4.4): дверь или лампа — сколько кВт берёт.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Consumer {
    pub draw_kw: f32,
}

/// Лампа (T4.4): потребитель со спрайтом света (отличает его от дверей).
/// Параметры — как `PointLight` в SS14 (`base_lighting.yml`: коридорный
/// светильник даёт `radius: 10`, `energy: 0.8`, цвет `#FFE4CE` — 5000K).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
pub struct Light {
    /// Радиус света в тайлах (`PointLight.radius`).
    pub radius: f32,
    /// Яркость (`PointLight.energy`).
    pub energy: f32,
    /// Цвет света (`PointLight.color`).
    pub color: [u8; 3],
}

impl Default for Light {
    fn default() -> Self {
        Self {
            radius: 10.0,
            energy: 0.8,
            color: [0xff, 0xe4, 0xce],
        }
    }
}

/// Питание (T4.4, реплицируется): подключён ли потребитель к запитанной сети.
/// Двери без питания не открываются, лампы не светят (как `*_unlit` в SS14).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Powered(pub bool);

/// Потребление двери, кВт.
pub const DOOR_DRAW_KW: f32 = 4.0;
/// Потребление лампы, кВт.
pub const LIGHT_DRAW_KW: f32 = 1.0;

/// Бит направления в маске кабеля (T4.4). Нумерация удобна для логики
/// (N/E/S/W), но у стейтов `lvcable_N` другой порядок битов: N=1, S=2, E=4,
/// W=8 — для отрисовки используем [`cable_state`].
pub const CABLE_NORTH: u8 = 1;
pub const CABLE_EAST: u8 = 2;
pub const CABLE_SOUTH: u8 = 4;
pub const CABLE_WEST: u8 = 8;

/// Индекс стейта `lvcable_N` по маске соединений.
/// Проверено по картинкам: `lvcable_1` — рука N, `lvcable_2` — S,
/// `lvcable_4` — E, `lvcable_8` — W, `lvcable_5` — NE, `lvcable_3` — NS.
pub fn cable_state(mask: u8) -> u8 {
    let north = mask & CABLE_NORTH;
    let south = (mask & CABLE_SOUTH) >> 1;
    let east = (mask & CABLE_EAST) << 1;
    let west = mask & CABLE_WEST;
    north | south | east | west
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cable_state_orders_bits_like_rsi() {
        // Прямая вертикаль: N+S (маска 5) → стейт 3 (lvcable_3, руки NS).
        assert_eq!(cable_state(CABLE_NORTH | CABLE_SOUTH), 3);
        // Прямая горизонталь: E+W (маска 10) → стейт 12 (lvcable_12, EW).
        assert_eq!(cable_state(CABLE_EAST | CABLE_WEST), 12);
        // Угол N+E → стейт 5 (lvcable_5, NE).
        assert_eq!(cable_state(CABLE_NORTH | CABLE_EAST), 5);
        // Одиночные направления.
        assert_eq!(cable_state(CABLE_NORTH), 1);
        assert_eq!(cable_state(CABLE_SOUTH), 2);
        assert_eq!(cable_state(CABLE_EAST), 4);
        assert_eq!(cable_state(CABLE_WEST), 8);
        // Полный крест.
        assert_eq!(
            cable_state(CABLE_NORTH | CABLE_EAST | CABLE_SOUTH | CABLE_WEST),
            15
        );
    }

    #[test]
    fn cable_mask_bits() {
        // Маска N|E|S|W = 15 — полный крест (стейт lvcable_15).
        assert_eq!(CABLE_NORTH | CABLE_EAST | CABLE_SOUTH | CABLE_WEST, 15);
        assert_eq!(CABLE_EAST, 2);
        assert_eq!(CABLE_SOUTH, 4);
        assert_eq!(CABLE_WEST, 8);
    }
}
