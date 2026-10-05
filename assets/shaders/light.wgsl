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
//     используется как маска видимости для света и для тумана войны. Рядом —
//     `fov_far_map`: расстояние до ВЫХОДА из первого тела стены по углу
//     (аналог второго рендера карты глубины с front-face culling в движке —
//     «смотрим внутрь стен»); по нему hard FOV прячет ВСЁ за первой стеной
//     непрозрачным чёрным (`fov.swsl`, `occludeColor = Black`, `DrawHardFov`).
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

struct WallTile {
    // Мировой прямоугольник тайла стены: (x0, y0, x1, y1).
    rect: vec4<f32>,
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
    wall_tile_count: u32,
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
// Те же полярные карты для ЧТЕНИЯ в шейдере карты света: текстура не может быть
// и storage, и sampled в одном пайплайне, поэтому хост даёт на них отдельные
// привязки (движок тоже разводит запись и чтение по проходам). Объявления — до
// первого использования: WGSL требует порядок «объявил → применил».
@group(0) @binding(7) var shadow_map_read: texture_2d<f32>;
@group(0) @binding(8) var fov_read: texture_2d<f32>;
// Маска стен: тайлы стен (и закрытых дверей) мировыми прямоугольниками плюс
// готовая маска карты света. Аналог стенсила движка (`ApplyLightingFovToBuffer`):
// на стенах маска видимости НЕ гасит свет, поэтому стены остаются видны
// (просачивание света, `wall-bleed-blur`). Маска пишется проходом `wall_mask_cs`
// и обнуляется в `light_map_cs` каждый кадр.
@group(0) @binding(9) var<storage, read> wall_tiles: array<WallTile>;
@group(0) @binding(10) var wall_mask: texture_storage_2d<r8unorm, write>;
@group(0) @binding(11) var wall_mask_read: texture_2d<f32>;
// Дальняя граница первого тела стены от глаза (hard FOV, `fov.swsl`):
// запись в проходе карты FOV, чтение — в проходе применения маски.
@group(0) @binding(12) var fov_far_map: texture_storage_2d<r32float, write>;
@group(0) @binding(13) var fov_far_read: texture_2d<f32>;

const SHADOW_BINS: u32 = 512u; // ShadowMapSize в движке
const FOV_BINS: u32 = 2048u;   // FovMapSize в движке
/// Глубина просачивания света в стену в мировых единицах (`wall-bleed-blur` +
/// `MergeWallLayer` в движке): свет заходит за первый окклюдер и гаснет.
/// Без этого стены либо чернеют целиком, либо (при маске стен на весь тайл)
/// светятся квадратами в невидимой зоне.
const WALL_BLEED_DEPTH: f32 = 14.0;
/// Множитель света, переносимого с пола на стену (`wall-bleed-blur.swsl`: 1.1).
const WALL_BLEED_BOOST: f32 = 1.1;
/// Соседи текселя карты света (8 направлений) — для просачивания на стены.
const NEIGHBOUR_OFFSETS: array<vec2<i32>, 8> = array<vec2<i32>, 8>(
    vec2<i32>(1, 0), vec2<i32>(-1, 0), vec2<i32>(0, 1), vec2<i32>(0, -1),
    vec2<i32>(1, 1), vec2<i32>(1, -1), vec2<i32>(-1, 1), vec2<i32>(-1, -1),
);
/// Радиусы кольцевой выборки «затёкшего» света в текселях карты (движок размывает
/// карту на четвертьразрешении с σ ≈ 24 экранных пикселя — у нас это ~10-12
/// текселей карты; берём кольца 6/12/24, чтобы покрыть весь тайл стены).
const WALL_BLEED_RADII: array<i32, 3> = array<i32, 3>(6, 12, 24);
/// Допуск кластера входов в тело стены (как `FOV_CLUSTER_EPS` в `ssr_core::light`):
/// сомкнутые тайлы одной стены входят по лучу вплотную, зазоры между разными
/// конструкциями на сетке тайлов всегда больше тайла.
const FOV_CLUSTER_EPS: f32 = 0.5;
/// Bias hard FOV из `fov.swsl` (`−0.75/32` бина): точка за выходом из первого
/// тела стены получает непрозрачную тьму, само тело видно («внутрь стен»).
const HARD_FOV_BIAS: f32 = 0.75;
/// Досягаемость hard FOV в мировых единицах: дальше диагонали видимой области
/// (~31 тайл) карты глубины в движке клирятся 1234 — ничего не скрыто.
const FOV_REACH_UNITS: f32 = 40.0 * 32.0;

// Полярный угол: в движке `deflect = atan(rel.y, -rel.x) / PI`, `u = (deflect + 1) / 2`.
fn polar_u(dx: f32, dy: f32) -> f32 {
    return (atan2(dy, -dx) / PI + 1.0) * 0.5;
}

// Дробный индекс бина полярной карты для направления (dx, dy) — ИНВЕРСИЯ
// генерации: бин `b` смотрит под углом `b/bins*2*PI - PI` (0 — запад).
// Читать `polar_u` из НАШИХ карт нельзя: это UV движка с швом на востоке,
// из-за него полкарты вырождается в один бин (чёрные клинья «тумана войны»).
fn polar_bin(dx: f32, dy: f32, bins: f32) -> f32 {
    return (atan2(dy, dx) + PI) / (2.0 * PI) * bins - 0.5;
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

/// Луч против тела тайла (slab-тест): `(вход, выход)` по лучу; промах —
/// `vec2(1e30, −1e30)` (как `None` в `ssr_core::light::ray_rect_span`).
fn ray_rect_span(origin: vec2<f32>, dir: vec2<f32>, rect: vec4<f32>) -> vec2<f32> {
    var enter = -1e30;
    var exit = 1e30;
    // Ось X: параллельный луч попадает в плиту только изнутри её диапазона.
    if (abs(dir.x) < 1e-6) {
        if (origin.x < rect.x || origin.x > rect.z) {
            return vec2<f32>(1e30, -1e30);
        }
    } else {
        let inv = 1.0 / dir.x;
        var t0 = (rect.x - origin.x) * inv;
        var t1 = (rect.z - origin.x) * inv;
        if (t0 > t1) {
            let tmp = t0;
            t0 = t1;
            t1 = tmp;
        }
        enter = max(enter, t0);
        exit = min(exit, t1);
    }
    // Ось Y — то же самое.
    if (abs(dir.y) < 1e-6) {
        if (origin.y < rect.y || origin.y > rect.w) {
            return vec2<f32>(1e30, -1e30);
        }
    } else {
        let inv = 1.0 / dir.y;
        var t0 = (rect.y - origin.y) * inv;
        var t1 = (rect.w - origin.y) * inv;
        if (t0 > t1) {
            let tmp = t0;
            t0 = t1;
            t1 = tmp;
        }
        enter = max(enter, t0);
        exit = min(exit, t1);
    }
    if (enter > exit || exit <= 0.0) {
        return vec2<f32>(1e30, -1e30);
    }
    return vec2<f32>(max(enter, 0.0), exit);
}

/// Выход из первого тела стены по лучу (`ssr_core::light::first_body_exit`):
/// кластер сомкнутых тайлов расширяется, пока у тайлов с входом не дальше
/// текущего выхода есть более дальние выходы. Аналог front-face culling в
/// движке (`DrawFov` рендерит карту глубины второй раз с `CullFaceMode.Front`
/// — «смотрим внутрь стен», `Clyde.LightRendering.cs:237-265`).
fn first_body_exit(origin: vec2<f32>, dir: vec2<f32>) -> f32 {
    var near = NO_OCCLUDER;
    let reach = (FOV_REACH_UNITS + 64.0) * (FOV_REACH_UNITS + 64.0);
    for (var i = 0u; i < params.wall_tile_count; i = i + 1u) {
        let rect = wall_tiles[i].rect;
        let center = (rect.xy + rect.zw) * 0.5 - origin;
        if (dot(center, center) > reach) {
            continue; // тайл заведомо дальше досягаемости FOV
        }
        let span = ray_rect_span(origin, dir, rect);
        if (span.x <= span.y) {
            near = min(near, span.x);
        }
    }
    if (near >= NO_OCCLUDER) {
        return NO_OCCLUDER;
    }
    var far = near;
    for (var iter = 0; iter < 32; iter = iter + 1) {
        var best = far;
        for (var i = 0u; i < params.wall_tile_count; i = i + 1u) {
            let rect = wall_tiles[i].rect;
            let center = (rect.xy + rect.zw) * 0.5 - origin;
            if (dot(center, center) > reach) {
                continue;
            }
            let span = ray_rect_span(origin, dir, rect);
            if (span.x <= span.y && span.x <= far + FOV_CLUSTER_EPS) {
                best = max(best, span.y);
            }
        }
        if (best <= far + 1e-4) {
            break;
        }
        far = best;
    }
    return far;
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
    // Запись в storage-текстуру идёт вектором из четырёх компонент (у `rg32float`
    // значимы первые две — моменты; naga требует полный vec4).
    let moment = vec4<f32>(dist, dist * dist + 0.25, 0.0, 0.0);
    textureStore(shadow_map, vec2<i32>(i32(id.x), i32(id.y)), moment);
}

/// Полярная карта FOV от глаза игрока: один ряд, FOV_BINS бинов. Рядом пишем
/// карту ВЫХОДА из первого тела стены — по ней hard FOV прячет всё за стеной.
@compute @workgroup_size(64, 1)
fn fov_map_cs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= FOV_BINS {
        return;
    }
    let angle = (f32(id.x) / f32(FOV_BINS)) * 2.0 * PI - PI;
    let dir = vec2<f32>(cos(angle), sin(angle));
    let dist = bin_distance(params.eye, angle);
    textureStore(fov_map, vec2<i32>(i32(id.x), 0), vec4<f32>(dist, 0.0, 0.0, 1.0));
    let far = first_body_exit(params.eye, dir);
    textureStore(fov_far_map, vec2<i32>(i32(id.x), 0), vec4<f32>(far, 0.0, 0.0, 1.0));
}

/// Момент из карты теней по углу: ЛИНЕЙНАЯ интерполяция двух соседних бинов
/// (в движке полярная карта читается `texture.Sample` с линейным фильтром).
/// Раньше брался ровно один бин (`round`) — из-за этого края теней шли
/// ступеньками по бинам («квадратики») и дрожали, когда бин границы менялся.
/// Полярная карта циклична по углу, поэтому индексы заворачиваются по модулю.
fn occlude_depth(rel: vec2<f32>, light_index: u32) -> vec2<f32> {
    let u = polar_bin(rel.x, rel.y, f32(SHADOW_BINS));
    let base = floor(u);
    let f = u - base;
    let i0 = wrap_bin(i32(base), i32(SHADOW_BINS));
    let i1 = wrap_bin(i32(base) + 1, i32(SHADOW_BINS));
    let a = textureLoad(shadow_map_read, vec2<i32>(i0, i32(light_index)), 0).xy;
    let b = textureLoad(shadow_map_read, vec2<i32>(i1, i32(light_index)), 0).xy;
    return mix(a, b, f);
}

/// Заворот индекса бина в диапазон `[0, bins)` (карта углов замкнута в кольцо).
fn wrap_bin(index: i32, bins: i32) -> i32 {
    return ((index % bins) + bins) % bins;
}

/// Глубина стены от глаза по направлению: тоже линейная интерполяция соседних
/// бинов карты FOV (без неё граница видимости прыгает на бин и «дрожит»).
fn fov_depth(rel: vec2<f32>) -> f32 {
    let u = polar_bin(rel.x, rel.y, f32(FOV_BINS));
    let base = floor(u);
    let f = u - base;
    let i0 = wrap_bin(i32(base), i32(FOV_BINS));
    let i1 = wrap_bin(i32(base) + 1, i32(FOV_BINS));
    let a = textureLoad(fov_read, vec2<i32>(i0, 0), 0).x;
    let b = textureLoad(fov_read, vec2<i32>(i1, 0), 0).x;
    return mix(a, b, f);
}

/// Выход из первого тела стены по направлению — та же интерполяция бинов, что у
/// `fov_depth`, но по карте `fov_far` (hard FOV, `fov.swsl`).
fn fov_far_depth(rel: vec2<f32>) -> f32 {
    let u = polar_bin(rel.x, rel.y, f32(FOV_BINS));
    let base = floor(u);
    let f = u - base;
    let i0 = wrap_bin(i32(base), i32(FOV_BINS));
    let i1 = wrap_bin(i32(base) + 1, i32(FOV_BINS));
    let a = textureLoad(fov_far_read, vec2<i32>(i0, 0), 0).x;
    let b = textureLoad(fov_far_read, vec2<i32>(i1, 0), 0).x;
    return mix(a, b, f);
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
    // Маска стен обнуляется здесь же: проход карты света — единственный, кто
    // обходит ВСЕ тексели карты (у маски отдельный проход только по тайлам стен,
    // и без обнуления снятая стена осталась бы «видимой» навсегда).
    textureStore(wall_mask, vec2<i32>(i32(id.x), i32(id.y)), vec4<f32>(0.0));
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

/// Полярные карты для чтения в шейдере карты света (объявлены вверху, у
/// остальных привязок: WGSL требует объявление до использования).
/// Чтение карты света с зажимом координат: у края карты `textureLoad` за
/// границей возвращает нули и подмешивал бы темноту (полосы по краям экрана).
/// В движке то же самое делает `WrapMode.ClampToEdge` у таргета блюра.
fn load_light(coord: vec2<i32>) -> vec4<f32> {
    let max_coord = vec2<i32>(params.map_size) - vec2<i32>(1);
    return textureLoad(src, clamp(coord, vec2<i32>(0), max_coord), 0);
}

/// Размытие карты света (гаусс из `light-blur.swsl`), направление — из параметров
/// (у нас оба прохода — одна и та же функция с разным `dir`).
@compute @workgroup_size(8, 8)
fn blur_cs(@builtin(global_invocation_id) id: vec3<u32>) {
    let map = vec2<u32>(params.map_size);
    if id.x >= map.x || id.y >= map.y {
        return;
    }
    let coord = vec2<i32>(i32(id.x), i32(id.y));
    let base = load_light(coord);
    let du = vec2<i32>(i32(round(params.blur_dir.x)), i32(round(params.blur_dir.y)));
    var sum = base * 0.375;
    sum += load_light(coord + du) * 0.25;
    sum += load_light(coord - du) * 0.25;
    sum += load_light(coord + du * 2) * 0.0625;
    sum += load_light(coord - du * 2) * 0.0625;
    // Просачивание на стены добавляет яркость (×1.1 в wall-bleed-blur.swsl).
    sum = vec4<f32>(sum.rgb * params.blur_boost, sum.a);
    textureStore(dst, coord, sum);
}

/// Маска стен: один воркгрупп (8×8) на тайл стены — каждый поток пишет свою
/// долю текселей прямоугольника тайла. Все потоки разных тайлов пишут одно и то
/// же значение (1), гонки безвредны. Аналог отрисовки стен в стенсил в движке.
@compute @workgroup_size(8, 8)
fn wall_mask_cs(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(local_invocation_id) lid: vec3<u32>,
) {
    if wg.x >= params.wall_tile_count {
        return;
    }
    let rect = wall_tiles[wg.x].rect;
    let map = vec2<i32>(params.map_size);
    let uv0 = (rect.xy - params.camera) / params.viewport;
    let uv1 = (rect.zw - params.camera) / params.viewport;
    let lo = clamp(vec2<i32>(floor(uv0 * params.map_size)), vec2<i32>(0), map);
    let hi = clamp(vec2<i32>(ceil(uv1 * params.map_size)), vec2<i32>(0), map);
    var y = lo.y + i32(lid.y);
    loop {
        if y >= hi.y {
            break;
        }
        var x = lo.x + i32(lid.x);
        loop {
            if x >= hi.x {
                break;
            }
            textureStore(wall_mask, vec2<i32>(x, y), vec4<f32>(1.0));
            x = x + 8;
        }
        y = y + 8;
    }
}

/// Умножение карты света на видимость глаза (`fov-lighting.swsl`):
/// `occlusion = Chebyshev(момент FOV, расстояние)`, при полной видимости — без
/// изменений, иначе свет гасится (occludeColor чёрный). На тайлах стен маска не
/// гасит свет — их перезаписывает просачивание (`wall-merge.swsl`), а гасит
/// только hard FOV: всё за первым телом стены заливается непрозрачным чёрным
/// (`fov.swsl`, `ApplyFovToBuffer`, `DrawHardFov = true` в движке).
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
    // Глубина стены от глаза по этому направлению (с интерполяцией бинов).
    let wall_dist = fov_depth(rel);
    var occlusion = chebyshev(vec2<f32>(wall_dist, wall_dist * wall_dist + 0.25), our_dist);
    // `Eye.DrawFov = false` (призрак, `observer.yml`): проход FOV не гасит свет —
    // параметр `fov_range` приходит как 1.0, и видимость считается полной.
    if (params.fov_range > 0.5) {
        occlusion = 1.0;
    }
    // Просачивание света на стены (`wall-bleed-blur.swsl`): видимая грань стены
    // освещается светом соседних НЕ-стеновых текселей, поэтому стена рядом с
    // освещённым полом видна, а глубина стены (там и соседи — стена) остаётся
    // тёмной. Раньше яркость стены гасилась по расстоянию от глаза
    // (`exp(-depth / 14)`) — из-за этого темнела ровно та стена, что перед нами.
    let wall = textureLoad(wall_mask_read, vec2<i32>(i32(id.x), i32(id.y)), 0).x;
    var color = textureLoad(src, vec2<i32>(i32(id.x), i32(id.y)), 0);
    if (wall > 0.5) {
        // Стены: `BlurOntoWalls` + `MergeWallLayer` — последние два прохода
        // движка. Полноэкранное размытие карты света на четвертьразрешении
        // (три итерации H+V, радиус `7e-3 * 14/cameraSize * (i+1)` ≈ σ 24
        // экранных пикселя) даёт «затёкший» на стены свет, после чего полигоны
        // окклюдеров ПЕРЕЗАПИСЫВАЮТ значение буфера этим размытым светом ×1.1
        // (`wall-merge.swsl`, без смешивания).
        //
        // Внутри тайла стены собственного света нет (самозатенение VSM), поэтому
        // без замены стена прямо перед игроком остаётся чёрной — ровно дефект
        // «тень падает на стены перед нами». Размытие приближаем кольцевой
        // выборкой: 8 направлений × 3 радиуса, вес 1/r, только НЕ-стеновые
        // тексели (пол рядом со стеной её и освещает), стена не гасится FOV.
        var sum = vec3<f32>(0.0);
        var weight = 0.0;
        for (var r = 0u; r < 3u; r = r + 1u) {
            let radius = WALL_BLEED_RADII[r];
            for (var k = 0u; k < 8u; k = k + 1u) {
                let step = vec2<i32>(NEIGHBOUR_OFFSETS[k]) * radius;
                let ncoord = vec2<i32>(i32(id.x), i32(id.y)) + step;
                let in_map =
                    all(ncoord >= vec2<i32>(0)) && all(ncoord < vec2<i32>(params.map_size));
                if (!in_map) {
                    continue;
                }
                if (textureLoad(wall_mask_read, ncoord, 0).x > 0.5) {
                    continue; // сосед — тоже стена, света там нет
                }
                let w = 1.0 / f32(radius);
                sum = sum + textureLoad(src, ncoord, 0).rgb * w;
                weight = weight + w;
            }
        }
        if (weight > 0.0) {
            let spilled = clamp(sum / weight, vec3<f32>(0.0), vec3<f32>(1.0)) * WALL_BLEED_BOOST;
            color = vec4<f32>(spilled, color.a);
        }
        // `MergeWallLayer` перезаписывает свет стен: мягкая маска видимости их
        // не гасит — ВИДИМЫЕ стены гасит только hard FOV ниже.
        occlusion = 1.0;
    }
    color = color * occlusion;
    // Hard FOV (`fov.swsl`, `ApplyFovToBuffer` при `DrawHardFov = true`): точка
    // за ВЫХОДОМ из первого тела стены — непрозрачный чёрный (rgb = 0 ⇒
    // оверлей-умножение даёт чистый чёрный; alpha = 1 — для CPU-пути). Именно
    // этот проход прячет стены и двери ЗА другими стенами, которых раньше было
    // видно: маска их не гасила, а ambient давал alpha < 1.
    if (params.fov_range <= 0.5 && our_dist > fov_far_depth(rel) - HARD_FOV_BIAS) {
        color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    } else {
        // Тьма оверлея только от итогового цвета: alpha = 1 − свет.
        let luminance = clamp(dot(color.rgb, vec3<f32>(0.299, 0.587, 0.114)), 0.0, 1.0);
        color = vec4<f32>(color.rgb, 1.0 - luminance);
    }
    textureStore(dst, vec2<i32>(i32(id.x), i32(id.y)), color);
}
