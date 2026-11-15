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
use ssr_core::damage::Damageable;
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
    /// Открыт ряд кнопок выбора канала (`ChannelSelectorPopup`).
    pub selector_open: bool,
    /// Открыт список фильтров (`ChannelFilterPopup`).
    pub filter_open: bool,
    /// Каналы, скрытые фильтром (галочка снята).
    pub hidden: Vec<ChatChannel>,
    /// Свой игрок — призрак (меняет набор доступных каналов).
    pub ghost: bool,
    /// Реплики над головой, пришедшие с сервера (забирает `speech_bubbles`).
    pub pending_bubbles: Vec<ssr_protocol::SpeechBubble>,
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
            selector_open: false,
            filter_open: false,
            hidden: Vec::new(),
            ghost: false,
            pending_bubbles: Vec::new(),
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
        let shown: Vec<&ChatLine> = self
            .lines
            .iter()
            .filter(|line| !self.hidden.contains(&line.channel))
            .collect();
        let end = shown.len().saturating_sub(self.scroll);
        let start = end.saturating_sub(CHAT_VISIBLE);
        shown[start..end].to_vec().into_iter()
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

/// Порядок кнопок выбора канала — `ChannelSelectorPopup.ChannelSelectorOrder`
/// (Local, Whisper, Emotes, Radio, LOOC, OOC, Dead, Admin; рации пока нет).
const CHANNEL_CYCLE: [ChatChannel; 7] = [
    ChatChannel::Local,
    ChatChannel::Whisper,
    ChatChannel::Emotes,
    ChatChannel::Looc,
    ChatChannel::Ooc,
    ChatChannel::Dead,
    ChatChannel::AdminChat,
];

/// Доступные каналы (`ChatUIController.SelectableChannels`): живому — без
/// чата мёртвых, призраку — без речи/шёпота/эмоций, но с чатом мёртвых.
pub fn available_channels(ghost: bool) -> Vec<ChatChannel> {
    CHANNEL_CYCLE
        .into_iter()
        .filter(|channel| {
            if ghost {
                !channel.is_ic()
            } else {
                *channel != ChatChannel::Dead
            }
        })
        .collect()
}

/// Следующий доступный канал (`CycleChatChannelForward`, Tab).
pub fn next_channel(current: ChatChannel, ghost: bool) -> ChatChannel {
    let channels = available_channels(ghost);
    let index = channels
        .iter()
        .position(|channel| *channel == current)
        .map(|index| (index + 1) % channels.len())
        .unwrap_or(0);
    channels[index]
}

/// Префикс канала для подписи кнопки (`ChatUIController.ChannelPrefixes`).
fn channel_prefix(channel: ChatChannel) -> char {
    match channel {
        ChatChannel::Local => '>',
        ChatChannel::Whisper => ',',
        ChatChannel::Emotes => '%',
        ChatChannel::Looc => '_',
        ChatChannel::Ooc => '[',
        ChatChannel::Dead => '\\',
        ChatChannel::AdminChat => ']',
        ChatChannel::System => ' ',
    }
}

/// Подпись кнопки в ряду выбора (`hud-chatbox-select-name-prefixed`:
/// `{ $prefix } { $name }`).
pub fn selector_item_label(channel: ChatChannel) -> String {
    format!("{} {}", channel_prefix(channel), channel_label(channel))
}

/// Подпись строки фильтра (`hud-chatbox-channel-*`).
fn filter_label(channel: ChatChannel) -> &'static str {
    match channel {
        ChatChannel::AdminChat => "Админ Чат",
        ChatChannel::System => "Сервер",
        other => channel_label(other),
    }
}

/// Каналы в окне фильтра (`FilterableChannels`).
fn filterable_channels(ghost: bool) -> Vec<ChatChannel> {
    let mut channels = vec![
        ChatChannel::Local,
        ChatChannel::Whisper,
        ChatChannel::Emotes,
        ChatChannel::Looc,
        ChatChannel::Ooc,
    ];
    if ghost {
        channels.push(ChatChannel::Dead);
    }
    channels.push(ChatChannel::AdminChat);
    channels.push(ChatChannel::System);
    channels
}

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

/// Подсказка в пустом поле ввода (`hud-chatbox-info`: `{ $talk-key } чтобы
/// говорить, { $cycle-key } для переключения каналов.`).
const INPUT_HINT: &str = "T чтобы говорить, Tab для переключения каналов.";
/// Цвета иконки фильтра (`ChannelFilterButton`).
const FILTER_NORMAL: Color = Color::srgb(
    0x7b as f32 / 255.0,
    0x7e as f32 / 255.0,
    0x9e as f32 / 255.0,
);
const FILTER_HOVERED: Color = Color::srgb(
    0x96 as f32 / 255.0,
    0x99 as f32 / 255.0,
    0xbb as f32 / 255.0,
);
const FILTER_PRESSED: Color = Color::srgb(
    0x78 as f32 / 255.0,
    0x9b as f32 / 255.0,
    0x8c as f32 / 255.0,
);
/// Сдвиг выпадающего списка фильтров влево (`FilterDropdownOffset`).
const FILTER_DROPDOWN_OFFSET: f32 = 120.0;

/// Кнопка фильтра каналов (`ChannelFilterButton`).
#[derive(Component)]
pub struct ChatFilterButton;

/// Иконка на кнопке фильтра (красится по состоянию кнопки).
#[derive(Component)]
pub struct ChatFilterIcon;

/// Ряд кнопок выбора канала (`ChannelSelectorPopup`).
#[derive(Component)]
pub struct ChatSelectorPopup;

/// Кнопка канала в ряду выбора.
#[derive(Component, Clone, Copy)]
pub struct ChatSelectorItem(pub ChatChannel);

/// Список фильтров (`ChannelFilterPopup`).
#[derive(Component)]
pub struct ChatFilterPopup;

/// Строка-галочка фильтра.
#[derive(Component, Clone, Copy)]
pub struct ChatFilterItem(pub ChatChannel);

/// Создаёт панель чата при входе в игру.
pub fn spawn_chat(
    mut commands: Commands,
    own: Res<OwnPlayerEntity>,
    roots: Query<(), With<ChatRoot>>,
    assets: Res<AssetServer>,
) {
    if own.0.is_none() || !roots.is_empty() {
        return;
    }
    let filter_icon: Handle<Image> =
        assets.load("sprites/ss14/Interface/Nano/filter.svg.96dpi.png");
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
                    // Кнопка фильтра каналов (`ChannelFilterButton`): иконка
                    // `Interface/Nano/filter.svg.96dpi.png`, цвет по состоянию.
                    row.spawn((
                        Button,
                        ChatFilterButton,
                        Node {
                            width: px(24),
                            height: px(24),
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::Center,
                            ..default()
                        },
                        BackgroundColor(ui::GLASS_BUTTON),
                    ))
                    .with_child((
                        ChatFilterIcon,
                        ImageNode::new(filter_icon).with_color(FILTER_NORMAL),
                        Node {
                            width: px(16),
                            height: px(16),
                            ..default()
                        },
                    ));
                });
        });
}

/// Кнопка канала (`ChannelSelectorButton`, ToggleMode) открывает и закрывает
/// ряд выбора (`ChannelSelectorPopup`); подпись и цвет кнопки —
/// `ChannelSelectColor`. Если канал стал недоступен (смерть/возврат в тело) —
/// селектор переходит на первый доступный, как `SetChannels`.
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
        chat.selector_open = !chat.selector_open;
        chat.filter_open = false;
    }
    if !available_channels(chat.ghost).contains(&chat.channel) {
        chat.channel = available_channels(chat.ghost)[0];
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

/// Подпись открытых выпадающих окон чата: пересобираем их только при смене.
type PopupSignature = (bool, bool, bool, Vec<ChatChannel>);

/// Ряд выбора канала и список фильтров под полем ввода. Пересобираются при
/// смене состояния; клики обрабатывает `chat_popup_clicks`.
pub fn chat_popups(
    mut commands: Commands,
    chat: Res<ChatState>,
    roots: Query<Entity, With<ChatRoot>>,
    selectors: Query<Entity, With<ChatSelectorPopup>>,
    filters: Query<Entity, With<ChatFilterPopup>>,
    mut last: Local<Option<PopupSignature>>,
) {
    let signature = (
        chat.selector_open,
        chat.filter_open,
        chat.ghost,
        chat.hidden.clone(),
    );
    if last.as_ref() == Some(&signature) {
        return;
    }
    let Ok(root) = roots.single() else {
        return;
    };
    *last = Some(signature);
    for entity in selectors.iter().chain(filters.iter()) {
        commands.entity(entity).despawn();
    }
    if chat.selector_open {
        // Горизонтальный ряд кнопок с отступом 1 (`SeparationOverride = 1`)
        // прямо под полем ввода.
        commands.entity(root).with_children(|panel| {
            panel
                .spawn((
                    ChatSelectorPopup,
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Percent(100.0),
                        left: px(0),
                        margin: UiRect::top(px(2)),
                        column_gap: px(1),
                        ..default()
                    },
                    GlobalZIndex(50),
                ))
                .with_children(|row| {
                    for channel in available_channels(chat.ghost) {
                        row.spawn((
                            Button,
                            ChatSelectorItem(channel),
                            Node {
                                height: px(24),
                                padding: UiRect::axes(px(6), px(2)),
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(ui::GLASS_BUTTON),
                        ))
                        .with_child((
                            Text::new(selector_item_label(channel)),
                            super::chat_text_font(),
                            TextColor(ui::TEXT),
                        ));
                    }
                });
        });
    }
    if chat.filter_open {
        // Список галочек: левый край сдвинут на `FilterDropdownOffset` от
        // кнопки фильтра (она у правого края панели).
        commands.entity(root).with_children(|panel| {
            panel
                .spawn((
                    ChatFilterPopup,
                    Node {
                        position_type: PositionType::Absolute,
                        top: Val::Percent(100.0),
                        right: px(24.0 + 2.0 - FILTER_DROPDOWN_OFFSET),
                        margin: UiRect::top(px(2)),
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::all(px(4)),
                        row_gap: px(2),
                        min_width: px(FILTER_DROPDOWN_OFFSET + 24.0),
                        ..default()
                    },
                    BackgroundColor(ui::CHAT_BACKGROUND),
                    GlobalZIndex(50),
                ))
                .with_children(|list| {
                    for channel in filterable_channels(chat.ghost) {
                        let shown = !chat.hidden.contains(&channel);
                        list.spawn((
                            Button,
                            ChatFilterItem(channel),
                            Node {
                                height: px(20),
                                column_gap: px(6),
                                align_items: AlignItems::Center,
                                ..default()
                            },
                        ))
                        .with_children(|item| {
                            // Галочка `CheckBox`: квадрат 12 px, заливка акцентом.
                            item.spawn((
                                Node {
                                    width: px(12),
                                    height: px(12),
                                    border: UiRect::all(px(1)),
                                    ..default()
                                },
                                BorderColor::all(ui::TEXT_MUTED),
                                BackgroundColor(if shown { ui::ACCENT } else { Color::NONE }),
                            ));
                            item.spawn((
                                Text::new(filter_label(channel)),
                                super::chat_text_font(),
                                TextColor(ui::TEXT),
                            ));
                        });
                    }
                });
        });
    }
}

/// Клики по ряду выбора, по фильтрам и по кнопке фильтра; цвет иконки
/// фильтра — Normal/Hover/Pressed из `ChannelFilterButton`.
#[allow(clippy::type_complexity)]
pub fn chat_popup_clicks(
    mut chat: ResMut<ChatState>,
    items: Query<(&Interaction, &ChatSelectorItem), Changed<Interaction>>,
    filters: Query<(&Interaction, &ChatFilterItem), Changed<Interaction>>,
    filter_button: Query<&Interaction, With<ChatFilterButton>>,
    filter_changed: Query<&Interaction, (Changed<Interaction>, With<ChatFilterButton>)>,
    mut icons: Query<&mut ImageNode, With<ChatFilterIcon>>,
) {
    for (interaction, item) in items.iter() {
        if *interaction == Interaction::Pressed {
            chat.channel = item.0;
            chat.selector_open = false;
            tracing::info!(channel = ?item.0, "chat channel selected");
        }
    }
    for (interaction, item) in filters.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Some(index) = chat.hidden.iter().position(|c| *c == item.0) {
            chat.hidden.remove(index);
        } else {
            chat.hidden.push(item.0);
        }
        chat.scroll = 0;
    }
    for interaction in filter_changed.iter() {
        if *interaction == Interaction::Pressed {
            chat.filter_open = !chat.filter_open;
            chat.selector_open = false;
        }
    }
    let color = match filter_button.iter().next() {
        _ if chat.filter_open => FILTER_PRESSED,
        Some(Interaction::Hovered) => FILTER_HOVERED,
        Some(Interaction::Pressed) => FILTER_PRESSED,
        _ => FILTER_NORMAL,
    };
    for mut icon in icons.iter_mut() {
        if icon.color != color {
            icon.color = color;
        }
    }
}

/// Индикатор набора (`TypingIndicatorSystem`): «печатает», пока текст
/// менялся последние 2 с; «ждёт», пока поле в фокусе; иначе — нет. Серверу
/// уходит только смена состояния.
pub fn chat_typing_indicator(
    time: Res<Time>,
    chat: Res<ChatState>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut state: Local<(String, f32, Option<ssr_protocol::TypingState>)>,
) {
    let now = time.elapsed_secs();
    if chat.input != state.0 {
        state.0 = chat.input.clone();
        if !chat.input.is_empty() {
            state.1 = now;
        }
    }
    let typing = !chat.input.is_empty() && now - state.1 < ssr_core::speech::TYPING_TIMEOUT;
    let wanted = match ssr_core::speech::typing_state(chat.focused, typing) {
        ssr_core::speech::TypingIndicator::None => ssr_protocol::TypingState::None,
        ssr_core::speech::TypingIndicator::Idle => ssr_protocol::TypingState::Idle,
        ssr_core::speech::TypingIndicator::Typing => ssr_protocol::TypingState::Typing,
    };
    if state.2 == Some(wanted) {
        return;
    }
    let mut sent = false;
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Typing { state: wanted });
        sent = true;
    }
    if sent {
        state.2 = Some(wanted);
        tracing::debug!(?wanted, "typing state sent");
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
    chat.ghost = ghost;
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
#[allow(clippy::too_many_arguments)]
pub fn chat_input(
    mut events: MessageReader<KeyboardInput>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    keys: Res<ButtonInput<KeyCode>>,
    binds: Res<crate::keybinds::Keybinds>,
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
        && binds.just_pressed(crate::keybinds::KeyFunction::FocusChat, &keys)
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
                // `CycleChatChannelForward` (Tab) — следующий доступный канал.
                Key::Tab => {
                    let next = next_channel(chat.channel, chat.ghost);
                    chat.channel = next;
                }
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
        // После отправки поле теряет фокус, как `ChatInputBox` в сборке.
        chat.focused = false;
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
    healths: Query<&Damageable>,
    mut chat: ResMut<ChatState>,
    mut last: Local<bool>,
) {
    let down = own
        .0
        .and_then(|entity| healths.get(entity).ok())
        // Крит или смерть (`MobState`) — без сознания.
        .is_some_and(|health| health.incapacitated());
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
    // Облачко набора и ряд выбора канала — для снимка.
    if state.1 == 6 && elapsed >= 12.5 {
        state.1 = 7;
        chat.focused = true;
        chat.input = "печатаю".to_string();
        chat.selector_open = true;
        tracing::info!("chat-test: typing + selector open");
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
