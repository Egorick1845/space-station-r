//! Крафт (PLAN.md T5.2): панель рецептов по клавише C.
//!
//! Список рецептов — из `assets/prototypes/recipes.ron` (тот же файл, что у
//! сервера). Доступность считается по своему рюкзаку; по клику отправляем
//! `Craft { recipe }`, сервер проверяет материалы и выдаёт результат.

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::inventory::{Inventory, Item};
use ssr_core::recipes::can_craft;
use ssr_protocol::ClientMessage;
use ssr_protocol::net::GameChannel;

use crate::content::ClientContent;
use crate::inventory_ui::OwnPlayerEntity;

/// Открыта ли панель крафта.
#[derive(Resource, Default)]
pub struct CraftingState {
    pub open: bool,
}

/// Корень панели крафта.
#[derive(Component)]
pub struct CraftingRoot;

/// Кнопка рецепта.
#[derive(Component)]
pub struct CraftButton(pub String);

/// Состояние для перерисовки (доступность рецептов).
type CraftSignature = Vec<(String, bool)>;
/// Клики по кнопкам крафта.
type CraftClicks<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static CraftButton),
    (Changed<Interaction>, With<Button>),
>;

/// Клавиша G открывает и закрывает панель крафта (в сборке `OpenCraftingMenu`
/// привязан к G, `Resources/keybinds.yml:248-250`). На Esc панель закрывает
/// `hud::close_windows_on_escape` — как `CloseModals` (Esc) в сборке.
pub fn toggle_crafting(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<crate::console::Console>,
    chat: Res<crate::chat::ChatState>,
    mut state: ResMut<CraftingState>,
    root: Query<Entity, With<CraftingRoot>>,
) {
    // Пока игрок печатает в чате или в консоли — клавиша не открывает панель
    // («слишком настойчиво открывается»).
    if console.open || chat.focused || !keys.just_pressed(KeyCode::KeyG) {
        return;
    }
    state.open = !state.open;
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
}

/// Имена предметов в своём рюкзаке (для проверки рецептов).
fn own_item_names(
    own: &OwnPlayerEntity,
    inventories: &Query<&Inventory>,
    items: &Query<&Item>,
) -> Vec<String> {
    let Some(inventory) = own.0.and_then(|entity| inventories.get(entity).ok()) else {
        return Vec::new();
    };
    inventory
        .cells
        .iter()
        .flatten()
        .copied()
        .filter_map(Entity::try_from_bits)
        .filter_map(|entity| items.get(entity).ok())
        .map(|item| item.name.clone())
        .collect()
}

/// Рисует панель крафта (пересобирает при изменении доступности рецептов).
#[allow(clippy::too_many_arguments)]
pub fn render_crafting(
    mut commands: Commands,
    state: Res<CraftingState>,
    content: Res<ClientContent>,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    items: Query<&Item>,
    root: Query<Entity, With<CraftingRoot>>,
    mut last: Local<Option<CraftSignature>>,
) {
    if !state.open {
        return;
    }
    let have = own_item_names(&own, &inventories, &items);
    let signature: CraftSignature = content
        .recipes
        .recipes
        .iter()
        .map(|recipe| (recipe.id.clone(), can_craft(recipe, &have)))
        .collect();
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature.clone());

    for entity in root.iter() {
        commands.entity(entity).despawn();
    }

    commands
        .spawn((
            CraftingRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(430),
                top: px(60),
                width: px(420),
                max_height: px(520),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                padding: UiRect::all(px(10)),
                border: UiRect::all(px(2)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.94)),
            BorderColor::from(Color::srgb(0.30, 0.30, 0.36)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Крафт (C — закрыть)"),
                TextFont::from_font_size(16.0),
                TextColor(Color::srgb(1.0, 0.75, 0.25)),
            ));
            for (index, recipe) in content.recipes.recipes.iter().enumerate() {
                let available = signature
                    .get(index)
                    .map(|(_, ready)| *ready)
                    .unwrap_or(false);
                let inputs: Vec<String> = recipe
                    .inputs
                    .iter()
                    .map(|(id, count)| format!("{}×{}", content.items.name_of(id), count))
                    .collect();
                let output = format!(
                    "{}×{}",
                    content.items.name_of(&recipe.output.0),
                    recipe.output.1
                );
                let label = format!("{}: {} → {}", recipe.name, inputs.join(" + "), output);
                let mut row = panel.spawn((
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        min_height: px(26),
                        ..default()
                    },
                    BackgroundColor(if available {
                        Color::srgba(0.16, 0.20, 0.16, 0.9)
                    } else {
                        Color::srgba(0.12, 0.12, 0.14, 0.9)
                    }),
                ));
                row.with_child((
                    Text::new(label),
                    TextFont::from_font_size(12.0),
                    TextColor(if available {
                        Color::srgb(0.85, 0.92, 0.85)
                    } else {
                        Color::srgb(0.60, 0.60, 0.64)
                    }),
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                ));
                if available {
                    row.with_child((
                        CraftButton(recipe.id.clone()),
                        Button,
                        Node {
                            width: px(28),
                            height: px(22),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            border: UiRect::all(px(2)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.18, 0.30, 0.18)),
                        BorderColor::from(Color::srgb(0.35, 0.55, 0.35)),
                    ))
                    .with_child((
                        Text::new("+"),
                        TextFont::from_font_size(14.0),
                        TextColor(Color::srgb(0.9, 0.95, 0.9)),
                    ));
                }
            }
        });
}

/// Клик по кнопке рецепта — отправляем серверу запрос на крафт.
pub fn craft_click(
    clicks: CraftClicks,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    for (interaction, button) in clicks.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Craft {
                recipe: button.0.clone(),
            });
        }
        tracing::info!(recipe = %button.0, "craft sent");
    }
}

/// Тест T5.2: SSR_CRAFT_TEST=1 — через 5 с крафтит прутья, через 8 с — кабель.
pub fn craft_test_mode(
    time: Res<Time>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut state: Local<(f32, u8)>,
) {
    if std::env::var_os("SSR_CRAFT_TEST").is_none() {
        return;
    }
    state.0 += time.delta_secs();
    let mut send = |recipe: &str| {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Craft {
                recipe: recipe.to_string(),
            });
        }
        tracing::info!(recipe, "craft-test: sent");
    };
    if state.1 == 0 && state.0 >= 5.0 {
        state.1 = 1;
        send("metal-rod");
    } else if state.1 == 1 && state.0 >= 8.0 {
        state.1 = 2;
        send("cable-coil");
    }
}
