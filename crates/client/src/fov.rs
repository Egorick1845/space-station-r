//! Обзор: стены блокируют зрение (запрос владельца).
//!
//! Строим маску тумана вокруг игрока: тайл виден, если отрезок от центра тайла
//! игрока до центра клетки не пересекает стену (обход по сетке, как DDA). Сами
//! стены в поле зрения тоже видны — иначе «за тенью» не разглядеть планировку.
//! Маска пишется с подвыборкой (несколько текселей на тайл) и сглаживается
//! билинейно, у границы радиуса обзора наплывает мягким кругом — теней-квадратов
//! и рваных краёв не видно; спрайт висит между миром и игроком (z = [`FOG_Z`]).

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ssr_core::PlayerPosition;
use ssr_core::tiles::{CHUNK_TILES, TILE_PX, TileChunkData, TileType};

use crate::inventory_ui::OwnPlayerEntity;

/// Полуразмер окна обзора в тайлах.
const FOV_RADIUS_TILES: i32 = 48;
/// Текселей на тайл: 1 — тени жёсткие, по границам тайлов (как отбрасывают стены).
const FOG_SUB: u32 = 1;
/// Как часто пересчитываем маску (сек).
const FOV_PERIOD: f32 = 0.2;
/// Слой тумана: в SS14 карта света УМНОЖАЕТСЯ на все спрайты (шейдер), поэтому
/// у нас затемнение кладётся ВЫШЕ всех мировых спрайтов (двери 1.5, тела до 1.22,
/// предметы в руках) и ниже интерфейса — иначе двери, предметы и подсветка
/// просвечивают из тёмных зон.
const FOG_Z: f32 = 2.0;
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
    let side = ((FOV_RADIUS_TILES * 2 + 1) as u32) * FOG_SUB;
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
    // Тени жёсткие: только ближайший тексель, никакого размытия и подсветки
    // сквозь стены (владелец: «мягкие тени не нужны»).
    image.sampler = bevy::image::ImageSampler::nearest();
    let handle = images.add(image);
    // Тексель = 1/FOG_SUB тайла: спрайт растягивается на всё окно обзора.
    let sprite = commands
        .spawn((
            Sprite::from_image(handle.clone()),
            Transform::from_xyz(0.0, 0.0, FOG_Z)
                .with_scale(Vec3::splat(TILE_UNITS / FOG_SUB as f32)),
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

/// Виден ли тайл `to` (в координатах локальной сетки) из тайла `from`:
/// обход по сетке, стены между концами блокируют зрение.
fn visible_in(grid: &[TileType], side: u32, from: (i32, i32), to: (i32, i32)) -> bool {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let steps = dx.abs().max(dy.abs());
    if steps <= 1 {
        return true;
    }
    for step in 1..steps {
        let t = step as f32 / steps as f32;
        let tx = (from.0 as f32 + dx as f32 * t).round() as i32;
        let ty = (from.1 as f32 + dy as f32 * t).round() as i32;
        if tx < 0 || ty < 0 || tx >= side as i32 || ty >= side as i32 {
            return false;
        }
        if grid[(ty as u32 * side + tx as u32) as usize] == TileType::Wall {
            return false;
        }
    }
    true
}

/// Пересчитывает маску тумана по таймеру (и сразу при появлении игрока).
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

    // Локальная карта тайлов окна: один обход чанков вместо тысяч выборок.
    let mut grid = vec![TileType::Wall; (side * side) as usize];
    for ly in 0..side {
        for lx in 0..side {
            let tx = origin.0 + lx as i32;
            let ty = origin.1 + ly as i32;
            grid[(ly * side + lx) as usize] = tile_at(&chunks, tx, ty);
        }
    }
    // Видимость по тайлам: центр окна — тайл игрока. Стены в поле зрения видны
    // (иначе «за тенью» не видно планировку), скрывает только стена между.
    let center = FOV_RADIUS_TILES;
    let mut values = vec![0.0f32; (side * side) as usize];
    for ly in 0..side {
        for lx in 0..side {
            let open = visible_in(&grid, side, (center, center), (lx as i32, ly as i32));
            values[(ly * side + lx) as usize] = if open { 1.0 } else { 0.0 };
        }
    }

    let Some(mut image) = images.get_mut(&fog.image) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };
    // Тексель = тайл: тень ложится ровно по клеткам, края прямые и жёсткие.
    for ly in 0..side {
        for lx in 0..side {
            let visible = values[(ly * side + lx) as usize] > 0.5;
            // Ряд 0 картинки — верх окна, а тайлы растут вверх: переворачиваем.
            let row = side - 1 - ly;
            let index = ((row * side + lx) * 4) as usize;
            data[index] = 0;
            data[index + 1] = 0;
            data[index + 2] = 0;
            data[index + 3] = if visible { 0 } else { 255 };
        }
    }

    if let Ok(mut transform) = transforms.get_mut(fog.sprite) {
        transform.translation.x = (origin.0 as f32 + side as f32 / 2.0) * TILE_UNITS;
        transform.translation.y = (origin.1 as f32 + side as f32 / 2.0) * TILE_UNITS;
    }
}
