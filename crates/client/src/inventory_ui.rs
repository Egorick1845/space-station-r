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
    Container, Hands, HeldBy, INVENTORY_COLS, INVENTORY_ROWS, Inventory, Item, SLOT_ANY,
};
use ssr_protocol::net::GameChannel;
use ssr_protocol::{ActionOption, ClientMessage};

use crate::PlayerEntity;
use crate::containers::ContainerSlot;
use crate::rsi::{RsiRegistry, RsiSprite};
use crate::windows;

/// Спрайт предмета в слотах UI: имя предмета → RSI-стейт иконки. Сначала наш
/// каталог, затем импортированные прототипы сборки (меню спавна показывает и их).
pub fn item_icon<'a>(
    registry: &'a RsiRegistry,
    content: &'a crate::content::ClientContent,
    name: &str,
) -> Option<&'a RsiSprite> {
    let key = content.sprite_key(name)?;
    registry.get(&format!("sprites/ss14/{key}"))
}

/// Спрайт предмета в руке: inhand-стейт из каталога (если есть).
pub fn item_inhand<'a>(
    registry: &'a RsiRegistry,
    catalog: &ssr_core::items::ItemSet,
    name: &str,
) -> Option<&'a RsiSprite> {
    let key = catalog.by_id(name)?.inhand.as_ref()?;
    registry.get(&format!("sprites/ss14/{key}"))
}

/// Юнитов на тайл (клик по тайловой сетке).
const TILE_UNITS: f32 = 32.0;

/// Время до авто-переноса в тестовом режиме SSR_INV_TEST.
const INV_TEST_DELAY: f32 = 5.0;

/// Интерполяция удалённого игрока: тот же prev/target за тик сети, что у себя —
/// без неё чужие персонажи дёргаются на частоте тиков (20 Гц).
#[derive(Component, Default)]
pub struct RemoteInterp {
    prev: Vec2,
    target: Vec2,
    elapsed: f32,
    started: bool,
}

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
            RemoteInterp::default(),
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
        &mut RemoteInterp,
        &mut crate::humanoid::Facing,
    )>,
) {
    for (visual_entity, visual, mut interp, mut facing) in visuals.iter_mut() {
        // Дубль своего игрока (успел появиться до маппинга) — убираем.
        if Some(visual.player) == own.0 {
            commands.entity(visual_entity).despawn();
            continue;
        }
        let Ok(position) = positions.get(visual.player) else {
            commands.entity(visual_entity).despawn();
            continue;
        };
        let target = Vec2::from_array(position.0);
        if !interp.started {
            interp.prev = target;
            interp.target = target;
            interp.started = true;
        } else if target != interp.target {
            // Новый снимок: продолжаем от текущей отрисованной точки.
            let alpha = (interp.elapsed / crate::NET_TICK_SECS).clamp(0.0, 1.0);
            interp.prev = interp.prev.lerp(interp.target, alpha);
            interp.target = target;
            interp.elapsed = 0.0;
        }
        let delta = interp.target - interp.prev;
        if delta.length() >= 0.5 {
            facing.0 = if delta.x.abs() > delta.y.abs() {
                if delta.x > 0.0 { 2 } else { 3 }
            } else if delta.y > 0.0 {
                1
            } else {
                0
            };
        }
    }
}

/// Рисует предмет из АКТИВНОЙ руки у спрайта держателя (SS14-модель):
/// спрайт берётся по имени предмета (inhand-стейт) и поворачивается по взгляду.
#[allow(clippy::too_many_arguments)]
pub fn sync_inhand_items(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    catalog: Res<crate::content::ClientContent>,
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
        // Спрайт inhand уже содержит положение руки: рисуем в точке держателя.
        transform.translation.x = position.0[0];
        transform.translation.y = position.0[1];
        let name = items
            .get(visual.item)
            .ok()
            .map(|(_, _, item)| item.name.clone());
        if let Some(name) = name
            && let Some(rsi) = item_inhand(&registry, &catalog.items, &name)
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
        let Some(rsi) = item_inhand(&registry, &catalog.items, &item.name) else {
            continue;
        };
        let mut sprite = Sprite::from_image(rsi.image.clone());
        sprite.texture_atlas = Some(TextureAtlas {
            layout: rsi.layout.clone(),
            index: rsi.index(0, 0),
        });
        let (x, y) = holders
            .get(holder_client)
            .map(|(position, _)| (position.0[0], position.0[1]))
            .unwrap_or((0.0, 0.0));
        commands.spawn((
            InHandVisual { item: item_entity },
            sprite,
            Transform::from_xyz(x, y, 1.13),
        ));
    }
}

// ---------------------------------------------------------------- UI

/// Доступ к спрайтам предметов для панелей UI (иконка по bits сущности).
#[derive(SystemParam)]
pub struct ItemSprites<'w, 's> {
    registry: Res<'w, RsiRegistry>,
    catalog: Res<'w, crate::content::ClientContent>,
    items: Query<'w, 's, &'static Item>,
    entity_map: Option<Res<'w, ServerEntityMap>>,
}

impl ItemSprites<'_, '_> {
    /// Поколение реестра RSI: меняется, когда подгрузились новые спрайты (T5.3).
    pub fn generation(&self) -> u32 {
        self.registry.generation()
    }

    /// Имя предмета по серверным bits: bits → серверная сущность → карта
    /// репликации → клиентская сущность (иначе иконка не найдётся).
    pub fn item_name(&self, bits: u64) -> Option<String> {
        let server = Entity::try_from_bits(bits)?;
        let client = self
            .entity_map
            .as_deref()?
            .to_client()
            .get(&server)
            .copied()?;
        self.items.get(client).ok().map(|item| item.name.clone())
    }

    /// Название предмета для UI по серверным bits (русское имя из каталога,
    /// для импортированных прототипов — имя из прототипа сборки).
    pub fn display_name(&self, bits: u64) -> Option<String> {
        let name = self.item_name(bits)?;
        Some(self.catalog.display_name(&name))
    }

    /// Иконка предмета по его id (для спрайта режима размещения).
    pub fn icon_by_name(&self, name: &str) -> Option<&RsiSprite> {
        item_icon(&self.registry, &self.catalog, name)
    }

    /// Иконка предмета по bits его сущности (None — предмет неизвестен).
    pub fn icon(&self, bits: u64) -> Option<&RsiSprite> {
        let name = self.item_name(bits)?;
        let sprite = item_icon(&self.registry, &self.catalog, &name);
        if sprite.is_none() {
            tracing::debug!(item = %name, "no icon for item");
        }
        sprite
    }
}

/// Отпечаток состояния окна рюкзака: открыто ли, содержимое, поколение RSI.
type InventorySignature = (bool, bool, Vec<Option<u64>>, u32);

/// Перетаскиваемый предмет (SS14: ЛКМ по предмету — тащишь, отпускаешь над клеткой).
#[derive(Resource, Default)]
pub struct DragItem {
    /// bits предмета (0 — ничего не тащим).
    pub item: u64,
    /// Клетка-источник, чтобы не перекладывать предмет в ту же клетку.
    pub from_slot: u8,
    /// Тащим из рюкзака игрока (иначе — из открытого ящика).
    pub from_backpack: bool,
    /// Взято из руки (тогда отпускание вне окон роняет предмет на пол, а не
    /// пытается убрать его в рюкзак) — SS14 позволяет тащить из рук.
    pub from_hand: bool,
    /// Взято из слота экипировки: снимаем вещь перетаскиванием (в SS14 снятие —
    /// клик по слоту, но перетаскивание тоже ожидаемо; сервер принимает Unequip).
    pub from_equip: Option<ssr_core::clothing::ClothingSlot>,
    /// Смещение курсора для спрайта-призрака.
    pub grabbed: bool,
}

/// Спрайт-призрак перетаскиваемого предмета (следует за курсором).
#[derive(Component)]
pub struct DragGhost;

/// Открыто ли окно рюкзака (кнопка-сумка в панели рук открывает/закрывает).
#[derive(Resource)]
pub struct InventoryUi {
    /// Окно рюкзака (хранилище; открывается по V, как `OpenBackpack` в SS14).
    pub open: bool,
    /// Окно персонажа со слотами одежды (`InventoryGui`).
    pub character_open: bool,
}

impl Default for InventoryUi {
    fn default() -> Self {
        Self {
            open: true,
            character_open: false,
        }
    }
}

/// Кнопка-сумка в панели рук (открывает рюкзак, как слоты сумок в SS14).
#[derive(Component)]
pub struct BackpackButton;

/// Красный крестик в шапке окна рюкзака (как кнопка закрытия в SS14).
#[derive(Component)]
pub struct InventoryCloseButton;

/// Клики по кнопкам панелей (сумка и крестик рюкзака).
type PanelClicks<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        &'static mut BackgroundColor,
        Has<InventoryCloseButton>,
    ),
    (
        Changed<Interaction>,
        With<Button>,
        Or<(With<BackpackButton>, With<InventoryCloseButton>)>,
    ),
>;

/// Клики по кнопкам панелей: сумка открывает/закрывает рюкзак, крестик — закрывает.
pub fn panel_buttons_click(mut ui: ResMut<InventoryUi>, mut clicks: PanelClicks) {
    for (interaction, mut color, close) in clicks.iter_mut() {
        *color = if *interaction == Interaction::Hovered {
            BackgroundColor(Color::srgba(0.30, 0.30, 0.38, 0.55))
        } else {
            BackgroundColor(Color::NONE)
        };
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Крестик закрывает окно, а кнопка `Slots/toggle` открывает окно
        // персонажа со слотами одежды (как `InventoryButton` в SS14).
        if close {
            ui.open = false;
            ui.character_open = false;
        } else {
            ui.character_open = !ui.character_open;
        }
        tracing::info!(character = ui.character_open, "character window toggled");
    }
}

/// Картинка/подпись на всю площадь клетки или предмета (задел для панелей).
#[allow(dead_code)]
pub fn fill_node() -> Node {
    Node {
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    }
}

/// Иконка предмета в сетке (атлас RSI) — размер на всю клетку/предмет.
pub(crate) fn icon_node(sprite: &RsiSprite) -> ImageNode {
    let mut image = ImageNode::new(sprite.image.clone());
    image.texture_atlas = Some(TextureAtlas {
        layout: sprite.layout.clone(),
        index: sprite.index(0, 0),
    });
    image
}

/// Пересобирает панель рюкзака при изменениях содержимого (тетрис-сетка).
#[allow(clippy::too_many_arguments)]
pub fn render_inventory_panel(
    mut commands: Commands,
    sprites: ItemSprites,
    theme: Res<crate::ui_theme::UiTheme>,
    ui: Res<InventoryUi>,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    clothings: Query<&ssr_core::clothing::Clothing>,
    positions: Res<windows::WindowPositions>,
    root: Query<Entity, With<InventoryPanel>>,
    mut last: Local<Option<InventorySignature>>,
) {
    use crate::ui_theme as ui;
    // Правило владельца (как в SS14): инвентарь — это рюкзак; без него окна нет.
    let has_backpack = own
        .0
        .and_then(|entity| clothings.get(entity).ok())
        .is_some_and(|clothing| clothing.has_backpack());
    let signature = (
        ui_open(&ui),
        has_backpack,
        own_inventory(&own, &inventories)
            .map(|inventory| inventory.cells.clone())
            .unwrap_or_default(),
        sprites.generation(),
    );
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    if !ui_open(&ui) || !has_backpack {
        return;
    }
    let Some(inventory) = own_inventory(&own, &inventories) else {
        return;
    };

    // Окно хранилища SS14: без заголовка (cvar по умолчанию выключен), слева
    // сайдбар с красным крестом, справа сетка ячеек 32×32 без зазоров.
    let mut node = Node {
        position_type: PositionType::Absolute,
        left: px(760),
        bottom: px(150),
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Start,
        ..default()
    };
    windows::apply_saved_position(windows::WindowKind::Inventory, &mut node, &positions);

    let grid_width = INVENTORY_COLS as f32 * ui::STORAGE_CELL;
    let grid_height = INVENTORY_ROWS as f32 * ui::STORAGE_CELL;
    commands
        .spawn((
            InventoryPanel,
            windows::WindowKind::Inventory,
            windows::WindowDrag::default(),
            Interaction::default(),
            node,
        ))
        .with_children(|window| {
            // Сайдбар: крестик сверху, затем сегменты — как StorageWindow.cs.
            window
                .spawn(Node {
                    width: px(ui::STORAGE_CELL),
                    height: px(grid_height),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|sidebar| {
                    sidebar.spawn((
                        InventoryCloseButton,
                        Button,
                        crate::ui_theme::stretched(&theme.storage_exit),
                        Node {
                            width: px(ui::STORAGE_CELL),
                            height: px(ui::STORAGE_CELL),
                            ..default()
                        },
                    ));
                    let rows = (grid_height / ui::STORAGE_CELL).round() as usize;
                    for row in 1..rows {
                        let index = if row == rows - 1 { 2 } else { 1 };
                        let handle = theme
                            .storage_sidebar_segments
                            .get(index)
                            .cloned()
                            .unwrap_or_default();
                        sidebar.spawn((
                            crate::ui_theme::stretched(&handle),
                            Node {
                                width: px(ui::STORAGE_CELL),
                                height: px(ui::STORAGE_CELL),
                                ..default()
                            },
                        ));
                    }
                });
            // Сетка: ячейки вплотную, фон #222222 (StorageWindow.cs).
            window
                .spawn((
                    Node {
                        width: px(grid_width),
                        height: px(grid_height),
                        ..default()
                    },
                    BackgroundColor(ui::GRID_BACKGROUND),
                ))
                .with_children(|grid| {
                    for index in 0..(INVENTORY_COLS * INVENTORY_ROWS) {
                        let x = index % INVENTORY_COLS;
                        let y = index / INVENTORY_COLS;
                        let mut tile = crate::ui_theme::stretched(&theme.storage_tile);
                        // Текстура светлая: затемняем её модуляцией #222222,
                        // как таблицу сетки в StorageWindow.cs.
                        tile.color = ui::GRID_BACKGROUND;
                        grid.spawn((
                            InvSlot(index),
                            Button,
                            storage_cell_node(x, y),
                            tile,
                            // Фон нужен подсветке переноса (`drag_highlight`):
                            // по умолчанию прозрачный.
                            BackgroundColor(Color::NONE),
                        ));
                    }
                    // Предметы: рамка по футпринту (`Storage/piece_*`) и спрайт ×2
                    // по центру занятой площади (ItemGridPiece.cs).
                    for (bits, x, y, w, h) in ssr_core::inventory::item_layout(&inventory.cells) {
                        let Some(sprite) = sprites.icon(bits) else {
                            continue;
                        };
                        let mut frame = crate::ui_theme::stretched(&theme.storage_tile);
                        frame.color = ui::GRID_BACKGROUND;
                        grid.spawn((
                            InvSlot(y * INVENTORY_COLS + x),
                            Button,
                            storage_item_node(x, y, w, h),
                            BackgroundColor(ui::GLASS_BUTTON),
                        ))
                        .with_children(|piece| {
                            // Рамка формы (`ItemGridPiece.cs` сборки): клетка
                            // рисует ЧЕТЫРЕ кусочка по 16×16 (полклетки), каждый
                            // выбирается по своим соседям — есть ли за краем ещё
                            // клетка формы. Текстуры не режутся: каждый кусочек
                            // (8×8 px) растягивается в свою четверть.
                            let pick = |edge_y: bool,
                                        edge_x: bool,
                                        corner: u8,
                                        along_y: u8,
                                        along_x: u8| {
                                if edge_y && edge_x {
                                    corner as usize
                                } else if edge_y {
                                    along_y as usize
                                } else if edge_x {
                                    along_x as usize
                                } else {
                                    0 // центр: соседи со всех сторон
                                }
                            };
                            for dy in 0..h {
                                for dx in 0..w {
                                    let top = dy == 0;
                                    let bottom = dy + 1 == h;
                                    let left = dx == 0;
                                    let right = dx + 1 == w;
                                    // (четверть, «нет соседа» по Y, «нет соседа» по X,
                                    //  угловой кусочек, кусочек по Y, кусочек по X)
                                    let quarters = [
                                        ((0.0f32, 0.0f32), top, left, 5u8, 1u8, 3u8),
                                        ((16.0, 0.0), top, right, 6, 1, 4),
                                        ((0.0, 16.0), bottom, left, 7, 2, 3),
                                        ((16.0, 16.0), bottom, right, 8, 2, 4),
                                    ];
                                    for ((qx, qy), edge_y, edge_x, corner, along_y, along_x) in
                                        quarters
                                    {
                                        let index = pick(edge_y, edge_x, corner, along_y, along_x);
                                        let handle = theme
                                            .storage_pieces
                                            .get(index)
                                            .cloned()
                                            .unwrap_or_default();
                                        piece.spawn((
                                            crate::ui_theme::stretched(&handle),
                                            Node {
                                                position_type: PositionType::Absolute,
                                                left: px(dx as f32 * ui::STORAGE_CELL + qx),
                                                top: px(dy as f32 * ui::STORAGE_CELL + qy),
                                                width: px(16.0),
                                                height: px(16.0),
                                                ..default()
                                            },
                                        ));
                                    }
                                }
                            }
                            // Иконка ×2 (64 px) по центру формы — как `ItemGridPiece.cs`
                            // в сборке: спрайт рисуется с `TextureScale = 2` и выходит
                            // за клетку, поэтому предметы выглядят крупными.
                            let icon_size = ui::STORAGE_CELL * 2.0;
                            piece.spawn((
                                icon_node(sprite),
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: px((w as f32 * ui::STORAGE_CELL - icon_size) * 0.5),
                                    top: px((h as f32 * ui::STORAGE_CELL - icon_size) * 0.5),
                                    width: px(icon_size),
                                    height: px(icon_size),
                                    ..default()
                                },
                            ));
                        });
                    }
                });
        });
}

/// Ячейка сетки хранилища: без зазоров, сторона [`STORAGE_CELL`].
fn storage_cell_node(x: u8, y: u8) -> Node {
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

/// Предмет в сетке хранилища: w×h ячеек с общей иконкой.
fn storage_item_node(x: u8, y: u8, w: u8, h: u8) -> Node {
    let cell = crate::ui_theme::STORAGE_CELL;
    let mut node = storage_cell_node(x, y);
    node.width = px(w as f32 * cell);
    node.height = px(h as f32 * cell);
    node
}

/// Пересобирает панель рук при изменениях — по образцу SS14 (`HotbarGui.xaml`):
/// справа панель статуса руки, слоты 64×64 с текстурами `Slots/hand_l/r`,
/// предмет изображается прямо в слоте, активная рука подсвечивается рамкой
/// `Slots/slot_highlight`; справа снизу — кнопка окна рюкзака (`Slots/toggle`).
/// Отпечаток панели рук: руки, окно рюкзака и надетая одежда.
type HandsSignature = (Hands, bool, Vec<(ssr_core::clothing::ClothingSlot, u64)>);

#[allow(clippy::too_many_arguments)]
pub fn render_hands_panel(
    mut commands: Commands,
    sprites: ItemSprites,
    theme: Res<crate::ui_theme::UiTheme>,
    ui: Res<InventoryUi>,
    own: Res<OwnPlayerEntity>,
    hands: Query<&Hands>,
    clothings: Query<&ssr_core::clothing::Clothing>,
    positions: Res<windows::WindowPositions>,
    root: Query<Entity, With<HandsPanel>>,
    mut last: Local<Option<HandsSignature>>,
) {
    let Some(own_state) = own_hands(&own, &hands).cloned() else {
        return;
    };
    let clothing_signature = own
        .0
        .and_then(|entity| clothings.get(entity).ok())
        .map(|clothing| clothing.slots.clone())
        .unwrap_or_default();
    let signature = (own_state.clone(), ui_open(&ui), clothing_signature);
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);
    tracing::info!(active = own_state.active, "hands updated");

    for entity in root.iter() {
        commands.entity(entity).despawn();
    }

    // В SS14 панель рук привязана снизу по всей ширине с отступом 5
    // (`SetAnchorAndMarginPreset(Hotbar, BottomWide, margin: 5)`).
    let mut node = Node {
        position_type: PositionType::Absolute,
        left: px(0),
        right: px(0),
        bottom: px(5),
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::Center,
        align_items: AlignItems::FlexEnd,
        column_gap: px(0),
        ..default()
    };
    windows::apply_saved_position(windows::WindowKind::Hands, &mut node, &positions);

    let item_of = |hand: u8| own_state.slots.get(hand as usize).copied().flatten();
    commands
        .spawn((
            HandsPanel,
            windows::WindowKind::Hands,
            windows::WindowDrag::default(),
            Interaction::default(),
            node,
        ))
        .with_children(|bar| {
            // Порядок строго как в HotbarGui.xaml (слева направо):
            // SecondHotbar (id, belt, back) → status → руки → status → MainHotbar
            // (suitstorage, pocket1, pocket2). В руках правая стоит слева.
            let clothing = own
                .0
                .and_then(|entity| clothings.get(entity).ok())
                .cloned()
                .unwrap_or_default();
            for slot in [
                ssr_core::clothing::ClothingSlot::Id,
                ssr_core::clothing::ClothingSlot::Belt,
                ssr_core::clothing::ClothingSlot::Back,
            ] {
                equip_slot(bar, &theme, &sprites, slot, &clothing);
            }
            status_panel(
                bar,
                &theme,
                &sprites,
                item_of(0),
                false,
                own_state.active == 0,
            );
            hand_slot(bar, &theme, &sprites, 0, &own_state);
            hand_slot(bar, &theme, &sprites, 1, &own_state);
            status_panel(
                bar,
                &theme,
                &sprites,
                item_of(1),
                true,
                own_state.active == 1,
            );
            // MainHotbar (`MainHotbar` в SS14): карманы и разгрузка справа.
            // id/belt/back уже отрисованы слева выше — второй раз их не строим
            // (был дубликат справа).
            for slot in [
                ssr_core::clothing::ClothingSlot::SuitStorage,
                ssr_core::clothing::ClothingSlot::Pocket1,
                ssr_core::clothing::ClothingSlot::Pocket2,
            ] {
                equip_slot(bar, &theme, &sprites, slot, &clothing);
            }
        });
}

/// Панель статуса руки (`ItemStatusPanel`): имя предмета или «В руке пусто».
/// Сторона задаёт текстуру и patch margin: у левой панели скос слева (наружу),
/// у правой — справа (как в `ItemStatusPanel.xaml.cs`).
#[allow(clippy::too_many_arguments)]
fn status_panel(
    bar: &mut ChildSpawnerCommands,
    theme: &crate::ui_theme::UiTheme,
    sprites: &ItemSprites,
    item: Option<u64>,
    right: bool,
    active: bool,
) {
    use crate::ui_theme as ui;
    // Панель справа от рук берёт item_status_left, слева — item_status_right.
    let (texture, left, right_margin) = if right {
        (&theme.status_left, 8.0, 14.0)
    } else {
        (&theme.status_right, 14.0, 8.0)
    };
    // Патчи из темы (`_itemstatus_patch_margin = "#07060404"` при масштабе ×2)
    // и отступы текста: `Margin(4,0,0,2)`, шрифт 10.
    let mut image = crate::ui_theme::nine_slice_rect(texture, left, right_margin, 12.0, 8.0);
    image.color = Color::WHITE;
    bar.spawn((
        image,
        Node {
            width: px(ui::STATUS_WIDTH),
            height: px(ui::STATUS_HEIGHT),
            align_items: AlignItems::Center,
            padding: UiRect::new(px(6), px(6), px(6), px(4)),
            overflow: Overflow::clip(),
            ..default()
        },
    ))
    .with_children(|panel| {
        // Подсветка панели активной руки (`item_status_*_highlight`).
        if active && let Some(handle) = theme.status_highlights.get(if right { 1 } else { 0 }) {
            let mut highlight =
                crate::ui_theme::nine_slice_rect(handle, left, right_margin, 12.0, 8.0);
            highlight.image_mode = NodeImageMode::Sliced(TextureSlicer::default());
            highlight.image_mode = NodeImageMode::Sliced(TextureSlicer {
                border: BorderRect {
                    min_inset: Vec2::new(left, 12.0),
                    max_inset: Vec2::new(right_margin, 8.0),
                },
                center_scale_mode: SliceScaleMode::Stretch,
                sides_scale_mode: SliceScaleMode::Stretch,
                max_corner_scale: 1.0,
            });
            panel.spawn((
                highlight,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    width: px(ui::STATUS_WIDTH),
                    height: px(ui::STATUS_HEIGHT),
                    ..default()
                },
            ));
        }
        match item.and_then(|bits| sprites.display_name(bits)) {
            Some(name) => {
                panel.spawn((
                    Text::new(name),
                    TextFont::from_font_size(ui::FONT_SMALL),
                    TextColor(ui::TEXT),
                    Node {
                        margin: UiRect::new(px(4), px(0), px(0), px(2)),
                        ..default()
                    },
                ));
            }
            None => {
                panel.spawn((
                    Text::new("В руке пусто"),
                    TextFont::from_font_size(ui::FONT_SMALL),
                    TextColor(ui::TEXT_MUTED),
                    Node {
                        margin: UiRect::new(px(4), px(0), px(0), px(2)),
                        ..default()
                    },
                ));
            }
        }
    });
}

/// Слот руки: текстура SS14 64×64, предмет внутри, подсветка активной руки.
fn hand_slot(
    bar: &mut ChildSpawnerCommands,
    theme: &crate::ui_theme::UiTheme,
    sprites: &ItemSprites,
    hand: u8,
    hands: &Hands,
) {
    use crate::ui_theme as ui;
    let item = hands.slots.get(hand as usize).copied().flatten();
    let active = hands.active == hand;
    // В SS14 правая рука на экране слева (`ItemStatusPanel.xaml.cs`).
    let texture = if hand == 0 {
        theme.hand_r.clone()
    } else {
        theme.hand_l.clone()
    };
    bar.spawn((
        HandSlot(hand),
        Button,
        ImageNode::new(texture),
        Node {
            width: px(ui::SLOT_SIZE),
            height: px(ui::SLOT_SIZE),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    ))
    .with_children(|slot| {
        let resolved = item.and_then(|bits| {
            let name = sprites.item_name(bits);
            let sprite = sprites.icon(bits);
            tracing::info!(hand, ?name, found = sprite.is_some(), "hand slot item");
            sprite
        });
        if let Some(sprite) = resolved {
            slot.spawn((
                icon_node(sprite),
                Node {
                    width: px(ui::SLOT_SIZE),
                    height: px(ui::SLOT_SIZE),
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    ..default()
                },
            ));
        }
        if active {
            // Рамка активной руки — `slot_highlight` поверх слота (×2 в SS14).
            let mut highlight = ImageNode::new(theme.slot_highlight.clone());
            highlight.image_mode = NodeImageMode::Stretch;
            slot.spawn((
                highlight,
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    width: px(ui::SLOT_SIZE),
                    height: px(ui::SLOT_SIZE),
                    ..default()
                },
            ));
        }
    });
}

/// Открыто ли окно рюкзака (доступ к ресурсу через `Res<InventoryUi>`).
fn ui_open(ui: &InventoryUi) -> bool {
    ui.open
}

/// Меню действий (вербов): строится из ответа сервера, позиционируется у курсора.
/// Оформление — как `ContextMenuPopup`/`ContextMenuElement` в сборке: строки по
/// 32 px, иконка 32×32 слева, шрифт по типу верба (`InteractionVerb` — жирный
/// курсив, `ActivationVerb` — жирный, `AlternativeVerb` — курсив), недоступные
/// вербы серые с причиной, категории — заголовками с отступом.
pub fn render_action_menu(
    mut commands: Commands,
    menu: Res<ActionMenu>,
    root: Query<Entity, With<ActionMenuRoot>>,
    registry: Res<crate::rsi::RsiRegistry>,
    content: Res<crate::content::ClientContent>,
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
    // В сборке меню — Popup у курсора с полем 2 px и высотой элемента 32 px;
    // свыше 10 элементов появляется прокрутка (`ContextMenuPopup.xaml.cs:23`).
    commands
        .spawn((
            ActionMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(menu.cursor.x.clamp(0.0, 1100.0)),
                top: px(menu.cursor.y.clamp(0.0, 560.0)),
                max_height: px(32.0 * 10.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(2)),
                row_gap: px(2),
                ..default()
            },
            BackgroundColor(Color::srgba(0.04, 0.04, 0.05, 0.96)),
        ))
        .with_children(|list| {
            let mut current_category: Option<ssr_core::verbs::VerbCategory> = None;
            for (index, option) in menu.options.iter().enumerate() {
                // Заголовок категории — как элемент-категория в сборке
                // (`AddVerbCategory`): иконка + текст, клик открывает подменю.
                if option.category != current_category {
                    current_category = option.category;
                    if let Some(category) = option.category {
                        list.spawn((
                            Node {
                                height: px(32),
                                padding: UiRect::axes(px(6), px(4)),
                                align_items: AlignItems::Center,
                                column_gap: px(6),
                                border: UiRect::all(px(1)),
                                ..default()
                            },
                            BackgroundColor(Color::srgb(0.10, 0.10, 0.13)),
                            BorderColor::from(Color::srgb(0.28, 0.28, 0.33)),
                        ))
                        .with_children(|row| {
                            row.spawn((
                                Text::new(category.text()),
                                TextFont::from_font_size(13.0),
                                TextColor(Color::srgb(0.85, 0.85, 0.88)),
                            ));
                        });
                    }
                }
                let indented = option.category.is_some();
                let (font_size, bold, _italic) = match option.style_class() {
                    "InteractionVerb" => (12.0, true, true),
                    "ActivationVerb" => (12.0, true, false),
                    "AlternativeVerb" => (12.0, false, true),
                    _ => (12.0, false, false),
                };
                let color = if option.disabled {
                    Color::srgb(0.45, 0.45, 0.48)
                } else {
                    Color::srgb(0.92, 0.92, 0.94)
                };
                let label = match (&option.message, option.disabled) {
                    (Some(message), true) => format!("{} ({message})", option.label),
                    _ => option.label.clone(),
                };
                let icon = option
                    .icon
                    .as_deref()
                    .and_then(|path| registry.get(&format!("sprites/ss14/{path}")))
                    .map(|sprite| {
                        (
                            sprite.image.clone(),
                            sprite.layout.clone(),
                            sprite.index(0, 0),
                        )
                    });
                list.spawn((
                    ActionMenuOption(index),
                    Button,
                    Node {
                        height: px(32),
                        min_width: px(160),
                        margin: UiRect::left(px(if indented { 8 } else { 0 })),
                        padding: UiRect::axes(px(6), px(2)),
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        border: UiRect::all(px(1)),
                        ..default()
                    },
                    BackgroundColor(if option.disabled {
                        Color::srgb(0.08, 0.08, 0.10)
                    } else {
                        Color::srgb(0.11, 0.11, 0.14)
                    }),
                    BorderColor::from(Color::srgb(0.26, 0.26, 0.31)),
                ))
                .with_children(|row| {
                    // Иконка 32×32 (`SpriteView`/`TextureRect` в сборке).
                    if let Some((image, layout, index)) = icon {
                        row.spawn((
                            ImageNode::from_atlas_image(image, TextureAtlas { layout, index }),
                            Node {
                                width: px(24),
                                height: px(24),
                                ..default()
                            },
                        ));
                    }
                    let text = Text::new(label);
                    row.spawn((
                        text,
                        TextFont::from_font_size(if bold { font_size + 0.5 } else { font_size }),
                        TextColor(color),
                    ));
                });
            }
        });
    let _ = &content;
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
        // Предмет в руке — клик по клетке кладёт его сюда (владелец: «чтобы
        // можно было из руки предмет разместить в рюкзак, ткнув по сетке»).
        // Направление в SS14 то же: с предметом в руке клик по клетке хранилища
        // вставляет его, без предмета — забирает лежащий в руку.
        if let Some(item) = own_hands(&own, &hands).and_then(|hands| hands.active_item()) {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::TransferItem {
                    item,
                    to_slot: slot.0,
                    target_player: 0,
                });
            }
            tracing::info!(item, slot = slot.0, "hand item placed into slot");
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
    items: Query<(
        Entity,
        &ssr_core::inventory::Item,
        &ssr_core::inventory::ItemPosition,
        &ssr_core::inventory::HeldBy,
    )>,
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
        // Предмет под курсором: у него свои вербы, как в SS14 (взять, осмотреть
        // и т.д.). Клетка та же, что у обводки в `doors::hover_outline`.
        const HALF: f32 = ssr_core::tiles::TILE_PX as f32 / 2.0;
        let hovered_item = items
            .iter()
            .find(|(_, _, position, held)| {
                let point = Vec2::from_array(position.0);
                held.player == 0
                    && (point.x - world.x).abs() <= HALF
                    && (point.y - world.y).abs() <= HALF
            })
            .and_then(|(entity, _, _, _)| {
                entity_map
                    .as_deref()
                    .and_then(|map| map.to_server().get(&entity))
                    .copied()
                    .map(Entity::to_bits)
            });
        if let Some(bits) = hovered_item {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::RequestActions {
                    entity: bits,
                    tx: 0,
                    ty: 0,
                });
            }
            menu.cursor = cursor;
            tracing::info!(bits, "actions requested (item)");
            return;
        }
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

    // Ctrl+ЛКМ по крупному предмету или ящику — тянуть его за собой
    // (в SS14 это Ctrl+клик по объекту: `PullMessage` из Grab-интента).
    if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
        const HALF: f32 = ssr_core::tiles::TILE_PX as f32 / 2.0;
        let target = items
            .iter()
            .find(|(_, _, position, held)| {
                let point = Vec2::from_array(position.0);
                held.player == 0
                    && (point.x - world.x).abs() <= HALF
                    && (point.y - world.y).abs() <= HALF
            })
            .map(|(entity, _, _, _)| entity)
            .or_else(|| container_under_cursor(&containers, world));
        if let Some(target) = target
            && let Some(bits) = entity_map
                .as_deref()
                .and_then(|map| map.to_server().get(&target))
                .copied()
                .map(Entity::to_bits)
        {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::PerformAction {
                    action: ssr_protocol::ActionKind::Pull { target: bits },
                });
            }
            tracing::info!(target = bits, "pull toggled (ctrl+click)");
            return;
        }
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
/// Тест-режим `SSR_UNEQUIP_TEST=<слот>`: через 4 с снимает вещь из слота
/// (например `back`) — проверка, что снятое не исчезает: `TryUnequip` в сборке
/// кладёт вещь `DropNextTo`/`PickupOrDrop`, а не в собственное хранилище.
pub fn unequip_test_mode(
    time: Res<Time>,
    mut state: Local<(f32, bool)>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    let Ok(slot) = std::env::var("SSR_UNEQUIP_TEST") else {
        return;
    };
    if state.1 {
        return;
    }
    state.0 += time.delta_secs();
    if state.0 < 4.0 {
        return;
    }
    state.1 = true;
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Unequip { slot: slot.clone() });
    }
    tracing::info!(%slot, "unequip-test: снятие отправлено");
}

/// Тест-режим SSR_INV_TEST: перенос предмета в чужой инвентарь.
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
    let Some(item) = inventory.cells.iter().flatten().copied().next() else {
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
    items: Query<&Item>,
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

    // Тетрис: позиции предметов не фиксированы — ищем по имени, берём по якорю.
    let item_named = |name: &str| -> Option<(u64, u8)> {
        let inventory = own_inventory(&own, &inventories)?;
        inventory
            .cells
            .iter()
            .flatten()
            .copied()
            .find(|bits| {
                Entity::try_from_bits(*bits)
                    .and_then(|entity| items.get(entity).ok())
                    .map(|item| item.name == name)
                    .unwrap_or(false)
            })
            .map(|bits| (bits, inventory.anchor_of(bits).unwrap_or(0)))
    };

    if !state.taken_sheet
        && state.elapsed >= 5.0
        && let Some((sheet, anchor)) = item_named("SteelSheet")
    {
        state.sheet = Some(sheet);
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TakeInHand { slot: anchor });
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
        && let Some((crowbar, anchor)) = item_named("Crowbar")
    {
        state.crowbar = Some(crowbar);
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TakeInHand { slot: anchor });
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

/// Клетки рюкзака, доступные мышью (для перетаскивания).
type BackpackSlots<'w, 's> =
    Query<'w, 's, (&'static Interaction, &'static InvSlot), (With<Button>, Without<ContainerSlot>)>;
/// Клетки открытого ящика, доступные мышью.
type CrateSlots<'w, 's> =
    Query<'w, 's, (&'static Interaction, &'static ContainerSlot), (With<Button>, Without<InvSlot>)>;

/// Подсветка клеток под переносимым предметом (PORT_PLAN 1.6): считаем форму
/// предмета от клетки под курсором и красим её зелёным `#1E8000`, если все
/// клетки свободны, иначе красным `#B40046` — цвета из `StorageWindow.FrameUpdate`
/// сборки. Клетки без подсветки — прозрачные.
#[allow(clippy::too_many_arguments)]
pub fn drag_highlight(
    drag: Res<DragItem>,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    content: Res<crate::content::ClientContent>,
    items: Query<&ssr_core::inventory::Item>,
    mut cells: Query<(&Interaction, &InvSlot, &mut BackgroundColor)>,
) {
    // Форма переносимого предмета: сначала данные сборки (явная `Item.shape`,
    // затем `Item.size`), потом наш каталог — у лома 1×2, у стали 2×2.
    let shape = (drag.item != 0).then(|| {
        items
            .get(Entity::try_from_bits(drag.item).unwrap_or(Entity::PLACEHOLDER))
            .ok()
            .map(|item| {
                content
                    .proto_shapes
                    .get(&item.name)
                    .copied()
                    .or_else(|| {
                        content
                            .proto_sizes
                            .get(&item.name)
                            .and_then(|size| ssr_core::item_size::cells_of(size))
                    })
                    .unwrap_or_else(|| content.items.size_of(&item.name))
            })
    });
    // Клетка под курсором — через `Interaction` (та же механика, что у переноса).
    let hovered = cells
        .iter()
        .find(|(interaction, _, _)| **interaction == Interaction::Hovered)
        .map(|(_, slot, _)| slot.0);
    let shape = shape.flatten();
    let fits = match (shape, hovered) {
        (Some((w, h)), Some(index)) => {
            let (x, y) = (index % INVENTORY_COLS, index / INVENTORY_COLS);
            if x + w > INVENTORY_COLS || y + h > INVENTORY_ROWS {
                Some(false)
            } else {
                let inventory = own_inventory(&own, &inventories);
                let free = (0..h).all(|dy| {
                    (0..w).all(|dx| {
                        let cell = ((y + dy) * INVENTORY_COLS + (x + dx)) as usize;
                        inventory
                            .and_then(|inv| inv.cells.get(cell).copied().flatten())
                            .is_none()
                    })
                });
                Some(free)
            }
        }
        _ => None,
    };
    for (_, slot, mut color) in cells.iter_mut() {
        let target = match (fits, hovered, shape) {
            (Some(free), Some(index), Some((w, h))) => {
                let (x, y) = (index % INVENTORY_COLS, index / INVENTORY_COLS);
                let (cx, cy) = (slot.0 % INVENTORY_COLS, slot.0 / INVENTORY_COLS);
                let inside = cx >= x && cx < x + w && cy >= y && cy < y + h;
                if inside {
                    if free {
                        Color::srgb_u8(0x1e, 0x80, 0x00)
                    } else {
                        Color::srgb_u8(0xb4, 0x00, 0x46)
                    }
                } else {
                    Color::NONE
                }
            }
            _ => Color::NONE,
        };
        if color.0 != target {
            color.0 = target;
        }
    }
}

/// Начало перетаскивания: ЛКМ по занятой клетке рюкзака, ящика или по руке.
#[allow(clippy::too_many_arguments)]
pub fn drag_start(
    mouse: Res<ButtonInput<MouseButton>>,
    mut drag: ResMut<DragItem>,
    slots: BackpackSlots,
    hand_slots: Query<(&'static HandSlot, &'static Interaction), With<Button>>,
    equip_slots: Query<(&'static Interaction, &'static EquipSlotButton), With<Button>>,
    clothings: Query<&ssr_core::clothing::Clothing>,
    container_slots: CrateSlots,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    hands: Query<&ssr_core::inventory::Hands>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    drag.item = 0;
    drag.from_hand = false;
    // Руки: в SS14 предмет тащат и из руки (без этого вернуть вещь из руки в
    // рюкзак было нечем — владелец: «нельзя положить предмет обратно»).
    drag.from_equip = None;
    // Слот экипировки: в SS14 снятие — клик, но перетаскивание вещи из слота
    // (например, в рюкзак) ожидаемо и поддержано сервером (Unequip).
    if let Some(player) = own.0
        && let Ok(clothing) = clothings.get(player)
    {
        for (interaction, button) in equip_slots.iter() {
            if *interaction != Interaction::Pressed {
                continue;
            }
            if let Some(item) = clothing.get(button.0) {
                drag.item = item;
                drag.from_backpack = false;
                drag.from_hand = false;
                drag.from_equip = Some(button.0);
                drag.grabbed = false;
                tracing::info!(item, slot = button.0.id(), "drag started (equipped)");
            }
        }
    }
    if let Some(player) = own.0
        && let Ok(hands) = hands.get(player)
    {
        for (hand, interaction) in hand_slots.iter() {
            if *interaction != Interaction::Pressed {
                continue;
            }
            if let Some(item) = hands.slots.get(hand.0 as usize).copied().flatten() {
                drag.item = item;
                drag.from_slot = hand.0;
                drag.from_backpack = false;
                drag.from_hand = true;
                drag.grabbed = false;
                tracing::info!(item, hand = hand.0, "drag started (hand)");
            }
        }
    }
    // Рюкзак: предмет, лежащий в клетке под курсором.
    if let Some(inventory) = own_inventory(&own, &inventories) {
        for (interaction, slot) in slots.iter() {
            if *interaction != Interaction::Pressed {
                continue;
            }
            if let Some(item) = inventory.cells.get(slot.0 as usize).copied().flatten() {
                drag.item = item;
                drag.from_slot = inventory.anchor_of(item).unwrap_or(slot.0);
                drag.from_backpack = true;
                drag.grabbed = false;
                tracing::info!(item, slot = slot.0, "drag started (backpack)");
            }
        }
    }
    // Ящик: предмет из открытого контейнера.
    for (interaction, slot) in container_slots.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Ok(inventory) = inventories.get(slot.container) else {
            continue;
        };
        if let Some(item) = inventory.cells.get(slot.slot as usize).copied().flatten() {
            drag.item = item;
            drag.from_slot = inventory.anchor_of(item).unwrap_or(slot.slot);
            drag.from_backpack = false;
            drag.grabbed = false;
            tracing::info!(item, slot = slot.slot, "drag started (container)");
        }
    }
}

/// Спрайт-призрак предмета следует за курсором, пока идёт перетаскивание.
pub fn drag_ghost(
    mut commands: Commands,
    drag: Res<DragItem>,
    sprites: ItemSprites,
    windows: Query<&Window>,
    ghosts: Query<Entity, With<DragGhost>>,
) {
    let mut existing = ghosts.iter();
    let current = existing.next();
    if drag.item == 0 {
        if let Some(entity) = current {
            commands.entity(entity).despawn();
        }
        return;
    }
    let Some(sprite) = sprites.icon(drag.item) else {
        return;
    };
    let cursor = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position());
    let Some(cursor) = cursor else {
        return;
    };
    let mut image = icon_node(sprite);
    image.color = Color::srgba(1.0, 1.0, 1.0, 0.75);
    let node = Node {
        position_type: PositionType::Absolute,
        left: px(cursor.x - 16.0),
        top: px(cursor.y - 16.0),
        width: px(32),
        height: px(32),
        ..default()
    };
    match current {
        Some(entity) => {
            commands.entity(entity).insert((image, node));
        }
        None => {
            commands.spawn((DragGhost, image, node));
        }
    }
}

/// Отпускание ЛКМ: перенос предмета в клетку под курсором или на пол.
#[allow(clippy::too_many_arguments)]
pub fn drag_release(
    mouse: Res<ButtonInput<MouseButton>>,
    mut drag: ResMut<DragItem>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    slots: BackpackSlots,
    container_slots: CrateSlots,
    equip_targets: EquipSlotTargets,
    ui: Query<&Interaction, With<Button>>,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    crates: Query<(
        Entity,
        &ssr_core::inventory::Container,
        &ssr_core::inventory::ItemPosition,
    )>,
) {
    if !mouse.just_released(MouseButton::Left) || drag.item == 0 {
        return;
    }
    let item = drag.item;
    let from_slot = drag.from_slot;
    let from_backpack = drag.from_backpack;
    let drag_hand = drag.from_hand;
    let from_equip = drag.from_equip.take();
    drag.item = 0;
    // Вещь со слота экипировки: любое отпускание — снять (сервер положит в рюкзак).
    if let Some(slot) = from_equip {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Unequip {
                slot: slot.id().to_string(),
            });
        }
        tracing::info!(slot = slot.id(), "drag: unequipped");
        return;
    }
    // Клетка рюкзака под курсором.
    for (interaction, slot) in slots.iter() {
        if *interaction != Interaction::Hovered {
            continue;
        }
        if from_backpack && slot.0 == from_slot {
            return;
        }
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TransferItem {
                item,
                to_slot: slot.0,
                target_player: 0,
            });
        }
        tracing::info!(item, to_slot = slot.0, "drag: item moved in backpack");
        return;
    }
    // Ящик в мире под курсором: в сборке это `EntityStorage` — предмет просто
    // перетаскивают в ящик, и он там лежит (сеточного окна у ящика нет).
    let cursor_world = windows
        .iter()
        .next()
        .and_then(|window| window.cursor_position())
        .and_then(|cursor| {
            let (camera, transform) = *camera;
            camera.viewport_to_world_2d(transform, cursor).ok()
        });
    if let Some(point) = cursor_world {
        const HALF: f32 = ssr_core::tiles::TILE_PX as f32 / 2.0;
        let hit = crates.iter().find(|(_, _, position)| {
            let crate_point = Vec2::from_array(position.0);
            (crate_point.x - point.x).abs() <= HALF && (crate_point.y - point.y).abs() <= HALF
        });
        if let Some((crate_entity, _, _)) = hit {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::TransferItem {
                    item,
                    to_slot: SLOT_ANY,
                    target_player: crate_entity.to_bits(),
                });
            }
            tracing::info!(item, ?crate_entity, "drag: item put into crate");
            return;
        }
    }

    // Слот экипировки под курсором: надеваем перетаскиванием, как в SS14.
    for (interaction, slot) in equip_targets.iter() {
        if *interaction != Interaction::Hovered {
            continue;
        }
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Equip {
                item,
                slot: slot.0.id().to_string(),
            });
        }
        tracing::info!(item, slot = slot.0.id(), "drag: item equipped");
        return;
    }
    // Клетка ящика под курсором.
    for (interaction, slot) in container_slots.iter() {
        if *interaction != Interaction::Hovered {
            continue;
        }
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TransferItem {
                item,
                to_slot: slot.slot,
                target_player: slot.container.to_bits(),
            });
        }
        tracing::info!(item, slot = slot.slot, "drag: item moved into container");
        return;
    }
    // Отпустили вне окон: из рюкзака — на пол (как перетаскивание в мир в SS14),
    // из ящика — в свой рюкзак.
    let cursor_on_ui = ui
        .iter()
        .any(|interaction| *interaction != Interaction::None);
    if cursor_on_ui {
        tracing::info!(item, "drag cancelled over ui");
        return;
    }
    if from_backpack || drag_hand {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::DropItem { item });
        }
        tracing::info!(item, from_hand = drag_hand, "drag: item thrown on floor");
    } else {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TransferItem {
                item,
                to_slot: SLOT_ANY,
                target_player: 0,
            });
        }
        tracing::info!(item, "drag: item taken from container");
    }
}

/// Тест-режим SSR_DRAG_TEST=1: берёт первый предмет рюкзака «в руку» перетаскивания
/// (виден призрак), затем кладёт его в открытый ящик — проверка обеих веток.
pub fn drag_test_mode(
    time: Res<Time>,
    mut state: Local<(f32, u8)>,
    mut drag: ResMut<DragItem>,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    containers: Query<(Entity, &Container)>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if std::env::var_os("SSR_DRAG_TEST").is_none() {
        return;
    }
    state.0 += time.delta_secs();
    let elapsed = state.0;
    if state.1 == 0 && elapsed >= 4.0 {
        let Some(inventory) = own_inventory(&own, &inventories) else {
            return;
        };
        let Some(item) = inventory.cells.iter().flatten().copied().next() else {
            tracing::warn!("drag-test: inventory empty");
            return;
        };
        state.1 = 1;
        drag.item = item;
        drag.from_slot = inventory.anchor_of(item).unwrap_or(0);
        drag.from_backpack = true;
        tracing::info!(item, "drag-test: ghost shown");
    }
    if state.1 == 1 && elapsed >= 7.0 {
        state.1 = 2;
        let item = drag.item;
        drag.item = 0;
        let Some((container, _)) = containers.iter().next() else {
            tracing::warn!("drag-test: no container");
            return;
        };
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::TransferItem {
                item,
                to_slot: SLOT_ANY,
                target_player: container.to_bits(),
            });
        }
        tracing::info!(item, "drag-test: transferred to container");
    }
}

/// Слот экипировки в хотбаре: текстура `Slots/*` из сборки, при надетом
/// предмете — иконка поверх, клик — снять (SS14: клик по слоту снимает вещь).
fn equip_slot(
    bar: &mut ChildSpawnerCommands,
    theme: &crate::ui_theme::UiTheme,
    sprites: &ItemSprites,
    slot: ssr_core::clothing::ClothingSlot,
    clothing: &ssr_core::clothing::Clothing,
) {
    use crate::ui_theme as ui;
    let texture = match slot {
        ssr_core::clothing::ClothingSlot::Back => theme.slot_back.clone(),
        ssr_core::clothing::ClothingSlot::Belt => theme.slot_belt.clone(),
        ssr_core::clothing::ClothingSlot::Id => theme.slot_id.clone(),
        ssr_core::clothing::ClothingSlot::SuitStorage => theme.slot_suit_storage.clone(),
        _ => theme.slot_pocket.clone(),
    };
    let item = clothing.get(slot);
    bar.spawn((
        EquipSlotButton(slot),
        Button,
        ImageNode::new(texture),
        Node {
            width: px(ui::SLOT_SIZE),
            height: px(ui::SLOT_SIZE),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        },
    ))
    .with_children(|cell| {
        if let Some(sprite) = item.and_then(|bits| sprites.icon(bits)) {
            cell.spawn((
                icon_node(sprite),
                Node {
                    width: px(ui::SLOT_SIZE),
                    height: px(ui::SLOT_SIZE),
                    ..default()
                },
            ));
        }
    });
}

/// Слоты экипировки под курсором (для перетаскивания).
type EquipSlotTargets<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static EquipSlotButton),
    (With<Button>, Without<InvSlot>),
>;

/// Отпечаток окна персонажа: открыто ли и что надето.
type CharacterSignature = (bool, Vec<(ssr_core::clothing::ClothingSlot, u64)>);

type EquipSlotClicks<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static EquipSlotButton),
    (Changed<Interaction>, With<Button>),
>;

/// Клик по слоту экипировки: занят — снять вещь, пуст и в руке вещь — надеть.
pub fn equip_slot_click(
    buttons: EquipSlotClicks,
    own: Res<OwnPlayerEntity>,
    clothings: Query<&ssr_core::clothing::Clothing>,
    hands: Query<&Hands>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    for (interaction, button) in buttons.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let clothing = own.0.and_then(|entity| clothings.get(entity).ok());
        if let Some(item) = clothing.and_then(|clothing| clothing.get(button.0)) {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::Unequip {
                    slot: button.0.id().to_string(),
                });
            }
            tracing::info!(slot = button.0.id(), item, "unequip sent");
            continue;
        }
        // Пустой слот: кладём/надеваем предмет из активной руки.
        let item = own
            .0
            .and_then(|entity| hands.get(entity).ok())
            .and_then(|hands| hands.active_item());
        if let Some(item) = item {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::Equip {
                    item,
                    slot: button.0.id().to_string(),
                });
            }
            tracing::info!(slot = button.0.id(), item, "equip sent");
        }
    }
}

/// Кнопка слота экипировки.
#[derive(Component)]
pub struct EquipSlotButton(pub ssr_core::clothing::ClothingSlot);

/// Корень окна персонажа со слотами одежды (`InventoryGui` в SS14).
#[derive(Component)]
pub struct CharacterPanel;

/// Рисует окно персонажа: девять слотов одежды в сетке 3 колонки — ровно как
/// раскладывает `uiWindowPos` в `human_inventory_template.yml`
/// (обувь 1,0; комбинезон 0,1; куртка 1,1; перчатки 2,1; шея 0,2; маска 1,2;
/// уши 2,2; очки 0,3; голова 1,3).
#[allow(clippy::too_many_arguments)]
pub fn render_character_panel(
    mut commands: Commands,
    sprites: ItemSprites,
    theme: Res<crate::ui_theme::UiTheme>,
    ui: Res<InventoryUi>,
    own: Res<OwnPlayerEntity>,
    clothings: Query<&ssr_core::clothing::Clothing>,
    positions: Res<windows::WindowPositions>,
    root: Query<Entity, With<CharacterPanel>>,
    mut last: Local<Option<CharacterSignature>>,
) {
    use crate::ui_theme as ui;
    use ssr_core::clothing::ClothingSlot as Slot;
    let clothing = own
        .0
        .and_then(|entity| clothings.get(entity).ok())
        .cloned()
        .unwrap_or_default();
    let signature = (ui.character_open, clothing.slots.clone());
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    if !ui.character_open {
        return;
    }
    // (слот, колонка, строка) — из uiWindowPos шаблона инвентаря человека.
    let cell = ui::SLOT_SIZE;
    let mut node = Node {
        position_type: PositionType::Absolute,
        left: px(5),
        bottom: px(5),
        width: px(cell * 3.0),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::End,
        ..default()
    };
    windows::apply_saved_position(windows::WindowKind::Character, &mut node, &positions);
    let layout: [(Slot, u8, u8); 9] = [
        (Slot::Shoes, 1, 0),
        (Slot::Jumpsuit, 0, 1),
        (Slot::OuterClothing, 1, 1),
        (Slot::Gloves, 2, 1),
        (Slot::Neck, 0, 2),
        (Slot::Mask, 1, 2),
        (Slot::Ears, 2, 2),
        (Slot::Eyes, 0, 3),
        (Slot::Head, 1, 3),
    ];
    commands
        .spawn((
            CharacterPanel,
            windows::WindowKind::Character,
            windows::WindowDrag::default(),
            Interaction::default(),
            node,
        ))
        .with_children(|window| {
            // Сетка слотов: три колонки по uiWindowPos шаблона человека.
            window
                .spawn((
                    Node {
                        width: px(cell * 3.0),
                        height: px(cell * 4.0),
                        ..default()
                    },
                    BackgroundColor(ui::GLASS_PANEL),
                ))
                .with_children(|grid| {
                    for (slot, column, row) in layout {
                        let index = CHARACTER_SLOT_ORDER
                            .iter()
                            .position(|candidate| *candidate == slot)
                            .unwrap_or(0);
                        let texture = theme
                            .character_slots
                            .get(index)
                            .cloned()
                            .unwrap_or_default();
                        let item = clothing.get(slot);
                        grid.spawn((
                            EquipSlotButton(slot),
                            Button,
                            ImageNode::new(texture),
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(column as f32 * cell),
                                top: px(row as f32 * cell),
                                width: px(cell),
                                height: px(cell),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                        ))
                        .with_children(|cell_node| {
                            if let Some(sprite) = item.and_then(|bits| sprites.icon(bits)) {
                                cell_node.spawn((
                                    icon_node(sprite),
                                    Node {
                                        width: px(cell),
                                        height: px(cell),
                                        ..default()
                                    },
                                ));
                            }
                        });
                    }
                });
            window.spawn((
                BackpackButton,
                Button,
                crate::hud::IconTint {
                    normal: Color::WHITE,
                    hovered: Color::srgb(0.92, 0.92, 0.96),
                    pressed: Color::srgb(0.85, 0.85, 0.9),
                },
                ImageNode::new(theme.slot_toggle.clone()),
                Node {
                    width: px(cell),
                    height: px(cell),
                    ..default()
                },
            ));
        });
}

/// Порядок текстур в [`crate::ui_theme::CHARACTER_SLOTS`].
const CHARACTER_SLOT_ORDER: [ssr_core::clothing::ClothingSlot; 9] = [
    ssr_core::clothing::ClothingSlot::Head,
    ssr_core::clothing::ClothingSlot::Jumpsuit,
    ssr_core::clothing::ClothingSlot::OuterClothing,
    ssr_core::clothing::ClothingSlot::Gloves,
    ssr_core::clothing::ClothingSlot::Neck,
    ssr_core::clothing::ClothingSlot::Mask,
    ssr_core::clothing::ClothingSlot::Eyes,
    ssr_core::clothing::ClothingSlot::Ears,
    ssr_core::clothing::ClothingSlot::Shoes,
];

/// Проигрывает путь удалённого игрока ровно за тик сети — чужие персонажи
/// двигаются так же плавно, как свой (SS14 интерполирует все трансформы).
pub fn remote_player_interp(
    time: Res<Time>,
    mut visuals: Query<(&RemotePlayerVisual, &mut RemoteInterp, &mut Transform)>,
) {
    for (_, mut interp, mut transform) in visuals.iter_mut() {
        if !interp.started {
            continue;
        }
        interp.elapsed += time.delta_secs();
        let alpha = (interp.elapsed / crate::NET_TICK_SECS).clamp(0.0, 1.0);
        let position = interp.prev.lerp(interp.target, alpha);
        transform.translation.x = position.x;
        transform.translation.y = position.y;
    }
}
