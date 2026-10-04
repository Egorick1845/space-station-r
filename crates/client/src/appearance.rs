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
    pub hair_scroll: usize,
    pub beard_scroll: usize,
    pub search: String,
    pub search_focused: bool,
    /// Текущая внешность подтянута из компонентов игрока (на первое открытие).
    pub synced: bool,
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
type AppearanceSignature = (bool, usize, usize, bool, usize, usize, usize, String, u32);

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

/// Открывает/закрывает окно внешности (клавиша P).
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
        state.hair_scroll,
        state.beard_scroll,
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

    // Списки с поиском: видимое окно строк — как список ItemList в SS14.
    let hair_all = filtered_hair(&state.search);
    let hair_scroll = state
        .hair_scroll
        .min(hair_all.len().saturating_sub(HAIR_ROWS));
    let hair_visible: Vec<(usize, &str)> = hair_all
        .iter()
        .skip(hair_scroll)
        .take(HAIR_ROWS)
        .cloned()
        .collect();
    let beard_all = filtered_beard(&state.search);
    let beard_scroll = state
        .beard_scroll
        .min(beard_all.len().saturating_sub(BEARD_ROWS));
    let beard_visible: Vec<(usize, &str)> = beard_all
        .iter()
        .skip(beard_scroll)
        .take(BEARD_ROWS)
        .cloned()
        .collect();

    let window = crate::hud::menu_panel(
        &mut commands,
        &theme,
        "Внешность",
        50.0,
        50.0,
        430.0,
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
                &hair_visible,
                state.hair,
                HAIR_ROWS,
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
                &beard_visible,
                state.beard,
                BEARD_ROWS,
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

/// Список стилей: строки «иконка состояния RSI + имя», выбранная подсвечена.
#[allow(clippy::too_many_arguments)]
fn style_list(
    panel: &mut ChildSpawnerCommands,
    registry: &crate::rsi::RsiRegistry,
    rsi: &str,
    beard: bool,
    visible: &[(usize, &str)],
    selected: usize,
    rows: usize,
) {
    // Свободные строки добиваем пустыми, чтобы окно не «прыгало» по высоте.
    let blank = rows.saturating_sub(visible.len());
    panel
        .spawn((
            AppearanceList { beard },
            Interaction::default(),
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                ..default()
            },
        ))
        .with_children(|list| {
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
                list.spawn((
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
            for _ in 0..blank {
                list.spawn(Node {
                    width: Val::Percent(100.0),
                    height: px(STYLE_ROW_H),
                    ..default()
                });
            }
        });
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
            state.hair_scroll = 0;
            state.beard_scroll = 0;
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
        state.hair_scroll = 0;
        state.beard_scroll = 0;
    }
}

/// Колесо мыши прокручивает список, над которым курсор (причёски или бороды).
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
    let step: isize = if delta < 0.0 { 1 } else { -1 };
    let beard = lists
        .iter()
        .find(|(_, interaction)| **interaction == Interaction::Hovered)
        .map(|(list, _)| list.beard)
        .unwrap_or(false);
    if beard {
        let max = filtered_beard(&state.search)
            .len()
            .saturating_sub(BEARD_ROWS);
        state.beard_scroll = (state.beard_scroll as isize + step).clamp(0, max as isize) as usize;
    } else {
        let max = filtered_hair(&state.search).len().saturating_sub(HAIR_ROWS);
        state.hair_scroll = (state.hair_scroll as isize + step).clamp(0, max as isize) as usize;
    }
}
