//! Тянуть за собой (Pull) — перенос `Content.Shared/Movement/Pulling` из сборки.
//!
//! В движке тянущий и тянемый связаны distance-суставом: длина = расстоянию
//! между центрами в момент захвата, максимальная длина = длина + 0.15 м
//! (`PullingSystem.cs`, «Add an additional 15cm»), жёсткость нулевая — внутри
//! диапазона сустав не мешает, на границе тянет объект за игроком. Ходить при
//! этом можно чуть медленнее: `PullerComponent.WalkSpeedModifier = 0.95`
//! (и `SprintSpeedModifier = 0.95`).
//!
//! Формулы и числа держим в ядре: их проверяют тесты, а сервер только применяет.

/// Запас «верёвки» сверх длины захвата, в мировых единицах: 0.15 м из сборки
/// при 32 px на тайл (метр = тайл).
pub const PULL_SLACK_UNITS: f32 = 0.15 * 32.0;

/// Мировая длина верёвки в момент захвата (тайл — расстояние между центрами,
/// ближе игрок всё равно не подойдёт из-за коллизии).
pub const PULL_MIN_LENGTH_UNITS: f32 = 32.0;

/// Скорость тянущего (`WalkSpeedModifier` / `SprintSpeedModifier` = 0.95).
pub const PULL_SPEED_MODIFIER: f32 = 0.95;

/// Разрыв связи: сустав в движке жёсткий и сам не рвётся, но у нас телепорты и
/// спавн — при таком расхождении тянуть уже нельзя (дальше просто «верёвка»
/// растянулась бы через полкарты).
pub const PULL_BREAK_UNITS: f32 = 4.0 * 32.0;

/// Тянется ли предмет такого размера: в сборке `PullableComponent` стоит на
/// крупных объектах (ящики, шкафы, машины), а мелочь игрок носит в руках.
/// Порог — 2×2 клетки (`Normal` и больше, `ItemSize` в SS14).
pub fn is_pullable(size: (u8, u8)) -> bool {
    size.0 >= 2 && size.1 >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_follow_item_size_prototypes() {
        // Мелкое носят в руках, крупное (ящик, шкаф) — тянут.
        for (name, size, expected) in [
            ("Tiny 1×1", (1, 1), false),
            ("Small 1×2", (1, 2), false),
            ("Normal 2×2", (2, 2), true),
            ("Large 4×2", (4, 2), true),
            ("Huge 4×4", (4, 4), true),
        ] {
            assert_eq!(is_pullable(size), expected, "{name}");
        }
        // Реальные размеры из каталога предметов (assets/prototypes/items.ron).
        let set = crate::items::ItemSet::load(
            &crate::assets_root().join("prototypes/items.ron"),
        )
        .expect("items.ron");
        // Лом длинный, но узкий — его носят в руке, а не тянут.
        assert!(!is_pullable(set.size_of("Crowbar")));
        assert!(!is_pullable(set.size_of("SteelSheet")));
        // Ящики в игре — отдельные сущности `Container`, но крупная кладь
        // (рюкзак, аптечка, ящик для инструментов) тоже тянется.
        assert!(is_pullable(set.size_of("ToolboxRed")));
        assert!(is_pullable(set.size_of("Medkit")));
    }

    #[test]
    fn slack_is_the_engine_fifteen_centimetres() {
        // 0.15 м = 4.8 мировых единицы при 32 px/м.
        assert!((PULL_SLACK_UNITS - 4.8).abs() < 1e-3);
        assert!((PULL_SPEED_MODIFIER - 0.95).abs() < 1e-6);
        assert!(PULL_BREAK_UNITS > PULL_MIN_LENGTH_UNITS + PULL_SLACK_UNITS);
    }
}
