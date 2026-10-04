//! GPU-сторона конвейера света (PORT_PLAN 1.1, план §7 в `SS14_LIGHTING.md`):
//! лейауты и compute-пайплайны, буферы сцены (`walls`, `lights`, `Params`) и их
//! обновление из `LightScene` каждый кадр.
//!
//! Лейаутов пять: у каждой точки входа шейдера свой набор привязок — движок тоже
//! держит отдельный материал на проход. Одной общей группы не хватает: проход
//! теней ПИШЕТ карту теней, а проход карты света её ЧИТАЕТ, и держать запись и
//! чтение одной текстуры в одной группе нельзя.
//!
//! Униформ пять: базовая (тени, FOV, карта света, маска FOV) и четыре для блюра
//! (направление H/V × обычный блюр и просачивание на стены). Направление и
//! множитель — параметры прохода, а запись в буфер не может идти «между»
//! диспатчами одного энкодера, поэтому у каждого варианта свой буфер.

use bevy::asset::Handle;
use bevy::prelude::*;
use bevy::render::Extract;
use bevy::render::render_resource::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry,
    BindingResource, BindingType, Buffer, BufferBinding, BufferBindingType, BufferInitDescriptor,
    BufferSize, BufferUsages, CachedComputePipelineId, ComputePipelineDescriptor, PipelineCache,
    ShaderStages, StorageTextureAccess, TextureFormat, TextureSampleType, TextureViewDimension,
};
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::shader::Shader;
use std::borrow::Cow;

use super::{
    FOV_BINS, GpuLightUniform, GpuWallTileUniform, GpuWallUniform, LIGHT_MAP_SIZE, LightParams,
    LightSceneHandles, MAX_SHADOW_LIGHTS, SHADOW_BINS, pack_scene,
};

/// Минимальная ёмкость буфера стен в отрезках (`OccluderSegment`) — чтобы при
/// обычной станции буфер не пересоздавался на каждую открытую дверь.
const WALL_MIN_CAPACITY: usize = 8192;
/// Минимальная ёмкость буфера тайлов стен (маска стен) в тайлах.
const WALL_TILE_MIN_CAPACITY: usize = 4096;
/// Источников в буфере ровно столько, сколько строк у карты теней.
const LIGHT_CAPACITY: usize = MAX_SHADOW_LIGHTS as usize;

/// Хэндл WGSL-шейдера конвейера (ассет грузится AssetServer'ом в основном мире;
/// рендер-мир получает его через `Extract`, кэш пайплайнов сам достаёт из
/// `RenderAssets<Shader>`).
#[derive(Resource)]
pub struct LightShader(pub Handle<Shader>);

/// Идентификаторы GPU-пайплайнов конвейера света (кэш Bevy компилирует их
/// асинхронно — система прохода ждёт `get_compute_pipeline`).
#[derive(Resource)]
pub struct LightPipelines {
    pub shadow_map: CachedComputePipelineId,
    pub fov_map: CachedComputePipelineId,
    pub light_map: CachedComputePipelineId,
    pub blur: CachedComputePipelineId,
    pub apply_fov: CachedComputePipelineId,
    pub wall_mask: CachedComputePipelineId,
}

/// Лейауты привязок по проходам (см. `assets/shaders/light.wgsl`).
#[derive(Resource)]
pub struct LightLayouts {
    pub shadow: BindGroupLayout,
    pub fov: BindGroupLayout,
    pub light: BindGroupLayout,
    pub blur: BindGroupLayout,
    pub apply: BindGroupLayout,
    pub wall_mask: BindGroupLayout,
}

/// Всё, что нужно проходам: текстуры конвейера, пайплайны, буферы сцены и
/// счётчики. Ресурс рендер-мира, создаётся один раз.
#[derive(Resource)]
pub struct LightGpuState {
    pub handles: LightSceneHandles,
    pub pipelines: LightPipelines,
    pub layouts: LightLayouts,
    /// Отрезки стен: `array<Wall>` в шейдере.
    pub walls: Buffer,
    /// Ёмкость буфера стен в байтах.
    pub wall_capacity: usize,
    /// Число отрезков в буфере (шейдер идёт по нему циклом).
    pub wall_count: u32,
    /// Источники: `array<Light>` в шейдере.
    pub lights: Buffer,
    pub light_count: u32,
    /// Прямоугольники тайлов стен: `array<WallTile>` — для маски стен.
    pub wall_tiles: Buffer,
    /// Ёмкость буфера тайлов стен в байтах.
    pub wall_tile_capacity: usize,
    pub wall_tile_count: u32,
    /// Базовая униформа кадра.
    pub params: Buffer,
    /// Униформы блюра: `[H, V, H-просачивание, V-просачивание]`.
    pub blur_params: [Buffer; 4],
    /// Подпись геометрии сцены (`LightScene::generation`): по ней решается,
    /// перезаливать ли буферы стен и источников.
    pub geometry: Option<u64>,
}

/// Общие для всех проходов привязки: униформа и два сторадж-буфера.
fn common_entries() -> Vec<BindGroupLayoutEntry> {
    const COMPUTE: ShaderStages = ShaderStages::COMPUTE;
    vec![
        BindGroupLayoutEntry {
            binding: 0,
            visibility: COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(size_of::<LightParams>() as u64),
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 1,
            visibility: COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(size_of::<GpuWallUniform>() as u64),
            },
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 2,
            visibility: COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(size_of::<GpuLightUniform>() as u64),
            },
            count: None,
        },
    ]
}

/// Запись в текстуру (`texture_storage_2d<..., write>`).
fn storage_texture(binding: u32, format: TextureFormat) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::StorageTexture {
            access: StorageTextureAccess::WriteOnly,
            format,
            view_dimension: TextureViewDimension::D2,
        },
        count: None,
    }
}

/// Чтение текстуры. Полярные карты — `R32Float`/`Rg32Float`, они не
/// фильтруемые (`filterable: false`): шейдер читает их `textureLoad`.
fn sampled_texture(binding: u32) -> BindGroupLayoutEntry {
    BindGroupLayoutEntry {
        binding,
        visibility: ShaderStages::COMPUTE,
        ty: BindingType::Texture {
            sample_type: TextureSampleType::Float { filterable: false },
            view_dimension: TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Лейаут по номерам привязок, которые использует точка входа.
fn layout_entries(bindings: &[u32]) -> Vec<BindGroupLayoutEntry> {
    let common = common_entries();
    let mut entries = Vec::new();
    for binding in bindings {
        let entry = match binding {
            0..=2 => common[*binding as usize],
            3 => storage_texture(3, TextureFormat::Rg32Float),
            4 => storage_texture(4, TextureFormat::R32Float),
            5 => storage_texture(5, TextureFormat::Rgba8Unorm),
            6 | 7 | 8 | 11 => sampled_texture(*binding),
            9 => BindGroupLayoutEntry {
                binding: 9,
                visibility: ShaderStages::COMPUTE,
                ty: BindingType::Buffer {
                    ty: BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: BufferSize::new(size_of::<GpuWallTileUniform>() as u64),
                },
                count: None,
            },
            10 => storage_texture(10, TextureFormat::R8Unorm),
            other => panic!("неизвестная привязка шейдера света: {other}"),
        };
        entries.push(entry);
    }
    entries
}

/// Номера привязок каждой точки входа — из ОДНОГО списка строятся и лейаут, и
/// bind group: разойтись они не могут (иначе wgpu ругается на число привязок).
pub const SHADOW_BINDINGS: &[u32] = &[0, 1, 2, 3];
pub const FOV_BINDINGS: &[u32] = &[0, 1, 4];
pub const LIGHT_BINDINGS: &[u32] = &[0, 1, 2, 5, 6, 7, 10];
pub const BLUR_BINDINGS: &[u32] = &[0, 5, 6];
pub const APPLY_BINDINGS: &[u32] = &[0, 5, 6, 8, 11];
/// Маска стен: тайлы стен в буфере и запись в маску.
pub const WALL_MASK_BINDINGS: &[u32] = &[0, 9, 10];

/// Привязки одного прохода: номера из константы выше, униформа прохода и
/// текстуры по номерам (запись — `texture_storage`, чтение — `texture_2d`).
pub struct PassBindings<'a> {
    pub params: &'a Buffer,
    pub textures: &'a [(u32, &'a bevy::render::render_resource::TextureView)],
}

/// Собирает bind group по плану привязок. Буферы 0/1/2 (униформа, стены,
/// источники) подставляются по номеру, остальные обязаны быть в `textures`.
pub fn light_bind_group(
    state: &LightGpuState,
    device: &RenderDevice,
    layout: &BindGroupLayout,
    plan: &[u32],
    pass: PassBindings<'_>,
) -> BindGroup {
    let mut entries = Vec::with_capacity(plan.len());
    for binding in plan {
        let entry = match binding {
            0 => BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: pass.params,
                    offset: 0,
                    size: None,
                }),
            },
            1 => BindGroupEntry {
                binding: 1,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &state.walls,
                    offset: 0,
                    size: None,
                }),
            },
            2 => BindGroupEntry {
                binding: 2,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &state.lights,
                    offset: 0,
                    size: None,
                }),
            },
            9 => BindGroupEntry {
                binding: 9,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &state.wall_tiles,
                    offset: 0,
                    size: None,
                }),
            },
            texture => {
                let view = pass
                    .textures
                    .iter()
                    .find(|(candidate, _)| candidate == texture)
                    .map(|(_, view)| *view)
                    .unwrap_or_else(|| panic!("нет текстуры для привязки {texture}"));
                BindGroupEntry {
                    binding: *texture,
                    resource: BindingResource::TextureView(view),
                }
            }
        };
        entries.push(entry);
    }
    device.create_bind_group("light_pass", layout, &entries)
}

/// Извлечённое из основного мира на этот кадр: шейдер, хэндлы текстур конвейера
/// и упакованная сцена. `Extract`-параметры в Bevy доступны только в
/// `ExtractSchedule` (ресурс `MainWorld` живёт лишь там), поэтому всё нужное
/// переносится в рендер-мир одной системой `extract_light`.
#[derive(Resource)]
pub struct ExtractedLight {
    pub shader: Handle<Shader>,
    pub handles: LightSceneHandles,
    pub packed: super::PackedScene,
    /// Подпись геометрии сцены (см. `LightScene::generation`).
    pub generation: u64,
}

/// Извлечение сцены света в рендер-мир: вызывается в `ExtractSchedule`. Внутри
/// extract-системы «текущий» мир — рендер-мир, а ресурсы основного доступны
/// только через `Extract` (ресурс `MainWorld`); поэтому `Extract<Res<...>>` тут
/// обязателен, а в `Render`-расписании он, наоборот, недоступен.
pub fn extract_light(
    mut commands: Commands,
    scene: Extract<Res<crate::lighting::LightScene>>,
    textures: Extract<Res<super::LightGpu>>,
    shader: Extract<Res<LightShader>>,
) {
    commands.insert_resource(ExtractedLight {
        shader: shader.0.clone(),
        handles: LightSceneHandles {
            shadow_map: textures.shadow_map.clone(),
            fov_map: textures.fov_map.clone(),
            light_a: textures.light_a.clone(),
            light_b: textures.light_b.clone(),
            wall_mask: textures.wall_mask.clone(),
        },
        packed: pack_scene(&scene, ssr_core::tiles::TILE_PX as f32),
        generation: scene.generation,
    });
}

/// Шаг 1 проводки: создаёт лейауты, ставит пайплайны в очередь и создаёт буферы
/// сцены. Система стоит в `RenderSystems::PrepareResources` и работает один раз.
pub fn prepare_light_pipelines(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    extracted: Option<Res<ExtractedLight>>,
    mut made: Local<bool>,
) {
    if *made {
        return;
    }
    let Some(extracted) = extracted else {
        return;
    };
    *made = true;
    let shader: Handle<Shader> = extracted.shader.clone();
    let layout_desc = |label: &'static str, bindings: &[u32]| {
        BindGroupLayoutDescriptor::new(label, &layout_entries(bindings))
    };
    let shadow_layout_desc = layout_desc("light_shadow", SHADOW_BINDINGS);
    let fov_layout_desc = layout_desc("light_fov", FOV_BINDINGS);
    let light_layout_desc = layout_desc("light_map", LIGHT_BINDINGS);
    let blur_layout_desc = layout_desc("light_blur", BLUR_BINDINGS);
    let apply_layout_desc = layout_desc("light_apply_fov", APPLY_BINDINGS);
    let wall_mask_layout_desc = layout_desc("light_wall_mask", WALL_MASK_BINDINGS);
    let queue_pipeline =
        |label: &'static str, layout: BindGroupLayoutDescriptor, entry: &'static str| {
            pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
                label: Some(Cow::Borrowed(label)),
                layout: vec![layout],
                immediate_size: 0,
                shader: shader.clone(),
                shader_defs: Vec::new(),
                entry_point: Some(Cow::Borrowed(entry)),
                zero_initialize_workgroup_memory: true,
            })
        };
    let pipelines = LightPipelines {
        shadow_map: queue_pipeline("light_shadow", shadow_layout_desc.clone(), "shadow_map_cs"),
        fov_map: queue_pipeline("light_fov", fov_layout_desc.clone(), "fov_map_cs"),
        light_map: queue_pipeline("light_map", light_layout_desc.clone(), "light_map_cs"),
        blur: queue_pipeline("light_blur", blur_layout_desc.clone(), "blur_cs"),
        apply_fov: queue_pipeline(
            "light_apply_fov",
            apply_layout_desc.clone(),
            "light_apply_fov_cs",
        ),
        wall_mask: queue_pipeline(
            "light_wall_mask",
            wall_mask_layout_desc.clone(),
            "wall_mask_cs",
        ),
    };
    let layouts = LightLayouts {
        shadow: render_device.create_bind_group_layout("light_shadow", &shadow_layout_desc.entries),
        fov: render_device.create_bind_group_layout("light_fov", &fov_layout_desc.entries),
        light: render_device.create_bind_group_layout("light_map", &light_layout_desc.entries),
        blur: render_device.create_bind_group_layout("light_blur", &blur_layout_desc.entries),
        apply: render_device
            .create_bind_group_layout("light_apply_fov", &apply_layout_desc.entries),
        wall_mask: render_device
            .create_bind_group_layout("light_wall_mask", &wall_mask_layout_desc.entries),
    };
    // Буферы: пустые, но ненулевого размера (нулевой буфер wgpu не создаёт);
    // наполняются системой `update_light_buffers`.
    let wall_capacity = WALL_MIN_CAPACITY * size_of::<GpuWallUniform>();
    let walls = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("light_walls"),
        contents: &vec![0u8; wall_capacity],
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    });
    let lights = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("light_lights"),
        contents: &vec![0u8; LIGHT_CAPACITY * size_of::<GpuLightUniform>()],
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    });
    let wall_tile_capacity = WALL_TILE_MIN_CAPACITY * size_of::<GpuWallTileUniform>();
    let wall_tiles = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("light_wall_tiles"),
        contents: &vec![0u8; wall_tile_capacity],
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    });
    let uniform = |label: &'static str| {
        render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some(label),
            contents: bytemuck::bytes_of(&LightParams::default()),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        })
    };
    commands.insert_resource(LightGpuState {
        handles: extracted.handles.clone(),
        pipelines,
        layouts,
        walls,
        wall_capacity,
        wall_count: 0,
        lights,
        light_count: 0,
        wall_tiles,
        wall_tile_capacity,
        wall_tile_count: 0,
        params: uniform("light_params"),
        blur_params: [
            uniform("light_params_blur_h"),
            uniform("light_params_blur_v"),
            uniform("light_params_bleed_h"),
            uniform("light_params_bleed_v"),
        ],
        geometry: None,
    });
    tracing::info!("light gpu: пайплайны поставлены в очередь, буферы сцены созданы");
}

/// Обновляет буферы сцены из извлечённой сцены: униформы — каждый кадр (камера и
/// глаз едут за игроком), стены и источники — при смене геометрии (`generation`).
pub fn update_light_buffers(
    state: Option<ResMut<LightGpuState>>,
    mut render_device: Option<ResMut<RenderDevice>>,
    render_queue: Res<RenderQueue>,
    extracted: Option<Res<ExtractedLight>>,
) {
    let (Some(mut state), Some(extracted)) = (state, extracted) else {
        return;
    };
    let packed = &extracted.packed;
    let geometry_changed = state.geometry != Some(extracted.generation);
    if geometry_changed {
        if packed.walls.len() > state.wall_capacity {
            // Геометрия выросла (подгрузились новые чанки) — новый буфер.
            let capacity = packed.walls.len().next_power_of_two();
            if let Some(device) = render_device.as_deref_mut() {
                state.walls = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("light_walls"),
                    contents: &vec![0u8; capacity],
                    usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                });
                state.wall_capacity = capacity;
            }
        }
        if !packed.walls.is_empty() {
            render_queue.write_buffer(&state.walls, 0, &packed.walls);
        }
        if packed.wall_tiles.len() > state.wall_tile_capacity {
            // Тайлов стен стало больше — новый буфер маски.
            let capacity = packed.wall_tiles.len().next_power_of_two();
            if let Some(device) = render_device.as_deref_mut() {
                state.wall_tiles = device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("light_wall_tiles"),
                    contents: &vec![0u8; capacity],
                    usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
                });
                state.wall_tile_capacity = capacity;
            }
        }
        if !packed.wall_tiles.is_empty() {
            render_queue.write_buffer(&state.wall_tiles, 0, &packed.wall_tiles);
        }
        render_queue.write_buffer(&state.lights, 0, &packed.lights);
        state.geometry = Some(extracted.generation);
        tracing::debug!(
            walls = packed.params.wall_count,
            tiles = packed.params.wall_tile_count,
            lights = packed.params.light_count,
            "light gpu: буферы сцены обновлены"
        );
    }
    state.wall_count = packed.params.wall_count;
    state.light_count = packed.params.light_count;
    state.wall_tile_count = packed.params.wall_tile_count;
    render_queue.write_buffer(&state.params, 0, bytemuck::bytes_of(&packed.params));
    for (buffer, params) in state.blur_params.iter().zip(packed.blur_params.iter()) {
        render_queue.write_buffer(buffer, 0, bytemuck::bytes_of(params));
    }
}

/// Размеры диспатчей проходов — в одном месте, чтобы система прохода и тесты
/// считали одинаково.
pub fn dispatch_grids(light_count: u32) -> ((u32, u32), (u32, u32), (u32, u32)) {
    let (map_w, map_h) = LIGHT_MAP_SIZE;
    (
        (SHADOW_BINS.div_ceil(64).max(1), light_count.max(1)),
        (FOV_BINS.div_ceil(64).max(1), 1),
        (map_w.div_ceil(8), map_h.div_ceil(8)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Раскладка группы для каждой точки входа: проход теней пишет карту теней,
    /// проход карты света её читает — обе привязки обязаны быть в своих лейаутах
    /// и не пересекаться в одном.
    #[test]
    fn layouts_match_shader_bindings() {
        let bindings = |bindings: &[u32]| {
            layout_entries(bindings)
                .iter()
                .map(|entry| entry.binding)
                .collect::<Vec<_>>()
        };
        assert_eq!(bindings(SHADOW_BINDINGS), SHADOW_BINDINGS.to_vec());
        assert_eq!(bindings(FOV_BINDINGS), FOV_BINDINGS.to_vec());
        assert_eq!(bindings(LIGHT_BINDINGS), LIGHT_BINDINGS.to_vec());
        assert_eq!(bindings(BLUR_BINDINGS), BLUR_BINDINGS.to_vec());
        assert_eq!(bindings(APPLY_BINDINGS), APPLY_BINDINGS.to_vec());
        let shadow = layout_entries(SHADOW_BINDINGS);
        assert!(matches!(
            shadow[3].ty,
            BindingType::StorageTexture {
                access: StorageTextureAccess::WriteOnly,
                format: TextureFormat::Rg32Float,
                ..
            }
        ));
        let light = layout_entries(LIGHT_BINDINGS);
        assert!(matches!(
            light[3].ty,
            BindingType::StorageTexture {
                format: TextureFormat::Rgba8Unorm,
                ..
            }
        ));
        // Полярные карты не фильтруемые: R32Float/Rg32Float.
        assert!(matches!(
            light[5].ty,
            BindingType::Texture {
                sample_type: TextureSampleType::Float { filterable: false },
                ..
            }
        ));
    }

    /// Число привязок у плана и у лейаута совпадает: bind group собирается по
    /// тому же списку номеров, что и лейаут (иначе wgpu отвергает создание).
    #[test]
    fn bind_group_plan_matches_layout_size() {
        for plan in [
            SHADOW_BINDINGS,
            FOV_BINDINGS,
            LIGHT_BINDINGS,
            BLUR_BINDINGS,
            APPLY_BINDINGS,
            WALL_MASK_BINDINGS,
        ] {
            assert_eq!(layout_entries(plan).len(), plan.len());
            // Привязки 0..=11 — вся раскладка шейдера света.
            for binding in plan {
                assert!(
                    *binding <= 11,
                    "привязка {binding} выходит за раскладку шейдера"
                );
            }
        }
    }

    /// Диспатчи: тени — по строке на источник, FOV — одна строка, карта света и
    /// блюр — по текселям карты (группа 8×8).
    #[test]
    fn dispatch_grids_cover_map_and_lights() {
        let (shadow, fov, map) = dispatch_grids(5);
        assert_eq!(shadow, (SHADOW_BINS.div_ceil(64), 5));
        assert_eq!(fov, (FOV_BINS.div_ceil(64), 1));
        assert_eq!(
            map,
            (LIGHT_MAP_SIZE.0.div_ceil(8), LIGHT_MAP_SIZE.1.div_ceil(8))
        );
        // Даже без источников диспатч не вырождается в ноль (wgpu не любит 0).
        assert_eq!(dispatch_grids(0).0.1, 1);
    }
}
