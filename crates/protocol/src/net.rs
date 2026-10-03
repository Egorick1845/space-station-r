//! Сетевой слой протокола для lightyear (включается фичей `net`).
//!
//! Регистрирует [`ClientMessage`]/[`ServerMessage`] как lightyear-сообщения и
//! надёжный канал [`GameChannel`]. Типы сообщений остаются чистыми serde-типами.

use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{ClientMessage, ServerMessage};

/// Единственный канал игры: надёжный, упорядоченный (рукопожатие, ввод, дельты).
pub struct GameChannel;

/// Регистрирует протокол в Bevy-приложении (клиент и сервер).
pub struct ProtocolPlugin;

impl bevy_app::Plugin for ProtocolPlugin {
    fn build(&self, app: &mut bevy_app::App) {
        app.register_message::<ClientMessage>()
            .add_direction(NetworkDirection::Bidirectional);
        app.register_message::<ServerMessage>()
            .add_direction(NetworkDirection::Bidirectional);
        app.add_channel::<GameChannel>(ChannelSettings {
            mode: ChannelMode::OrderedReliable(ReliableSettings::default()),
            ..Default::default()
        })
        .add_direction(NetworkDirection::Bidirectional);
    }
}

/// Помечает, что тип сообщения сериализуем (тест совместимости с lightyear-слоем).
#[allow(dead_code)]
fn _assert_serde() {
    fn assert_bounds<T: Serialize + for<'de> Deserialize<'de> + Clone + std::fmt::Debug>() {}
    assert_bounds::<ClientMessage>();
    assert_bounds::<ServerMessage>();
}
