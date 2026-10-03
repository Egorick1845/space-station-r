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
}
