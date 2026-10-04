//! Роли (T4.2, PLAN.md): прототип роли в .ron — название, стартовый
//! инвентарь, доступы; антагонист с целью.
//!
//! Данные — `assets/prototypes/roles.ron`. Доступы — ключи дверей: роль
//! содержит список ключей, дверь на карте может требовать ключ (T4.2).
//! Механика целей раунда (победа/поражение) — T4.5, здесь только текст цели.

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Прототип роли (`assets/prototypes/roles.ron`).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Role {
    pub id: String,
    pub name: String,
    /// Стартовый инвентарь: имена предметов.
    #[serde(default)]
    pub items: Vec<String>,
    /// Доступы: ключи дверей ("general", "engineering", ...).
    #[serde(default)]
    pub access: Vec<String>,
    /// Антагонист с целью (T4.2: предатель).
    #[serde(default)]
    pub antagonist: bool,
    /// Текст цели антагониста.
    #[serde(default)]
    pub goal: String,
}

/// Набор ролей.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RoleSet {
    pub roles: Vec<Role>,
}

impl RoleSet {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    pub fn by_id(&self, id: &str) -> Option<&Role> {
        self.roles.iter().find(|role| role.id == id)
    }
}

/// Роль игрока (реплицируется клиенту, T4.2): имя для HUD, признак
/// антагониста и текст цели.
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct PlayerRole {
    pub id: String,
    pub name: String,
    pub antagonist: bool,
    pub goal: String,
}

/// Доступы игрока (T4.2): ключи дверей из роли. Только сервер — проверка
/// дверей авторитарна (ADR-3).
#[derive(Component, Clone, Debug, Default, PartialEq)]
pub struct Access {
    pub list: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_file_loads() {
        let path = crate::assets_root().join("prototypes/roles.ron");
        let set = RoleSet::load(&path).expect("roles.ron должен читаться");
        assert!(
            set.roles.len() >= 3,
            "минимум: ассистент, инженер, предатель"
        );

        let engineer = set.by_id("engineer").expect("engineer");
        assert!(engineer.access.contains(&"engineering".to_string()));
        assert!(!engineer.items.is_empty());

        let traitor = set.by_id("traitor").expect("traitor");
        assert!(traitor.antagonist);
        assert!(!traitor.goal.is_empty());
    }

    #[test]
    fn role_set_by_id_missing() {
        let set = RoleSet::default();
        assert!(set.by_id("nobody").is_none());
    }
}
