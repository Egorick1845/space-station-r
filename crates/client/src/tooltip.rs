//! Всплывающие подсказки — как `Control.ToolTip` / `TooltipSupplier` в SS14:
//! навёл курсор на кнопку и подержал (`ui.tooltip_delay`, у нас 0.4 с) —
//! у курсора появляется тёмная панель с заголовком и описанием
//! (`ActionAlertTooltip` у действий: название жирным, ниже — описание).

use bevy::prelude::*;

/// Текст подсказки: заголовок и необязательное описание.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct Tooltip {
    pub title: String,
    pub body: Option<String>,
}

impl Tooltip {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: None,
        }
    }

    pub fn with_body(title: impl Into<String>, body: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            body: Some(body.into()),
        }
    }
}

/// Корень показанной подсказки.
#[derive(Component)]
pub struct TooltipRoot;

const DELAY: f32 = 0.4;

/// Показ подсказки над наведённой кнопкой.
pub fn show_tooltips(
    mut commands: Commands,
    time: Res<Time>,
    windows: Query<&Window>,
    hovered: Query<(Entity, &Interaction, &Tooltip)>,
    roots: Query<Entity, With<TooltipRoot>>,
    mut state: Local<(Option<Entity>, f32, bool)>,
) {
    let current = hovered
        .iter()
        .find(|(_, interaction, _)| **interaction == Interaction::Hovered)
        .map(|(entity, _, tip)| (entity, tip.clone()));
    let Some((entity, tip)) = current else {
        if state.0.is_some() {
            for root in roots.iter() {
                commands.entity(root).despawn();
            }
        }
        *state = (None, 0.0, false);
        return;
    };
    if state.0 != Some(entity) {
        for root in roots.iter() {
            commands.entity(root).despawn();
        }
        *state = (Some(entity), 0.0, false);
    }
    state.1 += time.delta_secs();
    if state.2 || state.1 < DELAY {
        return;
    }
    state.2 = true;
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let left = (cursor.x + 14.0).min(window.width() - 260.0).max(0.0);
    let top = (cursor.y + 18.0).min(window.height() - 70.0).max(0.0);
    commands
        .spawn((
            TooltipRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(left),
                top: px(top),
                max_width: px(260),
                flex_direction: FlexDirection::Column,
                row_gap: px(2),
                padding: UiRect::axes(px(8), px(5)),
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.96)),
            BorderColor::from(Color::srgb(0.32, 0.32, 0.38)),
            GlobalZIndex(90),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new(tip.title.clone()),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(0.95, 0.95, 0.97)),
            ));
            if let Some(body) = &tip.body {
                panel.spawn((
                    Text::new(body.clone()),
                    TextFont::from_font_size(12.0),
                    TextColor(Color::srgb(0.68, 0.68, 0.72)),
                ));
            }
        });
}
