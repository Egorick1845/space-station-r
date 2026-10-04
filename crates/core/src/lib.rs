//! Общий игровой код Space Station R: компоненты, события, чистые системы,
//! загрузка прототипов.
//!
//! Правила (PLAN.md §6): крейт не знает о сети (`ssr-protocol`) и о движковых
//! плагинах клиента/сервера; компоненты — только данные, логика — в системах.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const GAME_NAME: &str = "Space Station R";

pub mod atmosphere;
pub mod inventory;
pub mod items;
pub mod power;
pub mod prototypes;
pub mod recipes;
pub mod roles;
pub mod rsi;
pub mod tiles;

/// Корень workspace (для доступа к assets/ из инструментов и headless-сервера).
/// Работает благодаря compile-time CARGO_MANIFEST_DIR крейтов workspace.
pub fn repo_root() -> &'static std::path::Path {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
}

/// Каталог ассетов.
pub fn assets_root() -> std::path::PathBuf {
    repo_root().join("assets")
}

/// Скорость игрока, пикселей в секунду (сервер применяет ввод по ADR-3).
pub const PLAYER_MOVE_SPEED: f32 = 300.0;

/// Размер тайла в юнитах мира: 1 тайл = 1 юнит сетки (ADR-8, IMP-1),
/// спрайты 32 px. Все мировые координаты в юнитах, тайл — минимальная единица.
pub const TILE_SIZE: f32 = 32.0;

/// Размер чанка в юнитах: ADR-5 (чанк = 32×32 тайла).
pub const CHUNK_UNITS: f32 = TILE_SIZE * tiles::CHUNK_TILES as f32;

/// Координаты ЧАНКА (ADR-5: 32×32 тайла) точки мира.
/// Именно в этой сетке работает interest management (ADR-4) и чанки карты.
pub fn chunk_coords(x: f32, y: f32) -> (i32, i32) {
    (
        (x / CHUNK_UNITS).floor() as i32,
        (y / CHUNK_UNITS).floor() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_coords_floors_negatives() {
        // Чанк = 1024 юнита (32 тайла × 32 юнита).
        assert_eq!(chunk_coords(0.0, 0.0), (0, 0));
        assert_eq!(chunk_coords(1023.9, -0.1), (0, -1));
        assert_eq!(chunk_coords(1024.0, 2048.0), (1, 2));
        assert_eq!(chunk_coords(-33.0, 33.0), (-1, 0));
    }
}

/// Сервер-авторитарная позиция игрока (ADR-3). Реплицируется клиентам (T1.3).
///
/// Внутренний тип — массив, чтобы сериализация не зависела от фич bevy;
/// координаты сетки тайлов (`IVec2`, ADR-8) появятся в фазе 2.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerPosition(pub [f32; 2]);

/// Дверь (T3.1): состояние меняется только сервером по Interact,
/// реплицируется клиентам (ADR-4); позиция статична, едет вместе с ней.
/// `access` (T4.2) — ключ доступа из карты: без него дверь открыта всем.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Door {
    pub open: bool,
    pub position: [f32; 2],
    #[serde(default)]
    pub access: Option<String>,
}

/// Радиус взаимодействия, юниты (PLAN.md T3.1: 1.5 тайла).
pub const INTERACT_RANGE: f32 = 1.5 * tiles::TILE_PX as f32;

/// Раса/вид игрока (T5.3): спрайты тела — `Mobs/Species/<id>/parts.rsi`.
/// Выбирается сервером (SSR_SPECIES), реплицируется клиенту.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Species {
    pub id: String,
}

impl Default for Species {
    fn default() -> Self {
        Self {
            id: "Human".to_string(),
        }
    }
}
