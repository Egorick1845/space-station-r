//! Окно выбора внешности (по мотивам лобби SS14: `HumanoidProfileEditor` и
//! `SingleMarkingPicker` из сборки): пол, причёска и борода — списками с
//! превью (строка `ItemList` = иконка + имя, поиск над списком) и палитрой
//! цветов. Изменения сразу уходят на сервер (`SetAppearance`) и видны на своей
//! кукле — она и служит превью. Открывается клавишей P (в SS14 редактор
//! внешности живёт в лобби).

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::mechanics::{FacialHair, Hair, Sex, facial_hair_style_names, hair_style_names};
use ssr_protocol::ClientMessage;
use ssr_protocol::net::GameChannel;

use crate::ui_theme as ui;

/// Палитра цветов волос (тёмный, каштановый, рыжий, блонд, седой, зелёный).
pub const HAIR_COLORS: [[u8; 3]; 6] = [
    [0x1e, 0x1a, 0x18],
    [0x6b, 0x4a, 0x2f],
    [0xb0, 0x53, 0x1f],
    [0xe0, 0xc0, 0x6a],
    [0xb8, 0xb8, 0xb8],
    [0x4a, 0x7a, 0x40],
];

/// Пути маркингов в реестре RSI (сборка: `Mobs/Customization/<файл>.rsi`).
const HAIR_RSI: &str = "sprites/ss14/Mobs/Customization/human_hair.rsi";
const FACIAL_HAIR_RSI: &str = "sprites/ss14/Mobs/Customization/human_facial_hair.rsi";

/// Видимых строк в списках: остальное — колесо мыши и поиск. В SS14 список
/// прокручивается целиком (`SingleMarkingPicker`), у нас — построчная
/// прокрутка, как в спавн-меню.
const HAIR_ROWS: usize = 6;
const BEARD_ROWS: usize = 3;
/// Высота строки списка стилей (иконка 32×32 + имя, как ItemList в движке).
const STYLE_ROW_H: f32 = 32.0;

/// Состояние окна внешности (индексы — в полных списках из RSI).
#[derive(Resource, Default)]
pub struct AppearanceUi {
    pub open: bool,
    /// Индекс в [`hair_style_names`].
    pub hair: usize,
    /// 0 — «нет бороды», иначе 1 + индекс в [`facial_hair_style_names`].
    pub beard: usize,
    pub color: usize,
    pub female: bool,
    /// Прокрутка списков в пикселях (непрерывная, как `ScrollContainer` в SS14);
    /// `*_target` — цель, текущее значение догоняет её с rate 15.
    pub hair_scroll: f32,
    pub hair_scroll_target: f32,
    pub beard_scroll: f32,
    pub beard_scroll_target: f32,
    pub search: String,
    pub search_focused: bool,
    /// Текущая внешность подтянута из компонентов игрока (на первое открытие).
    pub synced: bool,
}

/// Внутренний контейнер списка: прокрутка двигает его целиком, поэтому при
/// прокрутке окно НЕ пересобирается и ничего не мигает (раньше каждое
/// изменение прокрутки вызывало despawn+respawn — на кадр окно исчезало).
#[derive(Component, Clone, Copy)]
pub struct ScrollInner {
    pub beard: bool,
}

/// Сколько причёсок проходит поиск (без аллокации списка).
fn hair_count(search: &str) -> usize {
    let query = search.to_lowercase();
    hair_style_names()
        .iter()
        .filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
        .count()
}

/// Сколько строк в списке бород («нет» + стили) проходит поиск.
fn beard_count(search: &str) -> usize {
    let query = search.to_lowercase();
    1 + facial_hair_style_names()
        .iter()
        .filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
        .count()
}

/// Строка списка стилей: клик — выбрать. `index` — глобальный индекс
/// (у бороды 0 — «нет»).
#[derive(Component, Clone, Copy)]
pub struct AppearancePick {
    pub beard: bool,
    pub index: usize,
}

/// Квадратик палитры цветов волос.
#[derive(Component, Clone, Copy)]
pub struct AppearanceSwatch(pub usize);

/// Список стилей: колесо мыши крутит тот список, над которым курсор.
#[derive(Component, Clone, Copy)]
pub struct AppearanceList {
    pub beard: bool,
}

/// Стрелка «пола» в окне внешности.
#[derive(Component, Clone, Copy)]
pub struct AppearanceStep {
    pub forward: bool,
}

/// Строка поиска по спискам (клик — включить ввод, как у спавн-меню).
#[derive(Component)]
pub struct AppearanceSearchField;

/// Кнопка «Очистить» рядом с поиском.
#[derive(Component)]
pub struct AppearanceClearButton;

/// Отпечаток окна внешности для сравнения при перерисовке.
type AppearanceSignature = (bool, usize, usize, bool, usize, String, u32);

/// Клики по строкам списков, квадратикам и стрелкам окна внешности.
type AppearanceClicks<'w, 's> = Query<
    'w,
    's,
    (
        &'static Interaction,
        Option<&'static AppearancePick>,
        Option<&'static AppearanceSwatch>,
        Option<&'static AppearanceStep>,
    ),
    (Changed<Interaction>, With<Button>),
>;

/// Список причёсок, отфильтрованный поиском: (глобальный индекс, имя).
fn filtered_hair(search: &str) -> Vec<(usize, &'static str)> {
    let query = search.to_lowercase();
    hair_style_names()
        .iter()
        .enumerate()
        .filter(|(_, name)| query.is_empty() || name.to_lowercase().contains(&query))
        .map(|(index, name)| (index, name.as_str()))
        .collect()
}

/// Список бород, отфильтрованный поиском (индекс 0 — «нет»).
fn filtered_beard(search: &str) -> Vec<(usize, &'static str)> {
    let query = search.to_lowercase();
    std::iter::once((0usize, "нет"))
        .chain(
            facial_hair_style_names()
                .iter()
                .enumerate()
                .map(|(index, name)| (index + 1, name.as_str())),
        )
        .filter(|(index, name)| {
            *index == 0 || query.is_empty() || name.to_lowercase().contains(&query)
        })
        .collect()
}

/// Открывает/закрывает окно внешности. Хоткей P снят (владелец: «убери окно
/// смены причёски на P») — функция остаётся для тестового флага SSR_APPEARANCE.
#[allow(dead_code)]
pub fn appearance_hotkey(
    keys: Res<ButtonInput<KeyCode>>,
    mut ui_state: ResMut<AppearanceUi>,
    console: Res<crate::console::Console>,
    chat: Res<crate::chat::ChatState>,
) {
    if console.open || chat.focused {
        return;
    }
    // Пока идёт ввод в поиске, «p» — это буква, а не горячая клавиша.
    if keys.just_pressed(KeyCode::KeyP) && !ui_state.search_focused {
        ui_state.open = !ui_state.open;
        ui_state.search_focused = false;
        if !ui_state.open {
            ui_state.synced = false;
        }
        tracing::info!(open = ui_state.open, "appearance window toggled");
    }
}

/// Подтягивает внешность своего игрока при первом открытии окна: стиль на
/// спавне случайный, и списки должны открыться на нём, а не на первом элементе.
pub fn appearance_sync(
    mut ui_state: ResMut<AppearanceUi>,
    own: Res<crate::inventory_ui::OwnPlayerEntity>,
    hairs: Query<&Hair>,
    beards: Query<&FacialHair>,
    sexes: Query<&Sex>,
) {
    // Тест-режим SSR_APPEARANCE=1: окно открывается само (скриншот-проверка).
    if !ui_state.open && std::env::var_os("SSR_APPEARANCE").is_some() {
        ui_state.open = true;
    }
    if !ui_state.open {
        ui_state.synced = false;
        return;
    }
    if ui_state.synced {
        return;
    }
    let Some(player) = own.0 else {
        return;
    };
    ui_state.synced = true;
    if let Ok(hair) = hairs.get(player) {
        if let Some(index) = hair_style_names()
            .iter()
            .position(|name| name == &hair.style)
        {
            ui_state.hair = index;
        }
        if let Some(index) = HAIR_COLORS.iter().position(|color| color == &hair.color) {
            ui_state.color = index;
        }
    }
    ui_state.beard = beards
        .get(player)
        .ok()
        .and_then(|beard| {
            facial_hair_style_names()
                .iter()
                .position(|name| name == &beard.style)
        })
        .map_or(0, |index| index + 1);
    if let Ok(sex) = sexes.get(player) {
        ui_state.female = matches!(sex, Sex::Female);
    }
}

/// Рисует окно: пол стрелками, списки стилей с превью и поиском, палитра.
pub fn render_appearance_menu(
    mut commands: Commands,
    state: Res<AppearanceUi>,
    theme: Res<crate::ui_theme::UiTheme>,
    registry: Res<crate::rsi::RsiRegistry>,
    root: Query<Entity, With<crate::hud::AppearanceRoot>>,
    mut last: Local<Option<AppearanceSignature>>,
) {
    let signature = (
        state.open,
        state.hair,
        state.beard,
        state.female,
        state.color,
        // Прокрутка в подписи НЕ участвует: её применяет отдельная система
        // сдвигом строк, иначе окно пересобиралось бы на каждый пиксель.
        state.search.clone(),
        registry.generation(),
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

    let color = HAIR_COLORS[state.color.min(HAIR_COLORS.len() - 1)];
    let hair_name = hair_style_names()
        .get(state.hair)
        .map(String::as_str)
        .unwrap_or("—")
        .to_string();
    let beard_name = if state.beard == 0 {
        "нет".to_string()
    } else {
        facial_hair_style_names()
            .get(state.beard - 1)
            .cloned()
            .unwrap_or_else(|| "нет".to_string())
    };
    let sex = if state.female { "female" } else { "male" };

    // Списки: прокрутка в пикселях (непрерывная), как `ScrollContainer` в SS14.
    let hair_all = filtered_hair(&state.search);
    let beard_all = filtered_beard(&state.search);
    let hair_scroll = clamp_scroll(&state.hair_scroll, hair_all.len(), HAIR_ROWS);
    let beard_scroll = clamp_scroll(&state.beard_scroll, beard_all.len(), BEARD_ROWS);

    let window = crate::hud::menu_panel(
        &mut commands,
        &theme,
        "Внешность",
        50.0,
        50.0,
        WINDOW_W,
        |panel| {
            // Строка пола: подпись + стрелки (в SS14 — выпадающий список).
            panel
                .spawn(Node {
                    width: Val::Percent(100.0),
                    height: px(26),
                    align_items: AlignItems::Center,
                    column_gap: px(4),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new(format!("Пол: {sex}")),
                        TextFont::from_font_size(ui::FONT_BASE),
                        TextColor(ui::TEXT),
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                    ));
                    for forward in [false, true] {
                        row.spawn((
                            AppearanceStep { forward },
                            Button,
                            crate::hud::HudTint::button(),
                            BackgroundColor(ui::GLASS_BUTTON),
                            Node {
                                width: px(28),
                                height: px(22),
                                align_items: AlignItems::Center,
                                justify_content: JustifyContent::Center,
                                ..default()
                            },
                        ))
                        .with_child((
                            Text::new(if forward { ">" } else { "<" }),
                            TextFont::from_font_size(ui::FONT_BASE),
                            TextColor(ui::TEXT),
                        ));
                    }
                });
            // Поиск по обоим спискам: поле ввода + «Очистить».
            panel
                .spawn(Node {
                    width: Val::Percent(100.0),
                    height: px(24),
                    column_gap: px(4),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        AppearanceSearchField,
                        Button,
                        Node {
                            flex_grow: 1.0,
                            height: px(24),
                            align_items: AlignItems::Center,
                            padding: UiRect::new(px(8), px(8), px(4), px(4)),
                            ..default()
                        },
                        BackgroundColor(ui::GLASS_LINEEDIT),
                    ))
                    .with_child((
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
                        AppearanceClearButton,
                        Button,
                        crate::hud::HudTint::button(),
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
            // Список причёсок: строка = иконка RSI + имя (ItemList в SS14).
            panel.spawn((
                Text::new(format!("Причёска: {hair_name}")),
                TextFont::from_font_size(ui::FONT_SMALL),
                TextColor(ui::NANO_GOLD),
            ));
            style_list(
                panel,
                &registry,
                HAIR_RSI,
                false,
                &hair_all,
                state.hair,
                HAIR_ROWS,
                hair_scroll,
            );
            panel.spawn((
                Text::new(format!("Борода: {beard_name}")),
                TextFont::from_font_size(ui::FONT_SMALL),
                TextColor(ui::NANO_GOLD),
            ));
            style_list(
                panel,
                &registry,
                FACIAL_HAIR_RSI,
                true,
                &beard_all,
                state.beard,
                BEARD_ROWS,
                beard_scroll,
            );
            // Палитра: квадратики цвета, выбранный обведён акцентом.
            panel
                .spawn(Node {
                    width: Val::Percent(100.0),
                    height: px(26),
                    align_items: AlignItems::Center,
                    column_gap: px(6),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Text::new("Цвет волос:"),
                        TextFont::from_font_size(ui::FONT_SMALL),
                        TextColor(ui::TEXT_MUTED),
                    ));
                    for (index, swatch) in HAIR_COLORS.iter().enumerate() {
                        let selected = index == state.color;
                        row.spawn((
                            AppearanceSwatch(index),
                            Button,
                            BackgroundColor(Color::srgb_u8(swatch[0], swatch[1], swatch[2])),
                            BorderColor::from(if selected {
                                ui::ACCENT
                            } else {
                                ui::GLASS_BUTTON
                            }),
                            Node {
                                width: px(22),
                                height: px(22),
                                border: UiRect::all(px(2)),
                                ..default()
                            },
                        ));
                    }
                });
            panel.spawn((
                Text::new(format!(
                    "цвет #{:02X}{:02X}{:02X} · колесо — прокрутка · P/Esc — закрыть",
                    color[0], color[1], color[2]
                )),
                TextFont::from_font_size(ui::FONT_SMALL),
                TextColor(ui::TEXT_MUTED),
            ));
        },
    );
    commands.entity(window).insert(crate::hud::AppearanceRoot);
}

/// Шаг строки списка (высота строки + зазор 2 px, как зазор в `ItemList`).
const STYLE_ROW_STEP: f32 = STYLE_ROW_H + 2.0;
/// Ширина окна внешности и ширина контента внутри тела окна (минус отступы
/// `WINDOW_CONTENT_MARGIN`, как в `window_body`).
const WINDOW_W: f32 = 430.0;
const LIST_CONTENT_W: f32 = WINDOW_W - 2.0 * ui::WINDOW_CONTENT_MARGIN;

/// Высота вьюпорта списка на N строк (последний зазор не считаем).
fn list_view_h(visible_rows: usize) -> f32 {
    visible_rows as f32 * STYLE_ROW_STEP - 2.0
}

/// Высота контента списка на `rows` строк.
fn list_content_h(rows: usize) -> f32 {
    rows as f32 * STYLE_ROW_STEP
}

/// Прокрутка, зажатая в `0..(контент − вьюпорт)` (как `Range` в `ScrollBar.cs`).
fn clamp_scroll(scroll: &f32, rows: usize, visible_rows: usize) -> f32 {
    let max = (list_content_h(rows) - list_view_h(visible_rows)).max(0.0);
    scroll.clamp(0.0, max)
}

/// Догоняет цель прокрутки экспонентой (в движке `LerpAnimate(rate: 15)`).
pub fn appearance_scroll_anim(time: Res<Time>, mut state: ResMut<AppearanceUi>) {
    let k = 1.0 - (-ui::SCROLLBAR_ANIM_RATE * time.delta_secs()).exp();
    state.hair_scroll += (state.hair_scroll_target - state.hair_scroll) * k;
    state.beard_scroll += (state.beard_scroll_target - state.beard_scroll) * k;
}

/// Перетаскивание граббера: список, курсор в момент захвата, прокрутка.
#[derive(Resource, Default)]
pub struct ScrollDrag {
    active: Option<(crate::hud::ScrollList, f32, f32)>,
}

/// Тянется ТОЛЬКО граббер (в движке клик по дорожке ничего не делает):
/// `value = старт + Δy / (h − 10) · (контент − вьюпорт)`, как в `ScrollBar.cs`.
#[allow(clippy::too_many_arguments)]
pub fn appearance_scrollbar_drag(
    mut drag: ResMut<ScrollDrag>,
    mut state: ResMut<AppearanceUi>,
    mut hud: ResMut<crate::hud::HudState>,
    mut craft: ResMut<crate::crafting::CraftingState>,
    cache: Res<crate::hud::SpawnListCache>,
    content: Res<crate::content::ClientContent>,
    grabbers: Query<(&Interaction, &crate::hud::ScrollbarGrabber), Changed<Interaction>>,
    windows: Query<&Window>,
    mouse: Res<ButtonInput<MouseButton>>,
) {
    use crate::hud::ScrollList;
    let cursor_y = windows
        .iter()
        .next()
        .and_then(|window| window.cursor_position())
        .map(|position| position.y);
    // Захват: нажатие ровно по грабберу.
    for (interaction, grabber) in grabbers.iter() {
        if *interaction != Interaction::Pressed || drag.active.is_some() {
            continue;
        }
        let Some(y) = cursor_y else { continue };
        let scroll = match grabber.list {
            ScrollList::AppearanceHair => state.hair_scroll,
            ScrollList::AppearanceBeard => state.beard_scroll,
            ScrollList::SpawnMenu => hud.spawn_scroll,
            ScrollList::CraftMenu => craft.scroll,
        };
        drag.active = Some((grabber.list, y, scroll));
    }
    let Some((list, start_y, start_scroll)) = drag.active else {
        return;
    };
    if !mouse.pressed(MouseButton::Left) {
        drag.active = None;
        return;
    }
    let Some(y) = cursor_y else { return };
    let (content_h, view_h) = match list {
        ScrollList::AppearanceHair => (
            list_content_h(filtered_hair(&state.search).len()),
            list_view_h(HAIR_ROWS),
        ),
        ScrollList::AppearanceBeard => (
            list_content_h(filtered_beard(&state.search).len()),
            list_view_h(BEARD_ROWS),
        ),
        ScrollList::SpawnMenu => (
            crate::hud::spawn_content_h(cache.items.len()),
            crate::hud::spawn_view_h(),
        ),
        ScrollList::CraftMenu => (
            crate::crafting::matched_count(&craft, &content) as f32
                * crate::crafting::RECIPE_ROW_STEP,
            crate::crafting::list_view_h(),
        ),
    };
    let track = (view_h - ui::SCROLLBAR_MIN_GRABBER).max(0.0);
    if track <= 0.0 {
        return;
    }
    let max = (content_h - view_h).max(0.0);
    let value = (start_scroll + (y - start_y) / track * max).clamp(0.0, max);
    match list {
        ScrollList::AppearanceHair => {
            state.hair_scroll = value;
            state.hair_scroll_target = value;
        }
        ScrollList::AppearanceBeard => {
            state.beard_scroll = value;
            state.beard_scroll_target = value;
        }
        ScrollList::SpawnMenu => {
            hud.spawn_scroll = value;
            hud.spawn_scroll_target = value;
        }
        ScrollList::CraftMenu => {
            craft.scroll = value;
            craft.scroll_target = value;
        }
    }
}

/// Список стилей: строки «иконка состояния RSI + имя», выбранная подсвечена.
/// Прокрутка непрерывная (`ScrollContainer` в SS14): вьюпорт фиксированной
/// высоты с обрезкой, строки сдвинуты на дробную часть прокрутки, справа —
/// полоса прокрутки (`spawn_scrollbar`).
#[allow(clippy::too_many_arguments)]
fn style_list(
    panel: &mut ChildSpawnerCommands,
    registry: &crate::rsi::RsiRegistry,
    rsi: &str,
    beard: bool,
    rows: &[(usize, &str)],
    selected: usize,
    visible_rows: usize,
    scroll: f32,
) {
    let view_h = list_view_h(visible_rows);
    let content_h = list_content_h(rows.len());
    let scroll = scroll.clamp(0.0, (content_h - view_h).max(0.0));
    // Рисуем ВСЕ строки списка, а прокрутку применяет сдвиг контейнера
    // (`appearance_scroll_apply`): при прокрутке окно не пересобирается.
    let visible = rows.iter();
    // Как `ScrollContainer` в SS14: при видимой полосе контент ужимается на её
    // ширину, чтобы полоса не закрывала текст.
    let bar_width = if content_h > view_h + 1e-3 {
        ui::SCROLLBAR_WIDTH
    } else {
        0.0
    };
    panel
        .spawn((
            AppearanceList { beard },
            Interaction::default(),
            Node {
                width: Val::Percent(100.0),
                height: px(view_h),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .with_children(|list| {
            list.spawn((
                ScrollInner { beard },
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(0),
                    width: px(LIST_CONTENT_W - bar_width),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    ..default()
                },
                UiTransform::from_translation(Val2::new(Val::Px(0.0), Val::Px(-scroll))),
            ))
            .with_children(|inner| {
                for (index, name) in visible {
                    let is_selected = *index == selected;
                    let tint = if is_selected {
                        crate::hud::HudTint {
                            normal: ui::GLASS_BUTTON_PRESSED,
                            hovered: ui::GLASS_BUTTON_PRESSED,
                            pressed: ui::GLASS_BUTTON_PRESSED,
                        }
                    } else {
                        crate::hud::HudTint::button()
                    };
                    let icon = if *index == 0 && beard {
                        None // «нет бороды» — строка без иконки
                    } else {
                        registry
                            .get(&format!("{rsi}#{name}"))
                            .map(crate::inventory_ui::icon_node)
                    };
                    inner
                        .spawn((
                            AppearancePick {
                                beard,
                                index: *index,
                            },
                            Button,
                            tint,
                            BackgroundColor(if is_selected {
                                ui::GLASS_BUTTON_PRESSED
                            } else {
                                ui::GLASS_BUTTON
                            }),
                            Node {
                                width: Val::Percent(100.0),
                                height: px(STYLE_ROW_H),
                                align_items: AlignItems::Center,
                                column_gap: px(6),
                                padding: UiRect::horizontal(px(6)),
                                ..default()
                            },
                        ))
                        .with_children(|row| {
                            row.spawn(Node {
                                width: px(STYLE_ROW_H),
                                height: px(STYLE_ROW_H),
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
                                Text::new((*name).to_string()),
                                TextFont::from_font_size(ui::FONT_BASE),
                                TextColor(if is_selected { ui::ACCENT } else { ui::TEXT }),
                            ));
                        });
                }
            });
            // Полоса прокрутки — как `ScrollContainer`: только при контенте
            // выше вьюпорта, прижата вправо.
            let kind = if beard {
                crate::hud::ScrollList::AppearanceBeard
            } else {
                crate::hud::ScrollList::AppearanceHair
            };
            crate::hud::spawn_scrollbar(list, content_h, view_h, scroll, kind);
        });
}

/// Применяет прокрутку сдвигом готовых строк и двигает грабберы — без
/// пересборки окна, поэтому при прокрутке ничего не мигает.
#[allow(clippy::too_many_arguments)]
pub fn appearance_scroll_apply(
    state: Res<AppearanceUi>,
    hud: Res<crate::hud::HudState>,
    craft: Res<crate::crafting::CraftingState>,
    cache: Res<crate::hud::SpawnListCache>,
    content: Res<crate::content::ClientContent>,
    mut inners: Query<(&ScrollInner, &mut UiTransform)>,
    mut grabbers: Query<(&crate::hud::ScrollbarGrabber, &mut Node)>,
) {
    use crate::hud::ScrollList;
    for (inner, mut transform) in inners.iter_mut() {
        let scroll = if inner.beard {
            clamp_scroll(&state.beard_scroll, beard_count(&state.search), BEARD_ROWS)
        } else {
            clamp_scroll(&state.hair_scroll, hair_count(&state.search), HAIR_ROWS)
        };
        transform.translation = Val2::new(Val::Px(0.0), Val::Px(-scroll));
    }
    for (grabber, mut node) in grabbers.iter_mut() {
        // (высота контента, высота вьюпорта, прокрутка)
        let (content_h, view_h, scroll) = match grabber.list {
            ScrollList::AppearanceHair => (
                list_content_h(hair_count(&state.search)),
                list_view_h(HAIR_ROWS),
                state.hair_scroll,
            ),
            ScrollList::AppearanceBeard => (
                list_content_h(beard_count(&state.search)),
                list_view_h(BEARD_ROWS),
                state.beard_scroll,
            ),
            // Спавн-меню и крафт: граббер тоже двигается ПОКАДРОВО — раньше
            // он прыгал раз в строку (окна с ним не пересобираются).
            ScrollList::SpawnMenu => (
                crate::hud::spawn_content_h(cache.items.len()),
                crate::hud::spawn_view_h(),
                hud.spawn_scroll,
            ),
            ScrollList::CraftMenu => (
                crate::crafting::matched_count(&craft, &content) as f32
                    * crate::crafting::RECIPE_ROW_STEP,
                crate::crafting::list_view_h(),
                craft.scroll,
            ),
        };
        // Геометрия граббера — как в `ScrollBar.cs`.
        let track = (view_h - ui::SCROLLBAR_MIN_GRABBER).max(0.0);
        let scroll = scroll.clamp(0.0, (content_h - view_h).max(0.0));
        let ratio = (scroll / content_h).clamp(0.0, 1.0);
        node.top = px((ratio * track).round());
        node.height = px(((view_h / content_h) * track).round() + ui::SCROLLBAR_MIN_GRABBER);
    }
}

/// Отправляет текущую внешность на сервер (кукла на экране — превью).
fn send_appearance(
    state: &AppearanceUi,
    senders: &mut Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    let color = HAIR_COLORS[state.color.min(HAIR_COLORS.len() - 1)];
    let hair = hair_style_names()
        .get(state.hair)
        .cloned()
        .unwrap_or_default();
    let beard = if state.beard == 0 {
        String::new()
    } else {
        facial_hair_style_names()
            .get(state.beard - 1)
            .cloned()
            .unwrap_or_default()
    };
    let message = ClientMessage::SetAppearance {
        sex: Some(if state.female { "female" } else { "male" }.to_string()),
        hair: Some(hair),
        beard: Some(beard),
        hair_color: Some(color),
    };
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(message.clone());
    }
}

/// Клики в окне: строки списков, квадратики цвета и стрелка пола.
pub fn appearance_click(
    mut state: ResMut<AppearanceUi>,
    clicks: AppearanceClicks,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    for (interaction, pick, swatch, step) in clicks.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Some(pick) = pick {
            if pick.beard {
                state.beard = pick.index;
            } else {
                state.hair = pick.index;
            }
            tracing::info!(beard = pick.beard, index = pick.index, "appearance: pick");
            send_appearance(&state, &mut senders);
        } else if let Some(swatch) = swatch {
            state.color = swatch.0.min(HAIR_COLORS.len() - 1);
            send_appearance(&state, &mut senders);
        } else if let Some(step) = step {
            state.female = step.forward;
            send_appearance(&state, &mut senders);
        }
    }
}

/// Клик по строке поиска включает ввод; «Очистить» сбрасывает фильтр.
pub fn appearance_search_click(
    field: Query<&Interaction, (Changed<Interaction>, With<AppearanceSearchField>)>,
    clear: Query<&Interaction, (Changed<Interaction>, With<AppearanceClearButton>)>,
    mut state: ResMut<AppearanceUi>,
) {
    for interaction in field.iter() {
        if *interaction == Interaction::Pressed {
            state.search_focused = true;
        }
    }
    for interaction in clear.iter() {
        if *interaction == Interaction::Pressed {
            state.search.clear();
            state.hair_scroll = 0.0;
            state.hair_scroll_target = 0.0;
            state.beard_scroll = 0.0;
            state.beard_scroll_target = 0.0;
        }
    }
}

/// Ввод в строку поиска (паттерн спавн-меню: символы, Backspace, пробел).
pub fn appearance_search_input(
    mut events: MessageReader<KeyboardInput>,
    keys: Res<ButtonInput<KeyCode>>,
    mut state: ResMut<AppearanceUi>,
    console: Res<crate::console::Console>,
    chat: Res<crate::chat::ChatState>,
) {
    if !state.search_focused || !state.open {
        return;
    }
    if console.open || chat.focused {
        state.search_focused = false;
        events.clear();
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
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
            _ => continue,
        }
        state.hair_scroll = 0.0;
        state.hair_scroll_target = 0.0;
        state.beard_scroll = 0.0;
        state.beard_scroll_target = 0.0;
    }
}

/// Колесо мыши прокручивает список, над которым курсор (причёски или бороды).
/// Шаг — 50 px за щелчок, как `ScrollContainer.ScrollSpeedY` в движке;
/// значение догоняет цель в [`appearance_scroll_anim`].
pub fn appearance_scroll(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    mut state: ResMut<AppearanceUi>,
    chat: Res<crate::chat::ChatState>,
    console: Res<crate::console::Console>,
    lists: Query<(&AppearanceList, &Interaction)>,
) {
    if !state.open || chat.focused || console.open {
        wheel.clear();
        return;
    }
    let mut delta = 0.0;
    for event in wheel.read() {
        delta += event.y;
    }
    if delta == 0.0 {
        return;
    }
    // Колесо вниз (delta < 0) — вниз по списку.
    let shift = -delta * ui::SCROLLBAR_WHEEL_STEP;
    let beard = lists
        .iter()
        .find(|(_, interaction)| **interaction == Interaction::Hovered)
        .map(|(list, _)| list.beard)
        .unwrap_or(false);
    if beard {
        let max = (list_content_h(filtered_beard(&state.search).len()) - list_view_h(BEARD_ROWS))
            .max(0.0);
        state.beard_scroll_target = (state.beard_scroll_target + shift).clamp(0.0, max);
    } else {
        let max =
            (list_content_h(filtered_hair(&state.search).len()) - list_view_h(HAIR_ROWS)).max(0.0);
        state.hair_scroll_target = (state.hair_scroll_target + shift).clamp(0.0, max);
    }
}
