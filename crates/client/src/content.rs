//! Контент-каталоги клиента (T5.2): предметы и рецепты из assets/prototypes,
//! плюс ИМПОРТИРОВАННЫЕ прототипы сборки (`prototypes_ss14.ron`, IMP.2/IMP.3) —
//! из них собирается меню спавна: «как можно больше сущностей», а не только наш
//! ручной каталог из 38 предметов.

use std::collections::HashMap;

use bevy::prelude::*;

/// Каталоги предметов и рецептов + спрайты импортированных прототипов.
#[derive(Resource, Default)]
pub struct ClientContent {
    pub items: ssr_core::items::ItemSet,
    pub recipes: ssr_core::recipes::RecipeSet,
    /// Спрайт импортированного прототипа: id → `"path/to/name.rsi#state"`.
    pub proto_sprites: HashMap<String, String>,
    /// Имя импортированного прототипа: id → локализованное имя (или id).
    pub proto_names: HashMap<String, String>,
    /// Размер предмета из прототипа: id → id размера (`Normal`, `Small`, …).
    pub proto_sizes: HashMap<String, String>,
    /// Явная форма предмета (`Item.shape`): id → габариты в клетках (лом 1×2).
    pub proto_shapes: HashMap<String, (u8, u8)>,
    /// `IconSmooth` прототипа: id → (ключ соединения, база состояния) — у столов
    /// `("table", "state_")`, состояния спрайта `state_0`…`state_15`.
    pub proto_smooth: HashMap<String, (String, String)>,
}

impl ClientContent {
    /// Ключ спрайта предмета: сначала наш каталог, затем импортированные
    /// прототипы сборки (у них спрайт уже в виде `path#state`).
    pub fn sprite_key(&self, name: &str) -> Option<&str> {
        if let Some(sprite) = self
            .items
            .by_id(name)
            .and_then(|item| item.sprite.as_deref())
        {
            return Some(sprite);
        }
        self.proto_sprites.get(name).map(String::as_str)
    }

    /// Имя для списка спавна: каталог, затем импортированный прототип.
    pub fn display_name(&self, id: &str) -> String {
        if let Some(item) = self.items.by_id(id) {
            return item.name.clone();
        }
        self.proto_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }
}

/// Загружает каталоги при старте.
pub fn load_content(mut commands: Commands) {
    let root = ssr_core::assets_root().join("prototypes");
    let items = ssr_core::items::ItemSet::load(&root.join("items.ron"));
    let recipes = ssr_core::recipes::RecipeSet::load(&root.join("recipes.ron"));
    let mut content = ClientContent::default();
    match (items, recipes) {
        (Ok(items), Ok(recipes)) => {
            content.items = items;
            content.recipes = recipes;
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
    // Русские имена (import-names: ru-RU .ftl → names_ru.ron) — перекрывают
    // английские `name` из YAML прототипов.
    let names_ru: HashMap<String, String> =
        std::fs::read_to_string(ssr_core::assets_root().join("prototypes/names_ru.ron"))
            .ok()
            .and_then(|text| ron::from_str(&text).ok())
            .unwrap_or_default();
    let ru_count = names_ru.len();
    for (id, name) in names_ru {
        content.proto_names.insert(id, name);
    }

    // Импортированные прототипы сборки (19k+ сущностей). Нужны для меню спавна:
    // берём спавнимые (не abstract, без категории `HideSpawnMenu` — как
    // `EntitySpawningUIController.BuildEntityList:203-211`) и только со спрайтом.
    let started = std::time::Instant::now();
    let path = ssr_core::assets_root().join("prototypes_ss14.ron");
    match ssr_core::prototypes::ProtoSet::load(&path) {
        Ok(set) => {
            let mut hidden = 0usize;
            for proto in set.protos {
                if proto.abstract_ || proto.kind != "entity" {
                    continue;
                }
                if proto
                    .categories
                    .iter()
                    .any(|category| category == "HideSpawnMenu")
                {
                    hidden += 1;
                    continue;
                }
                let Some(sprite) = proto.sprite else {
                    continue;
                };
                if let Some(name) = proto.name {
                    content.proto_names.insert(proto.id.clone(), name);
                }
                if let Some(size) = proto.size {
                    content.proto_sizes.insert(proto.id.clone(), size);
                }
                // Явная форма (`Item.shape`) — перекрывает размер (лом 1×2).
                if let Some(cells) = ssr_core::item_size::cells_of_shape(&proto.shape) {
                    content.proto_shapes.insert(proto.id.clone(), cells);
                }
                if let Some(smooth) = proto.smooth {
                    content
                        .proto_smooth
                        .insert(proto.id.clone(), (smooth.key, smooth.base));
                }
                content.proto_sprites.insert(proto.id, sprite);
            }
            tracing::info!(
                items = content.items.items.len(),
                recipes = content.recipes.recipes.len(),
                protos = content.proto_sprites.len(),
                ru = ru_count,
                hidden,
                ms = started.elapsed().as_millis() as u64,
                "content catalog loaded (client)"
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "prototypes_ss14.ron not loaded");
            tracing::info!(
                items = content.items.items.len(),
                recipes = content.recipes.recipes.len(),
                "content catalog loaded (client, без прототипов)"
            );
        }
    }
    commands.insert_resource(content);
}
