//! GPU-конвейер света по модели движка (план §7 в `SS14_LIGHTING.md`).
//!
//! Здесь создаются текстуры конвейера: полярная карта теней (строка на
//! источник, моменты VSM), полярная карта FOV от глаза и карта света
//! половинного разрешения в двух экземплярах для пинг-понга (одну и ту же
//! текстуру нельзя одновременно читать и писать в compute-проходе).
//!
//! Все текстуры помечены `STORAGE_BINDING`, чтобы их писал compute-шейдер
//! (`assets/shaders/light.wgsl`), и `TEXTURE_BINDING` — чтобы карту света
//! семплил спрайт-оверлей тьмы. `Rgba8UnormSrgb` для storage использовать
//! нельзя (формат не поддерживает запись) — берём линейный `Rgba8Unorm`.
//!
//! Проводка (PORT_PLAN 1.1): сцена (`LightScene`) упаковывается в сторадж-буферы
//! системой `prepare::update_light_buffers` в `RenderSystems::PrepareResources`,
//! пайплайны ставятся в очередь один раз там же, а цепочка compute-проходов
//! (`тень → FOV → свет → блюр ×2 → просачивание на стены → маска FOV`) идёт
//! системой `pass::light_gpu_pass` в расписании `Core2d` перед основным проходом
//! камеры (`Core2dSystems::Prepass`). Оверлей мира — квад с материалом
//! `overlay::LightOverlayMaterial` (умножение кадра на карту света, как
//! `COLOR * LIGHT` в движке); CPU-путь остаётся фолбэком под `SSR_LIGHT_CPU=1`.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};

/// Ширина полярной карты теней — `ShadowMapSize` в движке.
pub const SHADOW_BINS: u32 = 512;
/// Ширина полярной карты FOV — `FovMapSize` в движке.
pub const FOV_BINS: u32 = 2048;
/// Сколько источников получает карту теней: в движке лимит 128
/// (`light.max_shadowcasting_lights`), у нас хватает 32.
pub const MAX_SHADOW_LIGHTS: u32 = 32;
/// Масштаб карты света — `light.resolution_scale` (0.5) в движке.
#[allow(dead_code)]
pub const LIGHT_MAP_SCALE: f32 = 0.5;
/// Базовый размер карты света (половина кадра 1920×1080). Пока фиксирован:
/// сначала сверяем картинку, потом подгоним под размер окна.
/// Размер карты света. В движке таргет света — `light.resolution_scale = 0.5`
/// от РАЗМЕРА ОКНА (`Clyde.LightRendering`), то есть на окне 2880×1620 это
/// 1440×810. У нас было 960×540: тексель карты покрывал ~3 экранных пикселя, и
/// края света/теней выглядели «квадратиками». 1440×810 даёт ~2 пикселя на
/// тексель (столько же, сколько в движке) при умеренной цене проходов.
pub const LIGHT_MAP_SIZE: (u32, u32) = (1440, 810);

/// Хэндлы четырёх текстур конвейера — их копия уезжает в рендер-мир
/// (`prepare::LightGpuState`), чтобы системы проходов не зависели от ресурса
/// основного мира целиком.
#[derive(Clone)]
pub struct LightSceneHandles {
    pub shadow_map: Handle<Image>,
    pub fov_map: Handle<Image>,
    /// Выход из первого тела стены по углу — hard FOV (`fov.swsl` движка).
    pub fov_far: Handle<Image>,
    pub light_a: Handle<Image>,
    pub light_b: Handle<Image>,
    /// Маска стен (аналог стенсила движка): 1 — тайл стены, свет не гасится.
    pub wall_mask: Handle<Image>,
}

/// Текстуры конвейера света: их пишут compute-проходы, а карту света семплит
/// оверлей мира.
#[derive(Resource)]
pub struct LightGpu {
    /// Полярная карта теней: `MAX_SHADOW_LIGHTS` строк по `SHADOW_BINS`
    /// угловых бинов, `Rg32Float` — моменты `(d, d²)` для VSM.
    pub shadow_map: Handle<Image>,
    /// Полярная карта FOV от глаза игрока, `R32Float` — расстояние до стены.
    pub fov_map: Handle<Image>,
    /// Полярная карта выхода из первого тела стены (`fov_far`): всё, что дальше
    /// неё, прячет hard FOV непрозрачным чёрным (`fov.swsl` при
    /// `DrawHardFov = true`) — стена за стеной в SS14 не видна.
    pub fov_far: Handle<Image>,
    /// Карта света: rgb — накопленный свет (аддитивные источники + ambient),
    /// альфа — прозрачность тьмы для оверлея (`1 − свет`).
    pub light_a: Handle<Image>,
    /// Вторая карта света — приёмник пинг-понга при блюре и маске FOV.
    pub light_b: Handle<Image>,
    /// Маска стен `R8Unorm` размером карты света: единица там, где тайл стены
    /// (или закрытая дверь). В движке эту роль играет стенсил
    /// (`ApplyLightingFovToBuffer`): на стенах FOV-маска не гасит свет.
    pub wall_mask: Handle<Image>,
}

/// Создаёт текстуры конвейера (один раз на старте).
pub fn setup_light_gpu(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let shadow_map = images.add(polar_texture(
        SHADOW_BINS,
        MAX_SHADOW_LIGHTS,
        TextureFormat::Rg32Float,
    ));
    let fov_map = images.add(polar_texture(FOV_BINS, 1, TextureFormat::R32Float));
    let fov_far = images.add(polar_texture(FOV_BINS, 1, TextureFormat::R32Float));
    let light_a = images.add(light_texture());
    let light_b = images.add(light_texture());
    let wall_mask = images.add(wall_mask_texture());
    commands.insert_resource(LightGpu {
        shadow_map,
        fov_map,
        fov_far,
        light_a,
        light_b,
        wall_mask,
    });
    tracing::info!(
        bins = SHADOW_BINS,
        lights = MAX_SHADOW_LIGHTS,
        map = ?LIGHT_MAP_SIZE,
        "light gpu: текстуры конвейера созданы"
    );
}

/// Маска стен: один канал, размер карты света. Формат `R8Unorm` — пишется
/// compute-шейдером (`wall_mask_cs`) и читается проходом маски FOV.
fn wall_mask_texture() -> Image {
    let (width, height) = LIGHT_MAP_SIZE;
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; (width * height) as usize],
        TextureFormat::R8Unorm,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage |= TextureUsages::STORAGE_BINDING;
    image.sampler = bevy::image::ImageSampler::nearest();
    image
}

/// Полярная текстура: 1 канал расстояний (FOV) или 2 канала моментов (тени).
fn polar_texture(width: u32, height: u32, format: TextureFormat) -> Image {
    let bytes_per_pixel = match format {
        TextureFormat::R32Float => 4,
        _ => 8, // Rg32Float
    };
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; (width * height) as usize * bytes_per_pixel],
        format,
        RenderAssetUsages::default(),
    );
    // Compute-шейдер пишет карту, проходы света её читают.
    image.texture_descriptor.usage |= TextureUsages::STORAGE_BINDING;
    // Полярная карта должна замыкаться по углу: шов на ±π сглаживается
    // линейной фильтрацией (в движке `WrapMode.Repeat, Filter = true`).
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

/// Карта света половинного разрешения: rgb — свет, a — тьма оверлея.
fn light_texture() -> Image {
    let (width, height) = LIGHT_MAP_SIZE;
    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![0; (width * height) as usize * 4],
        // Линейный формат: `Rgba8UnormSrgb` не поддерживает запись из шейдера.
        TextureFormat::Rgba8Unorm,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage |= TextureUsages::STORAGE_BINDING;
    // Карта растягивается на кадр — билинейная фильтрация даёт мягкие градиенты
    // света, как `Filter = true` у `LightRenderTarget` в движке.
    image.sampler = bevy::image::ImageSampler::linear();
    image
}

/// Униформа конвейера — раскладка 1:1 с `Params` в `assets/shaders/light.wgsl`.
///
/// WGSL выравнивает `vec2<f32>` по 8 байт, поэтому после `tile` стоит явный
/// паддинг, а порядок полей фиксирован. Тест `params_layout_matches_wgsl`
/// держит размер и смещения: без него Rust и шейдер разошлись бы молча
/// (шейдер читал бы чужие числа).
#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct LightParams {
    pub tile: f32,
    /// Выравнивание `camera` до 8 байт (правило раскладки WGSL).
    pub _pad0: f32,
    pub camera: [f32; 2],
    pub light_count: u32,
    pub wall_count: u32,
    pub wall_tile_count: u32,
    /// Выравнивание `map_size` до 8 байт (правило раскладки WGSL).
    pub _pad1: f32,
    pub map_size: [f32; 2],
    pub viewport: [f32; 2],
    pub eye: [f32; 2],
    pub fov_range: f32,
    pub ambient: f32,
    pub blur_radius: f32,
    pub blur_boost: f32,
    pub blur_dir: [f32; 2],
}

/// Источник света в раскладке `Light` из шейдера.
#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuLightUniform {
    /// (x, y, radius, energy) — как `data` в WGSL.
    pub data: [f32; 4],
    /// (falloff, curve, 0, 0) — как `params`.
    pub params: [f32; 4],
    /// (r, g, b, 0) — как `color`.
    pub color: [f32; 4],
}

/// Отрезок окклюдера в раскладке `Wall` из шейдера: (ax, ay, bx, by).
#[repr(C)]
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuWallUniform {
    pub ab: [f32; 4],
}

/// Тайл стены в раскладке `WallTile` из шейдера: (x0, y0, x1, y1) — мировой
/// прямоугольник тайла (нужен для маски стен, аналога стенсила движка).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuWallTileUniform {
    pub rect: [f32; 4],
}

/// Готовые к загрузке в GPU данные конвейера света: байты буферов стен и
/// источников плюс униформы проходов (источник — `crate::lighting::LightScene`,
/// которая заполняется при пересчёте карты света).
#[derive(Default)]
pub struct PackedScene {
    pub walls: Vec<u8>,
    pub lights: Vec<u8>,
    /// Мировые прямоугольники тайлов стен — для маски стен (стенсил движка).
    pub wall_tiles: Vec<u8>,
    /// Базовая униформа: тени, FOV, карта света, маска FOV.
    pub params: LightParams,
    /// Униформы блюра: `[H, V, H-просачивание, V-просачивание]` — направление и
    /// множитель (`wall-bleed-blur.swsl`: 1.1 при просачивании на стены).
    pub blur_params: [LightParams; 4],
}

/// Порядок униформ блюра в `PackedScene::blur_params`: два направления ×
/// (обычный блюр, просачивание на стены).
pub const BLUR_DIRECTIONS: [[f32; 2]; 4] = [[1.0, 0.0], [0.0, 1.0], [1.0, 0.0], [0.0, 1.0]];
/// Множитель яркости в блюре: `wall-bleed-blur.swsl` — 1.1, обычный блюр — 1.0.
pub const BLUR_BOOSTS: [f32; 4] = [1.0, 1.0, 1.1, 1.1];

/// Упаковывает сцену света в байты сторадж-буферов (раскладка — как в
/// `assets/shaders/light.wgsl`, за ней следит тест раскладки ниже).
pub fn pack_scene(scene: &crate::lighting::LightScene, tile: f32) -> PackedScene {
    let walls: Vec<GpuWallUniform> = scene
        .walls
        .iter()
        .map(|segment| GpuWallUniform {
            ab: [segment.a.0, segment.a.1, segment.b.0, segment.b.1],
        })
        .collect();
    let mut lights: Vec<GpuLightUniform> = scene
        .lights
        .iter()
        .map(|light| GpuLightUniform {
            data: [
                light.position.0,
                light.position.1,
                light.radius,
                light.energy,
            ],
            params: [light.falloff, light.curve, 0.0, 0.0],
            color: [light.color[0], light.color[1], light.color[2], 0.0],
        })
        .collect();
    // Карта теней имеет `MAX_SHADOW_LIGHTS` строк — источников берём не больше
    // (в движке тоже ограниченный набор «активных» источников:
    // `light.max_shadowcasting_lights`). Выбираем ближайшие к глазу, чтобы
    // набор не «прыгал» между кадрами при ходьбе.
    if lights.len() > MAX_SHADOW_LIGHTS as usize {
        let eye = scene.eye;
        lights.sort_by(|a, b| {
            let da = (a.data[0] - eye.0).powi(2) + (a.data[1] - eye.1).powi(2);
            let db = (b.data[0] - eye.0).powi(2) + (b.data[1] - eye.1).powi(2);
            da.total_cmp(&db)
        });
        lights.truncate(MAX_SHADOW_LIGHTS as usize);
    }
    let light_count = lights.len() as u32;
    let wall_count = walls.len() as u32;
    let wall_tiles: Vec<GpuWallTileUniform> = scene
        .wall_tiles
        .iter()
        .map(|rect| GpuWallTileUniform { rect: *rect })
        .collect();
    let base = LightParams {
        tile,
        _pad0: 0.0,
        camera: [scene.camera.0, scene.camera.1],
        light_count,
        wall_count,
        wall_tile_count: wall_tiles.len() as u32,
        _pad1: 0.0,
        map_size: [LIGHT_MAP_SIZE.0 as f32, LIGHT_MAP_SIZE.1 as f32],
        viewport: [scene.viewport.0, scene.viewport.1],
        eye: [scene.eye.0, scene.eye.1],
        // `fov_range` используется как флаг «FOV выключен» (`Eye.DrawFov = false`
        // у призрака): 1.0 — проход FOV не гасит свет.
        fov_range: if scene.no_fov { 1.0 } else { 0.0 },
        ambient: 0.082,
        blur_radius: 0.0,
        blur_boost: 1.0,
        blur_dir: [1.0, 0.0],
    };
    let blur_params = std::array::from_fn(|index| LightParams {
        blur_dir: BLUR_DIRECTIONS[index],
        blur_boost: BLUR_BOOSTS[index],
        ..base
    });
    PackedScene {
        walls: bytemuck::cast_slice(&walls).to_vec(),
        lights: bytemuck::cast_slice(&lights).to_vec(),
        wall_tiles: bytemuck::cast_slice(&wall_tiles).to_vec(),
        params: base,
        blur_params,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Раскладка униформы обязана совпадать с WGSL (`Params` в light.wgsl):
    /// vec2 — по 8 байт, итого 80 байт.
    #[test]
    fn params_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<LightParams>(), 80, "размер Params");
        assert_eq!(std::mem::offset_of!(LightParams, camera), 8);
        assert_eq!(std::mem::offset_of!(LightParams, light_count), 16);
        assert_eq!(std::mem::offset_of!(LightParams, wall_count), 20);
        assert_eq!(std::mem::offset_of!(LightParams, wall_tile_count), 24);
        assert_eq!(std::mem::offset_of!(LightParams, map_size), 32);
        assert_eq!(std::mem::offset_of!(LightParams, viewport), 40);
        assert_eq!(std::mem::offset_of!(LightParams, eye), 48);
        assert_eq!(std::mem::offset_of!(LightParams, fov_range), 56);
        assert_eq!(std::mem::offset_of!(LightParams, ambient), 60);
        assert_eq!(std::mem::offset_of!(LightParams, blur_boost), 68);
        assert_eq!(std::mem::offset_of!(LightParams, blur_dir), 72);
    }

    /// Упаковка сцены: размеры буферов соответствуют числу элементов, а
    /// униформа несёт число источников и стен (шейдер по ним идёт циклом).
    #[test]
    fn pack_scene_matches_element_sizes() {
        let mut scene = crate::lighting::LightScene::default();
        scene.walls.push(ssr_core::occluders::OccluderSegment {
            a: (1.0, 2.0),
            b: (3.0, 4.0),
        });
        scene.lights.push(crate::lighting::GpuLight {
            position: (10.0, 20.0),
            radius: 320.0,
            energy: 0.8,
            falloff: 6.8,
            curve: 0.0,
            color: [1.0, 0.78, 0.62],
        });
        let packed = pack_scene(&scene, 32.0);
        assert_eq!(packed.walls.len(), size_of::<GpuWallUniform>());
        assert_eq!(packed.lights.len(), size_of::<GpuLightUniform>());
        assert_eq!(packed.params.wall_count, 1);
        assert_eq!(packed.params.light_count, 1);
        assert_eq!(packed.params.tile, 32.0);
        assert_eq!(packed.params.camera, [0.0, 0.0]);
    }

    /// Тайлы стен упаковываются в свой буфер, а их число уезжает в униформу
    /// (по нему считается диспатч маски стен).
    #[test]
    fn pack_scene_packs_wall_tiles() {
        let mut scene = crate::lighting::LightScene::default();
        scene.wall_tiles.push([32.0, 64.0, 64.0, 96.0]);
        scene.wall_tiles.push([96.0, 64.0, 128.0, 96.0]);
        let packed = pack_scene(&scene, 32.0);
        assert_eq!(packed.params.wall_tile_count, 2);
        assert_eq!(packed.wall_tiles.len(), 2 * size_of::<GpuWallTileUniform>());
        let first: &GpuWallTileUniform =
            bytemuck::from_bytes(&packed.wall_tiles[..size_of::<GpuWallTileUniform>()]);
        assert_eq!(first.rect, [32.0, 64.0, 64.0, 96.0]);
    }

    /// Источников в буфере не больше строк карты теней (шейдер адресует строку
    /// карты индексом источника), и остаются ближайшие к глазу.
    #[test]
    fn pack_scene_caps_lights_to_shadow_map_rows() {
        // `eye` по умолчанию (0, 0) — от него считается близость источников.
        let mut scene = crate::lighting::LightScene::default();
        for index in 0..(MAX_SHADOW_LIGHTS as i32 + 7) {
            scene.lights.push(crate::lighting::GpuLight {
                // Ближе к глазу — меньший индекс: дальние должны отсечься.
                position: (index as f32 * 100.0, 0.0),
                radius: 320.0,
                energy: 1.0,
                falloff: 6.8,
                curve: 0.0,
                color: [1.0, 1.0, 1.0],
            });
        }
        let packed = pack_scene(&scene, 32.0);
        assert_eq!(packed.params.light_count, MAX_SHADOW_LIGHTS);
        assert_eq!(
            packed.lights.len(),
            MAX_SHADOW_LIGHTS as usize * size_of::<GpuLightUniform>()
        );
        let first: &GpuLightUniform =
            bytemuck::from_bytes(&packed.lights[..size_of::<GpuLightUniform>()]);
        assert_eq!(first.data[0], 0.0, "первым идёт ближайший к глазу источник");
    }

    /// Униформы блюра: направление H/V и множитель просачивания 1.1
    /// (`wall-bleed-blur.swsl`), остальные поля — из базовой униформы.
    #[test]
    fn blur_params_carry_direction_and_boost() {
        let scene = crate::lighting::LightScene {
            viewport: (672.0, 480.0),
            ..Default::default()
        };
        let packed = pack_scene(&scene, 32.0);
        assert_eq!(packed.blur_params[0].blur_dir, [1.0, 0.0]);
        assert_eq!(packed.blur_params[1].blur_dir, [0.0, 1.0]);
        assert_eq!(packed.blur_params[0].blur_boost, 1.0);
        assert_eq!(packed.blur_params[3].blur_boost, 1.1);
        assert_eq!(packed.blur_params[2].viewport, [672.0, 480.0]);
    }

    /// WGSL-шейдер конвейера обязан разбираться и проходить валидацию naga:
    /// ошибки шейдера иначе видны только при запуске клиента, когда Bevy
    /// создаёт compute-пайплайны (и то в логах).
    #[test]
    fn light_shader_parses_and_validates() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/shaders/light.wgsl");
        let source = std::fs::read_to_string(&path).expect("шейдер света читается");
        let module = naga::front::wgsl::parse_str(&source).expect("WGSL разбирается naga");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .expect("WGSL проходит валидацию");
    }

    /// Источник и отрезок — массивы по 48 и 16 байт (`vec4<f32>` в WGSL).
    #[test]
    fn scene_uniforms_match_wgsl() {
        assert_eq!(std::mem::size_of::<GpuLightUniform>(), 48);
        assert_eq!(std::mem::size_of::<GpuWallUniform>(), 16);
    }

    /// Карта видимости глаза считается в тех же единицах, что и стены: точка за
    /// ближайшей стеной скрыта, точка на полпути к ней — видна. Это ловит
    /// разъезд единиц (тайлы/мировые юниты) и ошибку чтения полярной карты —
    /// именно так появлялись «чёрные клинья» тумана войны.
    #[test]
    fn fov_mask_matches_wall_geometry_on_station_map() {
        let path = ssr_core::assets_root().join("maps/station.ron");
        let Ok(file) = ssr_core::tiles::MapFile::load(&path) else {
            return; // карты нет — тест пропускаем
        };
        let chunks: Vec<ssr_core::tiles::TileChunkData> = file
            .to_chunks()
            .expect("карта разбирается")
            .iter()
            .map(ssr_core::tiles::TileChunkData::from)
            .collect();
        let refs: Vec<&ssr_core::tiles::TileChunkData> = chunks.iter().collect();
        let walls = ssr_core::occluders::build_occluders(&refs, &[]);
        let spawn = file.spawn_points[0];
        let tile = ssr_core::tiles::TILE_PX as f32;
        // Карта FOV ровно как в шейдере: 2048 бинов, моменты VSM, Chebyshev.
        const BINS: usize = 2048;
        let mut fov = vec![ssr_core::light::NO_OCCLUDER; BINS];
        for (bin, slot) in fov.iter_mut().enumerate() {
            let angle =
                (bin as f32 / BINS as f32) * 2.0 * std::f32::consts::PI - std::f32::consts::PI;
            *slot = ssr_core::light::bin_distance(spawn, angle, walls.iter().map(|s| (s.a, s.b)));
        }
        assert!(
            fov.iter().any(|d| *d < ssr_core::light::NO_OCCLUDER),
            "ни одного окклюдера в карте видимости — стены не дошли"
        );
        // Что видно в направлении на юг (там ближайшая стена коридора).
        let occlusion = |dir: (f32, f32), dist: f32| -> f32 {
            let u = ssr_core::light::polar_bin(dir.0, dir.1, BINS);
            let bin = (u.round() as isize).clamp(0, BINS as isize - 1) as usize;
            let wall = fov[bin];
            ssr_core::light::chebyshev_upper_bound((wall, wall * wall + 0.25), dist)
        };
        let south = (0.0f32, -1.0f32);
        let wall = ssr_core::light::bin_distance(
            spawn,
            -std::f32::consts::FRAC_PI_2,
            walls.iter().map(|s| (s.a, s.b)),
        );
        assert!(wall < ssr_core::light::NO_OCCLUDER, "на юге нет стены");
        assert!(
            occlusion(south, wall * 0.5) > 0.9,
            "точка на полпути к стене обязана быть видна"
        );
        assert!(
            occlusion(south, wall + tile) < 0.5,
            "точка за стеной обязана быть скрыта"
        );
    }
}

/// Проводка в рендер-мир (PORT_PLAN 1.1): пайплайны, буферы сцены и цепочка
/// проходов; оверлей мира — материал-умножение карты света.
pub mod overlay;
pub mod pass;
pub mod prepare;

/// Подключает системы конвейера к рендер-миру: подготовка (лейауты, пайплайны,
/// буферы сцены) — в `RenderSystems::PrepareResources`, цепочка compute-проходов
/// — в расписание камеры `Core2d` набором `Core2dSystems::Prepass`, то есть перед
/// основным проходом мира (в движке это `BeforeRender`).
///
/// Вызывать после `DefaultPlugins`: расписание `Core2d` и под-приложение
/// `RenderApp` создаются ими.
pub fn install_render_systems(app: &mut App) {
    let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) else {
        tracing::warn!("light gpu: рендер-мир недоступен, конвейер света выключен");
        return;
    };
    render_app.add_systems(bevy::render::ExtractSchedule, prepare::extract_light);
    render_app.add_systems(
        bevy::render::Render,
        (
            prepare::prepare_light_pipelines,
            prepare::update_light_buffers,
        )
            .chain()
            .in_set(bevy::render::RenderSystems::PrepareResources),
    );
    render_app.add_systems(
        bevy::core_pipeline::Core2d,
        pass::light_gpu_pass.in_set(bevy::core_pipeline::Core2dSystems::Prepass),
    );
    tracing::info!("light gpu: системы рендера подключены");
}
