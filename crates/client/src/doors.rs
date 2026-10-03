//! Двери на клиенте (PLAN.md T3.1): визуал по реплицированному состоянию
//! + отправка Interact по клику мышью.

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

/// RSI-ключи спрайтов двери (Structures/Doors/Airlocks/Standard/basic.rsi).
const DOOR_CLOSED: &str = "sprites/ss14/Structures/Doors/Airlocks/Standard/basic.rsi#closed";
const DOOR_OPEN: &str = "sprites/ss14/Structures/Doors/Airlocks/Standard/basic.rsi#open";
const DOOR_OPENING: &str = "sprites/ss14/Structures/Doors/Airlocks/Standard/basic.rsi#opening";
const DOOR_CLOSING: &str = "sprites/ss14/Structures/Doors/Airlocks/Standard/basic.rsi#closing";

/// Анимация двери: проигрывание opening/closing по delays из RSI.
#[derive(Clone, Copy, PartialEq)]
enum DoorAnim {
    Idle,
    Opening { frame: u32, elapsed: f32 },
    Closing { frame: u32, elapsed: f32 },
}

/// Визуальный спрайт двери, привязанный к реплицированной сущности.
/// Публичный: параметр систем Bevy требует публичного типа (E0446).
#[derive(Component)]
pub struct DoorVisual {
    door: Entity,
    last_open: bool,
    anim: DoorAnim,
}

/// Спавнит спрайт при появлении реплицированной двери.
pub fn spawn_door_visuals(
    mut commands: Commands,
    doors: Query<(Entity, &Door), Added<Door>>,
    registry: Res<RsiRegistry>,
) {
    let (Some(closed), Some(open)) = (registry.get(DOOR_CLOSED), registry.get(DOOR_OPEN)) else {
        for (entity, door) in doors.iter() {
            tracing::warn!(?entity, open = door.open, "door rsi sprites missing");
        }
        return;
    };
    for (entity, door) in doors.iter() {
        let target = if door.open { open } else { closed };
        let mut sprite = Sprite::from_image(target.image.clone());
        sprite.texture_atlas = Some(TextureAtlas {
            layout: target.layout.clone(),
            index: 0,
        });
        commands.spawn((
            DoorVisual {
                door: entity,
                last_open: door.open,
                anim: DoorAnim::Idle,
            },
            sprite,
            // Поверх тайлов (z=0), но под игроком (z=1).
            Transform::from_xyz(door.position[0], door.position[1], 0.5),
        ));
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

/// Меняет спрайт по состоянию двери и проигрывает анимацию opening/closing
/// по `delays` из RSI (полноценная анимация, а не мгновенная смена).
pub fn update_door_visuals(
    time: Res<Time>,
    mut commands: Commands,
    doors: Query<&Door>,
    mut visuals: Query<(Entity, &mut DoorVisual, &mut Sprite)>,
    registry: Res<RsiRegistry>,
) {
    let (Some(closed), Some(open), Some(opening), Some(closing)) = (
        registry.get(DOOR_CLOSED),
        registry.get(DOOR_OPEN),
        registry.get(DOOR_OPENING),
        registry.get(DOOR_CLOSING),
    ) else {
        return;
    };
    let dt = time.delta_secs();

    for (visual_entity, mut visual, mut sprite) in visuals.iter_mut() {
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

        match visual.anim {
            DoorAnim::Idle => {
                let target = if door.open { open } else { closed };
                if sprite.image != target.image {
                    apply_door_state(&mut sprite, target, 0);
                }
            }
            DoorAnim::Opening { frame, elapsed } => {
                advance_anim(
                    &mut visual.anim,
                    &mut sprite,
                    opening,
                    frame,
                    elapsed,
                    dt,
                    true,
                );
            }
            DoorAnim::Closing { frame, elapsed } => {
                advance_anim(
                    &mut visual.anim,
                    &mut sprite,
                    closing,
                    frame,
                    elapsed,
                    dt,
                    false,
                );
            }
        }
    }
}

/// Продвигает кадр анимации по длительностям RSI; по завершении — конечное состояние.
fn advance_anim(
    anim: &mut DoorAnim,
    sprite: &mut Sprite,
    rsi: &crate::rsi::RsiSprite,
    frame: u32,
    elapsed: f32,
    dt: f32,
    to_open: bool,
) {
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
        *anim = DoorAnim::Idle;
    } else {
        *anim = if to_open {
            DoorAnim::Opening { frame, elapsed }
        } else {
            DoorAnim::Closing { frame, elapsed }
        };
    }
    apply_door_state(sprite, rsi, frame.min(frames.saturating_sub(1)));
}

/// Клик левой кнопкой по двери в зоне попадания → `Interact` на сервер.
///
/// Клиентская проверка — только выбор цели (ближайшая дверь к курсору);
/// радиус взаимодействия (1.5 тайла) проверяет сервер.
pub fn click_interact(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    doors: Query<(Entity, &Door)>,
    entity_map: Option<Res<ServerEntityMap>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
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

    let Some(door_entity) = nearest_door(&doors, world) else {
        return;
    };
    send_interact(door_entity, &entity_map, &mut senders);
}

/// Ближайшая дверь к точке клика в пределах [`CLICK_RADIUS`].
fn nearest_door(doors: &Query<(Entity, &Door)>, point: Vec2) -> Option<Entity> {
    doors
        .iter()
        .filter_map(|(entity, door)| {
            let distance = Vec2::from_array(door.position).distance(point);
            (distance <= CLICK_RADIUS).then_some((entity, distance))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(entity, _)| entity)
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
