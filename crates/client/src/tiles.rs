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
use ssr_core::tiles::{CHUNK_TILES, TILE_PX, TileChunkData, TileProto, TilePrototypes};

/// Прототипы и декодированные спрайты тайлов (грузятся один раз при старте).
#[derive(Resource)]
pub struct TileVisuals {
    protos: TilePrototypes,
    sprites: HashMap<String, RgbaImage>,
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
    TileVisuals { protos, sprites }
}

/// Выбирает вариант спрайта тайла детерминированно по координатам
/// (чек-лист SS14_IMPORT.md: варианты тайлов рандомизируются при рендере).
fn variant_index(tx: i32, ty: i32, variants: u32) -> u32 {
    ((tx as i64 * 7 + ty as i64 * 13).rem_euclid(variants as i64)) as u32
}

/// Собирает картинку чанка 1024×1024 px из реплицированных тайлов (T2.3).
fn compose_chunk(data: &TileChunkData, visuals: &TileVisuals) -> RgbaImage {
    let side = CHUNK_TILES * TILE_PX;
    let mut canvas = RgbaImage::new(side, side);
    for ly in 0..CHUNK_TILES {
        for lx in 0..CHUNK_TILES {
            let tile = data.get_local(lx, ly);
            let proto: &TileProto = visuals.protos.get(tile);
            let Some(sprite) = proto
                .sprite
                .as_ref()
                .and_then(|_| visuals.sprites.get(tile.key()))
            else {
                continue;
            };
            let variants = (sprite.width() / TILE_PX).max(1);
            let variant = variant_index(
                data.coords.0 * CHUNK_TILES as i32 + lx as i32,
                data.coords.1 * CHUNK_TILES as i32 + ly as i32,
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

/// Рендерит реплицированные чанки карты (T2.3): на каждый `Added<TileChunkData>`
/// собирается картинка чанка и спавнится спрайт.
pub fn render_map_chunks(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    visuals: Option<Res<TileVisuals>>,
    new_chunks: Query<(Entity, &TileChunkData), Added<TileChunkData>>,
) {
    let Some(visuals) = visuals else {
        return;
    };
    let chunk_px = (CHUNK_TILES * TILE_PX) as f32;
    for (_, data) in new_chunks.iter() {
        let canvas = compose_chunk(data, &visuals);
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
            TileChunkVisual,
            Sprite::from_image(handle),
            Transform::from_translation(center),
        ));
        tracing::debug!(coords = ?data.coords, "map chunk rendered");
    }
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
