//! Двери на клиенте (PLAN.md T3.1): визуал по реплицированному состоянию
//! + отправка Interact по клику мышью.

use bevy::prelude::*;
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
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

/// Визуальный спрайт двери, привязанный к реплицированной сущности.
/// Публичный: параметр систем Bevy требует публичного типа (E0446).
#[derive(Component)]
pub struct DoorVisual {
    door: Entity,
    last_open: bool,
}

fn color_for(open: bool) -> Color {
    if open {
        // Открытая — светлая (просвет).
        Color::srgb(0.35, 0.75, 0.35)
    } else {
        // Закрытая — тёмно-рыжая (как дверное полотно).
        Color::srgb(0.55, 0.27, 0.07)
    }
}

/// Спавнит спрайт при появлении реплицированной двери.
pub fn spawn_door_visuals(mut commands: Commands, doors: Query<(Entity, &Door), Added<Door>>) {
    for (entity, door) in doors.iter() {
        commands.spawn((
            DoorVisual {
                door: entity,
                last_open: door.open,
            },
            Sprite::from_color(color_for(door.open), Vec2::splat(DOOR_HALF * 2.0)),
            // Поверх тайлов (z=0), но под игроком (z=1).
            Transform::from_xyz(door.position[0], door.position[1], 0.5),
        ));
        tracing::info!(door = ?entity, open = door.open, "door spawned");
    }
}

/// Обновляет цвет по реплицированному состоянию и убирает осиротевшие спрайты.
pub fn update_door_visuals(
    mut commands: Commands,
    doors: Query<&Door>,
    mut visuals: Query<(Entity, &mut DoorVisual, &mut Sprite)>,
) {
    for (visual_entity, mut visual, mut sprite) in visuals.iter_mut() {
        let Ok(door) = doors.get(visual.door) else {
            // Дверь исчезла (despawn с сервера).
            commands.entity(visual_entity).despawn();
            continue;
        };
        if door.open != visual.last_open {
            visual.last_open = door.open;
            let color = color_for(door.open);
            if sprite.color != color {
                sprite.color = color;
            }
            tracing::info!(door = ?visual.door, open = door.open, "door state changed");
        }
    }
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
