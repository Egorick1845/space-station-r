//! Математика освещения движка SS14 — дословный перенос формул из
//! `RobustToolbox/Resources/Shaders/light_shared.swsl`, `shadow_cast_shared.swsl`
//! и `light-soft.swsl` (см. SS14_LIGHTING.md). Живёт в ядре, чтобы:
//!   * GPU-шейдер (`assets/shaders/light.wgsl`) и CPU-фолбэк считали одним и тем
//!     же способом;
//!   * числа, снятые со сборки, были закреплены тестами (коридорная лампа
//!     radius 10 / energy 0.8 / falloff 6.8 → таблица яркостей из отчёта).
//!
//! Модуль чистый: без Bevy-компонентов и без сети (PLAN.md §6).

/// Высота источника над полом (`LIGHTING_HEIGHT` в `light_shared.swsl`).
pub const LIGHTING_HEIGHT: f32 = 1.0;
/// Затухание по умолчанию (`SharedPointLightComponent.Falloff`).
pub const DEFAULT_FALLOFF: f32 = 6.8;
/// Идентификатор «нет окклюдера»: в движке клир-цвет карты теней 1234.
pub const NO_OCCLUDER: f32 = 1234.0;

/// Формула затухания `light_shared.swsl`:
/// `s = sqrt(sqr_dist)/radius; val = ((1−s²)²)/(1 + falloff·mix(s, s², curve)) · energy`.
///
/// `sqr_dist` — квадрат расстояния **плюс** [`LIGHTING_HEIGHT`] (в шейдере это
/// `dot(diff, diff) + LIGHTING_HEIGHT`), поэтому в этот аргумент передаётся
/// `d² + 1.0`, а не просто `d²`.
pub fn attenuation(sqr_dist: f32, radius: f32, energy: f32, falloff: f32, curve: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    let s = (sqr_dist.max(0.0f32).sqrt() / radius).clamp(0.0, 1.0);
    let s2 = s * s;
    let curve_factor = s + (s2 - s) * curve.clamp(0.0, 1.0);
    let val = ((1.0 - s2) * (1.0 - s2)) / (1.0 + falloff * curve_factor);
    val.clamp(0.0, 1.0) * energy
}

/// `ChebyshevUpperBound` из `shadow_cast_shared.swsl` (VSM): вероятность того,
/// что точка на расстоянии `t` **не** затенена при моментах `(d, d²)`.
pub fn chebyshev_upper_bound(moments: (f32, f32), t: f32) -> f32 {
    let p: f32 = if t <= moments.0 { 1.0 } else { 0.0 };
    let variance = (moments.1 - moments.0 * moments.0).max(0.0);
    let d = t - moments.0;
    let p_max = variance / (variance + d * d);
    p.max(p_max)
}

/// Полярная координата угла: в движке `deflect = atan(rel.y, -rel.x)/PI`,
/// `u = (deflect + 1)/2` — так угол отображается в 512-биновую строку карты теней.
///
/// ВНИМАНИЕ: это UV движка (шов на востоке, отсчёт по часовой стрелке). Наши
/// генераторы карт (`bin_distance`) пишут бины в своём порядке — для выборки из
/// НАШИХ карт используйте [`polar_bin`], иначе половина окружности вырождается.
pub fn polar_u(dx: f32, dy: f32) -> f32 {
    (dy.atan2(-dx) / std::f32::consts::PI + 1.0) * 0.5
}

/// Дробный индекс бина полярной карты для направления `(dx, dy)` — ИНВЕРСИЯ
/// генерации [`bin_distance`]: бин `b` смотрит под углом `b/bins·2π − π`
/// (0 — запад, дальше против часовой стрелки: юг, восток, север, шов на западе).
/// Дробный — под линейную интерполяцию соседних бинов (`Filter = true` в движке);
/// читающий обязан взять `floor` и завёрнутые `rem_euclid` соседей.
pub fn polar_bin(dx: f32, dy: f32, bins: usize) -> f32 {
    (dy.atan2(dx) + std::f32::consts::PI) / (2.0 * std::f32::consts::PI) * bins as f32 - 0.5
}

/// Расстояние от `origin` до отрезка `a..b` вдоль луча `dir` (единичного).
/// `None`, если луч не пересекает отрезок (в движке это делает `shadow-depth.frag`
/// аналитически: `dist = |z / cos(x − y)|`).
pub fn ray_segment_distance(
    origin: (f32, f32),
    dir: (f32, f32),
    a: (f32, f32),
    b: (f32, f32),
) -> Option<f32> {
    let e = (b.0 - a.0, b.1 - a.1);
    let denom = dir.0 * e.1 - dir.1 * e.0;
    if denom.abs() < 1e-6 {
        return None; // параллельно
    }
    let diff = (a.0 - origin.0, a.1 - origin.1);
    let t = (diff.0 * e.1 - diff.1 * e.0) / denom; // расстояние по лучу
    let s = (diff.0 * dir.1 - diff.1 * dir.0) / denom; // параметр на отрезке
    if t <= 0.0 || !(0.0..=1.0).contains(&s) {
        return None;
    }
    Some(t)
}

/// Ближайшее расстояние до стены по углу `angle` (радианы) от точки `origin` —
/// один бин полярной карты теней. Стены — отрезки окклюдеров (`ssr_core::occluders`).
pub fn bin_distance(
    origin: (f32, f32),
    angle: f32,
    walls: impl IntoIterator<Item = ((f32, f32), (f32, f32))>,
) -> f32 {
    let dir = (angle.cos(), angle.sin());
    let mut best = NO_OCCLUDER;
    for (a, b) in walls {
        if let Some(dist) = ray_segment_distance(origin, dir, a, b) {
            best = best.min(dist);
        }
    }
    best
}

/// Допуск кластера входов в тело стены: соседние тайлы одной стены входятся по
/// лучу вплотную (общая грань), поэтому входы ближе этого допуска — одно тело.
/// Зазор между разными конструкциями на сетке тайлов всегда больше тайла.
pub const FOV_CLUSTER_EPS: f32 = 0.5;
/// Bias hard FOV из `fov.swsl` движка (`−0.75/32` бина глубины на шкале тайла):
/// точка за ВЫХОДОМ из первого тела стены получает непрозрачную тьму, а само
/// тело (в том числе его дальняя грань с допуском) остаётся видно — «смотрим
/// внутрь стен», `ApplyFovToBuffer` при `DrawHardFov = true`.
pub const HARD_FOV_BIAS: f32 = 0.75;

/// Луч против прямоугольника (slab-тест): `(вход, выход)` по лучу с единичным
/// `dir`, `None` — если луч не проходит через тело прямоугольника.
pub fn ray_rect_span(
    origin: (f32, f32),
    dir: (f32, f32),
    rect: (f32, f32, f32, f32),
) -> Option<(f32, f32)> {
    let mut enter = f32::NEG_INFINITY;
    let mut exit = f32::INFINITY;
    for (o, d, lo, hi) in [
        (origin.0, dir.0, rect.0, rect.2),
        (origin.1, dir.1, rect.1, rect.3),
    ] {
        if d.abs() < 1e-6 {
            // Луч параллелен плоскости: попадание только если origin внутри плиты.
            if o < lo || o > hi {
                return None;
            }
            continue;
        }
        let inv = 1.0 / d;
        let (mut t0, mut t1) = ((lo - o) * inv, (hi - o) * inv);
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        enter = enter.max(t0);
        exit = exit.min(t1);
    }
    if enter > exit || exit <= 0.0 {
        return None;
    }
    Some((enter.max(0.0), exit))
}

/// Расстояние по лучу до ВЫХОДА из первого тела стены (кластера сомкнутых
/// прямоугольников тайлов) — аналог front-face culling в движке: карта FOV
/// рендерится второй раз с `CullFaceMode.Front`, чтобы «смотреть внутрь стен»
/// (`DrawFov`, `Clyde.LightRendering.cs:237-265`). Точка дальше этого выхода
/// скрыта hard FOV непрозрачным чёрным (`fov.swsl`, `occludeColor = Black`).
///
/// `rects` — тела тайлов `(x0, y0, x1, y1)` в тех же единицах, что `origin`.
/// Возвращает [`NO_OCCLUDER`], если по лучу стен нет вовсе.
pub fn first_body_exit(
    origin: (f32, f32),
    dir: (f32, f32),
    rects: impl IntoIterator<Item = (f32, f32, f32, f32)>,
) -> f32 {
    let rects: Vec<(f32, f32, f32, f32)> = rects.into_iter().collect();
    let mut near = NO_OCCLUDER;
    for &rect in &rects {
        if let Some((enter, _)) = ray_rect_span(origin, dir, rect) {
            near = near.min(enter);
        }
    }
    if near >= NO_OCCLUDER {
        return NO_OCCLUDER;
    }
    // Кластер расширяется, пока среди тайлов с входом не дальше текущего выхода
    // находятся более дальние выходы (луч идёт вдоль стены через соседние тайлы).
    let mut far = near;
    loop {
        let mut best = far;
        for &rect in &rects {
            if let Some((enter, exit)) = ray_rect_span(origin, dir, rect)
                && enter <= far + FOV_CLUSTER_EPS
            {
                best = best.max(exit);
            }
        }
        if best <= far + 1e-4 {
            return far;
        }
        far = best;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Таблица яркостей коридорной лампы из отчёта по сборке
    /// (`Poweredlight`: radius 10, energy 0.8, falloff 6.8, curve 0).
    /// Это закрепление чисел движка: если формула изменится — тест поймает.
    #[test]
    fn corridor_lamp_brightness_matches_engine() {
        let (radius, energy, falloff, curve) = (10.0, 0.8, DEFAULT_FALLOFF, 0.0);
        let value = |d: f32| attenuation(d * d + LIGHTING_HEIGHT, radius, energy, falloff, curve);
        for (d, expected) in [
            (0.0, 0.4667),
            (1.0, 0.3917),
            (2.0, 0.2864),
            (3.0, 0.2057),
            (5.0, 0.0981),
            (7.0, 0.0344),
            (9.0, 0.0036),
        ] {
            let got = value(d);
            assert!(
                (got - expected).abs() < 0.0005,
                "на {d} м ожидалось {expected}, получено {got}"
            );
        }
        // На радиусе лампа гаснет полностью (s = 1 → (1 − s²)² = 0).
        assert_eq!(value(10.0), 0.0);
    }

    #[test]
    fn wall_blocks_the_ray() {
        // Стена — вертикальный отрезок x = 64 (2 тайла) от y = -32 до y = 32.
        let walls = [((64.0, -32.0), (64.0, 32.0))];
        // Луч из начала координат строго на восток упирается в стену на 64.
        let east = bin_distance((0.0, 0.0), 0.0, walls);
        assert!((east - 64.0).abs() < 1e-3, "восток: {east}");
        // Луч на север стены не касается.
        let north = bin_distance((0.0, 0.0), std::f32::consts::FRAC_PI_2, walls);
        assert_eq!(north, NO_OCCLUDER, "север не должен упираться в стену");
    }

    #[test]
    fn chebyshev_is_one_when_not_occluded() {
        // Точка ближе записанного расстояния — свет не затенён.
        assert_eq!(chebyshev_upper_bound((10.0, 100.0), 5.0), 1.0);
        // Точка далеко за окклюдером — почти полная тень.
        let shadowed = chebyshev_upper_bound((10.0, 100.0), 30.0);
        assert!(shadowed < 0.05, "ожидалась тень, получено {shadowed}");
    }

    #[test]
    fn polar_maps_cardinal_directions() {
        // Запад (−1, 0): atan2(0, 1) = 0 → u = 0.5 (начало отсчёта бинов).
        assert!((polar_u(-1.0, 0.0) - 0.5).abs() < 1e-6);
        // Восток (1, 0): atan2(0, −1) = π → u = 1.0 (шов карты).
        assert!((polar_u(1.0, 0.0) - 1.0).abs() < 1e-6);
        // Север (0, 1) → atan2(1, -0) = π/2 → u = 0.75.
        assert!((polar_u(0.0, 1.0) - 0.75).abs() < 1e-6);
        // Юг (0, −1) → −π/2 → u = 0.25.
        assert!((polar_u(0.0, -1.0) - 0.25).abs() < 1e-6);
    }

    /// `polar_bin` обязан быть ОБРАТНОЙ к генерации карты: направление → бин,
    /// который смотрит туда же. Ошибка тут (потерянный множитель 1/2, знак `-dx`)
    /// даёт чёрные клинья «тумана войны»: полкарты вырождается в один бин.
    #[test]
    fn polar_bin_inverts_generation() {
        const BINS: usize = 512;
        // Центр бина `b` смотрит под углом `b/BINS·2π − π` (как в генерации).
        let angle_of =
            |b: f32| (b / BINS as f32) * 2.0 * std::f32::consts::PI - std::f32::consts::PI;
        // Для каждого кардинального направления выборка должна попасть в бин,
        // чей угол совпадает с направлением (в пределах одного бина).
        for (name, dx, dy) in [
            ("восток", 1.0f32, 0.0f32),
            ("север", 0.0, 1.0),
            ("запад", -1.0, 0.0),
            ("юг", 0.0, -1.0),
        ] {
            let b = polar_bin(dx, dy, BINS);
            let angle = angle_of(b);
            let sampled = (angle.cos(), angle.sin());
            let dot = sampled.0 * dx + sampled.1 * dy;
            assert!(dot > 0.999, "{name}: бин смотрит мимо (dot = {dot})");
        }
        // Круг замкнут: дробный индекс лежит в [−0.5, BINS−0.5).
        for step in 0..64 {
            let angle = step as f32 / 64.0 * 2.0 * std::f32::consts::PI;
            let b = polar_bin(angle.cos(), angle.sin(), BINS);
            assert!(
                (-0.5..BINS as f32 - 0.5).contains(&b),
                "индекс вне круга: {b}"
            );
        }
    }

    /// Hard FOV (§1 SS14_PORT_PLAN): точка на лице первой стены и внутри её тела
    /// видна, точка ЗА телом — скрыта (выход из первого тела стены). Это и есть
    /// ответ на жалобу «видно стены и двери, скрытые за другими стенами».
    /// Единицы — мировые, как в GPU-конвейере (1 тайл = 32 юнита).
    #[test]
    fn first_body_exit_hides_only_behind_first_wall() {
        let rects = [(0.0f32, 0.0, 32.0, 32.0)];
        let east = (1.0f32, 0.0f32);
        // Глаз в (−96, 16): вход в тело на t = 96, выход на t = 128.
        let far = first_body_exit((-96.0, 16.0), east, rects);
        assert!((far - 128.0).abs() < 1e-3, "выход из тела: {far}");
        // Лицо стены (96) и середина тела (112) видны («внутрь стен»).
        assert!(96.0 < far - HARD_FOV_BIAS);
        assert!(112.0 < far - HARD_FOV_BIAS);
        // Пол за стеной (160 > выхода 128) — скрыт.
        assert!(160.0 > far - HARD_FOV_BIAS);
        // Без стен — ничего не скрыто.
        assert_eq!(first_body_exit((-96.0, 16.0), east, []), NO_OCCLUDER);
    }

    /// Кластер: луч идёт ВДОЛЬ стены из трёх сомкнутых тайлов — выход по концу
    /// кластера, а не по первому тайлу (в движке это сплошной полигон стены).
    #[test]
    fn first_body_exit_follows_contiguous_cluster() {
        let rects = [
            (0.0f32, 0.0, 32.0, 32.0),
            (32.0, 0.0, 64.0, 32.0),
            (64.0, 0.0, 96.0, 32.0),
        ];
        // Луч из x = −32 через все три тайла вдоль стены: выход на x = 96 → t = 128.
        let far = first_body_exit((-32.0, 16.0), (1.0, 0.0), rects);
        assert!((far - 128.0).abs() < 1e-3, "выход из кластера: {far}");
        // Третий тайл через зазор: кластер — два тайла (выход на x = 64 → t = 96),
        // отдельно стоящий тайл его не расширяет.
        let gapped = [
            (0.0f32, 0.0, 32.0, 32.0),
            (32.0, 0.0, 64.0, 32.0),
            (160.0, 0.0, 192.0, 32.0),
        ];
        let far = first_body_exit((-32.0, 16.0), (1.0, 0.0), gapped);
        assert!((far - 96.0).abs() < 1e-3, "зазор рвёт кластер: {far}");
    }

    /// Диагональный луч через угол: тело одного тайла по диагонали длиннее
    /// тайла — выход обязан быть на дальнем углу.
    #[test]
    fn first_body_exit_handles_diagonal() {
        let rects = [(0.0f32, 0.0, 32.0, 32.0)];
        let eye = (-16.0f32, -16.0f32);
        let len = (eye.0 * eye.0 + eye.1 * eye.1).sqrt();
        let dir = (-eye.0 / len, -eye.1 / len); // к (32, 32) — диагональ тайла
        let far = first_body_exit(eye, dir, rects);
        let expected = len + std::f32::consts::SQRT_2 * 32.0;
        assert!((far - expected).abs() < 1e-2, "диагональ: {far} vs {expected}");
    }
}
