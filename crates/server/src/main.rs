//! Headless-сервер Space Station R.
//!
//! T0.3: Bevy MinimalPlugins, фиксированный тик-рейт 20 TPS, лог тиков.
//! T1.2: lightyear raw-connection на 127.0.0.1:7777, рукопожатие Connect/Welcome.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use lightyear::connection::server::Start;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use ssr_protocol::net::GameChannel;
use ssr_protocol::{
    ClientMessage, DEFAULT_SERVER_PORT, PROTOCOL_VERSION, ProtocolPlugin, ServerMessage,
    is_compatible,
};
use tracing_subscriber::EnvFilter;

/// Тик-рейт сервера (PLAN.md T0.3).
const TPS: f64 = 20.0;

/// Адрес, который слушает сервер.
const SERVER_ADDR: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_SERVER_PORT);

fn main() {
    // Фильтр логов берётся из RUST_LOG (например, `RUST_LOG=debug`), по умолчанию info.
    // Внимание: bevy_log включает у tracing-subscriber фичу env-filter, из-за чего
    // `fmt::try_init()` без явного фильтра строит пустой EnvFilter и молчит.
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if let Err(e) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("tracing init failed: {e}");
    }

    App::new()
        .add_plugins(
            MinimalPlugins
                .set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                    1.0 / TPS,
                )))
                // ServerPlugins использует init_state — нужен StateTransition-расписок.
                .add(bevy::state::app::StatesPlugin),
        )
        .add_plugins(ServerPlugins {
            tick_duration: Duration::from_secs_f64(1.0 / TPS),
        })
        .add_plugins(ProtocolPlugin)
        .init_resource::<Players>()
        .init_resource::<TickState>()
        .add_systems(Startup, startup)
        .add_systems(Update, (tick_logger, handle_handshake))
        .add_observer(on_link_connected)
        .add_observer(on_link_disconnected)
        .run();
}

/// Сетевой сервер: одна сущность-линк на клиента.
fn startup(mut commands: Commands) -> Result {
    // RawServer: идентификация клиента по адресу (netcode/авторизация — позже, с привязкой к сайту).
    let server = commands
        .spawn((RawServer, LocalAddr(SERVER_ADDR), ServerUdpIo::default()))
        .id();
    commands.trigger(Start { entity: server });
    tracing::info!(%SERVER_ADDR, "server listening");
    Ok(())
}

/// Список подключённых игроков: bits линка → имя.
#[derive(Resource, Default)]
struct Players {
    names: Vec<(u64, String)>,
}

fn on_link_connected(trigger: On<Add, Connected>) {
    tracing::debug!(link = ?trigger.entity, "link established");
}

fn on_link_disconnected(trigger: On<Add, Disconnected>, mut players: ResMut<Players>) {
    let bits = trigger.entity.to_bits();
    if let Some(index) = players.names.iter().position(|(id, _)| *id == bits) {
        let (_, name) = players.names.remove(index);
        tracing::info!(name, "Player disconnected");
    } else {
        tracing::debug!(link = ?trigger.entity, "link disconnected before handshake");
    }
}

/// Рукопожатие: клиент шлёт Connect, сервер проверяет версию и отвечает Welcome.
///
/// Важно: MessageSender авто-добавляется required-компонентом, а MessageReceiver
/// появляется лениво при первом входящем сообщении — поэтому приём и отправка
/// разделены на независимые запросы.
fn handle_handshake(
    mut receivers: Query<(Entity, &RemoteId, &mut MessageReceiver<ClientMessage>), With<Connected>>,
    mut senders: Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
    mut players: ResMut<Players>,
) {
    let mut connected: Vec<(Entity, String)> = Vec::new();
    for (link_entity, remote_id, mut receiver) in receivers.iter_mut() {
        for message in receiver.receive() {
            match message {
                ClientMessage::Connect {
                    protocol_version,
                    name,
                } => {
                    if !is_compatible(protocol_version) {
                        // Несовпадение версии = отказ соединения (PLAN.md T1.1).
                        tracing::warn!(
                            client = ?remote_id,
                            peer_version = protocol_version,
                            "version mismatch, connection refused"
                        );
                        continue;
                    }
                    tracing::info!(client = ?remote_id, name, "Player connected");
                    connected.push((link_entity, name));
                }
                ClientMessage::Input { .. } => {} // репликация позиций — T1.3
                ClientMessage::Interact { .. } => {} // взаимодействие — T3.1
            }
        }
    }

    for (link_entity, name) in connected {
        players.names.push((link_entity.to_bits(), name));
        if let Ok((_, mut sender)) = senders.get_mut(link_entity) {
            sender.send::<GameChannel>(ServerMessage::Welcome {
                player_entity: link_entity.to_bits(),
                protocol_version: PROTOCOL_VERSION,
            });
        }
    }
}

/// Счётчик тиков сервера с момента запуска (T0.3).
#[derive(Resource, Default)]
struct TickState {
    tick: u64,
}

fn tick_logger(mut state: ResMut<TickState>) {
    state.tick += 1;
    tracing::debug!(tick = state.tick, "server tick");
}
