//! Гуманоидные спрайты игроков (T5.3): тело собирается из частей расы
//! (`Mobs/Species/<id>/parts.rsi`) — части уже позиционированы внутри своих
//! клеток 32×32, поэтому просто накладываются друг на друга.
//!
//! Свой игрок и другие игроки рисуются одинаково: сущность-визуал носит
//! [`Facing`], части тела — её дети ([`HumanoidPart`]).

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use ssr_core::Species;
use ssr_core::mechanics::{Ghost, KnockedDown};

use crate::inventory_ui::{OwnPlayerEntity, RemotePlayerVisual};
use crate::rsi::RsiRegistry;

/// Части тела снизу вверх: порядок взят из движка (HumanoidVisualLayers):
/// Chest, Groin, Head, Eyes, RArm, LArm, RHand, LHand, RLeg, LLeg, RFoot, LFoot —
/// ноги и ступни рисуются ПОВЕРХ торса, поэтому контуры частей не режут тело.
/// z — слой внутри тайла.
const PARTS: &[(&str, f32)] = &[
    ("chest_m", 0.00),
    ("groin_m", 0.01),
    ("head_m", 0.02),
    ("r_arm", 0.03),
    ("l_arm", 0.04),
    ("r_hand", 0.05),
    ("l_hand", 0.06),
    ("r_leg", 0.07),
    ("l_leg", 0.08),
    ("r_foot", 0.09),
    ("l_foot", 0.10),
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

/// Слой призрака (заменяет тело в режиме призрака, механики владельца).
#[derive(Component)]
pub struct GhostLayer;

/// Ключ спрайта части тела: `Mobs/Species/<раса>/parts.rsi#<часть>`.
fn part_key(species: &str, part: &str) -> String {
    format!("sprites/ss14/Mobs/Species/{species}/parts.rsi#{part}")
}

/// Глаза — отдельный слой поверх головы (как `MobHumanoidEyes` в SS14).
const EYES_KEY: &str = "sprites/ss14/Mobs/Customization/eyes.rsi#eyes";
/// Ключ спрайта призрака.
const GHOST_KEY: &str = "sprites/ss14/Mobs/Ghosts/ghost_human.rsi#animated";
/// Псевдораса для тела-призрака.
pub const GHOST_SPECIES: &str = "Ghost";
/// Расы, которым глаза-человеческие не рисуем (своя голова/маска).
const NO_EYES: &[&str] = &["Skeleton", "Diona", "Gingerbread"];

/// Собирает тело расы детьми сущности-визуала (родитель носит [`Facing`]).
/// Возвращает число прикреплённых частей (0 — спрайты расы не найдены).
pub fn attach_body(
    commands: &mut Commands,
    registry: &RsiRegistry,
    owner: Entity,
    species: &str,
) -> usize {
    // Призрак: вместо тела — один спрайт призрака (механики владельца).
    if species == GHOST_SPECIES {
        let Some(sprite) = registry.get(GHOST_KEY) else {
            return 0;
        };
        let mut component = Sprite::from_image(sprite.image.clone());
        component.texture_atlas = Some(TextureAtlas {
            layout: sprite.layout.clone(),
            index: sprite.index(0, 0),
        });
        commands.entity(owner).with_children(|parent| {
            parent.spawn((GhostLayer, component, Transform::from_xyz(0.0, 0.0, 0.05)));
        });
        return PARTS.len();
    }
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
        // Глаза: отдельный слой поверх головы (кроме рас со своей головой).
        if !NO_EYES.contains(&species)
            && let Some(sprite) = registry.get(EYES_KEY)
        {
            let mut component = Sprite::from_image(sprite.image.clone());
            component.texture_atlas = Some(TextureAtlas {
                layout: sprite.layout.clone(),
                index: sprite.index(0, 0),
            });
            parent.spawn((
                HumanoidPart {
                    owner,
                    key: EYES_KEY.to_string(),
                },
                component,
                Transform::from_xyz(0.0, 0.0, 0.025),
            ));
            attached += 1;
        }
    });
    if attached == 0 {
        // Норма на первом кадре при ленивой загрузке: спрайты ещё не пришли,
        // вызов повторится (см. sync_bodies). Ошибка загрузки логируется в rsi.rs.
        tracing::debug!(species, "humanoid parts not loaded yet");
    }
    attached
}

/// Снимает старые части тела перед пересборкой. Дети удаляются поштучно:
/// `despawn_related` паникует, если сущность уже удалена в этом же кадре
/// (например, дубль своего игрока убирает `sync_remote_players`).
fn detach_body(commands: &mut Commands, children: &Query<&Children>, owner: Entity) {
    let Ok(list) = children.get(owner) else {
        return;
    };
    for child in list.iter() {
        commands.entity(child).despawn();
    }
}

/// Запросы сущностей для сборки тел (сокращает число аргументов системы).
#[derive(SystemParam)]
pub struct BodyQueries<'w, 's> {
    pub players: Query<'w, 's, Entity, With<crate::Player>>,
    pub ghosts: Query<'w, 's, &'static Ghost>,
    pub visuals: Query<'w, 's, (Entity, &'static RemotePlayerVisual)>,
    pub bodies: Query<'w, 's, &'static BodySpecies>,
    pub children: Query<'w, 's, &'static Children>,
}

/// Собирает/пересобирает тела, когда раса появилась или сменилась.
pub fn sync_bodies(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    own: Res<OwnPlayerEntity>,
    species: Query<&Species>,
    q: BodyQueries,
) {
    let BodyQueries {
        players,
        ghosts,
        visuals,
        bodies,
        children,
    } = q;
    let species_of = |entity: Entity| {
        // Призрак важнее расы: тело заменяется спрайтом призрака.
        if ghosts.get(entity).is_ok() {
            return GHOST_SPECIES.to_string();
        }
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
            detach_body(&mut commands, &children, player_entity);
            let parts = attach_body(&mut commands, &registry, player_entity, &species_id);
            // Спрайты грузятся лениво (T5.3): пока частей не хватает — не
            // помечаем тело собранным, на следующем кадре попробуем снова.
            if parts < PARTS.len() {
                continue;
            }
            commands
                .entity(player_entity)
                .insert(BodySpecies(species_id.clone()));
            tracing::info!(species = %species_id, parts, "player body attached");
        }
    }

    for (visual_entity, visual) in visuals.iter() {
        // Свой игрок рисуется отдельным визуалом Player: его дубль убирает
        // sync_remote_players — здесь тело ему собирать не нужно.
        if Some(visual.player) == own.0 {
            continue;
        }
        let species_id = species_of(visual.player);
        if bodies.get(visual_entity).ok().map(|b| b.0.as_str()) == Some(species_id.as_str()) {
            continue;
        }
        detach_body(&mut commands, &children, visual_entity);
        let parts = attach_body(&mut commands, &registry, visual_entity, &species_id);
        if parts < PARTS.len() {
            continue;
        }
        commands
            .entity(visual_entity)
            .insert(BodySpecies(species_id.clone()));
        tracing::info!(player = ?visual.player, species = %species_id, parts, "remote body attached");
    }
}

/// Отладка (SSR_DEBUG_BODY=1): раз в секунду печатает позицию и видимость
/// каждой части тела — проверка, что тело реально рисуется и не разъехалось.
pub fn debug_body(
    time: Res<Time>,
    mut next_log: Local<f32>,
    parts: Query<(Entity, &HumanoidPart, &GlobalTransform, &ViewVisibility)>,
) {
    if std::env::var_os("SSR_DEBUG_BODY").is_none() {
        return;
    }
    *next_log += time.delta_secs();
    if *next_log < 1.0 {
        return;
    }
    *next_log = 0.0;
    for (entity, part, transform, visible) in parts.iter() {
        tracing::info!(
            ?entity,
            owner = ?part.owner,
            x = transform.translation().x,
            y = transform.translation().y,
            z = transform.translation().z,
            visible = visible.get(),
            part = %part.key,
            "body part"
        );
    }
}

/// Лежачий игрок: тело поворачивается на 90° (падение, механики владельца).
pub fn update_knocked(
    mut bodies: Query<(&KnockedDown, &mut Transform), With<crate::Player>>,
    mut visuals: Query<(&RemotePlayerVisual, &mut Transform), Without<crate::Player>>,
    knocked: Query<&KnockedDown>,
) {
    for (state, mut transform) in bodies.iter_mut() {
        let target = if state.seconds > 0.0 {
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };
        if transform.rotation != target {
            transform.rotation = target;
            tracing::info!("body rotation updated (knocked={})", state.seconds > 0.0);
        }
    }
    for (visual, mut transform) in visuals.iter_mut() {
        let target = if knocked.get(visual.player).is_ok() {
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
        } else {
            Quat::IDENTITY
        };
        if transform.rotation != target {
            transform.rotation = target;
        }
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
