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
/// Текселей карты света на тайл: свет и тени считаются по подтайлу, поэтому
/// тень от стены — прямая линия, а не квадрат тайла (в движке карта света
/// тоже вдвое мельче экрана, `light.resolution_scale = 0.5`; у нас окно карты
/// шире вьюпорта, поэтому 8 подтайлов дают ~11 px на тексель — вплотную к
/// «полпикселя экрана» движка).
const LIGHT_SUBTEXELS: u32 = 8;
/// Сторона карты света в текселях.
const LIGHT_DIM: u32 = WINDOW_SIDE * LIGHT_SUBTEXELS;
/// Ширина полярной карты теней — `ShadowMapSize` в движке (512 px по кругу).
const SHADOW_BINS: usize = 512;
const PI: f32 = std::f32::consts::PI;
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
/// Ограничение стороны светового поля в тайлах: у импортированных карт
/// (aspid и т.п.) карта огромная, а поле света полуэкранное и меньше.
const FIELD_MAX_TILES: i32 = 128;
/// Как часто обновляется маска видимости окна (сек). Свет живёт в поле и при
/// ходьбе не пересчитывается — маска мягкая, 20 Гц ей достаточно.
const OVERLAY_PERIOD: f32 = 0.05;

/// Карта света: слой тьмы и слой тёплого оттенка.
#[derive(Resource)]
pub struct LightMap {
    image: Handle<Image>,
    sprite: Entity,
    glow_image: Handle<Image>,
    glow_sprite: Entity,
    /// Пересчёт уже был — дальше только при изменениях.
    ready: bool,
    /// Световое поле (мир, `LIGHT_SUBTEXELS` текселей на тайл): свет ламп не
    /// зависит от игрока, поэтому считается один раз на изменения мира.
    field: Vec<f32>,
    tint_field: Vec<[f32; 3]>,
    field_origin: (i32, i32),
    field_dim: (u32, u32),
    field_valid: bool,
}

/// Отпечаток мира: при изменении карта пересобирается.
#[derive(Default, PartialEq, Clone)]
pub(crate) struct WorldState {
    player_tile: (i32, i32),
    lamps: Vec<(i32, i32, u8, u8)>, // тайл, радиус, яркость×100
    closed_doors: Vec<(i32, i32)>,
    chunks: u64, // хеш набора чанков (репликация карты могла подгрузить новый)
}

/// Кэш полярных карт теней ламп: карта лампы зависит только от стен в её
/// радиусе (отрезки строятся по всем чанкам), поэтому при неизменном мире
/// пересчитывается лишь световая сумма, а не 512 бинов на каждую лампу.
#[derive(Default)]
pub struct LampMapCache {
    /// Подпись геометрии мира (чанки + закрытые двери).
    geometry: u64,
    maps: HashMap<(i32, i32, u8), Vec<f32>>,
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
        field: Vec::new(),
        tint_field: Vec::new(),
        field_origin: (0, 0),
        field_dim: (0, 0),
        field_valid: false,
    });
}

/// Затухание света из `light_shared.swsl`: `s = clamp(sqrt(d²+1)/radius)`,
/// `val = (1-s²)² / (1 + falloff·s)`.
#[allow(clippy::too_many_arguments)]
pub fn update_lighting(
    time: Res<Time>,
    mut next_rebuild: Local<f32>,
    mut state: Local<WorldState>,
    mut lamp_cache: Local<LampMapCache>,
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
    // Геометрия (лампы, двери, чанки) меняется редко: световое поле считается
    // только тогда. При обычной ходьбе пересчитывается лишь маска видимости
    // поверх готового поля — иначе 47 ламп × 512 бинов считались на каждый шаг.
    let geometry_changed = {
        let mut probe = signature.clone();
        probe.player_tile = state.player_tile;
        probe != *state
    };
    let tile_changed = signature.player_tile != state.player_tile;
    if !tile_changed && !geometry_changed && map.ready {
        return;
    }
    *next_rebuild += time.delta_secs();
    // Маску обновляем не чаще 20 раз в секунду: она мягкая, а пересчёт дорогой.
    // Позицию не «теряем»: `state` обновляется только при настоящем пересчёте.
    if !geometry_changed && map.ready && *next_rebuild < OVERLAY_PERIOD {
        return;
    }
    *next_rebuild = 0.0;
    *state = signature;
    map.ready = true;
    use ssr_core::light::{
        NO_OCCLUDER, attenuation, chebyshev_upper_bound, ray_segment_distance,
    };
    let rebuild_started = std::time::Instant::now();

    // Чанки — в хеш-таблицу: раньше каждый тайл сканировал все чанки линейно.
    let mut chunk_map: HashMap<(i32, i32), &TileChunkData> = HashMap::new();
    let size_i = CHUNK_TILES as i32;
    let mut bounds = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for chunk in chunks.iter() {
        chunk_map.insert((chunk.coords.0, chunk.coords.1), chunk);
        bounds.0 = bounds.0.min(chunk.coords.0 * size_i);
        bounds.1 = bounds.1.min(chunk.coords.1 * size_i);
        bounds.2 = bounds.2.max(chunk.coords.0 * size_i + size_i);
        bounds.3 = bounds.3.max(chunk.coords.1 * size_i + size_i);
    }
    if bounds.0 > bounds.2 {
        return; // чанков ещё нет
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
    let solid_tile = |tx: i32, ty: i32| -> bool {
        tile_at(tx, ty) == TileType::Wall || closed_doors.contains(&(tx, ty))
    };

    // === Световое поле (по ВСЕЙ карте, но не больше `FIELD_MAX_TILES`) ===
    // Отрезки стен строим по всем чанкам сразу: тогда полярные карты ламп не
    // зависят от положения игрока и кэшируются, а поле света не пересчитывается
    // при ходьбе (в SS14 свет тоже считается в мировых координатах, а не от игрока).
    let mut segments: Vec<((f32, f32), (f32, f32))> = Vec::new();
    for chunk in chunks.iter() {
        let base = (chunk.coords.0 * size_i, chunk.coords.1 * size_i);
        for ly in 0..size_i {
            for lx in 0..size_i {
                if chunk.get_local(lx as u32, ly as u32) != TileType::Wall {
                    continue;
                }
                let (x, y) = (base.0 + lx, base.1 + ly);
                let (x0, y0) = (x as f32, y as f32);
                let (x1, y1) = (x0 + 1.0, y0 + 1.0);
                if !solid_tile(x, y - 1) {
                    segments.push(((x0, y0), (x1, y0)));
                }
                if !solid_tile(x, y + 1) {
                    segments.push(((x0, y1), (x1, y1)));
                }
                if !solid_tile(x - 1, y) {
                    segments.push(((x0, y0), (x0, y1)));
                }
                if !solid_tile(x + 1, y) {
                    segments.push(((x1, y0), (x1, y1)));
                }
            }
        }
    }

    // Полярная карта: для точки — расстояние до стены по каждому углу.
    // Окклюдеры отсекаются по «досягаемости» вокруг точки: у лампы стены
    // дальше радиуса света луч не перекрывают, у глаза — весь вьюпорт.
    let polar_map = |point: (f32, f32), reach: f32| -> Vec<f32> {
        let near: Vec<((f32, f32), (f32, f32))> = segments
            .iter()
            .copied()
            .filter(|(a, b)| {
                let mid = ((a.0 + b.0) * 0.5, (a.1 + b.1) * 0.5);
                let dx = mid.0 - point.0;
                let dy = mid.1 - point.1;
                dx * dx + dy * dy <= (reach + 1.0) * (reach + 1.0)
            })
            .collect();
        let mut map = vec![NO_OCCLUDER; SHADOW_BINS];
        for (bin, slot) in map.iter_mut().enumerate() {
            let angle = (bin as f32 / SHADOW_BINS as f32) * 2.0 * PI - PI;
            let dir = (angle.cos(), angle.sin());
            let mut best = NO_OCCLUDER;
            for &(a, b) in &near {
                if let Some(dist) = ray_segment_distance(point, dir, a, b) {
                    best = best.min(dist);
                }
            }
            *slot = best;
        }
        map
    };
    let lamp_maps: Vec<Vec<f32>> = {
        // Подпись геометрии: закрытые двери меняют окклюдеры.
        let mut geometry = chunk_hash;
        for (x, y) in &closed_doors {
            geometry = geometry
                .wrapping_mul(1099511628211)
                .wrapping_add((*x as u64) << 32 | *y as u64);
        }
        if lamp_cache.geometry != geometry {
            lamp_cache.maps.clear();
            lamp_cache.geometry = geometry;
        }
        lamp_state
            .iter()
            .map(|(lx, ly, radius, _)| {
                let key = (*lx, *ly, *radius);
                match lamp_cache.maps.get(&key) {
                    Some(cached) => cached.clone(),
                    None => {
                        let built = polar_map((*lx as f32 + 0.5, *ly as f32 + 0.5), *radius as f32);
                        lamp_cache.maps.insert(key, built.clone());
                        built
                    }
                }
            })
            .collect()
    };
    let walls_ms = rebuild_started.elapsed().as_millis() as u64;

    // Чтение карты по направлению: линейная интерполяция бинов и заворот на шве
    // ±π (в движке `WrapMode.Repeat` при `Filter = true`). Индекс даёт
    // `polar_bin` — инверсия генерации (тест в ядре); формула движкового
    // `polar_u` (без деления на 2) вырождала полкарты в один бин — это и были
    // чёрные клинья «тумана войны» при живом свете вокруг.
    let sample_map = |map: &[f32], dx: f32, dy: f32| -> f32 {
        let u = ssr_core::light::polar_bin(dx, dy, SHADOW_BINS);
        let base = u.floor();
        let t = u - base;
        let i0 = (base as isize).rem_euclid(SHADOW_BINS as isize) as usize;
        let i1 = (base as isize + 1).rem_euclid(SHADOW_BINS as isize) as usize;
        map[i0] * (1.0 - t) + map[i1] * t
    };
    // Момент VSM по расстоянию (дисперсию добавляем, как `shadow-depth.frag`).
    let moment = |dist: f32| (dist, dist * dist + 0.25);

    // Мягкая тень из `light-soft.swsl`: 7 выборок по перпендикуляру к лучу,
    // сигма и гауссовы веса по расстоянию до ближайшего окклюдера.
    // Быстрый выход: если по самому лучу стена дальше точки (или её нет),
    // шесть дополнительных выборок (дорогой `atan2` в каждой) не нужны —
    // смещения в движке крошечные (±0.14 юнита), результат там всё равно 1.
    let soft_shadow = |map: &[f32], diff: (f32, f32)| -> f32 {
        let len = (diff.0 * diff.0 + diff.1 * diff.1).sqrt();
        if sample_map(map, diff.0, diff.1) >= len {
            return 1.0;
        }
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

    const SUB: f32 = LIGHT_SUBTEXELS as f32;
    let step = 1.0 / SUB;

    // Окно поля: вокруг центра чанка игрока, но не больше карты и не шире
    // `FIELD_MAX_TILES` (импортированные карты бывают очень большими).
    let chunk_tiles = CHUNK_TILES as i32;
    let chunk = (
        player_tile.0.div_euclid(chunk_tiles),
        player_tile.1.div_euclid(chunk_tiles),
    );
    let field_tiles_x = (bounds.2 - bounds.0).min(FIELD_MAX_TILES);
    let field_tiles_y = (bounds.3 - bounds.1).min(FIELD_MAX_TILES);
    let clamp_axis = |center: i32, size: i32, low: i32, high: i32| -> i32 {
        let start = center - size / 2;
        start.clamp(low, (high - size).max(low))
    };
    let field_origin = (
        clamp_axis(
            chunk.0 * chunk_tiles + chunk_tiles / 2,
            field_tiles_x,
            bounds.0,
            bounds.2,
        ),
        clamp_axis(
            chunk.1 * chunk_tiles + chunk_tiles / 2,
            field_tiles_y,
            bounds.1,
            bounds.3,
        ),
    );
    let field_dim_x = (field_tiles_x * LIGHT_SUBTEXELS as i32) as u32;
    let field_dim_y = (field_tiles_y * LIGHT_SUBTEXELS as i32) as u32;
    if geometry_changed
        || !map.field_valid
        || map.field_origin != field_origin
        || map.field_dim != (field_dim_x, field_dim_y)
    {
        map.field_origin = field_origin;
        map.field_dim = (field_dim_x, field_dim_y);
        map.field_valid = true;
        map.field.clear();
        map.field
            .resize((field_dim_x * field_dim_y) as usize, AMBIENT);
        map.tint_field.clear();
        map.tint_field
            .resize((field_dim_x * field_dim_y) as usize, [0.0; 3]);
        let fdim = field_dim_x as usize;
        let field_span = (
            field_origin.0 as f32,
            field_origin.1 as f32,
            (field_origin.0 + field_tiles_x) as f32,
            (field_origin.1 + field_tiles_y) as f32,
        );
        for (lamp_index, (lamp_x, lamp_y, radius, energy)) in lamp_state.iter().enumerate() {
            let polar = &lamp_maps[lamp_index];
            let center = (*lamp_x as f32 + 0.5, *lamp_y as f32 + 0.5);
            let radius = *radius as f32;
            let energy = *energy as f32 / 100.0;
            let color = lamp_color(*lamp_x, *lamp_y);
            // Лампа, чей радиус не дотягивается до поля, не освещает ни один
            // тексель — без этого пропуска цикл всё равно шёл по обрезанному окну.
            if center.0 + radius < field_span.0
                || center.0 - radius > field_span.2
                || center.1 + radius < field_span.1
                || center.1 - radius > field_span.3
            {
                continue;
            }
            let min_x = (((center.0 - radius) - field_span.0) * SUB).floor().max(0.0) as u32;
            let max_x = ((((center.0 + radius) - field_span.0) * SUB).ceil())
                .min(field_dim_x as f32) as u32;
            let min_y = (((center.1 - radius) - field_span.1) * SUB).floor().max(0.0) as u32;
            let max_y = ((((center.1 + radius) - field_span.1) * SUB).ceil())
                .min(field_dim_y as f32) as u32;
            for ty in min_y..max_y {
                for tx in min_x..max_x {
                    let px = field_span.0 + (tx as f32 + 0.5) * step;
                    let py = field_span.1 + (ty as f32 + 0.5) * step;
                    let diff = (px - center.0, py - center.1);
                    let dist2 = diff.0 * diff.0 + diff.1 * diff.1;
                    if dist2 > radius * radius {
                        continue;
                    }
                    let occlusion = soft_shadow(polar, diff);
                    if occlusion <= 0.0 {
                        continue;
                    }
                    // `sqr_dist` в шейдере считается с `LIGHTING_HEIGHT`: свет
                    // висит на высоте 1 м над полом.
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
                    let index = (ty as usize * fdim) + tx as usize;
                    map.field[index] += value;
                    for (channel, tint) in map.tint_field[index].iter_mut().enumerate() {
                        *tint += color[channel] * value;
                    }
                }
            }
        }
        // Просачивание света на стены (`wall-bleed` в движке): стена берёт
        // максимум света соседей, иначе стены были бы чёрными.
        let mut bleed = std::mem::take(&mut map.field);
        for ty in 1..field_dim_y - 1 {
            for tx in 1..field_dim_x - 1 {
                let tile = (
                    field_origin.0 + (tx / LIGHT_SUBTEXELS) as i32,
                    field_origin.1 + (ty / LIGHT_SUBTEXELS) as i32,
                );
                if !solid_tile(tile.0, tile.1) {
                    continue;
                }
                let index = (ty as usize * fdim) + tx as usize;
                let mut best = bleed[index];
                for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                    let neighbor = ((ty as i32 + dy) as usize * fdim) + (tx as i32 + dx) as usize;
                    best = best.max(bleed[neighbor]);
                }
                bleed[index] = best;
            }
        }
        map.field = bleed;
    }
    let maps_ms = rebuild_started.elapsed().as_millis() as u64;

    // === Оверлей окна игрока: маска видимости × готовое поле света ===
    let origin = (player_tile.0 - WINDOW_RADIUS, player_tile.1 - WINDOW_RADIUS);
    let side = WINDOW_SIDE;
    let dim = LIGHT_DIM;
    // Сетка стен окна — для просачивания видимости и диагностического дампа.
    let mut grid = vec![false; (side * side) as usize];
    for ly in 0..side {
        for lx in 0..side {
            let (tx, ty) = (origin.0 + lx as i32, origin.1 + ly as i32);
            grid[(ly * side + lx) as usize] = solid_tile(tx, ty);
        }
    }
    let wall_at = |lx: i32, ly: i32| -> bool {
        if lx < 0 || ly < 0 || lx >= side as i32 || ly >= side as i32 {
            return true;
        }
        grid[(ly as u32 * side + lx as u32) as usize]
    };

    // Видимость глаза — ровно как `fov-lighting.swsl`: ОДНА выборка полярной
    // карты по направлению на точку + Chebyshev-тест (`0.25` в моменте).
    // Мягкая 7-точечная тень применяется только к СВЕТУ источников: у маски
    // глаза она давала «слепые зоны» в глубине коридоров (боковые стены
    // затеняли точку, до которой луч доходит свободно).
    let eye_map = polar_map(
        (player_tile.0 as f32 + 0.5, player_tile.1 as f32 + 0.5),
        WINDOW_RADIUS as f32 + 1.0,
    );
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
    // Стена берёт максимум видимости соседей: луч в центр тайла упирается в саму
    // стену, поэтому без просачивания её texel был бы чёрным (в движке стены
    // видно по `wall-bleed`).
    for ty in 1..dim - 1 {
        for tx in 1..dim - 1 {
            let tile = ((tx / LIGHT_SUBTEXELS) as i32, (ty / LIGHT_SUBTEXELS) as i32);
            if !wall_at(tile.0, tile.1) {
                continue;
            }
            let index = (ty * dim + tx) as usize;
            let mut visible_best = visible[index];
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let neighbor =
                    (((ty as i32 + dy) as u32) * dim + ((tx as i32 + dx) as u32)) as usize;
                visible_best = visible_best.max(visible[neighbor]);
            }
            visible[index] = visible_best;
        }
    }
    let mask_ms = rebuild_started.elapsed().as_millis() as u64;

    // Свет для текселя окна — из поля (оба выровнены по `LIGHT_SUBTEXELS`,
    // поэтому сдвиг целочисленный и интерполяция не нужна).
    let field_dim_x_i = field_dim_x as i32;
    let offset_x = (origin.0 - map.field_origin.0) * LIGHT_SUBTEXELS as i32;
    let offset_y = (origin.1 - map.field_origin.1) * LIGHT_SUBTEXELS as i32;
    let field_luminance = |lx: u32, ly: u32| -> f32 {
        let fx = (lx as i32 + offset_x).clamp(0, field_dim_x_i - 1) as usize;
        let fy = (ly as i32 + offset_y).clamp(0, field_dim_y as i32 - 1) as usize;
        map.field[fy * field_dim_x as usize + fx]
    };
    let field_tint = |lx: u32, ly: u32| -> [f32; 3] {
        let fx = (lx as i32 + offset_x).clamp(0, field_dim_x_i - 1) as usize;
        let fy = (ly as i32 + offset_y).clamp(0, field_dim_y as i32 - 1) as usize;
        map.tint_field[fy * field_dim_x as usize + fx]
    };

    // Диагностика `SSR_LIGHT_DEBUG=1`: сетка окна и средняя тьма по тайлам —
    // так видно, ЧТО именно затемнено: стена (законно) или «слепая зона».
    if std::env::var_os("SSR_LIGHT_DEBUG").is_some() {
        let mut grid_art = String::from("\n");
        for ly in (0..side as i32).rev() {
            for lx in 0..side as i32 {
                grid_art.push(if (lx, ly) == (WINDOW_RADIUS, WINDOW_RADIUS) {
                    '@'
                } else if wall_at(lx, ly) {
                    '#'
                } else {
                    '.'
                });
            }
            grid_art.push('\n');
        }
        let mut light_art = String::from("\n");
        for ly in (0..side as i32).rev() {
            for lx in 0..side as i32 {
                let mut sum = 0.0f32;
                for sy in 0..LIGHT_SUBTEXELS {
                    for sx in 0..LIGHT_SUBTEXELS {
                        let tx = lx as u32 * LIGHT_SUBTEXELS + sx;
                        let ty = ly as u32 * LIGHT_SUBTEXELS + sy;
                        let index = (ty * dim + tx) as usize;
                        sum += 1.0 - field_luminance(tx, ty) * visible[index];
                    }
                }
                let count = (LIGHT_SUBTEXELS * LIGHT_SUBTEXELS) as f32;
                let alpha = sum / count;
                let ch = match (alpha * 6.0) as u32 {
                    0 => ' ',
                    1 => '.',
                    2 => ':',
                    3 => '+',
                    4 => '*',
                    5 => '#',
                    _ => '@',
                };
                light_art.push(if wall_at(lx, ly) {
                    ch.to_ascii_uppercase()
                } else {
                    ch
                });
            }
            light_art.push('\n');
        }
        tracing::info!("light debug grid:{grid_art}light map (тьма):{light_art}");
    }

    // Пишем слои: тьма alpha = 1 − свет×видимость (это и есть light×fov
    // одним оверлеем), оттенок — под тьмой, она его маскирует.
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
                let luminance = field_luminance(lx, ly);
                let occlusion = 1.0 - luminance * visible[(ly * dim + lx) as usize];
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
                let contribution = (field_luminance(lx, ly) - AMBIENT).max(0.0);
                if contribution < 0.001 {
                    continue;
                }
                let rgb = field_tint(lx, ly);
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
    tracing::info!(
        lamps = lamp_state.len(),
        ?player_tile,
        ms = rebuild_started.elapsed().as_millis() as u64,
        field = ?map.field_dim,
        geometry = geometry_changed,
        walls_ms = walls_ms,
        maps_ms = maps_ms,
        mask_ms = mask_ms,
        "light map rebuilt"
    );
}

/// Цвет лампы: у нас все лампы тёплые как коридорные в SS14 (`#FFE4CE`).
fn lamp_color(_tx: i32, _ty: i32) -> [f32; 3] {
    [1.0, 0.7758, 0.6172] // #FFE4CE в линейном пространстве
}
