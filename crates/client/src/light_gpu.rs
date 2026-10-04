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
//! Дальше по плану: загрузка `LightScene` в сторадж-буферы, `ComputePipeline`
//! в `RenderSystems::Prepare` и цепочка проходов системой в `Core2d` перед
//! основным проходом (`tень → FOV → свет → блюр ×3 → стены → маска FOV`).

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
pub const LIGHT_MAP_SIZE: (u32, u32) = (960, 540);

/// Текстуры конвейера света. Пока поля читает только следующий шаг плана
/// (проходы compute), поэтому предупреждение о неиспользуемых полях снимаем.
#[allow(dead_code)]
#[derive(Resource)]
pub struct LightGpu {
    /// Полярная карта теней: `MAX_SHADOW_LIGHTS` строк по `SHADOW_BINS`
    /// угловых бинов, `Rg32Float` — моменты `(d, d²)` для VSM.
    pub shadow_map: Handle<Image>,
    /// Полярная карта FOV от глаза игрока, `R32Float` — расстояние до стены.
    pub fov_map: Handle<Image>,
    /// Карта света: rgb — накопленный свет (аддитивные источники + ambient),
    /// альфа — прозрачность тьмы для оверлея (`1 − свет`).
    pub light_a: Handle<Image>,
    /// Вторая карта света — приёмник пинг-понга при блюре и маске FOV.
    pub light_b: Handle<Image>,
}

/// Создаёт текстуры конвейера (один раз на старте).
pub fn setup_light_gpu(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let shadow_map = images.add(polar_texture(
        SHADOW_BINS,
        MAX_SHADOW_LIGHTS,
        TextureFormat::Rg32Float,
    ));
    let fov_map = images.add(polar_texture(FOV_BINS, 1, TextureFormat::R32Float));
    let light_a = images.add(light_texture());
    let light_b = images.add(light_texture());
    commands.insert_resource(LightGpu {
        shadow_map,
        fov_map,
        light_a,
        light_b,
    });
    tracing::info!(
        bins = SHADOW_BINS,
        lights = MAX_SHADOW_LIGHTS,
        map = ?LIGHT_MAP_SIZE,
        "light gpu: текстуры конвейера созданы"
    );
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LightParams {
    pub tile: f32,
    /// Выравнивание `camera` до 8 байт (правило раскладки WGSL).
    pub _pad0: f32,
    pub camera: [f32; 2],
    pub light_count: u32,
    pub wall_count: u32,
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuWallUniform {
    pub ab: [f32; 4],
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Раскладка униформы обязана совпадать с WGSL (`Params` в light.wgsl):
    /// vec2 — по 8 байт, итого 72 байта.
    #[test]
    fn params_layout_matches_wgsl() {
        assert_eq!(std::mem::size_of::<LightParams>(), 72, "размер Params");
        assert_eq!(std::mem::offset_of!(LightParams, camera), 8);
        assert_eq!(std::mem::offset_of!(LightParams, light_count), 16);
        assert_eq!(std::mem::offset_of!(LightParams, wall_count), 20);
        assert_eq!(std::mem::offset_of!(LightParams, map_size), 24);
        assert_eq!(std::mem::offset_of!(LightParams, viewport), 32);
        assert_eq!(std::mem::offset_of!(LightParams, eye), 40);
        assert_eq!(std::mem::offset_of!(LightParams, fov_range), 48);
        assert_eq!(std::mem::offset_of!(LightParams, blur_boost), 60);
        assert_eq!(std::mem::offset_of!(LightParams, blur_dir), 64);
    }

    /// Источник и отрезок — массивы по 48 и 16 байт (`vec4<f32>` в WGSL).
    #[test]
    fn scene_uniforms_match_wgsl() {
        assert_eq!(std::mem::size_of::<GpuLightUniform>(), 48);
        assert_eq!(std::mem::size_of::<GpuWallUniform>(), 16);
    }
}
