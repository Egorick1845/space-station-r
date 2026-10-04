//! Прототипы контента (ADR-6): наши структуры + загрузка из .ron.
//!
//! Импорт из SS14 (парсер с наследованием и конвертер) — IMP.2/IMP.3
//! (SS14_IMPORT.md §4), инструмент `ssr-tools --bin import-proto`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Один прототип: сущность/предмет/роль и т.п.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Proto {
    /// Оригинальный id SS14 (IMP-2: сохраняем для трассировки).
    pub id: String,
    /// Родитель (в уже сконвертированном наборе отсутствует — слит).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Вид прототипа SS14: entity/job/reagent/...
    #[serde(default)]
    pub kind: String,
    /// Спрайт: `"path/to/name.rsi#state"` (или `.png`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<String>,
    /// Теги (для рецептов/взаимодействий).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Размер предмета (Small/Medium/Large).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// Слот экипировки (belt/head/...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equip: Option<String>,
    /// Абстрактный прототип — только шаблон, не спавнить.
    #[serde(default)]
    pub abstract_: bool,
    /// `Storage` — сетка (Box2i со включительными границами) и максимальный
    /// размер вкладываемого предмета (`maxItemSize`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<ProtoStorage>,
    /// `Clothing.slots` — флаги слотов (`INNERCLOTHING`, `HEAD`, `POCKET`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clothing_slots: Vec<String>,
    /// `Tool.qualities` — качества инструмента (`Prying`, `Anchoring`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// `Stack` — тип стека и количество.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<ProtoStack>,
    /// Есть ли компонент `Pullable` (`BaseItem`/`BaseStructure`).
    #[serde(default)]
    pub pullable: bool,
    /// `categories` (в т.ч. `HideSpawnMenu` — прототип не показывать в меню).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// `suffix` (подпись в редакторских меню).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// `Physics.bodyType` (`Static`/`Dynamic`/`Kinematic`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_type: Option<String>,
    /// `PointLight` — радиус/энергия/цвет, если компонент есть.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<ProtoLight>,
    /// Компоненты, которые мы ещё не разобрали по полям: `(тип, поля)` —
    /// чтобы ничего не терять при импорте (IMP.3, задача конвертера).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<(String, ron::Value)>,
}

/// `Storage` прототипа: сетка и максимальный размер вкладываемого.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoStorage {
    /// Боксы сетки `(left, bottom, right, top)` — границы включительно.
    pub grid: Vec<(i32, i32, i32, i32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_item_size: Option<String>,
}

/// `Stack` прототипа: тип стека, количество и максимум из прототипа стека.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoStack {
    pub kind: String,
    #[serde(default)]
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_count: Option<u32>,
}

/// `PointLight` прототипа (числа как в сборке).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoLight {
    pub radius: f32,
    pub energy: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

/// Набор прототипов (`assets/prototypes/*.ron`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProtoSet {
    pub protos: Vec<Proto>,
}

impl ProtoSet {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
    }

    /// Индекс id → прототип.
    pub fn by_id(&self) -> HashMap<&str, &Proto> {
        self.protos.iter().map(|p| (p.id.as_str(), p)).collect()
    }

    /// Габариты предмета по его размеру-прототипу (`Item.size` → `itemSize`).
    pub fn size_cells(&self, id: &str) -> Option<(u8, u8)> {
        let proto = self.protos.iter().find(|proto| proto.id == id)?;
        let size_id = proto.size.as_deref()?;
        crate::item_size::cells_of(size_id)
    }

    /// Показывать ли прототип в меню спавна: не abstract и нет категории
    /// `HideSpawnMenu` (`EntitySpawningUIController.BuildEntityList:203-211`).
    pub fn spawnable(&self, id: &str) -> bool {
        self.protos
            .iter()
            .find(|proto| proto.id == id)
            .is_some_and(|proto| {
                !proto.abstract_
                    && !proto
                        .categories
                        .iter()
                        .any(|category| category == "HideSpawnMenu")
            })
    }
}
