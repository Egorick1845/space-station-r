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

/// Тянется ли предмет: в сборке это КОМПОНЕНТ `Pullable`, а не размер — он стоит
/// на `BaseItem` (`Resources/Prototypes/Entities/Objects/base_item.yml:62`) и на
/// `BaseStructure` (`.../Structures/base_structure.yml:27`), то есть тянуть можно
/// любой предмет и любую конструкцию. Флаг берётся из прототипа
/// (`ItemSet::pullable`); прежнее правило «только от 2×2» было выдумкой.
pub const PULLABLE_BY_DEFAULT: bool = true;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pullable_comes_from_prototype_not_size() {
        // `Pullable` есть и на `BaseItem`, и на `BaseStructure`: тянуть можно
        // всё, размер роли не играет (в сборке это компонент, не порог размера).
        let set = crate::items::ItemSet::load(
            &crate::assets_root().join("prototypes/items.ron"),
        )
        .expect("items.ron");
        for id in ["Crowbar", "SteelSheet", "ToolboxRed", "Medkit", "Paper"] {
            assert!(
                set.pullable(id),
                "{id} должен тянуться (`Pullable` у `BaseItem`)"
            );
        }
    }

    #[test]
    fn slack_is_the_engine_fifteen_centimetres() {
        // 0.15 м = 4.8 мировых единицы при 32 px/м.
        assert!((PULL_SLACK_UNITS - 4.8).abs() < 1e-3);
        assert!((PULL_SPEED_MODIFIER - 0.95).abs() < 1e-6);
        assert!(PULL_BREAK_UNITS > PULL_MIN_LENGTH_UNITS + PULL_SLACK_UNITS);
    }
}
