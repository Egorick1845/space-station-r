use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{
    BindGroup, BindGroupEntry, BindGroupLayout, BindGroupLayoutDescriptor, BindGroupLayoutEntry,
    BindingResource, BindingType, Buffer, BufferBinding, BufferBindingType, BufferInitDescriptor,
    BufferSize, BufferUsages, CachedComputePipelineId, ComputePipelineDescriptor, PipelineCache,
    ShaderStages, TextureSampleType, TextureViewDimension,
};
use bevy::render::renderer::RenderDevice;
use bevy::render::Extract;
use bevy::shader::Shader;
use std::borrow::Cow;

use bevy::prelude::*;

/// Хэндл WGSL-шейдера конвейера (ассет грузится AssetServer'ом в основном мире;
/// рендер-мир получает его через `Extract`, кэш пайплайнов сам достаёт из
/// `RenderAssets<Shader>`).
#[derive(Resource)]
pub struct LightShader(pub Handle<Shader>);

use super::{pack_scene, GpuLightUniform, GpuWallUniform, LightParams};

/// Идентификаторы GPU-пайплайнов конвейера света (кэш Bevy компилирует их
/// асинхронно — система прохода ждёт `get_compute_pipeline`).
#[derive(Resource)]
pub struct LightPipelines {
    pub shadow_map: CachedComputePipelineId,
    pub fov_map: CachedComputePipelineId,
    pub light_map: CachedComputePipelineId,
    pub blur: CachedComputePipelineId,
    pub apply_fov: CachedComputePipelineId,
}

/// GPU-буферы сцены и лейаут привязок (bind group собирается в проходе).
#[derive(Resource)]
pub struct LightBindings {
    pub walls: Buffer,
    pub lights: Buffer,
    pub params: Buffer,
    pub layout: BindGroupLayout,
    pub wall_count: u32,
    pub light_count: u32,
}

/// Раскладка группы 0 — должна совпадать с `group(0)` в
/// `assets/shaders/light.wgsl` (uniform, storage×2, sampled×3; storage-текстуры
/// записываются в разных проходах, поэтому привязываются в проходе, а не здесь).
fn bind_group_layout_entries() -> Vec<BindGroupLayoutEntry> {
    const COMPUTE: ShaderStages = ShaderStages::COMPUTE;
    let texture = |filterable: bool| BindingType::Texture {
        sample_type: TextureSampleType::Float { filterable },
        view_dimension: TextureViewDimension::D2,
        multisampled: false,
    };
    let storage = |min: u64| BindingType::Buffer {
        ty: BufferBindingType::Storage { read_only: true },
        has_dynamic_offset: false,
        min_binding_size: BufferSize::new(min),
    };
    vec![
        // 0: uniform Params
        BindGroupLayoutEntry {
            binding: 0,
            visibility: COMPUTE,
            ty: BindingType::Buffer {
                ty: BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: BufferSize::new(std::mem::size_of::<LightParams>() as u64),
            },
            count: None,
        },
        // 1: стены, 2: источники
        BindGroupLayoutEntry {
            binding: 1,
            visibility: COMPUTE,
            ty: storage(std::mem::size_of::<GpuWallUniform>() as u64),
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 2,
            visibility: COMPUTE,
            ty: storage(std::mem::size_of::<GpuLightUniform>() as u64),
            count: None,
        },
        // 6: sampled карта света (src), 7: shadow_read, 8: fov_read
        BindGroupLayoutEntry {
            binding: 6,
            visibility: COMPUTE,
            ty: texture(true),
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 7,
            visibility: COMPUTE,
            ty: texture(true),
            count: None,
        },
        BindGroupLayoutEntry {
            binding: 8,
            visibility: COMPUTE,
            ty: texture(true),
            count: None,
        },
    ]
}

/// Шаг 1 проводки (PORT_PLAN 1.1): пайплайны и буферы сцены. Система стоит в
/// `Render` — сцена извлекается из основного мира (`Extract`), пайплайны
/// ставятся в очередь один раз; bind group собирается в проходе.
pub fn prepare_light_pipelines(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    shader: Extract<Res<LightShader>>,
    scenes: Extract<Res<crate::lighting::LightScene>>,
    mut made: Local<bool>,
) {
    if *made {
        return;
    }
    *made = true;
    // Шейдер — обычный ассет (загружен AssetServer'ом основного мира и извлечён
    // в рендер-мир штатной системой шейдеров); кэш пайплайнов сам скомпилирует
    // модуль из него.
    let shader: Handle<Shader> = shader.0.clone();
    let entries = bind_group_layout_entries();
    let make_pipeline = |entry: &'static str| {
        pipeline_cache.queue_compute_pipeline(ComputePipelineDescriptor {
            label: Some(Cow::Borrowed(entry)),
            layout: vec![BindGroupLayoutDescriptor::new("light", &entries)],
            immediate_size: 0,
            shader: shader.clone(),
            shader_defs: Vec::new(),
            entry_point: Some(Cow::Borrowed(entry)),
            zero_initialize_workgroup_memory: true,
        })
    };
    let pipelines = LightPipelines {
        shadow_map: make_pipeline("shadow_map_cs"),
        fov_map: make_pipeline("fov_map_cs"),
        light_map: make_pipeline("light_map_cs"),
        blur: make_pipeline("blur_cs"),
        apply_fov: make_pipeline("light_apply_fov_cs"),
    };
    // Буферы сцены: упаковываем текущее состояние; при изменении сцены они
    // пересоздаются системой синхронизации (следующий шаг плана).
    let packed = pack_scene(&scenes, ssr_core::tiles::TILE_PX as f32);
    let walls = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("light_walls"),
        contents: &packed.walls,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    });
    let lights = render_device.create_buffer_with_data(&BufferInitDescriptor {
        label: Some("light_lights"),
        contents: &packed.lights,
        usage: BufferUsages::STORAGE | BufferUsages::COPY_DST,
    });
    let mut params = packed.params;
    params.wall_count =
        packed.walls.len() as u32 / std::mem::size_of::<GpuWallUniform>() as u32;
    params.light_count =
        packed.lights.len() as u32 / std::mem::size_of::<GpuLightUniform>() as u32;
    let params_buffer =
        render_device.create_buffer_with_data(&BufferInitDescriptor {
            label: Some("light_params"),
            contents: bytemuck::bytes_of(&params),
            usage: BufferUsages::UNIFORM | BufferUsages::COPY_DST,
        });
    let layout = render_device.create_bind_group_layout("light", &entries);
    commands.insert_resource(LightBindings {
        walls,
        lights,
        params: params_buffer,
        layout,
        wall_count: params.wall_count,
        light_count: params.light_count,
    });
    commands.insert_resource(pipelines);
    tracing::info!(
        walls = params.wall_count,
        lights = params.light_count,
        "light gpu: пайплайны поставлены в очередь, буферы созданы"
    );
}

/// Привязки одного прохода: uniform + стены + источники + sampled-текстуры.
/// Storage-текстуры (запись) добавляются в проходе по номерам 3/4/5.
pub fn light_bind_group(
    bindings: &LightBindings,
    device: &RenderDevice,
    images: &RenderAssets<bevy::render::texture::GpuImage>,
    light_a: &bevy::asset::Handle<bevy::image::Image>,
    shadow_map: &bevy::asset::Handle<bevy::image::Image>,
    fov_map: &bevy::asset::Handle<bevy::image::Image>,
) -> Option<BindGroup> {
    let src = images.get(light_a)?.texture_view.clone();
    let shadow = images.get(shadow_map)?.texture_view.clone();
    let fov = images.get(fov_map)?.texture_view.clone();
    Some(device.create_bind_group(
        "light_pass",
        &bindings.layout,
        &[
            BindGroupEntry {
                binding: 0,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &bindings.params,
                    offset: 0,
                    size: None,
                }),
            },
            BindGroupEntry {
                binding: 1,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &bindings.walls,
                    offset: 0,
                    size: None,
                }),
            },
            BindGroupEntry {
                binding: 2,
                resource: BindingResource::Buffer(BufferBinding {
                    buffer: &bindings.lights,
                    offset: 0,
                    size: None,
                }),
            },
            BindGroupEntry {
                binding: 6,
                resource: BindingResource::TextureView(&src),
            },
            BindGroupEntry {
                binding: 7,
                resource: BindingResource::TextureView(&shadow),
            },
            BindGroupEntry {
                binding: 8,
                resource: BindingResource::TextureView(&fov),
            },
        ],
    ))
}
