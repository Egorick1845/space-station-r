# -*- coding: utf-8 -*-
"""Общие шапка/тело окна SS14 (стекло) вместо текстур; чат на плоском поле."""
import io
import re

p = "crates/client/src/hud.rs"
s = io.open(p, encoding="utf-8").read()

# --- добавляем общие билдеры окон перед menu_panel
s = s.replace("""/// Каркас окна меню — как `DefaultWindow` в SS14""",
"""/// Шапка окна по SS14: плоский фон `Accent("#2A2A38D9", 0.26)`, акцентная
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

/// Каркас окна меню — как `DefaultWindow` в SS14""", 1)

# --- menu_panel: стеклянные шапка и тело
start = s.index("fn menu_panel(\n    commands: &mut Commands,")
end = s.index("/// Сколько строк помещается в спавн-меню")
new = '''fn menu_panel(
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
                left: px(left),
                top: px(top),
                width: px(width),
                max_height: px(520),
                flex_direction: FlexDirection::Column,
                ..default()
            },
        ))
        .with_children(|window| {
            window_header(window, theme, title);
            let (node, background) = window_body();
            window.spawn((node, background)).with_children(build);
        });
}

'''
s = s[:start] + new + s[end:]

# --- вызов menu_panel в админ-меню (добавился параметр top)
s = s.replace("""    menu_panel(
        &mut commands,
        &theme,
        "Админ-меню",
        950.0,
        320.0,
        |panel| {""",
"""    menu_panel(
        &mut commands,
        &theme,
        "Админ-меню",
        520.0,
        260.0,
        320.0,
        |panel| {""", 1)

# --- спавн-меню: шапка/тело из общих билдеров, позиция CenterLeft
old = s[s.index("            // Шапка окна: заголовок золотом + крестик"):s.index("                    // Строка поиска: поле ввода + «Очистить» (как в SS14).")]
new = '''            window_header(window, &theme, "Панель спавна сущностей");
            let (node, background) = window_body();
            window
                .spawn((node, background))
                .with_children(|body| {
'''
s = s.replace(old, new, 1)
# закрыть лишнюю скобку от старой структуры (тело было вложено глубже)
s = s.replace("""                    // Подсказка снизу: счётчик и режим размещения.
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
}""",
"""                    // Подсказка снизу: счётчик и режим размещения.
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
                        TextColor(ui::TEXT_MUTED),
                    ));
                });
        });
}""", 1)

# --- окно телепорта: общие шапка/тело
old = s[s.index("            window\n                .spawn((\n                    Node {\n                        width: Val::Percent(100.0),\n                        height: px(25.0),\n                        align_items: AlignItems::Center,\n                        padding: UiRect::horizontal(px(5)),\n                        column_gap: px(6),\n                        ..default()\n                    },\n                    ui::nine_slice_rect(&theme.window_header, 0.0, 0.0, 0.0, 3.0),"):s.index("                    for (name, position) in rows {")]
new = '''            window_header(window, &theme, "Телепорт призрака");
            let (node, background) = window_body();
            window
                .spawn((node, background))
                .with_children(|body| {
                    body.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            height: px(22),
                            align_items: AlignItems::Center,
                            padding: UiRect::horizontal(px(8)),
                            margin: UiRect::vertical(px(4)),
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
'''
s = s.replace(old, new, 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
print("окна на стекле ok")

# ---------------- чат: плоское поле ввода и точные размеры
p = "crates/client/src/chat.rs"
s = io.open(p, encoding="utf-8").read()
s = s.replace("""/// Высота области сообщений.
const CHAT_OUTPUT_HEIGHT: f32 = 150.0;""",
"""/// Высота всего блока чата (`ChatBox.xaml`: `MinSize 465×225`).
const CHAT_HEIGHT: f32 = 225.0;
/// Отступ области сообщений (`Margin="8 8 8 4"`).
const CHAT_OUTPUT_MARGIN: f32 = 8.0;""", 1)
s = s.replace("""            Node {
                position_type: PositionType::Absolute,
                right: px(10),
                top: px(10),
                width: px(CHAT_WIDTH),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(4)),
                row_gap: px(4),
                ..default()
            },
            BackgroundColor(ui::CHAT_BACKGROUND),
        ))""",
"""            Node {
                position_type: PositionType::Absolute,
                right: px(10),
                top: px(10),
                width: px(CHAT_WIDTH),
                height: px(CHAT_HEIGHT),
                flex_direction: FlexDirection::Column,
                row_gap: px(4),
                ..default()
            },
            BackgroundColor(ui::CHAT_BACKGROUND),
        ))""", 1)
s = s.replace("""                Node {
                    width: Val::Percent(100.0),
                    height: px(CHAT_OUTPUT_HEIGHT),
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::FlexEnd,
                    overflow: Overflow::clip(),
                    ..default()
                },""",
"""                Node {
                    width: Val::Percent(100.0),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    justify_content: JustifyContent::FlexEnd,
                    padding: UiRect::new(
                        px(CHAT_OUTPUT_MARGIN),
                        px(CHAT_OUTPUT_MARGIN),
                        px(CHAT_OUTPUT_MARGIN),
                        px(4.0),
                    ),
                    overflow: Overflow::clip(),
                    ..default()
                },""", 1)
s = s.replace("""            // Поле ввода: подпись канала + рамка LineEdit (Nano/lineedit.png).
            panel
                .spawn((
                    ui::nine_slice(&theme.lineedit, 3.0),
                    Node {
                        width: Val::Percent(100.0),
                        height: px(22),
                        align_items: AlignItems::Center,
                        padding: UiRect::horizontal(px(5)),
                        column_gap: px(5),
                        ..default()
                    },
                ))
                .with_child((
                    ChatInputText,
                    Text::new("Нажмите T, чтобы писать"),
                    TextFont::from_font_size(12.0),
                    TextColor(ui::TEXT_MUTED),
                ));""",
"""            // Поле ввода (`ChatInputBox`): кнопка канала 75 px + строка ввода
            // на плоском фоне панели чата (в SS14 рамки у LineEdit нет).
            panel
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: px(28),
                        align_items: AlignItems::Center,
                        padding: UiRect::axes(px(2), px(2)),
                        column_gap: px(4),
                        ..default()
                    },
                    BackgroundColor(ui::GLASS_LINEEDIT),
                ))
                .with_children(|row| {
                    row.spawn((
                        Node {
                            width: px(75),
                            height: px(24),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(ui::GLASS_BUTTON),
                    ))
                    .with_child((
                        Text::new("OOC"),
                        TextFont::from_font_size(12.0),
                        TextColor(channel_color(ChatChannel::Ooc)),
                    ));
                    row.spawn((
                        ChatInputText,
                        Text::new("Нажмите T, чтобы писать"),
                        TextFont::from_font_size(12.0),
                        TextColor(ui::TEXT_MUTED),
                    ));
                });""", 1)
# строки чата: без скобочных тегов канала (в SS14 канал виден по цвету)
s = s.replace("""                let text = if line.from.is_empty() {
                    format!("{} {}", channel_tag(line.channel), line.text)
                } else {
                    format!("{} {}: {}", channel_tag(line.channel), line.from, line.text)
                };""",
"""                let text = if line.from.is_empty() {
                    line.text.clone()
                } else {
                    format!("{}: {}", line.from, line.text)
                };""", 1)
s = s.replace("""/// Подпись канала в строке сообщения.
fn channel_tag(channel: ChatChannel) -> &'static str {
    match channel {
        ChatChannel::Ooc => "[OOC]",
        ChatChannel::Looc => "[LOOC]",
        ChatChannel::System => "",
    }
}

""", "", 1)
s = s.replace("""pub fn spawn_chat(
    mut commands: Commands,
    theme: Res<crate::ui_theme::UiTheme>,
    own: Res<OwnPlayerEntity>,""",
"""pub fn spawn_chat(
    mut commands: Commands,
    own: Res<OwnPlayerEntity>,""", 1)
io.open(p, "w", encoding="utf-8", newline="\n").write(s)
print("чат на стекле ok")
