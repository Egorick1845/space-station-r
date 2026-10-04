//! Освещение по модели SS14 (изучено по `Clyde.LightRendering.cs` и
//! `light_shared.swsl`): карта света строится из постоянного света карты
//! (`MapLightComponent.ambientLightColor`, у станции `#151515` ≈ 8%) и точек
//! света (`PointLight`: `radius`, `energy`, `color`), стены перекрывают свет,
//! карта размывается (`light.blur`). Итог рисуется тёмным оверлеем поверх мира.
//!
//! Отличия от движка (осознанные, документируются): карта считается на CPU по
//! тайлам, а не в полярной shadow map с VSM/PCF на GPU; мягкость даёт размытие
//! всей карты, а не PCF-выборка по `softness`; двери свет пока не перекрывают
//! (у нас они сущности, а не тайлы).

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ssr_core::PlayerPosition;
use ssr_core::inventory::ItemPosition;
use ssr_core::power::{Light, Powered};
use ssr_core::tiles::{CHUNK_TILES, TILE_PX, TileChunkData, TileType};

use crate::inventory_ui::OwnPlayerEntity;

/// Постоянный свет станции: `MapLight.ambientLightColor: '#151515FF'`
/// (cluster.yml) — без ламп станция почти чёрная, но силуэты видны.
const AMBIENT: f32 = 0x15 as f32 / 255.0;
/// Затухание из `SharedPointLightComponent.Falloff`.
const FALLOFF: f32 = 6.8;
/// Высота источника над полом (`LIGHTING_HEIGHT = 1.0` в шейдере).
const LIGHTING_HEIGHT: f32 = 1.0;
/// Полуразмер окна карты света в тайлах.
const LIGHT_RADIUS_TILES: i32 = 40;
/// Как часто пересобираем карту (сек): в движке — каждый кадр в рендерере.
const LIGHT_PERIOD: f32 = 0.1;
/// Слой тёмного оверлея: выше тумана (0.85), ниже тел (0.9).
const LIGHT_Z: f32 = 0.86;
/// Юнитов на тайл.
const TILE_UNITS: f32 = TILE_PX as f32;

/// Карта света: картинка по тайлам и её спрайт.
#[derive(Resource)]
pub struct LightMap {
    image: Handle<Image>,
    sprite: Entity,
    ready: bool,
}

/// Создаёт картинку и спрайт карты света.
pub fn setup_lighting(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let side = ((LIGHT_RADIUS_TILES * 2 + 1) as u32).max(1);
    let image = Image::new(
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
    let handle = images.add(image);
    // Тексель = тайл, но карта размывается — берём линейную фильтрацию.
    let sprite = commands
        .spawn((
            Sprite::from_image(handle.clone()),
            Transform::from_xyz(0.0, 0.0, LIGHT_Z).with_scale(Vec3::splat(TILE_UNITS)),
        ))
        .id();
    commands.insert_resource(LightMap {
        image: handle,
        sprite,
        ready: false,
    });
}

/// Тайл по мировым координатам из реплицированных чанков (стена = блок света).
fn tile_at(chunks: &Query<&TileChunkData>, tx: i32, ty: i32) -> TileType {
    let size = CHUNK_TILES as i32;
    let coords = (tx.div_euclid(size), ty.div_euclid(size));
    for chunk in chunks.iter() {
        if chunk.coords == coords {
            return chunk.get_local((tx - coords.0 * size) as u32, (ty - coords.1 * size) as u32);
        }
    }
    TileType::Wall
}

/// Виден ли тайл из точки (обход по сетке, стены перекрывают — как окклюдеры).
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

/// Затухание света из `light_shared.swsl`:
/// `s = clamp(sqrt(dist² + 1)/radius)`, `val = (1-s²)² / (1 + falloff*s)`,
/// затем `val *= energy`.
fn attenuation(dist_tiles: f32, radius: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    let squared = dist_tiles * dist_tiles + LIGHTING_HEIGHT;
    let s = (squared.sqrt() / radius).clamp(0.0, 1.0);
    let s2 = s * s;
    ((1.0 - s2) * (1.0 - s2) / (1.0 + FALLOFF * s)).clamp(0.0, 1.0)
}

/// Пересчитывает карту света: ambient + лампы, стены перекрывают, размытие.
#[allow(clippy::too_many_arguments)]
pub fn update_lighting(
    time: Res<Time>,
    mut next_update: Local<f32>,
    own: Res<OwnPlayerEntity>,
    positions: Query<&PlayerPosition>,
    chunks: Query<&TileChunkData>,
    lamps: Query<(&Light, &ItemPosition, Option<&Powered>)>,
    doors: Query<&ssr_core::Door>,
    light_map: Option<Res<LightMap>>,
    mut images: ResMut<Assets<Image>>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(map) = light_map else {
        return;
    };
    *next_update += time.delta_secs();
    if map.ready && *next_update < LIGHT_PERIOD {
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

    let side = ((LIGHT_RADIUS_TILES * 2 + 1) as u32).max(1);
    let origin = (
        player_tile.0 - LIGHT_RADIUS_TILES,
        player_tile.1 - LIGHT_RADIUS_TILES,
    );

    // 1) локальная карта тайлов: стены блокируют свет. Закрытая дверь — тоже
    // окклюдер (в SS14 `Door.Occludes` + `Occluder`, при открытии выключается).
    let mut grid = vec![TileType::Wall; (side * side) as usize];
    for ly in 0..side {
        for lx in 0..side {
            grid[(ly * side + lx) as usize] =
                tile_at(&chunks, origin.0 + lx as i32, origin.1 + ly as i32);
        }
    }
    for door in doors.iter() {
        if door.open {
            continue;
        }
        let tx = (door.position[0] / TILE_UNITS).floor() as i32 - origin.0;
        let ty = (door.position[1] / TILE_UNITS).floor() as i32 - origin.1;
        if tx >= 0 && ty >= 0 && tx < side as i32 && ty < side as i32 {
            grid[(ty as u32 * side + tx as u32) as usize] = TileType::Wall;
        }
    }

    // 2) источники света в окне: лампы реплицируются с позицией и питанием.
    let mut lights: Vec<(i32, i32, f32, f32)> = Vec::new();
    for (light, position, powered) in lamps.iter() {
        // Без питания лампа не светит (в SS14 гасит `PoweredLightSystem`).
        if powered.is_some_and(|powered| !powered.0) {
            continue;
        }
        let tx = (position.0[0] / TILE_UNITS).floor() as i32;
        let ty = (position.0[1] / TILE_UNITS).floor() as i32;
        if (tx - origin.0).abs() > LIGHT_RADIUS_TILES + light.radius as i32
            || (ty - origin.1).abs() > LIGHT_RADIUS_TILES + light.radius as i32
        {
            continue;
        }
        lights.push((tx, ty, light.radius, light.energy));
    }

    // 3) свет по тайлам: ambient плюс вклад ламп с учётом перекрытия стенами.
    let mut values = vec![AMBIENT; (side * side) as usize];
    for ly in 0..side {
        for lx in 0..side {
            let center = (side as i32 / 2, side as i32 / 2);
            let index = (ly * side + lx) as usize;
            if grid[index] == TileType::Wall {
                // Стены почти не освещаются (в SS14 их подсвечивает wall-bleed).
                values[index] = AMBIENT * 0.6;
                continue;
            }
            for (tx, ty, radius, energy) in &lights {
                // Позиция лампы в локальной сетке.
                let lx_local = tx - origin.0;
                let ly_local = ty - origin.1;
                if lx_local < 0
                    || ly_local < 0
                    || lx_local >= side as i32
                    || ly_local >= side as i32
                {
                    continue;
                }
                if !visible_in(&grid, side, (lx_local, ly_local), (lx as i32, ly as i32)) {
                    continue;
                }
                let dx = lx as f32 - lx_local as f32;
                let dy = ly as f32 - ly_local as f32;
                let distance = (dx * dx + dy * dy).sqrt();
                if distance > *radius {
                    continue;
                }
                values[index] += attenuation(distance, *radius) * energy;
            }
            let _ = center;
            values[index] = values[index].min(1.0);
        }
    }

    // 4) размытие 3×3 — как `light.blur` в движке (мягкие края).
    let mut blurred = values.clone();
    for ly in 1..side - 1 {
        for lx in 1..side - 1 {
            let mut sum = 0.0f32;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let x = (lx as i32 + dx) as u32;
                    let y = (ly as i32 + dy) as u32;
                    sum += values[(y * side + x) as usize];
                }
            }
            blurred[(ly * side + lx) as usize] = sum / 9.0;
        }
    }

    // 5) пишем тёмный оверлей: alpha = 1 - свет.
    let Some(mut image) = images.get_mut(&map.image) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };
    for ly in 0..side {
        for lx in 0..side {
            let light_value = blurred[(ly * side + lx) as usize];
            let row = side - 1 - ly;
            let index = ((row * side + lx) * 4) as usize;
            data[index] = 0;
            data[index + 1] = 0;
            data[index + 2] = 0;
            data[index + 3] = ((1.0 - light_value) * 255.0) as u8;
        }
    }
    if let Ok(mut transform) = transforms.get_mut(map.sprite) {
        transform.translation.x = (origin.0 as f32 + side as f32 / 2.0) * TILE_UNITS;
        transform.translation.y = (origin.1 as f32 + side as f32 / 2.0) * TILE_UNITS;
    }
    tracing::debug!(lamps = lights.len(), ambient = AMBIENT, "light map rebuilt");
}
