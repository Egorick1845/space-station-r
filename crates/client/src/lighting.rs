//! Освещение и туман войны по модели SS14 — ЕДИНЫМ слоем, как в движке.
//!
//! В SS14 итоговый пиксель спрайта = `COLOR * LIGHT`, где карта света LIGHT =
//! ambient карты (`MapLight.ambientLightColor`, у станции `#151515` ≈ 0.082) плюс
//! АДДИТИВНЫЕ точки света с затуханием `(1-s²)²/(1+6.8·s)·energy`, умноженная на
//! окклюзию тумана войны (fov-lighting.swsl: `light *= occlusion`). Стены у ламп
//! подсвечиваются «просачиванием» (wall bleed), UI свет не трогает.
//!
//! У нас вместо умножения каждого спрайта — один оверлей тьмы поверх мира
//! (ниже интерфейса): `alpha = 1 − свет × видимость`. Математически это то же
//! затемнение `world × light`, и оно же вбирает туман войны — отдельного слоя
//! тумана больше нет (раньше было ДВА чёрных слоя с двойной работой).
//!
//! Расчёт: окно карты фиксировано 33×33 тайла (вьюпорт 21×15 + поля), BFS от
//! ламп по нестенным клеткам (без «спиц» от лучей, работает у настенных ламп),
//! BFS от игрока для видимости, размытие 3×3 по полу и одностороннее
//! «просачивание» света в стены. Пересчёт — только при изменениях (тайл игрока,
//! лампы, двери, чанки), поэтому карта больше не считается каждый кадр.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ssr_core::PlayerPosition;
use ssr_core::inventory::ItemPosition;
use ssr_core::power::{Light, Powered};
use ssr_core::tiles::{CHUNK_TILES, TILE_PX, TileChunkData, TileType};

use crate::inventory_ui::OwnPlayerEntity;

/// Постоянный свет станции (`MapLight.ambientLightColor: '#151515FF'` → 0.082
/// в линейном пространстве).
const AMBIENT: f32 = 0.082;
/// Затухание из `SharedPointLightComponent.Falloff`.
const FALLOFF: f32 = 6.8;
/// Полуразмер окна карты в тайлах: вьюпорт 21×15, диагональ ≈ 13, плюс поля.
const WINDOW_RADIUS: i32 = 16;
/// Сторона окна (квадрат, нечётная — центр совпадает с тайлом игрока).
const WINDOW_SIDE: u32 = (WINDOW_RADIUS * 2 + 1) as u32;
/// Текселей карты света на тайл: свет и тени считаются по полтайла, поэтому
/// тень от стены — прямая линия, а не квадрат тайла (в движке карта света
/// тоже вдвое мельче экрана, `light.resolution_scale = 0.5`).
const LIGHT_SUBTEXELS: u32 = 2;
/// Сторона карты света в текселях.
const LIGHT_DIM: u32 = WINDOW_SIDE * LIGHT_SUBTEXELS;
/// Ширина полярной карты теней — `ShadowMapSize` в движке (512 px по кругу).
const SHADOW_BINS: usize = 512;
const PI: f32 = std::f32::consts::PI;
/// Как часто карта может пересчитываться при изменениях (сек).
const REBUILD_PERIOD: f32 = 0.15;
/// Слой тьмы — выше ВСЕХ мировых спрайтов (двери 1.5, тела до 1.23), ниже UI:
/// в SS14 свет умножается на спрайты, а у нас оверлей обязан их накрыть.
const DARK_Z: f32 = 2.01;
/// Слой тёплого оттенка ламп — ПОД тьмой: она (с туманом) сама маскирует
/// свечение за стенами и в невидимой зоне.
const GLOW_Z: f32 = 2.00;
/// Сила тёплого оттенка на освещённых клетках.
const GLOW_ALPHA: f32 = 0.55;
/// Юнитов на тайл.
const TILE_UNITS: f32 = TILE_PX as f32;
/// Кадр мира в тайлах — как `ViewportUIController` в движке (672×480 px).
const VIEWPORT_TILES: (f32, f32) = (21.0, 15.0);

/// Карта света: слой тьмы и слой тёплого оттенка.
#[derive(Resource)]
pub struct LightMap {
    image: Handle<Image>,
    sprite: Entity,
    glow_image: Handle<Image>,
    glow_sprite: Entity,
    /// Пересчёт уже был — дальше только при изменениях.
    ready: bool,
}

/// Отпечаток мира: при изменении карта пересобирается.
#[derive(Default, PartialEq)]
pub(crate) struct WorldState {
    player_tile: (i32, i32),
    lamps: Vec<(i32, i32, u8, u8)>, // тайл, радиус, яркость×100
    closed_doors: Vec<(i32, i32)>,
    chunks: u64, // хеш набора чанков (репликация карты могла подгрузить новый)
}

/// Источник света для GPU-конвейера (SS14_LIGHTING.md §7): мировые единицы,
/// параметры — как у `PointLightComponent` (`Falloff` 6.8, `CurveFactor` 0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GpuLight {
    pub position: (f32, f32),
    /// Мировой радиус: `PointLight.radius` в тайлах × `TILE_PX`.
    pub radius: f32,
    pub energy: f32,
    pub falloff: f32,
    pub curve: f32,
    /// Линейный цвет: движок берёт hex как линейный множитель (без sRGB→linear).
    pub color: [f32; 3],
}

/// Сцена света для GPU-конвейера: окклюдеры из тайлов, источники и камера.
/// Заполняется при каждом пересчёте карты света — та же подписка на изменения
/// мира, поэтому CPU- и GPU-пути не расходятся.
#[derive(Resource, Default)]
pub struct LightScene {
    pub walls: Vec<ssr_core::occluders::OccluderSegment>,
    pub lights: Vec<GpuLight>,
    /// Левый-нижний угол видимой области мира (кадр 21×15 тайлов, как у камеры).
    pub camera: (f32, f32),
    /// Размер видимой области в мировых единицах (672×480).
    pub viewport: (f32, f32),
    /// Позиция глаза (свой игрок) — центр карты FOV.
    pub eye: (f32, f32),
}

/// Создаёт картинки карты света и спрайты.
pub fn setup_lighting(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let size = (LIGHT_DIM * LIGHT_DIM * 4) as usize;
    let mut dark = Image::new(
        Extent3d {
            width: LIGHT_DIM,
            height: LIGHT_DIM,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; size],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // Карта растягивается на тайлы — линейная фильтрация даёт мягкие края
    // (в SS14 карта света семплируется билинейно).
    // Ближайший сосед, а не линейная фильтрация: при одном текселе на тайл
    // линейная интерполяция размазывала свет через границу стены — за стеной
    // было видно полосу освещённого пространства. Мягкость даёт размытие самой
    // карты (3×3), а не апскейл.
    dark.sampler = bevy::image::ImageSampler::nearest();
    let glow = Image::new(
        Extent3d {
            width: LIGHT_DIM,
            height: LIGHT_DIM,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; size],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    let image = images.add(dark);
    let glow_image = images.add(glow);
    let sprite = commands
        .spawn((
            Sprite::from_image(image.clone()),
            Transform::from_xyz(0.0, 0.0, DARK_Z)
                .with_scale(Vec3::splat(TILE_UNITS / LIGHT_SUBTEXELS as f32)),
        ))
        .id();
    let glow_sprite = commands
        .spawn((
            Sprite::from_image(glow_image.clone()),
            Transform::from_xyz(0.0, 0.0, GLOW_Z)
                .with_scale(Vec3::splat(TILE_UNITS / LIGHT_SUBTEXELS as f32)),
        ))
        .id();
    commands.insert_resource(LightMap {
        image,
        sprite,
        glow_image,
        glow_sprite,
        ready: false,
    });
}

/// Затухание света из `light_shared.swsl`: `s = clamp(sqrt(d²+1)/radius)`,
/// `val = (1-s²)² / (1 + falloff·s)`.
#[allow(clippy::too_many_arguments)]
pub fn update_lighting(
    time: Res<Time>,
    mut next_rebuild: Local<f32>,
    mut state: Local<WorldState>,
    own: Res<OwnPlayerEntity>,
    positions: Query<&PlayerPosition>,
    chunks: Query<&TileChunkData>,
    lamps: Query<(&Light, &ItemPosition, Option<&Powered>)>,
    doors: Query<&ssr_core::Door>,
    light_map: Option<ResMut<LightMap>>,
    scene: Option<ResMut<LightScene>>,
    mut images: ResMut<Assets<Image>>,
    mut transforms: Query<&mut Transform>,
) {
    let Some(mut map) = light_map else {
        return;
    };
    let Some(player_position) = own.0.and_then(|entity| positions.get(entity).ok()) else {
        return;
    };
    let player_tile = (
        (player_position.0[0] / TILE_UNITS).floor() as i32,
        (player_position.0[1] / TILE_UNITS).floor() as i32,
    );

    // Спрайты центрируем каждый кадр по игроку (дёшево), карту — по изменениям.
    let center_x = (player_tile.0 as f32 + 0.5) * TILE_UNITS;
    let center_y = (player_tile.1 as f32 + 0.5) * TILE_UNITS;
    for sprite in [map.sprite, map.glow_sprite] {
        if let Ok(mut transform) = transforms.get_mut(sprite) {
            transform.translation.x = center_x;
            transform.translation.y = center_y;
        }
    }

    // Отпечаток мира: если ничего не изменилось — пересчёта нет.
    let lamp_state: Vec<(i32, i32, u8, u8)> = lamps
        .iter()
        .filter(|(_, _, powered)| !powered.is_some_and(|powered| !powered.0))
        .map(|(light, position, _)| {
            (
                (position.0[0] / TILE_UNITS).floor() as i32,
                (position.0[1] / TILE_UNITS).floor() as i32,
                light.radius.round().clamp(1.0, 30.0) as u8,
                (light.energy * 100.0).clamp(0.0, 255.0) as u8,
            )
        })
        .collect();
    let closed_doors: Vec<(i32, i32)> = doors
        .iter()
        .filter(|door| !door.open)
        .map(|door| {
            (
                (door.position[0] / TILE_UNITS).floor() as i32,
                (door.position[1] / TILE_UNITS).floor() as i32,
            )
        })
        .collect();
    let mut chunk_hash: u64 = 0;
    for chunk in chunks.iter() {
        chunk_hash = chunk_hash
            .wrapping_mul(33)
            .wrapping_add(chunk.coords.0 as u64 ^ chunk.coords.1 as u64);
    }
    // Сцена для GPU-конвейера: окклюдеры строит core (грани стен без общих
    // рёбер — правило движка), источники — в мировых единицах.
    if let Some(mut scene) = scene {
        let chunk_refs: Vec<&TileChunkData> = chunks.iter().collect();
        scene.walls = ssr_core::occluders::build_occluders(&chunk_refs, &closed_doors);
        scene.lights = lamps
            .iter()
            .filter(|(_, _, powered)| powered.map(|p| p.0).unwrap_or(true))
            .map(|(light, position, _)| GpuLight {
                position: (position.0[0], position.0[1]),
                radius: light.radius * TILE_UNITS,
                energy: light.energy,
                falloff: FALLOFF,
                curve: 0.0,
                color: [
                    light.color[0] as f32 / 255.0,
                    light.color[1] as f32 / 255.0,
                    light.color[2] as f32 / 255.0,
                ],
            })
            .collect();
        let center = (player_position.0[0], player_position.0[1]);
        let viewport = (VIEWPORT_TILES.0 * TILE_UNITS, VIEWPORT_TILES.1 * TILE_UNITS);
        scene.camera = (center.0 - viewport.0 * 0.5, center.1 - viewport.1 * 0.5);
        scene.viewport = viewport;
        scene.eye = center;
    }
    let signature = WorldState {
        player_tile,
        lamps: lamp_state.clone(),
        closed_doors: closed_doors.clone(),
        chunks: chunk_hash,
    };
    *next_rebuild += time.delta_secs();
    let changed = *state != signature;
    if !changed || (map.ready && *next_rebuild < REBUILD_PERIOD) {
        return;
    }
    *next_rebuild = 0.0;
    *state = signature;
    map.ready = true;

    // Чанки — в хеш-таблицу: раньше каждый тайл сканировал все чанки линейно.
    let mut chunk_map: HashMap<(i32, i32), &TileChunkData> = HashMap::new();
    let size_i = CHUNK_TILES as i32;
    for chunk in chunks.iter() {
        chunk_map.insert((chunk.coords.0, chunk.coords.1), chunk);
    }
    let tile_at = |tx: i32, ty: i32| -> TileType {
        let coords = (tx.div_euclid(size_i), ty.div_euclid(size_i));
        chunk_map
            .get(&coords)
            .map(|chunk| {
                chunk.get_local(
                    (tx - coords.0 * size_i) as u32,
                    (ty - coords.1 * size_i) as u32,
                )
            })
            .unwrap_or(TileType::Wall)
    };

    let origin = (player_tile.0 - WINDOW_RADIUS, player_tile.1 - WINDOW_RADIUS);
    let side = WINDOW_SIDE;
    let wall = |tx: i32, ty: i32| -> bool {
        let (lx, ly) = (tx - origin.0, ty - origin.1);
        if lx < 0 || ly < 0 || lx >= side as i32 || ly >= side as i32 {
            return true;
        }
        tile_at(tx, ty) == TileType::Wall
    };
    let mut grid = vec![false; (side * side) as usize];
    for ly in 0..side {
        for lx in 0..side {
            grid[(ly * side + lx) as usize] = wall(origin.0 + lx as i32, origin.1 + ly as i32);
        }
    }
    for (tx, ty) in &closed_doors {
        let (lx, ly) = (tx - origin.0, ty - origin.1);
        if lx >= 0 && ly >= 0 && lx < side as i32 && ly < side as i32 {
            grid[(ly as u32 * side + lx as u32) as usize] = true;
        }
    }

    // === Свет и тени ЛУЧАМИ по модели движка (SS14_LIGHTING.md) ===
    // Вместо тайловой заливки: полярные карты теней на каждый источник
    // (`shadow-depth` в движке — 512 бинов «расстояние до стены по углу»),
    // мягкая тень из `light-soft.swsl` (7 выборок вдоль перпендикуляра, сигма
    // и гауссовы веса от расстояния до ближайшего окклюдера) и мягкая маска
    // видимости по Chebyshev. Карта считается по полтайла, поэтому тени —
    // прямые лучи с полутенями, а не квадраты тайлов.
    use ssr_core::light::{NO_OCCLUDER, attenuation, chebyshev_upper_bound, ray_segment_distance};
    const SUB: f32 = LIGHT_SUBTEXELS as f32;
    let dim = LIGHT_DIM;
    let step = 1.0 / SUB;

    // 1) Стены окна — отрезками: грань стены, у которой сосед не стена (общие
    //    рёбра массива стен теней не дают — правило движка).
    let wall_at = |lx: i32, ly: i32| -> bool {
        if lx < 0 || ly < 0 || lx >= side as i32 || ly >= side as i32 {
            return true;
        }
        grid[(ly as u32 * side + lx as u32) as usize]
    };
    let mut segments: Vec<((f32, f32), (f32, f32))> = Vec::new();
    for ly in 0..side as i32 {
        for lx in 0..side as i32 {
            if !wall_at(lx, ly) {
                continue;
            }
            let (x0, y0) = ((origin.0 + lx) as f32, (origin.1 + ly) as f32);
            let (x1, y1) = (x0 + 1.0, y0 + 1.0);
            if !wall_at(lx, ly - 1) {
                segments.push(((x0, y0), (x1, y0)));
            }
            if !wall_at(lx, ly + 1) {
                segments.push(((x0, y1), (x1, y1)));
            }
            if !wall_at(lx - 1, ly) {
                segments.push(((x0, y0), (x0, y1)));
            }
            if !wall_at(lx + 1, ly) {
                segments.push(((x1, y0), (x1, y1)));
            }
        }
    }

    // 2) Полярная карта: для точки — расстояние до стены по каждому углу.
    let polar_map = |point: (f32, f32)| -> Vec<f32> {
        let mut map = vec![NO_OCCLUDER; SHADOW_BINS];
        for (bin, slot) in map.iter_mut().enumerate() {
            let angle = (bin as f32 / SHADOW_BINS as f32) * 2.0 * PI - PI;
            let dir = (angle.cos(), angle.sin());
            let mut best = NO_OCCLUDER;
            for &(a, b) in &segments {
                if let Some(dist) = ray_segment_distance(point, dir, a, b) {
                    best = best.min(dist);
                }
            }
            *slot = best;
        }
        map
    };
    let lamp_maps: Vec<Vec<f32>> = lamp_state
        .iter()
        .map(|(lx, ly, _, _)| polar_map((*lx as f32 + 0.5, *ly as f32 + 0.5)))
        .collect();
    let eye_map = polar_map((player_tile.0 as f32 + 0.5, player_tile.1 as f32 + 0.5));

    // Чтение карты по направлению: линейная интерполяция бинов и заворот на шве
    // ±π (в движке `WrapMode.Repeat` при `Filter = true`).
    let sample_map = |map: &[f32], dx: f32, dy: f32| -> f32 {
        let u = (dy.atan2(-dx) / PI + 1.0).clamp(0.0, 1.0) * SHADOW_BINS as f32 - 0.5;
        let base = u.floor();
        let t = u - base;
        let i0 = (base as isize).rem_euclid(SHADOW_BINS as isize) as usize;
        let i1 = (base as isize + 1).rem_euclid(SHADOW_BINS as isize) as usize;
        map[i0] * (1.0 - t) + map[i1] * t
    };
    // Момент VSM по расстоянию (дисперсию добавляем, как `shadow-depth.frag`).
    let moment = |dist: f32| (dist, dist * dist + 0.25);

    // 3) Мягкая тень из `light-soft.swsl`: 7 выборок по перпендикуляру к лучу,
    //    сигма и гауссовы веса по расстоянию до ближайшего окклюдера.
    let soft_shadow = |map: &[f32], diff: (f32, f32)| -> f32 {
        let len = (diff.0 * diff.0 + diff.1 * diff.1).sqrt();
        let norm = len.max(1e-6);
        let perp = (-diff.1 / norm, diff.0 / norm);
        // `1/32 * lightSoftness * 1.5` при softness = 1 — как в шейдере.
        let offset_step = 1.0 / 32.0 * 1.5;
        let mut samples = [0.0f32; 7];
        let mut mindist = NO_OCCLUDER;
        for k in 0..4 {
            let offset = (
                perp.0 * offset_step * k as f32,
                perp.1 * offset_step * k as f32,
            );
            let plus = sample_map(map, diff.0 + offset.0, diff.1 + offset.1);
            if k == 0 {
                samples[0] = plus;
                mindist = mindist.min(plus);
            } else {
                let minus = sample_map(map, diff.0 - offset.0, diff.1 - offset.1);
                samples[k] = plus;
                samples[k + 3] = minus;
                mindist = mindist.min(plus).min(minus);
            }
        }
        let mindist = mindist.max(0.001);
        let sigma = ((len - mindist) * 0.75).max(0.001);
        let mut weights = [0.0f32; 4];
        for (k, weight) in weights.iter_mut().enumerate() {
            let kf = k as f32;
            *weight = (-(kf * kf) / (2.0 * sigma * sigma)).exp();
        }
        let total = weights[0] + 2.0 * (weights[1] + weights[2] + weights[3]);
        let mut occlusion = 0.0f32;
        for (k, &value) in samples.iter().enumerate() {
            let weight = match k {
                0 => weights[0],
                1 | 2 => weights[1],
                3 | 4 => weights[2],
                _ => weights[3],
            };
            occlusion += chebyshev_upper_bound(moment(value), len) * weight;
        }
        occlusion / total.max(1e-4)
    };

    // 4) Свет по текселям полтайла: аддитивные вклады источников (в движке
    //    `BlendFunc(SrcAlpha, One)`), затем ambient карты.
    let mut light = vec![AMBIENT; (dim * dim) as usize];
    let mut tint = vec![[0.0f32; 3]; (dim * dim) as usize];
    for (lamp_index, (lamp_x, lamp_y, radius, energy)) in lamp_state.iter().enumerate() {
        let map = &lamp_maps[lamp_index];
        let center = (*lamp_x as f32 + 0.5, *lamp_y as f32 + 0.5);
        let radius = *radius as f32;
        let energy = *energy as f32 / 100.0;
        let color = lamp_color(*lamp_x, *lamp_y);
        let min_x = (((center.0 - radius) - origin.0 as f32) * SUB)
            .floor()
            .max(0.0) as u32;
        let max_x = ((((center.0 + radius) - origin.0 as f32) * SUB).ceil()).min(dim as f32) as u32;
        let min_y = (((center.1 - radius) - origin.1 as f32) * SUB)
            .floor()
            .max(0.0) as u32;
        let max_y = ((((center.1 + radius) - origin.1 as f32) * SUB).ceil()).min(dim as f32) as u32;
        for ty in min_y..max_y {
            for tx in min_x..max_x {
                let px = origin.0 as f32 + (tx as f32 + 0.5) * step;
                let py = origin.1 as f32 + (ty as f32 + 0.5) * step;
                let diff = (px - center.0, py - center.1);
                let dist2 = diff.0 * diff.0 + diff.1 * diff.1;
                if dist2 > radius * radius {
                    continue;
                }
                let occlusion = soft_shadow(map, diff);
                if occlusion <= 0.0 {
                    continue;
                }
                // `sqr_dist` в шейдере считается с `LIGHTING_HEIGHT`: свет висит
                // на высоте 1 м над полом.
                let value = attenuation(
                    dist2 + ssr_core::light::LIGHTING_HEIGHT,
                    radius,
                    energy,
                    FALLOFF,
                    0.0,
                ) * occlusion;
                if value <= 0.0 {
                    continue;
                }
                let index = (ty * dim + tx) as usize;
                light[index] += value;
                for channel in 0..3 {
                    tint[index][channel] += color[channel] * value;
                }
            }
        }
    }

    // 5) Видимость глаза — мягкая маска (`fov-lighting.swsl`: `occlusion =
    //    Chebyshev(момент карты FOV, расстояние)`), а не бинарный флаг.
    let mut visible = vec![0.0f32; (dim * dim) as usize];
    for ty in 0..dim {
        for tx in 0..dim {
            let px = origin.0 as f32 + (tx as f32 + 0.5) * step;
            let py = origin.1 as f32 + (ty as f32 + 0.5) * step;
            let diff = (
                px - (player_tile.0 as f32 + 0.5),
                py - (player_tile.1 as f32 + 0.5),
            );
            let len = (diff.0 * diff.0 + diff.1 * diff.1).sqrt();
            let wall = sample_map(&eye_map, diff.0, diff.1);
            visible[(ty * dim + tx) as usize] = chebyshev_upper_bound(moment(wall), len);
        }
    }

    // 6) Просачивание света на стены (`wall-bleed` в движке): тайл-стена берёт
    //    максимум света соседей, иначе стены были бы чёрными.
    let mut bleed = light.clone();
    for ty in 1..dim - 1 {
        for tx in 1..dim - 1 {
            let tile = ((tx / LIGHT_SUBTEXELS) as i32, (ty / LIGHT_SUBTEXELS) as i32);
            if !wall_at(tile.0, tile.1) {
                continue;
            }
            let index = (ty * dim + tx) as usize;
            let mut best = light[index];
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let neighbor =
                    (((ty as i32 + dy) as u32) * dim + ((tx as i32 + dx) as u32)) as usize;
                best = best.max(light[neighbor]);
            }
            bleed[index] = best;
        }
    }
    let (blurred, blurred_tint) = (bleed, tint);

    // 5) Пишем слои: тьма alpha = 1 − свет×видимость (это и есть light×fov
    //    одним оверлеем), оттенок — под тьмой, она его маскирует.
    {
        let Some(mut image) = images.get_mut(&map.image) else {
            return;
        };
        let Some(data) = image.data.as_mut() else {
            return;
        };
        data.fill(0);
        for ly in 0..dim {
            for lx in 0..dim {
                let index = (ly * dim + lx) as usize;
                let luminance = blurred[index];
                let occlusion = 1.0 - luminance * visible[index];
                let row = dim - 1 - ly;
                let target = ((row * dim + lx) * 4) as usize;
                data[target + 3] = (occlusion.clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }
    if let Some(mut glow_image) = images.get_mut(&map.glow_image)
        && let Some(glow_data) = glow_image.data.as_mut()
    {
        glow_data.fill(0);
        for ly in 0..dim {
            for lx in 0..dim {
                let index = (ly * dim + lx) as usize;
                let contribution = (blurred[index] - AMBIENT).max(0.0);
                if contribution < 0.001 {
                    continue;
                }
                let rgb = blurred_tint[index];
                let norm = rgb[0].max(rgb[1]).max(rgb[2]).max(0.001);
                let row = dim - 1 - ly;
                let target = ((row * dim + lx) * 4) as usize;
                glow_data[target] = ((rgb[0] / norm) * 255.0) as u8;
                glow_data[target + 1] = ((rgb[1] / norm) * 255.0) as u8;
                glow_data[target + 2] = ((rgb[2] / norm) * 255.0) as u8;
                glow_data[target + 3] =
                    ((contribution * visible[index] * GLOW_ALPHA).min(1.0) * 255.0) as u8;
            }
        }
    }
    tracing::info!(lamps = lamp_state.len(), ?player_tile, "light map rebuilt");
}

/// Цвет лампы: у нас все лампы тёплые как коридорные в SS14 (`#FFE4CE`).
fn lamp_color(_tx: i32, _ty: i32) -> [f32; 3] {
    [1.0, 0.7758, 0.6172] // #FFE4CE в линейном пространстве
}
