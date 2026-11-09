//! Панель чата по образцу SS14 (`ChatBox.xaml` + `ChatUIController`):
//! окно справа вверху с отступом 10, фон `#25252ADD`, список сообщений
//! (шрифт 12) и поле ввода снизу.
//!
//! Каналы и цвета как в движке: OOC — `LightSkyBlue`, LOOC — `MediumTurquoise`,
//! системные — Orange, речь/эмоции — LightGray, шёпот — DarkGray. Ввод открывается по T, отправка — Enter, выход — Esc;
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
    /// Сколько раз подряд повторилось это же сообщение (0 — один раз).
    /// В сборке: `chat.coalesce_identical_messages` (GoobCCVars, default true),
    /// `ChatBox.OnMessageAdded` при совпадении текста и цвета увеличивает счётчик
    /// и заменяет предыдущую строку (`Contents.RemoveEntry(^2)`).
    pub repeat: u32,
}

/// Состояние чата: история, черновик ввода, фокус и прокрутка.
#[derive(Resource)]
pub struct ChatState {
    pub lines: Vec<ChatLine>,
    pub input: String,
    /// Поле ввода в фокусе (набор текста).
    pub focused: bool,
    /// На сколько строк прокручена история вверх от конца.
    pub scroll: usize,
    /// Выбранный канал кнопкой-селектором (`ChannelSelectorPopup`).
    pub channel: ChatChannel,
}

impl Default for ChatState {
    fn default() -> Self {
        Self {
            lines: Vec::new(),
            input: String::new(),
            focused: false,
            scroll: 0,
            // В сборке селектор по умолчанию — Local (речь персонажа).
            channel: ChatChannel::Local,
        }
    }
}

impl ChatState {
    /// Добавляет строку (история ограничена). Одинаковые подряд сообщения
    /// схлопываются в одно со счётчиком, как `chat.coalesce_identical_messages`.
    pub fn push(&mut self, channel: ChatChannel, from: impl Into<String>, text: impl Into<String>) {
        let from = from.into();
        let text = text.into();
        // Коалесценция только для чужих сообщений (у системных from пустой, но
        // их повтор тоже логично схлопывать — как в сборке, где сравнивается
        // текст и цвет).
        if let Some(last) = self.lines.last_mut()
            && last.channel == channel
            && last.from == from
            && last.text == text
        {
            last.repeat += 1;
            self.scroll = 0;
            return;
        }
        self.lines.push(ChatLine {
            channel,
            from,
            text,
            repeat: 0,
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

/// Поле ввода (клик открывает набор — как LineEdit `ChatInputBox` в сборке).
#[derive(Component)]
pub struct ChatInputField;

/// Строка ввода (показывает черновик и курсор).
#[derive(Component)]
pub struct ChatInputText;

/// Кнопка-селектор канала (`ChannelSelectorButton`): текст и цвет меняются по
/// выбранному каналу. Клик переключает канал (у нас OOC ⇄ LOOC).
#[derive(Component)]
pub struct ChatChannelButton;

/// Цвет канала — `ChatChannelExtensions.TextColor` сборки
/// (`Content.Shared/Chat/ChatChannelExtensions.cs`): Server Orange, Dead
/// MediumPurple, AdminChat HotPink, OOC LightSkyBlue, LOOC MediumTurquoise.
fn channel_color(channel: ChatChannel) -> Color {
    match channel {
        ChatChannel::Ooc => Color::srgb(0.53, 0.81, 0.98), // LightSkyBlue
        ChatChannel::Looc => Color::srgb(0.28, 0.82, 0.80), // MediumTurquoise
        ChatChannel::System => Color::srgb(1.0, 0.647, 0.0), // Orange
        ChatChannel::Dead => Color::srgb(0.576, 0.439, 0.859), // MediumPurple
        ChatChannel::AdminChat => Color::srgb(1.0, 0.412, 0.706), // HotPink
        ChatChannel::Whisper => Color::srgb(0.663, 0.663, 0.663), // DarkGray
        // `_ => Color.LightGray` — речь и эмоции.
        ChatChannel::Local | ChatChannel::Emotes => Color::srgb(0.827, 0.827, 0.827),
    }
}

/// Цвет кнопки-селектора — `ChannelSelectorButton.ChannelSelectColor`: у
/// Local/Whisper/Emotes это `_ => Color.DarkGray`, остальные — как строки.
fn selector_color(channel: ChatChannel) -> Color {
    if channel.is_ic() {
        Color::srgb(0.663, 0.663, 0.663)
    } else {
        channel_color(channel)
    }
}

/// Кнопка-селектор переключает каналы по кругу в порядке `ChatSelectChannel`
/// сборки: Local, Whisper, LOOC, OOC, Emotes, Dead, Admin (рации пока нет).
const CHANNEL_CYCLE: [ChatChannel; 7] = [
    ChatChannel::Local,
    ChatChannel::Whisper,
    ChatChannel::Looc,
    ChatChannel::Ooc,
    ChatChannel::Emotes,
    ChatChannel::Dead,
    ChatChannel::AdminChat,
];

/// Подпись кнопки канала (`hud-chatbox-select-channel-*` в ru-RU локали сборки).
fn channel_label(channel: ChatChannel) -> &'static str {
    match channel {
        ChatChannel::Local => "Рядом",
        ChatChannel::Whisper => "Шёпот",
        ChatChannel::Emotes => "Эмоции",
        ChatChannel::Ooc => "OOC",
        ChatChannel::Looc => "LOOC",
        ChatChannel::Dead => "Мёртвые",
        ChatChannel::AdminChat => "Админ",
        ChatChannel::System => "SYS",
    }
}

/// Префикс в начале строки выбирает канал — `ChatUIController.PrefixToChannel`
/// и константы `SharedChatSystem`: `>` речь, `,` шёпот, `%`/`*` эмоции,
/// `_` LOOC, `[` OOC, `\` мёртвые, `]` админ. Без префикса — канал селектора.
pub fn parse_prefix(text: &str, selected: ChatChannel) -> (ChatChannel, String) {
    let mut chars = text.chars();
    let channel = match chars.next() {
        Some('>') => Some(ChatChannel::Local),
        Some(',') => Some(ChatChannel::Whisper),
        Some('%') | Some('*') => Some(ChatChannel::Emotes),
        Some('_') => Some(ChatChannel::Looc),
        Some('[') => Some(ChatChannel::Ooc),
        Some('\\') => Some(ChatChannel::Dead),
        Some(']') => Some(ChatChannel::AdminChat),
        _ => None,
    };
    match channel {
        Some(channel) => (channel, chars.as_str().trim().to_string()),
        None => (selected, text.trim().to_string()),
    }
}

/// Подсказка в пустом поле ввода.
const INPUT_HINT: &str = "T — писать; , шёпот, * эмоция, _ LOOC, [ OOC";

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
            // Область сообщений: строки пересобираются в render_chat. Button —
            // чтобы клик по истории чата не бил по миру под ним (UI-барьер).
            panel.spawn((
                Button,
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
                    // Кнопка канала (`ChannelSelectorButton`, MinWidth = 75).
                    row.spawn((
                        Button,
                        ChatChannelButton,
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
                        Text::new(channel_label(ChatChannel::Local)),
                        super::chat_text_font(),
                        TextColor(selector_color(ChatChannel::Local)),
                    ));
                    // Поле ввода кликабельно: щелчок открывает набор текста,
                    // как LineEdit в сборке (раньше работала только клавиша T).
                    row.spawn((
                        ChatInputField,
                        Button,
                        Node {
                            flex_grow: 1.0,
                            height: px(24),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ))
                    .with_child((
                        ChatInputText,
                        Text::new(INPUT_HINT),
                        super::chat_text_font(),
                        TextColor(ui::TEXT_MUTED),
                    ));
                });
        });
}

/// Клик по кнопке канала переключает каналы по кругу (OOC → LOOC → DEAD →
/// ADMIN → OOC, как выбор канала в `ChannelSelectorPopup`); подпись и цвет
/// кнопки — `ChannelSelectColor` из `ChatChannelExtensions.TextColor`.
pub fn chat_channel_button(
    buttons: Query<&Interaction, (Changed<Interaction>, With<ChatChannelButton>)>,
    mut chat: ResMut<ChatState>,
    mut buttons_q: Query<(Entity, &Children), With<ChatChannelButton>>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
) {
    for interaction in buttons.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let index = CHANNEL_CYCLE
            .iter()
            .position(|channel| *channel == chat.channel)
            .map(|index| (index + 1) % CHANNEL_CYCLE.len())
            .unwrap_or(0);
        chat.channel = CHANNEL_CYCLE[index];
        tracing::info!(channel = ?chat.channel, "chat channel selected");
    }
    let wanted = channel_label(chat.channel);
    let wanted_color = selector_color(chat.channel);
    for (_, children) in buttons_q.iter_mut() {
        for child in children.iter() {
            let Ok((mut label, mut color)) = texts.get_mut(child) else {
                continue;
            };
            if label.0 != wanted {
                label.0 = wanted.to_string();
            }
            if color.0 != wanted_color {
                color.0 = wanted_color;
            }
        }
    }
}

/// Клик по полю ввода открывает набор текста (LineEdit в сборке фокусируется
/// кликом; раньше работала только клавиша T).
pub fn chat_input_click(
    clicks: Query<&Interaction, (Changed<Interaction>, With<ChatInputField>)>,
    mut chat: ResMut<ChatState>,
) {
    for interaction in clicks.iter() {
        if *interaction == Interaction::Pressed {
            chat.focused = true;
            chat.scroll = 0;
        }
    }
}

/// При смерти канал ввода переключается на чат мёртвых, при возвращении в
/// тело — обратно на OOC (в сборке `OnDeath`/`OnRestore`: SelectedChannel).
pub fn chat_ghost_channel(
    own: Res<OwnPlayerEntity>,
    ghosts: Query<&ssr_core::mechanics::Ghost>,
    mut chat: ResMut<ChatState>,
    mut last: Local<bool>,
) {
    let ghost = own
        .0
        .map(|entity| ghosts.get(entity).is_ok())
        .unwrap_or(false);
    if ghost == *last {
        return;
    }
    *last = ghost;
    chat.channel = if ghost {
        ChatChannel::Dead
    } else {
        ChatChannel::Local
    };
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
                // Счётчик повторов: `chat-system-repeated-message-counter`,
                // размер = 8 + min(repeat / 6, 5) в сборке (ChatBox.AddLine).
                let text = if line.repeat > 0 {
                    format!("{text} ×{}", line.repeat + 1)
                } else {
                    text
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

/// Текст и цвет строки ввода (без кнопки канала — у неё свой маркер).
type InputTextQuery<'w, 's> = Query<
    'w,
    's,
    (&'static mut Text, &'static mut TextColor),
    (With<ChatInputText>, Without<ChatChannelButton>),
>;

/// Набор текста: T — открыть, Enter — отправить, Esc — закрыть, колесо — прокрутка.
pub fn chat_input(
    mut events: MessageReader<KeyboardInput>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<Console>,
    mut chat: ResMut<ChatState>,
    mut text: InputTextQuery,
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
            let (channel, message) = parse_prefix(&text, chat.channel);
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
        (INPUT_HINT.to_string(), ui::TEXT_MUTED)
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
    // IC-речь: сервер оборачивает каждую реплику (`Имя говорит, "…"`).
    if state.1 == 3 && elapsed >= 10.0 {
        state.1 = 4;
        send(&mut senders, ChatChannel::Local, "есть кто живой?");
        tracing::info!("chat-test: Local sent");
    }
    if state.1 == 4 && elapsed >= 11.0 {
        state.1 = 5;
        send(&mut senders, ChatChannel::Whisper, "тихо, тут кто-то есть");
        tracing::info!("chat-test: Whisper sent");
    }
    if state.1 == 5 && elapsed >= 12.0 {
        state.1 = 6;
        send(&mut senders, ChatChannel::Emotes, "машет рукой");
        tracing::info!("chat-test: Emotes sent");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Коалесценция одинаковых сообщений (`chat.coalesce_identical_messages`):
    /// три одинаковых подряд дают одну строку со счётчиком 2, другое сообщение —
    /// новую строку.
    #[test]
    fn identical_messages_coalesce_with_counter() {
        let mut chat = ChatState::default();
        chat.push(ChatChannel::Ooc, "Игрок", "привет");
        chat.push(ChatChannel::Ooc, "Игрок", "привет");
        chat.push(ChatChannel::Ooc, "Игрок", "привет");
        assert_eq!(chat.lines.len(), 1);
        assert_eq!(chat.lines[0].repeat, 2);
        chat.push(ChatChannel::Ooc, "Игрок", "другое");
        assert_eq!(chat.lines.len(), 2);
        assert_eq!(chat.lines[1].repeat, 0);
        // Другой канал — не коалесцируется даже с тем же текстом.
        chat.push(ChatChannel::Looc, "Игрок", "другое");
        assert_eq!(chat.lines.len(), 3);
    }

    /// Префиксы каналов — `ChatUIController.PrefixToChannel` сборки.
    #[test]
    fn prefixes_select_channel() {
        let sel = ChatChannel::Local;
        assert_eq!(
            parse_prefix("привет", sel),
            (ChatChannel::Local, "привет".into())
        );
        assert_eq!(
            parse_prefix(">привет", ChatChannel::Ooc).0,
            ChatChannel::Local
        );
        assert_eq!(
            parse_prefix(", тсс", sel),
            (ChatChannel::Whisper, "тсс".into())
        );
        assert_eq!(parse_prefix("*машет", sel).0, ChatChannel::Emotes);
        assert_eq!(parse_prefix("%машет", sel).0, ChatChannel::Emotes);
        assert_eq!(parse_prefix("_ooc рядом", sel).0, ChatChannel::Looc);
        assert_eq!(parse_prefix("[всем", sel).0, ChatChannel::Ooc);
        assert_eq!(parse_prefix("\\дух", sel).0, ChatChannel::Dead);
        assert_eq!(parse_prefix("]админы", sel).0, ChatChannel::AdminChat);
    }

    /// Селектор по умолчанию — речь персонажа, а в цикле есть все IC-каналы.
    #[test]
    fn selector_defaults_to_local_and_cycles_ic_channels() {
        assert_eq!(ChatState::default().channel, ChatChannel::Local);
        for channel in [
            ChatChannel::Local,
            ChatChannel::Whisper,
            ChatChannel::Emotes,
        ] {
            assert!(CHANNEL_CYCLE.contains(&channel));
        }
    }
}
