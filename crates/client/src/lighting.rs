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
/// Высота источника над полом (`LIGHTING_HEIGHT = 1.0` в шейдере).
const LIGHTING_HEIGHT: f32 = 1.0;
/// Полуразмер окна карты в тайлах: вьюпорт 21×15, диагональ ≈ 13, плюс поля.
const WINDOW_RADIUS: i32 = 16;
/// Сторона окна (квадрат, нечётная — центр совпадает с тайлом игрока).
const WINDOW_SIDE: u32 = (WINDOW_RADIUS * 2 + 1) as u32;
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
    let size = (WINDOW_SIDE * WINDOW_SIDE * 4) as usize;
    let mut dark = Image::new(
        Extent3d {
            width: WINDOW_SIDE,
            height: WINDOW_SIDE,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; size],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    // Карта растягивается на тайлы — линейная фильтрация даёт мягкие края
    // (в SS14 карта света семплируется билинейно).
    dark.sampler = bevy::image::ImageSampler::linear();
    let glow = Image::new(
        Extent3d {
            width: WINDOW_SIDE,
            height: WINDOW_SIDE,
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
            Transform::from_xyz(0.0, 0.0, DARK_Z).with_scale(Vec3::splat(TILE_UNITS)),
        ))
        .id();
    let glow_sprite = commands
        .spawn((
            Sprite::from_image(glow_image.clone()),
            Transform::from_xyz(0.0, 0.0, GLOW_Z).with_scale(Vec3::splat(TILE_UNITS)),
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
/// Видно ли тайл `to` из тайла `from`: суперпокрывающая линия Брезенхэма.
/// Стена на пути перекрывает видимость (сам конечный тайл может быть стеной —
/// его лицевая грань видна). Диагональный шаг требует свободных обеих
/// ортогональных клеток — через стык двух стен под углом не видно, как и в
/// геометрии окклюдеров движка.
fn line_of_sight(from: (i32, i32), to: (i32, i32), solid: &impl Fn(i32, i32) -> bool) -> bool {
    let (x0, y0) = from;
    let (x1, y1) = to;
    let (dx, dy) = ((x1 - x0).abs(), (y1 - y0).abs());
    let (sx, sy) = (if x0 < x1 { 1 } else { -1 }, if y0 < y1 { 1 } else { -1 });
    let mut err = dx - dy;
    let (mut x, mut y) = (x0, y0);
    while (x, y) != (x1, y1) {
        let e2 = 2 * err;
        if e2 > -dy && e2 < dx {
            if solid(x + sx, y) || solid(x, y + sy) {
                return false;
            }
            x += sx;
            y += sy;
            err += dx - dy;
        } else if e2 > -dy {
            x += sx;
            err -= dy;
        } else {
            y += sy;
            err += dx;
        }
        if (x, y) != (x1, y1) && solid(x, y) {
            return false;
        }
    }
    true
}

fn attenuation(dist: f32, radius: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    let s = ((dist * dist + LIGHTING_HEIGHT).sqrt() / radius).clamp(0.0, 1.0);
    let s2 = s * s;
    ((1.0 - s2) * (1.0 - s2) / (1.0 + FALLOFF * s)).clamp(0.0, 1.0)
}

/// Пересобирает объединённую карту «свет × туман» при изменениях мира.
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

    // 1) Видимость (туман войны): СТРОГАЯ прямая видимость из тайла игрока —
    //    как теневой конус FOV в движке (`fov-lighting.swsl`): стена перекрывает
    //    всё, что за ней. Раньше здесь была заливка BFS, и свет был виден за
    //    углами стен — владелец: «не должно быть видно ничего за стеной».
    let mut fov = vec![0.0f32; (side * side) as usize];
    {
        let center = (WINDOW_RADIUS, WINDOW_RADIUS);
        let solid = |x: i32, y: i32| -> bool {
            if x < 0 || y < 0 || x >= side as i32 || y >= side as i32 {
                return true;
            }
            grid[(y as u32 * side + x as u32) as usize]
        };
        for ly in 0..side as i32 {
            for lx in 0..side as i32 {
                if line_of_sight(center, (lx, ly), &solid) {
                    fov[(ly as u32 * side + lx as u32) as usize] = 1.0;
                }
            }
        }
    }

    // 2) Свет: ambient + BFS от каждой лампы по нестенным клеткам.
    //    Складываем вклады аддитивно (как SrcAlpha+One в SS14).
    let mut light = vec![AMBIENT; (side * side) as usize];
    let mut tint = vec![[0.0f32; 3]; (side * side) as usize];
    for (lamp_x, lamp_y, radius, energy) in &lamp_state {
        let (lx, ly) = (lamp_x - origin.0, lamp_y - origin.1);
        let radius = *radius as f32;
        let energy = *energy as f32 / 100.0;
        if lx < 0 || ly < 0 || lx >= side as i32 || ly >= side as i32 {
            continue;
        }
        let start = (lx, ly);
        let start_index = (ly as u32 * side + lx as u32) as usize;
        if grid[start_index] {
            continue; // лампа внутри стены (как отключённая в SS14)
        }
        let mut queue = std::collections::VecDeque::new();
        let mut visited = vec![false; (side * side) as usize];
        visited[start_index] = true;
        queue.push_back(start);
        while let Some((cx, cy)) = queue.pop_front() {
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let (nx, ny) = (cx + dx, cy + dy);
                if nx < 0 || ny < 0 || nx >= side as i32 || ny >= side as i32 {
                    continue;
                }
                let index = (ny as u32 * side + nx as u32) as usize;
                if grid[index] || visited[index] {
                    continue;
                }
                visited[index] = true;
                let distance = (((nx - lx) as f32).powi(2) + ((ny - ly) as f32).powi(2)).sqrt();
                if distance > radius {
                    continue;
                }
                let value = attenuation(distance, radius) * energy;
                if value < 0.001 {
                    continue;
                }
                light[index] += value;
                // Цвет лампы (тёплый #FFE4CE) накапливается тем же вкладом.
                let color = lamp_color(*lamp_x, *lamp_y);
                for channel in 0..3 {
                    tint[index][channel] += color[channel] * value;
                }
                queue.push_back((nx, ny));
            }
        }
    }

    // 3) Wall bleed: стены берут максимум света соседей (свет «просачивается»
    //    в стену, но не сквозь неё — как wall-bleed-blur + wall-merge в SS14).
    let mut bleed = light.clone();
    for ly in 1..side - 1 {
        for lx in 1..side - 1 {
            let index = (ly * side + lx) as usize;
            if !grid[index] {
                continue;
            }
            let mut best = light[index];
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let neighbor = ((ly as i32 + dy) as u32 * side + (lx as i32 + dx) as u32) as usize;
                best = best.max(light[neighbor]);
            }
            bleed[index] = best;
        }
    }
    let light = bleed;

    // 4) Мягкие края: размытие 3×3 по полу (стены и туман не перетекают).
    let mut blurred = light.clone();
    let mut blurred_tint = tint.clone();
    for ly in 1..side - 1 {
        for lx in 1..side - 1 {
            let index = (ly * side + lx) as usize;
            if grid[index] {
                continue;
            }
            let mut sum = 0.0f32;
            let mut count = 0.0f32;
            let mut rgb = [0.0f32; 3];
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let x = (lx as i32 + dx) as u32;
                    let y = (ly as i32 + dy) as u32;
                    let neighbor = (y * side + x) as usize;
                    if grid[neighbor] {
                        continue;
                    }
                    sum += light[neighbor];
                    for channel in 0..3 {
                        rgb[channel] += tint[neighbor][channel];
                    }
                    count += 1.0;
                }
            }
            blurred[index] = sum / count.max(1.0f32);
            for channel in 0..3 {
                blurred_tint[index][channel] = rgb[channel] / count.max(1.0f32);
            }
        }
    }

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
        for ly in 0..side {
            for lx in 0..side {
                let index = (ly * side + lx) as usize;
                let visible = fov[index];
                let luminance = blurred[index];
                let occlusion = 1.0 - luminance * visible;
                let row = side - 1 - ly;
                let target = ((row * side + lx) * 4) as usize;
                data[target + 3] = (occlusion.clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }
    if let Some(mut glow_image) = images.get_mut(&map.glow_image)
        && let Some(glow_data) = glow_image.data.as_mut()
    {
        glow_data.fill(0);
        for ly in 0..side {
            for lx in 0..side {
                let index = (ly * side + lx) as usize;
                let contribution = (blurred[index] - AMBIENT).max(0.0);
                if contribution < 0.001 {
                    continue;
                }
                let rgb = blurred_tint[index];
                let norm = rgb[0].max(rgb[1]).max(rgb[2]).max(0.001);
                let row = side - 1 - ly;
                let target = ((row * side + lx) * 4) as usize;
                glow_data[target] = ((rgb[0] / norm) * 255.0) as u8;
                glow_data[target + 1] = ((rgb[1] / norm) * 255.0) as u8;
                glow_data[target + 2] = ((rgb[2] / norm) * 255.0) as u8;
                glow_data[target + 3] =
                    ((contribution * fov[index] * GLOW_ALPHA).min(1.0) * 255.0) as u8;
            }
        }
    }
    tracing::info!(lamps = lamp_state.len(), ?player_tile, "light map rebuilt");
}

/// Цвет лампы: у нас все лампы тёплые как коридорные в SS14 (`#FFE4CE`).
fn lamp_color(_tx: i32, _ty: i32) -> [f32; 3] {
    [1.0, 0.7758, 0.6172] // #FFE4CE в линейном пространстве
}
