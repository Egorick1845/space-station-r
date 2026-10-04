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
    /// Прокрутка списка спавн-меню в пикселях (непрерывная, как `ScrollContainer`);
    /// текущее значение догоняет `spawn_scroll_target` с rate 15.
    pub spawn_scroll: f32,
    pub spawn_scroll_target: f32,
    /// Поле поиска в фокусе (включается кликом — иначе буквы «съедались» при
    /// открытии F5 во время игры).
    pub search_focused: bool,
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

/// Корень окна «Панель спавна сущностей» (F5).
#[derive(Component)]
pub struct SpawnMenuRoot;

/// Корень окна «Телепорт призрака».
#[derive(Component)]
pub struct WarpMenuRoot;

/// Корень админ-меню (F7).
#[derive(Component)]
pub struct AdminMenuRoot;

/// Корень окна выбора внешности (P).
#[derive(Component)]
pub struct AppearanceRoot;

/// Иконка кнопки боевого режима (меняется по состоянию режима).
#[derive(Component)]
pub struct CombatButtonIcon;

/// Маркер у курсора в боевом режиме. В сборке это оверлей
/// `CombatModeIndicatorsOverlay`: спрайт `Interface/Misc/crosshair_pointers.rsi`
/// (64×64) состояний `gun_sight` / `gun_bolt_sight` / `melee_sight`, масштаб
/// `min(UIScale, 1.25) * 0.6` — то есть 38.4 px, БЕЗ смещения от курсора
/// (центр спрайта в точке курсора), белым с альфой 0.3, поверх чёрного
/// прямоугольника на 7 px больше (45.4 px, чёрный с альфой 0.5) — обводка.
#[derive(Component)]
pub struct CombatCursor;

/// Размер прицела: `64 × Scale`, где `Scale = min(UIScale, 1.25) * 0.6`
/// (`CombatModeIndicatorsOverlay.DrawSight`). При UIScale = 1 — 38.4 px.
const COMBAT_SIGHT_SIZE: f32 = 64.0 * 0.6;
/// Обводка шире прицела на `Vector2(7, 7)` из сборки.
const COMBAT_SIGHT_STROKE: f32 = 7.0;
/// Цвета оверлея: `MainColor = White.WithAlpha(0.3)`,
/// `StrokeColor = Black.WithAlpha(0.5)`.
const COMBAT_SIGHT_MAIN: Color = Color::srgba(1.0, 1.0, 1.0, 0.3);
const COMBAT_SIGHT_STROKE_COLOR: Color = Color::srgba(0.0, 0.0, 0.0, 0.5);

/// Меняет иконку кнопки боевого режима: включён — `harm.png`, выключен —
/// `harmOff.png` (`Interface/Actions/harm*.png` сборки).
pub fn sync_combat_button(
    state: Res<HudState>,
    theme: Res<crate::ui_theme::UiTheme>,
    mut buttons: Query<&mut ImageNode, With<CombatButtonIcon>>,
) {
    let wanted = theme.action_icons.get(if state.combat { 2 } else { 3 });
    let Some(wanted) = wanted else {
        return;
    };
    for mut image in buttons.iter_mut() {
        if image.image != *wanted {
            image.image = wanted.clone();
        }
    }
}

/// Рисует прицел у курсора в боевом режиме и убирает его вне боя: как
/// `CombatModeIndicatorsOverlay` в сборке — центр спрайта в точке курсора,
/// обводка под ним. Спрайт: `melee_sight` (в руках нет оружия; у `GunComponent`
/// сборка берёт `gun_sight`/`gun_bolt_sight`).
pub fn combat_cursor_marker(
    mut commands: Commands,
    state: Res<HudState>,
    settings: Res<crate::settings::Settings>,
    registry: Res<crate::rsi::RsiRegistry>,
    windows: Query<&Window>,
    markers: Query<Entity, With<CombatCursor>>,
) {
    let cursor = windows
        .iter()
        .next()
        .and_then(|window| window.cursor_position());
    let existing: Vec<Entity> = markers.iter().collect();
    let sight = registry.get("sprites/ss14/Interface/Misc/crosshair_pointers.rsi#melee_sight");
    // Настройка «Основные» → «Прицел боевого режима»
    // (`hud.combat_mode_indicators_point_show` в сборке).
    let (Some(cursor), true, true, Some(sight)) =
        (cursor, state.combat, settings.combat_indicators, sight)
    else {
        for entity in existing {
            commands.entity(entity).despawn();
        }
        return;
    };
    let size = COMBAT_SIGHT_SIZE;
    let stroke = size + COMBAT_SIGHT_STROKE;
    // Корень — обводка (чёрный прямоугольник под прицелом), ребёнок — сам прицел.
    let root_node = Node {
        position_type: PositionType::Absolute,
        left: px(cursor.x - stroke * 0.5),
        top: px(cursor.y - stroke * 0.5),
        width: px(stroke),
        height: px(stroke),
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        ..default()
    };
    let mut icon = crate::inventory_ui::icon_node(sight);
    icon.color = COMBAT_SIGHT_MAIN;
    let icon_node = Node {
        width: px(size),
        height: px(size),
        ..default()
    };
    if let Some(entity) = existing.first() {
        commands
            .entity(*entity)
            .insert((root_node, BackgroundColor(COMBAT_SIGHT_STROKE_COLOR)))
            .despawn_related::<Children>()
            .with_child((icon, icon_node));
    } else {
        commands
            .spawn((
                CombatCursor,
                root_node,
                BackgroundColor(COMBAT_SIGHT_STROKE_COLOR),
            ))
            .with_child((icon, icon_node));
    }
}

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
    // Верхняя панель — меню-кнопки (в SS14 их 10: гайд, персонаж, эмоции,
    // крафт, действия, админ, песочница, AHelp). У нас есть что открыть:
    // настройки (Esc), крафт (G — как OpenCraftingMenu в движке), спавн (F5),
    // админка (F7). Бой и осмотр — это ДЕЙСТВИЯ, они в левой колонке.
    let top: [(&str, HudAction, usize); 4] = [
        ("Esc", HudAction::OpenSettings, 0),
        ("G", HudAction::ToggleCraft, 4),
        ("F5", HudAction::ToggleSpawn, 7),
        ("F7", HudAction::ToggleAdmin, 6),
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
    // Колонка действий (ActionsBar): слоты 64×64, в углу — клавиша СЛОТА
    // (в SS14 это цифры 1..0, `TriggerAction(index)`), иконка — действие.
    // Порядок: как в движке, боевой режим первым (priority -100).
    let actions: [(&str, HudAction, usize); 3] = [
        ("1", HudAction::ToggleCombat, 2),
        ("2", HudAction::Examine, 0),
        ("3", HudAction::Drop, 1),
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
                let icon = theme
                    .action_icons
                    .get(icon_index)
                    .cloned()
                    .unwrap_or_default();
                // Боевой режим: иконка кнопки меняется по состоянию
                // (`harm.png` включён / `harmOff.png` выключен — как ActionButton
                // в сборке, где иконка берётся из состояния режима).
                let combat_button = matches!(action, HudAction::ToggleCombat);
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
                        if combat_button {
                            slot.spawn((
                                CombatButtonIcon,
                                ImageNode::new(icon),
                                Node {
                                    width: px(64),
                                    height: px(64),
                                    ..default()
                                },
                            ));
                        } else {
                            slot.spawn((
                                ImageNode::new(icon),
                                Node {
                                    width: px(64),
                                    height: px(64),
                                    ..default()
                                },
                            ));
                        }
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

/// Корень колонки алертов (правый верхний угол, `AlertsUI.xaml`: Right + Top,
/// столбец 64×64 с отступом 10). Внутри: Health, Stamina — порядок алертов.
#[derive(Component)]
pub struct AlertRoot;

/// Алерт здоровья (`HumanHealth` в alerts.yml): 5 иконок `human_alive.rsi`
/// health0..health4. Уровень — как `MobThresholdSystem`: лерп от 0 (максимальная
/// тяжесть) до 4 (здоров) по проценту до следующего порога состояния.
pub fn health_alert_level(health: &ssr_core::inventory::Health) -> u8 {
    // `MobThresholdSystem` сборки: `severity = round(lerp(0, 4, доля урона до
    // следующего состояния))`, иконки HumanHealth — health0..health4 по
    // возрастанию severity. То есть 0 урона → health0 («ХОРОШО», зелёная) и
    // порог крита → health4 («ОПАСНО!», красная). Раньше у нас было наоборот:
    // полностью здоровый игрок показывал красную иконку опасности.
    let damage = (health.max - health.current).max(0) as f32;
    let fraction = (damage / health.max.max(1) as f32).clamp(0.0, 1.0);
    (4.0 * fraction).round().clamp(0.0, 4.0) as u8
}

/// Иконка алерта: ключ RSI и текущий кадр. В сборке (`AlertControl` +
/// `SpriteView`) спрайт алерта — это ФЛИПБУК из RSI: движок проигрывает кадры
/// состояния по `delays` из meta.json (`SpriteSystem.FrameUpdate`,
/// `Loop = true`), поэтому «мигание» здоровья и стамины — не код, а сами кадры
/// (у `health4` и `stamina0..4` амплитуда альфы в разы больше, чем у соседних).
#[derive(Component)]
pub struct AlertIcon {
    pub key: String,
    pub frame: u32,
    pub elapsed: f32,
}

/// Следующий кадр флипбука: накапливаем время и идём вперёд, пока его хватает
/// на кадр (цикл, а не один шаг — как `SpriteComponent.FrameUpdate`). Возвращает
/// (остаток времени, кадр).
pub fn advance_alert_frame(
    mut elapsed: f32,
    mut frame: u32,
    delays: &[f32],
    frames: u32,
) -> (f32, u32) {
    if frames == 0 {
        return (elapsed, 0);
    }
    let mut guard = 0;
    while guard < frames * 2 {
        guard += 1;
        let delay = delays
            .get(frame as usize)
            .copied()
            .unwrap_or(0.1)
            .max(0.001);
        if elapsed < delay {
            break;
        }
        elapsed -= delay;
        frame = (frame + 1) % frames;
    }
    (elapsed, frame)
}

/// Проигрывает RSI-анимацию иконок алертов: кадр вперёд, когда накопилось
/// время кадра, по кругу (`Loop = true` в `SpriteComponent`). Без этой системы
/// колонка показывала только нулевой кадр — статичную картинку.
pub fn animate_alerts(
    time: Res<Time>,
    registry: Res<crate::rsi::RsiRegistry>,
    mut icons: Query<(&mut AlertIcon, &mut ImageNode)>,
) {
    let dt = time.delta_secs();
    for (mut icon, mut node) in icons.iter_mut() {
        let Some(sprite) = registry.get(&icon.key) else {
            continue; // RSI ещё грузится (реестр ленивый)
        };
        let frames = sprite
            .frames_per_direction
            .first()
            .copied()
            .unwrap_or(1)
            .max(1);
        if frames <= 1 {
            continue;
        }
        let delays = sprite.delays.first().map(Vec::as_slice).unwrap_or(&[]);
        let (elapsed, frame) = advance_alert_frame(icon.elapsed + dt, icon.frame, delays, frames);
        icon.elapsed = elapsed;
        icon.frame = frame;
        let index = sprite.index(0, icon.frame);
        if let Some(atlas) = node.texture_atlas.as_mut()
            && atlas.index != index
        {
            atlas.index = index;
        }
    }
}

/// Подпись колонки алертов: уровни Health/Stamina и алерт давления (если есть).
type AlertsSignature = (u8, u8, Option<(bool, u8)>, bool);

/// Рисует колонку алертов: Health (5 иконок human_alive), Stamina
/// (7 иконок stamina) и, в опасной зоне, давление (pressure.rsi) — правый
/// верхний угол под чатом, столбец 64×64 (AlertsUI в сборке).
#[allow(clippy::too_many_arguments)]
pub fn render_alerts_column(
    mut commands: Commands,
    own: Res<OwnPlayerEntity>,
    staminas: Query<&ssr_core::stamina::Stamina>,
    healths: Query<&ssr_core::inventory::Health>,
    knocked: Query<&ssr_core::mechanics::KnockedDown>,
    positions: Query<&ssr_core::PlayerPosition>,
    atmospheres: Query<(
        &ssr_core::atmosphere::ChunkAtmosphere,
        &ssr_core::tiles::TileChunkData,
    )>,
    registry: Res<crate::rsi::RsiRegistry>,
    root: Query<Entity, With<AlertRoot>>,
    mut last: Local<Option<AlertsSignature>>,
) {
    let level_health = own
        .0
        .and_then(|entity| healths.get(entity).ok())
        .map(health_alert_level)
        .unwrap_or(0);
    let level_stamina = own
        .0
        .and_then(|entity| staminas.get(entity).ok())
        .map(|stamina| stamina.alert_level())
        .unwrap_or(6);
    // Давление: в сборке (`BarotraumaSystem`) алерт показывается ТОЛЬКО в
    // опасной зоне — предупреждение (уровень 1) и урон (2), иначе категория
    // «Pressure» снимается целиком.
    let pressure_alert = own
        .0
        .and_then(|entity| positions.get(entity).ok())
        .and_then(|position| {
            let tile_units = ssr_core::tiles::TILE_PX as f32;
            let tx = (position.0[0] / tile_units).floor() as i32;
            let ty = (position.0[1] / tile_units).floor() as i32;
            let size = ssr_core::tiles::CHUNK_TILES as i32;
            let coords = (tx.div_euclid(size), ty.div_euclid(size));
            let (atmosphere, _) = atmospheres
                .iter()
                .find(|(_, chunk)| chunk.coords == coords)?;
            atmosphere
                .at((tx - coords.0 * size) as u32, (ty - coords.1 * size) as u32)
                .and_then(|gas| ssr_core::atmosphere::pressure_alerts::alert_for(gas.pressure))
        });
    // Тест-режим SSR_PRESSURE_TEST=1: рисует иконку давления принудительно —
    // серверный вакуум-тест опустошает другую точку спавна, и без этого
    // проверить саму иконку (RSI, состояние) нечем.
    let pressure_alert = if std::env::var_os("SSR_PRESSURE_TEST").is_some() {
        Some((false, 2))
    } else {
        pressure_alert
    };
    // Алерт «Knockdown» (`alerts.yml`: `stunnable.rsi#knocked-down`) — пока
    // игрок лежит (нокдаун от стамина-крита или урона). В сборке при
    // стамина-крите он показывается БЕЗ кольца-таймера.
    let knockdown = own.0.is_some_and(|entity| knocked.get(entity).is_ok());
    let signature = (level_health, level_stamina, pressure_alert, knockdown);
    if last.as_ref() == Some(&signature) {
        return;
    }
    let mut icons = vec![
        format!("sprites/ss14/Interface/Alerts/human_alive.rsi#health{level_health}"),
        format!("sprites/ss14/Interface/Alerts/stamina.rsi#stamina{level_stamina}"),
    ];
    if knockdown {
        icons.push("sprites/ss14/Interface/Alerts/stunnable.rsi#knocked-down".to_string());
    }
    if let Some((high, level)) = pressure_alert {
        let side = if high { "high" } else { "low" };
        icons.push(format!(
            "sprites/ss14/Interface/Alerts/pressure.rsi#{side}pressure{level}"
        ));
    }
    // Реестр RSI ленивый: в первый кадр спрайтов ещё нет. Без этой проверки
    // колонка один раз собиралась пустой, а `last` уже запрещал повтор — алерты
    // не появлялись вообще (владелец: «индикатор стамины работает неправильно»).
    if icons.iter().any(|key| registry.get(key).is_none()) {
        return;
    }
    *last = Some(signature);
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    commands
        .spawn((
            AlertRoot,
            Node {
                position_type: PositionType::Absolute,
                right: px(10),
                // В сборке чат и алерты оба `TopRight`, но алерты сдвинуты вниз
                // на высоту чата (`DefaultGameScreen`: `SetMarginTop(Alerts, …)`),
                // иначе колонка прячется за окном чата.
                top: px(crate::chat::CHAT_HEIGHT + 20.0),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                ..default()
            },
        ))
        .with_children(|column| {
            for key in icons {
                let Some(sprite) = registry.get(&key) else {
                    continue; // RSI подгрузится следующим кадром (реестр ленивый)
                };
                let icon = crate::inventory_ui::icon_node(sprite);
                // Иконка 32×32 в масштабе ×2 (AlertControl: Scale=(2,2)).
                // Кадры играет `animate_alerts` (флипбук RSI, как в сборке).
                column.spawn((
                    icon,
                    AlertIcon {
                        key: key.clone(),
                        frame: 0,
                        elapsed: 0.0,
                    },
                    Node {
                        width: px(64),
                        height: px(64),
                        ..default()
                    },
                ));
            }
        });
}

/// Space — спринт-тоггл (`Sprint` в `keybinds.yml` сборки): отправляем намерение,
/// сервер проверит запреты (лежание, призрак) и паузу 3 с между спринтами.
pub fn sprint_hotkey(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<Console>,
    chat: Res<crate::chat::ChatState>,
    own: Res<OwnPlayerEntity>,
    sprintings: Query<&ssr_core::mechanics::Sprinting>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if console.open || chat.focused || !keys.just_pressed(KeyCode::Space) {
        return;
    }
    let Some(sprinting) = own.0.and_then(|entity| sprintings.get(entity).ok()) else {
        return;
    };
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::ToggleSprint {
            sprint: !sprinting.0,
        });
    }
    tracing::info!(sprint = !sprinting.0, "sprint toggle sent");
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
    // Цифры запускают действия колонки — как `TriggerAction(index)` по HotbarN
    // в SS14 (выбор руки там на X, у нас тоже).
    for (key, action) in [
        (KeyCode::Digit1, HudAction::ToggleCombat),
        (KeyCode::Digit2, HudAction::Examine),
        (KeyCode::Digit3, HudAction::Drop),
    ] {
        if !keys.just_pressed(key) {
            continue;
        }
        match action {
            HudAction::ToggleCombat => {
                state.combat = !state.combat;
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::SetCombat {
                        combat: state.combat,
                    });
                }
                tracing::info!(combat = state.combat, "action: combat toggled by hotkey");
            }
            HudAction::Drop => {
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::DropHand);
                }
            }
            HudAction::Examine => tracing::info!("осмотр: наведите курсор и нажмите E"),
            _ => {}
        }
    }
    // G — окно крафта (в SS14 `OpenCraftingMenu` = G).
    if keys.just_pressed(KeyCode::KeyG) {
        crafting.open = !crafting.open;
    }
    // I — окно персонажа со слотами одежды (в SS14 `OpenInventoryMenu` = I).
    if keys.just_pressed(KeyCode::KeyI) {
        for mut ui in uis.iter_mut() {
            ui.character_open = !ui.character_open;
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

/// Полоса прокрутки как в SS14 (`ScrollBar.cs` + `StyleBase`): текстуры нет,
/// дорожка не рисуется, граббер 10 px шириной и минимум 10 px длиной, прижат
/// вправо на всю высоту списка. Появляется только при контенте выше вьюпорта.
///
/// Геометрия ровно из движка: `track = h − 10`, `grab_h = (viewport/content)·track + 10`,
/// `grab_y = (scroll/content)·track`, всё округляется до целых px.
pub(crate) fn spawn_scrollbar(
    parent: &mut ChildSpawnerCommands,
    content_h: f32,
    viewport_h: f32,
    scroll: f32,
    list: ScrollList,
) -> Option<Entity> {
    use crate::ui_theme as ui;
    if content_h <= viewport_h + 1e-3 {
        return None;
    }
    let track = (viewport_h - ui::SCROLLBAR_MIN_GRABBER).max(0.0);
    let ratio = (scroll / content_h).clamp(0.0, 1.0);
    let grab_h = ((viewport_h / content_h) * track).round() + ui::SCROLLBAR_MIN_GRABBER;
    let grab_y = (ratio * track).round();
    Some(
        parent
            .spawn(Node {
                position_type: PositionType::Absolute,
                right: px(0),
                top: px(0),
                width: px(ui::SCROLLBAR_WIDTH),
                height: px(viewport_h),
                ..default()
            })
            .with_child((
                ScrollbarGrabber { list },
                Interaction::default(),
                Node {
                    position_type: PositionType::Absolute,
                    top: px(grab_y),
                    left: px(0),
                    width: px(ui::SCROLLBAR_WIDTH),
                    height: px(grab_h),
                    ..default()
                },
                BackgroundColor(ui::SCROLLBAR_GRABBER),
            ))
            .id(),
    )
}

/// Какой список прокручивает полоса (нужно перетаскиванию граббера: у каждого
/// списка своё состояние прокрутки).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollList {
    AppearanceHair,
    AppearanceBeard,
    SpawnMenu,
}

/// Граббер полосы прокрутки (маркер нужен, чтобы подсветка не трогала другие
/// кнопки: у них тоже есть `Interaction`).
#[derive(Component, Clone, Copy)]
pub struct ScrollbarGrabber {
    pub list: ScrollList,
}

/// Грабберы, у которых сменилось состояние наведения.
type GrabberCursor<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static mut BackgroundColor),
    (Changed<Interaction>, With<ScrollbarGrabber>),
>;

/// Цвет граббера по состоянию (покой `#80808059`, hover `#8C8C8C59`,
/// перетаскивание `#A0A0A059` — псевдоклассы `hover`/`grabbed` в `ScrollBar.cs`).
pub fn tint_scrollbar_grabber(mut grabbers: GrabberCursor) {
    use crate::ui_theme as ui;
    for (interaction, mut color) in grabbers.iter_mut() {
        let target = match interaction {
            Interaction::None => ui::SCROLLBAR_GRABBER,
            Interaction::Hovered => ui::SCROLLBAR_GRABBER_HOVERED,
            Interaction::Pressed => ui::SCROLLBAR_GRABBER_GRABBED,
        };
        color.0 = target;
    }
}

/// Каркас окна меню — как `DefaultWindow` в SS14: шапка `window_header`,
/// заголовок цветом `NanoGold`, крестик `cross.svg` с модуляцией `#4B596A`
/// и фон `window_background_bordered`. Возвращает корень окна: вызывающий
/// вешает на него СВОЙ маркер (`SpawnMenuRoot` и т.п.), чтобы окна не стирали
/// друг друга при перерисовке (в SS14 они сосуществуют).
pub(crate) fn menu_panel(
    commands: &mut Commands,
    theme: &crate::ui_theme::UiTheme,
    title: &str,
    left: f32,
    top: f32,
    width: f32,
    build: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
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
        })
        .id()
}

/// Сколько строк помещается в спавн-меню (SS14 прокручивает список, у нас
/// список ограничен — остальное уточняется поиском).
const SPAWN_MENU_ROWS: usize = 11;
/// Ширина окна спавна (`SetSize="350 400"` в `EntitySpawnWindow.xaml`) и
/// ширина списка внутри тела окна (минус отступы `WINDOW_CONTENT_MARGIN`).
const SPAWN_WINDOW_W: f32 = 350.0;
const SPAWN_LIST_W: f32 = SPAWN_WINDOW_W - 2.0 * crate::ui_theme::WINDOW_CONTENT_MARGIN;
/// Шаг строки списка спавн-меню (строка 32 px + зазор 2 px).
const SPAWN_ROW_STEP: f32 = 34.0;

/// Высота вьюпорта списка спавн-меню (последний зазор не считаем).
pub(crate) fn spawn_view_h() -> f32 {
    SPAWN_MENU_ROWS as f32 * SPAWN_ROW_STEP - 2.0
}

/// Высота контента списка спавн-меню.
pub(crate) fn spawn_content_h(total: usize) -> f32 {
    total as f32 * SPAWN_ROW_STEP
}

/// Сколько предметов проходит фильтр поиска (для полосы прокрутки и колеса).
pub(crate) fn spawn_matched_count(search: &str, content: &ClientContent) -> usize {
    let query = search.to_lowercase();
    content
        .items
        .items
        .iter()
        .filter(|item| {
            query.is_empty()
                || format!("{} {}", item.id.to_lowercase(), item.name.to_lowercase())
                    .contains(&query)
        })
        .count()
}

/// Догоняет цель прокрутки экспонентой (`LerpAnimate(rate: 15)` в движке).
pub fn spawn_scroll_anim(time: Res<Time>, mut state: ResMut<HudState>) {
    use crate::ui_theme as ui;
    let k = 1.0 - (-ui::SCROLLBAR_ANIM_RATE * time.delta_secs()).exp();
    state.spawn_scroll += (state.spawn_scroll_target - state.spawn_scroll) * k;
}

/// Отпечаток состояния спавн-меню (открыто, поиск, спрайты, режим размещения).
type SpawnMenuSignature = (bool, String, u32, Option<String>, i32);

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
    root: Query<Entity, With<SpawnMenuRoot>>,
    mut last: Local<Option<SpawnMenuSignature>>,
) {
    use crate::ui_theme as ui;
    let signature = (
        state.spawn_open,
        state.search.clone(),
        registry.generation(),
        placement.item.clone(),
        // Округляем: анимация приближается к цели асимптотически, без округления
        // окно перерисовывалось бы каждый кадр вечно.
        state.spawn_scroll.round() as i32,
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
    // Список — как `EntitySpawnWindow` в сборке: ВСЕ сущности, у которых есть
    // спрайт (наш каталог + импортированные прототипы `prototypes_ss14.ron`),
    // а не только ручной набор предметов.
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
    let mut seen: std::collections::HashSet<String> =
        matched.iter().map(|(id, _)| id.clone()).collect();
    for (id, sprite) in content.proto_sprites.iter() {
        if seen.contains(id) {
            continue;
        }
        let name = content
            .proto_names
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.clone());
        if !query.is_empty()
            && !format!("{} {}", id.to_lowercase(), name.to_lowercase()).contains(&query)
        {
            continue;
        }
        let _ = sprite;
        seen.insert(id.clone());
        matched.push((id.clone(), name));
    }
    matched.sort_by(|a, b| a.1.cmp(&b.1));
    let total = matched.len();
    // Прокрутка непрерывная, как в `EntitySpawnWindow`/`ScrollContainer`.
    let view_h = spawn_view_h();
    let content_h = spawn_content_h(total);
    let max_scroll = (content_h - view_h).max(0.0);
    let scroll = state.spawn_scroll.clamp(0.0, max_scroll);
    let first = ((scroll / SPAWN_ROW_STEP).floor() as usize).min(total);
    let offset = scroll - first as f32 * SPAWN_ROW_STEP;
    // Как `ScrollContainer`: при видимой полосе контент ужимается на её ширину.
    let bar_width = if content_h > view_h + 1e-3 {
        crate::ui_theme::SCROLLBAR_WIDTH
    } else {
        0.0
    };
    matched = matched.split_off(first);
    matched.truncate(SPAWN_MENU_ROWS + 1);
    let icons: Vec<Option<ImageNode>> = matched
        .iter()
        .map(|(id, _)| {
            crate::inventory_ui::item_icon(&registry, &content, id)
                .map(crate::inventory_ui::icon_node)
        })
        .collect();

    // Окно 350×400 у левого края по центру экрана (`LayoutPreset.CenterLeft`).
    commands
        .spawn((
            HudMenuRoot,
            SpawnMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(10),
                top: Val::Percent(50.0),
                width: px(SPAWN_WINDOW_W),
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
                // Список: строка = иконка 32×32 + имя, зазор 2 px; прокрутка
                // непрерывная (`ScrollContainer`), справа — полоса прокрутки.
                body.spawn(Node {
                    width: Val::Percent(100.0),
                    height: px(view_h),
                    overflow: Overflow::clip(),
                    ..default()
                })
                .with_children(|list| {
                    list.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(0),
                            top: px(0),
                            width: px(SPAWN_LIST_W - bar_width),
                            flex_direction: FlexDirection::Column,
                            row_gap: px(2),
                            ..default()
                        },
                        UiTransform::from_translation(Val2::new(Val::Px(0.0), Val::Px(-offset))),
                    ))
                    .with_children(|inner| {
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
                            inner
                                .spawn((
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
                    spawn_scrollbar(list, content_h, view_h, scroll, ScrollList::SpawnMenu);
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

/// Поле поиска в спавн-меню (клик — включить ввод).
#[derive(Component)]
pub struct MenuSearchButton;

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
    root: Query<Entity, With<WarpMenuRoot>>,
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
            WarpMenuRoot,
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
    root: Query<Entity, With<AdminMenuRoot>>,
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
    let admin_root = menu_panel(
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
    commands.entity(admin_root).insert(AdminMenuRoot);
}

/// Набор текста в поиске спавн-меню (пока меню открыто).
pub fn spawn_menu_input(
    mut events: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<HudState>,
    search_clicks: Query<&Interaction, (Changed<Interaction>, With<MenuSearchButton>)>,
) {
    if !state.spawn_open && !state.warp_open {
        return;
    }
    // Клик по полю поиска включает ввод; пока не включён — буквы не перехватываем.
    for interaction in search_clicks.iter() {
        if *interaction == Interaction::Pressed {
            state.search_focused = true;
        }
    }
    if !state.search_focused {
        return;
    }
    if keys.just_pressed(KeyCode::F5) || keys.just_pressed(KeyCode::Escape) {
        state.search_focused = false;
        return;
    }
    for event in events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        match &event.logical_key {
            Key::Backspace => {
                state.search.pop();
                state.spawn_scroll = 0.0;
                state.spawn_scroll_target = 0.0;
            }
            Key::Space => {
                state.search.push(' ');
                state.spawn_scroll = 0.0;
                state.spawn_scroll_target = 0.0;
            }
            Key::Character(text) => {
                state.search.push_str(text);
                state.spawn_scroll = 0.0;
                state.spawn_scroll_target = 0.0;
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
    let max_scroll = (spawn_content_h(total) - spawn_view_h()).max(0.0);
    for event in wheel.read() {
        // Шаг 50 px за щелчок, как `ScrollContainer.ScrollSpeedY`.
        state.spawn_scroll_target = (state.spawn_scroll_target
            - event.y * crate::ui_theme::SCROLLBAR_WHEEL_STEP)
            .clamp(0.0, max_scroll);
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

/// Esc закрывает все открытые окна (владелец: «сделай, чтобы все окна могли
/// закрываться на esc»): меню, окно персонажа и рюкзак, крафт. Чат при этом
/// обрабатывается своей системой (сначала снимает фокус) — здесь он пропускается.
#[allow(clippy::too_many_arguments)]
pub fn close_windows_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<Console>,
    chat: Res<crate::chat::ChatState>,
    mut state: ResMut<HudState>,
    mut crafting: ResMut<crate::crafting::CraftingState>,
    mut placement: ResMut<Placement>,
    mut uis: Query<&mut crate::inventory_ui::InventoryUi>,
    mut appearance: ResMut<crate::appearance::AppearanceUi>,
) {
    if console.open || chat.focused || !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if appearance.open {
        appearance.open = false;
        tracing::info!("escape: appearance closed");
        return;
    }
    // Сначала Esc снимает фокус поиска, потом закрывает окно.
    if state.search_focused {
        state.search_focused = false;
        return;
    }
    let any_open = state.spawn_open
        || state.admin_open
        || state.warp_open
        || crafting.open
        || placement.item.is_some();
    state.spawn_open = false;
    state.admin_open = false;
    state.warp_open = false;
    placement.item = None;
    if any_open {
        tracing::info!("escape: windows closed");
        return;
    }
    for mut ui in uis.iter_mut() {
        if ui.open || ui.character_open {
            ui.open = false;
            ui.character_open = false;
            tracing::info!("escape: inventory closed");
            return;
        }
    }
    crafting.open = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Флипбук иконки алерта идёт по `delays` и зацикливается: у `health0`
    /// (`human_alive.rsi`) 28 кадров по 0.05 с — ровно 1.4 с на цикл, как в
    /// meta.json сборки. Проверяем и переход через несколько кадров за тик.
    #[test]
    fn alert_flipbook_loops_by_rsi_delays() {
        let delays = vec![0.05f32; 28];
        // Меньше кадра — стоим на месте.
        assert_eq!(advance_alert_frame(0.02, 0, &delays, 28), (0.02, 0));
        // Ровно кадр — следующий.
        assert_eq!(advance_alert_frame(0.05, 0, &delays, 28), (0.0, 1));
        // Полтора цикла — вернулись к тому же кадру с остатком.
        let (elapsed, frame) = advance_alert_frame(1.4 + 0.7, 0, &delays, 28);
        assert_eq!(frame, 14);
        assert!((elapsed - 0.0).abs() < 1e-4, "остаток {elapsed}");
        // Зацикливание: 28 кадров = полный круг.
        assert_eq!(advance_alert_frame(1.4, 0, &delays, 28).1, 0);
    }

    /// Реальные иконки алертов из сборки анимированные: у `health0` 28 кадров по
    /// 0.05 с, и за цикл флипбук проходит ВСЕ кадры (иначе индикатор выглядел бы
    /// статичным, как было до `animate_alerts`).
    #[test]
    fn alert_icons_animate_through_all_frames() {
        let dir = ssr_core::assets_root().join("sprites/ss14/Interface/Alerts/human_alive.rsi");
        let Ok(rsi) = ssr_core::rsi::load_rsi(&dir) else {
            return; // ассетов нет — тест пропускаем
        };
        let Some(state) = rsi.states.iter().find(|state| state.name == "health0") else {
            panic!("в human_alive.rsi нет состояния health0");
        };
        let frames = state.frames_per_direction.first().copied().unwrap_or(1);
        let delays = state.delays.first().cloned().unwrap_or_default();
        assert!(
            frames >= 2,
            "у health0 должно быть больше кадра, получено {frames}"
        );
        assert_eq!(
            delays.len(),
            frames as usize,
            "задержек столько же, сколько кадров"
        );
        let total: f32 = delays.iter().sum();
        let mut seen = std::collections::HashSet::new();
        let mut frame = 0u32;
        seen.insert(frame);
        // Остаток времени переносится в следующий вызов — как в `animate_alerts`.
        let mut elapsed = 0.0f32;
        let mut played = 0.0f32;
        while played < total {
            let (rest, next) = advance_alert_frame(elapsed + 1.0 / 60.0, frame, &delays, frames);
            elapsed = rest;
            frame = next;
            played += 1.0 / 60.0;
            seen.insert(frame);
        }
        assert_eq!(
            seen.len(),
            frames as usize,
            "за цикл должны проигрываться все кадры (frames={frames}, delays={:?}, total={total}, seen={seen:?})",
            &delays[..delays.len().min(4)]
        );
    }
}
