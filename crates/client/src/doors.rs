//! Двери на клиенте (PLAN.md T3.1): визуал по реплицированному состоянию
//! + отправка Interact по клику мышью.
//!
//! Спрайт двери — два слоя как в SS14: базовый (`closed/open/opening/closing`)
//! и лампа (`*_unlit`): по умолчанию синяя подсветка, зелёная при открытии,
//! красная при отказе доступа (`deny_unlit`).

use bevy::prelude::*;
use bevy_replicon::shared::server_entity_map::ServerEntityMap;

use crate::rsi::RsiRegistry;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::Door;
use ssr_protocol::ClientMessage;
use ssr_protocol::net::GameChannel;

use crate::PlayerEntity;

/// Половинка двери в юнитах (коллайдер/спрайт 32×32).
const DOOR_HALF: f32 = 16.0;

/// Клиентская зона попадания по клику: дверь плюс небольшой запас.
const CLICK_RADIUS: f32 = 24.0;

/// Времени между авто-взаимодействиями в тестовом режиме (SSR_INTERACT_TEST).
const AUTO_INTERACT_PERIOD: f32 = 1.0;

/// Префикс ключей RSI двери (Structures/Doors/Airlocks/Standard/basic.rsi).
const DOOR_BASE: &str = "sprites/ss14/Structures/Doors/Airlocks/Standard/basic.rsi#";
const DOOR_CLOSED: &str = "closed";
const DOOR_OPEN: &str = "open";
const DOOR_OPENING: &str = "opening";
const DOOR_CLOSING: &str = "closing";
const DOOR_CLOSED_LIGHT: &str = "closed_unlit";
const DOOR_OPEN_LIGHT: &str = "open_unlit";
const DOOR_OPENING_LIGHT: &str = "opening_unlit";
const DOOR_CLOSING_LIGHT: &str = "closing_unlit";
const DOOR_DENY_LIGHT: &str = "deny_unlit";

/// Вид проигрываемой анимации (для продвижения кадров).
#[derive(Clone, Copy, PartialEq)]
enum DoorAnimKind {
    Opening,
    Closing,
    Deny,
}

/// Анимация двери: проигрывание opening/closing/deny по delays из RSI.
#[derive(Clone, Copy, PartialEq)]
enum DoorAnim {
    Idle,
    Opening {
        frame: u32,
        elapsed: f32,
    },
    Closing {
        frame: u32,
        elapsed: f32,
    },
    /// Отказ доступа: мигает красная лампа, затем состояние двери.
    Deny {
        frame: u32,
        elapsed: f32,
    },
}

/// Двери, которым сервер отказал в доступе (bits) — клиент показывает
/// красную лампу (T4.2). Заполняется в `receive_server`.
#[derive(Resource, Default)]
pub struct DeniedDoors(pub Vec<u64>);

/// Включает красную лампу на двери, которой сервер отказал в доступе.
pub fn apply_denied_doors(
    mut denied: ResMut<DeniedDoors>,
    entity_map: Option<Res<ServerEntityMap>>,
    mut visuals: Query<&mut DoorVisual>,
) {
    if denied.0.is_empty() {
        return;
    }
    let Some(map) = entity_map.as_deref() else {
        return;
    };
    let requests = std::mem::take(&mut denied.0);
    for bits in requests {
        let Some(server_entity) = Entity::try_from_bits(bits) else {
            continue;
        };
        let Some(client_entity) = map.to_client().get(&server_entity).copied() else {
            continue;
        };
        for mut visual in visuals.iter_mut() {
            if visual.door == client_entity {
                visual.anim = DoorAnim::Deny {
                    frame: 0,
                    elapsed: 0.0,
                };
                tracing::info!(door = ?visual.door, "access denied: red light");
            }
        }
    }
}

/// Визуальный спрайт двери, привязанный к реплицированной сущности.
/// Публичный: параметр систем Bevy требует публичного типа (E0446).
#[derive(Component)]
pub struct DoorVisual {
    door: Entity,
    last_open: bool,
    anim: DoorAnim,
    /// Дочерняя сущность слоя лампы (синяя/зелёная/красная подсветка).
    overlay: Entity,
}

/// Маркер слоя лампы двери.
#[derive(Component)]
pub struct DoorOverlay;

/// Спавнит спрайт при появлении реплицированной двери: базовый слой + слой
/// лампы (`*_unlit`) поверх, как в SS14 (по умолчанию синяя подсветка).
pub fn spawn_door_visuals(
    mut commands: Commands,
    doors: Query<(Entity, &Door), Added<Door>>,
    registry: Res<RsiRegistry>,
) {
    for (entity, door) in doors.iter() {
        let (base_key, light_key) = if door.open {
            (DOOR_OPEN, DOOR_OPEN_LIGHT)
        } else {
            (DOOR_CLOSED, DOOR_CLOSED_LIGHT)
        };
        let (Some(base), Some(light)) = (
            registry.get(&format!("{DOOR_BASE}{base_key}")),
            registry.get(&format!("{DOOR_BASE}{light_key}")),
        ) else {
            tracing::warn!(?entity, "door rsi sprites missing");
            continue;
        };
        let mut base_sprite = Sprite::from_image(base.image.clone());
        base_sprite.texture_atlas = Some(TextureAtlas {
            layout: base.layout.clone(),
            index: 0,
        });
        let mut light_sprite = Sprite::from_image(light.image.clone());
        light_sprite.texture_atlas = Some(TextureAtlas {
            layout: light.layout.clone(),
            index: 0,
        });
        let overlay = commands
            .spawn((
                DoorOverlay,
                light_sprite,
                Transform::from_xyz(0.0, 0.0, 0.01),
            ))
            .id();
        commands
            .spawn((
                DoorVisual {
                    door: entity,
                    last_open: door.open,
                    anim: DoorAnim::Idle,
                    overlay,
                },
                base_sprite,
                // Поверх тайлов (z=0), но под игроком (z=1).
                Transform::from_xyz(door.position[0], door.position[1], 0.5),
            ))
            .add_child(overlay);
        tracing::info!(door = ?entity, open = door.open, "door spawned");
    }
}

/// Применяет к спрайту кадр состояния двери.
fn apply_door_state(sprite: &mut Sprite, rsi: &crate::rsi::RsiSprite, frame: u32) {
    sprite.image = rsi.image.clone();
    sprite.texture_atlas = Some(TextureAtlas {
        layout: rsi.layout.clone(),
        index: rsi.index(0, frame),
    });
}

/// Продвигает кадр анимации по длительностям RSI; по завершении — Idle.
fn advance(anim: DoorAnim, rsi: Option<&crate::rsi::RsiSprite>, dt: f32) -> DoorAnim {
    let (kind, frame, elapsed) = match anim {
        DoorAnim::Opening { frame, elapsed } => (DoorAnimKind::Opening, frame, elapsed),
        DoorAnim::Closing { frame, elapsed } => (DoorAnimKind::Closing, frame, elapsed),
        DoorAnim::Deny { frame, elapsed } => (DoorAnimKind::Deny, frame, elapsed),
        DoorAnim::Idle => return DoorAnim::Idle,
    };
    let Some(rsi) = rsi else {
        return DoorAnim::Idle;
    };
    let frames = rsi.frames_per_direction.first().copied().unwrap_or(1);
    let mut frame = frame;
    let mut elapsed = elapsed + dt;
    // Возможно пройти несколько кадров за один тик — цикл, а не один шаг.
    while frame < frames {
        let delay = rsi
            .delays
            .first()
            .and_then(|d| d.get(frame as usize))
            .copied()
            .unwrap_or(0.1)
            .max(0.001);
        if elapsed < delay {
            break;
        }
        elapsed -= delay;
        frame += 1;
    }
    if frame >= frames {
        return DoorAnim::Idle;
    }
    match kind {
        DoorAnimKind::Opening => DoorAnim::Opening { frame, elapsed },
        DoorAnimKind::Closing => DoorAnim::Closing { frame, elapsed },
        DoorAnimKind::Deny => DoorAnim::Deny { frame, elapsed },
    }
}

/// Обновляет оба слоя двери: базовый спрайт и лампу (`*_unlit`), проигрывая
/// анимацию opening/closing/deny по `delays` из RSI.
pub fn update_door_visuals(
    time: Res<Time>,
    mut commands: Commands,
    doors: Query<&Door>,
    mut visuals: Query<(Entity, &mut DoorVisual)>,
    mut sprites: Query<&mut Sprite>,
    registry: Res<RsiRegistry>,
) {
    let dt = time.delta_secs();

    for (visual_entity, mut visual) in visuals.iter_mut() {
        let Ok(door) = doors.get(visual.door) else {
            // Дверь исчезла (despawn с сервера).
            commands.entity(visual_entity).despawn();
            continue;
        };

        // Сервер сообщил новое состояние — запускаем анимацию перехода.
        if door.open != visual.last_open {
            visual.last_open = door.open;
            visual.anim = if door.open {
                DoorAnim::Opening {
                    frame: 0,
                    elapsed: 0.0,
                }
            } else {
                DoorAnim::Closing {
                    frame: 0,
                    elapsed: 0.0,
                }
            };
            tracing::info!(door = ?visual.door, open = door.open, "door state changed");
        }

        // Продвигаем кадры по delays того состояния, которое играем.
        let anim = match visual.anim {
            DoorAnim::Opening { .. } => advance(
                visual.anim,
                registry.get(&format!("{DOOR_BASE}{DOOR_OPENING}")),
                dt,
            ),
            DoorAnim::Closing { .. } => advance(
                visual.anim,
                registry.get(&format!("{DOOR_BASE}{DOOR_CLOSING}")),
                dt,
            ),
            DoorAnim::Deny { .. } => advance(
                visual.anim,
                registry.get(&format!("{DOOR_BASE}{DOOR_DENY_LIGHT}")),
                dt,
            ),
            DoorAnim::Idle => DoorAnim::Idle,
        };
        visual.anim = anim;

        // Какие спрайты показывать на обоих слоях.
        let (base_key, light_key, animated) = match anim {
            DoorAnim::Idle => {
                if door.open {
                    (DOOR_OPEN, DOOR_OPEN_LIGHT, false)
                } else {
                    (DOOR_CLOSED, DOOR_CLOSED_LIGHT, false)
                }
            }
            DoorAnim::Opening { .. } => (DOOR_OPENING, DOOR_OPENING_LIGHT, true),
            DoorAnim::Closing { .. } => (DOOR_CLOSING, DOOR_CLOSING_LIGHT, true),
            // Отказ: базовый слой остаётся состоянием двери, лампа — красная.
            DoorAnim::Deny { .. } => (
                if door.open { DOOR_OPEN } else { DOOR_CLOSED },
                DOOR_DENY_LIGHT,
                true,
            ),
        };
        let frame = match anim {
            DoorAnim::Opening { frame, .. }
            | DoorAnim::Closing { frame, .. }
            | DoorAnim::Deny { frame, .. } => frame,
            DoorAnim::Idle => 0,
        };
        let Some(base) = registry.get(&format!("{DOOR_BASE}{base_key}")) else {
            continue;
        };
        if let Ok(mut sprite) = sprites.get_mut(visual_entity)
            && (animated || sprite.image != base.image)
        {
            apply_door_state(&mut sprite, base, frame);
        }
        if let Some(light) = registry.get(&format!("{DOOR_BASE}{light_key}"))
            && let Ok(mut sprite) = sprites.get_mut(visual.overlay)
        {
            apply_door_state(&mut sprite, light, frame);
        }
    }
}

/// Отправляет Interact для клиентской сущности двери (маппинг → серверные bits).
pub fn send_interact(
    door_entity: Entity,
    entity_map: &Option<Res<ServerEntityMap>>,
    senders: &mut Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) -> Option<()> {
    let Some(map) = entity_map else {
        tracing::warn!("server entity map unavailable");
        return None;
    };
    let Some(server_entity) = map.to_server().get(&door_entity) else {
        tracing::warn!(?door_entity, "door has no server entity mapping");
        return None;
    };
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Interact {
            entity: server_entity.to_bits(),
        });
    }
    tracing::info!(?door_entity, "Interact sent");
    Some(())
}

/// Обводка объекта под курсором (T3.1; задел под предметы в T3.2):
/// жёлтая рамка вокруг двери, на которую наведён курсор.
pub fn hover_outline(
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    doors: Query<&Door>,
    mut gizmos: Gizmos,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let (camera, camera_transform) = *camera;
    let Ok(world) = camera.viewport_to_world_2d(camera_transform, cursor) else {
        return;
    };
    for door in doors.iter() {
        let position = Vec2::from_array(door.position);
        if position.distance(world) <= CLICK_RADIUS {
            gizmos.rect_2d(
                Isometry2d::from_translation(position),
                Vec2::splat(DOOR_HALF * 2.0 + 4.0),
                Color::srgb(1.0, 0.9, 0.35),
            );
        }
    }
}

/// Тестовый режим SSR_INTERACT_TEST=1: раз в [`AUTO_INTERACT_PERIOD`] секунд
/// клиент «кликает» по ближайшей к своему игроку двери (открыть/закрыть).
pub fn auto_interact(
    time: Res<Time>,
    mut next_interact: Local<f32>,
    player_entity: Res<PlayerEntity>,
    positions: Query<&ssr_core::PlayerPosition>,
    doors: Query<(Entity, &Door)>,
    entity_map: Option<Res<ServerEntityMap>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if std::env::var_os("SSR_INTERACT_TEST").is_none() {
        return;
    }
    *next_interact += time.delta_secs();
    if *next_interact < AUTO_INTERACT_PERIOD {
        return;
    }
    *next_interact = 0.0;

    // Своя позиция: серверные bits из Welcome → клиентская сущность через маппинг.
    let Some(bits) = player_entity.0 else {
        return;
    };
    let Some(map) = entity_map.as_deref() else {
        return;
    };
    let Some(server_entity) = Entity::try_from_bits(bits) else {
        return;
    };
    let Some(client_entity) = map.to_client().get(&server_entity) else {
        return;
    };
    let Ok(position) = positions.get(*client_entity) else {
        return;
    };
    let own = Vec2::from_array(position.0);
    let Some((door_entity, _)) = doors
        .iter()
        .filter_map(|(entity, door)| {
            let distance = Vec2::from_array(door.position).distance(own);
            (distance <= ssr_core::INTERACT_RANGE + DOOR_HALF).then_some((entity, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        tracing::debug!(?own, "auto-interact: no door in range");
        return;
    };
    send_interact(door_entity, &entity_map, &mut senders);
}
