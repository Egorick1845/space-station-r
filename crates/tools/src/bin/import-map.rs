//! Импорт карт SS14 (SS14_IMPORT.md §5, задача IMP.4): YAML формата 6/7 →
//! наш `.ron` (`MapFile`). Тайлы чанков читаются по коду движка
//! (`MapChunkSerializer.cs`): v7 — i32 id + flags + variant + rotation (7 байт),
//! v6 — 6 байт, v<6 — u16 id + flags + variant (4 байта); обход построчный.
//!
//! Запуск: `cargo run -p ssr-tools --bin import-map -- <in.yml> <out.ron>`

use std::collections::HashMap;
use std::path::Path;

use base64::Engine as _;
use serde_yaml_ng::Value;
use ssr_core::tiles::{CHUNK_TILES, MapFile, TileChunk, TileType};

/// Наш масштаб: 1 тайл SS14 = 32 юнита мира (IMP-1).
const TILE_UNITS: f32 = 32.0;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: import-map <in.yml> <out.ron>");
        std::process::exit(2);
    }
    let (input, output) = (&args[1], &args[2]);

    let text = std::fs::read_to_string(input).expect("read map");
    let root: Value = serde_yaml_ng::from_str(&text).expect("parse map yaml");

    let tilemap: HashMap<i64, String> = root["tilemap"]
        .as_mapping()
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.as_i64()?, v.as_str()?.to_string())))
                .collect()
        })
        .unwrap_or_default();
    println!("tilemap: {} записей", tilemap.len());

    // Мировая карта тайлов: (tx, ty) → тип.
    let mut tiles: HashMap<(i32, i32), TileType> = HashMap::new();
    let mut counters = Report::default();

    // --- Тайлы из гридов (проходим все MapGrid, но гриды не первого смещения тоже поддержим) ---
    let mut grid_offsets: Vec<(i64, (i64, i64))> = Vec::new(); // uid грида → смещение
    if let Some(entities) = root["entities"].as_sequence() {
        for group in entities {
            let Some(list) = group["entities"].as_sequence() else {
                continue;
            };
            for entity in list {
                let Some(components) = entity["components"].as_sequence() else {
                    continue;
                };
                let uid = entity["uid"].as_i64().unwrap_or_default();
                let mut offset = (0i64, 0i64);
                for component in components {
                    if component["type"].as_str() == Some("Transform") {
                        offset = parse_pos(component.get("pos").and_then(|p| p.as_str()))
                            .map(|(x, y)| (x.floor() as i64, y.floor() as i64))
                            .unwrap_or((0, 0));
                    }
                    if component["type"].as_str() == Some("MapGrid")
                        && let Some(chunks) = component["chunks"].as_mapping()
                    {
                        grid_offsets.push((uid, offset));
                        for (_, chunk) in chunks {
                            decode_chunk(chunk, offset, &tilemap, &mut tiles, &mut counters);
                        }
                    }
                }
            }
        }
    }
    println!("тайлов из гридов: {}", counters.grid_tiles);

    // --- Сущности: стены, двери, точки спавна ---
    let mut spawn_points = Vec::new();
    let mut doors = Vec::new();
    if let Some(entities) = root["entities"].as_sequence() {
        for group in entities {
            let proto = group["proto"].as_str().unwrap_or_default();
            let Some(list) = group["entities"].as_sequence() else {
                continue;
            };
            let kind = classify_proto(proto);
            if kind == ProtoKind::Ignored {
                counters.skipped_protos += list.len() as u64;
                continue;
            }
            for entity in list {
                let components = entity["components"].as_sequence();
                let pos = components
                    .and_then(|c| {
                        c.iter()
                            .find(|comp| comp["type"].as_str() == Some("Transform"))
                    })
                    .and_then(|comp| comp.get("pos"))
                    .and_then(|p| p.as_str())
                    .and_then(|p| parse_pos(Some(p)));
                let Some((x, y)) = pos else {
                    counters.without_transform += 1;
                    continue;
                };
                match kind {
                    ProtoKind::Wall => {
                        tiles.insert((x.floor() as i32, y.floor() as i32), TileType::Wall);
                        counters.walls += 1;
                    }
                    ProtoKind::Door => {
                        doors.push((x * TILE_UNITS, y * TILE_UNITS));
                        counters.doors += 1;
                    }
                    ProtoKind::Spawn => {
                        spawn_points.push((x * TILE_UNITS, y * TILE_UNITS));
                        counters.spawns += 1;
                    }
                    ProtoKind::Ignored => {}
                }
            }
        }
    }

    // --- Сборка чанков 32×32 из тайлов ---
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

    // Дедупликация точек спавна (карты часто содержат десятки), берём до 32.
    spawn_points.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    spawn_points.dedup();
    spawn_points.truncate(32);
    let _ = grid_offsets;

    let name = Path::new(input)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("map")
        .to_string();
    MapFile::save(
        Path::new(output),
        &name,
        spawn_points.clone(),
        doors.clone(),
        &chunk_list,
    )
    .expect("save map");
    println!(
        "карта сохранена: {output}\n  чанков: {}\n  стен: {}\n  дверей: {}\n  спавнов: {}\n  \
         пропущено сущностей: {} (без Transform: {})",
        chunk_list.len(),
        counters.walls,
        doors.len(),
        spawn_points.len(),
        counters.skipped_protos,
        counters.without_transform
    );
}

#[derive(Default)]
struct Report {
    grid_tiles: u64,
    walls: u64,
    doors: u64,
    spawns: u64,
    skipped_protos: u64,
    without_transform: u64,
}

#[derive(PartialEq)]
enum ProtoKind {
    Wall,
    Door,
    Spawn,
    Ignored,
}

/// Классификация прототипа SS14 для нашего базового мира.
fn classify_proto(proto: &str) -> ProtoKind {
    if proto.starts_with("Wall") {
        ProtoKind::Wall
    } else if proto.contains("Airlock") || proto.starts_with("Door") || proto.contains("Windoor") {
        ProtoKind::Door
    } else if proto.starts_with("SpawnPoint") {
        ProtoKind::Spawn
    } else {
        ProtoKind::Ignored
    }
}

/// Тип нашего тайла по имени тайла SS14.
fn tile_kind(name: &str) -> TileType {
    if name.starts_with("Space") {
        TileType::Space
    } else if name.starts_with("Wall") {
        TileType::Wall
    } else {
        // Plating, Floor* и всё остальное — ходибельный пол.
        TileType::Floor
    }
}

/// `"12.5,-0.5"` → `(12.5, -0.5)`.
fn parse_pos(text: Option<&str>) -> Option<(f32, f32)> {
    let (x, y) = text?.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Декодирует один чанк грида (форматы 6/7/ранее) в мировые тайлы.
fn decode_chunk(
    chunk: &Value,
    offset: (i64, i64),
    tilemap: &HashMap<i64, String>,
    tiles: &mut HashMap<(i32, i32), TileType>,
    report: &mut Report,
) {
    let Some(tiles_b64) = chunk["tiles"].as_str() else {
        return;
    };
    let version = chunk["version"].as_i64().unwrap_or(1);
    let size = chunk["size"].as_i64().unwrap_or(16) as usize;
    let index = chunk["ind"].as_str().unwrap_or("0,0");
    let (cx, cy) = parse_pos(Some(index)).unwrap_or((0.0, 0.0));
    let (cx, cy) = (cx as i32, cy as i32);

    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(tiles_b64) else {
        eprintln!("чанк {index}: не декодируется base64");
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
            let tx = cx * size as i32 + x as i32 + offset.0 as i32;
            let ty = cy * size as i32 + y as i32 + offset.1 as i32;
            tiles.insert((tx, ty), kind);
            report.grid_tiles += 1;
        }
    }
}
