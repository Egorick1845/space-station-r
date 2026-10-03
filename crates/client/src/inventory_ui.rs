//! Инвентарь на клиенте (PLAN.md T3.2): визуалы других игроков, панель слотов
//! 7×4, перенос предметов (клик — взять, клик по слоту/игроку — положить).

use bevy::prelude::*;
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::PlayerPosition;
use ssr_core::inventory::{INVENTORY_COLS, INVENTORY_ROWS, Inventory, SLOT_ANY};
use ssr_protocol::ClientMessage;
use ssr_protocol::net::GameChannel;

use crate::PlayerEntity;
use crate::rsi::RsiRegistry;

/// Иконка предмета (до системы прототипов в игре — лом из сборки).
const ITEM_ICON: &str = "sprites/ss14/Objects/Tools/crowbar.rsi#icon";
/// Спрайт других игроков (тот же, что у себя; свой — отдельный визуал).
const REMOTE_SPRITE: &str = "sprites/ss14/Mobs/Animals/monkey.rsi#monkey";

/// Взят (`held`) предмет: клик — подобрать, следующий клик — положить.
#[derive(Resource, Default)]
pub struct HeldItem(pub Option<u64>);

/// Клиентская сущность своего игрока (резолвится раз в кадр из Welcome+маппинга).
#[derive(Resource, Default)]
pub struct OwnPlayerEntity(pub Option<Entity>);

/// Резолв своей сущности: bits из Welcome → ServerEntityMap → клиент.
pub fn resolve_own_player(
    player_entity: Res<PlayerEntity>,
    entity_map: Option<Res<ServerEntityMap>>,
    mut own: ResMut<OwnPlayerEntity>,
) {
    let resolved = crate::own_player_entity(&player_entity, &entity_map);
    if own.0 != resolved {
        tracing::info!(own = ?resolved, "own player entity resolved");
        own.0 = resolved;
    }
}

/// Состояние последней отрисовки панели (слоты + взятый предмет).
type LastRendered = Option<(Vec<Option<u64>>, Option<u64>)>;
/// Новые реплицированные позиции игроков.
type AddedPositions<'w, 's> =
    Query<'w, 's, (Entity, &'static PlayerPosition), (With<Remote>, Added<PlayerPosition>)>;
/// Клики по слотам инвентаря.
type SlotClicks<'w, 's> =
    Query<'w, 's, (&'static Interaction, &'static InvSlot), (Changed<Interaction>, With<Button>)>;

/// Визуал другого игрока, привязанный к реплицированной сущности.
#[derive(Component)]
pub struct RemotePlayerVisual {
    player: Entity,
}

/// Слот панели инвентаря.
#[derive(Component, Clone, Copy)]
pub struct InvSlot(pub u8);

/// Корень панели инвентаря (пересобирается при изменениях).
#[derive(Component)]
pub struct InventoryPanel;

/// Время до авто-переноса в тестовом режиме SSR_INV_TEST.
const INV_TEST_DELAY: f32 = 5.0;

/// Спавнит спрайты других игроков: без этого их не было видно вообще.
pub fn spawn_remote_players(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    player_entity: Res<PlayerEntity>,
    entity_map: Option<Res<ServerEntityMap>>,
    added: AddedPositions,
) {
    let own = crate::own_player_entity(&player_entity, &entity_map);
    let Some(sprite) = registry.get(REMOTE_SPRITE) else {
        return;
    };
    for (entity, position) in added.iter() {
        if Some(entity) == own {
            continue;
        }
        let mut sprite_component = Sprite::from_image(sprite.image.clone());
        sprite_component.texture_atlas = Some(TextureAtlas {
            layout: sprite.layout.clone(),
            index: sprite.index(0, 0),
        });
        commands.spawn((
            RemotePlayerVisual { player: entity },
            sprite_component,
            Transform::from_xyz(position.0[0], position.0[1], 0.9),
        ));
        tracing::info!(player = ?entity, "remote player visual spawned");
    }
}

/// Двигает визуалы других игроков за реплицированными позициями и убирает
/// «осиротевшие» (игрок вышел из интереса).
pub fn sync_remote_players(
    mut commands: Commands,
    positions: Query<&PlayerPosition>,
    mut visuals: Query<(Entity, &RemotePlayerVisual, &mut Transform)>,
) {
    for (visual_entity, visual, mut transform) in visuals.iter_mut() {
        let Ok(position) = positions.get(visual.player) else {
            commands.entity(visual_entity).despawn();
            continue;
        };
        transform.translation.x = position.0[0];
        transform.translation.y = position.0[1];
    }
}

/// Свой инвентарь на клиенте (сущность локального игрока).
fn own_inventory<'a>(
    own: &OwnPlayerEntity,
    inventories: &'a Query<&Inventory>,
) -> Option<&'a Inventory> {
    inventories.get(own.0?).ok()
}

/// Пересобирает панель инвентаря, когда меняется содержимое или «взятый» предмет.
pub fn render_inventory_panel(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    held: Res<HeldItem>,
    root: Query<Entity, With<InventoryPanel>>,
    mut last: Local<LastRendered>,
) {
    let Some(inventory) = own_inventory(&own, &inventories) else {
        return;
    };
    let state = (inventory.slots.clone(), held.0);
    if last.as_ref() == Some(&state) {
        return;
    }
    *last = Some(state);
    tracing::info!(
        items = inventory.slots.iter().filter(|s| s.is_some()).count(),
        "inventory updated"
    );

    for entity in root.iter() {
        commands.entity(entity).despawn();
    }

    let icon = registry.get(ITEM_ICON).map(|rsi| rsi.image.clone());
    let held_value = held.0;

    commands
        .spawn((
            InventoryPanel,
            Node {
                position_type: PositionType::Absolute,
                left: px(12),
                bottom: px(12),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(8)),
                row_gap: px(6),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.72)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Инвентарь"),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.88, 0.88, 0.90)),
            ));
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(3),
                    ..default()
                })
                .with_children(|grid| {
                    for row in 0..INVENTORY_ROWS {
                        grid.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: px(3),
                            ..default()
                        })
                        .with_children(|line| {
                            for col in 0..INVENTORY_COLS {
                                let index = (row * INVENTORY_COLS + col) as usize;
                                let item = inventory.slots.get(index).copied().flatten();
                                let is_held = item.is_some() && item == held_value;
                                let mut slot = line.spawn((
                                    InvSlot(index as u8),
                                    Button,
                                    Node {
                                        width: px(40),
                                        height: px(40),
                                        border: UiRect::all(px(2)),
                                        align_items: AlignItems::Center,
                                        justify_content: JustifyContent::Center,
                                        ..default()
                                    },
                                    BackgroundColor(if is_held {
                                        Color::srgb(0.35, 0.28, 0.10)
                                    } else {
                                        Color::srgb(0.12, 0.12, 0.15)
                                    }),
                                    BorderColor::from(if is_held {
                                        Color::srgb(1.0, 0.75, 0.25)
                                    } else {
                                        Color::srgb(0.28, 0.28, 0.33)
                                    }),
                                ));
                                if item.is_some()
                                    && let Some(icon) = &icon
                                {
                                    slot.with_child((
                                        ImageNode::new(icon.clone()),
                                        Node {
                                            width: px(30),
                                            height: px(30),
                                            ..default()
                                        },
                                    ));
                                }
                            }
                        });
                    }
                });
            panel.spawn((
                Text::new(if held_value.is_some() {
                    "Взят предмет: клик по слоту или игроку — положить"
                } else {
                    "Клик по предмету — взять; в игре: клик по игроку — передать"
                }),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgb(0.62, 0.62, 0.66)),
            ));
        });
}

/// Клик по слоту: взять предмет или положить «взятый» в выбранный слот.
pub fn inventory_slot_click(
    slots: SlotClicks,
    mut held: ResMut<HeldItem>,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    for (interaction, slot) in slots.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(inventory) = own_inventory(&own, &inventories) else {
            continue;
        };
        let item = inventory.slots.get(slot.0 as usize).copied().flatten();
        match (held.0, item) {
            // Ничего не держим, в слоте предмет — берём.
            (None, Some(item)) => {
                held.0 = Some(item);
                tracing::info!(item, slot = slot.0, "inventory: picked up");
            }
            // Держим, клик по пустому слоту — кладём.
            (Some(held_item), None) => {
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::TransferItem {
                        item: held_item,
                        to_slot: slot.0,
                        target_player: 0,
                    });
                }
                held.0 = None;
                tracing::info!(item = held_item, slot = slot.0, "inventory: placed");
            }
            // Держим, клик по занятому слоту — меняем «взятoе» на этот предмет.
            (Some(_), Some(item)) => {
                held.0 = Some(item);
            }
            (None, None) => {}
        }
    }
}

/// Клик по другому игроку со «взятым» предметом — передать ему (T3.2).
pub fn inventory_click_transfer(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    mut held: ResMut<HeldItem>,
    visuals: Query<(&RemotePlayerVisual, &Transform)>,
    entity_map: Option<Res<ServerEntityMap>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    let Some(item) = held.0 else {
        return;
    };
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

    // Ближайший другой игрок в радиусе клика (полтайла — с запасом на спрайт).
    let mut best: Option<(Entity, f32)> = None;
    for (visual, transform) in visuals.iter() {
        let distance = transform.translation.truncate().distance(world);
        if distance <= 32.0 && best.is_none_or(|(_, d)| distance < d) {
            best = Some((visual.player, distance));
        }
    }
    let Some((target_entity, _)) = best else {
        return;
    };

    let Some(map) = entity_map.as_deref() else {
        return;
    };
    let Some(server_entity) = map.to_server().get(&target_entity) else {
        tracing::warn!(?target_entity, "transfer: no server mapping");
        return;
    };
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::TransferItem {
            item,
            to_slot: SLOT_ANY,
            target_player: server_entity.to_bits(),
        });
    }
    held.0 = None;
    tracing::info!(item, to = ?target_entity, "inventory: передано игроку");
}

/// Тестовый режим SSR_INV_TEST=1: через [`INV_TEST_DELAY`] секунд после
/// получения инвентаря клиент передаёт первый предмет ближайшему игроку —
/// так критерий T3.2 проверяется без ручного перетаскивания.
pub fn inventory_test_mode(
    time: Res<Time>,
    own: Res<OwnPlayerEntity>,
    entity_map: Option<Res<ServerEntityMap>>,
    inventories: Query<&Inventory>,
    visuals: Query<&RemotePlayerVisual>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut state: Local<(f32, bool)>,
) {
    if std::env::var_os("SSR_INV_TEST").is_none() || state.1 {
        return;
    }
    state.0 += time.delta_secs();
    if state.0 < INV_TEST_DELAY {
        return;
    }
    let Some(inventory) = own_inventory(&own, &inventories) else {
        tracing::warn!(own = ?own.0, "inv-test: own inventory not resolved");
        return;
    };
    let Some(item) = inventory.slots.iter().flatten().copied().next() else {
        tracing::warn!(?inventory.slots, "inv-test: own inventory is empty");
        state.1 = true;
        return;
    };
    let Some(visual) = visuals.iter().next() else {
        tracing::warn!("inv-test: no remote player visuals");
        return;
    };
    let Some(map) = entity_map.as_deref() else {
        return;
    };
    let Some(server_entity) = map.to_server().get(&visual.player) else {
        return;
    };
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::TransferItem {
            item,
            to_slot: SLOT_ANY,
            target_player: server_entity.to_bits(),
        });
    }
    state.1 = true;
    tracing::info!(item, "inv-test: transfer sent");
}
