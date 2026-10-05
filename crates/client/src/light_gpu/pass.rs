//! Цепочка compute-проходов конвейера света (план §7 в `SS14_LIGHTING.md`).
//!
//! Порядок — как в движке (`Clyde.LightRendering`): карта теней → карта FOV →
//! карта света → блюр H/V дважды → просачивание на стены (тот же блюр с
//! множителем 1.1) → маска FOV. Карта света пинг-понгится между двумя
//! текстурами: одна и та же текстура не может быть записью и чтением в одном
//! проходе. Результат последнего прохода — в `light_b`, её и семплит оверлей.
//!
//! Система идёт в расписании `Core2d` набором `Core2dSystems::Prepass` — то есть
//! перед основным проходом камеры (`main_opaque_pass_2d`), как `BeforeRender` в
//! движке.

use bevy::prelude::*;
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{ComputePassDescriptor, PipelineCache, TextureView};
use bevy::render::renderer::{RenderContext, RenderDevice};
use bevy::render::texture::GpuImage;

use super::prepare::{
    APPLY_BINDINGS, BLUR_BINDINGS, FOV_BINDINGS, LIGHT_BINDINGS, LightGpuState, PassBindings,
    SHADOW_BINDINGS, WALL_MASK_BINDINGS, dispatch_grids, light_bind_group,
};

/// Полный проход конвейера: рисует карту света и карту FOV на этот кадр.
pub fn light_gpu_pass(
    mut ctx: RenderContext,
    state: Option<Res<LightGpuState>>,
    pipeline_cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    render_device: Res<RenderDevice>,
) {
    let Some(state) = state else {
        return;
    };
    let (Some(shadow), Some(fov), Some(fov_far), Some(light_a), Some(light_b), Some(wall_mask)) = (
        images.get(&state.handles.shadow_map),
        images.get(&state.handles.fov_map),
        images.get(&state.handles.fov_far),
        images.get(&state.handles.light_a),
        images.get(&state.handles.light_b),
        images.get(&state.handles.wall_mask),
    ) else {
        // Текстуры ещё не подняты на GPU (первый кадр после старта).
        return;
    };
    // Пайплайны компилируются асинхронно: пока не готовы — кадр без света
    // (ровно как `PipelineCache` в движке, который ждёт компиляции).
    let (
        Some(shadow_pipeline),
        Some(fov_pipeline),
        Some(light_pipeline),
        Some(blur_pipeline),
        Some(apply_pipeline),
        Some(mask_pipeline),
    ) = (
        pipeline_cache.get_compute_pipeline(state.pipelines.shadow_map),
        pipeline_cache.get_compute_pipeline(state.pipelines.fov_map),
        pipeline_cache.get_compute_pipeline(state.pipelines.light_map),
        pipeline_cache.get_compute_pipeline(state.pipelines.blur),
        pipeline_cache.get_compute_pipeline(state.pipelines.apply_fov),
        pipeline_cache.get_compute_pipeline(state.pipelines.wall_mask),
    )
    else {
        return;
    };
    let shadow_view: &TextureView = &shadow.texture_view;
    let fov_view: &TextureView = &fov.texture_view;
    let far_view: &TextureView = &fov_far.texture_view;
    let a_view: &TextureView = &light_a.texture_view;
    let b_view: &TextureView = &light_b.texture_view;
    let mask_view: &TextureView = &wall_mask.texture_view;

    // Диспатчи: тени — строка на источник, FOV — одна строка, карта света и
    // блюр — по текселям карты (группа 8×8 у шейдера).
    let (shadow_grid, fov_grid, map_grid) = dispatch_grids(state.light_count);

    let layouts = &state.layouts;
    let bind = |layout,
                plan: &[u32],
                params: &bevy::render::render_resource::Buffer,
                textures: &[(u32, &TextureView)]| {
        light_bind_group(
            &state,
            &render_device,
            layout,
            plan,
            PassBindings { params, textures },
        )
    };
    let shadow_bind = bind(
        &layouts.shadow,
        SHADOW_BINDINGS,
        &state.params,
        &[(3, shadow_view)],
    );
    let fov_bind = bind(
        &layouts.fov,
        FOV_BINDINGS,
        &state.params,
        &[(4, fov_view), (12, far_view)],
    );
    let light_bind = bind(
        &layouts.light,
        LIGHT_BINDINGS,
        &state.params,
        &[(5, a_view), (6, b_view), (7, shadow_view), (10, mask_view)],
    );
    let blur_h = bind(
        &layouts.blur,
        BLUR_BINDINGS,
        &state.blur_params[0],
        &[(5, b_view), (6, a_view)],
    );
    let blur_v = bind(
        &layouts.blur,
        BLUR_BINDINGS,
        &state.blur_params[1],
        &[(5, a_view), (6, b_view)],
    );
    let bleed_h = bind(
        &layouts.blur,
        BLUR_BINDINGS,
        &state.blur_params[2],
        &[(5, b_view), (6, a_view)],
    );
    let bleed_v = bind(
        &layouts.blur,
        BLUR_BINDINGS,
        &state.blur_params[3],
        &[(5, a_view), (6, b_view)],
    );
    let apply_bind = bind(
        &layouts.apply,
        APPLY_BINDINGS,
        &state.params,
        &[(5, b_view), (6, a_view), (8, fov_view), (11, mask_view), (13, far_view)],
    );
    let mask_bind = bind(
        &layouts.wall_mask,
        WALL_MASK_BINDINGS,
        &state.params,
        &[(10, mask_view)],
    );

    let encoder = ctx.command_encoder();
    let mut pass = encoder.begin_compute_pass(&ComputePassDescriptor {
        label: Some("light_gpu"),
        timestamp_writes: None,
    });
    // 1. Карта теней: полярные моменты по каждому источнику.
    if state.light_count > 0 {
        pass.set_pipeline(shadow_pipeline);
        pass.set_bind_group(0, &shadow_bind, &[]);
        pass.dispatch_workgroups(shadow_grid.0, shadow_grid.1, 1);
    }
    // 2. Карта FOV от глаза — маска видимости.
    pass.set_pipeline(fov_pipeline);
    pass.set_bind_group(0, &fov_bind, &[]);
    pass.dispatch_workgroups(fov_grid.0, fov_grid.1, 1);
    // 3. Карта света: аддитивные источники + ambient, выборка карты теней;
    //    она же обнуляет маску стен (единственный проход по всем текселям карты).
    pass.set_pipeline(light_pipeline);
    pass.set_bind_group(0, &light_bind, &[]);
    pass.dispatch_workgroups(map_grid.0, map_grid.1, 1);
    // 3б. Маска стен: воркгрупп на тайл стены (аналог стенсила движка).
    if state.wall_tile_count > 0 {
        pass.set_pipeline(mask_pipeline);
        pass.set_bind_group(0, &mask_bind, &[]);
        pass.dispatch_workgroups(state.wall_tile_count, 1, 1);
    }
    // 4. Блюр: два полных прохода H/V (гаусс 0.375/0.25/0.25/0.0625/0.0625).
    for bind_group in [&blur_h, &blur_v, &blur_h, &blur_v] {
        pass.set_pipeline(blur_pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(map_grid.0, map_grid.1, 1);
    }
    // 5. Просачивание на стены: тот же блюр с множителем 1.1.
    for bind_group in [&bleed_h, &bleed_v] {
        pass.set_pipeline(blur_pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.dispatch_workgroups(map_grid.0, map_grid.1, 1);
    }
    // 6. Маска FOV на карту света — итог в `light_b`.
    pass.set_pipeline(apply_pipeline);
    pass.set_bind_group(0, &apply_bind, &[]);
    pass.dispatch_workgroups(map_grid.0, map_grid.1, 1);
}
