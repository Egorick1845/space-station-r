//! Headless-сервер Space Station R.
//!
//! T0.3: Bevy MinimalPlugins, фиксированный тик-рейт 20 TPS, лог тиков.
//! T1.2: lightyear raw-connection на 127.0.0.1:7777, рукопожатие Connect/Welcome.
//! T1.3: ввод клиента двигает сущность игрока; позиция реплицируется компонентом.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use lightyear::connection::server::Start;
use lightyear::prelude::server::*;
use lightyear::prelude::*;
use ssr_core::{PLAYER_MOVE_SPEED, PlayerPosition};
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

    let mut app = App::new();
    app.add_plugins(
        MinimalPlugins
            .set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / TPS,
            )))
            // ServerPlugins использует init_state — нужен StateTransition-расписок.
            .add(bevy::state::app::StatesPlugin),
    );
    app.add_plugins(ServerPlugins {
        tick_duration: Duration::from_secs_f64(1.0 / TPS),
    });
    app.add_plugins(ProtocolPlugin);
    app.init_resource::<Players>();
    app.init_resource::<TickState>();
    app.add_systems(Startup, startup);
    app.add_systems(Update, (tick_logger, handle_client_messages, movement));
    app.add_observer(on_link_connected);
    app.add_observer(on_link_disconnected);
    // Регистрация реплицируемых компонентов — одинакова на сервере и клиенте (T1.3).
    app.component::<PlayerPosition>().replicate();
    app.run();
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

/// Подключённый игрок: линк (по bits), имя и его игровая сущность.
#[derive(Resource, Default)]
struct Players {
    entries: Vec<(u64, String, Entity)>,
}

impl Players {
    fn player_entity(&self, link_bits: u64) -> Option<Entity> {
        self.entries
            .iter()
            .find(|(id, _, _)| *id == link_bits)
            .map(|(_, _, entity)| *entity)
    }

    fn remove_by_link(&mut self, link_bits: u64) -> Option<(String, Entity)> {
        let index = self
            .entries
            .iter()
            .position(|(id, _, _)| *id == link_bits)?;
        let (_, name, entity) = self.entries.remove(index);
        Some((name, entity))
    }
}

/// Текущий ввод игрока; сервер применяет его каждый тик (ADR-3).
/// Не реплицируется — это серверная деталь применения ввода.
#[derive(Component, Default)]
struct PlayerInput([f32; 2]);

fn on_link_connected(trigger: On<Add, Connected>, mut commands: Commands) {
    // ReplicationSender нужен на линке, чтобы сервер слал компоненты этому клиенту (T1.3).
    commands.entity(trigger.entity).insert(ReplicationSender);
    tracing::debug!(link = ?trigger.entity, "link established");
}

fn on_link_disconnected(
    trigger: On<Add, Disconnected>,
    mut commands: Commands,
    mut players: ResMut<Players>,
) {
    let bits = trigger.entity.to_bits();
    if let Some((name, entity)) = players.remove_by_link(bits) {
        // despawn реплицируется: у клиентов сущность игрока удалится (T1.3).
        commands.entity(entity).despawn();
        tracing::info!(name, "Player disconnected");
    } else {
        tracing::debug!(link = ?trigger.entity, "link disconnected before handshake");
    }
}

/// Единая точка приёма сообщений клиента: рукопожатие (Connect/Welcome, T1.2)
/// и ввод (Input → PlayerInput, T1.3).
///
/// ВАЖНО: MessageReceiver осушается одним вызовом receive(), поэтому все
/// обработчики сообщений клиента живут в этой одной системе — иначе системы
/// конкурировали бы за один буфер и теряли сообщения.
fn handle_client_messages(
    mut commands: Commands,
    mut receivers: Query<(Entity, &RemoteId, &mut MessageReceiver<ClientMessage>), With<Connected>>,
    mut senders: Query<(Entity, &mut MessageSender<ServerMessage>), With<Connected>>,
    mut inputs: Query<&mut PlayerInput>,
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
                ClientMessage::Input { movement } => {
                    let Some(player) = players.player_entity(link_entity.to_bits()) else {
                        tracing::warn!(link = ?link_entity, "input for unknown player");
                        continue;
                    };
                    match inputs.get_mut(player) {
                        Ok(mut input) => {
                            input.0 = movement;
                            tracing::debug!(player = ?player, input = ?movement, "input applied");
                        }
                        Err(e) => tracing::warn!(player = ?player, error = %e, "no PlayerInput"),
                    }
                }
                ClientMessage::Interact { .. } => {} // взаимодействие — T3.1
            }
        }
    }

    for (link_entity, name) in connected {
        // Игровая сущность игрока: позиция реплицируется всем клиентам (T1.3).
        let player = commands
            .spawn((
                PlayerPosition::default(),
                PlayerInput::default(),
                Replicate::to_clients(NetworkTarget::All),
            ))
            .id();
        players.entries.push((link_entity.to_bits(), name, player));

        if let Ok((_, mut sender)) = senders.get_mut(link_entity) {
            sender.send::<GameChannel>(ServerMessage::Welcome {
                player_entity: player.to_bits(),
                protocol_version: PROTOCOL_VERSION,
            });
        }
    }
}

/// Двигает игроков по их вводу; результат реплицируется компонентом PlayerPosition.
fn movement(time: Res<Time>, mut players: Query<(&PlayerInput, &mut PlayerPosition)>) {
    let dt = time.delta_secs();
    for (input, mut position) in players.iter_mut() {
        let dir = Vec2::from_array(input.0);
        if dir == Vec2::ZERO {
            continue;
        }
        let next = Vec2::from_array(position.0) + dir.normalize_or_zero() * PLAYER_MOVE_SPEED * dt;
        position.0 = next.to_array();
        tracing::debug!(position = ?position.0, "player moved");
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
