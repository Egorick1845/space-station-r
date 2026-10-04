//! Игровой HUD (запросы владельца, по образцу SS14): верхняя панель кнопок,
//! боковые кнопки действий, спавн-меню (F5), админ-меню (F7), боевой режим,
//! осмотр по E.
//!
//! Действия, меняющие мир, идут теми же админ-командами, что и консоль
//! (`spawn`, `kick`, `heal`, `ghost`, `tpto`), — сервер остаётся источником
//! правды и проверяет права.

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::PlayerPosition;
use ssr_core::mechanics::PlayerName;
use ssr_core::roles::PlayerRole;
use ssr_protocol::ClientMessage;
use ssr_protocol::net::GameChannel;

use crate::console::Console;
use crate::content::ClientContent;
use crate::inventory_ui::OwnPlayerEntity;
use crate::settings::Settings;

/// Режим размещения (как в SS14 `EntitySpawningUIController`): после выбора
/// предмета в спавн-меню он «висит» на курсоре, ЛКМ ставит его в мир.
#[derive(Resource, Default)]
pub struct Placement {
    pub item: Option<String>,
}

/// Полупрозрачный спрайт предмета, следующий за курсором в режиме размещения.
#[derive(Component)]
pub struct PlacementGhost;

/// Состояние HUD: открытые меню, боевой режим, поиск в спавн-меню.
#[derive(Resource, Default)]
pub struct HudState {
    pub spawn_open: bool,
    pub admin_open: bool,
    /// Окно «Телепорт призрака» (`MiniGhostTargetWindow` в SS14).
    pub warp_open: bool,
    pub combat: bool,
    pub search: String,
    /// Прокрутка списка спавн-меню (строк).
    pub spawn_scroll: usize,
}

/// Действие кнопки HUD.
#[derive(Component, Clone, PartialEq)]
pub enum HudAction {
    /// Свернуть/развернуть меню крафта.
    ToggleCraft,
    /// Спавн-меню (F5).
    ToggleSpawn,
    /// Админ-меню (F7).
    ToggleAdmin,
    /// Окно телепорта призрака (кнопка панели призрака).
    GhostWarp,
    /// Меню настроек (Esc).
    OpenSettings,
    /// Боевой режим (F).
    ToggleCombat,
    /// Осмотр тайла под курсором (E).
    Examine,
    /// Выбросить предмет из активной руки (Q).
    Drop,
    /// Админ-команда серверу.
    Admin(String),
    /// Спавн предмета из каталога (клик — в мир, Ctrl+клик — в рюкзак).
    SpawnItem(String),
}

/// Корень HUD (верхняя панель и боковые кнопки).
#[derive(Component)]
pub struct HudRoot;

/// Корень пересобираемого меню (спавн/админ).
#[derive(Component)]
pub struct HudMenuRoot;

/// Строка списка игроков в админ-меню.
#[derive(Component, Clone, PartialEq)]
pub struct AdminPlayerButton {
    pub name: String,
    pub kick: bool,
}

/// Клики по кнопкам HUD.
type HudClicks<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        &'static HudAction,
        &'static mut BackgroundColor,
    ),
    (Changed<Interaction>, With<Button>),
>;
/// Клики по строкам админ-меню.
type AdminPlayerClicks<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        &'static AdminPlayerButton,
        &'static mut BackgroundColor,
    ),
    (Changed<Interaction>, With<Button>, Without<HudAction>),
>;

/// Цвет кнопки HUD по состоянию.
fn button_bg(hovered: bool, pressed: bool) -> BackgroundColor {
    if pressed {
        BackgroundColor(Color::srgb(0.22, 0.22, 0.28))
    } else if hovered {
        BackgroundColor(Color::srgb(0.16, 0.16, 0.20))
    } else {
        BackgroundColor(Color::srgba(0.08, 0.08, 0.11, 0.85))
    }
}

/// Тонировка фона кнопки (стеклянные кнопки StyleNano).
#[derive(Component)]
pub struct HudTint {
    pub normal: Color,
    pub hovered: Color,
    pub pressed: Color,
}

impl HudTint {
    /// Обычная кнопка интерфейса (`glassButton*` из StyleNano).
    pub fn button() -> Self {
        Self {
            normal: crate::ui_theme::GLASS_BUTTON,
            hovered: crate::ui_theme::GLASS_BUTTON_HOVERED,
            pressed: crate::ui_theme::GLASS_BUTTON_PRESSED,
        }
    }

    fn color(&self, state: crate::ui_theme::UiButtonState) -> Color {
        match state {
            crate::ui_theme::UiButtonState::Normal => self.normal,
            crate::ui_theme::UiButtonState::Hovered => self.hovered,
            crate::ui_theme::UiButtonState::Pressed => self.pressed,
        }
    }
}

/// Тонировка иконки (`MenuButton.Color*` и `StyleBase` для крестиков).
#[derive(Component)]
pub struct IconTint {
    pub normal: Color,
    pub hovered: Color,
    pub pressed: Color,
}

impl IconTint {
    /// Иконка верхней панели (MenuButton).
    pub fn menu() -> Self {
        Self {
            normal: crate::ui_theme::TOP_ICON,
            hovered: crate::ui_theme::TOP_ICON_HOVERED,
            pressed: crate::ui_theme::TOP_ICON_PRESSED,
        }
    }

    /// Крестик закрытия окна (`StyleBase`: #4B596A / #7F3636 / #753131).
    pub fn cross() -> Self {
        Self {
            normal: Color::srgb_u8(0x4b, 0x59, 0x6a),
            hovered: Color::srgb_u8(0x7f, 0x36, 0x36),
            pressed: Color::srgb_u8(0x75, 0x31, 0x31),
        }
    }

    fn color(&self, state: crate::ui_theme::UiButtonState) -> Color {
        match state {
            crate::ui_theme::UiButtonState::Normal => self.normal,
            crate::ui_theme::UiButtonState::Hovered => self.hovered,
            crate::ui_theme::UiButtonState::Pressed => self.pressed,
        }
    }
}

/// Кнопки HUD с тонировкой — запрос вынесен в алиас ради лимита clippy.
type TintedButtons<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        Option<&'static HudTint>,
        Option<&'static IconTint>,
        &'static mut BackgroundColor,
        Option<&'static mut ImageNode>,
        Option<&'static Children>,
    ),
    (Changed<Interaction>, With<Button>),
>;

/// Перекрашивает кнопки и их иконки при наведении/нажатии.
pub fn hud_button_tint(
    mut buttons: TintedButtons,
    mut child_icons: Query<(&IconTint, &mut ImageNode), Without<Button>>,
) {
    for (interaction, bg_tint, icon_tint, mut background, image, children) in buttons.iter_mut() {
        let state = crate::ui_theme::UiButtonState::from_interaction(interaction);
        if let Some(tint) = bg_tint {
            background.0 = tint.color(state);
        }
        if let Some(mut image) = image
            && let Some(tint) = icon_tint
        {
            image.color = tint.color(state);
        }
        let Some(children) = children else {
            continue;
        };
        for child in children.iter() {
            if let Ok((tint, mut child_image)) = child_icons.get_mut(child) {
                child_image.color = tint.color(state);
            }
        }
    }
}

/// Спавнит HUD при входе в игру — по образцу SS14 (`DefaultGameScreen.xaml`):
/// верхняя панель иконок слева вверху (`GameTopMenuBar`, кнопки 42×64 с
/// подписью горячей клавиши), под ней колонка действий (`ActionsBar`, слоты
/// 64×64 с подписью клавиши в углу), снизу — панель призрака (`GhostGui`,
/// кнопки по центру, отступ 80).
pub fn spawn_hud(
    mut commands: Commands,
    own: Res<OwnPlayerEntity>,
    theme: Res<crate::ui_theme::UiTheme>,
    roots: Query<(), With<HudRoot>>,
) {
    use crate::ui_theme as ui;
    if own.0.is_none() || !roots.is_empty() {
        return;
    }
    // Верхняя панель: иконка + подпись горячей клавиши (порядок — TOP_ICONS).
    let top: [(&str, HudAction, usize); 5] = [
        ("Esc", HudAction::OpenSettings, 0),
        ("F5", HudAction::ToggleSpawn, 7),
        ("F7", HudAction::ToggleAdmin, 6),
        ("C", HudAction::ToggleCraft, 4),
        ("F", HudAction::ToggleCombat, 5),
    ];
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(10),
                top: px(10),
                flex_direction: FlexDirection::Row,
                column_gap: px(5),
                ..default()
            },
        ))
        .with_children(|bar| {
            for (index, (key, action, icon_index)) in top.into_iter().enumerate() {
                let icon = theme.icons.get(icon_index).cloned().unwrap_or_default();
                // Первая кнопка панели в SS14 шире остальных (70×64).
                let width = if index == 0 { 70.0 } else { 42.0 };
                bar.spawn((
                    action,
                    Button,
                    HudTint::button(),
                    BackgroundColor(ui::GLASS_BUTTON),
                    Node {
                        width: px(width),
                        height: px(64),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        row_gap: px(2),
                        ..default()
                    },
                ))
                .with_children(|button| {
                    button.spawn((
                        IconTint::menu(),
                        ImageNode::new(icon),
                        Node {
                            width: px(24),
                            height: px(24),
                            ..default()
                        },
                    ));
                    button.spawn((
                        Text::new(key),
                        TextFont::from_font_size(ui::FONT_LABEL),
                        TextColor(ui::TOP_ICON),
                    ));
                });
            }
        });
    // Колонка действий (ActionsBar): слот 64×64 с фоном SlotBackground,
    // иконка действия заполняет слот (32 px при масштабе ×2), подпись клавиши —
    // в левом верхнем углу (ActionButton.cs: Margin(5,0,0,0), цвет whiteText).
    // Крафт берёт молоток из иконок верхней панели, бой — иконки Actions.
    let actions: [(&str, HudAction, usize); 4] = [
        ("E", HudAction::Examine, 0),
        ("Q", HudAction::Drop, 1),
        ("C", HudAction::ToggleCraft, 4),
        ("F", HudAction::ToggleCombat, 2),
    ];
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(10),
                top: px(84),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                ..default()
            },
        ))
        .with_children(|column| {
            for (key, action, icon_index) in actions {
                let icon = if action == HudAction::ToggleCraft {
                    theme.icons.get(icon_index).cloned().unwrap_or_default()
                } else {
                    theme
                        .action_icons
                        .get(icon_index)
                        .cloned()
                        .unwrap_or_default()
                };
                column
                    .spawn((
                        action,
                        Button,
                        HudTint {
                            normal: Color::WHITE,
                            hovered: Color::WHITE,
                            pressed: Color::srgb(0.92, 0.92, 0.96),
                        },
                        crate::ui_theme::stretched(&theme.slot_background),
                        BackgroundColor(Color::WHITE),
                        Node {
                            width: px(64),
                            height: px(64),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                    ))
                    .with_children(|slot| {
                        slot.spawn((
                            ImageNode::new(icon),
                            Node {
                                width: px(64),
                                height: px(64),
                                ..default()
                            },
                        ));
                        slot.spawn((
                            Text::new(key),
                            TextFont::from_font_size(13.0),
                            TextColor(ui::TEXT),
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(5),
                                top: px(0),
                                ..default()
                            },
                        ));
                    });
            }
        });
    // Кнопка окна инвентаря (`Slots/toggle`) — внизу слева, `BottomLeft` margin 5.
    commands.spawn((
        HudRoot,
        crate::inventory_ui::BackpackButton,
        Button,
        crate::hud::IconTint {
            normal: Color::WHITE,
            hovered: Color::srgb(0.92, 0.92, 0.96),
            pressed: Color::srgb(0.85, 0.85, 0.9),
        },
        ImageNode::new(theme.slot_toggle.clone()),
        Node {
            position_type: PositionType::Absolute,
            left: px(5),
            bottom: px(5),
            width: px(64),
            height: px(64),
            ..default()
        },
    ));

    // Панель призрака: в SS14 стоит снизу по центру с отступом 80.
    commands
        .spawn((
            GhostBarRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                bottom: px(80),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .with_children(|bar| {
            for (label, action) in [
                ("Вернуться в тело", HudAction::Admin("unghost".to_string())),
                ("Телепорт призрака", HudAction::GhostWarp),
                ("Настройки", HudAction::OpenSettings),
            ] {
                bar.spawn((
                    GhostBarButton,
                    action,
                    Button,
                    HudTint::button(),
                    BackgroundColor(ui::GLASS_BUTTON),
                    Node {
                        padding: UiRect::axes(px(ui::BUTTON_PADDING_H), px(ui::BUTTON_PADDING_V)),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                ))
                .with_child((
                    Text::new(label),
                    TextFont::from_font_size(ui::FONT_BASE),
                    TextColor(ui::TEXT),
                ));
            }
        });
}

/// Корень панели призрака (показывается только призраку, как в SS14).
#[derive(Component)]
pub struct GhostBarRoot;

/// Кнопка панели призрака.
#[derive(Component)]
pub struct GhostBarButton;

/// Показывает панель призрака только когда свой игрок — призрак (`Ghost`).
pub fn update_ghost_bar(
    own: Res<OwnPlayerEntity>,
    ghosts: Query<(), With<ssr_core::mechanics::Ghost>>,
    mut bars: Query<&mut Node, With<GhostBarRoot>>,
) {
    let ghost = own.0.is_some_and(|entity| ghosts.contains(entity));
    for mut node in bars.iter_mut() {
        let display = if ghost { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
}

/// Позиция курсора в мировых координатах (для осмотра по E, подбора кликом).
pub(crate) fn cursor_world(
    windows: &Query<&Window>,
    camera: &Single<(&Camera, &GlobalTransform), With<Camera2d>>,
) -> Option<Vec2> {
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, transform) = **camera;
    camera.viewport_to_world_2d(transform, cursor).ok()
}

/// Действие по E: открыть ближайшую дверь или ящик под курсором; если рядом
/// ничего нет — применить предмет из активной руки к тайлу (как в SS14).
/// Подбор предметов с пола — кликом мыши (SS14), не на E.
fn send_interact(
    senders: &mut Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    world: Vec2,
    entity_map: Option<&ServerEntityMap>,
    own: Option<Entity>,
    hands: &Query<&ssr_core::inventory::Hands>,
    doors: &Query<(Entity, &ssr_core::Door)>,
    containers: &Query<
        (Entity, &ssr_core::inventory::ItemPosition),
        With<ssr_core::inventory::Container>,
    >,
) {
    let mut nearest: Option<(f32, Entity)> = None;
    for (entity, door) in doors.iter() {
        let distance = Vec2::from_array(door.position).distance(world);
        if distance <= 24.0 && nearest.is_none_or(|(best, _)| distance < best) {
            nearest = Some((distance, entity));
        }
    }
    for (entity, position) in containers.iter() {
        let distance = Vec2::from_array(position.0).distance(world);
        if distance <= 24.0 && nearest.is_none_or(|(best, _)| distance < best) {
            nearest = Some((distance, entity));
        }
    }
    let tx = (world.x / 32.0).floor() as i32;
    let ty = (world.y / 32.0).floor() as i32;
    // Объект рядом: клиентская сущность → серверные bits → Interact.
    if let Some((_, target)) = nearest
        && let Some(map) = entity_map
        && let Some(server_entity) = map.to_server().get(&target)
    {
        let bits = server_entity.to_bits();
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Interact { entity: bits });
        }
        return;
    }
    // Иначе — предмет из активной руки к тайлу.
    let item = own
        .and_then(|entity| hands.get(entity).ok())
        .and_then(|hands| hands.active_item());
    if let Some(item) = item {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::UseItem { item, tx, ty });
        }
    }
}

/// Отправляет осмотр тайла по мировым координатам.
fn send_examine(
    senders: &mut Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    world: Vec2,
) {
    let tx = (world.x / 32.0).floor() as i32;
    let ty = (world.y / 32.0).floor() as i32;
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Examine { entity: 0, tx, ty });
    }
}

/// Горячие клавиши: F5 спавн, F7 админ, F бой, E действие, Q бросить, C крафт.
/// Предмет с пола поднимается кликом мыши (как в SS14).
#[allow(clippy::too_many_arguments)]
pub fn hud_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<Console>,
    chat: Res<crate::chat::ChatState>,
    mut state: ResMut<HudState>,
    mut crafting: ResMut<crate::crafting::CraftingState>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    doors: Query<(Entity, &ssr_core::Door)>,
    containers: Query<
        (Entity, &ssr_core::inventory::ItemPosition),
        With<ssr_core::inventory::Container>,
    >,
    entity_map: Option<Res<ServerEntityMap>>,
    own: Res<OwnPlayerEntity>,
    hands: Query<&ssr_core::inventory::Hands>,
    mut uis: Query<&mut crate::inventory_ui::InventoryUi>,
) {
    // Текст набирается в консоли или чате — горячие клавиши мира не работают.
    if console.open || chat.focused {
        return;
    }
    if keys.just_pressed(KeyCode::F5) {
        state.spawn_open = !state.spawn_open;
        state.admin_open = false;
        tracing::info!(open = state.spawn_open, "spawn menu toggled");
    }
    if keys.just_pressed(KeyCode::F7) {
        state.admin_open = !state.admin_open;
        state.spawn_open = false;
        tracing::info!(open = state.admin_open, "admin menu toggled");
    }
    if keys.just_pressed(KeyCode::KeyF) {
        state.combat = !state.combat;
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::SetCombat {
                combat: state.combat,
            });
        }
        tracing::info!(combat = state.combat, "combat mode toggled");
    }
    if keys.just_pressed(KeyCode::KeyQ) {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::DropHand);
        }
    }
    // E — действие: дверь/ящик под курсором открыть, иначе применить предмет
    // из активной руки к тайлу (как в SS14). Shift+E — осмотр.
    if keys.just_pressed(KeyCode::KeyE)
        && let Some(world) = cursor_world(&windows, &camera)
    {
        let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
        if shift {
            send_examine(&mut senders, world);
        } else {
            send_interact(
                &mut senders,
                world,
                entity_map.as_deref(),
                own.0,
                &hands,
                &doors,
                &containers,
            );
        }
    }
    // 1/2 — выбрать руку напрямую (как хотбар в SS14).
    for (key, index) in [(KeyCode::Digit1, 0u8), (KeyCode::Digit2, 1u8)] {
        if keys.just_pressed(key) {
            for mut sender in senders.iter_mut() {
                sender.send::<GameChannel>(ClientMessage::TakeInHand { slot: index });
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyC) {
        crafting.open = !crafting.open;
    }
    // V — окно рюкзака (в SS14 это клавиша `OpenBackpack`).
    if keys.just_pressed(KeyCode::KeyV) {
        for mut ui in uis.iter_mut() {
            ui.open = !ui.open;
        }
    }
}

/// Клики по кнопкам HUD.
#[allow(clippy::too_many_arguments)]
pub fn hud_click(
    mut state: ResMut<HudState>,
    mut crafting: ResMut<crate::crafting::CraftingState>,
    mut placement: ResMut<Placement>,
    mut commands: Commands,
    settings: Res<Settings>,
    keys: Res<ButtonInput<KeyCode>>,
    menus: Query<Entity, With<crate::settings::SettingsMenu>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut clicks: HudClicks,
) {
    for (interaction, action, mut color) in clicks.iter_mut() {
        *color = button_bg(
            *interaction == Interaction::Hovered,
            *interaction == Interaction::Pressed,
        );
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action {
            HudAction::ToggleCraft => crafting.open = !crafting.open,
            HudAction::ToggleSpawn => {
                state.spawn_open = !state.spawn_open;
                state.admin_open = false;
                state.warp_open = false;
            }
            HudAction::ToggleAdmin => {
                state.admin_open = !state.admin_open;
                state.spawn_open = false;
                state.warp_open = false;
            }
            HudAction::GhostWarp => {
                state.warp_open = !state.warp_open;
                state.spawn_open = false;
                state.admin_open = false;
                state.search.clear();
            }
            HudAction::OpenSettings => {
                crate::settings::open_menu(&mut commands, &settings, &menus);
            }
            HudAction::ToggleCombat => {
                state.combat = !state.combat;
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::SetCombat {
                        combat: state.combat,
                    });
                }
            }
            HudAction::Examine => tracing::info!("осмотр: наведите курсор и нажмите E"),
            HudAction::Drop => {
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::DropHand);
                }
            }
            HudAction::Admin(command) => {
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::Admin {
                        command: command.clone(),
                    });
                }
            }
            HudAction::SpawnItem(id) => {
                // Как в SS14: обычный клик — режим размещения (предмет «висит» на
                // курсоре, ЛКМ ставит), Ctrl+клик — сразу в рюкзак.
                if keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight]) {
                    for mut sender in senders.iter_mut() {
                        sender.send::<GameChannel>(ClientMessage::Admin {
                            command: format!("spawn {id} 1"),
                        });
                    }
                } else {
                    placement.item = Some(id.clone());
                    tracing::info!(item = %id, "placement mode started");
                }
            }
        }
    }
}

/// Клики по строкам админ-меню: телепорт к игроку или кик.
pub fn admin_player_click(
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut clicks: AdminPlayerClicks,
) {
    for (interaction, row, mut color) in clicks.iter_mut() {
        *color = button_bg(
            *interaction == Interaction::Hovered,
            *interaction == Interaction::Pressed,
        );
        if *interaction != Interaction::Pressed {
            continue;
        }
        let command = if row.kick {
            format!("kick {}", row.name)
        } else {
            format!("tpto {}", row.name)
        };
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Admin {
                command: command.clone(),
            });
        }
    }
}

/// Шапка окна по SS14: плоский фон `Accent("#2A2A38D9", 0.26)`, акцентная
/// линия 2 px снизу, заголовок `#EAF2FF` (14) и крестик `cross.svg`.
fn window_header(header: &mut ChildSpawnerCommands, theme: &crate::ui_theme::UiTheme, title: &str) {
    use crate::ui_theme as ui;
    header
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: px(25.0),
                align_items: AlignItems::Center,
                padding: UiRect::new(px(6), px(6), px(2), px(2)),
                column_gap: px(6),
                border: UiRect::bottom(px(2)),
                ..default()
            },
            BackgroundColor(ui::GLASS_HEADER),
            BorderColor::from(ui::GLASS_HEADER_LINE),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new(title),
                TextFont::from_font_size(ui::FONT_LABEL),
                TextColor(ui::WINDOW_TITLE),
            ));
            row.spawn((
                MenuCloseButton,
                Button,
                IconTint::cross(),
                ImageNode::new(theme.cross.clone()),
                Node {
                    width: px(22),
                    height: px(22),
                    margin: UiRect::left(Val::Auto),
                    ..default()
                },
            ));
        });
}

/// Тело окна: плоская панель StyleNano с отступом содержимого 10.
fn window_body() -> (Node, BackgroundColor) {
    use crate::ui_theme as ui;
    (
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(ui::WINDOW_CONTENT_MARGIN)),
            row_gap: px(4),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(ui::GLASS_PANEL),
    )
}

/// Плоское поле ввода (в сборке у LineEdit нет рамки — только фон).
fn glass_field() -> (Node, BackgroundColor) {
    use crate::ui_theme as ui;
    (
        Node {
            height: px(24),
            align_items: AlignItems::Center,
            padding: UiRect::new(px(8), px(8), px(4), px(4)),
            ..default()
        },
        BackgroundColor(ui::GLASS_LINEEDIT),
    )
}

/// Каркас окна меню — как `DefaultWindow` в SS14: шапка `window_header`,
/// заголовок цветом `NanoGold`, крестик `cross.svg` с модуляцией `#4B596A`
/// и фон `window_background_bordered`.
fn menu_panel(
    commands: &mut Commands,
    theme: &crate::ui_theme::UiTheme,
    title: &str,
    left: f32,
    top: f32,
    width: f32,
    build: impl FnOnce(&mut ChildSpawnerCommands),
) {
    commands
        .spawn((
            HudMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(left),
                top: Val::Percent(top),
                width: px(width),
                max_height: px(520),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            UiTransform::from_translation(Val2::new(Val::Percent(-50.0), Val::Percent(-50.0))),
        ))
        .with_children(|window| {
            window_header(window, theme, title);
            let (node, background) = window_body();
            window.spawn((node, background)).with_children(build);
        });
}

/// Сколько строк помещается в спавн-меню (SS14 прокручивает список, у нас
/// список ограничен — остальное уточняется поиском).
const SPAWN_MENU_ROWS: usize = 11;

/// Отпечаток состояния спавн-меню (открыто, поиск, спрайты, режим размещения).
type SpawnMenuSignature = (bool, String, u32, Option<String>, usize);

/// Перерисовывает спавн-меню (F5) по образцу `EntitySpawnWindow.xaml`:
/// окно 350×400 у левого края, поле поиска с кнопкой «Очистить», список
/// строк «иконка 32×32 + имя», снизу — подсказка режима размещения.
#[allow(clippy::too_many_arguments)]
pub fn render_spawn_menu(
    mut commands: Commands,
    state: Res<HudState>,
    placement: Res<Placement>,
    content: Res<ClientContent>,
    registry: Res<crate::rsi::RsiRegistry>,
    theme: Res<crate::ui_theme::UiTheme>,
    root: Query<Entity, With<HudMenuRoot>>,
    mut last: Local<Option<SpawnMenuSignature>>,
) {
    use crate::ui_theme as ui;
    let signature = (
        state.spawn_open,
        state.search.clone(),
        registry.generation(),
        placement.item.clone(),
        state.spawn_scroll,
    );
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    if !state.spawn_open {
        return;
    }
    let query = state.search.to_lowercase();
    let mut matched: Vec<(String, String)> = content
        .items
        .items
        .iter()
        .filter(|item| {
            query.is_empty()
                || format!("{} {}", item.id.to_lowercase(), item.name.to_lowercase())
                    .contains(&query)
        })
        .map(|item| (item.id.clone(), item.name.clone()))
        .collect();
    matched.sort_by(|a, b| a.1.cmp(&b.1));
    let total = matched.len();
    // Прокрутка колесом (`EntitySpawnWindow` прокручивается целиком).
    let max_scroll = total.saturating_sub(SPAWN_MENU_ROWS);
    let scroll = state.spawn_scroll.min(max_scroll);
    matched = matched.split_off(scroll);
    matched.truncate(SPAWN_MENU_ROWS);
    let icons: Vec<Option<ImageNode>> = matched
        .iter()
        .map(|(id, _)| {
            crate::inventory_ui::item_icon(&registry, &content.items, id)
                .map(crate::inventory_ui::icon_node)
        })
        .collect();

    // Окно 350×400 у левого края по центру экрана (`LayoutPreset.CenterLeft`).
    commands
        .spawn((
            HudMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(10),
                top: Val::Percent(50.0),
                width: px(350),
                max_height: px(400),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            UiTransform::from_translation(Val2::new(Val::Px(0.0), Val::Percent(-50.0))),
        ))
        .with_children(|window| {
            window_header(window, &theme, "Панель спавна сущностей");
            let (node, background) = window_body();
            window.spawn((node, background)).with_children(|body| {
                // Строка поиска: поле ввода + «Очистить» (как в SS14).
                body.spawn(Node {
                    width: Val::Percent(100.0),
                    height: px(24),
                    column_gap: px(4),
                    ..default()
                })
                .with_children(|row| {
                    let (mut field, field_bg) = glass_field();
                    field.flex_grow = 1.0;
                    row.spawn((field, field_bg)).with_child((
                        Text::new(if state.search.is_empty() {
                            "поиск".to_string()
                        } else {
                            format!("{}_", state.search)
                        }),
                        TextFont::from_font_size(ui::FONT_BASE),
                        TextColor(if state.search.is_empty() {
                            ui::TEXT_MUTED
                        } else {
                            ui::TEXT
                        }),
                    ));
                    row.spawn((
                        MenuClearButton,
                        Button,
                        HudTint::button(),
                        BackgroundColor(ui::GLASS_BUTTON),
                        Node {
                            height: px(24),
                            padding: UiRect::horizontal(px(ui::BUTTON_PADDING_H)),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_child((
                        Text::new("Очистить"),
                        TextFont::from_font_size(ui::FONT_BASE),
                        TextColor(ui::TEXT),
                    ));
                });
                // Список: строка = иконка 32×32 + имя, зазор 2 px.
                body.spawn(Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    ..default()
                })
                .with_children(|list| {
                    for ((id, name), icon) in matched.into_iter().zip(icons) {
                        let selected = placement.item.as_deref() == Some(id.as_str());
                        let tint = if selected {
                            HudTint {
                                normal: ui::GLASS_BUTTON_PRESSED,
                                hovered: ui::GLASS_BUTTON_PRESSED,
                                pressed: ui::GLASS_BUTTON_PRESSED,
                            }
                        } else {
                            HudTint::button()
                        };
                        list.spawn((
                            HudAction::SpawnItem(id.clone()),
                            Button,
                            tint,
                            BackgroundColor(if selected {
                                ui::GLASS_BUTTON_PRESSED
                            } else {
                                ui::GLASS_BUTTON
                            }),
                            Node {
                                width: Val::Percent(100.0),
                                height: px(32),
                                align_items: AlignItems::Center,
                                column_gap: px(6),
                                padding: UiRect::horizontal(px(6)),
                                ..default()
                            },
                        ))
                        .with_children(|row| {
                            // Иконка 32×32, как EntityPrototypeView в SS14.
                            row.spawn(Node {
                                width: px(32),
                                height: px(32),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            })
                            .with_children(|cell| {
                                if let Some(icon) = icon {
                                    cell.spawn((
                                        icon,
                                        Node {
                                            width: px(28),
                                            height: px(28),
                                            ..default()
                                        },
                                    ));
                                }
                            });
                            row.spawn((
                                Text::new(name),
                                TextFont::from_font_size(ui::FONT_BASE),
                                TextColor(ui::TEXT),
                            ));
                            row.spawn((
                                Text::new(id),
                                TextFont::from_font_size(ui::FONT_SMALL),
                                TextColor(ui::TEXT_MUTED),
                            ));
                        });
                    }
                });
                // Подсказка снизу: счётчик и режим размещения.
                let hint = match &placement.item {
                    Some(item) => format!("Размещение: {item} — ЛКМ поставить, ПКМ отменить"),
                    None => format!(
                        "{}/{} · клик — размещать, Ctrl+клик — в рюкзак",
                        total, total
                    ),
                };
                body.spawn((
                    Text::new(hint),
                    TextFont::from_font_size(ui::FONT_SMALL),
                    TextColor(ui::NANO_GOLD),
                ));
            });
        });
}

/// Кнопка «Очистить» в спавн-меню.
#[derive(Component)]
pub struct MenuClearButton;

/// Крестик закрытия окна меню.
#[derive(Component)]
pub struct MenuCloseButton;

/// Обрабатывает «Очистить» и крестик в окнах меню.
pub fn menu_buttons_click(
    mut state: ResMut<HudState>,
    mut placement: ResMut<Placement>,
    clear: Query<&Interaction, (Changed<Interaction>, With<MenuClearButton>)>,
    close: Query<&Interaction, (Changed<Interaction>, With<MenuCloseButton>)>,
) {
    for interaction in clear.iter() {
        if *interaction == Interaction::Pressed {
            state.search.clear();
        }
    }
    for interaction in close.iter() {
        if *interaction == Interaction::Pressed {
            state.spawn_open = false;
            state.admin_open = false;
            state.warp_open = false;
            placement.item = None;
        }
    }
}

/// Двигает полупрозрачный спрайт предмета за курсором в режиме размещения.
pub fn update_placement_ghost(
    mut commands: Commands,
    placement: Res<Placement>,
    sprites: crate::inventory_ui::ItemSprites,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    ghosts: Query<Entity, With<PlacementGhost>>,
) {
    let cursor = cursor_world(&windows, &camera);
    let active = placement.item.as_deref().and_then(|item| {
        cursor.map(|world| sprites.icon_by_name(item).map(|sprite| (sprite, world)))
    });
    match (active.flatten(), ghosts.iter().next()) {
        (Some((sprite, world)), Some(entity)) => {
            commands.entity(entity).insert((
                Sprite {
                    image: sprite.image.clone(),
                    texture_atlas: Some(TextureAtlas {
                        layout: sprite.layout.clone(),
                        index: sprite.index(0, 0),
                    }),
                    color: Color::srgba(1.0, 1.0, 1.0, 0.55),
                    ..default()
                },
                Transform::from_xyz(world.x, world.y, PLACEMENT_Z),
            ));
        }
        (Some((sprite, world)), None) => {
            commands.spawn((
                PlacementGhost,
                Sprite {
                    image: sprite.image.clone(),
                    texture_atlas: Some(TextureAtlas {
                        layout: sprite.layout.clone(),
                        index: sprite.index(0, 0),
                    }),
                    color: Color::srgba(1.0, 1.0, 1.0, 0.55),
                    ..default()
                },
                Transform::from_xyz(world.x, world.y, PLACEMENT_Z),
            ));
        }
        (None, Some(entity)) => {
            commands.entity(entity).despawn();
        }
        (None, None) => {}
    }
}

/// Слой спрайта-призрака размещения: выше предметов на полу, ниже игрока.
const PLACEMENT_Z: f32 = 0.8;

/// Клик в мире в режиме размещения: ЛКМ — поставить предмет (админ-команда
/// с координатами), ПКМ — отменить (как выход из режима в SS14).
#[allow(clippy::too_many_arguments)]
pub fn placement_click(
    mouse: Res<ButtonInput<MouseButton>>,
    mut placement: ResMut<Placement>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
    ui: Query<&Interaction, With<Button>>,
) {
    let Some(item) = placement.item.clone() else {
        return;
    };
    if mouse.just_pressed(MouseButton::Right) {
        placement.item = None;
        tracing::info!("placement cancelled");
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    // Клик по интерфейсу принадлежит UI (строки списка, кнопки).
    if ui
        .iter()
        .any(|interaction| *interaction != Interaction::None)
    {
        return;
    }
    let Some(world) = cursor_world(&windows, &camera) else {
        return;
    };
    let command = format!("spawn {item} 1 floor {:.1} {:.1}", world.x, world.y);
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Admin {
            command: command.clone(),
        });
    }
    tracing::info!(command, "placement spawn sent");
}

/// Перерисовывает окно «Телепорт призрака» (MiniGhostTargetWindow в SS14):
/// поле поиска и список игроков с координатами; клик — телепорт к игроку.
pub fn render_warp_menu(
    mut commands: Commands,
    state: Res<HudState>,
    theme: Res<crate::ui_theme::UiTheme>,
    players: Query<(&PlayerName, &PlayerPosition)>,
    root: Query<Entity, With<HudMenuRoot>>,
    mut last: Local<Option<(bool, String, usize)>>,
) {
    use crate::ui_theme as ui;
    let signature = (
        state.warp_open,
        state.search.clone(),
        players.iter().count(),
    );
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    if !state.warp_open {
        return;
    }
    let query = state.search.to_lowercase();
    let own_name = std::env::var("SSR_NAME").unwrap_or_default();
    let mut rows: Vec<(String, [f32; 2])> = players
        .iter()
        .filter(|(name, _)| name.0 != own_name)
        .filter(|(name, _)| query.is_empty() || name.0.to_lowercase().contains(&query))
        .map(|(name, position)| (name.0.clone(), position.0))
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    // В SS14 окно телепорта открывается по центру экрана (`OpenCentered`).
    commands
        .spawn((
            HudMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(50.0),
                top: Val::Percent(50.0),
                width: px(450),
                height: px(450),
                flex_direction: FlexDirection::Column,
                row_gap: px(3),
                ..default()
            },
            UiTransform::from_translation(Val2::new(Val::Percent(-50.0), Val::Percent(-50.0))),
        ))
        .with_children(|window| {
            window_header(window, &theme, "Телепорт призрака");
            let (node, background) = window_body();
            window.spawn((node, background)).with_children(|body| {
                body.spawn(glass_field()).with_child((
                    Text::new(if state.search.is_empty() {
                        "поиск".to_string()
                    } else {
                        format!("{}_", state.search)
                    }),
                    TextFont::from_font_size(ui::FONT_BASE),
                    TextColor(if state.search.is_empty() {
                        ui::TEXT_MUTED
                    } else {
                        ui::TEXT
                    }),
                ));
                for (name, position) in rows {
                    body.spawn((
                        AdminPlayerButton {
                            name: name.clone(),
                            kick: false,
                        },
                        Button,
                        HudTint::button(),
                        BackgroundColor(ui::GLASS_BUTTON),
                        Node {
                            width: Val::Percent(100.0),
                            height: px(26),
                            align_items: AlignItems::Center,
                            padding: UiRect::horizontal(px(8)),
                            column_gap: px(6),
                            ..default()
                        },
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Text::new(name),
                            TextFont::from_font_size(ui::FONT_BASE),
                            TextColor(ui::TEXT),
                        ));
                        row.spawn((
                            Text::new(format!("{:.0}, {:.0}", position[0], position[1])),
                            TextFont::from_font_size(ui::FONT_SMALL),
                            TextColor(ui::TEXT_MUTED),
                        ));
                    });
                }
            });
        });
}

/// Перерисовывает админ-меню (F7): действия и список игроков в интересе.
pub fn render_admin_menu(
    mut commands: Commands,
    state: Res<HudState>,
    theme: Res<crate::ui_theme::UiTheme>,
    players: Query<(&PlayerName, &PlayerRole, &PlayerPosition)>,
    root: Query<Entity, With<HudMenuRoot>>,
    mut last: Local<Option<(bool, usize, String)>>,
) {
    // Свой игрок исключается по имени (сервер прислал его в Connect).
    let own_name = std::env::var("SSR_NAME").unwrap_or_default();
    let names: Vec<String> = players
        .iter()
        .map(|(name, role, _)| {
            if role.name.is_empty() {
                name.0.clone()
            } else {
                format!("{} — {}", name.0, role.name)
            }
        })
        .collect();
    let signature = (state.admin_open, names.len(), names.join("|"));
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    if !state.admin_open {
        return;
    }
    let _ = own_name;
    menu_panel(
        &mut commands,
        &theme,
        "Админ-меню",
        50.0,
        50.0,
        360.0,
        |panel| {
            for (label, command) in [
                ("Лечить себя", "heal"),
                ("Стать призраком (полёт)", "ghost"),
                ("Вернуться в тело", "unghost"),
            ] {
                panel
                    .spawn((
                        HudAction::Admin(command.to_string()),
                        Button,
                        Node {
                            height: px(24),
                            align_items: AlignItems::Center,
                            padding: UiRect::horizontal(px(6)),
                            border: UiRect::all(px(2)),
                            ..default()
                        },
                        button_bg(false, false),
                        BorderColor::from(Color::srgb(0.40, 0.30, 0.30)),
                    ))
                    .with_child((
                        Text::new(label),
                        TextFont::from_font_size(12.0),
                        TextColor(Color::srgb(0.92, 0.88, 0.88)),
                    ));
            }
            panel.spawn((
                Text::new(format!("Игроки в интересе ({}):", names.len())),
                TextFont::from_font_size(12.0),
                TextColor(Color::srgb(0.72, 0.72, 0.76)),
            ));
            for entry in names {
                let name = entry.split(" — ").next().unwrap_or_default().to_string();
                panel
                    .spawn((
                        AdminPlayerButton {
                            name: name.clone(),
                            kick: false,
                        },
                        Button,
                        Node {
                            height: px(22),
                            align_items: AlignItems::Center,
                            padding: UiRect::horizontal(px(6)),
                            ..default()
                        },
                        button_bg(false, false),
                    ))
                    .with_child((
                        Text::new(format!("{entry} → телепорт")),
                        TextFont::from_font_size(12.0),
                        TextColor(Color::srgb(0.85, 0.85, 0.88)),
                    ));
                panel
                    .spawn((
                        AdminPlayerButton { name, kick: true },
                        Button,
                        Node {
                            height: px(20),
                            align_items: AlignItems::Center,
                            padding: UiRect::horizontal(px(6)),
                            ..default()
                        },
                        button_bg(false, false),
                    ))
                    .with_child((
                        Text::new("  кикнуть"),
                        TextFont::from_font_size(11.0),
                        TextColor(Color::srgb(0.95, 0.65, 0.60)),
                    ));
            }
        },
    );
}

/// Набор текста в поиске спавн-меню (пока меню открыто).
pub fn spawn_menu_input(
    mut events: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<HudState>,
) {
    if !state.spawn_open && !state.warp_open {
        return;
    }
    if keys.just_pressed(KeyCode::F5) || keys.just_pressed(KeyCode::Escape) {
        return;
    }
    for event in events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        match &event.logical_key {
            Key::Backspace => {
                state.search.pop();
                state.spawn_scroll = 0;
            }
            Key::Space => {
                state.search.push(' ');
                state.spawn_scroll = 0;
            }
            Key::Character(text) => {
                state.search.push_str(text);
                state.spawn_scroll = 0;
            }
            _ => {}
        }
    }
}

/// Прокрутка списков меню колесом мыши (спавн-меню и телепорт призрака).
pub fn menu_scroll(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    mut state: ResMut<HudState>,
    chat: Res<crate::chat::ChatState>,
    content: Res<ClientContent>,
) {
    if chat.focused {
        wheel.clear();
        return;
    }
    if !state.spawn_open && !state.warp_open {
        return;
    }
    let query = state.search.to_lowercase();
    let total = content
        .items
        .items
        .iter()
        .filter(|item| {
            query.is_empty()
                || format!("{} {}", item.id.to_lowercase(), item.name.to_lowercase())
                    .contains(&query)
        })
        .count();
    let max_scroll = total.saturating_sub(SPAWN_MENU_ROWS);
    for event in wheel.read() {
        if event.y > 0.0 {
            state.spawn_scroll = (state.spawn_scroll + 1).min(max_scroll);
        } else if event.y < 0.0 {
            state.spawn_scroll = state.spawn_scroll.saturating_sub(1);
        }
    }
}

/// Тест механик: SSR_MECH_TEST=1 — бой (4 с), осмотр тайла (5.5 с),
/// призрак (8 с), возврат в тело (12 с).
pub fn mech_test_mode(
    time: Res<Time>,
    mut state: Local<(f32, u8)>,
    mut hud: ResMut<HudState>,
    own: Res<OwnPlayerEntity>,
    positions: Query<&ssr_core::PlayerPosition>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if std::env::var_os("SSR_MECH_TEST").is_none() {
        return;
    }
    state.0 += time.delta_secs();
    let send = |senders: &mut Query<&mut MessageSender<ClientMessage>, With<Connected>>,
                message: ClientMessage| {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(message.clone());
        }
    };
    if state.1 == 0 && state.0 >= 4.0 {
        state.1 = 1;
        hud.combat = true;
        send(&mut senders, ClientMessage::SetCombat { combat: true });
        tracing::info!("mech-test: combat on");
    } else if state.1 == 1 && state.0 >= 5.5 {
        state.1 = 2;
        if let Some(entity) = own.0
            && let Ok(position) = positions.get(entity)
        {
            send(
                &mut senders,
                ClientMessage::Examine {
                    entity: 0,
                    tx: (position.0[0] / 32.0).floor() as i32,
                    ty: (position.0[1] / 32.0).floor() as i32,
                },
            );
            tracing::info!("mech-test: examine sent");
        }
    } else if state.1 == 2 && state.0 >= 8.0 {
        state.1 = 3;
        send(
            &mut senders,
            ClientMessage::Admin {
                command: "ghost".to_string(),
            },
        );
        tracing::info!("mech-test: ghost sent");
    } else if state.1 == 3 && state.0 >= 12.0 {
        state.1 = 4;
        send(
            &mut senders,
            ClientMessage::Admin {
                command: "unghost".to_string(),
            },
        );
        tracing::info!("mech-test: unghost sent");
    }
}
