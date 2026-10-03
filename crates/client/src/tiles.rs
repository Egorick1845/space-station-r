//! Рендер тайловой карты (PLAN.md T2.1).
//!
//! Каждый чанк 32×32 тайлов собирается в одну картинку 1024×1024 px и
//! рисуется одним спрайтом. Стены рисуются с угловым сглаживанием как в
//! движке SS14 (`IconSmooth` в режиме Corners): 4 угловых слоя `solid0..7`
//! из `Structures/Walls/solid.rsi`, выбор слоя по 8 соседям клетки.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use image::RgbaImage;
use ssr_core::rsi::Rsi;
use ssr_core::tiles::{CHUNK_TILES, TILE_PX, TileChunkData, TileProto, TilePrototypes, TileType};

/// Прототипы и декодированные спрайты тайлов (грузятся один раз при старте).
#[derive(Resource)]
pub struct TileVisuals {
    protos: TilePrototypes,
    sprites: HashMap<String, RgbaImage>,
    /// RSI стен для углового сглаживания (`solid0..7`, 4 направления каждый).
    wall_rsi: Option<Rsi>,
}

/// Грузит прототипы и спрайты тайлов из каталога ассетов.
pub fn load_tile_visuals(root: &Path) -> TileVisuals {
    let protos = TilePrototypes::load(&root.join("prototypes/tiles.ron"))
        .expect("parse assets/prototypes/tiles.ron");
    let mut sprites = HashMap::new();
    for (key, proto) in &protos.tiles {
        let Some(rel) = &proto.sprite else { continue };
        let bytes = std::fs::read(root.join(rel.trim_start_matches('/')))
            .unwrap_or_else(|e| panic!("read tile sprite {rel}: {e}"));
        let image = image::load_from_memory(&bytes)
            .unwrap_or_else(|e| panic!("decode tile sprite {rel}: {e}"))
            .to_rgba8();
        sprites.insert(key.clone(), image);
    }
    let wall_dir = root.join("sprites/ss14/Structures/Walls/solid.rsi");
    let wall_rsi = match ssr_core::rsi::load_rsi(&wall_dir) {
        Ok(rsi) => Some(rsi),
        Err(e) => {
            tracing::warn!(path = %wall_dir.display(), error = %e, "wall rsi load failed");
            None
        }
    };
    TileVisuals {
        protos,
        sprites,
        wall_rsi,
    }
}

/// Выбирает вариант спрайта тайла детерминированно по координатам
/// (чек-лист SS14_IMPORT.md: варианты тайлов рандомизируются при рендере).
fn variant_index(tx: i32, ty: i32, variants: u32) -> u32 {
    ((tx as i64 * 7 + ty as i64 * 13).rem_euclid(variants as i64)) as u32
}

/// Тайл карты по глобальным координатам из доступных чанков.
fn tile_at(map: &HashMap<(i32, i32), &TileChunkData>, tx: i32, ty: i32) -> TileType {
    let cx = tx.div_euclid(CHUNK_TILES as i32);
    let cy = ty.div_euclid(CHUNK_TILES as i32);
    match map.get(&(cx, cy)) {
        Some(chunk) => chunk.get_local(
            (tx - cx * CHUNK_TILES as i32) as u32,
            (ty - cy * CHUNK_TILES as i32) as u32,
        ),
        None => TileType::Space,
    }
}

/// Кладёт 32×32 тайл из полосы вариантов в точку пиксельной сетки чанка.
fn blit_tile(canvas: &mut RgbaImage, sprite: &RgbaImage, sx: u32, px: u32, py: u32) {
    let view = image::imageops::crop_imm(sprite, sx, 0, TILE_PX, TILE_PX);
    let mut cell = RgbaImage::new(TILE_PX, TILE_PX);
    image::imageops::replace(&mut cell, &view.to_image(), 0, 0);
    image::imageops::overlay(canvas, &cell, px as i64, py as i64);
}

/// Клетка ячейки RSI (направление, кадр) из листа состояния.
fn blit_rsi_cell(canvas: &mut RgbaImage, sheet: &RgbaImage, cell: u32, px: u32, py: u32) {
    let (cols, _) = (
        (sheet.width() / TILE_PX).max(1),
        (sheet.height() / TILE_PX).max(1),
    );
    let col = cell % cols;
    let row = cell / cols;
    let view = image::imageops::crop_imm(sheet, col * TILE_PX, row * TILE_PX, TILE_PX, TILE_PX);
    let mut tile = RgbaImage::new(TILE_PX, TILE_PX);
    image::imageops::replace(&mut tile, &view.to_image(), 0, 0);
    image::imageops::overlay(canvas, &tile, px as i64, py as i64);
}

/// Рисует стену с угловым сглаживанием (правила IconSmooth/Corners движка):
/// четыре слоя `solid{corner}` со сдвигами направлений SE=юг, NE=запад,
/// NW=север, SW=восток.
#[allow(clippy::too_many_arguments)]
fn blit_wall(
    canvas: &mut RgbaImage,
    rsi: &Rsi,
    walls: &impl Fn(i32, i32) -> bool,
    tx: i32,
    ty: i32,
    px: u32,
    py: u32,
) {
    const CCW: u8 = 1;
    const DIAG: u8 = 2;
    const CW: u8 = 4;

    let n = walls(tx, ty + 1);
    let ne = walls(tx + 1, ty + 1);
    let e = walls(tx + 1, ty);
    let se = walls(tx + 1, ty - 1);
    let s = walls(tx, ty - 1);
    let sw = walls(tx - 1, ty - 1);
    let w = walls(tx - 1, ty);
    let nw = walls(tx - 1, ty + 1);

    let corner_ne = (n as u8 * CCW) | (ne as u8 * DIAG) | (e as u8 * CW);
    let corner_se = (e as u8 * CCW) | (se as u8 * DIAG) | (s as u8 * CW);
    let corner_sw = (s as u8 * CCW) | (sw as u8 * DIAG) | (w as u8 * CW);
    let corner_nw = (w as u8 * CCW) | (nw as u8 * DIAG) | (n as u8 * CW);

    // Порядок и направления слоёв — как в движке (SetCornerLayers).
    for (corner, direction) in [
        (corner_se, 0u32),
        (corner_ne, 3),
        (corner_nw, 2),
        (corner_sw, 1),
    ] {
        let state_name = format!("solid{corner}");
        let Some(state) = rsi.state(&state_name) else {
            continue;
        };
        let cell = state.cell_index(direction, 0);
        let sheet = state.sheet.to_rgba8();
        blit_rsi_cell(canvas, &sheet, cell, px, py);
    }
}

/// Собирает картинку чанка 1024×1024 px из реплицированных тайлов (T2.3):
/// соседи берутся из всех доступных чанков (стыки стен между чанками).
pub fn compose_chunk(
    data: &TileChunkData,
    visuals: &TileVisuals,
    map: &HashMap<(i32, i32), &TileChunkData>,
) -> RgbaImage {
    let side = CHUNK_TILES * TILE_PX;
    let mut canvas = RgbaImage::new(side, side);
    let walls = |wx: i32, wy: i32| tile_at(map, wx, wy) == TileType::Wall;

    for ly in 0..CHUNK_TILES {
        for lx in 0..CHUNK_TILES {
            let tile = data.get_local(lx, ly);
            let tx = data.coords.0 * CHUNK_TILES as i32 + lx as i32;
            let ty = data.coords.1 * CHUNK_TILES as i32 + ly as i32;
            // Ряд ly=0 — низ чанка в мировых координатах, а пиксельный ряд 0 — верх.
            let px = lx * TILE_PX;
            let py = (CHUNK_TILES - 1 - ly) * TILE_PX;

            match tile {
                TileType::Space => {}
                TileType::Wall => {
                    if let Some(rsi) = &visuals.wall_rsi {
                        blit_wall(&mut canvas, rsi, &walls, tx, ty, px, py);
                    } else if let Some(sprite) = visuals.sprites.get(tile.key()) {
                        blit_tile(&mut canvas, sprite, 0, px, py);
                    }
                }
                TileType::Floor => {
                    let proto: &TileProto = visuals.protos.get(tile);
                    if proto.sprite.is_none() {
                        continue;
                    }
                    let Some(sprite) = visuals.sprites.get(tile.key()) else {
                        continue;
                    };
                    let variants = (sprite.width() / TILE_PX).max(1);
                    let variant = variant_index(tx, ty, variants);
                    blit_tile(&mut canvas, sprite, variant * TILE_PX, px, py);
                }
            }
        }
    }
    canvas
}

/// Состояние пересборки: набор чанков стабилен не раньше, чем через кадр
/// после последнего изменения (соседи успевают прийти), плюс флаг «грязно».
#[derive(Resource, Default)]
pub struct ChunkRenderState {
    seen: BTreeSet<(i32, i32)>,
    dirty: bool,
}

/// Рендерит чанки карты (T2.3): собирает картинки, когда набор чанков
/// (и, значит, стыки стен) стабилен; старые визуалы заменяет.
pub fn render_map_chunks(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    visuals: Option<Res<TileVisuals>>,
    chunks: Query<(Entity, &TileChunkData)>,
    visuals_q: Query<(Entity, &TileChunkVisual)>,
    mut state: ResMut<ChunkRenderState>,
) {
    let Some(visuals) = visuals else {
        return;
    };
    let map: HashMap<(i32, i32), &TileChunkData> =
        chunks.iter().map(|(_, data)| (data.coords, data)).collect();
    let coords: BTreeSet<(i32, i32)> = map.keys().copied().collect();

    if coords != state.seen {
        // Набор чанков изменился: ждём следующий кадр (возможно, придут соседи).
        state.seen = coords;
        state.dirty = true;
        return;
    }
    if !state.dirty {
        return;
    }
    state.dirty = false;

    // Пересобираем все видимые чанки: при смене интереса стыки стен соседей
    // могли измениться, поэтому меняем картинки целиком.
    for (visual_entity, _) in visuals_q.iter() {
        commands.entity(visual_entity).despawn();
    }
    let chunk_px = (CHUNK_TILES * TILE_PX) as f32;
    for (chunk_entity, data) in chunks.iter() {
        let canvas = compose_chunk(data, &visuals, &map);
        let image = Image::new(
            Extent3d {
                width: canvas.width(),
                height: canvas.height(),
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            canvas.into_raw(),
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        let handle = images.add(image);
        let center = Vec3::new(
            (data.coords.0 as f32 + 0.5) * chunk_px,
            (data.coords.1 as f32 + 0.5) * chunk_px,
            0.0,
        );
        commands.spawn((
            TileChunkVisual {
                chunk: chunk_entity,
            },
            Sprite::from_image(handle),
            Transform::from_translation(center),
        ));
        tracing::debug!(coords = ?data.coords, "map chunk rendered");
    }
}

/// Визуальный чанк карты, привязанный к реплицированной сущности чанка.
#[derive(Component)]
pub struct TileChunkVisual {
    chunk: Entity,
}

/// Убирает спрайты чанков, чьи сущности ушли из интереса (иначе — мигание дублей).
pub fn despawn_orphan_chunks(
    mut commands: Commands,
    visuals: Query<(Entity, &TileChunkVisual)>,
    chunks: Query<()>,
) {
    for (visual_entity, visual) in visuals.iter() {
        if chunks.get(visual.chunk).is_err() {
            commands.entity(visual_entity).despawn();
        }
    }
}
