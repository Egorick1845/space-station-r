// Освещение по конвейеру SS14 (перенос `Clyde.LightRendering.cs` + шейдеров
// `light_shared/light-soft/shadow_cast_shared/light-blur/wall-bleed-blur/fov*`).
//
// Что здесь считается (числа — из движка, см. SS14_LIGHTING.md):
//  1. `shadow_map` — полярная карта теней: на каждый источник строка из
//     SHADOW_BINS угловых бинов, в каждом `vec2(расстояние, расстояние²)` —
//     моменты VSM. В движке это геометрия рёбер (`shadow-depth.vert/.frag`),
//     у нас те же данные (отрезки стен) трассируются по лучам: результат —
//     та же функция «расстояние до стены по углу».
//  2. `fov_map` — такая же полярная карта от глаза (2048 бинов, как `FovMapSize`),
//     используется как маска видимости для света и для тумана войны.
//  3. `light_map` — карта света половинного разрешения: аддитивные вклады
//     источников с формулой затухания движка, выборкой по карте теней
//     (`ChebyshevUpperBound` + 7-точечный PCF из `light-soft.swsl`), маской FOV
//     и ambient карты (`MapLight.ambientLightColor`).
//  4. `blur_h`/`blur_v` — гаусс 0.375/0.25/0.25/0.0625/0.0625 (карта света и
//     просачивание на стены, у просачивания множитель 1.1 — как
//     `wall-bleed-blur.swsl`).
//  5. `light_apply_fov` — умножение карты света на видимость (`fov-lighting.swsl`:
//     `alpha = 1 − occlusion`, `occludeColor` чёрный).
//
// Текстуры: карта теней RG32F (моменты), карта FOV — R32F (расстояния),
// карта света — RGBA8 (rgb = свет, a = 1 − тьма для оверлея).

const PI: f32 = 3.14159265358979323846;
const LIGHTING_HEIGHT: f32 = 1.0; // `LIGHTING_HEIGHT` из light_shared.swsl
// Идентификатор «нет окклюдера»: в движке клир-цвет 1234 (см. SS14_LIGHTING.md).
const NO_OCCLUDER: f32 = 1234.0;

struct Wall {
    // Отрезок стены: (ax, ay, bx, by) в мировых единицах.
    ab: vec4<f32>,
};

struct Light {
    // (x, y) — позиция, radius, energy, falloff, curve, (r, g, b) — цвет.
    data: vec4<f32>,   // x, y, radius, energy
    params: vec4<f32>, // falloff, curve, _, _
    color: vec4<f32>,  // r, g, b, _
};

struct Params {
    // Мир: размер тайла и камера (левый-верхний угол видимой области).
    tile: f32,
    camera: vec2<f32>,
    light_count: u32,
    wall_count: u32,
    // Карта света: её размер и размер вьюпорта.
    map_size: vec2<f32>,
    viewport: vec2<f32>,
    // Глаз (игрок) и параметры FOV.
    eye: vec2<f32>,
    fov_range: f32,
    ambient: f32,
    // Размытие: радиус в текселях по осям (как `radius` у light-blur).
    blur_radius: f32,
    blur_boost: f32, // 1.0 у карты света, 1.1 у просачивания на стены
    // Направление блюра в текселях (1,0) или (0,1) — как `direction` в light-blur.swsl.
    blur_dir: vec2<f32>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> walls: array<Wall>;
@group(0) @binding(2) var<storage, read> lights: array<Light>;
// Карта теней: строк на источники, столбцов SHADOW_BINS (радиальная функция).
@group(0) @binding(3) var shadow_map: texture_storage_2d<rg32float, write>;
// Карта FOV: одна строка из FOV_BINS расстояний до стены по углу.
@group(0) @binding(4) var fov_map: texture_storage_2d<r32float, write>;
// Карта света (половинное разрешение вьюпорта): rgb — свет, a — прозрачность тьмы.
// Хост обязательно пинг-понгит две текстуры: `dst` — куда пишем, `src` — откуда
// читаем (одна и та же текстура не может быть storage и sampled в одном проходе).
@group(0) @binding(5) var dst: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(6) var src: texture_2d<f32>;

const SHADOW_BINS: u32 = 512u; // ShadowMapSize в движке
const FOV_BINS: u32 = 2048u;   // FovMapSize в движке

// Полярный угол: в движке `deflect = atan(rel.y, -rel.x) / PI`, `u = (deflect + 1) / 2`.
fn polar_u(dx: f32, dy: f32) -> f32 {
    return (atan2(dy, -dx) / PI + 1.0) * 0.5;
}

/// Ближайшее расстояние от точки `origin` до отрезка стены вдоль направления
/// `dir` (единичного). `1e30`, если луч не пересекает отрезок.
fn ray_wall(origin: vec2<f32>, dir: vec2<f32>, wall: vec4<f32>) -> f32 {
    let e = wall.zw - wall.xy;
    let denom = dir.x * e.y - dir.y * e.x;
    if abs(denom) < 1e-6 {
        return 1e30;
    }
    let diff = wall.xy - origin;
    let t = (diff.x * e.y - diff.y * e.x) / denom; // расстояние по лучу
    let s = (diff.x * dir.y - diff.y * dir.x) / denom; // параметр на отрезке
    if t <= 0.0 || s < 0.0 || s > 1.0 {
        return 1e30;
    }
    return t;
}

/// Один угловой бин: минимум расстояния по всем стенам.
fn bin_distance(origin: vec2<f32>, angle: f32) -> f32 {
    let dir = vec2<f32>(cos(angle), sin(angle));
    var best = NO_OCCLUDER;
    for (var i = 0u; i < params.wall_count; i = i + 1u) {
        best = min(best, ray_wall(origin, dir, walls[i].ab));
    }
    return best;
}

/// Полярная карта теней: `dispatch(SHADOW_BINS, N)` — x = бин угла,
/// y = индекс источника.
@compute @workgroup_size(64, 1)
fn shadow_map_cs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= SHADOW_BINS || id.y >= params.light_count {
        return;
    }
    let light_pos = lights[id.y].data.xy;
    let angle = (f32(id.x) / f32(SHADOW_BINS)) * 2.0 * PI - PI;
    let dist = bin_distance(light_pos, angle);
    // VSM: моменты (d, d²). Дисперсию, как в движке, добавляем из малой дельты.
    let moment = vec2<f32>(dist, dist * dist + 0.25);
    textureStore(shadow_map, vec2<i32>(i32(id.x), i32(id.y)), moment);
}

/// Полярная карта FOV от глаза игрока: один ряд, FOV_BINS бинов.
@compute @workgroup_size(64, 1)
fn fov_map_cs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= FOV_BINS {
        return;
    }
    let angle = (f32(id.x) / f32(FOV_BINS)) * 2.0 * PI - PI;
    let dist = bin_distance(params.eye, angle);
    textureStore(fov_map, vec2<i32>(i32(id.x), 0), vec4<f32>(dist, 0.0, 0.0, 1.0));
}

/// Момент из карты теней по углу (соответствует `occludeDepth` в движке).
fn occlude_depth(rel: vec2<f32>, light_index: u32) -> vec2<f32> {
    let u = clamp(polar_u(rel.x, rel.y), 0.0, 1.0);
    let x = clamp(i32(u * f32(SHADOW_BINS)), 0, i32(SHADOW_BINS) - 1);
    return textureLoad(shadow_map_read, vec2<i32>(x, i32(light_index)), 0).xy;
}

/// `ChebyshevUpperBound` из `shadow_cast_shared.swsl` дословно.
fn chebyshev(moments: vec2<f32>, t: f32) -> f32 {
    let p = select(0.0, 1.0, t <= moments.x);
    let variance = max(moments.y - moments.x * moments.x, 0.0);
    let d = t - moments.x;
    let p_max = variance / (variance + d * d);
    return max(p, p_max);
}

/// Мягкая тень из `light-soft.swsl`: перпендикуляр, 7 выборок, сигма и гауссовы
/// веса по расстоянию до ближайшего окклюдера.
fn soft_occlusion(diff: vec2<f32>, light_index: u32, softness: f32) -> f32 {
    let our_dist = length(diff);
    // perpendicular = normalize(cross(diff, z)) / 32 * softness * 1.5
    var perpendicular = normalize(vec2<f32>(diff.y, -diff.x)) * (1.0 / 32.0) * softness * 1.5;
    // 7 выборок вдоль перпендикуляра: 0, ±1, ±2, ±3.
    var mindist = NO_OCCLUDER;
    var samples: array<vec2<f32>, 7>;
    samples[0] = occlude_depth(diff, light_index);
    for (var k = 1; k <= 3; k = k + 1) {
        samples[k] = occlude_depth(diff + perpendicular * f32(k), light_index);
        samples[k + 3] = occlude_depth(diff - perpendicular * f32(k), light_index);
    }
    for (var k = 0; k < 7; k = k + 1) {
        mindist = min(mindist, samples[k].x);
    }
    mindist = max(0.001, mindist);
    // sigma = max(0.001, 0.75 * (наше расстояние − ближайший окклюдер))
    let sigma = max(0.001, (our_dist - mindist) * 0.75);
    // Веса: exp(-k²/(2σ²)) для k = 0..3, нормировка на сумму.
    var w: array<f32, 4>;
    var total = 0.0;
    for (var k = 0; k <= 3; k = k + 1) {
        w[k] = exp(-(f32(k) * f32(k)) / (2.0 * sigma * sigma));
    }
    total = w[0] + 2.0 * w[1] + 2.0 * w[2] + 2.0 * w[3];
    var occlusion = chebyshev(samples[0], our_dist) * w[0];
    occlusion += chebyshev(samples[1], our_dist) * w[1];
    occlusion += chebyshev(samples[2], our_dist) * w[1];
    occlusion += chebyshev(samples[3], our_dist) * w[2];
    occlusion += chebyshev(samples[4], our_dist) * w[2];
    occlusion += chebyshev(samples[5], our_dist) * w[3];
    occlusion += chebyshev(samples[6], our_dist) * w[3];
    return occlusion / max(total, 1e-4);
}

/// Формула затухания из `light_shared.swsl` (числа — оттуда же).
fn attenuation(sqr_dist: f32, radius: f32, energy: f32, falloff: f32, curve: f32) -> f32 {
    let s = clamp(sqrt(sqr_dist) / radius, 0.0, 1.0);
    let s2 = s * s;
    let curve_factor = mix(s, s2, clamp(curve, 0.0, 1.0));
    let val = clamp(((1.0 - s2) * (1.0 - s2)) / (1.0 + falloff * curve_factor), 0.0, 1.0);
    return val * energy;
}

/// Карта света: `dispatch(map_w, map_h)` — на каждый тексель суммируем источники
/// аддитивно (как `BlendFunc(SrcAlpha, One)` в движке), затем ambient.
@compute @workgroup_size(8, 8)
fn light_map_cs(@builtin(global_invocation_id) id: vec3<u32>) {
    let map = vec2<u32>(params.map_size);
    if id.x >= map.x || id.y >= map.y {
        return;
    }
    // Тексель карты → мировая точка (камера — левый-верхний угол видимой области).
    let uv = (vec2<f32>(f32(id.x), f32(id.y)) + 0.5) / params.map_size;
    let world = params.camera + uv * params.viewport;

    var rgb = vec3<f32>(params.ambient);
    for (var i = 0u; i < params.light_count; i = i + 1u) {
        let light = lights[i];
        let diff = world - light.data.xy;
        let sqr_dist = dot(diff, diff) + LIGHTING_HEIGHT;
        if sqr_dist > light.data.z * light.data.z {
            continue; // вне радиуса
        }
        // Мягкость: у нас всегда включена (light.soft_shadows = true).
        let occlusion = soft_occlusion(diff, i, 1.0);
        if occlusion <= 0.0 {
            continue;
        }
        let val = attenuation(sqr_dist, light.data.z, light.data.w, light.params.x, light.params.y);
        rgb += light.color.rgb * val * occlusion;
    }
    // Альфа оверлея: 1 − яркость света (тьма). В движке свет применяется
    // умножением к каждому спрайту (`COLOR * LIGHT`); у нас ту же роль играет
    // чёрный оверлей поверх мира — математически то же самое.
    let luminance = clamp(dot(rgb, vec3<f32>(0.299, 0.587, 0.114)), 0.0, 1.0);
    let alpha = 1.0 - luminance;
    let out = vec4<f32>(rgb, alpha);
    textureStore(dst, vec2<i32>(i32(id.x), i32(id.y)), out);
}

/// Полярные карты для чтения в шейдере карты света (текстуры не могут быть
/// и storage, и sampled в одном пайплайне — движок тоже использует отдельные
/// проходы). Здесь только чтение.
@group(0) @binding(7) var shadow_map_read: texture_2d<f32>;

/// Размытие карты света (гаусс из `light-blur.swsl`), направление — из параметров
/// (у нас оба прохода — одна и та же функция с разным `dir`).
@compute @workgroup_size(8, 8)
fn blur_cs(@builtin(global_invocation_id) id: vec3<u32>) {
    let map = vec2<u32>(params.map_size);
    if id.x >= map.x || id.y >= map.y {
        return;
    }
    let coord = vec2<i32>(i32(id.x), i32(id.y));
    let base = textureLoad(src, coord, 0);
    let du = vec2<i32>(i32(round(params.blur_dir.x)), i32(round(params.blur_dir.y)));
    var sum = base * 0.375;
    sum += textureLoad(src, coord + du, 0) * 0.25;
    sum += textureLoad(src, coord - du, 0) * 0.25;
    sum += textureLoad(src, coord + du * 2, 0) * 0.0625;
    sum += textureLoad(src, coord - du * 2, 0) * 0.0625;
    // Просачивание на стены добавляет яркость (×1.1 в wall-bleed-blur.swsl).
    sum = vec4<f32>(sum.rgb * params.blur_boost, sum.a);
    textureStore(dst, coord, sum);
}

/// Умножение карты света на видимость глаза (`fov-lighting.swsl`):
/// `occlusion = Chebyshev(момент FOV, расстояние)`, при полной видимости — без
/// изменений, иначе свет гасится (occludeColor чёрный).
@compute @workgroup_size(8, 8)
fn light_apply_fov_cs(@builtin(global_invocation_id) id: vec3<u32>) {
    let map = vec2<u32>(params.map_size);
    if id.x >= map.x || id.y >= map.y {
        return;
    }
    let uv = (vec2<f32>(f32(id.x), f32(id.y)) + 0.5) / params.map_size;
    let world = params.camera + uv * params.viewport;
    let rel = world - params.eye;
    let our_dist = length(rel);
    let u = clamp(polar_u(rel.x, rel.y), 0.0, 1.0);
    let bin = clamp(i32(u * f32(FOV_BINS)), 0, i32(FOV_BINS) - 1);
    let wall_dist = textureLoad(fov_read, vec2<i32>(bin, 0), 0).x;
    let occlusion = chebyshev(vec2<f32>(wall_dist, wall_dist * wall_dist + 0.25), our_dist);
    var color = textureLoad(src, vec2<i32>(i32(id.x), i32(id.y)), 0);
    color = color * occlusion;
    // Тьма оверлея считается заново: alpha = 1 − свет (движок рисует
    // occludeColor с alpha = 1 − occlusion поверх карты света).
    let luminance = clamp(dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114)), 0.0, 1.0);
    let alpha = max(1.0 - luminance, 1.0 - occlusion);
    textureStore(dst, vec2<i32>(i32(id.x), i32(id.y)), vec4<f32>(color.rgb, alpha));
}

@group(0) @binding(8) var fov_read: texture_2d<f32>;
