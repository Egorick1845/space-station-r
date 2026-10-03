//! Рендер тайловой карты (PLAN.md T2.1).
//!
//! Каждый чанк 32×32 тайлов собирается в одну картинку 1024×1024 px и
//! рисуется одним спрайтом: 16 чанков тестовой карты = 16 сущностей.

use std::collections::HashMap;
use std::path::Path;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use image::RgbaImage;
use ssr_core::tiles::{CHUNK_TILES, TILE_PX, TileChunk, TileProto, TilePrototypes};

/// Загружает прототипы тайлов и декодирует их спрайты (полосы вариантов).
fn load_visuals(root: &Path) -> HashMap<String, RgbaImage> {
    let protos = TilePrototypes::load(&root.join("prototypes/tiles.ron"))
        .expect("parse assets/prototypes/tiles.ron");
    let mut visuals = HashMap::new();
    for (key, proto) in &protos.tiles {
        let Some(rel) = &proto.sprite else { continue };
        let bytes = std::fs::read(root.join(rel.trim_start_matches('/')))
            .unwrap_or_else(|e| panic!("read tile sprite {rel}: {e}"));
        let image = image::load_from_memory(&bytes)
            .unwrap_or_else(|e| panic!("decode tile sprite {rel}: {e}"))
            .to_rgba8();
        visuals.insert(key.clone(), image);
    }
    visuals
}

/// Выбирает вариант спрайта тайла детерминированно по координатам
/// (чек-лист SS14_IMPORT.md: варианты тайлов рандомизируются при рендере).
fn variant_index(tx: i32, ty: i32, variants: u32) -> u32 {
    ((tx as i64 * 7 + ty as i64 * 13).rem_euclid(variants as i64)) as u32
}

/// Собирает картинку чанка 1024×1024 px из спрайтов тайлов.
fn compose_chunk(
    chunk: &TileChunk,
    visuals: &HashMap<String, RgbaImage>,
    protos: &TilePrototypes,
) -> RgbaImage {
    let side = CHUNK_TILES * TILE_PX;
    let mut canvas = RgbaImage::new(side, side);
    for ly in 0..CHUNK_TILES {
        for lx in 0..CHUNK_TILES {
            let tile = chunk.get_local(lx, ly);
            let proto: &TileProto = protos.get(tile);
            let Some(_rel) = &proto.sprite else { continue };
            let sprite = &visuals[tile.key()];
            let variants = (sprite.width() / TILE_PX).max(1);
            let variant = variant_index(
                chunk.coords.x * CHUNK_TILES as i32 + lx as i32,
                chunk.coords.y * CHUNK_TILES as i32 + ly as i32,
                variants,
            );
            let view = image::imageops::crop_imm(sprite, variant * TILE_PX, 0, TILE_PX, TILE_PX);
            let mut tile_img = RgbaImage::new(TILE_PX, TILE_PX);
            image::imageops::replace(&mut tile_img, &view.to_image(), 0, 0);
            image::imageops::overlay(
                &mut canvas,
                &tile_img,
                (lx * TILE_PX) as i64,
                (ly * TILE_PX) as i64,
            );
        }
    }
    canvas
}

/// Генерирует тестовую карту, собирает чанки и спавнит их спрайты.
/// Возвращает число заспавненных чанков (для проверки в тесте).
pub fn spawn_map(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    root: &Path,
    origin: Vec3,
) -> usize {
    let protos = TilePrototypes::load(&root.join("prototypes/tiles.ron"))
        .expect("parse assets/prototypes/tiles.ron");
    let visuals = load_visuals(root);
    let chunks = ssr_core::tiles::gen_test_map();

    let mut spawned = 0;
    for chunk in &chunks {
        let canvas = compose_chunk(chunk, &visuals, &protos);
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
        // Центр чанка в мировых координатах (тайл (0,0) карты — угол чанка (0,0)).
        let chunk_px = (CHUNK_TILES * TILE_PX) as f32;
        let center = origin
            + Vec3::new(
                (chunk.coords.x as f32 + 0.5) * chunk_px,
                (chunk.coords.y as f32 + 0.5) * chunk_px,
                0.0,
            );
        commands.spawn((
            TileChunkVisual,
            Sprite::from_image(handle),
            Transform::from_translation(center),
        ));
        spawned += 1;
    }
    spawned
}

/// Маркер визуального чанка карты.
#[derive(Component)]
pub struct TileChunkVisual;

/// Камера двигается от курсора у краёв окна (критерий T2.1: скролл по краям).
pub fn camera_edge_scroll(
    windows: Query<&Window>,
    mut camera: Single<&mut Transform, With<Camera2d>>,
    time: Res<Time>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    const MARGIN: f32 = 32.0;
    const SPEED: f32 = 1200.0;

    let mut direction = Vec2::ZERO;
    if cursor.x < MARGIN {
        direction.x -= 1.0;
    }
    if cursor.x > window.width() - MARGIN {
        direction.x += 1.0;
    }
    if cursor.y < MARGIN {
        direction.y += 1.0;
    }
    if cursor.y > window.height() - MARGIN {
        direction.y -= 1.0;
    }
    if direction != Vec2::ZERO {
        camera.translation += (direction * SPEED * time.delta_secs()).extend(0.0);
    }
}
