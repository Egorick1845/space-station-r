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
/// Бинов в полярной карте тени лампы. В движке `ShadowMapSize = 512`, но там
/// карту считает GPU. У нас световое поле — 8 мировых юнитов на тексель, и
/// 256 бинов дают ту же угловую точность (10 тайлов → 8 юнитов), а пересчёт
/// карты (в том числе когда рядом открылась дверь) вдвое дешевле.
const LAMP_BINS: usize = 256;
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
/// Текселей светового поля на тайл. Поле — это сумма вкладов ламп, она гладкая,
/// и её край всё равно интерполируется в оверлей, поэтому поле вдвое грубее
/// карты тьмы: пересчёт поля (в том числе при открытии/закрытии двери, которая
/// меняет окклюдеры) дешевле вчетверо — 234 мс → ~40 мс.
const FIELD_SUBTEXELS: u32 = 4;
/// Бинов в полярной карте глаза: у движка `FovMapSize = 2048`, но там карту
/// считает GPU. 256 бинов при 10 тайлах дают угловую ошибку в 8 юнитов — ровно
/// размер текселя маски, а пересчёт идёт на каждый шаг игрока.
const FOV_BINS: usize = 256;
/// Досягаемость окклюдеров для карты глаза в тайлах: диагональ видимой области
/// (13.4, 7.5) ≈ 15.4, берём с запасом. Меньше отрезков — быстрее пересчёт.
const FOV_REACH_TILES: f32 = 16.0;
/// Ограничение стороны светового поля в тайлах: у импортированных карт
/// (aspid и т.п.) карта огромная, а поле света полуэкранное и меньше.
const FIELD_MAX_TILES: i32 = 128;

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
    /// Сумма вкладов ДО просачивания на стены: в неё применяются дельты ламп.
    sum_field: Vec<f32>,
    tint_field: Vec<[f32; 3]>,
    /// Вклады ламп по отдельности — чтобы дверь пересчитывала только соседние.
    contrib: HashMap<LampKey, LampContribution>,
    /// Подпись стен поля (`chunk_hash`): смена карты пересобирает вклады всех ламп.
    field_chunks: u64,
    field_origin: (i32, i32),
    field_dim: (u32, u32),
    field_valid: bool,
    /// Размер текселей слоёв тьмы/оттенка (по вьюпорту, меняется с окном).
    overlay_dim: (u32, u32),
}

impl LightMap {
    /// Сущности слоёв CPU-пути (тьма и оттенок ламп) — их скрывает
    /// `light_gpu::overlay::sync_light_overlay`, когда свет считает GPU.
    pub fn sprites(&self) -> [Entity; 2] {
        [self.sprite, self.glow_sprite]
    }
}

/// Отпечаток мира: при изменении карта пересобирается.
#[derive(Default, PartialEq, Clone)]
pub(crate) struct WorldState {
    player_tile: (i32, i32),
    lamps: Vec<(i32, i32, u8, u8, [u8; 3])>, // тайл, радиус, яркость×100, цвет
    closed_doors: Vec<(i32, i32)>,
    chunks: u64, // хеш набора чанков (репликация карты могла подгрузить новый)
}

/// Ключ лампы: тайл, радиус, яркость×100 и цвет — вклад лампы зависит только
/// от этих параметров и её окружения.
pub type LampKey = (i32, i32, u8, u8, [u8; 3]);

/// Вклад одной лампы в световое поле: прямоугольник текселей и значения.
/// Хранится, чтобы при изменении окружения (дверь рядом) пересчитывать только
/// эту лампу, а не всё поле.
struct LampContribution {
    /// Прямоугольник в текселях поля: x0, y0, ширина, высота.
    rect: (u32, u32, u32, u32),
    light: Vec<f32>,
    tint: Vec<[f32; 3]>,
    /// Подпись закрытых дверей в радиусе влияния лампы.
    doors: u64,
}

/// Прибавляет (`sign = 1`) или вычитает (`-1`) световой вклад лампы.
/// Отдельно от оттенка: поле и оттенок — разные поля `ResMut`-ресурса, и два
/// изменяемых заимствования сразу Rust не пропускает.
fn apply_light(field: &mut [f32], dim_x: usize, contribution: &LampContribution, sign: f32) {
    let (x0, y0, width, height) = contribution.rect;
    for row in 0..height as usize {
        let dst_row = (y0 as usize + row) * dim_x;
        let src_row = row * width as usize;
        for col in 0..width as usize {
            field[dst_row + x0 as usize + col] += sign * contribution.light[src_row + col];
        }
    }
}

/// То же для оттенка лампы (используется слоем свечения).
fn apply_tint(tint: &mut [[f32; 3]], dim_x: usize, contribution: &LampContribution, sign: f32) {
    let (x0, y0, width, height) = contribution.rect;
    for row in 0..height as usize {
        let dst_row = (y0 as usize + row) * dim_x;
        let src_row = row * width as usize;
        for col in 0..width as usize {
            let dst = dst_row + x0 as usize + col;
            for (channel, slot) in tint[dst].iter_mut().enumerate() {
                *slot += sign * contribution.tint[src_row + col][channel];
            }
        }
    }
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
///
/// Камера и глаз обновляются каждый кадр (они едут за игроком), а геометрия
/// (стены и источники) — только при изменении мира: `generation` растёт на
/// каждую пересборку, и рендер-мир по ней решает, перезаливать ли буферы.
#[derive(Resource, Default)]
pub struct LightScene {
    pub walls: Vec<ssr_core::occluders::OccluderSegment>,
    pub lights: Vec<GpuLight>,
    /// Прямоугольники сплошных тайлов (стены и закрытые двери) в мировых
    /// единицах: `(x0, y0, x1, y1)`. Нужны GPU-пути как аналог стенсила движка
    /// (`ApplyLightingFovToBuffer`): на стенах маска видимости НЕ гасит свет,
    /// поэтому стены остаются видны (просачивание, `wall-bleed-blur`).
    pub wall_tiles: Vec<[f32; 4]>,
    /// Левый-нижний угол видимой области мира (кадр 21×15 тайлов, как у камеры).
    pub camera: (f32, f32),
    /// Размер видимой области в мировых единицах (672×480).
    pub viewport: (f32, f32),
    /// Позиция глаза (свой игрок) — центр карты FOV.
    pub eye: (f32, f32),
    /// Подпись геометрии: 0 — сцена ещё не собиралась.
    pub generation: u64,
}

/// Создаёт картинки карты света и спрайты.
/// Пустая текстура слоя карты света (тьма или оттенок ламп): RGBA8, линейный
/// сэмплер — край тени интерполируется, а не квантуется текселями.
fn layer_image(dim: (u32, u32)) -> Image {
    let mut image = Image::new(
        Extent3d {
            width: dim.0,
            height: dim.1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; (dim.0 * dim.1 * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

pub fn setup_lighting(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    // Стартовый размер — квадрат окна по умолчанию; по первому кадру система
    // света пересоздаст текстуры под реальный вьюпорт (`overlay_dim`).
    let dim = (LIGHT_DIM, LIGHT_DIM);
    let image = images.add(layer_image(dim));
    let glow_image = images.add(layer_image(dim));
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
        sum_field: Vec::new(),
        tint_field: Vec::new(),
        contrib: HashMap::new(),
        field_chunks: 0,
        field_origin: (0, 0),
        field_dim: (0, 0),
        field_valid: false,
        overlay_dim: dim,
    });
}

/// Затухание света из `light_shared.swsl`: `s = clamp(sqrt(d²+1)/radius)`,
/// `val = (1-s²)² / (1 + falloff·s)`.
#[allow(clippy::too_many_arguments)]
pub fn update_lighting(
    mut commands: Commands,
    mut state: Local<WorldState>,
    own: Res<OwnPlayerEntity>,
    windows: Query<&Window>,
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
    // Аспект окна: видимая область мира — 15 тайлов в высоту, по ширине окно
    // (у широких мониторов видно больше тайлов). Нужен и карте света (окно
    // карты по вьюпорту), и сцене GPU-конвейера (та же область).
    let aspect = windows
        .iter()
        .next()
        .map(|window| (window.width() / window.height().max(1.0)).clamp(1.0, 4.0))
        .unwrap_or(16.0 / 9.0);

    // Спрайты центрируем каждый кадр (дёшево), карту — по изменениям.
    // При GPU-пути слой тьмы рисует квад конвейера, спрайты CPU скрыты.
    let center_x = (player_tile.0 as f32 + 0.5) * TILE_UNITS;
    let center_y = (player_tile.1 as f32 + 0.5) * TILE_UNITS;
    let gpu = crate::light_gpu::overlay::gpu_lighting();
    if !gpu {
        for sprite in [map.sprite, map.glow_sprite] {
            if let Ok(mut transform) = transforms.get_mut(sprite) {
                transform.translation.x = center_x;
                transform.translation.y = center_y;
            }
        }
    }

    // Отпечаток мира: если ничего не изменилось — пересчёта нет.
    let lamp_state: Vec<(i32, i32, u8, u8, [u8; 3])> = lamps
        .iter()
        .filter(|(_, _, powered)| !powered.is_some_and(|powered| !powered.0))
        .map(|(light, position, _)| {
            (
                (position.0[0] / TILE_UNITS).floor() as i32,
                (position.0[1] / TILE_UNITS).floor() as i32,
                light.radius.round().clamp(1.0, 30.0) as u8,
                (light.energy * 100.0).clamp(0.0, 255.0) as u8,
                light.color,
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
    // Сцена для GPU-конвейера: камера и глаз едут за игроком каждый кадр,
    // окклюдеры и источники пересобираются только при изменении мира
    // (построение окклюдеров — проход по всем тайлам чанков, в кадре ему не место).
    if let Some(mut scene) = scene {
        if geometry_changed || scene.generation == 0 {
            let chunk_refs: Vec<&TileChunkData> = chunks.iter().collect();
            scene.walls = ssr_core::occluders::build_occluders(&chunk_refs, &closed_doors);
            // Сплошные тайлы — для маски стен на GPU (стенсил движка).
            let mut wall_tiles: Vec<[f32; 4]> = Vec::new();
            for chunk in chunks.iter() {
                let base = (
                    chunk.coords.0 * CHUNK_TILES as i32,
                    chunk.coords.1 * CHUNK_TILES as i32,
                );
                for ly in 0..CHUNK_TILES {
                    for lx in 0..CHUNK_TILES {
                        let tx = base.0 + lx as i32;
                        let ty = base.1 + ly as i32;
                        if !ssr_core::occluders::is_solid(chunk.get_local(lx, ly))
                            && !closed_doors.contains(&(tx, ty))
                        {
                            continue;
                        }
                        let (x0, y0) = (tx as f32 * TILE_UNITS, ty as f32 * TILE_UNITS);
                        wall_tiles.push([x0, y0, x0 + TILE_UNITS, y0 + TILE_UNITS]);
                    }
                }
            }
            scene.wall_tiles = wall_tiles;
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
            scene.generation = scene.generation.wrapping_add(1);
        }
        let center = (player_position.0[0], player_position.0[1]);
        // Видимая область — как у камеры: 15 тайлов в высоту, по ширине окно
        // (у 16:9 это ~26.7 тайла, а не 21 из `VIEWPORT_TILES`): карта света
        // конвейера обязана накрывать ВЕСЬ кадр, иначе по краям экрана свет
        // пропадёт.
        let viewport = (
            (VIEWPORT_TILES.1 * aspect).ceil() * TILE_UNITS,
            VIEWPORT_TILES.1 * TILE_UNITS,
        );
        scene.camera = (center.0 - viewport.0 * 0.5, center.1 - viewport.1 * 0.5);
        scene.viewport = viewport;
        scene.eye = center;
    }
    if !tile_changed && !geometry_changed && map.ready {
        return;
    }
    // Пересчёт идёт СРАЗУ при смене тайла игрока: раньше здесь стоял троттл
    // 20 Гц, и карта отставала от спрайта — при каждом шаге весь свет сдвигался
    // на тайл и возвращался через 0.05 с («экран мигает после каждого шага»).
    *state = signature;
    map.ready = true;
    // GPU-путь считается в compute-шейдерах (`light_gpu`): CPU-карта тьмы и
    // оттенка не нужна (её слои скрыты). CPU-путь остаётся фолбэком.
    if gpu {
        return;
    }
    use ssr_core::light::{NO_OCCLUDER, attenuation, chebyshev_upper_bound, ray_segment_distance};
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
    let polar_map = |point: (f32, f32), reach: f32, bins: usize| -> Vec<f32> {
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
        let mut map = vec![NO_OCCLUDER; bins];
        for (bin, slot) in map.iter_mut().enumerate() {
            let angle = (bin as f32 / bins as f32) * 2.0 * PI - PI;
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
    let walls_ms = rebuild_started.elapsed().as_millis() as u64;

    // Чтение карты по направлению: линейная интерполяция бинов и заворот на шве
    // ±π (в движке `WrapMode.Repeat` при `Filter = true`). Индекс даёт
    // `polar_bin` — инверсия генерации (тест в ядре); формула движкового
    // `polar_u` (без деления на 2) вырождала полкарты в один бин — это и были
    // чёрные клинья «тумана войны» при живом свете вокруг.
    let sample_map = |map: &[f32], dx: f32, dy: f32, bins: usize| -> f32 {
        let u = ssr_core::light::polar_bin(dx, dy, bins);
        let base = u.floor();
        let t = u - base;
        let i0 = (base as isize).rem_euclid(bins as isize) as usize;
        let i1 = (base as isize + 1).rem_euclid(bins as isize) as usize;
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
        if sample_map(map, diff.0, diff.1, LAMP_BINS) >= len {
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
            let plus = sample_map(map, diff.0 + offset.0, diff.1 + offset.1, LAMP_BINS);
            if k == 0 {
                samples[0] = plus;
                mindist = mindist.min(plus);
            } else {
                let minus = sample_map(map, diff.0 - offset.0, diff.1 - offset.1, LAMP_BINS);
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
    let field_dim_x = (field_tiles_x * FIELD_SUBTEXELS as i32) as u32;
    let field_dim_y = (field_tiles_y * FIELD_SUBTEXELS as i32) as u32;
    let field_sub = FIELD_SUBTEXELS as f32;
    let field_step = 1.0 / field_sub;
    let fdim = field_dim_x as usize;
    let field_span = (
        field_origin.0 as f32,
        field_origin.1 as f32,
        (field_origin.0 + field_tiles_x) as f32,
        (field_origin.1 + field_tiles_y) as f32,
    );

    // === Световое поле через кэш вкладов по лампам ===
    // Раньше любое изменение геометрии (в том числе открытие двери, которое
    // меняет окклюдеры) пересчитывало ВСЁ поле: 47 ламп × 6400 текселей = 60 мс.
    // Теперь вклад каждой лампы хранится отдельно и пересчитывается, только
    // если рядом с ней изменились двери (или сменились сами стены): правка
    // поля — это вычесть старый вклад, прибавить новый и заново размыть стены.
    let field_geometry_ok = map.field_valid
        && map.field_origin == field_origin
        && map.field_dim == (field_dim_x, field_dim_y)
        && map.field_chunks == chunk_hash;
    if !field_geometry_ok {
        map.field_origin = field_origin;
        map.field_dim = (field_dim_x, field_dim_y);
        map.field_chunks = chunk_hash;
        map.field_valid = true;
        map.contrib.clear();
        map.field.clear();
        map.field
            .resize((field_dim_x * field_dim_y) as usize, AMBIENT);
        map.tint_field.clear();
        map.tint_field
            .resize((field_dim_x * field_dim_y) as usize, [0.0; 3]);
        map.sum_field.clear();
        map.sum_field
            .resize((field_dim_x * field_dim_y) as usize, AMBIENT);
    }
    // Подпись окружения лампы: только закрытые двери в радиусе её влияния
    // меняют её карту теней (стены учтены `chunk_hash`).
    let doors_signature = |lamp_x: i32, lamp_y: i32, radius: f32| -> u64 {
        let mut hash = 0u64;
        for (dx, dy) in &closed_doors {
            let reach = (lamp_x - dx).abs().max((lamp_y - dy).abs()) as f32;
            if reach <= radius + 2.0 {
                hash = hash
                    .wrapping_mul(1099511628211)
                    .wrapping_add((*dx as u64) << 32 | *dy as u64);
            }
        }
        hash
    };
    // Вклады ламп, которых больше нет на карте, вычитаем.
    let live: std::collections::HashSet<LampKey> = lamp_state.iter().copied().collect();
    let stale: Vec<LampKey> = map
        .contrib
        .keys()
        .copied()
        .filter(|key| !live.contains(key))
        .collect();
    for key in stale {
        if let Some(gone) = map.contrib.remove(&key) {
            apply_light(&mut map.sum_field, fdim, &gone, -1.0);
            apply_tint(&mut map.tint_field, fdim, &gone, -1.0);
        }
    }
    let mut recomputed_lamps = 0usize;
    for (lamp_x, lamp_y, radius_u8, energy_u8, color_u8) in lamp_state.iter() {
        let key: LampKey = (*lamp_x, *lamp_y, *radius_u8, *energy_u8, *color_u8);
        let radius = *radius_u8 as f32;
        let doors = doors_signature(*lamp_x, *lamp_y, radius);
        let valid = map.contrib.get(&key).is_some_and(|c| c.doors == doors);
        if valid {
            continue;
        }
        recomputed_lamps += 1;
        // Карта теней лампы строится только здесь и живёт в кэше вклада.
        let polar = polar_map(
            (*lamp_x as f32 + 0.5, *lamp_y as f32 + 0.5),
            radius,
            LAMP_BINS,
        );
        if let Some(old) = map.contrib.remove(&key) {
            apply_light(&mut map.sum_field, fdim, &old, -1.0);
            apply_tint(&mut map.tint_field, fdim, &old, -1.0);
        }
        let center = (*lamp_x as f32 + 0.5, *lamp_y as f32 + 0.5);
        let energy = *energy_u8 as f32 / 100.0;
        // Движок берёт цвет лампы из прототипа и использует hex как линейный
        // множитель (без sRGB→линейного преобразования) — как `PointLight.color`.
        let color = [
            color_u8[0] as f32 / 255.0,
            color_u8[1] as f32 / 255.0,
            color_u8[2] as f32 / 255.0,
        ];
        // Прямоугольник вклада: радиус лампы, обрезанный полем. Лампа, чей
        // радиус не дотягивается до поля, не освещает ни один тексель.
        if center.0 + radius < field_span.0
            || center.0 - radius > field_span.2
            || center.1 + radius < field_span.1
            || center.1 - radius > field_span.3
        {
            map.contrib.insert(
                key,
                LampContribution {
                    rect: (0, 0, 0, 0),
                    light: Vec::new(),
                    tint: Vec::new(),
                    doors,
                },
            );
            continue;
        }
        let min_x = (((center.0 - radius) - field_span.0) * field_sub)
            .floor()
            .max(0.0) as u32;
        let max_x = ((((center.0 + radius) - field_span.0) * field_sub).ceil())
            .min(field_dim_x as f32) as u32;
        let min_y = (((center.1 - radius) - field_span.1) * field_sub)
            .floor()
            .max(0.0) as u32;
        let max_y = ((((center.1 + radius) - field_span.1) * field_sub).ceil())
            .min(field_dim_y as f32) as u32;
        let width = max_x.saturating_sub(min_x);
        let height = max_y.saturating_sub(min_y);
        let mut light = vec![0.0f32; (width * height) as usize];
        let mut tint = vec![[0.0f32; 3]; (width * height) as usize];
        for ty in min_y..max_y {
            for tx in min_x..max_x {
                let px = field_span.0 + (tx as f32 + 0.5) * field_step;
                let py = field_span.1 + (ty as f32 + 0.5) * field_step;
                let diff = (px - center.0, py - center.1);
                let dist2 = diff.0 * diff.0 + diff.1 * diff.1;
                if dist2 > radius * radius {
                    continue;
                }
                let occlusion = soft_shadow(&polar, diff);
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
                let index = ((ty - min_y) as usize * width as usize) + (tx - min_x) as usize;
                light[index] = value;
                for (channel, tint) in tint[index].iter_mut().enumerate() {
                    *tint = color[channel] * value;
                }
            }
        }
        let contribution = LampContribution {
            rect: (min_x, min_y, width, height),
            light,
            tint,
            doors,
        };
        apply_light(&mut map.sum_field, fdim, &contribution, 1.0);
        apply_tint(&mut map.tint_field, fdim, &contribution, 1.0);
        map.contrib.insert(key, contribution);
    }
    if !field_geometry_ok || recomputed_lamps > 0 {
        // Сумма вкладов → в поле; просачивание на стены (`wall-bleed` в движке)
        // считаем в копии: стена берёт максимум света соседей, иначе была бы чёрной.
        let sum = std::mem::take(&mut map.sum_field);
        let mut bleed = sum.clone();
        for ty in 1..field_dim_y - 1 {
            for tx in 1..field_dim_x - 1 {
                let tile = (
                    field_origin.0 + (tx / FIELD_SUBTEXELS) as i32,
                    field_origin.1 + (ty / FIELD_SUBTEXELS) as i32,
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
        map.field.copy_from_slice(&bleed);
        map.sum_field = sum;
    }
    let maps_ms = rebuild_started.elapsed().as_millis() as u64;

    // === Оверлей окна игрока: маска видимости × готовое поле света ===
    // Окно — прямоугольник по вьюпорту (в движке кадр 21×15 тайлов), а не
    // квадрат 33×33: пересчёт идёт на КАЖДЫЙ шаг игрока, а квадрат считал
    // втрое больше текселей, чем видно на экране. Размер берётся по вьюпорту
    // (у широких мониторов он шире), с полями в пару тайлов. Аспект посчитан
    // выше — он же задаёт видимую область в сцене GPU-конвейера.
    let overlay_tiles = (
        // Чётные размеры: центр текстуры попадает ровно на тайл игрока.
        (((VIEWPORT_TILES.1 * aspect).ceil() as i32 + 3) & !1).max(8),
        ((VIEWPORT_TILES.1 as i32 + 3) & !1).max(8),
    );
    let overlay_dim = (
        overlay_tiles.0 as u32 * LIGHT_SUBTEXELS,
        overlay_tiles.1 as u32 * LIGHT_SUBTEXELS,
    );
    if map.overlay_dim != overlay_dim {
        // Разрешение/аспект окна изменились — пересоздаём текстуры слоёв.
        map.overlay_dim = overlay_dim;
        map.image = images.add(layer_image(overlay_dim));
        map.glow_image = images.add(layer_image(overlay_dim));
        commands
            .entity(map.sprite)
            .insert(Sprite::from_image(map.image.clone()));
        commands
            .entity(map.glow_sprite)
            .insert(Sprite::from_image(map.glow_image.clone()));
    }
    let (dim_x, dim_y) = overlay_dim;
    let (side_x, side_y) = overlay_tiles;
    let origin = (player_tile.0 - side_x / 2, player_tile.1 - side_y / 2);

    // Сетка стен окна — для просачивания видимости и диагностического дампа.
    let mut grid = vec![false; (side_x * side_y) as usize];
    for ly in 0..side_y {
        for lx in 0..side_x {
            let (tx, ty) = (origin.0 + lx, origin.1 + ly);
            grid[(ly * side_x + lx) as usize] = solid_tile(tx, ty);
        }
    }
    let wall_at = |lx: i32, ly: i32| -> bool {
        if lx < 0 || ly < 0 || lx >= side_x || ly >= side_y {
            return true;
        }
        grid[(ly * side_x + lx) as usize]
    };

    // Видимость глаза — ровно как `fov-lighting.swsl`: ОДНА выборка полярной
    // карты по направлению на точку + Chebyshev-тест (`0.25` в моменте).
    // Мягкая 7-точечная тень применяется только к СВЕТУ источников: у маски
    // глаза она давала «слепые зоны» в глубине коридоров (боковые стены
    // затеняли точку, до которой луч доходит свободно).
    // Бинов для глаза меньше, чем у карт теней (2048 в движке считает GPU):
    // шаг 256 бинов при 10 тайлах даёт ошибку в 8 юнитов — ровно тексель маски.
    let eye_map = polar_map(
        (player_tile.0 as f32 + 0.5, player_tile.1 as f32 + 0.5),
        FOV_REACH_TILES,
        FOV_BINS,
    );
    // Маска считается вдвое грубее текселей оверлея (`MASK_SUBTEXELS`) и
    // потом интерполируется: маска глаза мягкая, а пересчёт идёт на КАЖДЫЙ шаг
    // игрока — на полном разрешении это 4 мс на шаг.
    const MASK_SUBTEXELS: u32 = LIGHT_SUBTEXELS / 2;
    let mask_dim_x = side_x as u32 * MASK_SUBTEXELS;
    let mask_dim_y = side_y as u32 * MASK_SUBTEXELS;
    let mask_step = 1.0 / MASK_SUBTEXELS as f32;
    let mut visible = vec![0.0f32; (mask_dim_x * mask_dim_y) as usize];
    for ty in 0..mask_dim_y {
        for tx in 0..mask_dim_x {
            let px = origin.0 as f32 + (tx as f32 + 0.5) * mask_step;
            let py = origin.1 as f32 + (ty as f32 + 0.5) * mask_step;
            let diff = (
                px - (player_tile.0 as f32 + 0.5),
                py - (player_tile.1 as f32 + 0.5),
            );
            let len = (diff.0 * diff.0 + diff.1 * diff.1).sqrt();
            let wall = sample_map(&eye_map, diff.0, diff.1, FOV_BINS);
            visible[(ty * mask_dim_x + tx) as usize] = chebyshev_upper_bound(moment(wall), len);
        }
    }
    // Стена берёт максимум видимости соседей: луч в центр тайла упирается в саму
    // стену, поэтому без просачивания её texel был бы чёрным (в движке стены
    // видно по `wall-bleed`).
    for ty in 1..mask_dim_y - 1 {
        for tx in 1..mask_dim_x - 1 {
            let tile = ((tx / MASK_SUBTEXELS) as i32, (ty / MASK_SUBTEXELS) as i32);
            if !wall_at(tile.0, tile.1) {
                continue;
            }
            let index = (ty * mask_dim_x + tx) as usize;
            let mut visible_best = visible[index];
            for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
                let neighbor =
                    (((ty as i32 + dy) as u32) * mask_dim_x + ((tx as i32 + dx) as u32)) as usize;
                visible_best = visible_best.max(visible[neighbor]);
            }
            visible[index] = visible_best;
        }
    }
    // Апскейл маски в тексели оверлея. Маска ровно вдвое грубее оверлея и
    // начинается с того же тайла, поэтому центр текселя оверлея всегда лежит
    // между четырьмя текселями маски со смещением 0.25: веса постоянны, и
    // вместо floor/clamp на каждый тексель хватает сдвига индекса.
    const _: () = assert!(LIGHT_SUBTEXELS == MASK_SUBTEXELS * 2);
    let mask_dim_x_i = mask_dim_x as i32 - 1;
    let mask_dim_y_i = mask_dim_y as i32 - 1;
    let visible_at = |lx: u32, ly: u32| -> f32 {
        let x0 = ((lx >> 1) as i32).min(mask_dim_x_i).max(0) as usize;
        let y0 = ((ly >> 1) as i32).min(mask_dim_y_i).max(0) as usize;
        let x1 = ((x0 + 1) as i32).min(mask_dim_x_i) as usize;
        let y1 = ((y0 + 1) as i32).min(mask_dim_y_i) as usize;
        let tx = if lx & 1 == 0 { 0.25 } else { 0.75 };
        let ty = if ly & 1 == 0 { 0.25 } else { 0.75 };
        let md = mask_dim_x as usize;
        let top = visible[y0 * md + x0] * (1.0 - tx) + visible[y0 * md + x1] * tx;
        let bottom = visible[y1 * md + x0] * (1.0 - tx) + visible[y1 * md + x1] * tx;
        top * (1.0 - ty) + bottom * ty
    };
    let mask_ms = rebuild_started.elapsed().as_millis() as u64;

    // Свет для текселя окна — билинейно из поля: поле вдвое грубее окна
    // (`FIELD_SUBTEXELS` против `LIGHT_SUBTEXELS`), и интерполяция даёт мягкий
    // край тени вместо ступенек по текселям поля.
    // Билинейная выборка поля: поле ровно вдвое грубее оверлея (4 текселя на
    // тайл против 8), сетки выровнены по тайлам — значит центр текселя оверлея
    // всегда попадает между четырьмя текселями поля со смещением 0.25, веса
    // постоянны, и floor/clamp на каждый тексель не нужен (раньше это было
    // 2-3 мс на пересчёт, а пересчёт идёт на каждый шаг игрока).
    const _: () = assert!(LIGHT_SUBTEXELS == FIELD_SUBTEXELS * 2);
    let field_offset_x = (origin.0 - map.field_origin.0) * FIELD_SUBTEXELS as i32;
    let field_offset_y = (origin.1 - map.field_origin.1) * FIELD_SUBTEXELS as i32;
    let fdim = field_dim_x as usize;
    let field_dim_x_i = field_dim_x as i32 - 1;
    let field_dim_y_i = field_dim_y as i32 - 1;
    let field_coords = move |lx: u32, ly: u32| -> (usize, usize, usize, usize, f32, f32) {
        let x0 = (field_offset_x + (lx >> 1) as i32).clamp(0, field_dim_x_i) as usize;
        let y0 = (field_offset_y + (ly >> 1) as i32).clamp(0, field_dim_y_i) as usize;
        let x1 = ((x0 + 1) as i32).min(field_dim_x_i) as usize;
        let y1 = ((y0 + 1) as i32).min(field_dim_y_i) as usize;
        (
            x0,
            x1,
            y0,
            y1,
            if lx & 1 == 0 { 0.25 } else { 0.75 },
            if ly & 1 == 0 { 0.25 } else { 0.75 },
        )
    };
    let field_luminance = |lx: u32, ly: u32| -> f32 {
        let (x0, x1, y0, y1, tx, ty) = field_coords(lx, ly);
        let top = map.field[y0 * fdim + x0] * (1.0 - tx) + map.field[y0 * fdim + x1] * tx;
        let bottom = map.field[y1 * fdim + x0] * (1.0 - tx) + map.field[y1 * fdim + x1] * tx;
        top * (1.0 - ty) + bottom * ty
    };
    let field_tint = |lx: u32, ly: u32| -> [f32; 3] {
        let (x0, x1, y0, y1, tx, ty) = field_coords(lx, ly);
        let mut rgb = [0.0f32; 3];
        let (i00, i01, i10, i11) = (
            y0 * fdim + x0,
            y0 * fdim + x1,
            y1 * fdim + x0,
            y1 * fdim + x1,
        );
        for (channel, slot) in rgb.iter_mut().enumerate() {
            let top = map.tint_field[i00][channel] * (1.0 - tx) + map.tint_field[i01][channel] * tx;
            let bottom =
                map.tint_field[i10][channel] * (1.0 - tx) + map.tint_field[i11][channel] * tx;
            *slot = top * (1.0 - ty) + bottom * ty;
        }
        rgb
    };

    // Диагностика `SSR_LIGHT_DEBUG=1`: сетка окна и средняя тьма по тайлам —
    // так видно, ЧТО именно затемнено: стена (законно) или «слепая зона».
    if std::env::var_os("SSR_LIGHT_DEBUG").is_some() {
        let mut grid_art = String::from("\n");
        for ly in (0..side_y).rev() {
            for lx in 0..side_x {
                grid_art.push(if (lx, ly) == (side_x / 2, side_y / 2) {
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
        for ly in (0..side_y).rev() {
            for lx in 0..side_x {
                let mut sum = 0.0f32;
                for sy in 0..LIGHT_SUBTEXELS {
                    for sx in 0..LIGHT_SUBTEXELS {
                        let tx = lx as u32 * LIGHT_SUBTEXELS + sx;
                        let ty = ly as u32 * LIGHT_SUBTEXELS + sy;
                        sum += 1.0 - field_luminance(tx, ty) * visible_at(tx, ty);
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
        for ly in 0..dim_y {
            for lx in 0..dim_x {
                let luminance = field_luminance(lx, ly);
                let occlusion = 1.0 - luminance * visible_at(lx, ly);
                let row = dim_y - 1 - ly;
                let target = ((row * dim_x + lx) * 4) as usize;
                data[target + 3] = (occlusion.clamp(0.0, 1.0) * 255.0) as u8;
            }
        }
    }
    if let Some(mut glow_image) = images.get_mut(&map.glow_image)
        && let Some(glow_data) = glow_image.data.as_mut()
    {
        glow_data.fill(0);
        for ly in 0..dim_y {
            for lx in 0..dim_x {
                let contribution = (field_luminance(lx, ly) - AMBIENT).max(0.0);
                if contribution < 0.001 {
                    continue;
                }
                let mask = visible_at(lx, ly);
                if mask <= 0.0 {
                    continue;
                }
                let rgb = field_tint(lx, ly);
                let norm = rgb[0].max(rgb[1]).max(rgb[2]).max(0.001);
                let row = dim_y - 1 - ly;
                let target = ((row * dim_x + lx) * 4) as usize;
                glow_data[target] = ((rgb[0] / norm) * 255.0) as u8;
                glow_data[target + 1] = ((rgb[1] / norm) * 255.0) as u8;
                glow_data[target + 2] = ((rgb[2] / norm) * 255.0) as u8;
                glow_data[target + 3] = ((contribution * mask * GLOW_ALPHA).min(1.0) * 255.0) as u8;
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
