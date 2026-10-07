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
    /// Пол с покрытием: ходибельный, кабели под ним скрыты (T4.4+).
    Floor,
    /// Технический пол (плиты без покрытия): кабели видны, ходибельный.
    Plating,
    /// Стена: блокирует проход (коллизии — T2.2).
    Wall,
}

impl TileType {
    /// Ключ прототипа в `assets/prototypes/tiles.ron`.
    pub fn key(self) -> &'static str {
        match self {
            TileType::Space => "space",
            TileType::Floor => "floor",
            TileType::Plating => "plating",
            TileType::Wall => "wall",
        }
    }

    /// Ходибельный тайл (пол или техпол).
    pub fn is_walkable(self) -> bool {
        matches!(self, TileType::Floor | TileType::Plating)
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
    /// Конструктор по целочисленным координатам чанка.
    pub fn at(cx: i32, cy: i32) -> Self {
        Self::new(IVec2::new(cx, cy))
    }

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

    /// Текстовые строки чанка для файла карты (T2.3).
    pub fn rows(&self) -> Vec<String> {
        self.tiles
            .chunks(CHUNK_TILES as usize)
            .map(|row| row.iter().map(|t| tile_to_char(*t)).collect())
            .collect()
    }
}

/// Символ тайла в файле карты: ' ' — космос, '.' — пол, '=' — техпол,
/// '#' — стена.
fn tile_to_char(tile: TileType) -> char {
    match tile {
        TileType::Space => ' ',
        TileType::Floor => '.',
        TileType::Plating => '=',
        TileType::Wall => '#',
    }
}

fn tile_from_char(c: char) -> Result<TileType, String> {
    match c {
        ' ' => Ok(TileType::Space),
        '.' => Ok(TileType::Floor),
        '=' => Ok(TileType::Plating),
        '#' => Ok(TileType::Wall),
        other => Err(format!("unknown tile char {other:?}")),
    }
}

/// Реплицируемый чанк карты: сервер → клиент (T2.3, ADR-7).
#[derive(Component, Debug, Clone, Serialize, Deserialize, Default)]
pub struct TileChunkData {
    /// Координата чанка в сетке чанков.
    pub coords: (i32, i32),
    /// Тайлы построчно, `ly * CHUNK_TILES + lx`.
    pub tiles: Vec<TileType>,
}

impl From<&TileChunk> for TileChunkData {
    fn from(chunk: &TileChunk) -> Self {
        Self {
            coords: (chunk.coords.x, chunk.coords.y),
            tiles: chunk.tiles().to_vec(),
        }
    }
}

impl TileChunkData {
    pub fn get_local(&self, lx: u32, ly: u32) -> TileType {
        self.tiles[(ly * CHUNK_TILES + lx) as usize]
    }
}

/// Файл карты `assets/maps/<name>.ron` (T2.3, ADR-6): правится руками,
/// перезапуска сервера достаточно — без перекомпиляции.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapFile {
    pub name: String,
    /// Точки спавна в юнитах мира (центр тайла).
    #[serde(default)]
    pub spawn_points: Vec<(f32, f32)>,
    /// Центры дверей в юнитах мира (T3.1).
    #[serde(default)]
    pub doors: Vec<(f32, f32)>,
    /// Доступы дверей (T4.2): какие двери требуют ключ роли.
    #[serde(default)]
    pub door_access: Vec<DoorAccess>,
    /// Кабели (T4.4): центры тайлов с проводом.
    #[serde(default)]
    pub cables: Vec<(f32, f32)>,
    /// Генераторы (T4.4): (x, y, кВт).
    #[serde(default)]
    pub generators: Vec<(f32, f32, f32)>,
    /// Сущности карты из сборки: (id прототипа, x, y) — лампы, столы, шкафы,
    /// предметы. Спавнит сервер тем же правилом, что команда `spawn`.
    #[serde(default)]
    pub entities: Vec<(String, f32, f32)>,
    /// Лампы (T4.4): центры тайлов.
    #[serde(default)]
    pub lights: Vec<(f32, f32)>,
    pub chunks: Vec<MapChunkFile>,
}

/// Доступ двери на карте (T4.2): позиция (центр, юниты) → ключ доступа.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DoorAccess {
    pub position: (f32, f32),
    pub access: String,
}

/// Всё, что генераторы карт кладут в файл, кроме чанков (T4.4+: двери,
/// доступы, кабели, генераторы, лампы).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MapLayout {
    pub name: String,
    pub spawn_points: Vec<(f32, f32)>,
    pub doors: Vec<(f32, f32)>,
    pub door_access: Vec<DoorAccess>,
    pub cables: Vec<(f32, f32)>,
    pub generators: Vec<(f32, f32, f32)>,
    /// Сущности карты (save): (прототип, x, y).
    #[serde(default)]
    pub entities: Vec<(String, f32, f32)>,
    pub lights: Vec<(f32, f32)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapChunkFile {
    /// Координата чанка в сетке чанков.
    pub coords: (i32, i32),
    /// 32 строки по 32 символа: ' ' космос, '.' пол, '#' стена.
    pub rows: Vec<String>,
}

impl MapFile {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    pub fn save(path: &Path, layout: MapLayout, chunks: &[TileChunk]) -> Result<(), String> {
        let MapLayout {
            name,
            spawn_points,
            doors,
            door_access,
            cables,
            generators,
            lights,
            entities,
        } = layout;
        let file = Self {
            name,
            spawn_points,
            doors,
            door_access,
            cables,
            generators,
            lights,
            entities,
            chunks: chunks
                .iter()
                .map(|c| MapChunkFile {
                    coords: (c.coords.x, c.coords.y),
                    rows: c.rows(),
                })
                .collect(),
        };
        let text = ron::ser::to_string_pretty(&file, ron::ser::PrettyConfig::default())
            .map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
    }

    /// Разбирает файл в чанки (валидация размеров и символов).
    pub fn to_chunks(&self) -> Result<Vec<TileChunk>, String> {
        self.chunks
            .iter()
            .map(|file_chunk| {
                if file_chunk.rows.len() != CHUNK_TILES as usize {
                    return Err(format!(
                        "chunk {:?}: {} строк, ожидается {CHUNK_TILES}",
                        file_chunk.coords,
                        file_chunk.rows.len()
                    ));
                }
                let mut chunk =
                    TileChunk::new(IVec2::new(file_chunk.coords.0, file_chunk.coords.1));
                for (ly, row) in file_chunk.rows.iter().enumerate() {
                    let chars: Vec<char> = row.chars().collect();
                    if chars.len() != CHUNK_TILES as usize {
                        return Err(format!(
                            "chunk {:?} строка {ly}: {} символов, ожидается {CHUNK_TILES}",
                            file_chunk.coords,
                            chars.len()
                        ));
                    }
                    for (lx, c) in chars.iter().enumerate() {
                        chunk.set_local(lx as u32, ly as u32, tile_from_char(*c)?);
                    }
                }
                Ok(chunk)
            })
            .collect()
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
    fn map_file_round_trip() {
        let chunks = gen_test_map();
        let dir = std::env::temp_dir().join("ssr-map-test");
        std::fs::create_dir_all(&dir).expect("temp dir");
        let path = dir.join("round-trip.ron");

        MapFile::save(
            &path,
            MapLayout {
                name: "test".into(),
                spawn_points: vec![(16.0, 16.0)],
                doors: vec![(144.0, 16.0)],
                door_access: vec![DoorAccess {
                    position: (144.0, 16.0),
                    access: "engineering".into(),
                }],
                cables: vec![(0.0, 16.0), (32.0, 16.0)],
                generators: vec![(0.0, 16.0, 20.0)],
                lights: vec![(32.0, 16.0)],
                entities: vec![],
            },
            &chunks,
        )
        .expect("save");
        let file = MapFile::load(&path).expect("load");
        assert_eq!(file.name, "test");
        assert_eq!(file.spawn_points, vec![(16.0, 16.0)]);
        assert_eq!(file.doors, vec![(144.0, 16.0)]);
        assert_eq!(file.door_access.len(), 1);
        assert_eq!(file.door_access[0].access, "engineering");
        assert_eq!(file.cables.len(), 2);
        assert_eq!(file.generators[0].2, 20.0);
        assert_eq!(file.lights.len(), 1);
        let restored = file.to_chunks().expect("to_chunks");
        assert_eq!(restored.len(), chunks.len());
        for (original, restored) in chunks.iter().zip(restored.iter()) {
            assert_eq!(original.coords, restored.coords);
            assert_eq!(original.tiles(), restored.tiles());
        }
    }

    #[test]
    fn map_file_rejects_bad_size() {
        let file = MapFile {
            name: "bad".into(),
            spawn_points: vec![],
            doors: vec![],
            door_access: vec![],
            cables: vec![],
            generators: vec![],
            lights: vec![],
            entities: vec![],
            chunks: vec![MapChunkFile {
                coords: (0, 0),
                rows: vec![".".to_string()],
            }],
        };
        assert!(file.to_chunks().is_err());
    }

    #[test]
    fn prototypes_parse_from_assets() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root")
            .join("assets/prototypes/tiles.ron");
        let protos = TilePrototypes::load(&path).expect("parse tiles.ron");
        assert_eq!(protos.tiles.len(), 4);
        assert!(protos.get(TileType::Floor).sprite.is_some());
        assert!(protos.get(TileType::Plating).sprite.is_some());
        assert!(!protos.get(TileType::Plating).solid);
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
