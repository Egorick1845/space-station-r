//! Крафт-меню по образцу `ConstructionMenu` сборки (§10 SS14_PORT_PLAN,
//! `Content.Client/Construction/UI/ConstructionMenu.xaml(.cs)` +
//! `ConstructionMenuPresenter.cs`): окно 560×450, слева поиск + выпадающий
//! список категорий + список рецептов (иконка 32×32 + имя), справа иконка/
//! имя/шаги выбранного рецепта и кнопки Build/Erase/Clear.
//!
//! Рецепты — из `assets/prototypes/recipes.ron` (тот же файл, что у сервера).
//! Список НЕ фильтруется по крафтабельности (как в сборке): доступность
//! управляет только кнопкой Build. Клик Build отправляет `Craft { recipe }`,
//! сервер проверяет материалы и выдаёт результат. Erase/Clear — заглушки:
//! в сборке они относятся к строительным призракам (Construction-графы, §7).

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::ButtonState;
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::inventory::{Inventory, Item};
use ssr_core::recipes::can_craft;
use ssr_protocol::ClientMessage;
use ssr_protocol::net::GameChannel;

use crate::content::ClientContent;
use crate::inventory_ui::OwnPlayerEntity;
use crate::ui_theme as ui;

/// Размер окна (`ConstructionMenu.xaml.cs:88-89`: SetSize 560×450, MinSize 560×320).
const WINDOW_W: f32 = 560.0;
const WINDOW_H: f32 = 450.0;
/// Ширина левой колонки (XAML: MinWidth 243, margin 0 0 5 0) и зазор до правой.
const LEFT_COL_W: f32 = 243.0;
const LEFT_COL_GAP: f32 = 5.0;
/// Ширина выпадающего списка категорий (XAML: OptionButton MinSize 130×0).
const CATEGORY_W: f32 = 130.0;
/// Строка списка рецептов: иконка 32×32 + имя (шаг 36 px с зазором).
const ROW_H: f32 = 34.0;
const ROW_STEP: f32 = ROW_H + 2.0;
/// Высота строки поиска и кнопок.
const FIELD_H: f32 = 24.0;
/// Кнопка Build (XAML: VerticalExpand, ratio 0.5) и ряд Erase (0.7)/Clear (0.3).
const BUILD_H: f32 = 44.0;
const ERASE_RATIO: f32 = 0.7;

/// Состояние окна: открытость, поиск, категория, выбор, прокрутка.
#[derive(Resource, Default)]
pub struct CraftingState {
    pub open: bool,
    /// Поиск по имени (`SearchBar`, без учёта регистра).
    pub search: String,
    /// Поле поиска в фокусе (включается кликом — как в спавн-меню).
    pub search_focused: bool,
    /// Выбранная категория (`OptionButton`); пусто — «Все».
    pub category: String,
    /// Раскрыт ли список категорий.
    pub cats_open: bool,
    /// Выбранный рецепт (id) — правая колонка показывает его шаги.
    pub selected: Option<String>,
    /// Прокрутка списка рецептов в px (непрерывная, как `ScrollContainer`).
    pub scroll: f32,
    pub scroll_target: f32,
}

/// Корень окна крафта.
#[derive(Component)]
pub struct CraftingRoot;

/// Поле поиска (клик включает ввод).
#[derive(Component)]
pub struct CraftSearchField;

/// Кнопка категории (клик раскрывает/сворачивает список).
#[derive(Component)]
pub struct CategoryButton;

/// Вариант в списке категорий.
#[derive(Component)]
pub struct CategoryOption(pub String);

/// Строка рецепта (клик выбирает рецепт).
#[derive(Component)]
pub struct RecipeRow(pub String);

/// Кнопка Build (клик отправляет крафт).
#[derive(Component)]
pub struct BuildButton(pub String);

/// Клавиша G открывает и закрывает окно (в сборке `OpenCraftingMenu` привязан
/// к G, `Resources/keybinds.yml:248-250`). На Esc закрывает
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
    state.search_focused = false;
    state.cats_open = false;
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
}

/// Имена предметов в своём рюкзаке (для доступности рецептов).
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

/// Уникальные категории рецептов по алфавиту (как `ConstructionMenuPresenter.
/// PopulateCategories:332-375`, без Favorites — их у нас нет).
fn categories(recipes: &[ssr_core::recipes::Recipe]) -> Vec<String> {
    let mut cats: Vec<String> = recipes
        .iter()
        .filter_map(|recipe| recipe.category.clone())
        .collect();
    cats.sort();
    cats.dedup();
    cats
}

/// Проходит ли рецепт фильтры окна: категория и поиск по имени без учёта
/// регистра (`ConstructionMenuPresenter` — поиск `Name.Contains(search)`).
pub(crate) fn matches(recipe: &ssr_core::recipes::Recipe, category: &str, query: &str) -> bool {
    if !category.is_empty() && recipe.category.as_deref() != Some(category) {
        return false;
    }
    query.is_empty() || recipe.name.to_lowercase().contains(query)
}

/// Высота области списка рецептов: тело окна (450 − шапка 25) минус отступы
/// содержимого и строка поиска.
pub(crate) fn list_view_h() -> f32 {
    WINDOW_H - 25.0 - 2.0 * ui::WINDOW_CONTENT_MARGIN - FIELD_H - 4.0
}

/// Число рецептов, прошедших фильтры окна (для перетаскивания граббера).
pub(crate) fn matched_count(state: &CraftingState, content: &ClientContent) -> usize {
    let query = state.search.to_lowercase();
    content
        .recipes
        .recipes
        .iter()
        .filter(|recipe| matches(recipe, &state.category, &query))
        .count()
}

/// Отпечаток окна: открытость, поиск, категория, попап, выбор, прокрутка,
/// спрайты и доступность рецептов (кнопка Build).
type CraftSignature = (
    bool,
    String,
    String,
    bool,
    Option<String>,
    i32,
    u32,
    Vec<bool>,
);

/// Рисует окно (пересобирает при изменении отпечатка).
#[allow(clippy::too_many_arguments)]
pub fn render_crafting(
    mut commands: Commands,
    state: Res<CraftingState>,
    content: Res<ClientContent>,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    items: Query<&Item>,
    registry: Res<crate::rsi::RsiRegistry>,
    theme: Res<crate::ui_theme::UiTheme>,
    positions: Res<crate::windows::WindowPositions>,
    windows: Query<&Window>,
    root: Query<Entity, With<CraftingRoot>>,
    mut last: Local<Option<CraftSignature>>,
) {
    let have = own_item_names(&own, &inventories, &items);
    let craftable: Vec<bool> = content
        .recipes
        .recipes
        .iter()
        .map(|recipe| can_craft(recipe, &have))
        .collect();
    let signature: CraftSignature = (
        state.open,
        state.search.clone(),
        state.category.clone(),
        state.cats_open,
        state.selected.clone(),
        state.scroll.round() as i32,
        registry.generation(),
        craftable.clone(),
    );
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);

    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    if !state.open {
        return;
    }

    let query = state.search.to_lowercase();
    let cats = categories(&content.recipes.recipes);
    let mut matched: Vec<usize> = content
        .recipes
        .recipes
        .iter()
        .enumerate()
        .filter(|(_, recipe)| matches(recipe, &state.category, &query))
        .map(|(index, _)| index)
        .collect();
    // Сортировка по алфавиту (`ConstructionMenuPresenter:316-317`).
    matched.sort_by(|a, b| {
        content.recipes.recipes[*a]
            .name
            .cmp(&content.recipes.recipes[*b].name)
    });

    let content_h = matched.len() as f32 * ROW_STEP;
    let view_h = list_view_h();
    let max_scroll = (content_h - view_h).max(0.0);
    let scroll = state.scroll.clamp(0.0, max_scroll);
    let first = ((scroll / ROW_STEP).floor() as usize).min(matched.len());
    let offset = scroll - first as f32 * ROW_STEP;
    // Полоса прокрутки — как `ScrollContainer`: при видимой полосе список
    // ужимается на её ширину.
    let bar_width = if content_h > view_h + 1e-3 {
        ui::SCROLLBAR_WIDTH
    } else {
        0.0
    };
    let visible: Vec<usize> = {
        let mut rest = matched.split_off(first);
        // +1 строка на границу — чтобы при прокрутке ряд не «дорисовывался».
        let rows = (view_h / ROW_STEP).ceil() as usize + 1;
        rest.truncate(rows);
        rest
    };
    let icons: Vec<Option<ImageNode>> = visible
        .iter()
        .map(|&index| {
            let output = &content.recipes.recipes[index].output.0;
            crate::inventory_ui::item_icon(&registry, &content, output)
                .map(crate::inventory_ui::icon_node)
        })
        .collect();

    // Позиция по умолчанию — ЦЕНТР экрана (`DefaultWindow` в сборке открывается
    // по центру), при сохранённой позиции окна — она. Без UiTransform-сдвига:
    // перетаскивание пишет абсолютные left/top, сдвиг собил бы позицию.
    let (left, top) = match positions.get(crate::windows::WindowKind::Craft) {
        Some(pos) => (px(pos.x), px(pos.y)),
        None => {
            let size = windows
                .iter()
                .next()
                .map(|window| Vec2::new(window.width(), window.height()))
                .unwrap_or(Vec2::new(1280.0, 720.0));
            (
                px(((size.x - WINDOW_W) / 2.0).max(0.0)),
                px(((size.y - WINDOW_H) / 2.0).max(0.0)),
            )
        }
    };
    let node = Node {
        position_type: PositionType::Absolute,
        left,
        top,
        width: px(WINDOW_W),
        max_height: px(WINDOW_H),
        flex_direction: FlexDirection::Column,
        ..default()
    };
    commands
        .spawn((
            CraftingRoot,
            crate::windows::WindowKind::Craft,
            crate::windows::WindowDrag::default(),
            Interaction::default(),
            node,
        ))
        .with_children(|window| {
            crate::hud::window_header(window, &theme, "Строительство");
            let (mut body, background) = crate::hud::window_body();
            body.height = px(WINDOW_H - 25.0);
            window
                .spawn((body, background))
                .with_children(|body| {
                    // Горизонтальный корень XAML: левая колонка + правая.
                    body.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        flex_grow: 1.0,
                        column_gap: px(LEFT_COL_GAP),
                        min_height: px(0),
                        ..default()
                    })
                    .with_children(|columns| {
                        left_column(
                            columns,
                            &state,
                            &content,
                            &cats,
                            &visible,
                            &icons,
                            content_h,
                            view_h,
                            offset,
                            bar_width,
                        );
                        right_column(columns, &state, &content, &registry, &craftable);
                    });
                });
        });
}

/// Строка списка рецептов: иконка 32×32 (Scale 2 в сборке — у нас 28 px в
/// строке 34) + имя; фон `list-container-button` #373744, hover #4B4B56,
/// выбранная строка подсвечена (ItemList selected).
pub(crate) const RECIPE_ROW_STEP: f32 = ROW_STEP;
fn recipe_row(
    parent: &mut ChildSpawnerCommands,
    recipe: &ssr_core::recipes::Recipe,
    icon: Option<ImageNode>,
    selected: bool,
) {
    let tint = if selected {
        crate::hud::HudTint {
            normal: ui::LIST_CONTAINER_HOVER,
            hovered: ui::LIST_CONTAINER_HOVER,
            pressed: ui::LIST_CONTAINER_HOVER,
        }
    } else {
        crate::hud::HudTint {
            normal: ui::LIST_CONTAINER_ROW,
            hovered: ui::LIST_CONTAINER_HOVER,
            pressed: ui::LIST_CONTAINER_ROW,
        }
    };
    parent
        .spawn((
            RecipeRow(recipe.id.clone()),
            Button,
            tint,
            BackgroundColor(if selected {
                ui::LIST_CONTAINER_HOVER
            } else {
                ui::LIST_CONTAINER_ROW
            }),
            Node {
                width: Val::Percent(100.0),
                height: px(ROW_H),
                align_items: AlignItems::Center,
                column_gap: px(6),
                padding: UiRect::horizontal(px(5)),
                ..default()
            },
        ))
        .with_children(|row| {
            // Иконка выхода рецепта, как EntityPrototypeView в RecipeRow.
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
                Text::new(recipe.name.clone()),
                TextFont::from_font_size(ui::FONT_BASE),
                TextColor(ui::TEXT),
            ));
        });
}

/// Левая колонка: поиск + категории + список рецептов.
#[allow(clippy::too_many_arguments)]
fn left_column(
    parent: &mut ChildSpawnerCommands,
    state: &CraftingState,
    content: &ClientContent,
    cats: &[String],
    visible: &[usize],
    icons: &[Option<ImageNode>],
    content_h: f32,
    view_h: f32,
    offset: f32,
    bar_width: f32,
) {
    parent
        .spawn(Node {
            min_width: px(LEFT_COL_W),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            min_height: px(0),
            ..default()
        })
        .with_children(|left| {
            // Верхний ряд: поиск + выпадающий список категорий (XAML).
            left.spawn(Node {
                height: px(FIELD_H),
                column_gap: px(4),
                ..default()
            })
            .with_children(|row| {
                let (field, field_bg) = crate::hud::glass_field();
                row.spawn((
                    CraftSearchField,
                    Button,
                    field,
                    field_bg,
                ))
                .with_child((
                    Text::new(if state.search.is_empty() {
                        "Search".to_string()
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
                row.spawn((CategoryButton, Button, Node {
                    width: px(CATEGORY_W),
                    height: px(FIELD_H),
                    align_items: AlignItems::Center,
                    padding: UiRect::horizontal(px(8)),
                    ..default()
                }))
                .with_children(|button| {
                    let label = if state.category.is_empty() {
                        "Все".to_string()
                    } else {
                        state.category.clone()
                    };
                    button.spawn((
                        Text::new(label),
                        TextFont::from_font_size(ui::FONT_BASE),
                        TextColor(ui::TEXT),
                    ));
                    button.spawn((
                        Text::new(if state.cats_open { "▲" } else { "▼" }),
                        TextFont::from_font_size(ui::FONT_SMALL),
                        TextColor(ui::TEXT_MUTED),
                        Node {
                            margin: UiRect::left(Val::Auto),
                            ..default()
                        },
                    ));
                });
            });
            // Попап категорий поверх списка (OptionButton в сборке).
            if state.cats_open {
                let mut options: Vec<String> = vec![String::new()];
                options.extend(cats.iter().cloned());
                left.spawn(Node {
                    position_type: PositionType::Absolute,
                    // Под кнопкой категории: строка поиска + зазор.
                    left: px(LEFT_COL_W - CATEGORY_W),
                    top: px(FIELD_H + 4.0),
                    width: px(CATEGORY_W),
                    flex_direction: FlexDirection::Column,
                    ..default()
                })
                .with_children(|popup| {
                    for option in options {
                        let label = if option.is_empty() {
                            "Все".to_string()
                        } else {
                            option.clone()
                        };
                        let active = option == state.category;
                        popup
                            .spawn((
                                CategoryOption(option),
                                Button,
                                BackgroundColor(if active {
                                    ui::LIST_CONTAINER_HOVER
                                } else {
                                    ui::LIST_CONTAINER_ROW
                                }),
                                Node {
                                    width: Val::Percent(100.0),
                                    height: px(FIELD_H),
                                    align_items: AlignItems::Center,
                                    padding: UiRect::horizontal(px(8)),
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
            // Список рецептов (ListContainer, Toggle + Group).
            left.spawn(Node {
                height: px(view_h),
                overflow: Overflow::clip(),
                flex_grow: 1.0,
                ..default()
            })
            .with_children(|list| {
                list.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(0),
                        top: px(0),
                        width: px(LEFT_COL_W - bar_width),
                        flex_direction: FlexDirection::Column,
                        row_gap: px(2),
                        ..default()
                    },
                    UiTransform::from_translation(Val2::new(Val::Px(0.0), Val::Px(-offset))),
                ))
                .with_children(|inner| {
                    for (position, &index) in visible.iter().enumerate() {
                        let recipe = &content.recipes.recipes[index];
                        recipe_row(
                            inner,
                            recipe,
                            icons.get(position).cloned().flatten(),
                            state.selected.as_deref() == Some(recipe.id.as_str()),
                        );
                    }
                });
                crate::hud::spawn_scrollbar(
                    list,
                    content_h,
                    view_h,
                    state.scroll,
                    crate::hud::ScrollList::CraftMenu,
                );
            });
        });
}

/// Правая колонка: шапка рецепта (иконка + имя + результат), шаги с иконками,
/// кнопка Build и заглушки Erase/Clear (ConstructionMenuPresenter).
fn right_column(
    parent: &mut ChildSpawnerCommands,
    state: &CraftingState,
    content: &ClientContent,
    registry: &crate::rsi::RsiRegistry,
    craftable: &[bool],
) {
    let selected = state
        .selected
        .as_deref()
        .and_then(|id| content.recipes.by_id(id));
    let can = selected
        .and_then(|recipe| content.recipes.recipes.iter().position(|r| r.id == recipe.id))
        .map(|index| craftable[index])
        .unwrap_or(false);
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            row_gap: px(6),
            min_height: px(0),
            ..default()
        })
        .with_children(|right| {
            // Верхний ряд: «Сеткой» (заглушка — вид сетки из XAML, отложен).
            right
                .spawn((
                    Button,
                    BackgroundColor(ui::GLASS_BUTTON_DISABLED),
                    Node {
                        height: px(22),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                ))
                .with_child((
                    Text::new("Сеткой"),
                    TextFont::from_font_size(ui::FONT_SMALL),
                    TextColor(ui::TEXT_MUTED),
                ));
            // Шапка рецепта: иконка + имя + результат (TargetTexture/Name/Desc).
            if let Some(recipe) = selected {
                let icon = crate::inventory_ui::item_icon(registry, content, &recipe.output.0);
                right
                    .spawn(Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: px(10),
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|header| {
                        header
                            .spawn(Node {
                                width: px(64),
                                height: px(64),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            })
                            .with_children(|cell| {
                                if let Some(icon) = icon {
                                    cell.spawn((
                                        crate::inventory_ui::icon_node(icon),
                                        Node {
                                            width: px(56),
                                            height: px(56),
                                            ..default()
                                        },
                                    ));
                                }
                            });
                        header.spawn(Node {
                            flex_direction: FlexDirection::Column,
                            flex_grow: 1.0,
                            ..default()
                        })
                        .with_children(|labels| {
                            labels.spawn((
                                Text::new(recipe.name.clone()),
                                TextFont::from_font_size(ui::FONT_LABEL),
                                TextColor(ui::NANO_GOLD),
                            ));
                            labels.spawn((
                                Text::new(format!(
                                    "Результат: {} ×{}",
                                    content.items.name_of(&recipe.output.0),
                                    recipe.output.1
                                )),
                                TextFont::from_font_size(ui::FONT_BASE),
                                TextColor(ui::TEXT_MUTED),
                            ));
                        });
                    });
                // Шаги (RecipeStepList): каждый вход и результат с иконками.
                right
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        row_gap: px(4),
                        overflow: Overflow::clip(),
                        ..default()
                    })
                    .with_children(|steps| {
                        for (id, count) in &recipe.inputs {
                            step_row(steps, content, registry, id, *count, false);
                        }
                        step_row(
                            steps,
                            content,
                            registry,
                            &recipe.output.0,
                            recipe.output.1,
                            true,
                        );
                    });
                // Build: Enabled при крафтабельности (BuildButton в XAML —
                // toggle призрака строительства; у нас сервер крафтит сразу).
                right
                    .spawn((
                        BuildButton(recipe.id.clone()),
                        Button,
                        BackgroundColor(if can {
                            ui::GLASS_BUTTON
                        } else {
                            ui::GLASS_BUTTON_DISABLED
                        }),
                        Node {
                            height: px(BUILD_H),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                    ))
                    .with_child((
                        Text::new("Создать"),
                        TextFont::from_font_size(ui::FONT_BASE),
                        TextColor(if can { ui::TEXT } else { ui::TEXT_MUTED }),
                    ));
            } else {
                right.spawn(Node {
                    flex_grow: 1.0,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                })
                .with_child((
                    Text::new("Выберите рецепт в списке"),
                    TextFont::from_font_size(ui::FONT_BASE),
                    TextColor(ui::TEXT_MUTED),
                ));
            }
            // Ряд Erase (0.7) / Clear (0.3) — заглушки (Eraser Mode/Clear All
            // работают со строительными призраками).
            right
                .spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: px(4),
                    ..default()
                })
                .with_children(|row| {
                    for (label, ratio) in [("Режим ластика", ERASE_RATIO), ("Очистить всё", 1.0 - ERASE_RATIO)] {
                        row.spawn((
                            Button,
                            BackgroundColor(ui::GLASS_BUTTON_DISABLED),
                            Node {
                                height: px(24),
                                flex_grow: ratio,
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                        ))
                        .with_child((
                            Text::new(label),
                            TextFont::from_font_size(ui::FONT_SMALL),
                            TextColor(ui::TEXT_MUTED),
                        ));
                    }
                });
        });
}

/// Шаг рецепта: иконка 16 + текст; результат — с отступом-стрелкой (PadLeft).
fn step_row(
    parent: &mut ChildSpawnerCommands,
    content: &ClientContent,
    registry: &crate::rsi::RsiRegistry,
    item_id: &str,
    count: u32,
    output: bool,
) {
    let icon = crate::inventory_ui::item_icon(registry, content, item_id);
    let label = if output {
        format!("Получится: {} ×{}", content.items.name_of(item_id), count)
    } else {
        format!("{} ×{}", content.items.name_of(item_id), count)
    };
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: px(6),
            padding: UiRect::left(px(if output { 16.0 } else { 0.0 })),
            ..default()
        })
        .with_children(|row| {
            row.spawn(Node {
                width: px(20),
                height: px(20),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                ..default()
            })
            .with_children(|cell| {
                if let Some(icon) = icon {
                    cell.spawn((
                        crate::inventory_ui::icon_node(icon),
                        Node {
                            width: px(16),
                            height: px(16),
                            ..default()
                        },
                    ));
                }
            });
            row.spawn((
                Text::new(label),
                TextFont::from_font_size(ui::FONT_BASE),
                TextColor(if output { ui::TEXT } else { ui::TEXT_MUTED }),
            ));
        });
}

/// Клики окна: поле поиска, категория, строки рецептов, кнопка Build.
#[allow(clippy::too_many_arguments)]
pub fn craft_ui_clicks(
    mut state: ResMut<CraftingState>,
    content: Res<ClientContent>,
    own: Res<OwnPlayerEntity>,
    inventories: Query<&Inventory>,
    items: Query<&Item>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    search: Query<&Interaction, (Changed<Interaction>, With<CraftSearchField>)>,
    cats_button: Query<&Interaction, (Changed<Interaction>, With<CategoryButton>)>,
    cats_options: Query<(&Interaction, &CategoryOption), Changed<Interaction>>,
    rows: Query<(&Interaction, &RecipeRow), Changed<Interaction>>,
    build: Query<(&Interaction, &BuildButton), Changed<Interaction>>,
) {
    for interaction in search.iter() {
        if *interaction == Interaction::Pressed {
            state.search_focused = true;
        }
    }
    for interaction in cats_button.iter() {
        if *interaction == Interaction::Pressed {
            state.cats_open = !state.cats_open;
        }
    }
    for (interaction, option) in cats_options.iter() {
        if *interaction == Interaction::Pressed {
            state.category = option.0.clone();
            state.cats_open = false;
            // Смена категории очищает поиск (ConstructionMenuPresenter).
            state.search.clear();
            state.scroll = 0.0;
            state.scroll_target = 0.0;
        }
    }
    for (interaction, row) in rows.iter() {
        if *interaction == Interaction::Pressed {
            state.selected = Some(row.0.clone());
        }
    }
    for (interaction, button) in build.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(recipe) = content.recipes.by_id(&button.0) else {
            continue;
        };
        let have = own_item_names(&own, &inventories, &items);
        if !can_craft(recipe, &have) {
            continue; // кнопка серая: крафтить рано
        }
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Craft {
                recipe: button.0.clone(),
            });
        }
        tracing::info!(recipe = %button.0, "craft sent");
    }
}

/// Ввод в поле поиска окна крафта (клик по полю включает ввод — как в
/// спавн-меню, иначе буквы «съедались» горячими клавишами).
pub fn craft_input(
    mut events: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<CraftingState>,
) {
    if !state.open || !state.search_focused {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) || keys.just_pressed(KeyCode::KeyG) {
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
            }
            Key::Space => {
                state.search.push(' ');
            }
            Key::Character(text) => {
                state.search.push_str(text);
            }
            _ => {}
        }
        state.scroll = 0.0;
        state.scroll_target = 0.0;
    }
}

/// Прокрутка списка рецептов колесом (шаг 50 px, как `ScrollContainer`).
pub fn craft_scroll_wheel(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    chat: Res<crate::chat::ChatState>,
    mut state: ResMut<CraftingState>,
    content: Res<ClientContent>,
) {
    if !state.open || chat.focused {
        wheel.clear();
        return;
    }
    let max_scroll = (matched_count(&state, &content) as f32 * ROW_STEP - list_view_h()).max(0.0);
    for event in wheel.read() {
        state.scroll_target = (state.scroll_target - event.y * ui::SCROLLBAR_WHEEL_STEP)
            .clamp(0.0, max_scroll);
    }
}

/// Догоняет цель прокрутки экспонентой (`LerpAnimate(rate: 15)` в движке).
pub fn craft_scroll_anim(time: Res<Time>, mut state: ResMut<CraftingState>) {
    let k = 1.0 - (-ui::SCROLLBAR_ANIM_RATE * time.delta_secs()).exp();
    state.scroll += (state.scroll_target - state.scroll) * k;
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
