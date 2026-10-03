//! Общий игровой код Space Station R: компоненты, события, чистые системы,
//! загрузка прототипов.
//!
//! Правила (PLAN.md §6): крейт не знает о сети (`ssr-protocol`) и о движковых
//! плагинах клиента/сервера; компоненты — только данные, логика — в системах.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const GAME_NAME: &str = "Space Station R";

pub mod rsi;

/// Скорость игрока, пикселей в секунду (сервер применяет ввод по ADR-3).
pub const PLAYER_MOVE_SPEED: f32 = 300.0;

/// Размер чанка мира в юнитах: ADR-5 (32×32 тайла) × IMP-1 (1 тайл = 1 юнит).
pub const CHUNK_SIZE: f32 = 32.0;

/// Координаты чанка точки мира (floor-деление, корректно для отрицательных).
pub fn chunk_coords(x: f32, y: f32) -> (i32, i32) {
    (
        (x / CHUNK_SIZE).floor() as i32,
        (y / CHUNK_SIZE).floor() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_coords_floors_negatives() {
        assert_eq!(chunk_coords(0.0, 0.0), (0, 0));
        assert_eq!(chunk_coords(31.9, -0.1), (0, -1));
        assert_eq!(chunk_coords(64.0, 64.0), (2, 2));
        assert_eq!(chunk_coords(-33.0, 33.0), (-2, 1));
    }
}

/// Сервер-авторитарная позиция игрока (ADR-3). Реплицируется клиентам (T1.3).
///
/// Внутренний тип — массив, чтобы сериализация не зависела от фич bevy;
/// координаты сетки тайлов (`IVec2`, ADR-8) появятся в фазе 2.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerPosition(pub [f32; 2]);
