//! Контейнеры на клиенте (PLAN.md T3.4): спрайты ящиков (closed/open),
//! панель открытого ящика рядом с игроком, клик по ящику — открыть/закрыть.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::PlayerPosition;
use ssr_core::inventory::{Container, Hands, Inventory, ItemPosition, SLOT_ANY};
use ssr_protocol::ClientMessage;
use ssr_protocol::net::GameChannel;

use crate::rsi::RsiRegistry;

/// RSI ящика из сборки: состояния closed/open.
const CRATE_CLOSED: &str = "sprites/ss14/Structures/Storage/Crates/generic.rsi#closed";
const CRATE_OPEN: &str = "sprites/ss14/Structures/Storage/Crates/generic.rsi#open";
/// Иконка предмета в слотах (до системы прототипов — лом).
const ITEM_ICON: &str = "sprites/ss14/Objects/Tools/crowbar.rsi#icon";
/// Радиус, в котором открытый ящик показывается на экране.
const CONTAINER_UI_RANGE: f32 = 96.0;

/// Спрайт ящика, привязанный к реплицированной сущности.
#[derive(Component)]
pub struct CrateVisual {
    container: Entity,
}

/// Корень панели открытого ящика.
#[derive(Component)]
pub struct ContainerPanel;

/// Слот в панели ящика.
#[derive(Component, Clone, Copy)]
pub struct ContainerSlot {
    container: Entity,
    slot: u8,
}

/// Состояние панели ящика для сравнения при перерисовке.
type PanelSignature = Option<(Option<Entity>, Vec<Option<u64>>)>;
/// Клик по слоту ящика.
type ContainerSlotClicks<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static ContainerSlot),
    (Changed<Interaction>, With<Button>),
>;
/// Контейнеры мира (сущность, состояние, позиция).
type WorldContainers<'w, 's> = Query<'w, 's, (Entity, &'static Container, &'static ItemPosition)>;

/// Спавнит спрайты ящиков (закрытый/открытый состояния).
pub fn spawn_crate_visuals(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    added: Query<(Entity, &Container, &ItemPosition), Added<Container>>,
) {
    let (Some(closed), Some(open)) = (registry.get(CRATE_CLOSED), registry.get(CRATE_OPEN)) else {
        tracing::warn!("crate rsi sprites missing");
        return;
    };
    for (entity, container, position) in added.iter() {
        let target = if container.open { open } else { closed };
        let mut sprite = Sprite::from_image(target.image.clone());
        sprite.texture_atlas = Some(TextureAtlas {
            layout: target.layout.clone(),
            index: 0,
        });
        commands.spawn((
            CrateVisual { container: entity },
            sprite,
            Transform::from_xyz(position.0[0], position.0[1], 0.7),
        ));
        tracing::info!(container = ?entity, open = container.open, "crate visual spawned");
    }
}

/// Меняет спрайт при открытии/закрытии и убирает осиротевшие визуалы.
pub fn update_crate_visuals(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    containers: Query<&Container>,
    mut visuals: Query<(Entity, &mut CrateVisual, &mut Sprite)>,
) {
    let (Some(closed), Some(open)) = (registry.get(CRATE_CLOSED), registry.get(CRATE_OPEN)) else {
        return;
    };
    for (visual_entity, visual, mut sprite) in visuals.iter_mut() {
        let Ok(container) = containers.get(visual.container) else {
            commands.entity(visual_entity).despawn();
            continue;
        };
        let target = if container.open { open } else { closed };
        if sprite.image != target.image {
            sprite.image = target.image.clone();
            sprite.texture_atlas = Some(TextureAtlas {
                layout: target.layout.clone(),
                index: 0,
            });
            tracing::info!(container = ?visual.container, open = container.open, "crate state changed");
        }
    }
}

/// Ближайший открытый ящик в радиусе — его панель и показываем.
fn open_container_in_reach<'a>(
    own_position: Vec2,
    containers: &'a Query<(Entity, &Container, &ItemPosition)>,
) -> Option<(Entity, &'a Container)> {
    let mut best: Option<(Entity, &Container, f32)> = None;
    for (entity, container, position) in containers.iter() {
        if !container.open {
            continue;
        }
        let distance = Vec2::from_array(position.0).distance(own_position);
        if distance <= CONTAINER_UI_RANGE && best.is_none_or(|(_, _, d)| distance < d) {
            best = Some((entity, container, distance));
        }
    }
    best.map(|(entity, container, _)| (entity, container))
}

/// Рисует панель открытого ящика (слоты и содержимое).
#[allow(clippy::too_many_arguments)]
pub fn render_container_panel(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    own: Res<crate::inventory_ui::OwnPlayerEntity>,
    positions: Query<&PlayerPosition>,
    containers: WorldContainers,
    inventories: Query<&Inventory>,
    root: Query<Entity, With<ContainerPanel>>,
    mut last: Local<PanelSignature>,
) {
    let own_position = own
        .0
        .and_then(|entity| positions.get(entity).ok())
        .map(|p| Vec2::from_array(p.0));
    let open = own_position.and_then(|position| open_container_in_reach(position, &containers));
    let (container_entity, slots) = match open {
        Some((entity, _)) => {
            let slots = inventories
                .get(entity)
                .map(|inv| inv.slots.clone())
                .unwrap_or_default();
            (Some(entity), slots)
        }
        None => (None, Vec::new()),
    };
    let signature = (container_entity, slots.clone());
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);

    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    let Some(container_entity) = container_entity else {
        return;
    };
    let title = containers
        .get(container_entity)
        .map(|(_, container, _)| container.name.clone())
        .unwrap_or_else(|_| "Ящик".to_string());
    let icon = registry.get(ITEM_ICON).map(|rsi| rsi.image.clone());
    tracing::info!(?container_entity, "container panel opened");

    commands
        .spawn((
            ContainerPanel,
            Node {
                position_type: PositionType::Absolute,
                right: px(12),
                bottom: px(12),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(8)),
                row_gap: px(6),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.78)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new(title),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(1.0, 0.62, 0.15)),
            ));
            panel
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(3),
                    ..default()
                })
                .with_children(|grid| {
                    for row in 0..ssr_core::inventory::INVENTORY_ROWS {
                        grid.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: px(3),
                            ..default()
                        })
                        .with_children(|line| {
                            for col in 0..ssr_core::inventory::INVENTORY_COLS {
                                let index =
                                    (row * ssr_core::inventory::INVENTORY_COLS + col) as usize;
                                let item = slots.get(index).copied().flatten();
                                let mut slot = line.spawn((
                                    ContainerSlot {
                                        container: container_entity,
                                        slot: index as u8,
                                    },
                                    Button,
                                    Node {
                                        width: px(34),
                                        height: px(34),
                                        border: UiRect::all(px(2)),
                                        align_items: AlignItems::Center,
                                        justify_content: JustifyContent::Center,
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.12, 0.12, 0.15)),
                                    BorderColor::from(Color::srgb(0.28, 0.28, 0.33)),
                                ));
                                if item.is_some()
                                    && let Some(icon) = &icon
                                {
                                    slot.with_child((
                                        ImageNode::new(icon.clone()),
                                        Node {
                                            width: px(26),
                                            height: px(26),
                                            ..default()
                                        },
                                    ));
                                }
                            }
                        });
                    }
                });
            panel.spawn((
                Text::new("Клик по предмету — взять; ПКМ — действия"),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgb(0.62, 0.62, 0.66)),
            ));
        });
}

/// Клик по слоту ящика: взять предмет себе или положить из руки.
pub fn container_slot_click(
    slots: ContainerSlotClicks,
    containers: Query<&Container>,
    inventories: Query<&Inventory>,
    hands: Query<&Hands>,
    own: Res<crate::inventory_ui::OwnPlayerEntity>,
    entity_map: Option<Res<ServerEntityMap>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    for (interaction, slot) in slots.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Закрытый ящик недоступен (критерий T3.4).
        if containers
            .get(slot.container)
            .map(|c| !c.open)
            .unwrap_or(true)
        {
            tracing::warn!(?slot.container, "container is closed");
            continue;
        }
        let item = inventories
            .get(slot.container)
            .ok()
            .and_then(|inv| inv.slots.get(slot.slot as usize).copied().flatten());
        match item {
            Some(item) => {
                // Взять из ящика себе в рюкзак.
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::TransferItem {
                        item,
                        to_slot: SLOT_ANY,
                        target_player: 0,
                    });
                }
                tracing::info!(item, "take from container sent");
            }
            None => {
                // Положить предмет из активной руки в этот слот ящика.
                let hand_item = own
                    .0
                    .and_then(|entity| hands.get(entity).ok())
                    .and_then(|h| h.active_item());
                let Some(item) = hand_item else {
                    continue;
                };
                let Some(map) = entity_map.as_deref() else {
                    continue;
                };
                let Some(server_entity) = map.to_server().get(&slot.container) else {
                    continue;
                };
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::TransferItem {
                        item,
                        to_slot: slot.slot,
                        target_player: server_entity.to_bits(),
                    });
                }
                tracing::info!(item, slot = slot.slot, "put into container sent");
            }
        }
    }
}

/// Тестовый режим SSR_CONTAINER_TEST=1 (критерий T3.4): открывает ближайший
/// ящик, забирает предмет, закрывает и пробует забрать ещё раз (должен быть отказ).
pub fn container_test_mode(
    time: Res<Time>,
    params: ContainerTestParams,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut state: Local<ContainerTestState>,
) {
    if std::env::var_os("SSR_CONTAINER_TEST").is_none() {
        return;
    }
    state.elapsed += time.delta_secs();
    let ContainerTestParams {
        positions,
        containers,
        inventories,
        own,
        entity_map,
    } = params;
    let Some(own_entity) = own.0 else {
        return;
    };
    let Ok(position) = positions.get(own_entity) else {
        return;
    };
    let own_position = Vec2::from_array(position.0);
    let Some((container_entity, _, _)) = containers.iter().min_by(|a, b| {
        let da = Vec2::from_array(a.2.0).distance(own_position);
        let db = Vec2::from_array(b.2.0).distance(own_position);
        da.total_cmp(&db)
    }) else {
        return;
    };
    let Some(map) = entity_map.as_deref() else {
        return;
    };
    let Some(server_entity) = map.to_server().get(&container_entity) else {
        return;
    };
    let bits = server_entity.to_bits();

    if !state.opened && state.elapsed >= 5.0 {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Interact { entity: bits });
        }
        state.opened = true;
        tracing::info!("container-test: open sent");
    }
    if state.opened
        && !state.taken
        && state.elapsed >= 8.0
        && let Some(item) = inventories
            .get(container_entity)
            .ok()
            .and_then(|inv| inv.slots.iter().flatten().copied().next())
    {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TransferItem {
                item,
                to_slot: SLOT_ANY,
                target_player: 0,
            });
        }
        state.taken = true;
        tracing::info!(item, "container-test: take sent (container open)");
    }
    if state.taken && !state.closed && state.elapsed >= 11.0 {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Interact { entity: bits });
        }
        state.closed = true;
        tracing::info!("container-test: close sent");
    }
    if state.closed
        && !state.take_after_close
        && state.elapsed >= 13.5
        && let Some(item) = inventories
            .get(container_entity)
            .ok()
            .and_then(|inv| inv.slots.iter().flatten().copied().next())
    {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TransferItem {
                item,
                to_slot: SLOT_ANY,
                target_player: 0,
            });
        }
        state.take_after_close = true;
        tracing::info!(item, "container-test: take sent (container closed)");
    }
}

/// Параметры теста контейнера (собираем в структуру ради лимита аргументов).
#[derive(SystemParam)]
pub struct ContainerTestParams<'w, 's> {
    positions: Query<'w, 's, &'static PlayerPosition>,
    containers: WorldContainers<'w, 's>,
    inventories: Query<'w, 's, &'static Inventory>,
    own: Res<'w, crate::inventory_ui::OwnPlayerEntity>,
    entity_map: Option<Res<'w, ServerEntityMap>>,
}

/// Состояние теста контейнера.
#[derive(Default)]
pub struct ContainerTestState {
    elapsed: f32,
    opened: bool,
    taken: bool,
    closed: bool,
    take_after_close: bool,
}
