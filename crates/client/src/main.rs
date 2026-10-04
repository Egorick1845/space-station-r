//! Bevy-клиент Space Station R.
//!
//! T0.2: окно 1280×720, тёмный фон, спрайт в центре, движение WASD/стрелками.
//! T1.2: lightyear raw-connection к серверу, рукопожатие Connect/Welcome.
//! T1.3: ввод шлётся на сервер; позиция игрока — реплицированная (ADR-3),
//!       без локального предсказания (запланировано после T2.2).

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;

use bevy::asset::AssetId;
use bevy::prelude::*;
use bevy::text::Font;
use bevy::window::{Window, WindowPlugin, WindowResolution};
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::{GAME_NAME, PlayerPosition};

mod appearance;
mod audio;
mod bot;
mod chat;
mod console;
mod containers;
mod content;
mod crafting;
mod doors;
mod hud;
mod humanoid;
mod inventory_ui;
mod light_gpu;
mod lighting;
mod lobby;
mod power_view;
mod rsi;
mod settings;
mod tiles;
mod ui_theme;
mod windows;
mod world_items;
use ssr_protocol::net::GameChannel;
use ssr_protocol::{
    ClientMessage, DEFAULT_SERVER_PORT, PROTOCOL_VERSION, ProtocolPlugin, ServerMessage,
    is_compatible,
};

/// Тик-рейт сети (совпадает с сервером, T0.3).
const NET_TPS: f64 = 20.0;

/// Локальный адрес по умолчанию. Порт переопределяется `SSR_PORT` —
/// тестовые прогоны идут на отдельном порту, не мешая игре в 7777.
fn default_server_addr() -> SocketAddr {
    let port = std::env::var("SSR_PORT")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_SERVER_PORT);
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

/// Адрес сервера для подключения (T5.4): из настроек («host:port», можно имя
/// хоста), иначе локальный адрес по умолчанию.
fn server_addr(settings: &settings::Settings) -> SocketAddr {
    let address = settings.server.trim();
    if !address.is_empty() {
        if let Ok(parsed) = address.parse::<SocketAddr>() {
            return parsed;
        }
        use std::net::ToSocketAddrs;
        if let Ok(mut resolved) = address.to_socket_addrs()
            && let Some(first) = resolved.next()
        {
            return first;
        }
        tracing::warn!(address, "адрес сервера не разобран, беру локальный");
    }
    default_server_addr()
}

/// Локальный конец линка (порт 0 — любой свободный).
const CLIENT_ADDR: SocketAddr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0);

/// Имя игрока до появления экрана входа (T5.4).
const DEV_PLAYER_NAME: &str = "SSR-dev";

/// Длительность серверного тика: интерполяция проигрывает путь между двумя
/// последними серверными позициями ровно за это время (без рывков).
pub(crate) const NET_TICK_SECS: f32 = 1.0 / NET_TPS as f32;

fn main() {
    // SSR_BOT=<имя> — headless-бот для нагрузочного теста (T6.1).
    if let Ok(bot_name) = std::env::var("SSR_BOT") {
        bot::run_bot(bot_name);
        return;
    }

    // SSR_DUMP_ONLY=1 — офлайн-дамп чанков карты в PNG (без окна и сети):
    // глазами проверить сглаживание стен и тайлы.
    if std::env::var_os("SSR_DUMP_ONLY").is_some() {
        let map = std::env::var("SSR_MAP").unwrap_or_else(|_| "test.ron".to_string());
        let out = ssr_core::repo_root().join("target/chunk-dump");
        match tiles::dump_map_chunks(&map, &out) {
            Ok(count) => println!("chunks dumped: {count} -> {}", out.display()),
            Err(e) => eprintln!("dump failed: {e}"),
        }
        return;
    }

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: format!("{GAME_NAME} — dev client"),
                    resolution: WindowResolution::new(1280, 720),
                    ..default()
                }),
                ..default()
            })
            .set(asset_plugin()),
    );
    app.add_plugins(ClientPlugins {
        tick_duration: std::time::Duration::from_secs_f64(1.0 / NET_TPS),
    });
    app.add_plugins(ProtocolPlugin);
    app.init_resource::<Handshake>();
    app.init_resource::<PlayerEntity>();
    app.init_resource::<tiles::ChunkRenderState>();
    app.init_resource::<lobby::LobbyState>();
    app.init_resource::<inventory_ui::OwnPlayerEntity>();
    app.init_resource::<inventory_ui::ActionMenu>();
    app.init_resource::<inventory_ui::DragItem>();
    app.init_resource::<console::Console>();
    app.init_resource::<chat::ChatState>();
    app.init_resource::<windows::WindowPositions>();
    app.init_resource::<settings::Settings>();
    app.init_resource::<doors::DeniedDoors>();
    app.init_resource::<crafting::CraftingState>();
    app.init_resource::<hud::HudState>();
    app.init_resource::<hud::Placement>();
    app.init_resource::<lighting::LightScene>();
    app.init_resource::<appearance::AppearanceUi>();
    app.init_resource::<appearance::ScrollDrag>();
    app.init_resource::<inventory_ui::InventoryUi>();
    app.init_resource::<audio::SoundRequests>();
    // Ленивая подгрузка RSI (T5.3): обрабатываем заявки из реестра каждый кадр.
    app.add_systems(Update, rsi::load_requested_rsi);
    // Боевой режим: иконка кнопки действия и маркер у курсора.
    app.add_systems(Update, (hud::sync_combat_button, hud::combat_cursor_marker));
    // Спавн-меню: прокрутка догоняет цель (rate 15, как ScrollBar в SS14).
    app.add_systems(Update, hud::spawn_scroll_anim);
    // Кадр мира как в SS14: ограничение видимой области (ScalingViewport).
    app.add_systems(Update, fit_world_viewport);
    // Окно выбора внешности (P) — отдельной группой: у кортежей есть предел.
    app.add_systems(
        Update,
        (
            // Окно внешности на P убрано по просьбе владельца: смена причёски
            // и бороды осталась в окне персонажа (клавиша I).
            appearance::appearance_sync,
            appearance::appearance_search_click,
            appearance::appearance_search_input,
            appearance::appearance_scroll,
            appearance::appearance_scroll_anim,
            appearance::appearance_scroll_apply,
            appearance::appearance_scrollbar_drag,
            appearance::appearance_click,
            appearance::render_appearance_menu,
            hud::tint_scrollbar_grabber,
        )
            .chain(),
    );
    // Текстуры интерфейса SS14 (слоты, Storage, Nano-кнопки) — сразу на старте.
    app.add_systems(Startup, ui_theme::load_ui_theme);
    // Текстуры GPU-конвейера света (карты теней/FOV/света) — один раз на старте.
    app.add_systems(Startup, light_gpu::setup_light_gpu);
    // FPS-диагностика нужна строке FPS в углу (включается в настройках).
    app.add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default());
    // RsiRegistry строится сразу после DefaultPlugins: нужен и игроку (обезьяна),
    // и дверям (closed/open) уже на Startup.
    let rsi_root = Path::new(&assets_file_path()).join("sprites/ss14");
    let registry = app
        .world_mut()
        .resource_scope(|world, mut images: Mut<Assets<Image>>| {
            let mut layouts = world.resource_mut::<Assets<TextureAtlasLayout>>();
            rsi::build_registry(&mut images, &mut layouts, &rsi_root)
        });
    app.insert_resource(registry);
    app.add_systems(
        Startup,
        (
            setup_camera,
            startup_map,
            lighting::setup_lighting,
            install_default_font,
            settings::load_settings,
            settings::apply_saved_window_mode,
            content::load_content,
            settings::spawn_fps_text,
            audio::load_sounds,
        ),
    );
    app.add_systems(
        Update,
        (
            send_connect,
            send_input,
            receive_server,
            apply_player_state,
            count_replicated,
            tiles::render_map_chunks,
            tiles::despawn_orphan_chunks,
            camera_follow_player,
            lighting::update_lighting,
            doors::spawn_door_visuals,
            doors::update_door_visuals,
            doors::apply_denied_doors,
            doors::auto_interact,
            doors::hover_outline,
        )
            .run_if(in_game),
    );
    // UI-сервисы: перетаскивание окон, звук, настройки, консоль.
    app.add_systems(
        Update,
        (
            hotkeys,
            windows::drag_windows,
            audio::door_sounds,
            audio::damage_sound,
            audio::build_sound,
            audio::footsteps,
            audio::play_requested_sounds,
            settings::apply_volume,
            settings::update_fps,
            console::toggle_console,
            console::console_input,
            console::update_console_text,
            crafting::toggle_crafting,
            crafting::render_crafting,
            crafting::craft_click,
        )
            .run_if(in_game),
    );
    app.add_systems(
        Update,
        (
            inventory_ui::resolve_own_player,
            inventory_ui::spawn_remote_players,
            inventory_ui::sync_remote_players,
            inventory_ui::remote_player_interp,
            inventory_ui::sync_inhand_items,
            inventory_ui::render_inventory_panel,
            inventory_ui::render_hands_panel,
            inventory_ui::spawn_health_hud,
            inventory_ui::update_health_hud,
            inventory_ui::update_role_hud,
            inventory_ui::update_atmos_hud,
            inventory_ui::render_action_menu,
            inventory_ui::inventory_slot_click,
            inventory_ui::hands_ui_click,
            inventory_ui::world_click,
            inventory_ui::action_menu_click,
            inventory_ui::panel_buttons_click,
            inventory_ui::equip_slot_click,
            inventory_ui::render_character_panel,
        )
            .run_if(in_game),
    );
    // Перетаскивание предметов и его тест-режим — отдельной группой
    // (у кортежей add_systems есть предел числа систем).
    app.add_systems(
        Update,
        (
            inventory_ui::drag_start,
            inventory_ui::drag_ghost,
            inventory_ui::drag_release,
            inventory_ui::drag_test_mode,
        )
            .run_if(in_game),
    );
    // Тело и одежда игроков — отдельной группой (у кортежей есть предел).
    app.add_systems(
        Update,
        (
            humanoid::sync_bodies,
            humanoid::sync_worn_clothes,
            humanoid::update_facing,
            humanoid::update_knocked,
            humanoid::debug_body,
        )
            .run_if(in_game),
    );
    // HUD (запросы владельца): верхняя панель, боковые кнопки, F5/F7-меню.
    app.add_systems(
        Update,
        (
            chat::spawn_chat,
            chat::render_chat,
            chat::chat_input,
            chat::chat_health_notices,
            chat::chat_test_mode,
            hud::spawn_hud,
            hud::hud_hotkeys,
            hud::hud_click,
            hud::hud_button_tint,
            hud::update_ghost_bar,
            hud::admin_player_click,
            hud::menu_buttons_click,
            hud::render_spawn_menu,
            hud::render_admin_menu,
            hud::render_warp_menu,
            hud::menu_scroll,
            hud::spawn_menu_input,
            hud::update_placement_ghost,
            hud::placement_click,
            hud::mech_test_mode,
        )
            .run_if(in_game),
    );
    // Настройки (Esc) работают и в лобби, и в игре (T5.4).
    app.add_systems(
        Update,
        (
            settings::toggle_settings_menu,
            settings::settings_click,
            settings::settings_tab_click,
            settings::volume_slider_drag,
            settings::update_settings_text,
        ),
    );
    // Тест-режимы клиента (SSR_*_TEST) — отдельной группой.
    app.add_systems(
        Update,
        (
            inventory_ui::inventory_test_mode,
            inventory_ui::build_test_mode,
            inventory_ui::attack_test_mode,
            crafting::craft_test_mode,
            console::admin_test_mode,
        )
            .run_if(in_game),
    );
    app.add_systems(
        Update,
        (hud::close_windows_on_escape, screenshot_test_mode)
            .run_if(in_game)
            .chain(),
    );
    app.add_systems(
        Update,
        (
            power_view::spawn_power_visuals,
            power_view::update_cables,
            power_view::update_power_visuals,
            containers::spawn_crate_visuals,
            containers::update_crate_visuals,
            containers::render_container_panel,
            containers::container_slot_click,
            containers::container_close_click,
            containers::container_test_mode,
            world_items::sync_floor_item_icons,
            world_items::floor_item_click,
            world_items::pickup_test_mode,
        )
            .run_if(in_game),
    );
    // Лобби — только в релизной сборке (в dev сразу в игру; для отладки
    // лобби в dev: SSR_LOBBY=1; тестовый обход лобби: SSR_AUTO_PLAY=1).
    let force_lobby = std::env::var_os("SSR_LOBBY").is_some();
    let auto_play = std::env::var_os("SSR_AUTO_PLAY").is_some();
    let show_lobby = !auto_play && (force_lobby || cfg!(not(debug_assertions)));
    if show_lobby {
        app.init_resource::<lobby::LobbyInput>();
        app.add_systems(Startup, lobby::spawn_lobby);
        app.add_systems(
            Update,
            (
                enter_game,
                lobby::lobby_button,
                lobby::lobby_field_click,
                lobby::lobby_text_input,
                lobby::lobby_field_text,
                lobby::animate_lobby_background,
                lobby::lobby_auto_ready,
            ),
        );
    } else {
        app.insert_resource(lobby::LobbyState::Playing);
        app.add_systems(Update, enter_game);
    }
    // Репликация — см. register_replication (тот же список у бота).
    register_replication(&mut app);
    app.run();
}

/// Ставит Noto Sans (из сборки мини-станции, OFL) шрифтом по умолчанию:
/// встроенный шрифт Bevy — без кириллицы, из-за него в UI были «квадратики».
fn install_default_font(mut fonts: ResMut<Assets<Font>>) {
    let path = Path::new(&assets_file_path()).join("fonts/NotoSans-Regular.ttf");
    match std::fs::read(&path) {
        Ok(bytes) => {
            // Подмена дефолтного ассета: все тексты без явного шрифта
            // начинают использовать Noto Sans с кириллицей.
            let _ = fonts.insert(AssetId::default(), Font::from_bytes(bytes));
            tracing::info!("шрифт Noto Sans (кириллица) установлен");
        }
        Err(e) => tracing::warn!(path = %path.display(), error = %e, "шрифт не найден"),
    }
}

/// Игровые системы работают только вне лобби.
fn in_game(state: Res<lobby::LobbyState>) -> bool {
    *state == lobby::LobbyState::Playing
}

/// Вход в игру: после кнопки «Играть» создаём сетевой линк и спрайт игрока.
fn enter_game(
    mut commands: Commands,
    state: Res<lobby::LobbyState>,
    registry: Res<rsi::RsiRegistry>,
    settings: Res<settings::Settings>,
    mut entered: Local<bool>,
) {
    if *state != lobby::LobbyState::Playing || *entered {
        return;
    }
    *entered = true;
    // Сетевой линк клиента; идентификация по адресу (netcode — позже, T5.x).
    commands
        .spawn((
            Client,
            LocalAddr(CLIENT_ADDR),
            PeerAddr(server_addr(&settings)),
            Link::default(),
            RawClient,
            UdpIo::default(),
            // принимает реплицируемые компоненты сервера (T1.3)
            ReplicationReceiver,
        ))
        .trigger(Connect::from);
    spawn_player_sprite(&mut commands);
    // Лом из SS14 рядом со стартовой точкой (демо IMP.1).
    let key = "sprites/ss14/Objects/Tools/crowbar.rsi#icon";
    rsi::spawn_rsi_sprite(
        &mut commands,
        &registry,
        key,
        0,
        Vec3::new(220.0, 120.0, 0.0),
    );
}

/// Состояние рукопожатия.
#[derive(Resource, Default)]
struct Handshake {
    connect_sent: bool,
}

/// Серверная сущность игрока из Welcome (ADR-7: клиент хранит маппинг серверных ID).
#[derive(Resource, Default)]
struct PlayerEntity(Option<u64>);

/// Шлёт Connect, как только линк подключился.
///
/// MessageSender авто-добавляется required-компонентом на линк клиента.
fn send_connect(
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    settings: Res<settings::Settings>,
    mut handshake: ResMut<Handshake>,
) {
    if handshake.connect_sent {
        return;
    }
    for mut sender in senders.iter_mut() {
        // Имя: SSR_BOT/SSR_NAME перекрывают настройки (боты и тесты);
        // пустое — техническое.
        let name = std::env::var("SSR_BOT")
            .or_else(|_| std::env::var("SSR_NAME"))
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                if settings.player_name.trim().is_empty() {
                    DEV_PLAYER_NAME.to_string()
                } else {
                    settings.player_name.trim().to_string()
                }
            });
        sender.send::<GameChannel>(ClientMessage::Connect {
            protocol_version: PROTOCOL_VERSION,
            name: name.clone(),
        });
        handshake.connect_sent = true;
        tracing::info!(name = %name, "Connect sent");
    }
}

/// Направление ввода WASD/стрелок.
fn input_direction(input: &ButtonInput<KeyCode>) -> Vec2 {
    let mut direction = Vec2::ZERO;
    if input.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        direction.y += 1.0;
    }
    if input.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        direction.y -= 1.0;
    }
    if input.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        direction.x -= 1.0;
    }
    if input.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        direction.x += 1.0;
    }
    direction.normalize_or_zero()
}

/// Шлёт серверу текущее направление ввода (сервер сам двигает игрока, ADR-3).
///
/// Состояние шлётся каждый кадр (а не только при изменении): первый пакет может
/// прийти до создания сущности игрока на сервере — актуальный ввод самолечится.
/// Сжатие до тик-рейта сети сделаем в T1.4.
///
/// SSR_AUTO_WALK=1 — тестовый режим: после подключения клиент «держит вправо»
/// SSR_AUTO_WALK_MS миллисекунд (по умолчанию 3000; короткое значение оставляет
/// игрока у двери для тестов взаимодействия). Пригодится ботам в T6.1.
#[allow(clippy::too_many_arguments)]
fn send_input(
    input: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    player_entity: Res<PlayerEntity>,
    console: Res<console::Console>,
    chat: Res<chat::ChatState>,
    hud_state: Res<hud::HudState>,
    mut connected_elapsed: Local<f32>,
    mut state: Local<InputSendState>,
    connected: Query<(), With<Connected>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    // При открытой консоли или наборе текста в чате персонаж не двигается.
    let console_open = console.open || chat.focused;
    if !input_ready(&player_entity) {
        return;
    }
    if connected.iter().next().is_some() {
        *connected_elapsed += time.delta_secs();
    }
    let walk_secs = std::env::var("SSR_AUTO_WALK_MS")
        .ok()
        .and_then(|v| v.parse::<f32>().ok())
        .map(|ms| ms / 1000.0)
        .unwrap_or(3.0);
    // SSR_LOCK_INPUT=1 — тестовый режим: клавиатура игнорируется
    // (ввод только автоходом), тесты детерминированы даже при чужом вводе.
    let locked = std::env::var_os("SSR_LOCK_INPUT").is_some();
    // SSR_AUTO_WALK_DIR="0,1" — направление автохода (по умолчанию вправо).
    let walk_dir = std::env::var("SSR_AUTO_WALK_DIR")
        .ok()
        .and_then(|v| {
            let (x, y) = v.split_once(',')?;
            Some(Vec2::new(x.trim().parse().ok()?, y.trim().parse().ok()?))
        })
        .unwrap_or(Vec2::X);
    // SSR_BUILD_TEST: с 9-й по 12-ю секунду клиент идёт на восток — в новую стену.
    let build_test_walk =
        std::env::var_os("SSR_BUILD_TEST").is_some() && (9.0..12.0).contains(&*connected_elapsed);
    let direction = if std::env::var_os("SSR_AUTO_WALK").is_some() && *connected_elapsed < walk_secs
    {
        walk_dir
    } else if build_test_walk {
        Vec2::X
    } else if locked || console_open {
        Vec2::ZERO
    } else {
        input_direction(&input)
    };
    // Ввод шлём с частотой тика сети (T6.1): каждый кадр — это до тысяч
    // сообщений в секунду и лишний трафик. Смена направления уходит сразу.
    let movement = direction.to_array();
    state.elapsed += time.delta_secs();
    let changed = state.last != movement;
    if !changed && state.elapsed < NET_TICK_SECS {
        return;
    }
    state.elapsed = 0.0;
    state.last = movement;
    // Бег (Shift) и боевой режим (F) — в том же сообщении ввода.
    // В SS14 спринт по умолчанию, Shift включает ХОДЬБУ (DefaultSprinting).
    let running = input.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight])
        || std::env::var_os("SSR_RUN_TEST").is_some();
    let combat = hud_state.combat;
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::Input {
            movement,
            running,
            combat,
        });
    }
}

/// Горячие клавиши: X — сменить руку (как в SS14).
fn hotkeys(
    input: Res<ButtonInput<KeyCode>>,
    console: Res<console::Console>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if console.open || !input.just_pressed(KeyCode::KeyX) {
        return;
    }
    for mut sender in senders.iter_mut() {
        sender.send::<GameChannel>(ClientMessage::SwitchHand);
    }
}

/// Состояние отправки ввода: таймер до следующего пакета и последнее направление.
#[derive(Default)]
struct InputSendState {
    elapsed: f32,
    last: [f32; 2],
}

/// Готов ли клиент слать ввод: сервер создал игрока (Welcome получен).
/// До этого пакеты ввода бессмысленны и спамят лог сервера.
fn input_ready(player_entity: &PlayerEntity) -> bool {
    player_entity.0.is_some()
}

/// Принимает Welcome (и будущие серверные сообщения).
///
/// MessageReceiver появляется лениво, при первом входящем сообщении.
fn receive_server(
    mut receivers: Query<&mut MessageReceiver<ServerMessage>, With<Connected>>,
    mut player_entity: ResMut<PlayerEntity>,
    mut menu: ResMut<inventory_ui::ActionMenu>,
    mut denied: ResMut<doors::DeniedDoors>,
    mut sounds: ResMut<audio::SoundRequests>,
    mut console: ResMut<console::Console>,
    mut chat: ResMut<chat::ChatState>,
) {
    for mut receiver in receivers.iter_mut() {
        for message in receiver.receive() {
            match message {
                // Отказ доступа: сервер присылает bits двери — мигаем красной лампой.
                ServerMessage::Event { kind } => {
                    match kind
                        .strip_prefix("door_denied:")
                        .and_then(|value| value.parse::<u64>().ok())
                    {
                        Some(bits) => {
                            denied.0.push(bits);
                            tracing::info!(bits, "door denied event");
                        }
                        None if kind == "hit" => sounds.punch += 1,
                        None if let Some(text) = kind.strip_prefix("admin:") => {
                            // Ответ админ-команды (T5.5) — в консоль и в чат.
                            console.push_line(format!("[сервер] {text}"));
                            chat.system(text);
                            tracing::info!(reply = %text, "admin reply");
                        }
                        None => tracing::info!(kind, "server event"),
                    }
                }
                ServerMessage::Chat {
                    channel,
                    from,
                    text,
                } => {
                    chat.push(channel, from.clone(), text.clone());
                    tracing::info!(?channel, %from, %text, "chat received");
                }
                ServerMessage::Welcome {
                    player_entity: entity,
                    protocol_version,
                } => {
                    if !is_compatible(protocol_version) {
                        tracing::error!(
                            server_version = protocol_version,
                            "server version mismatch"
                        );
                        continue;
                    }
                    player_entity.0 = Some(entity);
                    tracing::info!(player_entity = entity, "Welcome accepted");
                }
                ServerMessage::WorldState { .. } => {} // полный снимок — T1.4
                ServerMessage::Actions { options } => {
                    // ВАЖНО: список действий обрабатывается ЗДЕСЬ же, потому что
                    // MessageReceiver осушается одним receive() — вторая система
                    // отбирала бы у этой сообщения (включая Welcome).
                    tracing::info!(count = options.len(), "actions received");
                    menu.options = options;
                }
                ServerMessage::EntityDelta { .. } => {} // дельты позиций — T1.3/T1.4
            }
        }
    }
}

/// Клиентская сущность своего игрока: bits из Welcome → ServerEntityMap → клиент.
pub(crate) fn own_player_entity(
    player_entity: &PlayerEntity,
    entity_map: &Option<Res<ServerEntityMap>>,
) -> Option<Entity> {
    let server = Entity::try_from_bits(player_entity.0?)?;
    entity_map.as_deref()?.to_client().get(&server).copied()
}

/// Интерполяция собственной позиции между серверными снимками (T3.x):
/// держим предыдущую и целевую точки и проигрываем путь ровно за тик сети —
/// движение равномерное, без экспоненциального «догоняния» и дробления.
#[derive(Default)]
struct PositionInterp {
    prev: Vec2,
    target: Vec2,
    elapsed: f32,
    started: bool,
}

/// Двигает спрайт к реплицированной позиции СВОЕГО игрока (не «единственного»:
/// рядом бывают другие игроки) и выбирает сторону из 4 по движению.
fn apply_player_state(
    time: Res<Time>,
    player_entity: Res<PlayerEntity>,
    entity_map: Option<Res<ServerEntityMap>>,
    positions: Query<&PlayerPosition>,
    mut interp: Local<PositionInterp>,
    mut sprites: Query<(&mut Transform, &mut humanoid::Facing), With<Player>>,
) {
    let Some(own) = own_player_entity(&player_entity, &entity_map) else {
        return;
    };
    let Ok(target) = positions.get(own) else {
        return;
    };
    let target = Vec2::from_array(target.0);

    if !interp.started {
        interp.prev = target;
        interp.target = target;
        interp.started = true;
    } else if target != interp.target {
        // Новая серверная позиция: продолжаем путь от текущей отрисованной точки
        // и стартуем с задержкой в один тик — путь всегда проигрывается целиком
        // и с постоянной скоростью (рывки шли от раннего/позднего прихода пакетов).
        interp.prev = interp.prev.lerp(
            interp.target,
            (interp.elapsed / NET_TICK_SECS).clamp(0.0, 1.0),
        );
        interp.target = target;
        interp.elapsed = 0.0;
    }
    interp.elapsed += time.delta_secs();
    let alpha = (interp.elapsed / NET_TICK_SECS).clamp(0.0, 1.0);
    let current = interp.prev.lerp(interp.target, alpha);

    // Направление — по серверному смещению за последний снимок.
    // Порядок RsiDirection движка: South=0, North=1, East=2, West=3.
    let delta = interp.target - interp.prev;
    let direction = (delta.length() >= 0.5).then(|| {
        if delta.x.abs() > delta.y.abs() {
            if delta.x > 0.0 { 2 } else { 3 } // восток / запад
        } else if delta.y > 0.0 {
            1 // север
        } else {
            0 // юг
        }
    });

    for (mut transform, mut facing) in sprites.iter_mut() {
        transform.translation.x = current.x;
        transform.translation.y = current.y;
        if let Some(direction) = direction {
            facing.0 = direction;
        }
    }
    tracing::debug!(target = ?target.to_array(), render = ?current.to_array(), "position applied");
}

/// Счётчик и позиции видимых игроков (T1.4/T2.4): лог раз в 2 секунды.
/// Позиции меняются на глазах — видно, что реплицируется движение.
fn count_replicated(
    time: Res<Time>,
    mut next_log: Local<f32>,
    replicated: Query<&PlayerPosition, With<Remote>>,
) {
    *next_log += time.delta_secs();
    if *next_log < 2.0 {
        return;
    }
    *next_log = 0.0;
    let mut positions: Vec<[f32; 2]> = replicated.iter().map(|p| p.0).collect();
    if positions.is_empty() {
        return;
    }
    positions.sort_by(|a, b| a[0].total_cmp(&b[0]).then(a[1].total_cmp(&b[1])));
    tracing::info!(count = positions.len(), positions = ?positions, "visible players");
}

fn setup_camera(mut commands: Commands) {
    // Поля кадра мира (letterbox) — тот же тёмный цвет, что и у камеры:
    // дефолтный clear-color Bevy (#2B2C2F) давал серую рамку по краям экрана.
    commands.insert_resource(ClearColor(Color::srgb_u8(16, 18, 24)));
    commands.spawn((
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb_u8(16, 18, 24)),
            ..default()
        },
    ));
}

/// Сущность-визуал своего игрока: тело (гуманоид) собирается отдельно
/// (`humanoid::sync_bodies`), здесь только трансформ и направление.
fn spawn_player_sprite(commands: &mut Commands) {
    // z = 1: игрок рисуется поверх тайлов карты.
    // Visibility обязателен родителю: без него части тела-дети не получают
    // видимость (B0004) и рисуются рвано.
    commands.spawn((
        Player,
        humanoid::Facing(0), // старт: смотрит на юг
        Visibility::default(),
        Transform::from_xyz(0.0, 0.0, 1.0),
    ));
}

/// Визуалы игрока (для камеры).
type PlayerTransforms<'w, 's> =
    Query<'w, 's, (Entity, &'static Transform), (With<Player>, Without<Camera2d>)>;

/// Камера жёстко следует за отрисованной позицией игрока: никакого второго
/// сглаживания — иначе мир «дрожит» относительно персонажа.
fn camera_follow_player(
    player: PlayerTransforms,
    mut camera: Single<&mut Transform, With<Camera2d>>,
    mut state: Local<(f32, f32)>,
) {
    let count = player.iter().count();
    let Ok((_, target)) = player.single() else {
        tracing::warn!(count, "camera follow: Player entity is not single");
        return;
    };
    camera.translation.x = target.translation.x;
    camera.translation.y = target.translation.y;
    if (state.0, state.1) != (camera.translation.x, camera.translation.y) {
        *state = (camera.translation.x, camera.translation.y);
        tracing::debug!(camera = ?state, "camera moved");
    }
}

/// Грузит прототипы/спрайты тайлов; сами чанки приходят с сервера (T2.3).
fn startup_map(mut commands: Commands) {
    let root = Path::new(&assets_file_path()).to_path_buf();
    let visuals = tiles::load_tile_visuals(&root);
    commands.insert_resource(visuals);
}

#[derive(Component)]
struct Player;

/// Плагин ассетов с явным путём к каталогу `assets/` в корне репозитория.
///
/// Bevy по умолчанию ищет ассеты относительно BEVY_ASSET_ROOT / CARGO_MANIFEST_DIR
/// (при `cargo run` это `crates/client` — мимо корня репо) или каталога exe,
/// поэтому путь задаём явно. Если BEVY_ASSET_ROOT выставлен вручную — не мешаем.
fn asset_plugin() -> AssetPlugin {
    if std::env::var_os("BEVY_ASSET_ROOT").is_some() {
        return AssetPlugin::default();
    }
    AssetPlugin {
        file_path: assets_file_path(),
        ..default()
    }
}

/// Абсолютный путь к `<repo>/assets/`, вычисленный из окружения запуска.
fn assets_file_path() -> String {
    // `cargo run`: CARGO_MANIFEST_DIR = <repo>/crates/client → корень через два уровня.
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR")
        && let Some(root) = Path::new(&manifest_dir).ancestors().nth(2)
    {
        return root.join("assets").to_string_lossy().into_owned();
    }
    // Прямой запуск: <repo>/target/<profile>/ssr-client.exe → корень через три уровня.
    if let Ok(exe_path) = std::env::current_exe()
        && let Some(root) = exe_path.ancestors().nth(3)
    {
        return root.join("assets").to_string_lossy().into_owned();
    }
    "assets".into()
}

/// Регистрация реплицируемых компонентов — общий список (ssr_protocol::net).
/// Так порядок не может разойтись с сервером: раньше он дублировался вручную,
/// из-за чего мы трижды ловили «Hit the end of buffer». Используется и клиентом,
/// и headless-ботом (T6.1).
pub fn register_replication(app: &mut App) {
    ssr_protocol::net::register_replication(app);
}

/// Тест-режим SSR_SCREENSHOT=<файл>: клиент сам сохраняет кадр окна (T6.3+).
/// Внешние снимки окна (PrintWindow/CopyFromScreen) отдают устаревший кадр,
/// если окно перекрыто — берём картинку прямо из рендера.
fn screenshot_test_mode(mut commands: Commands, time: Res<Time>, mut state: Local<(f32, bool)>) {
    let Ok(path) = std::env::var("SSR_SCREENSHOT") else {
        return;
    };
    if state.1 {
        return;
    }
    state.0 += time.delta_secs();
    let delay: f32 = std::env::var("SSR_SCREENSHOT_DELAY")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(6.0);
    if state.0 < delay {
        return;
    }
    state.1 = true;
    // Несколько кадров подряд: UI успевает отрисоваться к последнему.
    commands
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(bevy::render::view::screenshot::save_to_disk(path.clone()));
    tracing::info!(path, "screenshot requested");
}

/// Ограничивает видимую область мира как `ScalingViewport` в SS14: на большом
/// окне масштаб увеличивается, чтобы в кадр попадало не больше ~1920×1080
/// единиц (60×34 тайла), иначе на широком мониторе видно лишнее.
/// Видимая область мира как в SS14: фиксированный виртуальный кадр
/// **21×15 тайлов** (`ViewportUIController.ViewportSize = 672×480 px` при
/// 32 px/тайл). Окно получает кадр letterbox'ом по центру с целочисленным
/// масштабом (`ScalingViewport` + snap из `MainViewport.CalcSnappingFactor`).
pub fn fit_world_viewport(
    windows: Query<&Window>,
    mut camera: Single<(&mut Camera, &mut Projection), With<Camera2d>>,
) {
    // Высота кадра — 15 тайлов (480 единиц мира), как `ViewportUIController`
    // в движке. Поля (letterbox) не оставляем: владелец просил, чтобы мир
    // занимал экран целиком и никаких рамок не было ни на одном разрешении —
    // по ширине видно больше тайлов при широком окне, это ожидаемо.
    const VIRTUAL_H: f32 = 480.0; // 15 тайлов × 32
    let Ok(window) = windows.single() else {
        return;
    };
    let (camera, projection) = &mut *camera;
    let logical_h = window.height().max(1.0);
    // Вьюпорт — всё окно: камера рисует в него без отступов.
    if camera.viewport.is_some() {
        camera.viewport = None;
        tracing::info!("world viewport: рамки убраны, мир на весь экран");
    }
    let Projection::Orthographic(orthographic) = &mut **projection else {
        return;
    };
    // Масштаб такой, что по высоте видно ровно 480 единиц мира (15 тайлов):
    // видимая высота = высота окна × scale, значит scale = 480 / высота_окна
    // (в Bevy ортографический scale — это «отдаление», а не приближение).
    let scale = VIRTUAL_H / logical_h;
    if (orthographic.scale - scale).abs() > 1e-4 {
        orthographic.scale = scale;
        tracing::info!(
            scale,
            "world viewport updated (15 tiles tall, no letterbox)"
        );
    }
}
