//! Структуры мира из прототипов сборки (столы, машины, шкафы): данные, которые
//! нужны и серверу (коллизия, «поверхность»), и клиенту (спрайт, соединение
//! соседних структур — `IconSmooth`).
//!
//! Числа (`Fixtures`, `PlaceableSurface`, `IconSmooth`, `DrawDepth`) берутся из
//! прототипа: импортёр складывает их в [`crate::prototypes::Proto`], а сервер и
//! клиент только применяют.

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Структура из прототипа (`TableSteel`, `Airlock`, `ComputerFrame`, …).
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Structure {
    /// id прототипа — по нему клиент берёт спрайт и сглаживание.
    pub proto: String,
    /// Ключ `IconSmooth` (`table`, `walls`, …): соседние структуры с тем же
    /// ключом соединяются спрайтами. `None` — соединений нет.
    #[serde(default)]
    pub smooth: Option<String>,
    /// `PlaceableSurface`: на структуру можно класть предметы.
    #[serde(default)]
    pub surface: bool,
    /// Есть ли твёрдая фикстура (`Fixtures` с `hard: true`) — коллизия.
    #[serde(default)]
    pub solid: bool,
}

/// Углы структуры — порядок слоёв как в движке (`CornerLayers`: SE, NE, NW, SW).
pub const CORNERS: [&str; 4] = ["SE", "NE", "NW", "SW"];

/// Смещение направления слоя (`IconSmoothSystem.SetCornerLayers:94-101`):
/// SE — `DirectionOffset.None = 0`, NE — `CounterClockwise = 2`,
/// NW — `Flip = 3`, SW — `Clockwise = 1` (`SpriteComponent.cs:1076-1094`).
pub const CORNER_DIR_OFFSETS: [u8; 4] = [0, 2, 3, 1];

/// Флаги заполнения угла (`CornerFill` в `IconSmoothSystem.cs:523-538`).
const CORNER_CCW: u8 = 1;
const CORNER_DIAGONAL: u8 = 2;
const CORNER_CW: u8 = 4;

/// Флаги заполнения четырёх углов по восьми соседям — дословный перенос
/// `IconSmoothSystem.CalculateCornerFill` (`IconSmoothSystem.cs:430-508`):
/// углы `[SE, NE, NW, SW]`, соседи `[N, NE, E, SE, S, SW, W, NW]`.
///
/// Карта из движка: север даёт NE«против часовой» и NW«по часовой»; восток —
/// NE«по часовой» и SE«против часовой»; юг — SE«по часовой» и SW«против
/// часовой»; запад — SW«по часовой» и NW«против часовой»; диагональный сосед
/// закрывает свой угол целиком.
pub fn corner_fills(neighbours: [bool; 8]) -> [u8; 4] {
    let [
        north,
        north_east,
        east,
        south_east,
        south,
        south_west,
        west,
        north_west,
    ] = neighbours;
    // Индексы углов: 0 — SE, 1 — NE, 2 — NW, 3 — SW.
    let mut corners = [0u8, 0, 0, 0];
    if north {
        corners[1] |= CORNER_CCW;
        corners[2] |= CORNER_CW;
    }
    if north_east {
        corners[1] |= CORNER_DIAGONAL;
    }
    if east {
        corners[1] |= CORNER_CW;
        corners[0] |= CORNER_CCW;
    }
    if south_east {
        corners[0] |= CORNER_DIAGONAL;
    }
    if south {
        corners[0] |= CORNER_CW;
        corners[3] |= CORNER_CCW;
    }
    if south_west {
        corners[3] |= CORNER_DIAGONAL;
    }
    if west {
        corners[3] |= CORNER_CW;
        corners[2] |= CORNER_CCW;
    }
    if north_west {
        corners[2] |= CORNER_DIAGONAL;
    }
    corners
}

/// Маска соединений для [`IconSmooth`]: какие из четырёх соседей имеют тот же
/// ключ сглаживания. Порядок битов — как в движке (`IconSmoothComponent`):
/// `1` — север, `2` — восток, `4` — юг, `8` — запад.
pub fn smooth_mask(north: bool, east: bool, south: bool, west: bool) -> u8 {
    u8::from(north) | (u8::from(east) << 1) | (u8::from(south) << 2) | (u8::from(west) << 3)
}

/// Имя состояния спрайта по маске: в сборке `IconSmooth` подставляет маску в
/// состояние базового спрайта (`{base}{mask}`), а `0` (нет соседей) — одиночная
/// структура. Для столов база — `state_`, поэтому состояния `state_0`…`state_7`.
pub fn smooth_state(base: &str, mask: u8) -> String {
    format!("{base}{mask}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Биты маски — как в движке: 1 север, 2 восток, 4 юг, 8 запад.
    #[test]
    fn mask_bits_match_engine_order() {
        assert_eq!(smooth_mask(false, false, false, false), 0);
        assert_eq!(smooth_mask(true, false, false, false), 1);
        assert_eq!(smooth_mask(false, true, false, false), 2);
        assert_eq!(smooth_mask(false, false, true, false), 4);
        assert_eq!(smooth_mask(false, false, false, true), 8);
        assert_eq!(smooth_mask(true, true, true, true), 15);
    }

    /// Состояние спрайта: база + маска/флаг (`state_` + 0…7).
    #[test]
    fn state_is_base_plus_mask() {
        assert_eq!(smooth_state("state_", 0), "state_0");
        assert_eq!(smooth_state("state_", 5), "state_5");
        assert_eq!(smooth_state("state_", 7), "state_7");
    }

    /// Углы по алгоритму движка: одиночный стол — все углы пустые, сосед с
    /// севера закрывает NE «против часовой» и NW «по часовой» и т. д.
    #[test]
    fn corner_fills_match_engine() {
        // Соседи: [N, NE, E, SE, S, SW, W, NW], углы: [SE, NE, NW, SW].
        assert_eq!(corner_fills([false; 8]), [0, 0, 0, 0]);
        // Только север: NE |= CCW(1), NW |= CW(4).
        assert_eq!(
            corner_fills([true, false, false, false, false, false, false, false]),
            [0, 1, 4, 0]
        );
        // Только восток: NE |= CW(4), SE |= CCW(1).
        assert_eq!(
            corner_fills([false, false, true, false, false, false, false, false]),
            [1, 4, 0, 0]
        );
        // Только юг: SE |= CW(4), SW |= CCW(1).
        assert_eq!(
            corner_fills([false, false, false, false, true, false, false, false]),
            [4, 0, 0, 1]
        );
        // Только запад: SW |= CW(4), NW |= CCW(1).
        assert_eq!(
            corner_fills([false, false, false, false, false, false, true, false]),
            [0, 0, 1, 4]
        );
        // Диагонали закрывают свой угол целиком (Diagonal = 2).
        assert_eq!(
            corner_fills([false, true, false, false, false, false, false, false]),
            [0, 2, 0, 0]
        );
        assert_eq!(
            corner_fills([false, false, false, true, false, false, false, false]),
            [2, 0, 0, 0]
        );
        assert_eq!(
            corner_fills([false, false, false, false, false, true, false, false]),
            [0, 0, 0, 2]
        );
        assert_eq!(
            corner_fills([false, false, false, false, false, false, false, true]),
            [0, 0, 2, 0]
        );
        // Север + восток: у NE сходятся CCW(1), CW(4) и диагональ NE(2) = 7.
        assert_eq!(
            corner_fills([true, true, true, false, false, false, false, false]),
            [1, 7, 4, 0]
        );
        // Все восемь соседей: каждый угол = 1|2|4 = 7.
        assert_eq!(corner_fills([true; 8]), [7, 7, 7, 7]);
    }

    /// Смещения направлений слоёв — из `SetCornerLayers` (SE, NE, NW, SW).
    #[test]
    fn corner_direction_offsets_match_engine() {
        assert_eq!(CORNER_DIR_OFFSETS, [0, 2, 3, 1]);
        assert_eq!(CORNERS, ["SE", "NE", "NW", "SW"]);
    }
}
