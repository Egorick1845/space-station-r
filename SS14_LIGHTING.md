# Освещение и тени SS14 — точная спецификация движка

Собрано чтением исходников `RobustToolbox` (сабмодуль `C:\ss14\mini-station-goob\RobustToolbox`),
04.10.2026. Файлы и числа — проверены по коду; цитаты шейдеров дословные.
Наша цель: **повторить конвейер 1:1**, а не приближать его.

## 1. Что рисуется и куда (порядок кадра)

Всё живёт в `Robust.Client/Graphics/Clyde/Clyde.LightRendering.cs` (1994 строки).

| Что | Размер/формат | Где в коде |
|---|---|---|
| Карта теней (`_shadowRenderTarget`) | **512 × N**, N = число источников (cvar `light.max_shadowcasting_lights`, по умолчанию **128**), RG32F (2 момента VSM) | `Clyde.LightRendering.cs:1970` |
| Карта FOV (`_fovRenderTarget`) | **2048 × 2** (две строки: обычная FOV и «мягкая» для света) | `:180` |
| Карта света (`LightRenderTarget`) | размер вьюпорта × cvar `light.resolution_scale` = **0.5** | `:1890`, `:1905` |
| Свет-блюр (`LightBlurTarget`) | как карта света | `:1908` |
| Wall-bleed промежуточные | карта света / 4 (в коде `GetLightMapSize(size, true)`) | `:1913`, `:1918` |
| Маска стен (`WallMaskRenderTarget`) | размер вьюпорта, R8 | `:1902` |

Порядок проходов (по коду рендера кадра):
1. Мир рисуется, в **texture unit 1** подана карта света (`Clyde.Rendering.cs:288`) —
   спрайты/сетка умножаются на свет сами (шейдер с `lighting = true`).
2. Карта теней: геометрия окклюдеров (полярные координаты), VSM-моменты.
3. Карта света: для каждого источника рисуется квад его круга, **аддитивно**
   (`SrcAlpha/One`), фрагмент — `light-soft`/`light-hard` (см. §3).
4. Блюр карты света: 3 итерации гаусса по X и Y (`light-blur.swsl`), cvar `light.blur = on`,
   фактор `light.blur_factor = 0.001`, множитель 14 (`BlurRenderTarget`, `:828`).
5. Просачивание света на стены (`BlurOntoWalls`, `:890`) + `MergeWallLayer` (`:946`).
6. FOV-оверлей на мир (`ApplyFovToBuffer`, `:980`): чёрная заливка `alpha = 1 − occl`,
   цвет `render.fov_color` (обычно чёрный), со **стенсилом**.
7. FOV применяется и к карте света (`ApplyLightingFovToBuffer`, `:1010`) — с линейной
   фильтрацией карты FOV (VSM этого требует).

## 2. Карта теней: полярные координаты и VSM

`Resources/Shaders/Internal/shadow_cast_shared.swsl`:

```glsl
highp vec2 occludeDepth(highp vec2 rel, sampler2D shadowMap, highp float mapOffsetY)
{
    // It's a circle now, because it's faster to compute.
    highp float deflect = (atan(rel.y, -rel.x) / PI);
    highp float mapOffsetX = (deflect + 1.0) / 2.0;
    return zClydeShadowDepthUnpack(texture2D(shadowMap, vec2(mapOffsetX, mapOffsetY)));
}

highp float ChebyshevUpperBound(highp vec2 moments, highp float t)
{
    highp float p = float(t <= moments.x);
    highp float variance = moments.y - (moments.x * moments.x);
    variance = max(variance, g_MinVariance);
    highp float d = t - moments.x;
    highp float p_max = variance / (variance + d*d);
    return max(p, p_max);
}
```

Строка карты теней = один источник: `Строка N хранит для каждого угла (атлас 512 px по кругу)
`vec2(расстояние, расстояние²)` — моменты VSM. Окклюдеры пишут максимум расстояния по своему
угловому сектору; свет позже сравнивает своё расстояние с моментом.

## 3. Свет: формула и мягкие тени

`Resources/Shaders/Internal/light_shared.swsl` — общая часть (цитата):

```glsl
highp float mask = zTexture(UV).r;                 // маска FOV (мягкая строка карты FOV)
highp vec2 diff = worldPosition - lightCenter;
highp float occlusion = lightIndex < 0.0 ? 1.0 : createOcclusion(diff);
if (occlusion == 0.0) { discard; }

highp float sqr_dist = dot(diff, diff) + LIGHTING_HEIGHT;   // LIGHTING_HEIGHT = 1.0
highp float s = clamp(sqrt(sqr_dist) / lightRange, 0.0, 1.0);
highp float s2 = s * s;
highp float curveFactor = mix(s, s2, clamp(lightCurveFactor, 0.0, 1.0));
highp float val = clamp(((1.0 - s2) * (1.0 - s2)) / (1.0 + lightFalloff * curveFactor), 0.0, 1.0);
val *= lightPower;
val *= mask;                                        // свет виден только в зоне FOV
COLOR = vec4(lightColor.rgb, val * occlusion);
```

Мягкие тени — `light-soft.swsl`: 7 точек по линии, перпендикулярной лучу света
(`perpendicular = normalize(cross(vec3(diff,0), vec3(0,0,1)).xy) / 32 * lightSoftness * 1.5`),
сигма гаусса `sigma = max(0.001, (наше расстояние − минимальный момент) * 0.75)`,
веса `exp(-o²/(2σ²))` для смещений 0..3 и нормировка на их сумму; каждая точка —
`ChebyshevUpperBound(sample, ourDist)`. `light-hard.swsl` — та же формула без PCF
(`occlusion = length(diff) > moment.x`).

## 4. Блюр и просачивание на стены

`light-blur.swsl` / `wall-bleed-blur.swsl` — один и тот же гаусс:

```glsl
highp vec4 sum = zTexture(pos) * 0.375;
sum += zTexture(blurPos1.xy) * 0.25;   // pos ± offset
sum += zTexture(blurPos1.zw) * 0.25;
sum += zTexture(blurPos2.xy) * 0.0625; // pos ± 2·offset
sum += zTexture(blurPos2.zw) * 0.0625;
```

`offset = vec2(aspect * radius, radius) * direction`, `aspect = size.y / size.x`.

- Блюр карты света (`BlurRenderTarget`): 3 итерации, радиус `scale = (i+1) * factor`,
  `factor = light.blur_factor(0.001) * (14 / cameraSize)`, где
  `cameraSize = eye.Zoom.Y * viewport.Size.Y / RenderScale.Y / PixelsPerMeter` (множитель 14).
- Просачивание на стены (`BlurOntoWalls`): 3 итерации того же гаусса, но **`COLOR = sum * 1.1`**
  (движок добавляет яркость стене: без этого стена была бы вдвое темнее окружения),
  `factor = 7e-3 * (14 / cameraSize)`, работа идёт в **четвертьразмерных** промежуточных
  таргетах, затем `MergeWallLayer` копирует результат на карту света **только по геометрии
  окклюдеров** (маска стен) — то есть «просачивание» касается лишь тайлов стен.

## 5. FOV

- Карта FOV — тот же полярный приём, но шире: **2048** px по кругу, 2 строки, отдельная
  программа расчёта глубины (`_fovCalculationProgram`).
- `fov_shared.swsl`: квад растягивается на весь вьюпорт, `worldSpaceDiff` — смещение от
  центра глаза в мировых единицах (через `clipToDiff`).
- `fov-lighting.swsl`: `occlusion = ChebyshevUpperBound(occludeDepth(diff, ..., 0.25), ourDist)`;
  при `occlusion >= 1.0` — `discard`; иначе `COLOR = vec4(occludeColor.rgb, 1.0 - occlusion)` —
  это и есть «туман войны» (чёрная заливка с альфой непрозрачности).
- Свет умножается на эту же маску (`mask` в `light_shared.swsl`), поэтому свет виден только
  в зоне прямой видимости; дополнительно после всех проходов FOV применяется и к карте света
  (`ApplyLightingFovToBuffer`) со стенсилом.

## 6. Что это значит для нас (план переноса)

Наш текущий подход (BFS по тайлам + размытие всей карты + линейная фильтрация 1 тексель на
тайл) физически не может дать такие тени: в нём нет ни геометрии окклюдеров, ни расстояний.
Поэтому переносим конвейер, а не «улучшаем» приближение:

1. **Геометрия окклюдеров** из тайловой карты (стены/закрытые двери): рёбра тайлов со правилом
   общих граней (в движке `ClientOccluderSystem` отбрасывает общие рёбра соседних стен и
   помечает «выпуклые» вершины — уточнить по отчёту субагента).
2. **Карта теней** 512 × N (N = активные источники, у нас хватит 16–32) — VSM-моменты,
   полярные координаты, тот же `atan(y, -x)/π`.
3. **Карта света** половинного разрешения вьюпорта, аддитивные квады источников с
   `light-soft`-математикой (7 PCF-точек + Chebyshev + точная формула затухания).
4. **Блюр** 3 итерации (веса 0.375/0.25/0.25/0.0625/0.0625), затем **wall-bleed** ×1.1 и
   слияние по маске стен (четвертьразмерные таргеты).
5. **FOV** 2048-полярная карта от глаза, оверлей `alpha = 1 − occl`, та же маска умножается
   на свет.
6. Применение к миру — как в движке: мир × карта света; у нас это делает наш оверлей
   (математически то же), но карта теперь приходит из этого конвейера, а не из тайловой BFS.

Реализация в Bevy: кастомные рендер-проходы (render graph node) + WGSL-порты шейдеров
§2–§4, карты света/теней — `Image` с `RENDER_ATTACHMENT`, оверлей — существующий спрайт тьмы,
у которого текстура теперь GPU-карта.

Открытые вопросы (уточняются отчётом субагента): точные правила построения окклюдеров
(общие рёбра, выпуклые вершины), геометрия квада источника (какие вершины, как считается
`lightRange`/`lightPower` у прототипов), как именно спрайты умножаются на свет
(шейдерный инстанс с `lighting = true`), параметры `FovSetTransformAndBlit`/стенсила.
