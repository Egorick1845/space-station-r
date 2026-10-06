//! Сетевой слой протокола для lightyear (включается фичей `net`).
//!
//! Регистрирует [`ClientMessage`]/[`ServerMessage`] как lightyear-сообщения и
//! надёжный канал [`GameChannel`]. Типы сообщений остаются чистыми serde-типами.

use lightyear::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{ClientMessage, ServerMessage};

/// Регистрирует реплицируемые компоненты — И на сервере, И на клиенте.
///
/// Порядок `replicate()` в lightyear обязан совпадать на обеих сторонах: если
/// компонент зарегистрирован только на сервере, клиент ловит
/// `unable to apply update message ... Hit the end of buffer` и мир не грузится.
/// Держать список в одном месте — единственный способ не разойтись (эту ошибку
/// мы ловили трижды: Clothing, Sex и ранее).
pub fn register_replication(app: &mut bevy_app::App) {
    use ssr_core::atmosphere::ChunkAtmosphere;
    use ssr_core::clothing::Clothing;
    use ssr_core::inventory::{
        Container, Hands, Health, HeldBy, Inventory, Item, ItemPosition, ItemStorage,
    };
    use ssr_core::mechanics::{
        FacialHair, Ghost, Hair, KnockedDown, PlayerName, Sex, Sprinting, Ssd,
    };
    use ssr_core::power::{Cable, Consumer, Generator, Light, Powered};
    use ssr_core::roles::PlayerRole;
    use ssr_core::stamina::Stamina;
    use ssr_core::structures::Structure;
    use ssr_core::tiles::TileChunkData;
    use ssr_core::weapons::{AmmoProvider, Cartridge, Gun, Projectile, WorldSound};
    use ssr_core::{Door, PlayerPosition, Species};

    app.component::<PlayerPosition>().replicate();
    app.component::<TileChunkData>().replicate();
    app.component::<Door>().replicate();
    app.component::<Inventory>().replicate();
    app.component::<Item>().replicate();
    app.component::<Hands>().replicate();
    app.component::<Health>().replicate();
    app.component::<HeldBy>().replicate();
    app.component::<Container>().replicate();
    app.component::<ItemStorage>().replicate();
    app.component::<ItemPosition>().replicate();
    app.component::<Clothing>().replicate();
    app.component::<Sprinting>().replicate();
    app.component::<Stamina>().replicate();
    app.component::<Sex>().replicate();
    app.component::<PlayerRole>().replicate();
    app.component::<Species>().replicate();
    app.component::<ChunkAtmosphere>().replicate();
    app.component::<Cable>().replicate();
    app.component::<Generator>().replicate();
    app.component::<Consumer>().replicate();
    app.component::<Light>().replicate();
    app.component::<Powered>().replicate();
    app.component::<KnockedDown>().replicate();
    app.component::<Ghost>().replicate();
    app.component::<Ssd>().replicate();
    app.component::<PlayerName>().replicate();
    app.component::<Hair>().replicate();
    app.component::<FacialHair>().replicate();
    app.component::<Structure>().replicate();
    // Оружие/патроны/снаряды (W-план).
    app.component::<Gun>().replicate();
    app.component::<AmmoProvider>().replicate();
    app.component::<Cartridge>().replicate();
    app.component::<Projectile>().replicate();
    app.component::<WorldSound>().replicate();
}

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
