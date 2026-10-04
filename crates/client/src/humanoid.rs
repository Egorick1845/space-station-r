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
/// Цвет кожи по умолчанию: в SS14 арт `parts.rsi` нейтральный, а цвет задаёт
/// профиль (Human: HSV(25°, 20%, 100%) ≈ #FFE1CC).
pub const DEFAULT_SKIN: Color = Color::srgb_u8(0xff, 0xe1, 0xcc);
/// Цвет глаз по умолчанию (`HumanoidCharacterAppearance`: чёрный).
pub const DEFAULT_EYES: Color = Color::srgb_u8(0x10, 0x10, 0x10);

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

/// Часть с учётом пола: `_m`/`_f` только у head/chest/groin (SS14:
/// `HasSexMorph` — Groin, Chest, Head); остальные части без вариантов.
fn sexed_part(part: &str, sex: ssr_core::mechanics::Sex) -> String {
    match part {
        "head_m" | "chest_m" | "groin_m" => {
            let base = part.trim_end_matches("_m");
            format!("{base}{}", sex.part_suffix())
        }
        _ => part.to_string(),
    }
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
    sex: ssr_core::mechanics::Sex,
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
            let key = part_key(species, &sexed_part(part, sex));
            let Some(sprite) = registry.get(&key) else {
                continue;
            };
            let mut component = Sprite::from_image(sprite.image.clone());
            component.texture_atlas = Some(TextureAtlas {
                layout: sprite.layout.clone(),
                index: sprite.index(0, 0),
            });
            // Арт частей нейтральный: цвет кожи задаёт тонировка (как в SS14).
            component.color = DEFAULT_SKIN;
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
            component.color = DEFAULT_EYES;
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
    pub sexes: Query<'w, 's, &'static ssr_core::mechanics::Sex>,
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
        sexes,
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

    // Отпечаток тела: раса + пол (половые варианты только у head/chest/groin).
    let body_signature = |entity: Entity| {
        let sex = sexes.get(entity).copied().unwrap_or_default();
        format!("{}{}", species_of(entity), sex.part_suffix())
    };
    if let Some(own_entity) = own.0 {
        let species_id = species_of(own_entity);
        let sex = sexes.get(own_entity).copied().unwrap_or_default();
        for player_entity in players.iter() {
            let signature = body_signature(player_entity);
            if bodies.get(player_entity).ok().map(|b| b.0.as_str()) == Some(signature.as_str()) {
                continue;
            }
            detach_body(&mut commands, &children, player_entity);
            let parts = attach_body(&mut commands, &registry, player_entity, &species_id, sex);
            // Спрайты грузятся лениво (T5.3): пока частей не хватает — не
            // помечаем тело собранным, на следующем кадре попробуем снова.
            if parts < PARTS.len() {
                continue;
            }
            commands
                .entity(player_entity)
                .insert(BodySpecies(signature));
            tracing::info!(species = %species_id, sex = ?sex, parts, "player body attached");
        }
    }

    for (visual_entity, visual) in visuals.iter() {
        // Свой игрок рисуется отдельным визуалом Player: его дубль убирает
        // sync_remote_players — здесь тело ему собирать не нужно.
        if Some(visual.player) == own.0 {
            continue;
        }
        let species_id = species_of(visual.player);
        let sex = sexes.get(visual.player).copied().unwrap_or_default();
        let signature = body_signature(visual.player);
        if bodies.get(visual_entity).ok().map(|b| b.0.as_str()) == Some(signature.as_str()) {
            continue;
        }
        detach_body(&mut commands, &children, visual_entity);
        let parts = attach_body(&mut commands, &registry, visual_entity, &species_id, sex);
        if parts < PARTS.len() {
            continue;
        }
        commands
            .entity(visual_entity)
            .insert(BodySpecies(signature));
        tracing::info!(player = ?visual.player, species = %species_id, sex = ?sex, parts, "remote body attached");
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
            if part.key.contains("equipped") {
                tracing::debug!(key = %part.key, facing = facing.0, "worn layer turned");
            }
        }
    }
}

/// Отпечаток внешности: что надето, причёска, борода и НАПРАВЛЕНИЕ владельца —
/// при повороте слои пересобираются заново, поэтому одежда гарантированно смотрит
/// туда же, куда и тело (была жалоба, что одежда «поворачивается отдельно»).
type WornSignature = (
    Vec<(ssr_core::clothing::ClothingSlot, u64)>,
    Option<(String, [u8; 3])>,
    Option<(String, [u8; 3])>,
    u32,
);

/// Слой надетой одежды (для пересборки при смене одежды).
#[derive(Component)]
pub struct WornLayer {
    pub owner: Entity,
}

/// Доступ к каталогу и предметам для одежды (bits → имя → спрайт `worn`).
#[derive(bevy::ecs::system::SystemParam)]
pub struct WornContext<'w, 's> {
    content: Res<'w, crate::content::ClientContent>,
    items: Query<'w, 's, &'static ssr_core::inventory::Item>,
    entity_map: Option<Res<'w, bevy_replicon::shared::server_entity_map::ServerEntityMap>>,
}

impl WornContext<'_, '_> {
    /// Ключ спрайта «надетым» по bits предмета.
    fn key(&self, bits: u64) -> Option<String> {
        let server = Entity::try_from_bits(bits)?;
        let client = self
            .entity_map
            .as_deref()?
            .to_client()
            .get(&server)
            .copied()?;
        let name = self.items.get(client).ok()?.name.clone();
        let worn = self.content.items.worn_of(&name)?;
        Some(format!("sprites/ss14/{worn}"))
    }
}

/// Рисует надетую одежду поверх тела: спрайт `equipped-*` из каталога,
/// z — из [`ssr_core::clothing::ClothingSlot::layer_z`] (порядок `base.yml`).
/// Направление обновляет общая система `update_facing` (тот же `HumanoidPart`).
#[allow(clippy::too_many_arguments)]
pub fn sync_worn_clothes(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    context: WornContext,
    clothings: Query<(
        Entity,
        &ssr_core::clothing::Clothing,
        Option<&ssr_core::mechanics::Hair>,
        Option<&ssr_core::mechanics::FacialHair>,
    )>,
    visuals: Query<(Entity, &crate::inventory_ui::RemotePlayerVisual)>,
    own_visual: Res<crate::inventory_ui::OwnPlayerEntity>,
    body_owners: Query<Entity, With<crate::Player>>,
    parts: Query<&HumanoidPart>,
    children_of: Query<&Children>,
    facings: Query<&Facing>,
    worn: Query<(Entity, &WornLayer)>,
    mut last: Local<std::collections::HashMap<Entity, WornSignature>>,
) {
    for (entity, clothing, hair, beard) in clothings.iter() {
        let facing = facings
            .get(entity)
            .map(|facing| facing.0)
            .unwrap_or_default();
        let signature = (
            clothing.slots.clone(),
            hair.map(|hair| (hair.style.clone(), hair.color)),
            beard.map(|beard| (beard.style.clone(), beard.color)),
            facing,
        );
        if last.get(&entity) == Some(&signature) {
            continue;
        }
        // Визуал игрока: у своего — сама сущность (если уже разрешена), у чужого —
        // его визуал. Дубль визуала своего игрока брать нельзя: он удаляется
        // в sync_remote_players вместе с детьми, и одежда исчезала («кукла голая»).
        // Тело рисуется на сущности с маркером `Player` (свой) или на визуале
        // чужого игрока — одежда обязана висеть на ТОЙ ЖЕ сущности, иначе её
        // слои уходят в мировое начало координат (тело-родитель другой).
        let visual = if Some(entity) == own_visual.0 {
            body_owners.iter().next()
        } else if own_visual.0.is_none() {
            None
        } else {
            visuals
                .iter()
                .filter(|(_, remote)| remote.player != own_visual.0.unwrap())
                .find(|(_, remote)| remote.player == entity)
                .map(|(visual, _)| visual)
        };
        // Проверяем, что на цели действительно собрано тело: иначе ждём.
        let has_body = visual.is_some_and(|target| {
            children_of
                .get(target)
                .map(|children| children.iter().any(|child| parts.get(child).is_ok()))
                .unwrap_or(false)
        });
        let Some(visual) = visual.filter(|_| has_body) else {
            last.remove(&entity);
            continue;
        };
        tracing::debug!(?entity, ?visual, "worn clothes target resolved");
        for (layer, worn) in worn.iter() {
            if worn.owner == visual {
                commands.entity(layer).despawn();
            }
        }
        // Направление владельца — чтобы слои появились уже повёрнутыми.
        let facing = facings.get(visual).map(|facing| facing.0).unwrap_or(0);
        let mut spawned = 0;
        let helmet = clothing
            .get(ssr_core::clothing::ClothingSlot::Head)
            .is_some();
        if let Some(hair) = hair
            && !helmet
        {
            let key = format!(
                "sprites/ss14/Mobs/Customization/human_hair.rsi#{}",
                hair.style
            );
            if let Some(sprite) = registry.get(&key) {
                let mut component = Sprite::from_image(sprite.image.clone());
                component.texture_atlas = Some(TextureAtlas {
                    layout: sprite.layout.clone(),
                    index: sprite.index(facing.min(3), 0),
                });
                component.color = Color::srgb_u8(hair.color[0], hair.color[1], hair.color[2]);
                commands.entity(visual).with_children(|parent| {
                    parent.spawn((
                        HumanoidPart { owner: visual, key },
                        WornLayer { owner: visual },
                        component,
                        // Между шеей (1.21) и шлемом (1.22): шлем перекрывает волосы.
                        Transform::from_xyz(0.0, 0.0, 0.212),
                    ));
                });
                spawned += 1;
            }
        }
        // Борода — отдельный маркинг (слой FacialHair в SS14): ниже волос,
        // скрывается маской и шлемом.
        let beard_hidden = helmet
            || clothing
                .get(ssr_core::clothing::ClothingSlot::Mask)
                .is_some();
        if let Some(beard) = beard
            && !beard_hidden
        {
            let key = format!(
                "sprites/ss14/Mobs/Customization/human_facial_hair.rsi#{}",
                beard.style
            );
            if let Some(sprite) = registry.get(&key) {
                let mut component = Sprite::from_image(sprite.image.clone());
                component.texture_atlas = Some(TextureAtlas {
                    layout: sprite.layout.clone(),
                    index: sprite.index(facing.min(3), 0),
                });
                component.color = Color::srgb_u8(beard.color[0], beard.color[1], beard.color[2]);
                commands.entity(visual).with_children(|parent| {
                    parent.spawn((
                        HumanoidPart { owner: visual, key },
                        WornLayer { owner: visual },
                        component,
                        Transform::from_xyz(0.0, 0.0, 0.2115),
                    ));
                });
                spawned += 1;
            }
        }
        commands.entity(visual).with_children(|parent| {
            for (slot, item) in clothing.slots.iter() {
                let Some(key) = context.key(*item) else {
                    continue;
                };
                let Some(sprite) = registry.get(&key) else {
                    continue;
                };
                let mut component = Sprite::from_image(sprite.image.clone());
                component.texture_atlas = Some(TextureAtlas {
                    layout: sprite.layout.clone(),
                    index: sprite.index(facing.min(3), 0),
                });
                parent.spawn((
                    HumanoidPart { owner: visual, key },
                    WornLayer { owner: visual },
                    component,
                    // Абсолютный z = корень моба (1.0) + этот сдвиг: в SS14 слои
                    // относительны к мобу, а тьма/туман живут на 2.01 — одежда обязана
                    // быть ниже них, иначе «просвечивает» из тёмных зон.
                    Transform::from_xyz(0.0, 0.0, slot.layer_z() - 1.0),
                ));
                spawned += 1;
            }
        });
        // Кэшируем отпечаток только когда все слои создались: RSI грузятся лениво,
        // иначе пересборка шла бы КАЖДЫЙ кадр (из-за этого слои вечно пересоздавались
        // и гонка с update_facing оставляла одежду повёрнутой на юг).
        let expected = clothing.slots.len()
            + usize::from(hair.is_some() && !helmet)
            + usize::from(beard.is_some() && !beard_hidden);
        if spawned == expected {
            last.insert(entity, signature);
        }
        tracing::info!(
            slots = clothing.slots.len(),
            spawned,
            "worn clothes updated"
        );
    }
}
