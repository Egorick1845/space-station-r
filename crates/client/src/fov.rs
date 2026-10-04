//! Обзор: стены блокируют зрение (запрос владельца).
//!
//! Строим маску тумана по тайлам вокруг игрока: клетка видна, если отрезок
//! от центра тайла игрока до центра клетки не пересекает стену (обход по
//! сетке, как DDA). Маска — одна картинка на всё окно обзора, спрайт висит
//! между миром и игроком (z = [`FOG_Z`]): свой игрок и UI видны, чужие —
//! скрыты за стенами.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ssr_core::PlayerPosition;
use ssr_core::tiles::{CHUNK_TILES, TILE_PX, TileChunkData, TileType};

use crate::inventory_ui::OwnPlayerEntity;

/// Полуразмер окна обзора в тайлах.
const FOV_RADIUS_TILES: i32 = 48;
/// Как часто пересчитываем маску (сек).
const FOV_PERIOD: f32 = 0.2;
/// Слой тумана: выше чужих игроков (0.9) и ниже своего (1.0).
const FOG_Z: f32 = 0.95;
/// Юнитов на тайл (сетка мира).
const TILE_UNITS: f32 = TILE_PX as f32;

/// Текущая маска тумана и её положение в мире.
#[derive(Resource)]
pub struct FogOfWar {
    image: Handle<Image>,
    sprite: Entity,
    /// Левый нижний тайл окна, под которое построена маска.
    origin: (i32, i32),
    ready: bool,
}

/// Создаёт картинку тумана и спрайт при старте.
pub fn setup_fog(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let side = (FOV_RADIUS_TILES * 2 + 1) as u32;
    let mut image = Image::new(
        Extent3d {
            width: side,
            height: side,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![255; (side * side * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // Тайловая сетка должна оставаться чёткой: только ближайший сосед.
    image.sampler = bevy::image::ImageSampler::nearest();
    let handle = images.add(image);
    // Тексель маски = тайл: спрайт растягивается на всё окно обзора.
    let sprite = commands
        .spawn((
            Sprite::from_image(handle.clone()),
            Transform::from_xyz(0.0, 0.0, FOG_Z).with_scale(Vec3::splat(TILE_UNITS)),
        ))
        .id();
    commands.insert_resource(FogOfWar {
        image: handle,
        sprite,
        origin: (0, 0),
        ready: false,
    });
}

/// Тайл по мировым координатам тайла из реплицированных чанков.
fn tile_at(chunks: &Query<&TileChunkData>, tx: i32, ty: i32) -> TileType {
    let size = CHUNK_TILES as i32;
    let coords = (tx.div_euclid(size), ty.div_euclid(size));
    for chunk in chunks.iter() {
        if chunk.coords == coords {
            return chunk.get_local((tx - coords.0 * size) as u32, (ty - coords.1 * size) as u32);
        }
    }
    // Чанк неизвестен (вне интереса) — считаем стеной: не подсматриваем.
    TileType::Wall
}

/// Виден ли тайл (tx, ty) из тайла игрока: обход по сетке, стены блокируют.
fn visible(chunks: &Query<&TileChunkData>, from: (i32, i32), to: (i32, i32)) -> bool {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let steps = dx.abs().max(dy.abs());
    if steps <= 1 {
        return true;
    }
    for step in 1..steps {
        let t = step as f32 / steps as f32;
        let tx = from.0 as f32 + dx as f32 * t;
        let ty = from.1 as f32 + dy as f32 * t;
        if tile_at(chunks, tx.round() as i32, ty.round() as i32) == TileType::Wall {
            return false;
        }
    }
    true
}

/// Пересчитывает маску тумана при смене тайла игрока (не чаще FOV_PERIOD).
#[allow(clippy::too_many_arguments)]
pub fn update_fog(
    time: Res<Time>,
    mut next_update: Local<f32>,
    own: Res<OwnPlayerEntity>,
    positions: Query<&PlayerPosition>,
    chunks: Query<&TileChunkData>,
    fog: Option<ResMut<FogOfWar>>,
    mut images: ResMut<Assets<Image>>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(mut fog) = fog else {
        return;
    };
    // Первый пересчёт — сразу при появлении игрока, дальше по таймеру.
    *next_update += time.delta_secs();
    if fog.ready && *next_update < FOV_PERIOD {
        return;
    }
    let Some(own_position) = own.0.and_then(|entity| positions.get(entity).ok()) else {
        return;
    };
    let player_tile = (
        (own_position.0[0] / TILE_UNITS).floor() as i32,
        (own_position.0[1] / TILE_UNITS).floor() as i32,
    );
    *next_update = 0.0;

    let side = (FOV_RADIUS_TILES * 2 + 1) as u32;
    let origin = (
        player_tile.0 - FOV_RADIUS_TILES,
        player_tile.1 - FOV_RADIUS_TILES,
    );
    fog.origin = origin;
    fog.ready = true;

    let Some(mut image) = images.get_mut(&fog.image) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };
    for ly in 0..side {
        for lx in 0..side {
            let tx = origin.0 + lx as i32;
            let ty = origin.1 + ly as i32;
            let open = tile_at(&chunks, tx, ty) != TileType::Wall
                && visible(&chunks, player_tile, (tx, ty));
            // Ряд 0 картинки — верх окна, а тайлы растут вверх: переворачиваем.
            let row = side - 1 - ly;
            let index = ((row * side + lx) * 4) as usize;
            let alpha: u8 = if open { 0 } else { 255 };
            data[index] = 0;
            data[index + 1] = 0;
            data[index + 2] = 0;
            data[index + 3] = alpha;
        }
    }

    if let Ok(mut transform) = transforms.get_mut(fog.sprite) {
        transform.translation.x = (origin.0 as f32 + side as f32 / 2.0) * TILE_UNITS;
        transform.translation.y = (origin.1 as f32 + side as f32 / 2.0) * TILE_UNITS;
    }
}
