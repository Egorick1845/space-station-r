//! Тайлы мира (PLAN.md T2.1, ADR-5/ADR-6): чанки 32×32, прототипы в .ron.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Тайлов в чанке по оси (ADR-5).
pub const CHUNK_TILES: u32 = 32;

/// Пикселей спрайта на тайл (1 тайл = 1 юнит мира, IMP-1; спрайты 32px).
pub const TILE_PX: u32 = 32;

/// Тип тайла. Сериализуем для репликации карты (T2.3) и сохранений.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TileType {
    /// Открытый космос: нет спрайта, нет коллизии.
    #[default]
    Space,
    /// Пол: ходибельный.
    Floor,
    /// Стена: блокирует проход (коллизии — T2.2).
    Wall,
}

impl TileType {
    /// Ключ прототипа в `assets/prototypes/tiles.ron`.
    pub fn key(self) -> &'static str {
        match self {
            TileType::Space => "space",
            TileType::Floor => "floor",
            TileType::Wall => "wall",
        }
    }
}

/// Прототип тайла (ADR-6: контент — данные, не код).
#[derive(Debug, Clone, Deserialize)]
pub struct TileProto {
    /// Спрайт относительно `assets/` (полоса вариантов 32×N); None — не рисуется.
    pub sprite: Option<String>,
    /// Блокирует проход (используется физикой с T2.2).
    #[serde(default)]
    pub solid: bool,
}

/// Файл `assets/prototypes/tiles.ron`.
#[derive(Debug, Clone, Deserialize)]
pub struct TilePrototypes {
    pub tiles: HashMap<String, TileProto>,
}

impl TilePrototypes {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    pub fn get(&self, tile: TileType) -> &TileProto {
        self.tiles
            .get(tile.key())
            .unwrap_or_else(|| panic!("tile prototype missing: {}", tile.key()))
    }
}

/// Чанк карты 32×32 тайла. Координаты чанка — в чанках, не в тайлах.
#[derive(Component, Debug, Clone)]
pub struct TileChunk {
    /// Координата чанка в сетке чанков.
    pub coords: IVec2,
    tiles: Vec<TileType>,
}

impl TileChunk {
    pub fn new(coords: IVec2) -> Self {
        Self {
            coords,
            tiles: vec![TileType::default(); (CHUNK_TILES * CHUNK_TILES) as usize],
        }
    }

    fn index(lx: u32, ly: u32) -> usize {
        (ly * CHUNK_TILES + lx) as usize
    }

    /// Тайл по локальным координатам внутри чанка (0..32).
    pub fn get_local(&self, lx: u32, ly: u32) -> TileType {
        self.tiles[Self::index(lx, ly)]
    }

    /// Тайл по глобальным координатам тайлов.
    pub fn get(&self, tx: i32, ty: i32) -> TileType {
        let lx = (tx - self.coords.x * CHUNK_TILES as i32) as u32;
        let ly = (ty - self.coords.y * CHUNK_TILES as i32) as u32;
        self.get_local(lx, ly)
    }

    pub fn set_local(&mut self, lx: u32, ly: u32, tile: TileType) {
        self.tiles[Self::index(lx, ly)] = tile;
    }

    pub fn tiles(&self) -> &[TileType] {
        &self.tiles
    }
}

/// Тестовая карта 128×128 тайлов (чанки −2..1): космос по краю, стена-рамка,
/// пол с колоннами каждые 16 тайлов. Детерминированная, без rand.
pub fn gen_test_map() -> Vec<TileChunk> {
    let mut chunks = Vec::new();
    for cy in -2..=1 {
        for cx in -2..=1 {
            let mut chunk = TileChunk::new(IVec2::new(cx, cy));
            for ly in 0..CHUNK_TILES {
                for lx in 0..CHUNK_TILES {
                    let tx = cx * CHUNK_TILES as i32 + lx as i32;
                    let ty = cy * CHUNK_TILES as i32 + ly as i32;
                    let (ax, ay) = (tx.abs(), ty.abs());
                    let tile = if ax >= 63 || ay >= 63 {
                        TileType::Space
                    } else if ax == 62 || ay == 62 || (tx % 16 == 8 && ty % 16 == 8) {
                        TileType::Wall
                    } else {
                        TileType::Floor
                    };
                    chunk.set_local(lx, ly, tile);
                }
            }
            chunks.push(chunk);
        }
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prototypes_parse_from_assets() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root")
            .join("assets/prototypes/tiles.ron");
        let protos = TilePrototypes::load(&path).expect("parse tiles.ron");
        assert_eq!(protos.tiles.len(), 3);
        assert!(protos.get(TileType::Floor).sprite.is_some());
        assert!(protos.get(TileType::Wall).solid);
        assert!(protos.get(TileType::Space).sprite.is_none());
    }

    #[test]
    fn test_map_layout() {
        let chunks = gen_test_map();
        assert_eq!(chunks.len(), 16); // 4×4 чанка = 128×128 тайлов
        let chunk = |cx: i32, cy: i32| {
            chunks
                .iter()
                .find(|c| c.coords == IVec2::new(cx, cy))
                .expect("chunk exists")
        };
        let center = chunk(0, 0);
        // Центр карты — пол (колонны сдвинуты на 8 тайлов, чтобы не мешать спавну).
        assert_eq!(center.get(0, 0), TileType::Floor);
        // Колонны на (16k+8, 16m+8).
        assert_eq!(center.get(8, 8), TileType::Wall);
        assert_eq!(center.get(16, 16), TileType::Floor);
        // Рамка стен на |62|, космос на |63|+ — тайлы 32..63 живут в чанке (1,0).
        let east = chunk(1, 0);
        assert_eq!(east.get(62, 0), TileType::Wall);
        assert_eq!(east.get(63, 0), TileType::Space);
        // Углы карты (тайл −64) — космос.
        let corner = chunk(-2, -2);
        assert_eq!(corner.get(-64, -64), TileType::Space);
    }
}
