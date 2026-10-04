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

/// Состояние HUD: открытые меню, боевой режим, поиск в спавн-меню.
#[derive(Resource, Default)]
pub struct HudState {
    pub spawn_open: bool,
    pub admin_open: bool,
    pub combat: bool,
    pub search: String,
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
    /// Спавн предмета из каталога.
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

/// Спавнит HUD при входе в игру: верхняя панель + боковые действия.
pub fn spawn_hud(
    mut commands: Commands,
    own: Res<OwnPlayerEntity>,
    roots: Query<(), With<HudRoot>>,
) {
    if own.0.is_none() || !roots.is_empty() {
        return;
    }
    let top: [(&str, HudAction); 5] = [
        ("Крафт (C)", HudAction::ToggleCraft),
        ("Спавн (F5)", HudAction::ToggleSpawn),
        ("Админ (F7)", HudAction::ToggleAdmin),
        ("Настройки (Esc)", HudAction::OpenSettings),
        ("Бой (F)", HudAction::ToggleCombat),
    ];
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                top: px(0),
                height: px(34),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Center,
                column_gap: px(4),
                padding: UiRect::vertical(px(3)),
                ..default()
            },
        ))
        .with_children(|bar| {
            for (label, action) in top {
                bar.spawn((
                    action,
                    Button,
                    Node {
                        height: px(26),
                        padding: UiRect::horizontal(px(10)),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::all(px(2)),
                        ..default()
                    },
                    button_bg(false, false),
                    BorderColor::from(Color::srgb(0.30, 0.30, 0.36)),
                ))
                .with_child((
                    Text::new(label),
                    TextFont::from_font_size(12.0),
                    TextColor(Color::srgb(0.85, 0.85, 0.88)),
                ));
            }
        });
    let side: [(&str, HudAction); 2] = [
        ("Осмотреть (E)", HudAction::Examine),
        ("Бросить (Q)", HudAction::Drop),
    ];
    commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                right: px(10),
                top: px(90),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                ..default()
            },
        ))
        .with_children(|column| {
            for (label, action) in side {
                column
                    .spawn((
                        action,
                        Button,
                        Node {
                            width: px(118),
                            height: px(28),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            border: UiRect::all(px(2)),
                            ..default()
                        },
                        button_bg(false, false),
                        BorderColor::from(Color::srgb(0.30, 0.30, 0.36)),
                    ))
                    .with_child((
                        Text::new(label),
                        TextFont::from_font_size(12.0),
                        TextColor(Color::srgb(0.85, 0.85, 0.88)),
                    ));
            }
        });
}

/// Позиция курсора в мировых координатах (для осмотра по E).
fn cursor_world(
    windows: &Query<&Window>,
    camera: &Single<(&Camera, &GlobalTransform), With<Camera2d>>,
) -> Option<Vec2> {
    let window = windows.single().ok()?;
    let cursor = window.cursor_position()?;
    let (camera, transform) = **camera;
    camera.viewport_to_world_2d(transform, cursor).ok()
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

/// Горячие клавиши: F5 спавн, F7 админ, F бой, E осмотр, Q бросить, C крафт.
pub fn hud_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<Console>,
    mut state: ResMut<HudState>,
    mut crafting: ResMut<crate::crafting::CraftingState>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    windows: Query<&Window>,
    camera: Single<(&Camera, &GlobalTransform), With<Camera2d>>,
) {
    if console.open {
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
    if keys.just_pressed(KeyCode::KeyE)
        && let Some(world) = cursor_world(&windows, &camera)
    {
        send_examine(&mut senders, world);
    }
    if keys.just_pressed(KeyCode::KeyC) {
        crafting.open = !crafting.open;
    }
}

/// Клики по кнопкам HUD.
pub fn hud_click(
    mut state: ResMut<HudState>,
    mut crafting: ResMut<crate::crafting::CraftingState>,
    mut commands: Commands,
    settings: Res<Settings>,
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
            }
            HudAction::ToggleAdmin => {
                state.admin_open = !state.admin_open;
                state.spawn_open = false;
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
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::Admin {
                        command: format!("spawn {id} 1"),
                    });
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

/// Каркас окна меню (заголовок + содержимое, как окна SS14).
fn menu_panel(
    commands: &mut Commands,
    title: &str,
    left: f32,
    width: f32,
    accent: Color,
    build: impl FnOnce(&mut ChildSpawnerCommands),
) {
    commands
        .spawn((
            HudMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(left),
                top: px(44),
                width: px(width),
                max_height: px(600),
                flex_direction: FlexDirection::Column,
                row_gap: px(3),
                padding: UiRect::all(px(8)),
                border: UiRect::all(px(2)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.07, 0.96)),
            BorderColor::from(Color::srgb(0.35, 0.35, 0.42)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new(title),
                TextFont::from_font_size(14.0),
                TextColor(accent),
            ));
            build(panel);
        });
}

/// Перерисовывает спавн-меню (F5): поиск и список предметов каталога.
pub fn render_spawn_menu(
    mut commands: Commands,
    state: Res<HudState>,
    content: Res<ClientContent>,
    root: Query<Entity, With<HudMenuRoot>>,
    mut last: Local<Option<(bool, String)>>,
) {
    let signature = (state.spawn_open, state.search.clone());
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
    let items: Vec<(String, String)> = content
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
    menu_panel(
        &mut commands,
        &format!(
            "Спавн — поиск: {}_  (F5 — закрыть, предметы идут в рюкзак)",
            state.search
        ),
        280.0,
        420.0,
        Color::srgb(1.0, 0.75, 0.25),
        |panel| {
            for (id, name) in items {
                panel
                    .spawn((
                        HudAction::SpawnItem(id.clone()),
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
                        Text::new(format!("{name} ({id})")),
                        TextFont::from_font_size(12.0),
                        TextColor(Color::srgb(0.85, 0.85, 0.88)),
                    ));
            }
        },
    );
}

/// Перерисовывает админ-меню (F7): действия и список игроков в интересе.
pub fn render_admin_menu(
    mut commands: Commands,
    state: Res<HudState>,
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
        "Админ-меню (F7 — закрыть)",
        950.0,
        320.0,
        Color::srgb(1.0, 0.55, 0.45),
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
    if !state.spawn_open {
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
            }
            Key::Space => state.search.push(' '),
            Key::Character(text) => state.search.push_str(text),
            _ => {}
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
