//! Каталог предметов (PLAN.md T5.2, ADR-6): 30+ предметов в .ron —
//! идентификатор, название, спрайты (иконка и «в руке»), размер в клетках
//! инвентаря (тетрис) и теги для рецептов.

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Прототип предмета (`assets/prototypes/items.ron`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ItemProto {
    /// Идентификатор (он же `Item.name` в мире, напр. "Crowbar").
    pub id: String,
    /// Отображаемое название (русское).
    pub name: String,
    /// Спрайт иконки: `"path/to/name.rsi#state"`.
    #[serde(default)]
    pub sprite: Option<String>,
    /// Спрайт в руке (обычно inhand-стейт).
    #[serde(default)]
    pub inhand: Option<String>,
    /// Размер в клетках инвентаря: (ширина, высота). Используется, когда не
    /// задан `size_id` (в сборке размер всегда идёт прототипом `itemSize`).
    #[serde(default = "default_size")]
    pub size: (u8, u8),
    /// Размер как id прототипа `itemSize` сборки (`Tiny`/`Small`/`Normal`/
    /// `Large`/`Huge`/`Ginormous`) — числа берутся из
    /// [`crate::item_size`], а не подбираются (владелец: «стопка стали должна
    /// занимать 2 на 2 места» → `Normal`).
    #[serde(default)]
    pub size_id: Option<String>,
    /// Теги (материал/инструмент/еда) — для рецептов и правил.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Слот одежды, если предмет надевается (`ClothingSlot` как строка:
    /// "jumpsuit", "shoes", "gloves", "head", "back", ...).
    #[serde(default)]
    pub slot: Option<String>,
    /// Спрайт «надетым»: `"path.rsi#equipped-INNERCLOTHING"` (состояние из
    /// сборки: `equipped-<слот>` по карте `ClientClothingSystem.TemporarySlotMap`).
    #[serde(default)]
    pub worn: Option<String>,
    /// Есть ли в прототипе компонент `Pullable`. В сборке он стоит на `BaseItem`
    /// (`Entities/Objects/base_item.yml:62`) и `BaseStructure`
    /// (`Entities/Structures/base_structure.yml:27`), поэтому у предметов по
    /// умолчанию `true` (прежнее правило «тянуть только от 2×2» было выдумкой).
    #[serde(default = "default_pullable")]
    pub pullable: bool,
}

fn default_pullable() -> bool {
    true
}

fn default_size() -> (u8, u8) {
    (1, 1)
}

/// Набор предметов.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ItemSet {
    pub items: Vec<ItemProto>,
}

impl ItemSet {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    pub fn by_id(&self, id: &str) -> Option<&ItemProto> {
        self.items.iter().find(|item| item.id == id)
    }

    /// Размер предмета в клетках: сначала `size_id` по таблице `item_size.yml`
    /// сборки, иначе явные клетки из каталога (неизвестный — 1×1).
    pub fn size_of(&self, id: &str) -> (u8, u8) {
        let Some(item) = self.by_id(id) else {
            return (1, 1);
        };
        if let Some(size_id) = item.size_id.as_deref()
            && let Some(cells) = crate::item_size::cells_of(size_id)
        {
            return cells;
        }
        item.size
    }

    /// Id размера предмета (`None` — размер задан клетками или предмет неизвестен).
    pub fn size_id_of(&self, id: &str) -> Option<&str> {
        self.by_id(id).and_then(|item| item.size_id.as_deref())
    }

    /// Вес размера (`ItemSizePrototype.Weight`): по нему сравниваются размеры.
    pub fn weight_of(&self, id: &str) -> Option<u32> {
        let item = self.by_id(id)?;
        if let Some(size_id) = item.size_id.as_deref() {
            return crate::item_size::weight_of(size_id);
        }
        // Без `size_id` — подбираем ближайший размер по клеткам.
        let (w, h) = item.size;
        crate::item_size::ITEM_SIZES
            .iter()
            .find(|size| size.cells() == (w, h))
            .map(|size| size.weight)
    }

    /// Влезает ли предмет в карман (`PocketableItemSize = "Small"`): сравнение
    /// по весу размера, как `InventorySystem.Equip.cs:262-270`.
    pub fn pocketable(&self, id: &str) -> bool {
        self.weight_of(id)
            .is_some_and(|weight| weight <= crate::item_size::weight_of("Small").unwrap_or(2))
    }

    /// Тянется ли предмет: компонент `Pullable` из прототипа (а не размер).
    pub fn pullable(&self, id: &str) -> bool {
        self.by_id(id).map(|item| item.pullable).unwrap_or(false)
    }

    /// Слот одежды предмета (None — не надевается).
    pub fn slot_of(&self, id: &str) -> Option<&str> {
        self.by_id(id).and_then(|item| item.slot.as_deref())
    }

    /// Спрайт «надетым» (ключ `path.rsi#state`).
    pub fn worn_of(&self, id: &str) -> Option<&str> {
        self.by_id(id).and_then(|item| item.worn.as_deref())
    }

    /// Название для UI (неизвестный предмет — сам id).
    pub fn name_of(&self, id: &str) -> String {
        self.by_id(id)
            .map(|item| item.name.clone())
            .unwrap_or_else(|| id.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path() -> std::path::PathBuf {
        crate::assets_root().join("prototypes/items.ron")
    }

    #[test]
    fn items_catalog_is_big_enough() {
        let set = ItemSet::load(&path()).expect("items.ron");
        assert!(
            set.items.len() >= 30,
            "в каталоге {} предметов",
            set.items.len()
        );
        let crowbar = set.by_id("Crowbar").expect("Crowbar");
        // Размер — прототипом `itemSize` из сборки (`Normal` = 2×2), а не
        // выдуманными клетками: сверяем РАЗРЕШЁННЫЙ размер.
        assert_eq!(set.size_of("Crowbar"), (2, 2));
        assert_eq!(set.size_id_of("Crowbar"), Some("Normal"));
        assert!(crowbar.sprite.is_some());
    }

    #[test]
    fn unknown_item_defaults() {
        let set = ItemSet::default();
        assert_eq!(set.size_of("Nope"), (1, 1));
        assert_eq!(set.name_of("Nope"), "Nope");
    }
}
