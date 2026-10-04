//! Рецепты крафта (PLAN.md T5.2, ADR-6): вход — предметы из каталога,
//! выход — предмет. Считает и проверяет наличие сервер, клиент показывает
//! список и подсвечивает доступные (данные у обоих из одного .ron).

use serde::{Deserialize, Serialize};
use std::path::Path;

/// Рецепт: сколько чего нужно и что получится.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Recipe {
    pub id: String,
    /// Название для UI.
    pub name: String,
    /// Вход: (id предмета, количество).
    pub inputs: Vec<(String, u32)>,
    /// Выход: (id предмета, количество).
    pub output: (String, u32),
}

/// Набор рецептов.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecipeSet {
    pub recipes: Vec<Recipe>,
}

impl RecipeSet {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    pub fn by_id(&self, id: &str) -> Option<&Recipe> {
        self.recipes.iter().find(|recipe| recipe.id == id)
    }
}

/// Хватает ли материалов: `have` — идентификаторы предметов у игрока.
/// Каждый вход проверяется отдельно (использованные не переиспользуются).
pub fn can_craft(recipe: &Recipe, have: &[String]) -> bool {
    let mut pool: Vec<&String> = have.iter().collect();
    for (id, count) in &recipe.inputs {
        for _ in 0..*count {
            let Some(position) = pool.iter().position(|item| *item == id) else {
                return false;
            };
            pool.remove(position);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recipe() -> Recipe {
        Recipe {
            id: "rods".into(),
            name: "Прутья".into(),
            inputs: vec![("SteelSheet".into(), 2), ("Crowbar".into(), 1)],
            output: ("MetalRod".into(), 4),
        }
    }

    #[test]
    fn craft_requires_every_input() {
        let recipe = recipe();
        let have = vec![
            "SteelSheet".to_string(),
            "SteelSheet".to_string(),
            "Crowbar".to_string(),
        ];
        assert!(can_craft(&recipe, &have));
        // Не хватает одного листа.
        let have = vec!["SteelSheet".to_string(), "Crowbar".to_string()];
        assert!(!can_craft(&recipe, &have));
        // Лист нельзя использовать дважды.
        assert!(!can_craft(&recipe, &["SteelSheet".to_string()]));
    }

    #[test]
    fn recipes_file_loads() {
        let set = RecipeSet::load(&crate::assets_root().join("prototypes/recipes.ron"))
            .expect("recipes.ron");
        assert!(set.recipes.len() >= 10, "рецептов: {}", set.recipes.len());
    }
}
