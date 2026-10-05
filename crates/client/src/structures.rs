//! Структуры мира из прототипов (столы и прочее): спрайт, соединение соседних
//! структур по `IconSmooth` (`IconSmoothSystem` + `IconSmoothComponent` сборки) и
//! «поверхность» — предметы на столе рисуются выше него.
//!
//! Числа и состояния — из прототипов: у столов `IconSmooth { key: <материал>,
//! base: "state_" }`, спрайт `Structures/Furniture/Tables/*.rsi`, состояния
//! `state_0`…`state_15` по маске соседей (1 север, 2 восток, 4 юг, 8 запад).

use std::collections::HashMap;

use bevy::prelude::*;
use ssr_core::structures::{CORNER_DIR_OFFSETS, Structure, corner_fills, smooth_state};

use crate::content::ClientContent;
use crate::rsi::RsiRegistry;

/// Визуал структуры: спрайт-сущность, привязанная к серверной сущности.
#[derive(Component)]
pub struct StructureVisual {
    /// Серверная сущность структуры (по ней берётся спрайт и соединения).
    #[allow(dead_code)]
    pub owner: Entity,
}

/// z структур: в сборке стол — `DrawDepth.Objects = 0` (`DrawDepth.cs:73-77`),
/// ниже ящиков, предметов (`Items = +4`) и мобов (`Mobs = +6`).
pub const STRUCTURE_Z: f32 = 0.45;
/// z предметов, лежащих НА структуре с `PlaceableSurface`: в сборке предмет
/// (`Items = +4`) рисуется над столом — здесь чуть выше обычных предметов на
/// полу, чтобы не спорить с ними за один z.
pub const SURFACE_ITEM_Z: f32 = 0.78;

/// Тайл структуры (мировые единицы → индексы тайлов).
pub fn tile_of(position: [f32; 2]) -> (i32, i32) {
    let size = ssr_core::tiles::TILE_PX as f32;
    (
        (position[0] / size).floor() as i32,
        (position[1] / size).floor() as i32,
    )
}

/// Набор тайлов, на которых лежит структура с `PlaceableSurface` — по нему
/// предметы рисуются выше стола (`sync_floor_item_icons`).
#[derive(Resource, Default)]
pub struct SurfaceTiles(pub std::collections::HashSet<(i32, i32)>);

/// Запись подписи набора структур: сущность, прототип, ключ соединения, тайл.
type StructureEntry = (Entity, String, Option<String>, (i32, i32));

/// Пересобирает визуалы структур: спрайт прототипа, состояние — по маске
/// соседей с тем же ключом `IconSmooth`.
#[allow(clippy::too_many_arguments)]
pub fn render_structures(
    mut commands: Commands,
    content: Res<ClientContent>,
    registry: Res<RsiRegistry>,
    mut surfaces: ResMut<SurfaceTiles>,
    structures: Query<(Entity, &Structure, &ssr_core::inventory::ItemPosition)>,
    existing: Query<(Entity, &StructureVisual)>,
    mut last: Local<Option<u64>>,
    mut last_count: Local<usize>,
) {
    let replicated = structures.iter().count();
    if *last_count != replicated {
        *last_count = replicated;
        tracing::info!(replicated, "structures replicated (client)");
    }
    // Подпись набора: что и где стоит (маска соединений зависит от соседей,
    // поэтому перерисовываем при любом изменении набора).
    let mut entries: Vec<StructureEntry> = structures
        .iter()
        .map(|(entity, structure, position)| {
            (
                entity,
                structure.proto.clone(),
                structure.smooth.clone(),
                tile_of(position.0),
            )
        })
        .collect();
    entries.sort_by_key(|(_, proto, _, tile)| (tile.1, tile.0, proto.clone()));
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(&entries, &mut hasher);
    let signature = std::hash::Hasher::finish(&hasher);
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);

    for (entity, _) in existing.iter() {
        commands.entity(entity).despawn();
    }
    // Тайлы поверхностей — для z-порядка предметов на столах.
    surfaces.0.clear();
    for (_, structure, position) in structures.iter() {
        if structure.surface {
            surfaces.0.insert(tile_of(position.0));
        }
    }

    // Ключи сглаживания по тайлам: сосед соединяется, если у него тот же ключ.
    let mut keys: HashMap<(i32, i32), String> = HashMap::new();
    for (_, structure, position) in structures.iter() {
        if let Some(key) = &structure.smooth {
            keys.insert(tile_of(position.0), key.clone());
        }
    }

    let mut spawned = 0usize;
    for (owner, structure, position) in structures.iter() {
        let Some(base_sprite) = content.proto_sprites.get(&structure.proto) else {
            continue;
        };
        // `path.rsi#state` → путь и состояние (состояние переопределяет маска).
        let (path, default_state) = match base_sprite.split_once('#') {
            Some((path, state)) => (path, state),
            None => (base_sprite.as_str(), "0"),
        };
        let tile = tile_of(position.0);
        // Соседи: [N, NE, E, SE, S, SW, W, NW] — соединение только с тем же
        // ключом `IconSmooth` (`MatchingEntity`, `IconSmoothSystem.cs:380-394`).
        let same_key = |dx: i32, dy: i32| match (&structure.smooth, &content.proto_smooth) {
            (Some(_), _) => keys
                .get(&(tile.0 + dx, tile.1 + dy))
                .is_some_and(|other| Some(other) == structure.smooth.as_ref()),
            _ => false,
        };
        match (
            &structure.smooth,
            &content.proto_smooth.get(&structure.proto),
        ) {
            (Some(_), Some((_, base))) => {
                // Стол рисуется ЧЕТЫРЬМЯ слоями-углами (`SetCornerLayers`):
                // у каждого своё состояние `{base}{флаги угла}` и своё смещение
                // направления (SE — 0, NE — «против часовой», NW — «флип»,
                // SW — «по часовой»).
                let fills = corner_fills([
                    same_key(0, 1),
                    same_key(1, 1),
                    same_key(1, 0),
                    same_key(1, -1),
                    same_key(0, -1),
                    same_key(-1, -1),
                    same_key(-1, 0),
                    same_key(-1, 1),
                ]);
                for (corner, fill) in fills.iter().enumerate() {
                    let key = format!("sprites/ss14/{path}#{}", smooth_state(base, *fill));
                    let Some(sprite) = registry.get(&key) else {
                        *last = None;
                        continue;
                    };
                    let direction = (CORNER_DIR_OFFSETS[corner] as u32) % sprite.directions.max(1);
                    let mut component = Sprite::from_image(sprite.image.clone());
                    component.texture_atlas = Some(TextureAtlas {
                        layout: sprite.layout.clone(),
                        index: sprite.index(direction, 0),
                    });
                    commands.spawn((
                        StructureVisual { owner },
                        component,
                        // Слои рисуются по порядку: SE, NE, NW, SW.
                        Transform::from_xyz(
                            position.0[0],
                            position.0[1],
                            STRUCTURE_Z + corner as f32 * 0.001,
                        ),
                    ));
                    spawned += 1;
                }
            }
            _ => {
                let key = format!("sprites/ss14/{path}#{default_state}");
                let Some(sprite) = registry.get(&key) else {
                    *last = None;
                    continue;
                };
                let mut component = Sprite::from_image(sprite.image.clone());
                component.texture_atlas = Some(TextureAtlas {
                    layout: sprite.layout.clone(),
                    index: sprite.index(0, 0),
                });
                commands.spawn((
                    StructureVisual { owner },
                    component,
                    Transform::from_xyz(position.0[0], position.0[1], STRUCTURE_Z),
                ));
                spawned += 1;
            }
        }
    }
    if spawned > 0 {
        tracing::info!(
            structures = spawned,
            surfaces = surfaces.0.len(),
            "structure visuals built"
        );
    }
}
