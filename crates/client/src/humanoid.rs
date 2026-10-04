//! Гуманоидные спрайты игроков (T5.3): тело собирается из частей расы
//! (`Mobs/Species/<id>/parts.rsi`) — части уже позиционированы внутри своих
//! клеток 32×32, поэтому просто накладываются друг на друга.
//!
//! Свой игрок и другие игроки рисуются одинаково: сущность-визуал носит
//! [`Facing`], части тела — её дети ([`HumanoidPart`]).

use bevy::prelude::*;
use ssr_core::Species;

use crate::inventory_ui::{OwnPlayerEntity, RemotePlayerVisual};
use crate::rsi::RsiRegistry;

/// Части тела снизу вверх (порядок отрисовки); z — слой внутри тайла.
const PARTS: &[(&str, f32)] = &[
    ("l_leg", 0.00),
    ("r_leg", 0.01),
    ("l_foot", 0.02),
    ("r_foot", 0.03),
    ("groin_m", 0.04),
    ("chest_m", 0.05),
    ("l_arm", 0.06),
    ("r_arm", 0.07),
    ("l_hand", 0.08),
    ("r_hand", 0.09),
    ("head_m", 0.10),
];

/// Направление взгляда (порядок движка: 0 юг, 1 север, 2 восток, 3 запад).
#[derive(Component, Default, Clone, Copy, PartialEq)]
pub struct Facing(pub u32);

/// Часть тела: владелец (визуал) и ключ спрайта в реестре RSI.
#[derive(Component)]
pub struct HumanoidPart {
    pub owner: Entity,
    pub key: String,
}

/// Текущая раса собранного тела (для пересборки при смене расы).
#[derive(Component, Clone, PartialEq)]
pub struct BodySpecies(pub String);

/// Ключ спрайта части тела: `Mobs/Species/<раса>/parts.rsi#<часть>`.
fn part_key(species: &str, part: &str) -> String {
    format!("sprites/ss14/Mobs/Species/{species}/parts.rsi#{part}")
}

/// Собирает тело расы детьми сущности-визуала (родитель носит [`Facing`]).
pub fn attach_body(commands: &mut Commands, registry: &RsiRegistry, owner: Entity, species: &str) {
    let mut attached = 0usize;
    commands.entity(owner).with_children(|parent| {
        for (part, z) in PARTS {
            let key = part_key(species, part);
            let Some(sprite) = registry.get(&key) else {
                continue;
            };
            let mut component = Sprite::from_image(sprite.image.clone());
            component.texture_atlas = Some(TextureAtlas {
                layout: sprite.layout.clone(),
                index: sprite.index(0, 0),
            });
            parent.spawn((
                HumanoidPart { owner, key },
                component,
                Transform::from_xyz(0.0, 0.0, *z),
            ));
            attached += 1;
        }
    });
    if attached == 0 {
        tracing::warn!(species, "humanoid parts missing — раса не в STARTUP_RSI?");
    }
}

/// Собирает/пересобирает тела, когда раса появилась или сменилась.
pub fn sync_bodies(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    own: Res<OwnPlayerEntity>,
    species: Query<&Species>,
    players: Query<Entity, With<crate::Player>>,
    visuals: Query<(Entity, &RemotePlayerVisual)>,
    bodies: Query<&BodySpecies>,
) {
    let species_of = |entity: Entity| {
        species
            .get(entity)
            .map(|s| s.id.clone())
            .unwrap_or_else(|_| "Human".to_string())
    };

    if let Some(own_entity) = own.0 {
        let species_id = species_of(own_entity);
        for player_entity in players.iter() {
            if bodies.get(player_entity).ok().map(|b| b.0.as_str()) == Some(species_id.as_str()) {
                continue;
            }
            commands.entity(player_entity).despawn_related::<Children>();
            attach_body(&mut commands, &registry, player_entity, &species_id);
            commands
                .entity(player_entity)
                .insert(BodySpecies(species_id.clone()));
            tracing::info!(species = %species_id, "player body attached");
        }
    }

    for (visual_entity, visual) in visuals.iter() {
        let species_id = species_of(visual.player);
        if bodies.get(visual_entity).ok().map(|b| b.0.as_str()) == Some(species_id.as_str()) {
            continue;
        }
        commands.entity(visual_entity).despawn_related::<Children>();
        attach_body(&mut commands, &registry, visual_entity, &species_id);
        commands
            .entity(visual_entity)
            .insert(BodySpecies(species_id.clone()));
        tracing::info!(player = ?visual.player, species = %species_id, "remote body attached");
    }
}

/// Поворот всех частей тела вслед за направлением владельца.
pub fn update_facing(
    registry: Res<RsiRegistry>,
    facings: Query<&Facing>,
    mut parts: Query<(&HumanoidPart, &mut Sprite)>,
) {
    for (part, mut sprite) in parts.iter_mut() {
        let Ok(facing) = facings.get(part.owner) else {
            continue;
        };
        let Some(rsi) = registry.get(&part.key) else {
            continue;
        };
        let index = rsi.index(facing.0.min(3), 0);
        if let Some(atlas) = sprite.texture_atlas.as_mut()
            && atlas.index != index
        {
            atlas.index = index;
        }
    }
}
