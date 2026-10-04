//! Предметы на полу (механики владельца): пока предмет ничей (`HeldBy.player == 0`)
//! и лежит в мире (`ItemPosition`), клиент рисует его иконку в мировой позиции —
//! чтобы брошенный по Q предмет было видно и можно было поднять его по E.

use bevy::prelude::*;
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::inventory::{HeldBy, Item, ItemPosition};
use ssr_protocol::ClientMessage;
use ssr_protocol::net::GameChannel;

use crate::content::ClientContent;
use crate::inventory_ui::item_icon;
use crate::rsi::RsiRegistry;

/// Иконка предмета, лежащего на полу (живёт на самом реплицируемом предмете).
#[derive(Component)]
pub struct FloorItemIcon;

/// Слой отрисовки: над тайлами и кабелями, под дверями (0.5) и игроком (1.0).
const FLOOR_ITEM_Z: f32 = 0.45;

/// Синхронизирует иконки: предмет с мировой позицией без владельца показывается,
/// поднятый или убранный в рюкзак — скрывается.
pub fn sync_floor_item_icons(
    mut commands: Commands,
    items: Query<(Entity, &Item, Option<&ItemPosition>, &HeldBy)>,
    mut icons: Query<(&mut Transform, &mut Visibility), With<FloorItemIcon>>,
    registry: Res<RsiRegistry>,
    content: Res<ClientContent>,
) {
    for (entity, item, position, held) in items.iter() {
        let on_floor = position.filter(|_| held.player == 0);
        match (on_floor, icons.get_mut(entity)) {
            (Some(position), Ok((mut transform, mut visibility))) => {
                transform.translation.x = position.0[0];
                transform.translation.y = position.0[1];
                if *visibility != Visibility::Inherited {
                    *visibility = Visibility::Inherited;
                }
            }
            (Some(position), Err(_)) => {
                let Some(sprite) = item_icon(&registry, &content.items, &item.name) else {
                    continue;
                };
                commands.entity(entity).insert((
                    Sprite {
                        image: sprite.image.clone(),
                        texture_atlas: Some(TextureAtlas {
                            layout: sprite.layout.clone(),
                            index: sprite.index(0, 0),
                        }),
                        ..default()
                    },
                    Transform::from_xyz(position.0[0], position.0[1], FLOOR_ITEM_Z),
                    Visibility::Inherited,
                    FloorItemIcon,
                ));
            }
            (None, Ok((_, mut visibility))) => *visibility = Visibility::Hidden,
            (None, Err(_)) => {}
        }
    }
}

/// Ближайший к точке предмет, лежащий на полу (ничей, с мировой позицией).
pub fn nearest_floor_item(
    world: Vec2,
    radius: f32,
    items: &Query<(Entity, &Item, &ItemPosition, &HeldBy)>,
) -> Option<Entity> {
    let mut nearest: Option<(f32, Entity)> = None;
    for (entity, _, position, held) in items.iter() {
        if held.player != 0 {
            continue;
        }
        let distance = Vec2::from_array(position.0).distance(world);
        if distance <= radius && nearest.is_none_or(|(best, _)| distance < best) {
            nearest = Some((distance, entity));
        }
    }
    nearest.map(|(_, entity)| entity)
}

/// Копирование сущности под курсором по P (как в SS14: взять сущность и
/// поставить её ЛКМ). Предмет на полу отдаёт имя своего прототипа в режим
/// размещения — дальше работает обычный поток спавн-меню: призрак висит на
/// курсоре, ЛКМ ставит копию, ПКМ отменяет.
pub fn copy_entity_key(
    keys: Res<ButtonInput<KeyCode>>,
    chat: Res<crate::chat::ChatState>,
    console: Res<crate::console::Console>,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    items: Query<(Entity, &Item, &ItemPosition, &HeldBy)>,
    mut placement: ResMut<crate::hud::Placement>,
) {
    if !keys.just_pressed(KeyCode::KeyP) {
        return;
    }
    // Набор текста в чате/консоли приоритетнее: P там — буква.
    if chat.focused || console.open {
        return;
    }
    let Some(world) = crate::hud::cursor_world(&windows, &camera) else {
        return;
    };
    let Some(entity) = nearest_floor_item(world, 24.0, &items) else {
        tracing::info!("copy: под курсором нет сущности");
        return;
    };
    let Ok((_, item, _, _)) = items.get(entity) else {
        return;
    };
    placement.item = Some(item.name.clone());
    tracing::info!(item = %item.name, "copy: режим размещения копии");
}

/// Подбор предмета с пола кликом мыши (как в SS14): ЛКМ по предмету под
/// курсором. Клики по интерфейсу (кнопки, поля) мир не трогают.
pub fn floor_item_click(
    mouse: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    entity_map: Option<Res<ServerEntityMap>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    items: Query<(Entity, &Item, &ItemPosition, &HeldBy)>,
    placement: Res<crate::hud::Placement>,
    ui: Query<&Interaction, With<Button>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    // В режиме размещения (в т.ч. копии по P) ЛКМ ставит предмет, а не подбирает.
    if placement.item.is_some() {
        return;
    }
    // Курсор над интерфейсом — клик принадлежит UI, а не миру.
    if ui
        .iter()
        .any(|interaction| *interaction != Interaction::None)
    {
        return;
    }
    let Some(world) = crate::hud::cursor_world(&windows, &camera) else {
        return;
    };
    let Some(item) = nearest_floor_item(world, 24.0, &items) else {
        return;
    };
    let Some(map) = entity_map.as_deref() else {
        return;
    };
    let Some(server_entity) = map.to_server().get(&item) else {
        return;
    };
    let bits = server_entity.to_bits();
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Pickup { item: bits });
    }
    tracing::info!(item = bits, "pickup clicked");
}

/// Тест-режим SSR_PICKUP_TEST=1: клиент кладёт предмет на пол админ-командой
/// (как спавн-меню), открывает спавн-меню для скриншота и поднимает предмет
/// тем же кодом, что и клик мышью. Критерий: выброс/подбор в логах сервера.
#[allow(clippy::too_many_arguments)]
pub fn pickup_test_mode(
    time: Res<Time>,
    mut state: Local<(f32, u8)>,
    mut state_hud: ResMut<crate::hud::HudState>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    entity_map: Option<Res<ServerEntityMap>>,
    items: Query<(Entity, &Item, &ItemPosition, &HeldBy)>,
    own: Res<crate::inventory_ui::OwnPlayerEntity>,
    positions: Query<&ssr_core::PlayerPosition>,
) {
    if std::env::var_os("SSR_PICKUP_TEST").is_none() {
        return;
    }
    state.0 += time.delta_secs();
    let elapsed = state.0;
    if state.1 == 0 && elapsed >= 2.0 {
        state.1 = 1;
        state_hud.spawn_open = true;
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Admin {
                command: "spawn Crowbar 1 floor".to_string(),
            });
        }
        tracing::info!("pickup-test: spawn on floor sent, spawn menu opened");
    }
    if state.1 == 1 && elapsed >= 7.0 {
        state.1 = 2;
        let Some(map) = entity_map.as_deref() else {
            return;
        };
        let floor: Vec<(Entity, String, u64, [f32; 2])> = items
            .iter()
            .map(|(entity, item, position, held)| {
                (entity, item.name.clone(), held.player, position.0)
            })
            .collect();
        tracing::info!(count = floor.len(), items = ?floor, "pickup-test: items on client");
        // Берём ближайший к игроку предмет на полу — как клик мышью по цели.
        let own_position = own
            .0
            .and_then(|entity| positions.get(entity).ok())
            .map(|position| Vec2::from_array(position.0));
        let Some(item) = floor
            .iter()
            .filter(|(_, _, held, _)| *held == 0)
            .min_by(|a, b| {
                let da = own_position.map_or(a.3[0].abs() + a.3[1].abs(), |own| {
                    Vec2::from_array(a.3).distance(own)
                });
                let db = own_position.map_or(b.3[0].abs() + b.3[1].abs(), |own| {
                    Vec2::from_array(b.3).distance(own)
                });
                da.total_cmp(&db)
            })
            .map(|(entity, _, _, _)| *entity)
        else {
            tracing::warn!("pickup-test: no floor item found");
            return;
        };
        let Some(server_entity) = map.to_server().get(&item) else {
            tracing::warn!("pickup-test: no server mapping");
            return;
        };
        let bits = server_entity.to_bits();
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Pickup { item: bits });
        }
        tracing::info!(item = bits, "pickup-test: pickup sent");
    }
}

/// Тест-режим SSR_COPY_TEST=1: проверяет копирование сущностей (P + ЛКМ) без
/// ручного ввода — кладёт предмет на пол админ-командой, берёт его имя в режим
/// размещения тем же способом, что и клавиша P, и ставит копию размещением.
/// Критерий: в логе сервера два «размещено на полу … Crowbar».
pub fn copy_test_mode(
    time: Res<Time>,
    mut state: Local<(f32, u8)>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    items: Query<(Entity, &Item, &ItemPosition, &HeldBy)>,
    own: Res<crate::inventory_ui::OwnPlayerEntity>,
    positions: Query<&ssr_core::PlayerPosition>,
    mut placement: ResMut<crate::hud::Placement>,
) {
    if std::env::var_os("SSR_COPY_TEST").is_none() {
        return;
    }
    state.0 += time.delta_secs();
    let elapsed = state.0;
    if state.1 == 0 && elapsed >= 2.0 {
        state.1 = 1;
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Admin {
                command: "spawn Crowbar 1 floor".to_string(),
            });
        }
        tracing::info!("copy-test: предмет на полу запрошен");
    }
    if state.1 == 1 && elapsed >= 4.0 {
        state.1 = 2;
        // Тот же выбор цели, что и в `copy_entity_key` (ближайший предмет на полу).
        let Some(own_position) = own
            .0
            .and_then(|entity| positions.get(entity).ok())
            .map(|position| Vec2::from_array(position.0))
        else {
            tracing::warn!("copy-test: нет позиции игрока");
            return;
        };
        let Some(entity) = nearest_floor_item(own_position, 64.0, &items) else {
            tracing::warn!("copy-test: предмет на полу не найден");
            return;
        };
        let Ok((_, item, _, _)) = items.get(entity) else {
            return;
        };
        placement.item = Some(item.name.clone());
        tracing::info!(item = %item.name, "copy-test: копия взята в режим размещения");
    }
    if state.1 == 2 && elapsed >= 6.0 {
        state.1 = 3;
        let Some(item) = placement.item.clone() else {
            tracing::warn!("copy-test: режим размещения пуст");
            return;
        };
        let Some(own_position) = own
            .0
            .and_then(|entity| positions.get(entity).ok())
            .map(|position| Vec2::from_array(position.0))
        else {
            return;
        };
        // Размещение копии рядом с игроком (как ЛКМ по этому месту).
        let command = format!(
            "spawn {item} 1 floor {:.1} {:.1}",
            own_position.x + 32.0,
            own_position.y - 32.0
        );
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Admin {
                command: command.clone(),
            });
        }
        placement.item = None;
        tracing::info!(command, "copy-test: копия размещена");
    }
}
