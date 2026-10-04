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
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Light;

/// Питание (T4.4, реплицируется): подключён ли потребитель к запитанной сети.
/// Двери без питания не открываются, лампы не светят (как `*_unlit` в SS14).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct Powered(pub bool);

/// Потребление двери, кВт.
pub const DOOR_DRAW_KW: f32 = 4.0;
/// Потребление лампы, кВт.
pub const LIGHT_DRAW_KW: f32 = 1.0;

/// Бит направления в маске кабеля (T4.4): индексы соответствуют
/// `lvcable_0..15` из `Structures/Power/Cables/lv_cable.rsi`.
pub const CABLE_NORTH: u8 = 1;
pub const CABLE_EAST: u8 = 2;
pub const CABLE_SOUTH: u8 = 4;
pub const CABLE_WEST: u8 = 8;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cable_mask_bits() {
        // Маска N|E|S|W = 15 — полный крест (стейт lvcable_15).
        assert_eq!(CABLE_NORTH | CABLE_EAST | CABLE_SOUTH | CABLE_WEST, 15);
        assert_eq!(CABLE_EAST, 2);
        assert_eq!(CABLE_SOUTH, 4);
        assert_eq!(CABLE_WEST, 8);
    }
}
