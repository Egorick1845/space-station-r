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

/// «Звёзды» над головой при стамина-крите (`StunVisualLayers.StamCrit` в сборке:
/// слой поверх всех, offset (0, 0.3125), RSI `Mobs/Effects/stunned.rsi#stunned`,
/// 8 кадров по 0.1 с).
#[derive(Component)]
pub struct StunStars {
    /// Владелец (визуал), к которому привязан слой.
    pub owner: Entity,
    /// Текущий кадр флипбука.
    pub frame: u32,
    /// Время в текущем кадре.
    pub elapsed: f32,
}

/// Смещение слоя звёзд по Y (`StunSystem`: `offset (0, 0.3125)`).
const STUN_STARS_OFFSET_Y: f32 = 0.62;
/// Ключ спрайта звёзд (`StunVisualsComponent`: `Mobs/Effects/stunned.rsi#stunned`).
const STUN_STARS_KEY: &str = "sprites/ss14/Mobs/Effects/stunned.rsi#stunned";
/// z слоя звёзд: выше всех слоёв тела и одежды (`StunVisualLayers.StamCrit`).
const STUN_STARS_Z: f32 = 0.5;

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
    settings: Res<crate::settings::Settings>,
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
    // Призраки видны только призракам и при `showghosts on`: в сборке живые не
    // видят призраков вне PostRound (`GhostSystem.OnGhostStartup` перекладывает
    // слои видимости `Ghost`/`Normal`, показ включается командой `showghosts`).
    let own_is_ghost = own
        .0
        .map(|entity| ghosts.get(entity).is_ok())
        .unwrap_or(false);
    let show_all = own_is_ghost || settings.show_ghosts;
    let ghost_visible = |entity: Entity| show_all || ghosts.get(entity).is_err();

    if let Some(own_entity) = own.0 {
        let species_id = species_of(own_entity);
        let sex = sexes.get(own_entity).copied().unwrap_or_default();
        for player_entity in players.iter() {
            if !ghost_visible(player_entity) {
                continue;
            }
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
        // Призрак невидим живым (`visibilityMask` в `observer.yml`).
        if !ghost_visible(visual.player) {
            if bodies.get(visual_entity).is_ok() {
                detach_body(&mut commands, &children, visual_entity);
            }
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
///
/// В сборке это `RotationVisualsComponent` + `SharedRotationVisualsSystem`:
/// визуал поворачивается на 90° за `AnimationTime = 0.125` с (плавно, не рывком).
/// Важно: у СВОЕГО игрока состояние падения приходит на реплицированную
/// сущность (`OwnPlayerEntity`), а локальный визуал `Player` — отдельная
/// сущность, поэтому раньше падение своего персонажа не рисовалось вообще.
pub fn update_knocked(
    time: Res<Time>,
    own: Res<OwnPlayerEntity>,
    knocked: Query<&KnockedDown>,
    mut bodies: Query<(Entity, &mut Transform), With<crate::Player>>,
    mut visuals: Query<(Entity, &RemotePlayerVisual, &mut Transform), Without<crate::Player>>,
    mut progress: Local<std::collections::HashMap<Entity, f32>>,
    mut last_own: Local<bool>,
) {
    let dt = time.delta_secs();
    // Свой игрок: падение определяем по реплицированной сущности.
    let own_down = own.0.is_some_and(|entity| knocked.contains(entity));
    if own_down != *last_own {
        tracing::info!(own = ?own.0, own_down, "knocked state (own player)");
        *last_own = own_down;
    }
    for (entity, mut transform) in bodies.iter_mut() {
        let angle = advance_body_rotation(
            progress.entry(entity).or_default(),
            std::f32::consts::FRAC_PI_2,
            own_down,
            dt,
        );
        let target = Quat::from_rotation_z(angle);
        if transform.rotation != target {
            transform.rotation = target;
        }
    }
    for (entity, visual, mut transform) in visuals.iter_mut() {
        let angle = advance_body_rotation(
            progress.entry(entity).or_default(),
            std::f32::consts::FRAC_PI_2,
            knocked.contains(visual.player),
            dt,
        );
        let target = Quat::from_rotation_z(angle);
        if transform.rotation != target {
            transform.rotation = target;
        }
    }
}

/// Время поворота на 90° (`RotationVisualsComponent.AnimationTime` в сборке).
const ROTATION_TIME: f32 = 0.125;

/// Ведёт угол поворота к цели со скоростью «90° за [`ROTATION_TIME`]».
/// Возвращает текущий угол (0 — стоит, π/2 — лежит).
fn advance_body_rotation(current: &mut f32, limit: f32, down: bool, dt: f32) -> f32 {
    let target = if down { limit } else { 0.0 };
    let speed = if ROTATION_TIME > 0.0 {
        limit / ROTATION_TIME
    } else {
        limit
    };
    let step = speed * dt.max(0.0);
    if (*current - target).abs() <= step {
        *current = target;
    } else if *current < target {
        *current += step;
    } else {
        *current -= step;
    }
    *current
}

/// Фаза и прошлая позиция владельца для анимации шага.
#[derive(Default)]
pub(crate) struct FootPhase {
    phase: f32,
    last: Vec2,
}

// Параметры `FootWalkAnimationComponent` из сборки (`_Mini/FootWalk`): цикл 9 рад/с,
// множители 0.6375 (ходьба) и 1.2025 (бег), зажим от реальной скорости 0.35..1.1,
// порог остановки 0.04 (м/с)², амплитуда подъёма стопы 2.5/32 юнита.
const FOOT_CYCLE_SPEED: f32 = 9.0;
const FOOT_WALK_RATE: f32 = 0.6375;
const FOOT_SPRINT_RATE: f32 = 1.2025;
const FOOT_MIN_SLOW: f32 = 0.35;
const FOOT_MAX_SLOW: f32 = 1.1;
const FOOT_MIN_SPEED_SQR: f32 = 0.04;
/// Порог и крит выносливости — те же числа, что в ядре (`StaminaComponent`).
const BREATHING_THRESHOLD: f32 = ssr_core::stamina::BREATHING_THRESHOLD;
const CRIT_THRESHOLD: f32 = ssr_core::stamina::CRIT_THRESHOLD;
const FOOT_AMPLITUDE: f32 = 2.5 / 32.0;
/// Дальняя нога в боковых видах (`SideFarAmplitudeFactor`).
const FOOT_FAR_AMPLITUDE_FACTOR: f32 = 0.4;
/// Юнитов мира в тайле (скорость в сборке — метры/с, 1 тайл = 1 м).
const TILE_UNITS: f32 = ssr_core::tiles::TILE_PX as f32;
/// Ожидаемые скорости из `MovementSpeedModifierComponent`: ходьба и бег, м/с
/// (`DefaultBaseWalkSpeed = 2.5`, `DefaultBaseSprintSpeed = 4.5`).
const WALK_EXPECTED: f32 = 2.5;
const SPRINT_EXPECTED: f32 = 4.5;

/// Процедурная анимация шага (`FootWalkAnimationSystem` сборки): кадров ходьбы в
/// RSI нет — ноги и стопы поднимаются синусом от фазы, которая растёт со
/// скоростью. Поднимается только нога/стопа (руки не анимируются — как в сборке).
///
/// Множитель фазы берётся из ФЛАГА спринта (`MoverComponent.Sprinting`), а не из
/// фактической скорости: у нас это «не держит Shift» (в сборке `DefaultSprinting`
/// = true), у остальных игроков — по реплицированному `Sprinting`/скорости.
/// Фактическая скорость входит только через `slowFactor` (`GetStepRate`).
#[allow(clippy::too_many_arguments)]
pub fn foot_walk_animation(
    time: Res<Time>,
    settings: Res<crate::settings::Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    own: Res<OwnPlayerEntity>,
    knocked: Query<&KnockedDown>,
    players: Query<(), With<crate::Player>>,
    sprintings: Query<&ssr_core::mechanics::Sprinting>,
    facings: Query<&Facing>,
    mut walk: Local<std::collections::HashMap<Entity, FootPhase>>,
    mut parts: Query<(&HumanoidPart, &mut Transform)>,
    owners: Query<&GlobalTransform, Without<HumanoidPart>>,
    visuals: Query<&crate::inventory_ui::RemotePlayerVisual>,
    staminas: Query<&ssr_core::stamina::Stamina>,
) {
    // Настройка «Доступность» → «Анимация шага»
    // (`accessibility.foot_walk_animation` в сборке).
    if !settings.foot_walk_animation {
        for (_, mut transform) in parts.iter_mut() {
            transform.translation.y = 0.0;
        }
        return;
    }
    // Лежачий не шагает: в сборке анимация ног выключается вне
    // `StandingState.Standing` (лежит/оглушён/мёртв) — `CanAnimate`.
    let own_down = own.0.is_some_and(|entity| knocked.contains(entity));
    let is_down = |owner: Entity| -> bool {
        if players.get(owner).is_ok() {
            return own_down;
        }
        knocked.contains(owner)
            || visuals
                .get(owner)
                .is_ok_and(|visual| knocked.contains(visual.player))
    };
    // Спринт в сборке — «игрок не держит кнопку Walk» (`DefaultSprinting = true`).
    let own_sprinting = !keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let is_sprinting = |owner: Entity| -> bool {
        if players.get(owner).is_ok() {
            return own_sprinting;
        }
        match visuals.get(owner) {
            // Реплицированный спринт-тоггл (`SprinterComponent`) у чужого игрока.
            Ok(visual) if sprintings.get(visual.player).is_ok_and(|flag| flag.0) => true,
            _ => false,
        }
    };
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    // Фаза считается один раз на владельца (иначе скорость шага умножилась бы
    // на число его частей).
    let mut phases: std::collections::HashMap<Entity, f32> = std::collections::HashMap::new();
    for (part, _) in parts.iter() {
        if phases.contains_key(&part.owner) {
            continue;
        }
        if is_down(part.owner) {
            phases.insert(part.owner, 0.0);
            continue;
        }
        let Ok(global) = owners.get(part.owner) else {
            continue;
        };
        let pos = global.translation().truncate();
        let entry = walk.entry(part.owner).or_insert(FootPhase {
            phase: 0.0,
            last: pos,
        });
        let speed_tiles = (pos - entry.last).length() / dt / TILE_UNITS;
        entry.last = pos;
        if speed_tiles * speed_tiles < FOOT_MIN_SPEED_SQR {
            entry.phase = 0.0;
            phases.insert(part.owner, 0.0);
            continue;
        }
        // Спринт — по флагу; у чужих, если флага нет, по фактической скорости.
        let sprinting = is_sprinting(part.owner) || speed_tiles > WALK_EXPECTED;
        let expected = if sprinting {
            SPRINT_EXPECTED
        } else {
            WALK_EXPECTED
        };
        let slow = (speed_tiles / expected).clamp(FOOT_MIN_SLOW, FOOT_MAX_SLOW);
        let rate = if sprinting {
            FOOT_SPRINT_RATE
        } else {
            FOOT_WALK_RATE
        };
        entry.phase += dt * FOOT_CYCLE_SPEED * rate * slow;
        phases.insert(part.owner, entry.phase);
    }
    // Дыхание при усталости (`StaminaComponent`: порог 50, частота
    // 0.25 + шаг×1.75 Гц, подъём 0.04 + шаг×0.04 юнита; первая половина цикла —
    // вверх, затем вниз до −12.5 % амплитуды и возврат).
    let elapsed = time.elapsed_secs();
    let mut breathing: std::collections::HashMap<Entity, f32> = std::collections::HashMap::new();
    for (part, _) in parts.iter() {
        if breathing.contains_key(&part.owner) {
            continue;
        }
        let player = visuals
            .get(part.owner)
            .map(|visual| visual.player)
            .unwrap_or(part.owner);
        let Ok(stamina) = staminas.get(player) else {
            breathing.insert(part.owner, 0.0);
            continue;
        };
        if !stamina.breathing() {
            breathing.insert(part.owner, 0.0);
            continue;
        }
        let step = ((stamina.damage - BREATHING_THRESHOLD)
            / (CRIT_THRESHOLD - BREATHING_THRESHOLD))
            .clamp(0.0, 1.0);
        let frequency = 0.25 + step * 1.75;
        let amplitude = 0.04 + step * 0.04;
        let phase = (elapsed * frequency).fract();
        let wave = if phase < 0.5 {
            (phase * 2.0 * std::f32::consts::PI).sin()
        } else {
            -0.125 * ((phase - 0.5) * 2.0 * std::f32::consts::PI).sin()
        };
        breathing.insert(part.owner, wave * amplitude);
    }
    for (part, mut transform) in parts.iter_mut() {
        let name = part.key.rsplit('#').next().unwrap_or_default();
        if is_down(part.owner) {
            transform.translation.y = 0.0;
            continue;
        }
        let left = name.starts_with("l_leg") || name.starts_with("l_foot");
        let right = name.starts_with("r_leg") || name.starts_with("r_foot");
        // Смещение ног по `FootWalkAnimationSystem`:
        // leftY = max(0, sin(Phase)) × leftAmp, rightY = то же с Phase+π;
        // дальняя нога в боковых видах — ×0.4 (`SideFarAmplitudeFactor`),
        // East → дальняя левая, West → дальняя правая; в боковом виде обе ноги
        // идут по nearY = (East ? rightY : leftY), в фронтальном (S/N) каждая
        // нога по своей фазе. Смещение всегда (0, y) — по X сдвига нет.
        let phase = phases.get(&part.owner).copied().unwrap_or(0.0);
        let facing = facings.get(part.owner).map(|f| f.0).unwrap_or(0);
        let (left_y, right_y, near_y) = foot_offsets(facing, phase);
        let front = facing == 0 || facing == 1;
        let foot = if front {
            if left {
                left_y
            } else if right {
                right_y
            } else {
                0.0
            }
        } else if left || right {
            // Боковой вид: силуэт ног один, идёт по ближней ноге.
            near_y
        } else {
            // Руки и прочие части в шаге не участвуют (как в сборке), но дышат
            // вместе с телом.
            0.0
        };
        let breath = breathing.get(&part.owner).copied().unwrap_or(0.0);
        transform.translation.y = breath + foot;
    }
}

/// Смещение ног по фазе и направлению взгляда — ядро `FootWalkAnimationSystem`:
/// `leftY = max(0, sin(Phase)) * leftAmp`, `rightY` — тот же синус с `+ PI`
/// (строгая противофаза, подъём только вверх); дальняя нога в боковом виде
/// получает `SideFarAmplitudeFactor = 0.4` (East — дальняя левая, West — правая);
/// `nearY` — ближняя нога (`East ? rightY : leftY`), по ней идут обе ноги в боку.
/// Возвращает `(leftY, rightY, nearY)`.
pub(crate) fn foot_offsets(facing: u32, phase: f32) -> (f32, f32, f32) {
    let mut left_amp = FOOT_AMPLITUDE;
    let mut right_amp = FOOT_AMPLITUDE;
    if facing == 2 {
        left_amp *= FOOT_FAR_AMPLITUDE_FACTOR;
    } else if facing == 3 {
        right_amp *= FOOT_FAR_AMPLITUDE_FACTOR;
    }
    let left_y = phase.sin().max(0.0) * left_amp;
    let right_y = (phase + std::f32::consts::PI).sin().max(0.0) * right_amp;
    let near_y = if facing == 2 { right_y } else { left_y };
    (left_y, right_y, near_y)
}

/// Появление/исчезновение слоя «звёзд» при стамина-крите. В сборке это
/// `StunSystem` (клиент): слой `StunVisualLayers.StamCrit` включается, когда у
/// сущности есть `SeeingStars` (ставится при стамина-крите), и выключается при
/// снятии стана. У нас признак — реплицированный `KnockedDown`.
pub fn sync_stun_stars(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    own: Res<OwnPlayerEntity>,
    knocked: Query<&KnockedDown>,
    players: Query<Entity, With<crate::Player>>,
    visuals: Query<(Entity, &RemotePlayerVisual)>,
    stars: Query<(Entity, &StunStars)>,
) {
    // Кому положены звёзды: свой игрок — по реплицированной сущности,
    // остальные — по `RemotePlayerVisual.player`.
    let own_down = own.0.is_some_and(|entity| knocked.contains(entity));
    let mut wanted: Vec<Entity> = Vec::new();
    for entity in players.iter() {
        if own_down {
            wanted.push(entity);
        }
    }
    for (entity, visual) in visuals.iter() {
        if knocked.contains(visual.player) {
            wanted.push(entity);
        }
    }
    // Убираем звёзды у тех, кто уже не лежит.
    for (entity, star) in stars.iter() {
        if !wanted.contains(&star.owner) {
            commands.entity(entity).despawn();
        }
    }
    let existing: Vec<Entity> = stars.iter().map(|(_, star)| star.owner).collect();
    let Some(sprite) = registry.get(STUN_STARS_KEY) else {
        return;
    };
    for owner in wanted {
        if existing.contains(&owner) {
            continue;
        }
        let mut component = Sprite::from_image(sprite.image.clone());
        component.texture_atlas = Some(TextureAtlas {
            layout: sprite.layout.clone(),
            index: sprite.index(0, 0),
        });
        commands.entity(owner).with_children(|parent| {
            parent.spawn((
                StunStars {
                    owner,
                    frame: 0,
                    elapsed: 0.0,
                },
                component,
                Transform::from_xyz(0.0, STUN_STARS_OFFSET_Y, STUN_STARS_Z),
            ));
        });
    }
}

/// Флипбук «звёзд»: кадры по `delays` RSI (в `stunned.rsi` — 8 кадров по 0.1 с).
pub fn animate_stun_stars(
    time: Res<Time>,
    registry: Res<RsiRegistry>,
    mut stars: Query<(&mut StunStars, &mut Sprite)>,
) {
    if stars.is_empty() {
        return;
    }
    let dt = time.delta_secs();
    let Some(sprite) = registry.get(STUN_STARS_KEY) else {
        return;
    };
    let frames = sprite
        .frames_per_direction
        .first()
        .copied()
        .unwrap_or(1)
        .max(1);
    for (mut star, mut image) in stars.iter_mut() {
        star.elapsed += dt;
        let delays = sprite.delays.first().map(Vec::as_slice).unwrap_or(&[]);
        let (elapsed, frame) =
            crate::hud::advance_alert_frame(star.elapsed, star.frame, delays, frames);
        star.elapsed = elapsed;
        if frame != star.frame {
            star.frame = frame;
            if let Some(atlas) = image.texture_atlas.as_mut() {
                atlas.index = sprite.index(0, frame);
            }
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
    // Призрачность: тело-призрак перерисовывается без одежды-детей, и после
    // возврата в тело слои нужно пересоздать (жалоба «тело голое»).
    bool,
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
    ghosts: Query<'w, 's, &'static ssr_core::mechanics::Ghost>,
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
    mut images: ResMut<Assets<Image>>,
    layouts: Res<Assets<TextureAtlasLayout>>,
    mut halves: Local<HalvesCache>,
) {
    for (entity, clothing, hair, beard) in clothings.iter() {
        let facing = facings
            .get(entity)
            .map(|facing| facing.0)
            .unwrap_or_default();
        let signature = (
            context.ghosts.get(entity).is_ok(),
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
                // Обувь — ДВЕ половинки с ключами l_foot/r_foot: в сборке обувь
                // режется на левую/правую (`FootWalkAnimationSystem`, шейдер
                // SpriteFootHalfClip) и половинки двигаются с ногами — иначе
                // ходьба в обуви невидима (ноги скрыты комбинезоном).
                if *slot == ssr_core::clothing::ClothingSlot::Shoes {
                    for right_side in [false, true] {
                        let Some((atlas_index, image)) = shoe_half(
                            &registry,
                            &mut images,
                            &layouts,
                            &mut halves,
                            &key,
                            facing,
                            right_side,
                        ) else {
                            continue;
                        };
                        let mut half_sprite = Sprite::from_image(image);
                        half_sprite.texture_atlas = Some(TextureAtlas {
                            layout: sprite.layout.clone(),
                            index: atlas_index,
                        });
                        parent.spawn((
                            HumanoidPart {
                                owner: visual,
                                key: if right_side { "r_foot" } else { "l_foot" }.to_string(),
                            },
                            WornLayer { owner: visual },
                            half_sprite,
                            // Половинка центрируется на своей стороне кадра.
                            Transform::from_xyz(
                                if right_side { 0.25 } else { -0.25 },
                                0.0,
                                slot.layer_z() - 1.0,
                            ),
                        ));
                    }
                    spawned += 2;
                    continue;
                }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Амплитуда подъёма ноги — `2.5 / 32` юнита (2.5 px при 32 px на тайл).
    fn amp() -> f32 {
        2.5 / 32.0
    }

    /// Ноги строго в противофазе и поднимаются ТОЛЬКО вверх (фронтальный вид).
    #[test]
    fn legs_move_in_counterphase_upwards_only() {
        let half_pi = std::f32::consts::FRAC_PI_2;
        // Phase = π/2: левая нога вверху, правая на месте.
        let (left, right, _) = foot_offsets(0, half_pi);
        assert!((left - amp()).abs() < 1e-6, "левая на полной амплитуде");
        assert!(right.abs() < 1e-6, "правая внизу");
        // Phase = 3π/2: наоборот.
        let (left, right, _) = foot_offsets(0, 3.0 * half_pi);
        assert!(left.abs() < 1e-6, "левая внизу");
        assert!((right - amp()).abs() < 1e-6, "правая на полной амплитуде");
        // Отрицательная полуволна синуса не опускает ногу ниже базы.
        let (left, right, _) = foot_offsets(0, 1.5 * std::f32::consts::PI * 1.5);
        assert!(left >= 0.0 && right >= 0.0, "смещение только вверх");
    }

    /// Дальняя нога в боковом виде получает ×0.4 (`SideFarAmplitudeFactor`):
    /// East (2) — дальняя левая, West (3) — дальняя правая.
    #[test]
    fn side_view_far_leg_gets_forty_percent() {
        let half_pi = std::f32::consts::FRAC_PI_2;
        let (left, _, _) = foot_offsets(2, half_pi);
        assert!(
            (left - amp() * FOOT_FAR_AMPLITUDE_FACTOR).abs() < 1e-6,
            "East: левая (дальняя) = 0.4 амплитуды, получено {left}"
        );
        // В боку обе ноги идут по ближней: East — по правой.
        let (_, right, near_east) = foot_offsets(2, 3.0 * half_pi);
        assert!((right - amp()).abs() < 1e-6);
        assert!(
            (near_east - right).abs() < 1e-6,
            "East: nearY — правая (ближняя)"
        );
        // West (3): дальняя правая — она поднимается на фазе 3π/2.
        let (left_w, right_w, near_west) = foot_offsets(3, 3.0 * half_pi);
        assert!(
            (right_w - amp() * FOOT_FAR_AMPLITUDE_FACTOR).abs() < 1e-6,
            "West: правая (дальняя) = 0.4 амплитуды, получено {right_w}"
        );
        assert!(
            (near_west - left_w).abs() < 1e-6,
            "West: nearY — левая (ближняя), получено {near_west}"
        );
    }

    /// Поворот при падении идёт ровно 0.125 с (`RotationVisualsComponent`),
    /// то есть 90° за 4 кадра по 1/32 с, и без перелёта.
    #[test]
    fn body_rotation_takes_125_ms() {
        let mut angle = 0.0;
        let dt = 0.03125;
        let limit = std::f32::consts::FRAC_PI_2;
        let mut steps = 0;
        while angle < limit && steps < 100 {
            advance_body_rotation(&mut angle, limit, true, dt);
            steps += 1;
        }
        assert_eq!(steps, 4, "90° за 4 шага по 1/32 с = 0.125 с");
        assert!((angle - limit).abs() < 1e-6, "остановился точно на 90°");
        // Обратно — тоже 0.125 с.
        while angle > 0.0 && steps < 100 {
            advance_body_rotation(&mut angle, limit, false, dt);
            steps += 1;
        }
        assert!(angle.abs() < 1e-6, "встал обратно");
    }
}

/// Иконка профессии над головой (`ShowJobIconsSystem` в сборке: состояние
/// `job_icons.rsi#<JobName>`; у нас — реплицированное `PlayerRole.icon`).
#[derive(Component)]
pub struct JobIconOverlay {
    owner: Entity,
    state: String,
}

/// SSD-иконка над головой (`SSDIndicatorComponent`: спит/отвалившийся игрок,
/// спрайт `Effects/ssd.rsi#default0` — скопирован из сборки).
#[derive(Component)]
pub struct SsdOverlay {
    owner: Entity,
}

/// Полоска здоровья над головой (упрощённый `EntityHealthBarOverlay`).
#[derive(Component)]
pub struct HealthBarOverlay {
    owner: Entity,
}

const OVERHEAD_Y: f32 = 0.62;
const OVERHEAD_Z: f32 = 0.6;
const JOB_ICONS_PREFIX: &str = "sprites/ss14/Interface/Misc/job_icons.rsi#";
const SSD_ICON_KEY: &str = "sprites/ss14/Effects/ssd.rsi#default0";
const HEALTH_BAR_WIDTH: f32 = 0.5;
const HEALTH_BAR_HEIGHT: f32 = 0.045;

/// Белая текстура полоски здоровья (цвет — тонировкой спрайта).
#[derive(Resource)]
pub struct HealthBarTexture(pub Handle<Image>);

/// Создаёт текстуру полоски один раз.
pub fn setup_health_bar(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let handle = images.add(Image::new_fill(
        bevy::render::render_resource::Extent3d {
            width: 2,
            height: 2,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        &[255, 255, 255, 255],
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    ));
    commands.insert_resource(HealthBarTexture(handle));
}

/// Визоры над головой: должность (`PlayerRole.icon`), SSD (`Ssd`) и полоска
/// здоровья. Детские спрайты — по образцу `sync_stun_stars`.
#[allow(clippy::too_many_arguments)]
pub fn sync_overheads(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    own: Res<OwnPlayerEntity>,
    players: Query<Entity, With<crate::Player>>,
    visuals: Query<(Entity, &RemotePlayerVisual)>,
    roles: Query<&ssr_core::roles::PlayerRole>,
    ssds: Query<&ssr_core::mechanics::Ssd>,
    healths: Query<&ssr_core::inventory::Health>,
    jobs: Query<(Entity, &JobIconOverlay)>,
    ssd_icons: Query<(Entity, &SsdOverlay)>,
    bars: Query<(Entity, &HealthBarOverlay)>,
    bar_texture: Option<Res<HealthBarTexture>>,
    childrens: Query<&Children>,
    mut fills: Query<(Entity, &mut Sprite), With<HealthBarFill>>,
) {
    // (сущность визора, сущность с ДАННЫМИ): свой игрок смотрит в себя,
    // удалённый визуал — в реплицированное тело.
    let mut owners: Vec<(Entity, Entity)> = players
        .iter()
        .filter_map(|entity| own.0.filter(|data| data == &entity).map(|data| (entity, data)))
        .collect();
    owners.extend(
        visuals
            .iter()
            .map(|(entity, visual)| (entity, visual.player)),
    );

    // --- Должность: состояние из PlayerRole.icon ---
    for (owner, data) in owners.iter() {
        let state = roles
            .get(*data)
            .map(|role| role.icon.clone())
            .unwrap_or_default();
        let existing = jobs.iter().find(|(_, overlay)| overlay.owner == *owner);
        if state.is_empty() {
            if let Some((entity, _)) = existing {
                commands.entity(entity).despawn();
            }
            continue;
        }
        if existing.as_ref().is_some_and(|(_, overlay)| overlay.state == state) {
            continue;
        }
        if let Some((entity, _)) = existing {
            commands.entity(entity).despawn();
        }
        let Some(sprite) = registry.get(&format!("{JOB_ICONS_PREFIX}{state}")) else {
            continue;
        };
        let mut component = Sprite::from_image(sprite.image.clone());
        component.texture_atlas = Some(TextureAtlas {
            layout: sprite.layout.clone(),
            index: sprite.index(0, 0),
        });
        let state_clone = state.clone();
        commands.entity(*owner).with_children(move |parent| {
            parent.spawn((
                JobIconOverlay {
                    owner: *owner,
                    state: state_clone,
                },
                component,
                Transform::from_xyz(0.28, OVERHEAD_Y, OVERHEAD_Z),
            ));
        });
    }

    // --- SSD ---
    for (owner, data) in &owners {
        let sleeping = ssds.contains(*data);
        let existing = ssd_icons.iter().find(|(_, overlay)| overlay.owner == *owner);
        if !sleeping {
            if let Some((entity, _)) = existing {
                commands.entity(entity).despawn();
            }
            continue;
        }
        if existing.is_some() {
            continue;
        }
        let Some(sprite) = registry.get(SSD_ICON_KEY) else {
            continue;
        };
        let mut component = Sprite::from_image(sprite.image.clone());
        component.texture_atlas = Some(TextureAtlas {
            layout: sprite.layout.clone(),
            index: sprite.index(0, 0),
        });
        commands.entity(*owner).with_children(|parent| {
            parent.spawn((
                SsdOverlay { owner: *owner },
                component,
                Transform::from_xyz(-0.28, OVERHEAD_Y, OVERHEAD_Z),
            ));
        });
    }

    // --- Полоска здоровья ---
    let Some(texture) = bar_texture else {
        return;
    };
    for (owner, data) in &owners {
        let fraction = healths
            .get(*data)
            .ok()
            .map(|health| {
                (health.current as f32 / health.max.max(1) as f32).clamp(0.0, 1.0)
            });
        let existing = bars.iter().find(|(_, overlay)| overlay.owner == *owner);
        let Some(fraction) = fraction else {
            if let Some((entity, _)) = existing {
                commands.entity(entity).despawn();
            }
            continue;
        };
        // Полностью здоровый — без полоски (раненые показывают её сами).
        if fraction >= 0.999 {
            if let Some((entity, _)) = existing {
                commands.entity(entity).despawn();
            }
            continue;
        }
        let color = Color::srgb(1.0 - fraction, fraction * 0.85, 0.1);
        match existing {
            Some((_, _)) => {
                // Обновление ширины/цвета заполнения у существующей полоски.
                for child in childrens.get(*owner).map(|c| c.to_vec()).unwrap_or_default() {
                    if let Ok((_, mut sprite)) = fills.get_mut(child) {
                        sprite.custom_size =
                            Some(Vec2::new(HEALTH_BAR_WIDTH * fraction, HEALTH_BAR_HEIGHT));
                        sprite.color = color;
                    }
                }
            }
            None => {
                let mut background = Sprite::from_image(texture.0.clone());
                background.custom_size = Some(Vec2::new(HEALTH_BAR_WIDTH, HEALTH_BAR_HEIGHT));
                background.color = Color::srgba(0.0, 0.0, 0.0, 0.55);
                let mut fill = Sprite::from_image(texture.0.clone());
                fill.custom_size = Some(Vec2::new(
                    HEALTH_BAR_WIDTH * fraction,
                    HEALTH_BAR_HEIGHT,
                ));
                fill.color = color;
                commands.entity(*owner).with_children(move |parent| {
                    parent.spawn((
                        HealthBarOverlay { owner: *owner },
                        background,
                        Transform::from_xyz(0.0, OVERHEAD_Y + 0.06, OVERHEAD_Z),
                    ));
                    parent.spawn((
                        HealthBarFill,
                        fill,
                        Transform::from_xyz(0.0, OVERHEAD_Y + 0.06, OVERHEAD_Z + 0.01),
                    ));
                });
            }
        }
    }
}

/// Заполнение полоски здоровья (обновляется по здоровью).
#[derive(Component)]
pub struct HealthBarFill;

/// Половинка кадра обуви (в сборке — шейдер `SpriteFootHalfClip`; у нас —
/// CPU-нарезка кадра RSI на левую/правую половину, кэш по ключу и стороне).
#[allow(clippy::too_many_arguments)]
/// Кэш нарезанных половинок обуви: (ключ RSI, направление, правая?) →
/// (индекс ячейки атласа, половинчатое изображение).
pub type HalvesCache =
    std::collections::HashMap<(String, u32, bool), (usize, Handle<Image>)>;

fn shoe_half(
    registry: &RsiRegistry,
    images: &mut Assets<Image>,
    layouts: &Assets<TextureAtlasLayout>,
    halves: &mut HalvesCache,
    key: &str,
    facing: u32,
    right: bool,
) -> Option<(usize, Handle<Image>)> {
    let cache_key = (key.to_string(), facing.min(3), right);
    if let Some(cached) = halves.get(&cache_key) {
        return Some(cached.clone());
    }
    let sprite = registry.get(key)?;
    let image = images.get(&sprite.image)?.clone();
    let atlas = layouts.get(&sprite.layout)?;
    let index = sprite.index(facing.min(3), 0);
    let rect = *atlas.textures.get(index)?;
    let (iw, ih) = (image.width() as usize, image.height() as usize);
    let data = image.data.as_ref()?;
    let x0 = rect.min.x as usize;
    let x1 = (rect.max.x as usize).min(iw);
    let y0 = rect.min.y as usize;
    let y1 = (rect.max.y as usize).min(ih);
    let mid = (x0 + x1) / 2;
    let (cx0, cx1) = if right { (mid, x1) } else { (x0, mid) };
    if cx1 <= cx0 || y1 <= y0 {
        return None;
    }
    // Новое изображение размером с ВЕСЬ атлас (layout переиспользуется),
    // в кадре заполнена только своя половина — остальное прозрачно.
    let mut out = vec![0u8; data.len()];
    for y in y0..y1 {
        for x in cx0..cx1 {
            let src = ((y * iw) + x) * 4;
            let dst = src; // координаты те же — атлас один в один
            out[dst..dst + 4].copy_from_slice(&data[src..src + 4]);
        }
    }
    let handle = images.add(Image::new(
        bevy::render::render_resource::Extent3d {
            width: iw as u32,
            height: ih as u32,
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        out,
        bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    ));
    let entry = (index, handle.clone());
    halves.insert(cache_key, entry);
    Some((index, handle))
}
