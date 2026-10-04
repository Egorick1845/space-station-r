//! Headless-бот для нагрузочного теста (PLAN.md T6.1): без окна, графики и UI —
//! только сеть и случайное движение. Запуск: `SSR_BOT=Бот-1 ssr-client`.
//!
//! Один процесс = один клиент (настоящий UDP-трафик, как у живого игрока).

use std::time::Duration;

use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;

use ssr_protocol::net::GameChannel;
use ssr_protocol::{ClientMessage, ProtocolPlugin};

use crate::NET_TPS;

/// Имя бота (показывается в логе сервера и в его собственном).
#[derive(Resource)]
pub struct BotName(pub String);

/// Счётчики бота для нагрузочного лога: кадры и отправленные вводы.
#[derive(Resource, Default)]
pub struct BotStats {
    frames: u64,
    inputs: u64,
}

/// Простой ЛКГ — «случайное» движение без внешних крейтов.
#[derive(Default)]
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        let unit = (self.next() % 10_000) as f32 / 10_000.0;
        low + (high - low) * unit
    }
}

/// Запускает бота: минимальный набор плагинов (сеть + время + ввод).
pub fn run_bot(name: String) {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_plugins(bevy::state::app::StatesPlugin);
    // Клавиатура нужна `send_input`-совместимым системам и гейту консоли.
    app.add_plugins(bevy::input::InputPlugin);
    app.add_plugins(ClientPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / NET_TPS),
    });
    app.add_plugins(ProtocolPlugin);
    app.init_resource::<crate::Handshake>();
    app.init_resource::<crate::PlayerEntity>();
    app.insert_resource(crate::lobby::LobbyState::Playing);
    app.insert_resource(crate::settings::Settings::default());
    app.init_resource::<crate::inventory_ui::ActionMenu>();
    app.init_resource::<crate::doors::DeniedDoors>();
    app.init_resource::<crate::audio::SoundRequests>();
    app.init_resource::<crate::console::Console>();
    // Логи бота: в обычном клиенте их ставит DefaultPlugins, здесь — сами.
    app.add_plugins(bevy::log::LogPlugin {
        filter: std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        ..default()
    });
    app.insert_resource(BotName(name));
    app.init_resource::<BotStats>();
    // Та же регистрация репликации, что у обычного клиента (иначе паника FnsId).
    crate::register_replication(&mut app);
    app.add_systems(Startup, bot_connect);
    app.add_systems(
        Update,
        (
            crate::send_connect,
            crate::receive_server,
            bot_walk,
            bot_report,
        ),
    );
    app.run();
}

/// Бот подключается сразу при старте (аналог `enter_game` у клиента).
fn bot_connect(mut commands: Commands, settings: Res<crate::settings::Settings>) {
    commands
        .spawn((
            Client,
            LocalAddr(crate::CLIENT_ADDR),
            PeerAddr(crate::server_addr(&settings)),
            Link::default(),
            RawClient,
            UdpIo::default(),
            ReplicationReceiver,
        ))
        .trigger(Connect::from);
    tracing::info!("bot: connect triggered");
}

/// Случайное движение: держит направление 1.5–4 с, иногда стоит.
fn bot_walk(
    time: Res<Time>,
    name: Res<BotName>,
    mut stats: ResMut<BotStats>,
    player_entity: Res<crate::PlayerEntity>,
    mut state: Local<(f32, [f32; 2], Lcg, bool, f32)>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    stats.frames += 1;
    // До Welcome ввод бессмысленен (сервер ругается «input before handshake»).
    if player_entity.0.is_none() {
        return;
    }
    if !state.3 {
        // Сид из имени бота: у каждого свой маршрут, поведение детерминировано.
        let seed = name.0.bytes().fold(1u64, |acc, byte| {
            acc.wrapping_mul(31).wrapping_add(u64::from(byte))
        });
        state.2 = Lcg(seed | 1);
        state.3 = true;
    }
    state.0 -= time.delta_secs();
    if state.0 <= 0.0 {
        let angle = state.2.range(0.0, std::f32::consts::TAU);
        // Иногда бот стоит на месте (проверяем и «спокойный» профиль нагрузки).
        let direction = if state.2.range(0.0, 1.0) < 0.2 {
            [0.0, 0.0]
        } else {
            [angle.cos(), angle.sin()]
        };
        state.1 = direction;
        state.0 = state.2.range(1.5, 4.0);
    }
    // Ввод шлём с частотой тика сети (20 Гц): у headless-бота тысячи кадров
    // в секунду, ввод каждый кадр забивает канал и сервер (T6.1).
    state.4 += time.delta_secs();
    if state.4 < 1.0 / crate::NET_TPS as f32 {
        return;
    }
    state.4 = 0.0;
    stats.inputs += 1;
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Input { movement: state.1 });
    }
}

/// Раз в 5 секунд бот пишет, сколько видит игроков (интерес работает).
fn bot_report(
    time: Res<Time>,
    name: Res<BotName>,
    stats: Res<BotStats>,
    positions: Query<&ssr_core::PlayerPosition, With<Remote>>,
    mut last: Local<(f32, u64)>,
) {
    last.0 += time.delta_secs();
    if last.0 < 5.0 {
        return;
    }
    let seconds = last.0;
    last.0 = 0.0;
    let frames = stats.frames - last.1;
    last.1 = stats.frames;
    tracing::info!(
        bot = %name.0,
        visible = positions.iter().count(),
        fps = format!("{:.0}", frames as f64 / seconds as f64),
        inputs = stats.inputs,
        "bot alive"
    );
}
