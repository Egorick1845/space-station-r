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

/// RSI ящика из сборки: корпус (`base`) и крышка (`closed`/`open`) — в
/// движке это отдельные стейты, рисуются друг поверх друга.
const CRATE_BASE: &str = "sprites/ss14/Structures/Storage/Crates/generic.rsi#base";
const CRATE_CLOSED: &str = "sprites/ss14/Structures/Storage/Crates/generic.rsi#closed";
const CRATE_OPEN: &str = "sprites/ss14/Structures/Storage/Crates/generic.rsi#open";
/// Радиус, в котором открытый ящик показывается на экране.
const CONTAINER_UI_RANGE: f32 = 96.0;

/// Спрайт ящика (корпус), привязанный к реплицированной сущности.
#[derive(Component)]
pub struct CrateVisual {
    container: Entity,
}

/// Крышка ящика — дочерний спрайт поверх корпуса (стейты closed/open).
#[derive(Component)]
pub struct CrateLid {
    container: Entity,
}

/// Корень панели открытого ящика.
#[derive(Component)]
pub struct ContainerPanel;

/// Слот в панели ящика.
#[derive(Component, Clone, Copy)]
pub struct ContainerSlot {
    /// Сущность контейнера (клиентская, мапится в bits при отправке).
    pub container: Entity,
    /// Индекс клетки в сетке контейнера.
    pub slot: u8,
}

/// Состояние панели ящика для сравнения при перерисовке.
type PanelSignature = Option<(Option<Entity>, Vec<Option<u64>>, u32)>;
/// Клик по слоту ящика.
type ContainerSlotClicks<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static ContainerSlot),
    (Changed<Interaction>, With<Button>),
>;
/// Контейнеры мира (сущность, состояние, позиция).
type WorldContainers<'w, 's> = Query<'w, 's, (Entity, &'static Container, &'static ItemPosition)>;

/// Спавнит спрайты ящиков: корпус + крышка (closed/open) поверх него.
pub fn spawn_crate_visuals(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    added: Query<(Entity, &Container, &ItemPosition), Added<Container>>,
) {
    let (Some(base), Some(closed), Some(open)) = (
        registry.get(CRATE_BASE),
        registry.get(CRATE_CLOSED),
        registry.get(CRATE_OPEN),
    ) else {
        tracing::warn!("crate rsi sprites missing");
        return;
    };
    for (entity, container, position) in added.iter() {
        let lid = if container.open { open } else { closed };
        let mut base_sprite = Sprite::from_image(base.image.clone());
        base_sprite.texture_atlas = Some(TextureAtlas {
            layout: base.layout.clone(),
            index: 0,
        });
        let mut lid_sprite = Sprite::from_image(lid.image.clone());
        lid_sprite.texture_atlas = Some(TextureAtlas {
            layout: lid.layout.clone(),
            index: 0,
        });
        commands
            .spawn((
                CrateVisual { container: entity },
                base_sprite,
                Transform::from_xyz(position.0[0], position.0[1], 0.7),
            ))
            .with_child((
                CrateLid { container: entity },
                lid_sprite,
                Transform::from_xyz(0.0, 0.0, 0.1),
            ));
        tracing::info!(container = ?entity, open = container.open, "crate visual spawned");
    }
}

/// Меняет крышку при открытии/закрытии и убирает осиротевшие визуалы.
pub fn update_crate_visuals(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    containers: Query<&Container>,
    visuals: Query<(Entity, &CrateVisual)>,
    mut lids: Query<(&CrateLid, &mut Sprite)>,
) {
    let (Some(closed), Some(open)) = (registry.get(CRATE_CLOSED), registry.get(CRATE_OPEN)) else {
        return;
    };
    for (visual_entity, visual) in visuals.iter() {
        if containers.get(visual.container).is_err() {
            commands.entity(visual_entity).despawn();
        }
    }
    for (lid, mut sprite) in lids.iter_mut() {
        let Ok(container) = containers.get(lid.container) else {
            continue;
        };
        let target = if container.open { open } else { closed };
        if sprite.image != target.image {
            sprite.image = target.image.clone();
            sprite.texture_atlas = Some(TextureAtlas {
                layout: target.layout.clone(),
                index: 0,
            });
            tracing::info!(container = ?lid.container, open = container.open, "crate state changed");
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
#[allow(unreachable_code, unused_variables)]
pub fn render_container_panel(
    mut commands: Commands,
    sprites: crate::inventory_ui::ItemSprites,
    theme: Res<crate::ui_theme::UiTheme>,
    own: Res<crate::inventory_ui::OwnPlayerEntity>,
    positions: Query<&PlayerPosition>,
    window_positions: Res<crate::windows::WindowPositions>,
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
                .map(|inv| inv.cells.clone())
                .unwrap_or_default();
            (Some(entity), slots)
        }
        None => (None, Vec::new()),
    };
    let signature = (container_entity, slots.clone(), sprites.generation());
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);

    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    // Ящики в сборке — `EntityStorage`: сеточного окна у них НЕТ. Предмет просто
    // перетаскивают в ящик, и он там лежит (владелец: «нет никакого интерфейса
    // хранилища у ящика»). Ниже — прежний рендер StorageWindow, оставлен как
    // справка до полного переноса EntityStorage (PORT_PLAN 1.7).
    return;
    let Some(container_entity) = container_entity else {
        return;
    };
    tracing::info!(?container_entity, "container panel opened");

    // Окно как StorageWindow в SS14: сайдбар с крестиком + сетка ячеек 32×32.
    let cell = crate::ui_theme::STORAGE_CELL;
    let grid_width = ssr_core::inventory::INVENTORY_COLS as f32 * cell;
    let grid_height = ssr_core::inventory::INVENTORY_ROWS as f32 * cell;
    // Окно хранилища в SS14 открывается по левому краю по центру (`CenterLeft`).
    let mut node = Node {
        position_type: PositionType::Absolute,
        left: px(10),
        top: Val::Percent(50.0),
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Start,
        ..default()
    };
    let transform = UiTransform::from_translation(Val2::new(Val::Px(0.0), Val::Percent(-50.0)));
    let _ = &transform;
    crate::windows::apply_saved_position(
        crate::windows::WindowKind::Container,
        &mut node,
        &window_positions,
    );

    commands
        .spawn((
            ContainerPanel,
            crate::windows::WindowKind::Container,
            crate::windows::WindowDrag::default(),
            Interaction::default(),
            node,
        ))
        .with_children(|window| {
            // Сайдбар: красный крестик закрывает ящик (Interact по контейнеру).
            window
                .spawn((
                    crate::ui_theme::stretched(&theme.storage_sidebar),
                    Node {
                        width: px(cell),
                        height: px(grid_height),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                ))
                .with_children(|sidebar| {
                    sidebar.spawn((
                        ContainerCloseButton(container_entity),
                        Button,
                        crate::ui_theme::stretched(&theme.storage_exit),
                        Node {
                            width: px(cell),
                            height: px(cell),
                            ..default()
                        },
                    ));
                });
            window
                .spawn((
                    Node {
                        width: px(grid_width),
                        height: px(grid_height),
                        ..default()
                    },
                    BackgroundColor(crate::ui_theme::GRID_BACKGROUND),
                ))
                .with_children(|grid| {
                    // Пустые клетки (клик — положить из руки в эту позицию).
                    for index in 0..(ssr_core::inventory::INVENTORY_COLS
                        * ssr_core::inventory::INVENTORY_ROWS)
                    {
                        let x = index % ssr_core::inventory::INVENTORY_COLS;
                        let y = index / ssr_core::inventory::INVENTORY_COLS;
                        let mut tile = crate::ui_theme::stretched(&theme.storage_tile);
                        tile.color = crate::ui_theme::GRID_BACKGROUND;
                        grid.spawn((
                            ContainerSlot {
                                container: container_entity,
                                slot: index,
                            },
                            Button,
                            storage_cell(x, y),
                            tile,
                        ));
                    }
                    // Предметы поверх — во всю занятую площадь (тетрис).
                    for (bits, x, y, w, h) in ssr_core::inventory::item_layout(&slots) {
                        let Some(sprite) = sprites.icon(bits) else {
                            continue;
                        };
                        grid.spawn((
                            ContainerSlot {
                                container: container_entity,
                                slot: y * ssr_core::inventory::INVENTORY_COLS + x,
                            },
                            Button,
                            storage_cell_span(x, y, w, h),
                            BackgroundColor(crate::ui_theme::GLASS_BUTTON),
                        ))
                        .with_child((
                            crate::inventory_ui::icon_node(sprite),
                            Node {
                                position_type: PositionType::Absolute,
                                left: px((w as f32 - 1.0) * cell * 0.5),
                                top: px((h as f32 - 1.0) * cell * 0.5),
                                width: px(cell),
                                height: px(cell),
                                ..default()
                            },
                        ));
                    }
                });
        });
}

/// Ячейка сетки хранилища (клетка 32×32, без зазоров).
fn storage_cell(x: u8, y: u8) -> Node {
    let cell = crate::ui_theme::STORAGE_CELL;
    Node {
        position_type: PositionType::Absolute,
        left: px(x as f32 * cell),
        top: px(y as f32 * cell),
        width: px(cell),
        height: px(cell),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        ..default()
    }
}

/// Предмет в сетке: w×h ячеек.
fn storage_cell_span(x: u8, y: u8, w: u8, h: u8) -> Node {
    let cell = crate::ui_theme::STORAGE_CELL;
    let mut node = storage_cell(x, y);
    node.width = px(w as f32 * cell);
    node.height = px(h as f32 * cell);
    node
}

/// Крестик закрытия окна ящика.
#[derive(Component)]
pub struct ContainerCloseButton(pub Entity);

/// Клик по крестику — закрыть ящик (Interact по контейнеру).
pub fn container_close_click(
    buttons: Query<(&Interaction, &ContainerCloseButton), Changed<Interaction>>,
    entity_map: Option<Res<ServerEntityMap>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    for (interaction, close) in buttons.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(map) = entity_map.as_deref() else {
            continue;
        };
        let Some(server_entity) = map.to_server().get(&close.0) else {
            continue;
        };
        let bits = server_entity.to_bits();
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Interact { entity: bits });
        }
        tracing::info!(container = close.0.to_bits(), "container closed by cross");
    }
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
            .and_then(|inv| inv.cells.get(slot.slot as usize).copied().flatten());
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
            .and_then(|inv| inv.cells.iter().flatten().copied().next())
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
            .and_then(|inv| inv.cells.iter().flatten().copied().next())
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
