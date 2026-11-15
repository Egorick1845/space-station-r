//! Контекстное меню (ПКМ) — по образцу SS14:
//! - `Content.Client/ContextMenu/UI/ContextMenuElement.xaml` — строка 32 px:
//!   колонка иконки 32×32 (даже без иконки — текст выровнен), текст, справа
//!   стрелка подменю (`VerbIcons/group.svg`);
//! - `ContextMenuPopup` — до 10 строк, дальше прокрутка; категория вербов
//!   (`VerbMenuUIController.AddVerbCategory`) открывает ПОДМЕНЮ справа при
//!   наведении (`HoverDelay = 0.2` с) или клике;
//! - `EntityMenuUIController` — если под курсором несколько сущностей, сначала
//!   список сущностей, выбор — меню её вербов;
//! - цвета `StyleNano`: `ButtonColorContext #2d3341d9`, `…Hover #3d4556`,
//!   `…Pressed #404352`, `…Disabled` — чёрный.

use bevy::prelude::*;
use ssr_core::verbs::VerbCategory;
use ssr_protocol::ActionOption;

use crate::inventory_ui::{ActionMenu, IconCache, icon_texture};

const ELEMENT: f32 = 32.0;
const MARGIN: f32 = 2.0;
const MAX_ROWS: usize = 10;
const BG_POPUP: Color = Color::srgba(0.118, 0.118, 0.133, 0.97);
const BG_NORMAL: Color = Color::srgba(0.176, 0.200, 0.255, 0.85);
const BG_HOVER: Color = Color::srgb(0.239, 0.271, 0.337);
const BG_PRESSED: Color = Color::srgb(0.251, 0.263, 0.322);
const BG_DISABLED: Color = Color::srgb(0.02, 0.02, 0.025);
const TEXT: Color = Color::srgb(0.92, 0.92, 0.94);
const TEXT_DISABLED: Color = Color::srgb(0.45, 0.45, 0.48);
const SEPARATOR: Color = Color::srgb(0.30, 0.30, 0.36);
const EXPAND_ICON: &str = "Interface/VerbIcons/group.svg.192dpi.png";

/// Корень меню.
#[derive(Component)]
pub struct ContextMenuRoot;

/// Что делает строка меню.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuRow {
    /// Верб с индексом в `ActionMenu.options`.
    Verb(usize),
    /// Категория — открыть подменю.
    Category(VerbCategory),
    /// Сущность из списка под курсором (индекс в `ActionMenu.entities`).
    Entity(usize),
}

/// Сущность под курсором для `EntityMenu`: bits, имя, иконка `path#state`.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntity {
    pub bits: u64,
    pub name: String,
    pub icon: Option<String>,
}

/// Иконка строки: RSI (`path#state`) или PNG.
fn row_icon(
    row: &mut ChildSpawnerCommands,
    icon: Option<&str>,
    registry: &crate::rsi::RsiRegistry,
    icons: &mut IconCache,
    assets: &AssetServer,
) {
    let node = Node {
        width: px(ELEMENT),
        height: px(ELEMENT),
        flex_shrink: 0.0,
        ..default()
    };
    let image = icon.and_then(|path| {
        if path.contains('#') {
            registry.get(&format!("sprites/ss14/{path}")).map(|sprite| {
                ImageNode::from_atlas_image(
                    sprite.image.clone(),
                    TextureAtlas {
                        layout: sprite.layout.clone(),
                        index: sprite.index(0, 0),
                    },
                )
            })
        } else {
            Some(ImageNode::new(icon_texture(icons, assets, path)))
        }
    });
    match image {
        Some(image) => {
            row.spawn((image, node));
        }
        // Пустая колонка иконки — текст всех строк начинается ровно.
        None => {
            row.spawn(node);
        }
    }
}

/// Строка меню: иконка, текст, (стрелка подменю).
#[allow(clippy::too_many_arguments)]
fn element(
    list: &mut ChildSpawnerCommands,
    action: MenuRow,
    label: &str,
    font: TextFont,
    icon: Option<&str>,
    disabled: bool,
    expands: bool,
    open: bool,
    ctx: &mut (&crate::rsi::RsiRegistry, &mut IconCache, &AssetServer),
) -> Entity {
    let base = if disabled {
        BG_DISABLED
    } else if open {
        BG_HOVER
    } else {
        BG_NORMAL
    };
    let mut entity = list.spawn((
        Node {
            min_height: px(ELEMENT),
            min_width: px(180),
            margin: UiRect::all(px(MARGIN)),
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(base),
    ));
    if !disabled {
        entity.insert((action, Button));
    }
    let id = entity.id();
    entity.with_children(|row| {
        row_icon(row, icon, ctx.0, ctx.1, ctx.2);
        row.spawn((
            Text::new(label.to_string()),
            font,
            TextColor(if disabled { TEXT_DISABLED } else { TEXT }),
            Node {
                margin: UiRect::horizontal(px(4)),
                max_width: px(300),
                flex_grow: 1.0,
                ..default()
            },
        ));
        if expands {
            row.spawn((
                ImageNode::new(icon_texture(ctx.1, ctx.2, EXPAND_ICON)),
                Node {
                    width: px(16),
                    height: px(16),
                    margin: UiRect::horizontal(px(8)),
                    ..default()
                },
            ));
        }
    });
    id
}

/// Шрифт строки по типу верба (`InteractionVerb` — жирный курсив и т.д.).
fn verb_font(option: &ActionOption) -> TextFont {
    let size = match option.style_class() {
        "InteractionVerb" | "ActivationVerb" => 13.0,
        _ => 12.5,
    };
    TextFont::from_font_size(size)
}

/// Подпись с причиной недоступности.
fn verb_label(option: &ActionOption) -> String {
    match (&option.message, option.disabled) {
        (Some(message), true) => format!("{} ({message})", option.label),
        _ => option.label.clone(),
    }
}

/// Сигнатура для перерисовки.
type Signature = (usize, usize, Vec2, Option<VerbCategory>);

/// Перерисовка меню: список сущностей или вербы (категории — подменю).
#[allow(clippy::too_many_arguments)]
pub fn render_context_menu(
    mut commands: Commands,
    menu: Res<ActionMenu>,
    roots: Query<Entity, With<ContextMenuRoot>>,
    registry: Res<crate::rsi::RsiRegistry>,
    assets: Res<AssetServer>,
    mut icons: ResMut<IconCache>,
    windows: Query<&Window>,
    mut last: Local<Option<Signature>>,
) {
    let signature = (
        menu.options.len(),
        menu.entities.len(),
        menu.cursor,
        menu.category,
    );
    if last.as_ref() == Some(&signature) && !registry.is_changed() {
        return;
    }
    *last = Some(signature);
    for root in roots.iter() {
        commands.entity(root).despawn();
    }
    if menu.options.is_empty() && menu.entities.is_empty() {
        return;
    }
    // Не вылезать за край окна (раньше были жёсткие 1100×560).
    let (width, height) = windows
        .single()
        .map(|w| (w.width(), w.height()))
        .unwrap_or((1280.0, 720.0));
    let rows = if menu.entities.is_empty() {
        let categories = top_level(&menu.options).len();
        categories.min(MAX_ROWS)
    } else {
        menu.entities.len().min(MAX_ROWS)
    };
    let menu_height = rows as f32 * (ELEMENT + 2.0 * MARGIN) + 8.0;
    let left = menu.cursor.x.min(width - 240.0).max(0.0);
    // Места справа под подменю нет — открываем слева (`altPos` в
    // `ContextMenuUIController.OpenSubMenu`).
    let sub_on_left = left + 2.0 * 230.0 > width;
    let top = menu.cursor.y.min(height - menu_height).max(0.0);
    let mut ctx = (registry.as_ref(), icons.as_mut(), assets.as_ref());
    commands
        .spawn((
            ContextMenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(left),
                top: px(top),
                max_height: px(MAX_ROWS as f32 * (ELEMENT + 2.0 * MARGIN) + 8.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(2)),
                overflow: Overflow::clip_y(),
                ..default()
            },
            BackgroundColor(BG_POPUP),
            GlobalZIndex(60),
        ))
        .with_children(|list| {
            if !menu.entities.is_empty() {
                for (index, entity) in menu.entities.iter().enumerate() {
                    element(
                        list,
                        MenuRow::Entity(index),
                        &entity.name,
                        TextFont::from_font_size(13.0),
                        entity.icon.as_deref(),
                        false,
                        true,
                        false,
                        &mut ctx,
                    );
                }
                return;
            }
            let mut last_kind = None;
            for item in top_level(&menu.options) {
                match item {
                    TopItem::Verb(index) => {
                        let option = &menu.options[index];
                        // Разделитель между типами вербов.
                        if last_kind.is_some_and(|kind| kind != option.kind) {
                            list.spawn((
                                Node {
                                    height: px(1),
                                    margin: UiRect::horizontal(px(4)),
                                    ..default()
                                },
                                BackgroundColor(SEPARATOR),
                            ));
                        }
                        last_kind = Some(option.kind);
                        element(
                            list,
                            MenuRow::Verb(index),
                            &verb_label(option),
                            verb_font(option),
                            option.icon.as_deref(),
                            option.disabled,
                            false,
                            false,
                            &mut ctx,
                        );
                    }
                    TopItem::Category(category) => {
                        let open = menu.category == Some(category);
                        let row = element(
                            list,
                            MenuRow::Category(category),
                            category.text(),
                            TextFont::from_font_size(13.0),
                            category.icon(),
                            false,
                            true,
                            open,
                            &mut ctx,
                        );
                        if open {
                            // Подменю — справа от строки категории.
                            let mut ctx2 = (ctx.0, &mut *ctx.1, ctx.2);
                            list.commands().entity(row).with_children(|row| {
                                let (sub_left, sub_right) = if sub_on_left {
                                    (Val::Auto, Val::Percent(100.0))
                                } else {
                                    (Val::Percent(100.0), Val::Auto)
                                };
                                row.spawn((
                                    Node {
                                        position_type: PositionType::Absolute,
                                        left: sub_left,
                                        right: sub_right,
                                        top: px(-2),
                                        flex_direction: FlexDirection::Column,
                                        padding: UiRect::all(px(2)),
                                        ..default()
                                    },
                                    BackgroundColor(BG_POPUP),
                                ))
                                .with_children(|sub| {
                                    for (index, option) in menu.options.iter().enumerate() {
                                        if option.category != Some(category) {
                                            continue;
                                        }
                                        element(
                                            sub,
                                            MenuRow::Verb(index),
                                            &verb_label(option),
                                            verb_font(option),
                                            option.icon.as_deref(),
                                            option.disabled,
                                            false,
                                            false,
                                            &mut ctx2,
                                        );
                                    }
                                });
                            });
                        }
                    }
                }
            }
        });
}

/// Элемент верхнего уровня: верб без категории или категория (один раз).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TopItem {
    Verb(usize),
    Category(VerbCategory),
}

fn top_level(options: &[ActionOption]) -> Vec<TopItem> {
    let mut items = Vec::new();
    for (index, option) in options.iter().enumerate() {
        match option.category {
            None => items.push(TopItem::Verb(index)),
            Some(category) => {
                if !items.contains(&TopItem::Category(category)) {
                    items.push(TopItem::Category(category));
                }
            }
        }
    }
    items
}

/// Подсветка строк под курсором; наведение на категорию через 0.2 с
/// открывает её подменю (`ContextMenuUIController.HoverDelay`).
#[allow(clippy::type_complexity)]
pub fn context_menu_hover(
    time: Res<Time>,
    mut menu: ResMut<ActionMenu>,
    mut rows: Query<(&Interaction, &MenuRow, &mut BackgroundColor)>,
    mut hover: Local<(Option<VerbCategory>, f32)>,
) {
    let mut hovered_category = None;
    for (interaction, row, mut color) in rows.iter_mut() {
        let open = matches!(row, MenuRow::Category(c) if menu.category == Some(*c));
        color.0 = match interaction {
            Interaction::Pressed => BG_PRESSED,
            Interaction::Hovered => BG_HOVER,
            Interaction::None if open => BG_HOVER,
            Interaction::None => BG_NORMAL,
        };
        if *interaction != Interaction::None
            && let MenuRow::Category(category) = row
        {
            hovered_category = Some(*category);
        }
    }
    match hovered_category {
        Some(category) if menu.category != Some(category) => {
            if hover.0 == Some(category) {
                hover.1 += time.delta_secs();
                if hover.1 >= 0.2 {
                    menu.category = Some(category);
                }
            } else {
                *hover = (Some(category), 0.0);
            }
        }
        _ => *hover = (None, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssr_core::verbs::VerbType;
    use ssr_protocol::ActionKind;

    fn verb(label: &str, category: Option<VerbCategory>) -> ActionOption {
        ActionOption {
            label: label.into(),
            action: ActionKind::Examine { entity: 0 },
            kind: VerbType::Verb,
            category,
            icon: None,
            priority: 0,
            disabled: false,
            message: None,
            close_menu: None,
            client_exclusive: false,
            confirmation_popup: false,
        }
    }

    #[test]
    fn categories_collapse_to_one_row() {
        let options = vec![
            verb("Осмотреть", None),
            verb("Оживить", Some(VerbCategory::Debug)),
            verb("Удалить", Some(VerbCategory::Debug)),
            verb("15", Some(VerbCategory::Split)),
        ];
        assert_eq!(
            top_level(&options),
            vec![
                TopItem::Verb(0),
                TopItem::Category(VerbCategory::Debug),
                TopItem::Category(VerbCategory::Split)
            ]
        );
    }
}

/// Тест `SSR_CONTEXT_TEST=1`: как только пришли вербы с категориями — открыть
/// первую (снимок подменю без мыши).
pub fn context_menu_test(mut menu: ResMut<ActionMenu>, mut done: Local<bool>) {
    if *done || std::env::var_os("SSR_CONTEXT_TEST").is_none() || menu.category.is_some() {
        return;
    }
    if let Some(category) = menu.options.iter().find_map(|o| o.category) {
        menu.category = Some(category);
        *done = true;
    }
}
