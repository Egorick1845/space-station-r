//! Чтение карт SS14 НАПРЯМУЮ (YAML формата 6/7, `MapChunkSerializer.cs`
//! движка: v7 — 7 байт/тайл, v6 — 6, v<6 — 4). Раньше карта проходила через
//! RON-импортёр, который читал позиции сущностей как мировые — на деле они
//! ГРИД-ЛОКАЛЬНЫЕ (`parent: <грид>`), и двери/спавны/лампы попадали в космос:
//! комнаты «разгерметизировались», игрок умирал от вакуума на спавне.
//!
//! Классификация: `Wall*`/`*Window*`/`*Grille*` → стена (окна герметичны),
//! `*Airlock*`/`Door*`/`Windoor*` → дверь, `SpawnPoint*` → спавн, остальное —
//! через [`map_entity`] (лампы, мебель, шкафы, предметы). `Lattice` — космос.

use std::collections::HashMap;
use std::path::Path;

use base64::Engine as _;
use serde_yaml_ng::Value;

use crate::tiles::{CHUNK_TILES, MapFile, TileChunk, TileType};

/// Наш масштаб: 1 тайл SS14 = 32 юнита мира (IMP-1).
const TILE_UNITS: f32 = 32.0;

/// Читает карту SS14 (.yml) и переводит в наш `MapFile` (в памяти).
pub fn load(path: &Path) -> Result<MapFile, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let root: Value =
        serde_yaml_ng::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))?;

    let tilemap: HashMap<i64, String> = root["tilemap"]
        .as_mapping()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.as_i64()?, v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();

    // Смещения гридов: uid → (tx, ty) в тайлах (Transform.pos грида).
    let mut grid_offsets: HashMap<i64, (i32, i32)> = HashMap::new();
    // Мировая карта тайлов.
    let mut tiles: HashMap<(i32, i32), TileType> = HashMap::new();
    let mut walls = 0u64;
    if let Some(entities) = root["entities"].as_sequence() {
        for group in entities {
            let Some(list) = group["entities"].as_sequence() else {
                continue;
            };
            for entity in list {
                let uid = as_id(&entity["uid"]).unwrap_or_default();
                let Some(components) = entity["components"].as_sequence() else {
                    continue;
                };
                let Some(((fx, fy), _)) = grid_transform(components) else {
                    continue;
                };
                let offset = (fx.floor() as i32, fy.floor() as i32);
                grid_offsets.insert(uid, offset);
                if let Some(chunks) = components
                    .iter()
                    .find(|comp| comp["type"].as_str() == Some("MapGrid"))
                    .and_then(|grid| grid["chunks"].as_mapping())
                {
                    for (_, chunk) in chunks {
                        decode_chunk(chunk, offset, &tilemap, &mut tiles, &mut walls);
                    }
                }
            }
        }
    }

    // Сущности: двери, спавны, переносимая мебель/предметы. Позиция в YAML —
    // ГРИД-ЛОКАЛЬНАЯ (`parent: <грид>`): к ней добавляется смещение грида.
    let mut spawn_points = Vec::new();
    let mut doors = Vec::new();
    let mut door_access = Vec::new();
    let mut map_entities: Vec<(String, f32, f32)> = Vec::new();
    let mut skipped = 0u64;
    if let Some(entities) = root["entities"].as_sequence() {
        for group in entities {
            let proto = group["proto"].as_str().unwrap_or_default();
            let Some(list) = group["entities"].as_sequence() else {
                continue;
            };
            let kind = classify_proto(proto);
            // Группа целиком игнорируется, только если ничего не переносится.
            if kind == ProtoKind::Ignored && map_entity(proto).is_none() {
                skipped += list.len() as u64;
                continue;
            }
            for entity in list {
                let Some(components) = entity["components"].as_sequence() else {
                    continue;
                };
                let Some(((x, y), transform_parent)) = grid_transform(components) else {
                    continue;
                };
                // Грид-локальные координаты + смещение грида-родителя
                // (`parent` внутри Transform; сущности вне гридов — мир).
                let (wx, wy) = match transform_parent
                    .and_then(|uid| grid_offsets.get(&uid).copied())
                {
                    Some((ox, oy)) => (x + ox as f32, y + oy as f32),
                    None => (x, y),
                };
                match kind {
                    ProtoKind::Wall => {
                        tiles.insert((wx.floor() as i32, wy.floor() as i32), TileType::Wall);
                        walls += 1;
                    }
                    ProtoKind::Door => {
                        doors.push((wx * TILE_UNITS, wy * TILE_UNITS));
                        // Доступ двери — из компонентов (`AccessReader`), при
                        // отсутствии — всем открытая.
                        // Доступ двери: первая группа тегов `AccessReader`
                        // (`access: [[ "Command" ]]` в SS14).
                        let access = components
                            .iter()
                            .find(|comp| comp["type"].as_str() == Some("AccessReader"))
                            .and_then(|reader| reader["access"].as_sequence())
                            .and_then(|lists| lists.first())
                            .and_then(|list| list.as_sequence())
                            .and_then(|tags| tags.first())
                            .and_then(|tag| tag.as_str())
                            .map(|tag| tag.to_lowercase())
                            .unwrap_or_else(|| "general".to_string());
                        door_access.push(crate::tiles::DoorAccess {
                            position: (wx * TILE_UNITS, wy * TILE_UNITS),
                            access,
                        });
                    }
                    ProtoKind::Spawn => {
                        spawn_points.push((wx * TILE_UNITS, wy * TILE_UNITS));
                    }
                    ProtoKind::Ignored => {
                        if let Some(id) = map_entity(proto) {
                            map_entities.push((id, wx * TILE_UNITS, wy * TILE_UNITS));
                        } else {
                            skipped += 1;
                        }
                    }
                }
            }
        }
    }

    // Чанки 32×32 из мировых тайлов.
    let mut chunks: HashMap<(i32, i32), TileChunk> = HashMap::new();
    for (&(tx, ty), &kind) in &tiles {
        let coords = (
            tx.div_euclid(CHUNK_TILES as i32),
            ty.div_euclid(CHUNK_TILES as i32),
        );
        let chunk = chunks
            .entry(coords)
            .or_insert_with(|| TileChunk::at(coords.0, coords.1));
        chunk.set_local(
            (tx - coords.0 * CHUNK_TILES as i32) as u32,
            (ty - coords.1 * CHUNK_TILES as i32) as u32,
            kind,
        );
    }
    let mut chunk_list: Vec<TileChunk> = chunks.into_values().collect();
    chunk_list.sort_by_key(|c| (c.coords.y, c.coords.x));

    // Спавны: дедупликация, до 32 точек; точки на космосе/стене выкидываются —
    // иначе игрок спавнится в вакуум (Latejoin дев-карты стоит за стеной).
    spawn_points.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    spawn_points.dedup();
    spawn_points.retain(|(x, y)| {
        matches!(
            tiles.get(&((*x / TILE_UNITS).floor() as i32, (*y / TILE_UNITS).floor() as i32)),
            Some(TileType::Floor)
        )
    });
    spawn_points.truncate(32);
    if spawn_points.is_empty() {
        // Фолбэк: центр главной комнаты (первый пол в сетке).
        if let Some((&(tx, ty), _)) = tiles.iter().find(|(_, kind)| **kind == TileType::Floor) {
            spawn_points.push((
                tx as f32 * TILE_UNITS + TILE_UNITS / 2.0,
                ty as f32 * TILE_UNITS + TILE_UNITS / 2.0,
            ));
        }
    }

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("map")
        .to_string();
    println!(
        "ss14 map: {} чанков, стен {}, дверей {}, спавнов {}, сущностей {}, пропущено {}",
        chunk_list.len(),
        walls,
        doors.len(),
        spawn_points.len(),
        map_entities.len(),
        skipped
    );
    // Диагностика герметичности: тайл под спавном обязан быть полом.
    for (x, y) in &spawn_points {
        let tx = (*x / TILE_UNITS).floor() as i32;
        let ty = (*y / TILE_UNITS).floor() as i32;
        let kind = tiles.get(&(tx, ty)).copied();
        println!("  спавн: тайл ({tx}, {ty}) = {kind:?}");
    }

    Ok(MapFile {
        name,
        spawn_points,
        doors,
        door_access,
        cables: Vec::new(),
        generators: Vec::new(),
        lights: Vec::new(),
        entities: map_entities,
        chunks: chunk_list
            .iter()
            .map(|c| crate::tiles::MapChunkFile {
                coords: (c.coords.x, c.coords.y),
                rows: c.rows(),
            })
            .collect(),
    })
}

/// Идентификатор сущности в YAML: число или СТРОКА (`uid: 123` / `uid: "123"`).
fn as_id(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

/// `Transform` сущности: позиция в тайлах + РОДИТЕЛЬ (uid грида) — в картах
/// SS14 `parent` лежит ВНУТРИ Transform (`pos: 33.5,18.5, parent: 1`).
fn grid_transform(components: &[Value]) -> Option<((f32, f32), Option<i64>)> {
    let transform = components
        .iter()
        .find(|comp| comp["type"].as_str() == Some("Transform"))?;
    let pos = transform.get("pos").and_then(|p| p.as_str()).and_then(parse_pos)?;
    let parent = transform.get("parent").and_then(as_id);
    Some((pos, parent))
}

#[derive(PartialEq)]
enum ProtoKind {
    Wall,
    Door,
    Spawn,
    Ignored,
}

fn classify_proto(proto: &str) -> ProtoKind {
    if proto.starts_with("Wall") || proto.contains("Window") || proto.contains("Grille") {
        ProtoKind::Wall
    } else if proto.contains("Airlock") || proto.starts_with("Door") || proto.contains("Windoor") {
        ProtoKind::Door
    } else if proto.starts_with("SpawnPoint") {
        ProtoKind::Spawn
    } else {
        ProtoKind::Ignored
    }
}

/// Сущности карты для спавна (лампы, мебель, шкафы, предметы).
fn map_entity(proto: &str) -> Option<String> {
    if proto.starts_with("Poweredlight") || proto.starts_with("Lamp") {
        return Some("light".to_string());
    }
    if proto.starts_with("Table")
        || proto.starts_with("Chair")
        || proto.starts_with("ComfyChair")
        || proto.starts_with("OfficeChair")
        || proto.starts_with("SeatBase")
        || proto.starts_with("Star")
    {
        return Some(proto.to_string());
    }
    if proto.starts_with("Locker")
        || proto.starts_with("Cabinet")
        || proto.starts_with("Crate")
        || proto.starts_with("Toolbox")
    {
        return Some(proto.to_string());
    }
    if proto.starts_with("MedicalBed")
        || proto.starts_with("OperatingTable")
        || proto.starts_with("ChemDispenser")
        || proto.starts_with("ChemMaster")
        || proto.starts_with("Protolathe")
        || proto.starts_with("CloningPod")
        || proto.starts_with("Morgue")
        || proto.starts_with("Crematorium")
        || proto.starts_with("SMES")
        || proto.starts_with("Substation")
        || proto.starts_with("DebugGenerator")
        || proto.starts_with("DebugAPC")
        || proto.starts_with("GravityGenerator")
        || proto.starts_with("AirAlarm")
        || proto.starts_with("HighSecDoor")
        || proto.starts_with("Altar")
        || proto.starts_with("VendingMachine")
        || proto.starts_with("Wardrobe")
    {
        return Some(proto.to_string());
    }
    if proto.starts_with("Weapon")
        || proto.starts_with("Medkit")
        || proto.starts_with("Flashlight")
        || proto.starts_with("Crowbar")
        || proto.starts_with("Wrench")
        || proto.starts_with("Screwdriver")
        || proto.starts_with("Multitool")
        || proto.starts_with("Welder")
        || proto.starts_with("Magazine")
        || proto.starts_with("Box")
        || proto.starts_with("Grenade")
        || proto.starts_with("trayScanner")
        || proto.starts_with("computer")
    {
        return Some(proto.to_string());
    }
    None
}

/// Тип нашего тайла по имени тайла SS14.
fn tile_kind(name: &str) -> TileType {
    if name.starts_with("Space") || name.starts_with("Lattice") {
        // Решётка открыта космосу (вакуум).
        TileType::Space
    } else if name.starts_with("Wall") {
        TileType::Wall
    } else {
        // Plating, Floor* и всё остальное — ходибельный пол.
        TileType::Floor
    }
}

/// `"12.5,-0.5"` → `(12.5, -0.5)`.
fn parse_pos(text: &str) -> Option<(f32, f32)> {
    let (x, y) = text.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Декодирует один чанк грида в мировые тайлы.
fn decode_chunk(
    chunk: &Value,
    offset: (i32, i32),
    tilemap: &HashMap<i64, String>,
    tiles: &mut HashMap<(i32, i32), TileType>,
    walls: &mut u64,
) {
    let Some(tiles_b64) = chunk["tiles"].as_str() else {
        return;
    };
    let version = chunk["version"].as_i64().unwrap_or(1);
    let size = chunk["size"].as_i64().unwrap_or(16) as usize;
    let index = chunk["ind"].as_str().unwrap_or("0,0");
    let Some((cx, cy)) = parse_pos(index) else {
        return;
    };
    let (cx, cy) = (cx as i32, cy as i32);

    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(tiles_b64) else {
        return;
    };

    // Байт на тайл по версии (MapChunkSerializer.cs движка).
    let stride = if version >= 7 {
        7
    } else if version >= 6 {
        6
    } else {
        4
    };
    let mut offset_bytes = 0usize;
    for y in 0..size {
        for x in 0..size {
            if offset_bytes + stride > bytes.len() {
                return;
            }
            let id: i64 = if version >= 6 {
                i32::from_le_bytes(bytes[offset_bytes..offset_bytes + 4].try_into().unwrap()) as i64
            } else {
                u16::from_le_bytes(bytes[offset_bytes..offset_bytes + 2].try_into().unwrap()) as i64
            };
            offset_bytes += stride;
            let Some(name) = tilemap.get(&id) else {
                continue;
            };
            let kind = tile_kind(name);
            if kind == TileType::Wall {
                *walls += 1;
            }
            let tx = cx * size as i32 + x as i32 + offset.0;
            let ty = cy * size as i32 + y as i32 + offset.1;
            tiles.insert((tx, ty), kind);
        }
    }
}
