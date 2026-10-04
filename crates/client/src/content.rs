//! Контент-каталоги клиента (T5.2): предметы и рецепты из assets/prototypes.
//! Те же файлы, что у сервера, — клиент показывает иконки и меню крафта.

use bevy::prelude::*;

/// Каталоги предметов и рецептов.
#[derive(Resource, Default)]
pub struct ClientContent {
    pub items: ssr_core::items::ItemSet,
    pub recipes: ssr_core::recipes::RecipeSet,
}

/// Загружает каталоги при старте.
pub fn load_content(mut commands: Commands) {
    let root = ssr_core::assets_root().join("prototypes");
    let items = ssr_core::items::ItemSet::load(&root.join("items.ron"));
    let recipes = ssr_core::recipes::RecipeSet::load(&root.join("recipes.ron"));
    match (items, recipes) {
        (Ok(items), Ok(recipes)) => {
            tracing::info!(
                items = items.items.len(),
                recipes = recipes.recipes.len(),
                "content catalog loaded (client)"
            );
            commands.insert_resource(ClientContent { items, recipes });
        }
        (items, recipes) => {
            if let Err(e) = items {
                tracing::error!(error = %e, "items.ron not loaded");
            }
            if let Err(e) = recipes {
                tracing::error!(error = %e, "recipes.ron not loaded");
            }
        }
    }
}
