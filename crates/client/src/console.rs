//! Консоль клиента: открывается по ` (Backquote), набор текста и локальные
//! команды. Серверных прав у неё нет — это отладочный инструмент игрока
//! (админ-команды сервера — задача T5.5).

use bevy::ecs::system::SystemParam;
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::prelude::*;
use lightyear::prelude::client::*;
use lightyear::prelude::*;
use ssr_core::PlayerPosition;
use ssr_protocol::ClientMessage;
use ssr_protocol::net::GameChannel;

use crate::inventory_ui::{OwnPlayerEntity, RemotePlayerVisual};
use crate::settings::Settings;

/// Состояние консоли.
#[derive(Resource, Default)]
pub struct Console {
    pub open: bool,
    pub input: String,
    lines: Vec<String>,
}

/// Сколько строк истории храним.
const MAX_LINES: usize = 64;

impl Console {
    /// Добавляет строку в историю (для ответов сервера, T5.5).
    pub fn push_line(&mut self, line: impl Into<String>) {
        self.push(line);
    }

    fn push(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
        if self.lines.len() > MAX_LINES {
            self.lines.remove(0);
        }
    }
}

/// Корень окна консоли.
#[derive(Component)]
pub struct ConsoleRoot;

/// Текст истории.
#[derive(Component)]
pub struct ConsoleLog;

/// Текст строки ввода.
#[derive(Component)]
pub struct ConsoleInputText;

/// Открывает и закрывает консоль (клавиша `).
pub fn toggle_console(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut console: ResMut<Console>,
    root: Query<Entity, With<ConsoleRoot>>,
) {
    if !keys.just_pressed(KeyCode::Backquote) {
        return;
    }
    console.open = !console.open;
    for entity in root.iter() {
        commands.entity(entity).despawn();
    }
    if console.open {
        if console.lines.is_empty() {
            console.push("Space Station R — консоль. `help` — список команд.");
        }
        spawn_console(&mut commands);
    }
}

/// Строит окно консоли в верхней части экрана.
fn spawn_console(commands: &mut Commands) {
    commands
        .spawn((
            ConsoleRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                top: px(0),
                height: Val::Percent(42.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(10)),
                row_gap: px(6),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.03, 0.05, 0.92)),
        ))
        .with_children(|panel| {
            panel.spawn((
                ConsoleLog,
                Text::new(""),
                TextFont::from_font_size(13.0),
                TextColor(Color::srgb(0.80, 0.86, 0.80)),
                Node {
                    flex_grow: 1.0,
                    max_height: Val::Percent(100.0),
                    ..default()
                },
            ));
            panel.spawn((
                ConsoleInputText,
                Text::new("> _"),
                TextFont::from_font_size(14.0),
                TextColor(Color::srgb(1.0, 0.85, 0.45)),
            ));
        });
}

/// Данные, нужные командам консоли.
#[derive(SystemParam)]
pub struct ConsoleCommands<'w, 's> {
    settings: ResMut<'w, Settings>,
    own: Res<'w, OwnPlayerEntity>,
    positions: Query<'w, 's, &'static PlayerPosition>,
    remotes: Query<'w, 's, (), With<RemotePlayerVisual>>,
    senders: Query<'w, 's, &'static mut MessageSender<ClientMessage>, With<Connected>>,
    exit: MessageWriter<'w, AppExit>,
}

/// Набор текста и выполнение команд по Enter.
pub fn console_input(
    mut events: MessageReader<KeyboardInput>,
    mut commands: Commands,
    mut console: ResMut<Console>,
    mut cmd: ConsoleCommands,
    root: Query<Entity, With<ConsoleRoot>>,
) {
    if !console.open {
        return;
    }
    let mut submitted = false;
    for event in events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        match &event.logical_key {
            Key::Enter => submitted = true,
            Key::Escape => {
                console.open = false;
                for entity in root.iter() {
                    commands.entity(entity).despawn();
                }
                return;
            }
            Key::Backspace => {
                console.input.pop();
            }
            Key::Space => console.input.push(' '),
            Key::Character(text) => console.input.push_str(text),
            _ => {}
        }
    }
    if !submitted {
        return;
    }
    let command = std::mem::take(&mut console.input);
    let command = command.trim().to_string();
    if command.is_empty() {
        return;
    }
    console.push(format!("> {command}"));
    run_command(&command, &mut console, &mut cmd);
}

/// Локальные команды консоли.
fn run_command(command: &str, console: &mut Console, cmd: &mut ConsoleCommands) {
    let mut parts = command.split_whitespace();
    let name = parts.next().unwrap_or_default();
    let argument = parts.next().unwrap_or_default();
    // Админ-команды (T5.5): исполняет сервер, ответ придёт событием.
    if matches!(name, "tp" | "spawn" | "kick" | "heal") {
        for mut sender in cmd.senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Admin {
                command: command.to_string(),
            });
        }
        console.push_line("отправлено на сервер…");
        return;
    }
    // Внешность: hair/beard/haircolor/sex — отправляем серверу (протокол v10).
    if matches!(name, "hair" | "beard" | "haircolor" | "sex") {
        let mut message = ClientMessage::SetAppearance {
            sex: None,
            hair: None,
            beard: None,
            hair_color: None,
        };
        if let ClientMessage::SetAppearance {
            sex,
            hair,
            beard,
            hair_color,
        } = &mut message
        {
            match name {
                "hair" => *hair = Some(argument.to_string()),
                "beard" => *beard = Some(argument.to_string()),
                "sex" => *sex = Some(argument.to_string()),
                "haircolor" => {
                    let parts: Vec<u8> = command
                        .split_whitespace()
                        .skip(1)
                        .filter_map(|value| value.parse().ok())
                        .collect();
                    if parts.len() == 3 {
                        *hair_color = Some([parts[0], parts[1], parts[2]]);
                    }
                }
                _ => {}
            }
        }
        for mut sender in cmd.senders.iter_mut() {
            sender.send::<GameChannel>(message.clone());
        }
        console.push_line(format!("внешность: {command}"));
        return;
    }
    match name {
        "help" => {
            console.push("Команды: help, clear, volume <0..1>, fps <on|off>,");
            console.push("  fullscreen <on|off>, pos, players, quit");
            console.push("Админ (сервер): tp <x> <y>, spawn <предмет> [n], kick <имя>, heal");
            console.push("Внешность: hair <стиль>, beard <стиль|пусто>, sex <male|female>,");
            console.push("  haircolor <r> <g> <b>");
        }
        "clear" => console.lines.clear(),
        // `showghosts <on|off>` — показ призраков живым (в сборке команда
        // `ShowGhostsCommand`, `Content.Server/Administration/Commands/ShowGhostsCommand.cs`).
        "showghosts" => {
            let value = matches!(argument, "on" | "true" | "1");
            cmd.settings.show_ghosts = value;
            cmd.settings.save();
            console.push(format!(
                "показ призраков: {}",
                if value {
                    "включён"
                } else {
                    "выключен"
                }
            ));
        }
        "volume" => match argument.parse::<f32>() {
            Ok(value) => {
                cmd.settings.volume = value.clamp(0.0, 1.0);
                cmd.settings.save();
                console.push(format!("громкость: {:.0}%", cmd.settings.volume * 100.0));
            }
            Err(_) => console.push("использование: volume <0..1>"),
        },
        "fps" => match argument {
            "on" => {
                cmd.settings.show_fps = true;
                cmd.settings.save();
                console.push("FPS: вкл");
            }
            "off" => {
                cmd.settings.show_fps = false;
                cmd.settings.save();
                console.push("FPS: выкл");
            }
            _ => console.push("использование: fps <on|off>"),
        },
        "fullscreen" => match argument {
            "on" => {
                cmd.settings.fullscreen = true;
                cmd.settings.save();
                console.push("полный экран: вкл (применится при следующем запуске)");
            }
            "off" => {
                cmd.settings.fullscreen = false;
                cmd.settings.save();
                console.push("полный экран: выкл");
            }
            _ => console.push("использование: fullscreen <on|off>"),
        },
        "pos" => match cmd.own.0.and_then(|entity| cmd.positions.get(entity).ok()) {
            Some(position) => console.push(format!(
                "позиция: {:.1}, {:.1}",
                position.0[0], position.0[1]
            )),
            None => console.push("свой игрок ещё не появился"),
        },
        "players" => console.push(format!("видимых игроков: {}", cmd.remotes.iter().count())),
        "quit" => {
            console.push("выход…");
            cmd.exit.write(AppExit::Success);
        }
        other => console.push(format!("неизвестная команда: {other} (help)")),
    }
}

/// Печатает историю и строку ввода (перерисовка только при изменениях).
pub fn update_console_text(
    console: Res<Console>,
    mut last: Local<(String, String)>,
    mut log: Query<&mut Text, (With<ConsoleLog>, Without<ConsoleInputText>)>,
    mut input: Query<&mut Text, (With<ConsoleInputText>, Without<ConsoleLog>)>,
) {
    if !console.open {
        return;
    }
    let log_text = console.lines.join("\n");
    if last.0 != log_text {
        if let Ok(mut text) = log.single_mut() {
            text.0 = log_text.clone();
        }
        last.0 = log_text;
    }
    let input_text = format!("> {}_", console.input);
    if last.1 != input_text {
        if let Ok(mut text) = input.single_mut() {
            text.0 = input_text.clone();
        }
        last.1 = input_text;
    }
}

/// Тест T5.5: SSR_ADMIN_TEST=1 — через 4 с `tp`, через 6 с `spawn`, через 8 с `heal`.
pub fn admin_test_mode(
    time: Res<Time>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    mut state: Local<(f32, u8)>,
) {
    if std::env::var_os("SSR_ADMIN_TEST").is_none() {
        return;
    }
    state.0 += time.delta_secs();
    let mut send = |command: &str| {
        for mut sender in senders.iter_mut() {
            sender.send::<GameChannel>(ClientMessage::Admin {
                command: command.to_string(),
            });
        }
        tracing::info!(command, "admin-test: sent");
    };
    if state.1 == 0 && state.0 >= 4.0 {
        state.1 = 1;
        send("tp 1500 1080");
    } else if state.1 == 1 && state.0 >= 6.0 {
        state.1 = 2;
        send("spawn Medkit 2");
    } else if state.1 == 2 && state.0 >= 8.0 {
        state.1 = 3;
        send("heal");
    } else if state.1 == 3
        && state.0 >= 10.0
        && let Ok(target) = std::env::var("SSR_KICK_TEST")
    {
        state.1 = 4;
        send(&format!("kick {target}"));
    }
}
