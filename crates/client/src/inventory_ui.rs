//! Инвентарь, руки и действия на клиенте (PLAN.md T3.2/T3.3, SS14-модель):
//! рюкзак 7×4, две руки с активной, атака предметом из руки, контекстные
//! действия (verbs) по правому клику, предмет в руке рисуется у держателя.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::PlayerPosition;
use ssr_core::inventory::{
    HAND_SLOTS, Hands, Health, HeldBy, INVENTORY_COLS, INVENTORY_ROWS, Inventory, Item, SLOT_ANY,
};
use ssr_core::roles::PlayerRole;
use ssr_protocol::net::GameChannel;
use ssr_protocol::{ActionOption, ClientMessage};

use crate::PlayerEntity;
use crate::rsi::{RsiRegistry, RsiSprite};
use crate::windows;

/// Юнитов на тайл (клик по тайловой сетке).
const TILE_UNITS: f32 = 32.0;

/// Спрайт предмета в слотах UI: имя предмета → RSI-стейт иконки.
pub fn item_icon<'a>(registry: &'a RsiRegistry, name: &str) -> Option<&'a RsiSprite> {
    let path = match name {
        "Crowbar" => "Objects/Tools/crowbar.rsi#icon",
        "SteelSheet" => "Objects/Materials/Sheets/metal.rsi#steel",
        _ => return None,
    };
    registry.get(&format!("sprites/ss14/{path}"))
}

/// Спрайт предмета в руке: inhand-стейт (4 направления), если он есть.
pub fn item_inhand<'a>(registry: &'a RsiRegistry, name: &str) -> Option<&'a RsiSprite> {
    let path = match name {
        "Crowbar" => "Objects/Tools/crowbar.rsi#inhand-right",
        "SteelSheet" => "Objects/Materials/Sheets/metal.rsi#steel-inhand-right",
        _ => return None,
    };
    registry.get(&format!("sprites/ss14/{path}"))
}
/// Время до авто-переноса в тестовом режиме SSR_INV_TEST.
const INV_TEST_DELAY: f32 = 5.0;

/// Клиентская сущность своего игрока (резолвится раз в кадр из Welcome+маппинга).
#[derive(Resource, Default)]
pub struct OwnPlayerEntity(pub Option<Entity>);

/// Меню контекстных действий (verbs): список от сервера + позиция курсора.
#[derive(Resource, Default)]
pub struct ActionMenu {
    pub options: Vec<ActionOption>,
    pub cursor: Vec2,
}

/// Визуал другого игрока, привязанный к реплицированной сущности.
#[derive(Component)]
pub struct RemotePlayerVisual {
    pub player: Entity,
}

/// Слот рюкзака в UI.
#[derive(Component, Clone, Copy)]
pub struct InvSlot(pub u8);

/// Слот руки в UI.
#[derive(Component, Clone, Copy)]
pub struct HandSlot(pub u8);

/// Корень панели рюкзака.
#[derive(Component)]
pub struct InventoryPanel;

/// Корень панели рук.
#[derive(Component)]
pub struct HandsPanel;

/// Корень меню действий.
#[derive(Component)]
pub struct ActionMenuRoot;

/// Кнопка пункта меню действий (индекс в списке).
#[derive(Component, Clone, Copy)]
pub struct ActionMenuOption(pub usize);

/// Иконка предмета в руке держателя.
#[derive(Component)]
pub struct InHandVisual {
    item: Entity,
}

/// Новые реплицированные позиции игроков (для спавна визуалов).
type AddedPositions<'w, 's> =
    Query<'w, 's, (Entity, &'static PlayerPosition), (With<Remote>, Added<PlayerPosition>)>;
/// Клики по слотам рюкзака.
type SlotClicks<'w, 's> =
    Query<'w, 's, (&'static Interaction, &'static InvSlot), (Changed<Interaction>, With<Button>)>;
/// Клики по слотам рук.
type HandClicks<'w, 's> =
    Query<'w, 's, (&'static Interaction, &'static HandSlot), (Changed<Interaction>, With<Button>)>;
/// Клики по пунктам меню действий.
type MenuClicks<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static ActionMenuOption),
    (Changed<Interaction>, With<Button>),
>;

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

/// Свой инвентарь (рюкзак) на клиенте.
fn own_inventory<'a>(
    own: &OwnPlayerEntity,
    inventories: &'a Query<&Inventory>,
) -> Option<&'a Inventory> {
    inventories.get(own.0?).ok()
}

/// Свои руки на клиенте.
fn own_hands<'a>(own: &OwnPlayerEntity, hands: &'a Query<&Hands>) -> Option<&'a Hands> {
    hands.get(own.0?).ok()
}

// ---------------------------------------------------------------- визуалы игроков

/// Спавнит визуалы других игроков (тело собирает `humanoid::sync_bodies`).
pub fn spawn_remote_players(
    mut commands: Commands,
    own_resource: Res<OwnPlayerEntity>,
    player_entity: Res<PlayerEntity>,
    entity_map: Option<Res<ServerEntityMap>>,
    added: AddedPositions,
) {
    // Свой игрок рисуется отдельной сущностью Player: если не отсеять его тут,
    // появится второй (немгновенный) визуал — «дёргающаяся кукла».
    let own = own_resource
        .0
        .or_else(|| crate::own_player_entity(&player_entity, &entity_map));
    for (entity, position) in added.iter() {
        if Some(entity) == own {
            continue;
        }
        commands.spawn((
            RemotePlayerVisual { player: entity },
            crate::humanoid::Facing(0),
            // Родителю нужна Visibility, иначе части тела-дети не видны (B0004).
            Visibility::default(),
            Transform::from_xyz(position.0[0], position.0[1], 0.9),
        ));
        tracing::info!(player = ?entity, "remote player visual spawned");
    }
}

/// Двигает визуалы других игроков за реплицированными позициями, выбирает
/// сторону по смещению и убирает «осиротевшие» (игрок вышел из интереса).
pub fn sync_remote_players(
    mut commands: Commands,
    own: Res<OwnPlayerEntity>,
    positions: Query<&PlayerPosition>,
    mut visuals: Query<(
        Entity,
        &RemotePlayerVisual,
        &mut Transform,
        &mut crate::humanoid::Facing,
    )>,
) {
    for (visual_entity, visual, mut transform, mut facing) in visuals.iter_mut() {
        // Дубль своего игрока (успел появиться до маппинга) — убираем.
        if Some(visual.player) == own.0 {
            commands.entity(visual_entity).despawn();
            continue;
        }
        let Ok(position) = positions.get(visual.player) else {
            commands.entity(visual_entity).despawn();
            continue;
        };
        let (x, y) = (position.0[0], position.0[1]);
        let delta = Vec2::new(x - transform.translation.x, y - transform.translation.y);
        if delta.length() >= 0.5 {
            facing.0 = if delta.x.abs() > delta.y.abs() {
                if delta.x > 0.0 { 2 } else { 3 }
            } else if delta.y > 0.0 {
                1
            } else {
                0
            };
        }
        transform.translation.x = x;
        transform.translation.y = y;
    }
}

/// Рисует предмет из АКТИВНОЙ руки у спрайта держателя (SS14-модель):
/// спрайт берётся по имени предмета (inhand-стейт) и поворачивается по взгляду.
pub fn sync_inhand_items(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    entity_map: Option<Res<ServerEntityMap>>,
    items: Query<(Entity, &HeldBy, &Item)>,
    hands: Query<&Hands>,
    holders: Query<(&PlayerPosition, &crate::humanoid::Facing)>,
    mut visuals: Query<(Entity, &InHandVisual, &mut Transform, &mut Sprite)>,
) {
    let Some(map) = entity_map.as_deref() else {
        return;
    };

    for (visual_entity, visual, mut transform, mut sprite) in visuals.iter_mut() {
        let holder_bits = items
            .get(visual.item)
            .map(|(_, held, _)| held.player)
            .unwrap_or_default();
        let holder = Entity::try_from_bits(holder_bits)
            .and_then(|server| map.to_client().get(&server).copied());
        let Some(holder_client) = holder else {
            commands.entity(visual_entity).despawn();
            continue;
        };
        let active = hands.get(holder_client).ok().and_then(|h| h.active_item());
        if active != Some(visual.item.to_bits()) {
            commands.entity(visual_entity).despawn();
            continue;
        }
        let Ok((position, facing)) = holders.get(holder_client) else {
            continue;
        };
        transform.translation.x = position.0[0] + 14.0;
        transform.translation.y = position.0[1] - 6.0;
        let name = items
            .get(visual.item)
            .ok()
            .map(|(_, _, item)| item.name.clone());
        if let Some(name) = name
            && let Some(rsi) = item_inhand(&registry, &name)
            && let Some(atlas) = sprite.texture_atlas.as_mut()
        {
            let index = rsi.index(facing.0.min(3), 0);
            if atlas.index != index {
                atlas.index = index;
            }
        }
    }

    for (item_entity, held, item) in items.iter() {
        if held.player == 0
            || visuals
                .iter()
                .any(|(_, visual, _, _)| visual.item == item_entity)
        {
            continue;
        }
        let Some(holder_server) = Entity::try_from_bits(held.player) else {
            continue;
        };
        let Some(holder_client) = map.to_client().get(&holder_server).copied() else {
            continue;
        };
        let active_matches = hands
            .get(holder_client)
            .ok()
            .and_then(|h| h.active_item())
            .is_some_and(|active| active == item_entity.to_bits());
        if !active_matches {
            continue;
        }
        let Some(rsi) = item_inhand(&registry, &item.name) else {
            continue;
        };
        let mut sprite = Sprite::from_image(rsi.image.clone());
        sprite.texture_atlas = Some(TextureAtlas {
            layout: rsi.layout.clone(),
            index: rsi.index(0, 0),
        });
        let (x, y) = holders
            .get(holder_client)
            .map(|(position, _)| (position.0[0] + 14.0, position.0[1] - 6.0))
            .unwrap_or((0.0, 0.0));
        commands.spawn((
            InHandVisual { item: item_entity },
            sprite,
            Transform::from_xyz(x, y, 1.1),
        ));
    }
}

// ---------------------------------------------------------------- UI

/// Доступ к спрайтам предметов для панелей UI (иконка по bits сущности).
#[derive(SystemParam)]
pub struct ItemSprites<'w, 's> {
    registry: Res<'w, RsiRegistry>,
    items: Query<'w, 's, &'static Item>,
}

impl ItemSprites<'_, '_> {
    /// Иконка предмета по bits его сущности (None — предмет неизвестен).
    pub fn icon(&self, bits: u64) -> Option<&RsiSprite> {
        let name = Entity::try_from_bits(bits)
            .and_then(|entity| self.items.get(entity).ok())
            .map(|item| item.name.clone())?;
        let sprite = item_icon(&self.registry, &name);
        if sprite.is_none() {
            tracing::warn!(item = %name, "no icon for item");
        }
        sprite
    }
}

/// Пересобирает панель рюкзака при изменениях содержимого.
pub fn render_inventory_panel(
    mut commands: Commands,
    sprites: ItemSprites,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    positions: Res<windows::WindowPositions>,
    root: Query<Entity, With<InventoryPanel>>,
    mut last: Local<Option<Vec<Option<u64>>>>,
) {
    let Some(inventory) = own_inventory(&own, &inventories) else {
        return;
    };
    if last.as_ref() == Some(&inventory.slots) {
        return;
    }
    *last = Some(inventory.slots.clone());
    tracing::info!(
        items = inventory.slots.iter().filter(|s| s.is_some()).count(),
        "inventory updated"
    );

    for entity in root.iter() {
        commands.entity(entity).despawn();
    }

    let mut node = Node {
        position_type: PositionType::Absolute,
        left: px(490),
        bottom: px(96),
        flex_direction: FlexDirection::Column,
        padding: UiRect::all(px(8)),
        row_gap: px(6),
        ..default()
    };
    windows::apply_saved_position(windows::WindowKind::Inventory, &mut node, &positions);

    commands
        .spawn((
            InventoryPanel,
            windows::WindowKind::Inventory,
            windows::WindowDrag::default(),
            Interaction::default(),
            node,
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.72)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Рюкзак"),
                TextFont::from_font_size(13.0),
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
                                let mut slot = line.spawn((
                                    InvSlot(index as u8),
                                    Button,
                                    Node {
                                        width: px(36),
                                        height: px(36),
                                        border: UiRect::all(px(2)),
                                        align_items: AlignItems::Center,
                                        justify_content: JustifyContent::Center,
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgb(0.12, 0.12, 0.15)),
                                    BorderColor::from(Color::srgb(0.28, 0.28, 0.33)),
                                ));
                                if let Some(bits) = item
                                    && let Some(sprite) = sprites.icon(bits)
                                {
                                    let mut image = ImageNode::new(sprite.image.clone());
                                    image.texture_atlas = Some(TextureAtlas {
                                        layout: sprite.layout.clone(),
                                        index: sprite.index(0, 0),
                                    });
                                    slot.with_child((
                                        image,
                                        Node {
                                            width: px(28),
                                            height: px(28),
                                            ..default()
                                        },
                                    ));
                                }
                            }
                        });
                    }
                });
            panel.spawn((
                Text::new("ЛКМ — взять в руку; X — сменить руку; ПКМ — действия"),
                TextFont::from_font_size(11.0),
                TextColor(Color::srgb(0.62, 0.62, 0.66)),
            ));
        });
}

/// Пересобирает панель рук при изменениях (активная рука, предметы).
/// Расположение — внизу по центру, как хотбар рук в SS14.
pub fn render_hands_panel(
    mut commands: Commands,
    sprites: ItemSprites,
    own: Res<OwnPlayerEntity>,
    hands: Query<&Hands>,
    positions: Res<windows::WindowPositions>,
    root: Query<Entity, With<HandsPanel>>,
    mut last: Local<Option<Hands>>,
) {
    let Some(own_state) = own_hands(&own, &hands) else {
        return;
    };
    if last.as_ref() == Some(own_state) {
        return;
    }
    *last = Some(own_state.clone());
    tracing::info!(active = own_state.active, "hands updated");

    for entity in root.iter() {
        commands.entity(entity).despawn();
    }

    let mut node = Node {
        position_type: PositionType::Absolute,
        left: px(560),
        bottom: px(24),
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        padding: UiRect::all(px(8)),
        column_gap: px(4),
        ..default()
    };
    windows::apply_saved_position(windows::WindowKind::Hands, &mut node, &positions);

    commands
        .spawn((
            HandsPanel,
            windows::WindowKind::Hands,
            windows::WindowDrag::default(),
            Interaction::default(),
            node,
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.72)),
        ))
        .with_children(|row| {
            for hand_index in 0..HAND_SLOTS as u8 {
                let item = own_state.slots.get(hand_index as usize).copied().flatten();
                let is_active = own_state.active == hand_index;
                let mut slot = row.spawn((
                    HandSlot(hand_index),
                    Button,
                    Node {
                        width: px(46),
                        height: px(46),
                        border: UiRect::all(px(2)),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    BackgroundColor(if is_active {
                        Color::srgb(0.20, 0.20, 0.28)
                    } else {
                        Color::srgb(0.12, 0.12, 0.15)
                    }),
                    BorderColor::from(if is_active {
                        Color::srgb(1.0, 0.75, 0.25)
                    } else {
                        Color::srgb(0.28, 0.28, 0.33)
                    }),
                ));
                if let Some(bits) = item
                    && let Some(sprite) = sprites.icon(bits)
                {
                    let mut image = ImageNode::new(sprite.image.clone());
                    image.texture_atlas = Some(TextureAtlas {
                        layout: sprite.layout.clone(),
                        index: sprite.index(0, 0),
                    });
                    slot.with_child((
                        image,
                        Node {
                            width: px(34),
                            height: px(34),
                            ..default()
                        },
                    ));
                }
            }
        });
}

/// HUD здоровья своего игрока (T4.1): «HP 100/100» над панелью рук.
#[derive(Component)]
pub struct HealthHudRoot;

/// Текст HUD здоровья.
#[derive(Component)]
pub struct HealthHudText;

/// Строка роли в HUD (T4.2).
#[derive(Component)]
pub struct RoleHudText;

/// Строка атмосферы в HUD (T4.3): давление и кислород тайла под игроком.
#[derive(Component)]
pub struct AtmosHudText;

/// Создаёт HUD (здоровье + роль) один раз, когда свой игрок появился.
pub fn spawn_health_hud(
    mut commands: Commands,
    own: Res<OwnPlayerEntity>,
    roots: Query<(), With<HealthHudRoot>>,
) {
    if own.0.is_none() || !roots.is_empty() {
        return;
    }
    commands
        .spawn((
            HealthHudRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(12),
                bottom: px(295),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                padding: UiRect::new(px(10), px(10), px(6), px(6)),
                border: UiRect::all(px(2)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.72)),
            BorderColor::from(Color::srgb(0.28, 0.28, 0.33)),
        ))
        .with_children(|panel| {
            panel.spawn((
                HealthHudText,
                Text::new("HP 100/100"),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.55, 0.85, 0.55)),
            ));
            panel.spawn((
                RoleHudText,
                Text::new("Роль: —"),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.80, 0.80, 0.84)),
            ));
            panel.spawn((
                AtmosHudText,
                Text::new("Атм: —"),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.75, 0.85, 0.75)),
            ));
        });
}

/// Показывает атмосферу тайла под своим игроком (T4.3): давление и кислород,
/// красным — когда дышать нечем.
pub fn update_atmos_hud(
    own: Res<OwnPlayerEntity>,
    positions: Query<&PlayerPosition>,
    atmospheres: Query<&ssr_core::atmosphere::ChunkAtmosphere>,
    mut texts: Query<(&mut Text, &mut TextColor), With<AtmosHudText>>,
) {
    let Some(entity) = own.0 else {
        return;
    };
    let Ok(position) = positions.get(entity) else {
        return;
    };
    let tiles = ssr_core::tiles::CHUNK_TILES as i32;
    let tx = (position.0[0] / TILE_UNITS).floor() as i32;
    let ty = (position.0[1] / TILE_UNITS).floor() as i32;
    let chunk = (tx.div_euclid(tiles), ty.div_euclid(tiles));
    let gas = atmospheres
        .iter()
        .find(|atmosphere| atmosphere.coords == chunk)
        .and_then(|atmosphere| {
            atmosphere.at((tx - chunk.0 * tiles) as u32, (ty - chunk.1 * tiles) as u32)
        });
    let (value, color) = match gas {
        Some(gas) if gas.is_breathable() => (
            format!(
                "Атм: {:.0} кПа · O₂ {:.0}%",
                gas.pressure,
                gas.oxygen * 100.0
            ),
            Color::srgb(0.75, 0.85, 0.75),
        ),
        Some(gas) => (
            format!(
                "Атм: {:.0} кПа · O₂ {:.0}% ⚠",
                gas.pressure,
                gas.oxygen * 100.0
            ),
            Color::srgb(0.95, 0.45, 0.35),
        ),
        None => ("Атм: —".to_string(), Color::srgb(0.60, 0.60, 0.62)),
    };
    for (mut text, mut text_color) in &mut texts {
        if text.0 != value {
            text.0 = value.clone();
            tracing::info!(atmosphere = %value, "atmosphere hud updated");
        }
        if text_color.0 != color {
            text_color.0 = color;
        }
    }
}

/// Показывает роль своего игрока (T4.2): у антагониста — ещё и цель.
pub fn update_role_hud(
    own: Res<OwnPlayerEntity>,
    roles: Query<&PlayerRole>,
    mut texts: Query<&mut Text, With<RoleHudText>>,
) {
    let Some(entity) = own.0 else {
        return;
    };
    let Ok(role) = roles.get(entity) else {
        return;
    };
    let value = if role.antagonist && !role.goal.is_empty() {
        format!("Роль: {} — цель: {}", role.name, role.goal)
    } else {
        format!("Роль: {}", role.name)
    };
    for mut text in &mut texts {
        if text.0 != value {
            text.0 = value.clone();
            tracing::info!(role = %role.id, antagonist = role.antagonist, "own role received");
        }
    }
}

/// Обновляет HUD здоровья из реплицированного Health (T4.1).
pub fn update_health_hud(
    own: Res<OwnPlayerEntity>,
    healths: Query<&Health>,
    mut texts: Query<(&mut Text, &mut TextColor), With<HealthHudText>>,
) {
    let Some(entity) = own.0 else {
        return;
    };
    let Ok(health) = healths.get(entity) else {
        return;
    };
    let value = format!("HP {}/{}", health.current, health.max);
    let color = if health.current > 60 {
        Color::srgb(0.55, 0.85, 0.55)
    } else if health.current > 30 {
        Color::srgb(0.92, 0.82, 0.35)
    } else {
        Color::srgb(0.92, 0.35, 0.32)
    };
    for (mut text, mut text_color) in &mut texts {
        if text.0 != value {
            text.0 = value.clone();
            tracing::info!(hp = health.current, "own health changed");
        }
        if text_color.0 != color {
            text_color.0 = color;
        }
    }
}

/// Меню действий: строится из ответа сервера, позиционируется у курсора.
pub fn render_action_menu(
    mut commands: Commands,
    menu: Res<ActionMenu>,
    root: Query<Entity, With<ActionMenuRoot>>,
    mut last: Local<Option<(usize, Vec2)>>,
) {
    let signature = (menu.options.len(), menu.cursor);
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    if menu.options.is_empty() {
        return;
    }
    commands
        .spawn((
            ActionMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(menu.cursor.x.clamp(0.0, 1100.0)),
                top: px(menu.cursor.y.clamp(0.0, 600.0)),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(4)),
                row_gap: px(2),
                ..default()
            },
            BackgroundColor(Color::srgba(0.08, 0.08, 0.10, 0.95)),
        ))
        .with_children(|list| {
            for (index, option) in menu.options.iter().enumerate() {
                list.spawn((
                    ActionMenuOption(index),
                    Button,
                    Node {
                        padding: UiRect::axes(px(10), px(5)),
                        border: UiRect::all(px(1)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.13, 0.13, 0.16)),
                    BorderColor::from(Color::srgb(0.30, 0.30, 0.35)),
                ))
                .with_child((
                    Text::new(option.label.clone()),
                    TextFont::from_font_size(14.0),
                    TextColor(Color::srgb(0.90, 0.90, 0.92)),
                ));
            }
        });
}

// ---------------------------------------------------------------- ввод

/// Клик по слоту рюкзака: взять предмет в активную руку (SS14-модель).
pub fn inventory_slot_click(
    slots: SlotClicks,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    hands: Query<&Hands>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    for (interaction, slot) in slots.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if own_inventory(&own, &inventories).is_none() {
            continue;
        }
        let active_has_item = own_hands(&own, &hands)
            .map(|h| h.active_item().is_some())
            .unwrap_or(true);
        if active_has_item {
            tracing::debug!("slot click: active hand is busy");
            continue;
        }
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TakeInHand { slot: slot.0 });
        }
        tracing::info!(slot = slot.0, "take in hand sent");
    }
}

/// Клик по рукам: активная рука с предметом — убрать в рюкзак, иначе — переключить.
pub fn hands_ui_click(
    hands_slots: HandClicks,
    own: Res<OwnPlayerEntity>,
    hands: Query<&Hands>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    let own_state = own_hands(&own, &hands).cloned();
    for (interaction, slot) in hands_slots.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(state) = &own_state else {
            continue;
        };
        let is_active = state.active == slot.0;
        let item = state.item_in_hand(slot.0);
        if is_active && item.is_some() {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::DropHand);
            }
            tracing::info!("hand item stow sent");
        } else if !is_active {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::SwitchHand);
            }
        }
    }
}

/// Клик по миру: игрок под курсором — атака (Ctrl — передать предмет),
/// иначе — применить предмет из активной руки к тайлу. Правый клик — меню действий.
#[allow(clippy::too_many_arguments)]
pub fn world_click(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    own: Res<OwnPlayerEntity>,
    hands: Query<&Hands>,
    visuals: Query<(&RemotePlayerVisual, &Transform)>,
    containers: Query<(
        Entity,
        &ssr_core::inventory::Container,
        &ssr_core::inventory::ItemPosition,
    )>,
    entity_map: Option<Res<ServerEntityMap>>,
    mut menu: ResMut<ActionMenu>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
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

    // Правый клик — меню контекстных действий (verbs).
    if buttons.just_pressed(MouseButton::Right) {
        let target = player_under_cursor(&visuals, world);
        let (entity, tx, ty) = match target {
            Some((player_entity, _)) => {
                let bits = entity_map
                    .as_deref()
                    .and_then(|m| m.to_server().get(&player_entity))
                    .copied()
                    .map(Entity::to_bits)
                    .unwrap_or_default();
                (bits, 0, 0)
            }
            None => (
                0,
                (world.x / TILE_UNITS).floor() as i32,
                (world.y / TILE_UNITS).floor() as i32,
            ),
        };
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::RequestActions { entity, tx, ty });
        }
        menu.cursor = cursor;
        tracing::info!(entity, tx, ty, "actions requested");
        return;
    }

    if !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !menu.options.is_empty() {
        menu.options.clear();
    }

    let active_item = own_hands(&own, &hands).and_then(|h| h.active_item());

    // 1) Игрок под курсором: атака; с Ctrl — передать предмет из руки.
    if let Some((target_entity, _)) = player_under_cursor(&visuals, world) {
        let Some(map) = entity_map.as_deref() else {
            return;
        };
        let Some(server_entity) = map.to_server().get(&target_entity) else {
            return;
        };
        let target_bits = server_entity.to_bits();
        if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
            if let Some(item) = active_item {
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::TransferItem {
                        item,
                        to_slot: SLOT_ANY,
                        target_player: target_bits,
                    });
                }
                tracing::info!(item, "give item sent (ctrl+click)");
            }
        } else {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::Attack {
                    target: target_bits,
                });
            }
            tracing::info!(?target_entity, "attack sent");
        }
        return;
    }

    // 2) Ящик под курсором: открыть/закрыть (T3.4).
    if let Some(container_entity) = container_under_cursor(&containers, world)
        && let Some(bits) = entity_map
            .as_deref()
            .and_then(|m| m.to_server().get(&container_entity))
            .copied()
            .map(Entity::to_bits)
    {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Interact { entity: bits });
        }
        tracing::info!(?container_entity, "container interact sent");
        return;
    }

    // 3) Тайл: применить предмет из активной руки (стройка/разборка).
    if let Some(item) = active_item {
        let tx = (world.x / TILE_UNITS).floor() as i32;
        let ty = (world.y / TILE_UNITS).floor() as i32;
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::UseItem { item, tx, ty });
        }
        tracing::info!(tx, ty, "use item sent");
    }
}

/// Ближайший контейнер под точкой клика (для открытия/закрытия, T3.4).
fn container_under_cursor(
    containers: &Query<(
        Entity,
        &ssr_core::inventory::Container,
        &ssr_core::inventory::ItemPosition,
    )>,
    world: Vec2,
) -> Option<Entity> {
    let mut best: Option<(Entity, f32)> = None;
    for (entity, _, position) in containers.iter() {
        let distance = Vec2::from_array(position.0).distance(world);
        if distance <= 24.0 && best.is_none_or(|(_, d)| distance < d) {
            best = Some((entity, distance));
        }
    }
    best.map(|(entity, _)| entity)
}

/// Ближайший другой игрок под точкой клика.
fn player_under_cursor(
    visuals: &Query<(&RemotePlayerVisual, &Transform)>,
    world: Vec2,
) -> Option<(Entity, f32)> {
    let mut best: Option<(Entity, f32)> = None;
    for (visual, transform) in visuals.iter() {
        let distance = transform.translation.truncate().distance(world);
        if distance <= 32.0 && best.is_none_or(|(_, d)| distance < d) {
            best = Some((visual.player, distance));
        }
    }
    best
}

/// Клик по пункту меню действий: выполняем выбранное.
pub fn action_menu_click(
    options: MenuClicks,
    mut menu: ResMut<ActionMenu>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    for (interaction, option) in options.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(entry) = menu.options.get(option.0).cloned() else {
            continue;
        };
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::PerformAction {
                action: entry.action.clone(),
            });
        }
        tracing::info!(label = %entry.label, "action performed");
        menu.options.clear();
    }
}

// ---------------------------------------------------------------- тест-режимы

/// Тестовый режим SSR_INV_TEST=1: через [`INV_TEST_DELAY`] секунд клиент
/// передаёт первый предмет рюкзака ближайшему игроку (критерий T3.2).
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
        tracing::warn!("inv-test: own inventory is empty");
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

/// Состояние теста стройки.
#[derive(Default)]
pub struct BuildTestState {
    elapsed: f32,
    taken_sheet: bool,
    sheet: Option<u64>,
    built: bool,
    taken_tool: bool,
    crowbar: Option<u64>,
    destroyed: bool,
}

/// Тестовый режим SSR_BUILD_TEST=1 (T3.3): берёт лист в руку, строит стену в
/// двух тайлах на восток, затем берёт лом и разбирает её. Ходьба в стену
/// проверяется в send_input (упор = «твёрдость» стены).
pub fn build_test_mode(
    time: Res<Time>,
    own: Res<OwnPlayerEntity>,
    positions: Query<&PlayerPosition>,
    inventories: Query<&Inventory>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut state: Local<BuildTestState>,
) {
    if std::env::var_os("SSR_BUILD_TEST").is_none() {
        return;
    }
    state.elapsed += time.delta_secs();
    let Some(own_entity) = own.0 else {
        return;
    };
    let Ok(position) = positions.get(own_entity) else {
        return;
    };
    let tile = (
        (position.0[0] / TILE_UNITS).floor() as i32,
        (position.0[1] / TILE_UNITS).floor() as i32,
    );
    let target = (tile.0 + 2, tile.1);

    // Слоты спавна: Crowbar(0), SteelSheet(1), SteelSheet(2).
    let slot_item = |slot: usize| -> Option<u64> {
        own_inventory(&own, &inventories)?
            .slots
            .get(slot)
            .copied()
            .flatten()
    };

    if !state.taken_sheet
        && state.elapsed >= 5.0
        && let Some(sheet) = slot_item(1)
    {
        state.sheet = Some(sheet);
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TakeInHand { slot: 1 });
        }
        state.taken_sheet = true;
        tracing::info!("build-test: take sheet in hand");
    }
    if state.taken_sheet
        && !state.built
        && state.elapsed >= 6.5
        && let Some(sheet) = state.sheet
    {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::UseItem {
                item: sheet,
                tx: target.0,
                ty: target.1,
            });
        }
        state.built = true;
        tracing::info!(tx = target.0, ty = target.1, "build-test: build sent");
    }
    if state.built
        && !state.taken_tool
        && state.elapsed >= 14.0
        && let Some(crowbar) = slot_item(0)
    {
        state.crowbar = Some(crowbar);
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TakeInHand { slot: 0 });
        }
        state.taken_tool = true;
        tracing::info!("build-test: take crowbar in hand");
    }
    if state.taken_tool
        && !state.destroyed
        && state.elapsed >= 15.0
        && let Some(crowbar) = state.crowbar
    {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::UseItem {
                item: crowbar,
                tx: target.0,
                ty: target.1,
            });
        }
        state.destroyed = true;
        tracing::info!(tx = target.0, ty = target.1, "build-test: deconstruct sent");
    }
}

/// Тестовый режим SSR_ATTACK_TEST=1: через 6 секунд бьёт ближайшего игрока.
pub fn attack_test_mode(
    time: Res<Time>,
    entity_map: Option<Res<ServerEntityMap>>,
    visuals: Query<&RemotePlayerVisual>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut state: Local<(f32, bool)>,
) {
    if std::env::var_os("SSR_ATTACK_TEST").is_none() || state.1 {
        return;
    }
    state.0 += time.delta_secs();
    if state.0 < 6.0 {
        return;
    }
    let Some(visual) = visuals.iter().next() else {
        return;
    };
    let Some(map) = entity_map.as_deref() else {
        return;
    };
    let Some(server_entity) = map.to_server().get(&visual.player) else {
        return;
    };
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Attack {
            target: server_entity.to_bits(),
        });
    }
    state.1 = true;
    tracing::info!("attack-test: attack sent");
}
