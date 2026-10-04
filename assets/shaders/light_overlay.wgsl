// Оверлей мира по карте света GPU-конвейера (PORT_PLAN 1.1).
//
// В движке свет умножается на каждый спрайт в его шейдере (`COLOR * LIGHT`,
// `light_shared.swsl`). Здесь то же умножение делает один квад поверх мира:
// материал отдаёт карту света, а смешивание настроено как `src * dst` в
// `LightOverlayMaterial::specialize` (см. `light_gpu/overlay.rs`).
//
// Карта света считалась построчно от НИЖНЕЙ границы видимой области (мировое Y
// вверх), а UV меша идут сверху вниз — поэтому V переворачиваем.

#import bevy_sprite::mesh2d_vertex_output::VertexOutput

@group(2) @binding(0) var light_map: texture_2d<f32>;
@group(2) @binding(1) var light_map_sampler: sampler;

@fragment
fn fragment(mesh: VertexOutput) -> @location(0) vec4<f32> {
    let uv = vec2<f32>(mesh.uv.x, 1.0 - mesh.uv.y);
    // rgb — свет (ambient + источники, с мягкими тенями и маской FOV),
    // для умножения альфа не нужна.
    let light = textureSample(light_map, light_map_sampler, uv);
    return vec4<f32>(max(light.rgb, vec3<f32>(0.0)), 1.0);
}
