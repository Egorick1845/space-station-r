//! Окно выбора внешности (по мотивам лобби SS14, где это отдельная вкладка):
//! пол, причёска, борода и цвет волос переключаются стрелками, изменения сразу
//! уходят на сервер (`SetAppearance`) и видны на своей кукле — она и служит
//! превью. Открывается клавишей P (в SS14 редактор внешности живёт в лобби).

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::mechanics::{FACIAL_HAIR_STYLES, HAIR_STYLES};
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

/// Отпечаток окна внешности для сравнения при перерисовке.
type AppearanceSignature = (bool, usize, usize, u8, usize);
/// Клики по стрелкам окна внешности.
type AppearanceClicks<'w, 's> = Query<
    'w,
    's,
    (&'static Interaction, &'static AppearanceStep),
    (Changed<Interaction>, With<Button>),
>;

/// Состояние окна внешности.
#[derive(Resource, Default)]
pub struct AppearanceUi {
    pub open: bool,
    pub hair: usize,
    pub beard: usize,
    pub color: usize,
    pub female: bool,
}

/// Что именно переключила стрелка.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub enum AppearanceField {
    Sex,
    Hair,
    Beard,
    Color,
}

/// Кнопка-стрелка в окне внешности: поле и направление.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub struct AppearanceStep {
    pub field: AppearanceField,
    pub forward: bool,
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
    if keys.just_pressed(KeyCode::KeyP) {
        ui_state.open = !ui_state.open;
        tracing::info!(open = ui_state.open, "appearance window toggled");
    }
}

/// Рисует окно: четыре строки со стрелками и образцом цвета.
pub fn render_appearance_menu(
    mut commands: Commands,
    state: Res<AppearanceUi>,
    theme: Res<crate::ui_theme::UiTheme>,
    root: Query<Entity, With<crate::hud::HudMenuRoot>>,
    mut last: Local<Option<AppearanceSignature>>,
) {
    let signature = (
        state.open,
        state.hair,
        state.beard,
        state.female as u8,
        state.color,
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
    let hair = HAIR_STYLES[state.hair.min(HAIR_STYLES.len() - 1)];
    let beard = if state.beard == 0 {
        "нет".to_string()
    } else {
        FACIAL_HAIR_STYLES[(state.beard - 1).min(FACIAL_HAIR_STYLES.len() - 1)].to_string()
    };
    let sex = if state.female { "female" } else { "male" };

    crate::hud::menu_panel(
        &mut commands,
        &theme,
        "Внешность",
        420.0,
        40.0,
        360.0,
        |panel| {
            for (field, label, value) in [
                (AppearanceField::Sex, "Пол: ", sex.to_string()),
                (AppearanceField::Hair, "Причёска: ", hair.to_string()),
                (AppearanceField::Beard, "Борода: ", beard),
                (
                    AppearanceField::Color,
                    "Цвет: ",
                    format!("#{:02X}{:02X}{:02X}", color[0], color[1], color[2]),
                ),
            ] {
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
                            Text::new(format!("{label}{value}")),
                            TextFont::from_font_size(ui::FONT_BASE),
                            TextColor(ui::TEXT),
                            Node {
                                flex_grow: 1.0,
                                ..default()
                            },
                        ));
                        // Образец цвета — квадратик рядом со строкой цвета.
                        if field == AppearanceField::Color {
                            row.spawn((
                                Node {
                                    width: px(18),
                                    height: px(18),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb_u8(color[0], color[1], color[2])),
                            ));
                        }
                        for forward in [false, true] {
                            row.spawn((
                                AppearanceStep { field, forward },
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
            }
            panel.spawn((
                Text::new("P — закрыть, Esc — закрыть все окна"),
                TextFont::from_font_size(ui::FONT_SMALL),
                TextColor(ui::TEXT_MUTED),
            ));
        },
    );
}

/// Нажатие стрелки: меняем выбор и отправляем внешность на сервер.
pub fn appearance_click(
    mut state: ResMut<AppearanceUi>,
    clicks: AppearanceClicks,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    for (interaction, step) in clicks.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let delta = if step.forward { 1 } else { -1 };
        match step.field {
            // Пол — просто переключатель.
            AppearanceField::Sex => state.female = !state.female,
            AppearanceField::Hair => {
                let len = HAIR_STYLES.len() as i32;
                state.hair = (state.hair as i32 + delta).rem_euclid(len) as usize;
            }
            AppearanceField::Beard => {
                // 0 — «нет бороды», дальше стили из сборки.
                let len = FACIAL_HAIR_STYLES.len() as i32 + 1;
                state.beard = (state.beard as i32 + delta).rem_euclid(len) as usize;
            }
            AppearanceField::Color => {
                let len = HAIR_COLORS.len() as i32;
                state.color = (state.color as i32 + delta).rem_euclid(len) as usize;
            }
        }
        // Сразу отправляем новую внешность: кукла сама служит превью.
        let color = HAIR_COLORS[state.color.min(HAIR_COLORS.len() - 1)];
        let beard = if state.beard == 0 {
            String::new()
        } else {
            FACIAL_HAIR_STYLES[(state.beard - 1).min(FACIAL_HAIR_STYLES.len() - 1)].to_string()
        };
        let message = ClientMessage::SetAppearance {
            sex: Some(if state.female { "female" } else { "male" }.to_string()),
            hair: Some(HAIR_STYLES[state.hair.min(HAIR_STYLES.len() - 1)].to_string()),
            beard: Some(beard),
            hair_color: Some(color),
        };
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(message.clone());
        }
        tracing::info!(field = ?step.field, forward = step.forward, "appearance: sent");
    }
}
