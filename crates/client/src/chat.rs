//! Панель чата по образцу SS14 (`ChatBox.xaml` + `ChatUIController`):
//! окно справа вверху с отступом 10, фон `#25252ADD`, список сообщений
//! (шрифт 12) и поле ввода снизу.
//!
//! Каналы и цвета как в движке: OOC — `LightSkyBlue`, LOOC — `MediumTurquoise`,
//! системные — серые. Ввод открывается по T, отправка — Enter, выход — Esc;
//! пока чат в фокусе, игровой ввод (ходьба, действия) не работает.

use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::inventory::Health;
use ssr_protocol::net::GameChannel;
use ssr_protocol::{ChatChannel, ClientMessage};

use crate::console::Console;
use crate::inventory_ui::OwnPlayerEntity;
use crate::ui_theme as ui;

/// Сколько последних строк показываем (остальное — колесом мыши).
const CHAT_VISIBLE: usize = 9;
/// Сколько строк храним в истории.
const CHAT_HISTORY: usize = 200;
/// Ширина панели (`ChatBox.xaml`: `MinSize 465×225`).
pub const CHAT_WIDTH: f32 = 465.0;
/// Высота всего блока чата (`ChatBox.xaml`: `MinSize 465×225`).
pub const CHAT_HEIGHT: f32 = 225.0;
/// Отступ области сообщений (`Margin="8 8 8 4"`).
const CHAT_OUTPUT_MARGIN: f32 = 8.0;

/// Строка чата.
#[derive(Clone, PartialEq)]
pub struct ChatLine {
    pub channel: ChatChannel,
    pub from: String,
    pub text: String,
}

/// Состояние чата: история, черновик ввода, фокус и прокрутка.
#[derive(Resource, Default)]
pub struct ChatState {
    pub lines: Vec<ChatLine>,
    pub input: String,
    /// Поле ввода в фокусе (набор текста).
    pub focused: bool,
    /// На сколько строк прокручена история вверх от конца.
    pub scroll: usize,
}

impl ChatState {
    /// Добавляет строку (история ограничена).
    pub fn push(&mut self, channel: ChatChannel, from: impl Into<String>, text: impl Into<String>) {
        self.lines.push(ChatLine {
            channel,
            from: from.into(),
            text: text.into(),
        });
        if self.lines.len() > CHAT_HISTORY {
            self.lines.remove(0);
        }
        self.scroll = 0;
    }

    /// Системное сообщение.
    pub fn system(&mut self, text: impl Into<String>) {
        self.push(ChatChannel::System, "", text);
    }

    /// Видимые строки с учётом прокрутки.
    fn visible(&self) -> impl Iterator<Item = &ChatLine> {
        let end = self.lines.len().saturating_sub(self.scroll);
        let start = end.saturating_sub(CHAT_VISIBLE);
        self.lines[start..end].iter()
    }
}

/// Корень панели чата.
#[derive(Component)]
pub struct ChatRoot;

/// Текст готовых строк (пересобирается при изменениях).
#[derive(Component)]
pub struct ChatOutput;

/// Строка ввода (показывает черновик и курсор).
#[derive(Component)]
pub struct ChatInputText;

/// Цвет канала (как `ChatUIController.ChannelSelectorButton.cs`).
fn channel_color(channel: ChatChannel) -> Color {
    match channel {
        ChatChannel::Ooc => Color::srgb(0.53, 0.81, 0.98), // LightSkyBlue
        ChatChannel::Looc => Color::srgb(0.28, 0.82, 0.80), // MediumTurquoise
        ChatChannel::System => Color::srgb(0.78, 0.78, 0.80),
    }
}

/// Создаёт панель чата при входе в игру.
pub fn spawn_chat(
    mut commands: Commands,
    own: Res<OwnPlayerEntity>,
    roots: Query<(), With<ChatRoot>>,
) {
    if own.0.is_none() || !roots.is_empty() {
        return;
    }
    commands
        .spawn((
            ChatRoot,
            Node {
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
        ))
        .with_children(|panel| {
            // Область сообщений: строки пересобираются в render_chat.
            panel.spawn((
                ChatOutput,
                Node {
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
                },
            ));
            // Поле ввода (`ChatInputBox`): кнопка канала 75 px и строка ввода
            // на плоском фоне — в сборке у LineEdit чата рамки нет.
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
                });
        });
}

/// Пересобирает строки чата при изменениях (видны последние [`CHAT_VISIBLE`]).
pub fn render_chat(
    mut commands: Commands,
    chat: Res<ChatState>,
    output: Query<(Entity, Option<&Children>), With<ChatOutput>>,
    mut last: Local<Option<(usize, usize, bool, String)>>,
) {
    let last_line = chat
        .lines
        .last()
        .map(|line| line.text.clone())
        .unwrap_or_default();
    let signature = (chat.lines.len(), chat.scroll, chat.focused, last_line);
    if last.as_ref() == Some(&signature) {
        return;
    }
    *last = Some(signature);
    tracing::info!(
        lines = chat.lines.len(),
        visible = chat.visible().count(),
        nodes = output.iter().count(),
        "chat render"
    );
    for (entity, children) in output.iter() {
        // Children появляется только после первого добавления детей.
        if let Some(children) = children {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        commands.entity(entity).with_children(|column| {
            for line in chat.visible() {
                // В SS14 канал виден по цвету строки, без скобочных тегов.
                let text = if line.from.is_empty() {
                    line.text.clone()
                } else {
                    format!("{}: {}", line.from, line.text)
                };
                column.spawn((
                    Text::new(text),
                    TextFont::from_font_size(12.0),
                    TextColor(channel_color(line.channel)),
                ));
            }
        });
    }
}

/// Набор текста: T — открыть, Enter — отправить, Esc — закрыть, колесо — прокрутка.
pub fn chat_input(
    mut events: MessageReader<KeyboardInput>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<Console>,
    mut chat: ResMut<ChatState>,
    mut text: Query<(&mut Text, &mut TextColor), With<ChatInputText>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    // Прокрутка истории колесом (когда не набираешь текст).
    if !chat.focused {
        for event in wheel.read() {
            let up = event.y > 0.0;
            let limit = chat.lines.len().saturating_sub(CHAT_VISIBLE);
            if up {
                chat.scroll = (chat.scroll + 1).min(limit);
            } else {
                chat.scroll = chat.scroll.saturating_sub(1);
            }
        }
    } else {
        wheel.clear();
    }
    // T открывает поле ввода (как в SS14), если не занята консоль.
    if !chat.focused
        && !console.open
        && keys.just_pressed(KeyCode::KeyT)
        && !keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])
        && !keys.any_pressed([KeyCode::ControlLeft, KeyCode::ControlRight])
    {
        chat.focused = true;
        chat.scroll = 0;
    }

    let mut submitted = false;
    if chat.focused {
        for event in events.read() {
            if event.state != ButtonState::Pressed {
                continue;
            }
            match &event.logical_key {
                Key::Enter => submitted = true,
                Key::Escape => {
                    chat.focused = false;
                    chat.input.clear();
                }
                Key::Backspace => {
                    chat.input.pop();
                }
                Key::Space => chat.input.push(' '),
                Key::Character(text) => chat.input.push_str(text),
                _ => {}
            }
        }
    } else {
        events.clear();
    }

    if submitted {
        let text = std::mem::take(&mut chat.input);
        let text = text.trim().to_string();
        if !text.is_empty() {
            // Канал: LOOC — если сообщение начинается с «.» (как в SS14),
            // иначе общий OOC.
            let (channel, message) = match text.strip_prefix('.') {
                Some(rest) => (ChatChannel::Looc, rest.trim().to_string()),
                None => (ChatChannel::Ooc, text.clone()),
            };
            if !message.is_empty() {
                for mut sender in senders.iter_mut() {
                    sender.send::<GameChannel>(ClientMessage::Chat {
                        channel,
                        text: message.clone(),
                    });
                }
                tracing::info!(?channel, %message, "chat sent");
            }
        }
    }

    // Поле ввода: черновик с курсором или подсказка.
    let (label, color) = if chat.focused {
        (format!("{}_", chat.input), ui::TEXT)
    } else {
        ("Нажмите T, чтобы писать".to_string(), ui::TEXT_MUTED)
    };
    for (mut node_text, mut node_color) in text.iter_mut() {
        if node_text.0 != label {
            node_text.0 = label.clone();
        }
        if node_color.0 != color {
            node_color.0 = color;
        }
    }
}

/// Системные строки о своём состоянии (потеря сознания и возвращение).
pub fn chat_health_notices(
    own: Res<OwnPlayerEntity>,
    healths: Query<&Health>,
    mut chat: ResMut<ChatState>,
    mut last: Local<bool>,
) {
    let down = own
        .0
        .and_then(|entity| healths.get(entity).ok())
        .is_some_and(|health| health.current == 0);
    if down != *last {
        *last = down;
        if down {
            chat.system("Вы потеряли сознание");
        } else {
            chat.system("Вы пришли в себя");
        }
    }
}

/// Тест-режим SSR_CHAT_TEST=1: с 4-й секунды шлёт OOC, с 6-й — LOOC,
/// с 8-й — системное сообщение себе (проверка панели и каналов).
#[allow(clippy::too_many_arguments)]
pub fn chat_test_mode(
    time: Res<Time>,
    mut state: Local<(f32, u8)>,
    mut chat: ResMut<ChatState>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if std::env::var_os("SSR_CHAT_TEST").is_none() {
        return;
    }
    state.0 += time.delta_secs();
    let elapsed = state.0;
    let send = |senders: &mut Query<&mut MessageSender<ClientMessage>, With<Connected>>,
                channel: ChatChannel,
                text: &str| {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Chat {
                channel,
                text: text.to_string(),
            });
        }
    };
    if state.1 == 0 && elapsed >= 4.0 {
        state.1 = 1;
        send(&mut senders, ChatChannel::Ooc, "Всем привет из теста чата");
        tracing::info!("chat-test: OOC sent");
    }
    if state.1 == 1 && elapsed >= 6.0 {
        state.1 = 2;
        send(&mut senders, ChatChannel::Looc, "Шёпотом: локальный чат");
        tracing::info!("chat-test: LOOC sent");
    }
    if state.1 == 2 && elapsed >= 8.0 {
        state.1 = 3;
        chat.system("Тестовое системное сообщение");
        tracing::info!("chat-test: system line added");
    }
}
