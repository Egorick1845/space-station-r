//! Размеры предметов — перенос `Resources/Prototypes/item_size.yml` сборки
//! (`type: itemSize`).
//!
//! Числа 1:1 из файла: `weight` (сравнивается по весу — `ItemSizePrototype`
//! реализует сравнение только по весу) и `defaultShape` — список `Box2i` с
//! ВКЛЮЧИТЕЛЬНЫМИ границами `left,bottom,right,top`
//! (`RobustToolbox/Robust.Shared.Maths/Box2i.cs:116-126`). Поэтому число клеток
//! по боксу = `(right-left+1) × (top-bottom+1)`.
//!
//! Пустая форма-`Shape` на предмете переопределяет форму, но НЕ вес
//! (`ItemComponent.Shape` :47, `SharedItemSystem.GetItemShape` :186-192).

/// Один размер предмета (`type: itemSize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemSize {
    /// `id:` прототипа (`Tiny`, `Small`, …).
    pub id: &'static str,
    /// `weight:` — по нему сравниваются размеры («влезает ли в карман»).
    pub weight: u32,
    /// `defaultShape:` — боксы `(left, bottom, right, top)`, границы включительно.
    pub shape: &'static [(i32, i32, i32, i32)],
}

/// Все размеры из `item_size.yml` (порядок файла).
pub const ITEM_SIZES: &[ItemSize] = &[
    ItemSize {
        id: "Tiny",
        weight: 1,
        shape: &[(0, 0, 0, 0)],
    },
    ItemSize {
        id: "Small",
        weight: 2,
        shape: &[(0, 0, 0, 1)],
    },
    ItemSize {
        id: "Normal",
        weight: 4,
        shape: &[(0, 0, 1, 1)],
    },
    ItemSize {
        id: "Large",
        weight: 8,
        shape: &[(0, 0, 3, 1)],
    },
    ItemSize {
        id: "Huge",
        weight: 16,
        shape: &[(0, 0, 3, 3)],
    },
    ItemSize {
        id: "Ginormous",
        weight: 32,
        shape: &[(0, 0, 5, 5)],
    },
];

/// Максимальный размер, который влезает в карман
/// (`InventorySystem.Equip.cs:39` — `PocketableItemSize = "Small"`).
pub const POCKETABLE_SIZE: &str = "Small";

impl ItemSize {
    /// Габариты формы в клетках: `(ширина, высота)` по bounding box
    /// (`StorageHelpers.GetBoundingBox`: min left/bottom, max right/top).
    pub fn cells(&self) -> (u8, u8) {
        let mut left = i32::MAX;
        let mut bottom = i32::MAX;
        let mut right = i32::MIN;
        let mut top = i32::MIN;
        for (l, b, r, t) in self.shape {
            left = left.min(*l);
            bottom = bottom.min(*b);
            right = right.max(*r);
            top = top.max(*t);
        }
        if left > right || bottom > top {
            return (1, 1);
        }
        ((right - left + 1) as u8, (top - bottom + 1) as u8)
    }

    /// Площадь формы: сколько клеток реально покрыто боксами
    /// (`StorageHelpers.GetArea` — дырки в мульти-боксах не считаются).
    pub fn area(&self) -> u32 {
        let mut cells = 0;
        for (l, b, r, t) in self.shape {
            for y in *b..=*t {
                for x in *l..=*r {
                    cells += 1;
                    let _ = (x, y);
                }
            }
        }
        cells
    }
}

/// Размер по id прототипа.
pub fn by_id(id: &str) -> Option<&'static ItemSize> {
    ITEM_SIZES.iter().find(|size| size.id == id)
}

/// Габариты размера в клетках (`None` — неизвестный id).
pub fn cells_of(id: &str) -> Option<(u8, u8)> {
    by_id(id).map(ItemSize::cells)
}

/// Вес размера (`None` — неизвестный id).
pub fn weight_of(id: &str) -> Option<u32> {
    by_id(id).map(|size| size.weight)
}

/// Влезает ли предмет такого размера в карман: вес ≤ веса `Small`
/// (`InventorySystem.Equip.cs:262-270`).
pub fn pocketable(id: &str) -> bool {
    match (weight_of(id), weight_of(POCKETABLE_SIZE)) {
        (Some(item), Some(pocket)) => item <= pocket,
        _ => false,
    }
}

/// Сравнение размеров по весу (`ItemSizePrototype` сравнивает только вес):
/// `true`, если `left` не больше `right`.
pub fn fits(left: &str, right: &str) -> bool {
    match (weight_of(left), weight_of(right)) {
        (Some(left), Some(right)) => left <= right,
        _ => false,
    }
}

/// Габариты явной формы предмета (`Item.shape`): объединяющий прямоугольник
/// всех боксов. `None` — формы нет, берётся `defaultShape` размера.
/// У лома в сборке `shape: [0,0,0,1]` при размере `Normal` → 1×2 клетки.
pub fn cells_of_shape(shape: &[(i32, i32, i32, i32)]) -> Option<(u8, u8)> {
    let first = shape.first()?;
    let (mut left, mut bottom, mut right, mut top) = *first;
    for (l, b, r, t) in shape.iter().skip(1) {
        left = left.min(*l);
        bottom = bottom.min(*b);
        right = right.max(*r);
        top = top.max(*t);
    }
    Some((
        (right - left + 1).clamp(1, u8::MAX as i32) as u8,
        (top - bottom + 1).clamp(1, u8::MAX as i32) as u8,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Габариты и вес — ровно как в `item_size.yml` сборки.
    #[test]
    fn sizes_match_engine_table() {
        let expected = [
            ("Tiny", 1, (1, 1)),
            ("Small", 2, (1, 2)),
            ("Normal", 4, (2, 2)),
            ("Large", 8, (4, 2)),
            ("Huge", 16, (4, 4)),
            ("Ginormous", 32, (6, 6)),
        ];
        for (id, weight, cells) in expected {
            let size = by_id(id).unwrap_or_else(|| panic!("нет размера {id}"));
            assert_eq!(size.weight, weight, "вес {id}");
            assert_eq!(size.cells(), cells, "габариты {id}");
            assert_eq!(size.area(), cells.0 as u32 * cells.1 as u32, "площадь {id}");
        }
    }

    /// «Стопка стали» (SheetSteel, `Item size: Normal`) занимает 2×2 = 4 клетки.
    #[test]
    fn sheet_is_two_by_two() {
        assert_eq!(cells_of("Normal"), Some((2, 2)));
        assert_eq!(weight_of("Normal"), Some(4));
    }

    /// В карман влезают только Tiny и Small (вес ≤ 2), Normal — уже нет.
    #[test]
    fn pocket_fits_small_and_below() {
        assert!(pocketable("Tiny"));
        assert!(pocketable("Small"));
        assert!(!pocketable("Normal"), "сталь в карман не влезает");
        assert!(!pocketable("Huge"));
        assert!(fits("Small", "Small"), "равный размер разрешён");
        assert!(fits("Tiny", "Small"));
        assert!(!fits("Normal", "Small"));
    }

    /// Явная форма перекрывает размер: лом 1×2 при размере `Normal`.
    #[test]
    fn explicit_shape_overrides_size_default() {
        // Лом: `Item { size: Normal, shape: [0,0,0,1] }` → 1 клетка в ширину,
        // 2 в высоту (`Resources/Prototypes/Entities/Objects/Tools/crowbars.yml:57-61`).
        assert_eq!(cells_of_shape(&[(0, 0, 0, 1)]), Some((1, 2)));
        // Сталь: `Normal` без формы → 2×2 (`defaultShape` размера).
        assert_eq!(cells_of_shape(&[]), None);
        assert_eq!(cells_of("Normal"), Some((2, 2)));
        // Несколько боксов — объединяющий прямоугольник.
        assert_eq!(cells_of_shape(&[(0, 0, 0, 0), (2, 0, 3, 0)]), Some((4, 1)));
        assert_eq!(cells_of_shape(&[(0, 0, 0, 0), (0, 2, 0, 3)]), Some((1, 4)));
    }
}
