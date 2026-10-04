//! Геометрия окклюдеров для теней и FOV — модель движка
//! (`Robust.Client/GameObjects/EntitySystems/OccluderSystem` и
//! `ClientOccluderSystem`): из тайловой карты строятся ОТРЕЗКИ-ГРАНИ.
//! Ребро сплошного тайла становится окклюдером, кроме ребра, общего с соседним
//! сплошным тайлом: в движке «shared edges» отбрасываются (внутри массива стен
//! лишние грани не нужны), а наружные грани массива остаются замкнутым контуром.
//! Закрытая дверь — отдельный окклюдер-квадрат (в SS14 у двери свой
//! `OccluderComponent`).

use std::collections::{HashMap, HashSet};

use crate::tiles::{CHUNK_TILES, TILE_PX, TileChunkData, TileType};

/// Отрезок-окклюдер в мировых единицах (1 тайл = `TILE_PX`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OccluderSegment {
    pub a: (f32, f32),
    pub b: (f32, f32),
}

/// Сплошной ли тайл для света: стены. (Космос и техпол свет не перекрывают —
/// в движке окклюдер ставится компонентом, у нас его роль играет стена.)
pub fn is_solid(tile: TileType) -> bool {
    matches!(tile, TileType::Wall)
}

/// Строит окклюдеры из реплицированных чанков и закрытых дверей.
///
/// `doors` — тайлы закрытых дверей: у них окклюдер есть, но они не считаются
/// «соседями» для отбрасывания общих рёбер стен (дверь — отдельная сущность,
/// как `OccluderComponent` в сборке).
pub fn build_occluders(chunks: &[&TileChunkData], doors: &[(i32, i32)]) -> Vec<OccluderSegment> {
    let size = CHUNK_TILES as i32;
    let mut map: HashMap<(i32, i32), &TileChunkData> = HashMap::new();
    for chunk in chunks {
        map.insert((chunk.coords.0, chunk.coords.1), chunk);
    }
    let tile_at = |tx: i32, ty: i32| -> TileType {
        let coords = (tx.div_euclid(size), ty.div_euclid(size));
        map.get(&coords)
            .map(|chunk| {
                chunk.get_local((tx - coords.0 * size) as u32, (ty - coords.1 * size) as u32)
            })
            .unwrap_or(TileType::Space)
    };
    let doors: HashSet<(i32, i32)> = doors.iter().copied().collect();
    // Стена для окклюзии: сплошной тайл (двери — отдельно, они не «массив стен»).
    let wall = |tx: i32, ty: i32| -> bool { is_solid(tile_at(tx, ty)) };

    let px = TILE_PX as f32;
    let mut segments = Vec::new();
    let mut push = |a: (f32, f32), b: (f32, f32)| {
        segments.push(OccluderSegment { a, b });
    };
    for chunk in chunks {
        for ly in 0..CHUNK_TILES as i32 {
            for lx in 0..CHUNK_TILES as i32 {
                let tx = chunk.coords.0 * size + lx;
                let ty = chunk.coords.1 * size + ly;
                let is_door = doors.contains(&(tx, ty));
                if !is_door && !wall(tx, ty) {
                    continue;
                }
                let (x0, y0) = (tx as f32 * px, ty as f32 * px);
                let (x1, y1) = (x0 + px, y0 + px);
                // Грань — окклюдер, если сосед с этой стороны не сплошной
                // (у двери рёбра есть всегда: она отдельная сущность).
                let solid_neighbor = |nx: i32, ny: i32| -> bool { !is_door && wall(nx, ny) };
                if !solid_neighbor(tx, ty - 1) {
                    push((x0, y0), (x1, y0));
                }
                if !solid_neighbor(tx, ty + 1) {
                    push((x0, y1), (x1, y1));
                }
                if !solid_neighbor(tx - 1, ty) {
                    push((x0, y0), (x0, y1));
                }
                if !solid_neighbor(tx + 1, ty) {
                    push((x1, y0), (x1, y1));
                }
            }
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::TileChunk;

    fn chunk_with(walls: &[(i32, i32)]) -> TileChunkData {
        let mut chunk = TileChunk::at(0, 0);
        for (lx, ly) in walls {
            chunk.set_local(*lx as u32, *ly as u32, TileType::Wall);
        }
        TileChunkData::from(&chunk)
    }

    #[test]
    fn single_wall_has_four_edges() {
        let chunk = chunk_with(&[(5, 5)]);
        let segments = build_occluders(&[&chunk], &[]);
        assert_eq!(segments.len(), 4, "одиночная стена — 4 грани");
    }

    #[test]
    fn adjacent_walls_share_edges() {
        // Две стены рядом: 8 рёбер минус 2 общих = 6.
        let chunk = chunk_with(&[(5, 5), (6, 5)]);
        let segments = build_occluders(&[&chunk], &[]);
        assert_eq!(segments.len(), 6, "общая грань соседних стен не окклюдер");
    }

    #[test]
    fn closed_door_is_occluder() {
        let chunk = chunk_with(&[(5, 5)]);
        let with_door = build_occluders(&[&chunk], &[(9, 9)]);
        assert_eq!(with_door.len(), 8, "стена + дверь-квадрат");
    }
}
