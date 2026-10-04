//! ÐŸÐ°Ð½ÐµÐ»ÑŒ Ñ‡Ð°Ñ‚Ð° Ð¿Ð¾ Ð¾Ð±Ñ€Ð°Ð·Ñ†Ñƒ SS14 (`ChatBox.xaml` + `ChatUIController`):
//! Ð¾ÐºÐ½Ð¾ ÑÐ¿Ñ€Ð°Ð²Ð° Ð²Ð²ÐµÑ€Ñ…Ñƒ Ñ Ð¾Ñ‚ÑÑ‚ÑƒÐ¿Ð¾Ð¼ 10, Ñ„Ð¾Ð½ `#25252ADD`, ÑÐ¿Ð¸ÑÐ¾Ðº ÑÐ¾Ð¾Ð±Ñ‰ÐµÐ½Ð¸Ð¹
//! (ÑˆÑ€Ð¸Ñ„Ñ‚ 12) Ð¸ Ð¿Ð¾Ð»Ðµ Ð²Ð²Ð¾Ð´Ð° ÑÐ½Ð¸Ð·Ñƒ.
//!
//! ÐšÐ°Ð½Ð°Ð»Ñ‹ Ð¸ Ñ†Ð²ÐµÑ‚Ð° ÐºÐ°Ðº Ð² Ð´Ð²Ð¸Ð¶ÐºÐµ: OOC â€” `LightSkyBlue`, LOOC â€” `MediumTurquoise`,
//! ÑÐ¸ÑÑ‚ÐµÐ¼Ð½Ñ‹Ðµ â€” ÑÐµÑ€Ñ‹Ðµ. Ð’Ð²Ð¾Ð´ Ð¾Ñ‚ÐºÑ€Ñ‹Ð²Ð°ÐµÑ‚ÑÑ Ð¿Ð¾ T, Ð¾Ñ‚Ð¿Ñ€Ð°Ð²ÐºÐ° â€” Enter, Ð²Ñ‹Ñ…Ð¾Ð´ â€” Esc;
//! Ð¿Ð¾ÐºÐ° Ñ‡Ð°Ñ‚ Ð² Ñ„Ð¾ÐºÑƒÑÐµ, Ð¸Ð³Ñ€Ð¾Ð²Ð¾Ð¹ Ð²Ð²Ð¾Ð´ (Ñ…Ð¾Ð´ÑŒÐ±Ð°, Ð´ÐµÐ¹ÑÑ‚Ð²Ð¸Ñ) Ð½Ðµ Ñ€Ð°Ð±Ð¾Ñ‚Ð°ÐµÑ‚.

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

/// Ð¡ÐºÐ¾Ð»ÑŒÐºÐ¾ Ð¿Ð¾ÑÐ»ÐµÐ´Ð½Ð¸Ñ… ÑÑ‚Ñ€Ð¾Ðº Ð¿Ð¾ÐºÐ°Ð·Ñ‹Ð²Ð°ÐµÐ¼ (Ð¾ÑÑ‚Ð°Ð»ÑŒÐ½Ð¾Ðµ â€” ÐºÐ¾Ð»ÐµÑÐ¾Ð¼ Ð¼Ñ‹ÑˆÐ¸).
const CHAT_VISIBLE: usize = 9;
/// Ð¡ÐºÐ¾Ð»ÑŒÐºÐ¾ ÑÑ‚Ñ€Ð¾Ðº Ñ…Ñ€Ð°Ð½Ð¸Ð¼ Ð² Ð¸ÑÑ‚Ð¾Ñ€Ð¸Ð¸.
const CHAT_HISTORY: usize = 200;
/// Ð¨Ð¸Ñ€Ð¸Ð½Ð° Ð¿Ð°Ð½ÐµÐ»Ð¸ (`ChatBox.xaml`: `MinSize 465Ã—225`).
pub const CHAT_WIDTH: f32 = 465.0;
/// Ð’Ñ‹ÑÐ¾Ñ‚Ð° Ð²ÑÐµÐ³Ð¾ Ð±Ð»Ð¾ÐºÐ° Ñ‡Ð°Ñ‚Ð° (`ChatBox.xaml`: `MinSize 465Ã—225`).
pub const CHAT_HEIGHT: f32 = 225.0;
/// ÐžÑ‚ÑÑ‚ÑƒÐ¿ Ð¾Ð±Ð»Ð°ÑÑ‚Ð¸ ÑÐ¾Ð¾Ð±Ñ‰ÐµÐ½Ð¸Ð¹ (`Margin="8 8 8 4"`).
const CHAT_OUTPUT_MARGIN: f32 = 8.0;

/// Ð¡Ñ‚Ñ€Ð¾ÐºÐ° Ñ‡Ð°Ñ‚Ð°.
#[derive(Clone, PartialEq)]
pub struct ChatLine {
    pub channel: ChatChannel,
    pub from: String,
    pub text: String,
}

/// Ð¡Ð¾ÑÑ‚Ð¾ÑÐ½Ð¸Ðµ Ñ‡Ð°Ñ‚Ð°: Ð¸ÑÑ‚Ð¾Ñ€Ð¸Ñ, Ñ‡ÐµÑ€Ð½Ð¾Ð²Ð¸Ðº Ð²Ð²Ð¾Ð´Ð°, Ñ„Ð¾ÐºÑƒÑ Ð¸ Ð¿Ñ€Ð¾ÐºÑ€ÑƒÑ‚ÐºÐ°.
#[derive(Resource, Default)]
pub struct ChatState {
    pub lines: Vec<ChatLine>,
    pub input: String,
    /// ÐŸÐ¾Ð»Ðµ Ð²Ð²Ð¾Ð´Ð° Ð² Ñ„Ð¾ÐºÑƒÑÐµ (Ð½Ð°Ð±Ð¾Ñ€ Ñ‚ÐµÐºÑÑ‚Ð°).
    pub focused: bool,
    /// ÐÐ° ÑÐºÐ¾Ð»ÑŒÐºÐ¾ ÑÑ‚Ñ€Ð¾Ðº Ð¿Ñ€Ð¾ÐºÑ€ÑƒÑ‡ÐµÐ½Ð° Ð¸ÑÑ‚Ð¾Ñ€Ð¸Ñ Ð²Ð²ÐµÑ€Ñ… Ð¾Ñ‚ ÐºÐ¾Ð½Ñ†Ð°.
    pub scroll: usize,
}

impl ChatState {
    /// Ð”Ð¾Ð±Ð°Ð²Ð»ÑÐµÑ‚ ÑÑ‚Ñ€Ð¾ÐºÑƒ (Ð¸ÑÑ‚Ð¾Ñ€Ð¸Ñ Ð¾Ð³Ñ€Ð°Ð½Ð¸Ñ‡ÐµÐ½Ð°).
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

    /// Ð¡Ð¸ÑÑ‚ÐµÐ¼Ð½Ð¾Ðµ ÑÐ¾Ð¾Ð±Ñ‰ÐµÐ½Ð¸Ðµ.
    pub fn system(&mut self, text: impl Into<String>) {
        self.push(ChatChannel::System, "", text);
    }

    /// Ð’Ð¸Ð´Ð¸Ð¼Ñ‹Ðµ ÑÑ‚Ñ€Ð¾ÐºÐ¸ Ñ ÑƒÑ‡Ñ‘Ñ‚Ð¾Ð¼ Ð¿Ñ€Ð¾ÐºÑ€ÑƒÑ‚ÐºÐ¸.
    fn visible(&self) -> impl Iterator<Item = &ChatLine> {
        let end = self.lines.len().saturating_sub(self.scroll);
        let start = end.saturating_sub(CHAT_VISIBLE);
        self.lines[start..end].iter()
    }
}

/// ÐšÐ¾Ñ€ÐµÐ½ÑŒ Ð¿Ð°Ð½ÐµÐ»Ð¸ Ñ‡Ð°Ñ‚Ð°.
#[derive(Component)]
pub struct ChatRoot;

/// Ð¢ÐµÐºÑÑ‚ Ð³Ð¾Ñ‚Ð¾Ð²Ñ‹Ñ… ÑÑ‚Ñ€Ð¾Ðº (Ð¿ÐµÑ€ÐµÑÐ¾Ð±Ð¸Ñ€Ð°ÐµÑ‚ÑÑ Ð¿Ñ€Ð¸ Ð¸Ð·Ð¼ÐµÐ½ÐµÐ½Ð¸ÑÑ…).
#[derive(Component)]
pub struct ChatOutput;

/// Ð¡Ñ‚Ñ€Ð¾ÐºÐ° Ð²Ð²Ð¾Ð´Ð° (Ð¿Ð¾ÐºÐ°Ð·Ñ‹Ð²Ð°ÐµÑ‚ Ñ‡ÐµÑ€Ð½Ð¾Ð²Ð¸Ðº Ð¸ ÐºÑƒÑ€ÑÐ¾Ñ€).
#[derive(Component)]
pub struct ChatInputText;

/// Ð¦Ð²ÐµÑ‚ ÐºÐ°Ð½Ð°Ð»Ð° (ÐºÐ°Ðº `ChatUIController.ChannelSelectorButton.cs`).
fn channel_color(channel: ChatChannel) -> Color {
    match channel {
        ChatChannel::Ooc => Color::srgb(0.53, 0.81, 0.98), // LightSkyBlue
        ChatChannel::Looc => Color::srgb(0.28, 0.82, 0.80), // MediumTurquoise
        ChatChannel::System => Color::srgb(0.78, 0.78, 0.80),
    }
}

/// Ð¡Ð¾Ð·Ð´Ð°Ñ‘Ñ‚ Ð¿Ð°Ð½ÐµÐ»ÑŒ Ñ‡Ð°Ñ‚Ð° Ð¿Ñ€Ð¸ Ð²Ñ…Ð¾Ð´Ðµ Ð² Ð¸Ð³Ñ€Ñƒ.
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
            // ÐžÐ±Ð»Ð°ÑÑ‚ÑŒ ÑÐ¾Ð¾Ð±Ñ‰ÐµÐ½Ð¸Ð¹: ÑÑ‚Ñ€Ð¾ÐºÐ¸ Ð¿ÐµÑ€ÐµÑÐ¾Ð±Ð¸Ñ€Ð°ÑŽÑ‚ÑÑ Ð² render_chat.
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
            // ÐŸÐ¾Ð»Ðµ Ð²Ð²Ð¾Ð´Ð° (`ChatInputBox`): ÐºÐ½Ð¾Ð¿ÐºÐ° ÐºÐ°Ð½Ð°Ð»Ð° 75 px Ð¸ ÑÑ‚Ñ€Ð¾ÐºÐ° Ð²Ð²Ð¾Ð´Ð°
            // Ð½Ð° Ð¿Ð»Ð¾ÑÐºÐ¾Ð¼ Ñ„Ð¾Ð½Ðµ â€” Ð² ÑÐ±Ð¾Ñ€ÐºÐµ Ñƒ LineEdit Ñ‡Ð°Ñ‚Ð° Ñ€Ð°Ð¼ÐºÐ¸ Ð½ÐµÑ‚.
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
                        super::chat_text_font(),
                        TextColor(channel_color(ChatChannel::Ooc)),
                    ));
                    row.spawn((
                        ChatInputText,
                        Text::new("ÐÐ°Ð¶Ð¼Ð¸Ñ‚Ðµ T, Ñ‡Ñ‚Ð¾Ð±Ñ‹ Ð¿Ð¸ÑÐ°Ñ‚ÑŒ"),
                        super::chat_text_font(),
                        TextColor(ui::TEXT_MUTED),
                    ));
                });
        });
}

/// ÐŸÐµÑ€ÐµÑÐ¾Ð±Ð¸Ñ€Ð°ÐµÑ‚ ÑÑ‚Ñ€Ð¾ÐºÐ¸ Ñ‡Ð°Ñ‚Ð° Ð¿Ñ€Ð¸ Ð¸Ð·Ð¼ÐµÐ½ÐµÐ½Ð¸ÑÑ… (Ð²Ð¸Ð´Ð½Ñ‹ Ð¿Ð¾ÑÐ»ÐµÐ´Ð½Ð¸Ðµ [`CHAT_VISIBLE`]).
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
        // Children Ð¿Ð¾ÑÐ²Ð»ÑÐµÑ‚ÑÑ Ñ‚Ð¾Ð»ÑŒÐºÐ¾ Ð¿Ð¾ÑÐ»Ðµ Ð¿ÐµÑ€Ð²Ð¾Ð³Ð¾ Ð´Ð¾Ð±Ð°Ð²Ð»ÐµÐ½Ð¸Ñ Ð´ÐµÑ‚ÐµÐ¹.
        if let Some(children) = children {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        commands.entity(entity).with_children(|column| {
            for line in chat.visible() {
                // Ð’ SS14 ÐºÐ°Ð½Ð°Ð» Ð²Ð¸Ð´ÐµÐ½ Ð¿Ð¾ Ñ†Ð²ÐµÑ‚Ñƒ ÑÑ‚Ñ€Ð¾ÐºÐ¸, Ð±ÐµÐ· ÑÐºÐ¾Ð±Ð¾Ñ‡Ð½Ñ‹Ñ… Ñ‚ÐµÐ³Ð¾Ð².
                let text = if line.from.is_empty() {
                    line.text.clone()
                } else {
                    format!("{}: {}", line.from, line.text)
                };
                column.spawn((
                    Text::new(text),
                    super::chat_text_font(),
                    TextColor(channel_color(line.channel)),
                ));
            }
        });
    }
}

/// ÐÐ°Ð±Ð¾Ñ€ Ñ‚ÐµÐºÑÑ‚Ð°: T â€” Ð¾Ñ‚ÐºÑ€Ñ‹Ñ‚ÑŒ, Enter â€” Ð¾Ñ‚Ð¿Ñ€Ð°Ð²Ð¸Ñ‚ÑŒ, Esc â€” Ð·Ð°ÐºÑ€Ñ‹Ñ‚ÑŒ, ÐºÐ¾Ð»ÐµÑÐ¾ â€” Ð¿Ñ€Ð¾ÐºÑ€ÑƒÑ‚ÐºÐ°.
pub fn chat_input(
    mut events: MessageReader<KeyboardInput>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<Console>,
    mut chat: ResMut<ChatState>,
    mut text: Query<(&mut Text, &mut TextColor), With<ChatInputText>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    // ÐŸÑ€Ð¾ÐºÑ€ÑƒÑ‚ÐºÐ° Ð¸ÑÑ‚Ð¾Ñ€Ð¸Ð¸ ÐºÐ¾Ð»ÐµÑÐ¾Ð¼ (ÐºÐ¾Ð³Ð´Ð° Ð½Ðµ Ð½Ð°Ð±Ð¸Ñ€Ð°ÐµÑˆÑŒ Ñ‚ÐµÐºÑÑ‚).
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
    // T Ð¾Ñ‚ÐºÑ€Ñ‹Ð²Ð°ÐµÑ‚ Ð¿Ð¾Ð»Ðµ Ð²Ð²Ð¾Ð´Ð° (ÐºÐ°Ðº Ð² SS14), ÐµÑÐ»Ð¸ Ð½Ðµ Ð·Ð°Ð½ÑÑ‚Ð° ÐºÐ¾Ð½ÑÐ¾Ð»ÑŒ.
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
            // ÐšÐ°Ð½Ð°Ð»: LOOC â€” ÐµÑÐ»Ð¸ ÑÐ¾Ð¾Ð±Ñ‰ÐµÐ½Ð¸Ðµ Ð½Ð°Ñ‡Ð¸Ð½Ð°ÐµÑ‚ÑÑ Ñ Â«.Â» (ÐºÐ°Ðº Ð² SS14),
            // Ð¸Ð½Ð°Ñ‡Ðµ Ð¾Ð±Ñ‰Ð¸Ð¹ OOC.
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

    // ÐŸÐ¾Ð»Ðµ Ð²Ð²Ð¾Ð´Ð°: Ñ‡ÐµÑ€Ð½Ð¾Ð²Ð¸Ðº Ñ ÐºÑƒÑ€ÑÐ¾Ñ€Ð¾Ð¼ Ð¸Ð»Ð¸ Ð¿Ð¾Ð´ÑÐºÐ°Ð·ÐºÐ°.
    let (label, color) = if chat.focused {
        (format!("{}_", chat.input), ui::TEXT)
    } else {
        (
            "ÐÐ°Ð¶Ð¼Ð¸Ñ‚Ðµ T, Ñ‡Ñ‚Ð¾Ð±Ñ‹ Ð¿Ð¸ÑÐ°Ñ‚ÑŒ".to_string(),
            ui::TEXT_MUTED,
        )
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

/// Ð¡Ð¸ÑÑ‚ÐµÐ¼Ð½Ñ‹Ðµ ÑÑ‚Ñ€Ð¾ÐºÐ¸ Ð¾ ÑÐ²Ð¾Ñ‘Ð¼ ÑÐ¾ÑÑ‚Ð¾ÑÐ½Ð¸Ð¸ (Ð¿Ð¾Ñ‚ÐµÑ€Ñ ÑÐ¾Ð·Ð½Ð°Ð½Ð¸Ñ Ð¸ Ð²Ð¾Ð·Ð²Ñ€Ð°Ñ‰ÐµÐ½Ð¸Ðµ).
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
            chat.system("Ð’Ñ‹ Ð¿Ð¾Ñ‚ÐµÑ€ÑÐ»Ð¸ ÑÐ¾Ð·Ð½Ð°Ð½Ð¸Ðµ");
        } else {
            chat.system("Ð’Ñ‹ Ð¿Ñ€Ð¸ÑˆÐ»Ð¸ Ð² ÑÐµÐ±Ñ");
        }
    }
}

/// Ð¢ÐµÑÑ‚-Ñ€ÐµÐ¶Ð¸Ð¼ SSR_CHAT_TEST=1: Ñ 4-Ð¹ ÑÐµÐºÑƒÐ½Ð´Ñ‹ ÑˆÐ»Ñ‘Ñ‚ OOC, Ñ 6-Ð¹ â€” LOOC,
/// Ñ 8-Ð¹ â€” ÑÐ¸ÑÑ‚ÐµÐ¼Ð½Ð¾Ðµ ÑÐ¾Ð¾Ð±Ñ‰ÐµÐ½Ð¸Ðµ ÑÐµÐ±Ðµ (Ð¿Ñ€Ð¾Ð²ÐµÑ€ÐºÐ° Ð¿Ð°Ð½ÐµÐ»Ð¸ Ð¸ ÐºÐ°Ð½Ð°Ð»Ð¾Ð²).
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
        send(
            &mut senders,
            ChatChannel::Ooc,
            "Ð’ÑÐµÐ¼ Ð¿Ñ€Ð¸Ð²ÐµÑ‚ Ð¸Ð· Ñ‚ÐµÑÑ‚Ð° Ñ‡Ð°Ñ‚Ð°",
        );
        tracing::info!("chat-test: OOC sent");
    }
    if state.1 == 1 && elapsed >= 6.0 {
        state.1 = 2;
        send(
            &mut senders,
            ChatChannel::Looc,
            "Ð¨Ñ‘Ð¿Ð¾Ñ‚Ð¾Ð¼: Ð»Ð¾ÐºÐ°Ð»ÑŒÐ½Ñ‹Ð¹ Ñ‡Ð°Ñ‚",
        );
        tracing::info!("chat-test: LOOC sent");
    }
    if state.1 == 2 && elapsed >= 8.0 {
        state.1 = 3;
        chat.system("Ð¢ÐµÑÑ‚Ð¾Ð²Ð¾Ðµ ÑÐ¸ÑÑ‚ÐµÐ¼Ð½Ð¾Ðµ ÑÐ¾Ð¾Ð±Ñ‰ÐµÐ½Ð¸Ðµ");
        tracing::info!("chat-test: system line added");
    }
}
