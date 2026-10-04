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
    /// Размер в клетках инвентаря: (ширина, высота).
    #[serde(default = "default_size")]
    pub size: (u8, u8),
    /// Теги (материал/инструмент/еда) — для рецептов и правил.
    #[serde(default)]
    pub tags: Vec<String>,
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

    /// Размер предмета в клетках (неизвестный — 1×1).
    pub fn size_of(&self, id: &str) -> (u8, u8) {
        self.by_id(id).map(|item| item.size).unwrap_or((1, 1))
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
        assert_eq!(crowbar.size, (2, 1));
        assert!(crowbar.sprite.is_some());
    }

    #[test]
    fn unknown_item_defaults() {
        let set = ItemSet::default();
        assert_eq!(set.size_of("Nope"), (1, 1));
        assert_eq!(set.name_of("Nope"), "Nope");
    }
}
