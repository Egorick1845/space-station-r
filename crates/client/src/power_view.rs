//! Спрайты электрики (T4.4): кабели с соединениями, генератор, лампы.
//!
//! Кабель рисуется стейтом `lvcable_N`, где N — маска соединений по сторонам
//! (N=1, E=2, S=4, W=8), как в движке SS14. Лампа — корпус + свечение
//! (`glow`), которое включается только при питании; генератор — спрайт
//! `portgen0`/`portgen0on` по состоянию сети.

use bevy::prelude::*;
use ssr_core::inventory::ItemPosition;
use ssr_core::power::{
    CABLE_EAST, CABLE_NORTH, CABLE_SOUTH, CABLE_WEST, Cable, Generator, Light, Powered,
};

use crate::rsi::RsiRegistry;

const CABLE_PREFIX: &str = "sprites/ss14/Structures/Power/Cables/lv_cable.rsi#lvcable_";
const GENERATOR_ON: &str =
    "sprites/ss14/Structures/Power/Generation/portable_generator.rsi#portgen0on";
const GENERATOR_OFF: &str =
    "sprites/ss14/Structures/Power/Generation/portable_generator.rsi#portgen0";
const LIGHT_BASE: &str = "sprites/ss14/Structures/Wallmounts/Lighting/light_tube.rsi#base";
const LIGHT_GLOW: &str = "sprites/ss14/Structures/Wallmounts/Lighting/light_tube.rsi#glow";
/// Тайл в юнитах (маски кабелей считаются по тайловой сетке).
const TILE_UNITS: f32 = 32.0;

/// Визуал кабеля.
#[derive(Component)]
pub struct CableVisual;

/// Визуал лампы: корпус, дочернее свечение и исходная сущность (для питания).
#[derive(Component)]
pub struct LightVisual {
    glow: Entity,
    source: Entity,
}

/// Визуал генератора.
#[derive(Component)]
pub struct GeneratorVisual {
    source: Entity,
}

/// Спавнит спрайты кабелей, генераторов и ламп.
pub fn spawn_power_visuals(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    cables: Query<&ItemPosition, Added<Cable>>,
    generators: Query<(Entity, &ItemPosition), Added<Generator>>,
    lights: Query<(Entity, &ItemPosition), Added<Light>>,
) {
    for position in cables.iter() {
        let Some(sprite) = registry.get(&format!("{CABLE_PREFIX}0")) else {
            continue;
        };
        let mut component = Sprite::from_image(sprite.image.clone());
        component.texture_atlas = Some(TextureAtlas {
            layout: sprite.layout.clone(),
            index: 0,
        });
        commands.spawn((
            CableVisual,
            ItemPosition(position.0),
            component,
            // Под игроком и дверями, поверх тайлов.
            Transform::from_xyz(position.0[0], position.0[1], 0.2),
        ));
    }

    for (entity, position) in generators.iter() {
        let Some(sprite) = registry.get(GENERATOR_ON) else {
            continue;
        };
        let mut component = Sprite::from_image(sprite.image.clone());
        component.texture_atlas = Some(TextureAtlas {
            layout: sprite.layout.clone(),
            index: 0,
        });
        commands.spawn((
            GeneratorVisual { source: entity },
            ItemPosition(position.0),
            component,
            Transform::from_xyz(position.0[0], position.0[1], 0.6),
        ));
        tracing::debug!(generator = ?entity, "generator visual spawned");
    }

    for (entity, position) in lights.iter() {
        let (Some(base), Some(glow)) = (registry.get(LIGHT_BASE), registry.get(LIGHT_GLOW)) else {
            continue;
        };
        let mut base_sprite = Sprite::from_image(base.image.clone());
        base_sprite.texture_atlas = Some(TextureAtlas {
            layout: base.layout.clone(),
            index: 0,
        });
        let mut glow_sprite = Sprite::from_image(glow.image.clone());
        glow_sprite.texture_atlas = Some(TextureAtlas {
            layout: glow.layout.clone(),
            index: 0,
        });
        let glow_entity = commands
            .spawn((
                glow_sprite,
                // Свечение поверх корпуса, включается питанием.
                Transform::from_xyz(0.0, 0.0, 0.01),
                Visibility::Hidden,
            ))
            .id();
        commands
            .spawn((
                LightVisual {
                    glow: glow_entity,
                    source: entity,
                },
                ItemPosition(position.0),
                base_sprite,
                Transform::from_xyz(position.0[0], position.0[1], 0.55),
            ))
            .add_child(glow_entity);
        tracing::debug!(light = ?entity, "light visual spawned");
    }
}

/// Пересчитывает маску соединений кабелей (стейт `lvcable_N`).
pub fn update_cables(
    time: Res<Time>,
    registry: Res<RsiRegistry>,
    mut next_step: Local<f32>,
    mut last_tiles: Local<Vec<(i32, i32)>>,
    cables: Query<&ItemPosition, With<Cable>>,
    mut visuals: Query<(&ItemPosition, &mut Sprite), With<CableVisual>>,
) {
    // Пересчёт не каждый кадр: набор кабелей меняется редко.
    *next_step += time.delta_secs();
    if *next_step < 0.5 {
        return;
    }
    *next_step = 0.0;

    let tiles: Vec<(i32, i32)> = cables.iter().map(tile_of).collect();
    if tiles == *last_tiles {
        return;
    }
    *last_tiles = tiles.clone();

    for (position, mut sprite) in visuals.iter_mut() {
        let tile = tile_of(position);
        let mut mask = 0u8;
        if tiles.contains(&(tile.0, tile.1 + 1)) {
            mask |= CABLE_NORTH;
        }
        if tiles.contains(&(tile.0 + 1, tile.1)) {
            mask |= CABLE_EAST;
        }
        if tiles.contains(&(tile.0, tile.1 - 1)) {
            mask |= CABLE_SOUTH;
        }
        if tiles.contains(&(tile.0 - 1, tile.1)) {
            mask |= CABLE_WEST;
        }
        let Some(state) = registry.get(&format!("{CABLE_PREFIX}{mask}")) else {
            continue;
        };
        if sprite.image != state.image {
            sprite.image = state.image.clone();
            sprite.texture_atlas = Some(TextureAtlas {
                layout: state.layout.clone(),
                index: 0,
            });
        }
    }
}

/// Тайловые координаты позиции.
fn tile_of(position: &ItemPosition) -> (i32, i32) {
    (
        (position.0[0] / TILE_UNITS) as i32,
        (position.0[1] / TILE_UNITS) as i32,
    )
}

/// Лампы светят, а генераторы показывают работу — по состоянию сети (`Powered`).
pub fn update_power_visuals(
    registry: Res<RsiRegistry>,
    powered: Query<&Powered>,
    lights: Query<&LightVisual>,
    mut visibilities: Query<&mut Visibility>,
    generators: Query<(Entity, &GeneratorVisual)>,
    mut sprites: Query<&mut Sprite>,
) {
    for light in lights.iter() {
        let on = powered
            .get(light.source)
            .map(|state| state.0)
            .unwrap_or(false);
        if let Ok(mut visibility) = visibilities.get_mut(light.glow) {
            let target = if on {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *visibility != target {
                *visibility = target;
                tracing::info!(glow = ?light.glow, on, "light state changed");
            }
        }
    }
    for (visual_entity, generator) in generators.iter() {
        let on = powered
            .get(generator.source)
            .map(|state| state.0)
            .unwrap_or(true);
        let target = if on { GENERATOR_ON } else { GENERATOR_OFF };
        let Some(sprite) = registry.get(target) else {
            continue;
        };
        if let Ok(mut component) = sprites.get_mut(visual_entity)
            && component.image != sprite.image
        {
            component.image = sprite.image.clone();
            component.texture_atlas = Some(TextureAtlas {
                layout: sprite.layout.clone(),
                index: 0,
            });
        }
    }
}
