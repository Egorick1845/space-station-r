//! Лаунчер — экран до подключения, по образцу лаунчера SS14
//! (`SS14.Launcher`: `MainWindow`, вкладки `HomePageView`, `ServerListTabView`,
//! `NewsTabView`, `OptionsTabView`). Шапка с логотипом и кнопками ссылок,
//! золотая черта, вкладки снизу («Главная», «Серверы», «Новости»,
//! «Настройки») и версия справа. Логотип — свой (Space Station R), не SS14.
//!
//! Статус серверов (время раунда, игроки) берётся из `GET /status`
//! (см. [`ssr_protocol::status`]) в фоновых потоках: окно не подвисает, пока
//! ждём недоступный сервер.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use bevy::input::mouse::MouseWheel;
use bevy::prelude::*;
use bevy::window::{MonitorSelection, WindowMode};
use ssr_protocol::status::{ServerStatus, format_round_time, http_body, status_request};

use crate::lobby::LobbyState;

// Цвета лаунчера SS14 (тёмная тема `LauncherStyle`).
const BG: Color = Color::srgb(0.145, 0.145, 0.165);
const HEADER_BG: Color = Color::srgb(0.11, 0.11, 0.125);
const GOLD: Color = Color::srgb(0.66, 0.58, 0.33);
const TITLE: Color = Color::srgb(0.78, 0.70, 0.40);
const ROW_A: Color = Color::srgb(0.17, 0.17, 0.19);
const ROW_B: Color = Color::srgb(0.135, 0.135, 0.15);
const SEPARATOR: Color = Color::srgb(0.25, 0.25, 0.28);
const BUTTON: Color = Color::srgb(0.27, 0.29, 0.39);
const BUTTON_HOVER: Color = Color::srgb(0.35, 0.38, 0.50);
const BUTTON_DISABLED: Color = Color::srgb(0.20, 0.21, 0.26);
const TAB_BAR: Color = Color::srgb(0.10, 0.10, 0.11);
const TAB_ACTIVE: Color = Color::srgb(0.24, 0.43, 0.28);
const TAB_HOVER: Color = Color::srgb(0.18, 0.20, 0.22);
const FIELD: Color = Color::srgb(0.10, 0.10, 0.115);
const TEXT: Color = Color::srgb(0.90, 0.90, 0.92);
const TEXT_DIM: Color = Color::srgb(0.60, 0.60, 0.64);
const TEXT_DISABLED: Color = Color::srgb(0.45, 0.45, 0.50);
const LOGO_RED: Color = Color::srgb(0.84, 0.19, 0.17);

/// Строк в видимой части списка (дальше — прокрутка колесом).
const VISIBLE_ROWS: usize = 11;

/// Вкладка лаунчера.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tab {
    #[default]
    Home,
    Servers,
    News,
    Settings,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Home, Tab::Servers, Tab::News, Tab::Settings];

    fn title(self) -> &'static str {
        match self {
            Tab::Home => "Главная",
            Tab::Servers => "Серверы",
            Tab::News => "Новости",
            Tab::Settings => "Настройки",
        }
    }
}

/// Поле ввода лаунчера.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Field {
    Search,
    Direct,
    Add,
    Name,
}

/// Состояние опроса сервера.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Pending,
    Online(ServerStatus),
    Offline,
}

/// Переключатель настройки.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Toggle {
    Fullscreen,
    ShowFps,
    FootWalk,
}

/// Действие кнопки лаунчера.
#[derive(Component, Clone, PartialEq, Debug)]
pub enum Action {
    Tab(Tab),
    Refresh,
    Connect(String),
    Expand(String),
    Remove(String),
    OpenAdd,
    ConfirmAdd,
    OpenDirect,
    ConfirmDirect,
    Cancel,
    News(usize),
    Url(&'static str),
    Focus(Field),
    Toggle(Toggle),
}

/// Корень UI лаунчера.
#[derive(Component)]
pub struct LauncherRoot;

/// Цвета кнопки (обычный / под курсором).
#[derive(Component, Clone, Copy)]
pub struct Hover {
    base: Color,
    hover: Color,
}

/// Новость: заголовок и текст (из `PROGRESS.md` — дневник разработки).
#[derive(Clone, Debug, PartialEq)]
pub struct NewsEntry {
    pub title: String,
    pub body: String,
}

/// Состояние лаунчера.
#[derive(Resource)]
pub struct Launcher {
    pub tab: Tab,
    pub statuses: HashMap<String, Status>,
    tx: Sender<(String, Option<ServerStatus>)>,
    rx: Mutex<Receiver<(String, Option<ServerStatus>)>>,
    pub focused: Option<Field>,
    pub search: String,
    pub direct: String,
    pub add: String,
    pub adding: bool,
    pub direct_open: bool,
    pub expanded: Option<String>,
    pub news: Vec<NewsEntry>,
    pub news_open: Option<usize>,
    pub scroll: usize,
}

impl Default for Launcher {
    fn default() -> Self {
        let (tx, rx) = channel();
        Self {
            tab: Tab::Home,
            statuses: HashMap::new(),
            tx,
            rx: Mutex::new(rx),
            focused: None,
            search: String::new(),
            direct: String::new(),
            add: String::new(),
            adding: false,
            direct_open: false,
            expanded: None,
            news: load_news(),
            news_open: None,
            scroll: 0,
        }
    }
}

/// Новости — разделы `## …` из `PROGRESS.md` (свежие сверху).
fn load_news() -> Vec<NewsEntry> {
    let path = std::path::Path::new(&crate::assets_file_path())
        .parent()
        .map(|root| root.join("PROGRESS.md"));
    let Some(text) = path.and_then(|p| std::fs::read_to_string(p).ok()) else {
        return Vec::new();
    };
    parse_news(&text)
}

/// Разбор дневника: каждый заголовок второго уровня — новость.
pub fn parse_news(text: &str) -> Vec<NewsEntry> {
    let mut entries: Vec<NewsEntry> = Vec::new();
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("## ") {
            entries.push(NewsEntry {
                title: title.trim().to_string(),
                body: String::new(),
            });
        } else if let Some(entry) = entries.last_mut() {
            entry.body.push_str(line);
            entry.body.push('\n');
        }
    }
    for entry in &mut entries {
        entry.body = plain_markdown(entry.body.trim());
        entry.title = plain_markdown(&entry.title);
    }
    entries.reverse();
    entries
}

/// Markdown дневника → простой текст: без `**`, обратных кавычек и `#`.
pub fn plain_markdown(text: &str) -> String {
    text.lines()
        .map(|line| {
            let line = line.trim_start_matches('#').trim_start();
            line.replace("**", "").replace('`', "")
        })
        .collect::<Vec<_>>()
        .join("
")
}

/// Адрес по умолчанию (`SSR_PORT` или 7777).
fn local_address() -> String {
    let port = std::env::var("SSR_PORT").unwrap_or_else(|_| "7777".to_string());
    format!("127.0.0.1:{port}")
}

/// Список избранного: сохранённые серверы, локальный всегда есть.
fn favorites(settings: &crate::settings::Settings) -> Vec<String> {
    let mut list = settings.servers.clone();
    let local = local_address();
    if !list.contains(&local) {
        list.push(local);
    }
    list
}

/// Опрос `GET /status` в фоновом потоке (таймаут 1.5 с).
fn query_status(address: String, tx: Sender<(String, Option<ServerStatus>)>) {
    std::thread::spawn(move || {
        let result = (|| {
            let addr = address.to_socket_addrs().ok()?.next()?;
            let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(1500)).ok()?;
            stream.set_read_timeout(Some(Duration::from_millis(1500))).ok()?;
            stream.write_all(status_request(&address).as_bytes()).ok()?;
            let mut response = String::new();
            stream.read_to_string(&mut response).ok()?;
            serde_json::from_str::<ServerStatus>(http_body(&response)?).ok()
        })();
        let _ = tx.send((address, result));
    });
}

impl Launcher {
    fn refresh(&mut self, addresses: &[String]) {
        for address in addresses {
            self.statuses.insert(address.clone(), Status::Pending);
            query_status(address.clone(), self.tx.clone());
        }
    }
}

/// Старт: показать лаунчер и опросить избранное.
pub fn spawn_launcher(mut launcher: ResMut<Launcher>, settings: Res<crate::settings::Settings>) {
    let list = favorites(&settings);
    launcher.refresh(&list);
    tracing::info!(servers = list.len(), "launcher: опрос серверов");
}

/// Принимает ответы опроса.
pub fn poll_statuses(mut launcher: ResMut<Launcher>) {
    let results: Vec<_> = {
        let Ok(rx) = launcher.bypass_change_detection().rx.lock() else {
            return;
        };
        rx.try_iter().collect()
    };
    for (address, status) in results {
        let status = match status {
            Some(status) => Status::Online(status),
            None => Status::Offline,
        };
        launcher.statuses.insert(address, status);
    }
}

// ---------------------------------------------------------------- отрисовка

fn text(parent: &mut ChildSpawnerCommands, value: impl Into<String>, size: f32, color: Color) {
    parent.spawn((Text::new(value), TextFont::from_font_size(size), TextColor(color)));
}

/// Кнопка лаунчера: синяя плашка (`OpenRight` и т.п. стили SS14).
fn button(
    parent: &mut ChildSpawnerCommands,
    label: &str,
    action: Action,
    enabled: bool,
    node: Node,
) {
    let (base, color) = if enabled {
        (BUTTON, TEXT)
    } else {
        (BUTTON_DISABLED, TEXT_DISABLED)
    };
    let mut entity = parent.spawn((
        Node {
            padding: UiRect::axes(px(12), px(4)),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            border: UiRect::bottom(px(2)),
            ..node
        },
        BackgroundColor(base),
        BorderColor::from(Color::srgb(0.18, 0.19, 0.26)),
    ));
    if enabled {
        entity.insert((
            Button,
            action,
            Hover {
                base,
                hover: BUTTON_HOVER,
            },
        ));
    }
    entity.with_child((Text::new(label), TextFont::from_font_size(15.0), TextColor(color)));
}

/// Поле ввода (клик — фокус, в фокусе — курсор «▏»).
fn field(
    parent: &mut ChildSpawnerCommands,
    value: &str,
    placeholder: &str,
    which: Field,
    focused: bool,
    width: Val,
) {
    let shown = if focused {
        format!("{value}|")
    } else if value.is_empty() {
        placeholder.to_string()
    } else {
        value.to_string()
    };
    let color = if value.is_empty() && !focused {
        TEXT_DIM
    } else {
        TEXT
    };
    parent
        .spawn((
            Button,
            Action::Focus(which),
            Node {
                width,
                min_height: px(30),
                align_items: AlignItems::Center,
                padding: UiRect::horizontal(px(8)),
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(FIELD),
            BorderColor::from(if focused { GOLD } else { SEPARATOR }),
        ))
        .with_child((Text::new(shown), TextFont::from_font_size(15.0), TextColor(color)));
}

/// Строка сервера: ▸ название | время | игроки | «Подключиться».
fn server_row(
    parent: &mut ChildSpawnerCommands,
    launcher: &Launcher,
    address: &str,
    index: usize,
    removable: bool,
) {
    let status = launcher.statuses.get(address).cloned().unwrap_or(Status::Pending);
    let expanded = launcher.expanded.as_deref() == Some(address);
    let (name, time, players, online) = match &status {
        Status::Online(s) => (
            s.name.clone(),
            format_round_time(s.round_time_secs),
            format!("{} / {}", s.players, s.soft_max_players),
            true,
        ),
        Status::Offline => (address.to_string(), String::new(), String::new(), false),
        Status::Pending => (address.to_string(), "…".into(), String::new(), false),
    };
    let compatible = match &status {
        Status::Online(s) => s.protocol_version == ssr_protocol::PROTOCOL_VERSION,
        _ => false,
    };
    let bg = if index.is_multiple_of(2) { ROW_A } else { ROW_B };
    parent
        .spawn((
            Node {
                width: percent(100),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(bg),
        ))
        .with_children(|block| {
            block
                .spawn(Node {
                    width: percent(100),
                    min_height: px(32),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn((
                        Button,
                        Action::Expand(address.to_string()),
                        Hover {
                            base: Color::NONE,
                            hover: TAB_HOVER,
                        },
                        BackgroundColor(Color::NONE),
                        Node {
                            flex_grow: 1.0,
                            flex_basis: px(0),
                            height: px(32),
                            align_items: AlignItems::Center,
                            column_gap: px(10),
                            padding: UiRect::horizontal(px(8)),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                    ))
                    .with_children(|cell| {
                        // Шрифт без «▸/▾»: раскрытие — плюс/минус.
                        text(cell, if expanded { "−" } else { "+" }, 16.0, TEXT_DIM);
                        text(cell, name, 16.0, TEXT);
                    });
                    let column = |width: f32| Node {
                        width: px(width),
                        height: px(32),
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        border: UiRect::left(px(1)),
                        ..default()
                    };
                    if online {
                        row.spawn((column(100.0), BorderColor::from(SEPARATOR)))
                            .with_children(|c| text(c, time, 15.0, TEXT));
                        row.spawn((column(100.0), BorderColor::from(SEPARATOR)))
                            .with_children(|c| text(c, players, 15.0, TEXT));
                    } else {
                        row.spawn((column(200.0), BorderColor::from(SEPARATOR)))
                            .with_children(|c| {
                                let label = if status == Status::Pending {
                                    "ОПРОС…"
                                } else {
                                    "OFFLINE"
                                };
                                text(c, label, 15.0, TEXT_DIM);
                            });
                    }
                    row.spawn((column(130.0), BorderColor::from(SEPARATOR)))
                        .with_children(|c| {
                            button(
                                c,
                                "Подключиться",
                                Action::Connect(address.to_string()),
                                online && compatible,
                                Node::default(),
                            );
                        });
                });
            if expanded {
                block
                    .spawn(Node {
                        flex_direction: FlexDirection::Column,
                        row_gap: px(4),
                        padding: UiRect::new(px(32), px(8), px(4), px(8)),
                        ..default()
                    })
                    .with_children(|info| {
                        text(info, format!("Адрес: {address}"), 14.0, TEXT_DIM);
                        let version = match &status {
                            Status::Online(s) if !compatible => format!(
                                "Версия протокола сервера {}, у клиента {} — подключиться нельзя.",
                                s.protocol_version,
                                ssr_protocol::PROTOCOL_VERSION
                            ),
                            Status::Online(_) => "Совместимая версия.".to_string(),
                            Status::Offline => "Сервер не отвечает на /status.".to_string(),
                            Status::Pending => "Ждём ответа сервера…".to_string(),
                        };
                        text(info, version, 14.0, TEXT_DIM);
                        if removable {
                            button(
                                info,
                                "Удалить из избранного",
                                Action::Remove(address.to_string()),
                                true,
                                Node {
                                    align_self: AlignSelf::FlexStart,
                                    margin: UiRect::top(px(4)),
                                    ..default()
                                },
                            );
                        }
                    });
            }
        });
}

/// Подпись раздела (золотой заголовок).
fn section_title(parent: &mut ChildSpawnerCommands, title: &str) {
    text(parent, title, 22.0, TITLE);
}

/// Шапка: логотип, ссылки, имя игрока.
fn header(root: &mut ChildSpawnerCommands, settings: &crate::settings::Settings) {
    root.spawn((
        Node {
            width: percent(100),
            height: px(72),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            padding: UiRect::horizontal(px(14)),
            border: UiRect::bottom(px(2)),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(HEADER_BG),
        BorderColor::from(GOLD),
    ))
    .with_children(|top| {
        // Логотип: «SPACE STATION» и крупная красная «R».
        top.spawn(Node {
            align_items: AlignItems::FlexEnd,
            column_gap: px(6),
            ..default()
        })
        .with_children(|logo| {
            text(logo, "SPACE STATION", 32.0, TEXT);
            text(logo, "R", 44.0, LOGO_RED);
        });
        top.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        top.spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            align_items: AlignItems::Stretch,
            ..default()
        })
        .with_children(|right| {
            right
                .spawn(Node {
                    column_gap: px(4),
                    ..default()
                })
                .with_children(|links| {
                    button(
                        links,
                        "Правила",
                        Action::Url("https://ministation.ru"),
                        true,
                        Node::default(),
                    );
                    button(
                        links,
                        "Вики",
                        Action::Url("https://wiki.ministation.ru"),
                        true,
                        Node::default(),
                    );
                });
            // Имя игрока — как выбор аккаунта (ведёт в «Настройки»).
            button(
                right,
                &settings.player_name,
                Action::Tab(Tab::Settings),
                true,
                Node::default(),
            );
        });
    });
}

/// Нижняя панель вкладок и версия.
fn tab_bar(root: &mut ChildSpawnerCommands, active: Tab) {
    root.spawn((
        Node {
            width: percent(100),
            height: px(40),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Stretch,
            border: UiRect::top(px(1)),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(TAB_BAR),
        BorderColor::from(SEPARATOR),
    ))
    .with_children(|bar| {
        for tab in Tab::ALL {
            let base = if tab == active { TAB_ACTIVE } else { Color::NONE };
            bar.spawn((
                Button,
                Action::Tab(tab),
                Hover {
                    base,
                    hover: if tab == active { TAB_ACTIVE } else { TAB_HOVER },
                },
                BackgroundColor(base),
                Node {
                    padding: UiRect::horizontal(px(12)),
                    align_items: AlignItems::Center,
                    ..default()
                },
            ))
            .with_child((Text::new(tab.title()), TextFont::from_font_size(17.0), TextColor(TEXT)));
        }
        bar.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        bar.spawn(Node {
            align_items: AlignItems::Center,
            padding: UiRect::horizontal(px(12)),
            ..default()
        })
        .with_children(|v| {
            text(v, format!("v{}", env!("CARGO_PKG_VERSION")), 14.0, TEXT_DIM);
        });
    });
}

fn home(page: &mut ChildSpawnerCommands, launcher: &Launcher, settings: &crate::settings::Settings) {
    let list = favorites(settings);
    page.spawn(Node {
        width: percent(100),
        align_items: AlignItems::Center,
        margin: UiRect::bottom(px(6)),
        ..default()
    })
    .with_children(|row| {
        section_title(row, "Избранные серверы");
        row.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        row.spawn(Node {
            column_gap: px(4),
            ..default()
        })
        .with_children(|b| {
            button(b, "Обновить", Action::Refresh, true, Node::default());
            button(b, "+", Action::OpenAdd, true, Node::default());
        });
    });
    if launcher.adding {
        page.spawn(Node {
            column_gap: px(6),
            align_items: AlignItems::Center,
            margin: UiRect::bottom(px(6)),
            ..default()
        })
        .with_children(|row| {
            text(row, "Адрес:", 15.0, TEXT_DIM);
            field(
                row,
                &launcher.add,
                "хост:порт",
                Field::Add,
                launcher.focused == Some(Field::Add),
                px(260),
            );
            button(row, "Добавить", Action::ConfirmAdd, true, Node::default());
            button(row, "Отмена", Action::Cancel, true, Node::default());
        });
    }
    page.spawn(Node {
        width: percent(100),
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        overflow: Overflow::clip(),
        ..default()
    })
    .with_children(|rows| {
        for (index, address) in list.iter().enumerate().skip(launcher.scroll).take(VISIBLE_ROWS) {
            let removable = *address != local_address();
            server_row(rows, launcher, address, index, removable);
        }
    });
    page.spawn(Node {
        width: percent(100),
        justify_content: JustifyContent::Center,
        margin: UiRect::vertical(px(6)),
        ..default()
    })
    .with_children(|row| {
        button(
            row,
            "Перейти на вкладку серверов",
            Action::Tab(Tab::Servers),
            true,
            Node::default(),
        );
    });
    page.spawn(Node {
        width: percent(100),
        column_gap: px(6),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|row| {
        if launcher.direct_open {
            field(
                row,
                &launcher.direct,
                "хост:порт",
                Field::Direct,
                launcher.focused == Some(Field::Direct),
                px(260),
            );
            button(row, "Подключиться", Action::ConfirmDirect, true, Node::default());
            button(row, "Отмена", Action::Cancel, true, Node::default());
        } else {
            button(
                row,
                "Прямое подключение к серверу",
                Action::OpenDirect,
                true,
                Node::default(),
            );
        }
    });
}

fn servers(
    page: &mut ChildSpawnerCommands,
    launcher: &Launcher,
    settings: &crate::settings::Settings,
) {
    let all = favorites(settings);
    let needle = launcher.search.to_lowercase();
    let shown: Vec<&String> = all
        .iter()
        .filter(|address| {
            let name = match launcher.statuses.get(*address) {
                Some(Status::Online(s)) => s.name.to_lowercase(),
                _ => String::new(),
            };
            needle.is_empty() || address.contains(&needle) || name.contains(&needle)
        })
        .collect();
    // Заголовки колонок.
    page.spawn((
        Node {
            width: percent(100),
            height: px(28),
            align_items: AlignItems::Center,
            border: UiRect::bottom(px(1)),
            ..default()
        },
        BorderColor::from(SEPARATOR),
    ))
    .with_children(|head| {
        head.spawn(Node {
            flex_grow: 1.0,
            padding: UiRect::left(px(30)),
            ..default()
        })
        .with_children(|c| text(c, "Название сервера", 15.0, TEXT_DIM));
        for (title, width) in [("Время", 100.0), ("Игроки", 100.0), ("", 130.0)] {
            head.spawn(Node {
                width: px(width),
                justify_content: JustifyContent::Center,
                ..default()
            })
            .with_children(|c| text(c, title, 15.0, TEXT_DIM));
        }
    });
    page.spawn(Node {
        width: percent(100),
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        overflow: Overflow::clip(),
        ..default()
    })
    .with_children(|rows| {
        if shown.is_empty() {
            text(rows, "Ничего не найдено.", 15.0, TEXT_DIM);
        }
        for (index, address) in shown.iter().enumerate().skip(launcher.scroll).take(VISIBLE_ROWS) {
            let removable = **address != local_address();
            server_row(rows, launcher, address, index, removable);
        }
    });
    page.spawn(Node {
        width: percent(100),
        column_gap: px(6),
        align_items: AlignItems::Center,
        margin: UiRect::top(px(6)),
        ..default()
    })
    .with_children(|row| {
        field(
            row,
            &launcher.search,
            "Поиск серверов…",
            Field::Search,
            launcher.focused == Some(Field::Search),
            Val::Auto,
        );
        row.spawn(Node {
            flex_grow: 1.0,
            ..default()
        });
        text(row, format!("Показано {} из {}", shown.len(), all.len()), 14.0, TEXT_DIM);
        button(row, "Обновить", Action::Refresh, true, Node::default());
    });
}

fn news(page: &mut ChildSpawnerCommands, launcher: &Launcher) {
    section_title(page, "Последние новости:");
    page.spawn(Node {
        width: percent(100),
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        row_gap: px(4),
        margin: UiRect::top(px(6)),
        overflow: Overflow::clip(),
        ..default()
    })
    .with_children(|list| {
        if launcher.news.is_empty() {
            text(list, "Новостей пока нет.", 15.0, TEXT_DIM);
        }
        for (index, entry) in launcher.news.iter().enumerate().skip(launcher.scroll) {
            button(
                list,
                &entry.title,
                Action::News(index),
                true,
                Node {
                    width: percent(100),
                    min_height: px(28),
                    flex_shrink: 0.0,
                    ..default()
                },
            );
            if launcher.news_open == Some(index) {
                let body: String = entry.body.chars().take(2400).collect();
                list.spawn((
                    Node {
                        padding: UiRect::all(px(10)),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    BackgroundColor(ROW_B),
                ))
                .with_children(|b| text(b, body, 14.0, TEXT));
            }
        }
    });
}

fn settings_tab(
    page: &mut ChildSpawnerCommands,
    launcher: &Launcher,
    settings: &crate::settings::Settings,
) {
    let header_chip = |page: &mut ChildSpawnerCommands, title: &str| {
        page.spawn((
            Node {
                align_self: AlignSelf::FlexStart,
                padding: UiRect::axes(px(10), px(4)),
                margin: UiRect::vertical(px(6)),
                ..default()
            },
            BackgroundColor(BUTTON),
        ))
        .with_child((Text::new(title), TextFont::from_font_size(16.0), TextColor(TEXT)));
    };
    header_chip(page, "Персонаж");
    page.spawn(Node {
        column_gap: px(8),
        align_items: AlignItems::Center,
        ..default()
    })
    .with_children(|row| {
        text(row, "Имя игрока:", 15.0, TEXT);
        field(
            row,
            &settings.player_name,
            "Игрок",
            Field::Name,
            launcher.focused == Some(Field::Name),
            px(260),
        );
    });
    text(
        page,
        "Под этим именем вас увидят на сервере. Сохраняется сразу.",
        14.0,
        TEXT_DIM,
    );
    header_chip(page, "Графика и интерфейс");
    let toggle = |page: &mut ChildSpawnerCommands, on: bool, label: &str, desc: &str, t: Toggle| {
        page.spawn((
            Button,
            Action::Toggle(t),
            Hover {
                base: Color::NONE,
                hover: TAB_HOVER,
            },
            BackgroundColor(Color::NONE),
            Node {
                column_gap: px(8),
                align_items: AlignItems::Center,
                padding: UiRect::vertical(px(4)),
                ..default()
            },
        ))
        .with_children(|row| {
            row.spawn((
                Node {
                    width: px(18),
                    height: px(18),
                    border: UiRect::all(px(1)),
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                BackgroundColor(FIELD),
                BorderColor::from(TEXT_DIM),
            ))
            .with_children(|b| {
                // Отметка — залитый квадрат (в шрифте нет «✓»).
                if on {
                    b.spawn((
                        Node {
                            width: px(10),
                            height: px(10),
                            ..default()
                        },
                        BackgroundColor(GOLD),
                    ));
                }
            });
            text(row, label, 16.0, TEXT);
        });
        text(page, desc, 14.0, TEXT_DIM);
    };
    toggle(
        page,
        settings.fullscreen,
        "Полноэкранный режим",
        "Окно на весь экран без рамки.",
        Toggle::Fullscreen,
    );
    toggle(
        page,
        settings.show_fps,
        "Показывать FPS",
        "Счётчик кадров в углу экрана.",
        Toggle::ShowFps,
    );
    toggle(
        page,
        settings.foot_walk_animation,
        "Анимация шага",
        "Покачивание персонажа при ходьбе (`accessibility.foot_walk_animation`).",
        Toggle::FootWalk,
    );
}

/// Сигнатура для перерисовки: что видно на экране.
type Signature = (String, u64);

/// Перерисовывает лаунчер при изменении состояния или настроек.
pub fn render_launcher(
    mut commands: Commands,
    state: Res<LobbyState>,
    launcher: Res<Launcher>,
    settings: Res<crate::settings::Settings>,
    roots: Query<Entity, With<LauncherRoot>>,
    mut last: Local<Option<Signature>>,
) {
    if *state != LobbyState::Launcher {
        for root in roots.iter() {
            commands.entity(root).despawn();
        }
        *last = None;
        return;
    }
    if last.is_some() && !launcher.is_changed() && !settings.is_changed() {
        return;
    }
    *last = Some((format!("{:?}", launcher.tab), 0));
    for root in roots.iter() {
        commands.entity(root).despawn();
    }
    commands
        .spawn((
            LauncherRoot,
            Node {
                width: percent(100),
                height: percent(100),
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(BG),
            // Поверх игрового HUD (он ещё не нужен).
            GlobalZIndex(100),
        ))
        .with_children(|root| {
            header(root, &settings);
            root.spawn(Node {
                width: percent(100),
                flex_grow: 1.0,
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(px(10)),
                overflow: Overflow::clip(),
                ..default()
            })
            .with_children(|page| match launcher.tab {
                Tab::Home => home(page, &launcher, &settings),
                Tab::Servers => servers(page, &launcher, &settings),
                Tab::News => news(page, &launcher),
                Tab::Settings => settings_tab(page, &launcher, &settings),
            });
            tab_bar(root, launcher.tab);
        });
}

/// Подсветка кнопок под курсором.
pub fn launcher_hover(mut buttons: Query<(&Interaction, &Hover, &mut BackgroundColor), Changed<Interaction>>) {
    for (interaction, hover, mut color) in buttons.iter_mut() {
        color.0 = match interaction {
            Interaction::None => hover.base,
            _ => hover.hover,
        };
    }
}

/// Переход к подключению: адрес в настройки и экран лобби сервера.
fn connect(
    address: &str,
    settings: &mut crate::settings::Settings,
    state: &mut LobbyState,
    launcher: &mut Launcher,
) {
    let address = address.trim().to_string();
    if address.is_empty() {
        return;
    }
    settings.server = address.clone();
    if !settings.servers.contains(&address) {
        settings.servers.push(address.clone());
    }
    settings.save();
    launcher.focused = None;
    *state = LobbyState::Lobby;
    tracing::info!(server = %address, "launcher: подключение");
}

/// Клики по кнопкам лаунчера.
#[allow(clippy::too_many_arguments)]
pub fn launcher_click(
    buttons: Query<(&Interaction, &Action), Changed<Interaction>>,
    mut launcher: ResMut<Launcher>,
    mut settings: ResMut<crate::settings::Settings>,
    mut state: ResMut<LobbyState>,
    mut windows: Query<&mut Window>,
) {
    for (interaction, action) in buttons.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match action.clone() {
            Action::Tab(tab) => {
                launcher.tab = tab;
                launcher.scroll = 0;
                launcher.focused = None;
                if tab == Tab::Servers || tab == Tab::Home {
                    let list = favorites(&settings);
                    launcher.refresh(&list);
                }
            }
            Action::Refresh => {
                let list = favorites(&settings);
                launcher.refresh(&list);
            }
            Action::Connect(address) => {
                connect(&address, &mut settings, &mut state, &mut launcher);
            }
            Action::Expand(address) => {
                launcher.expanded = if launcher.expanded.as_deref() == Some(address.as_str()) {
                    None
                } else {
                    Some(address)
                };
            }
            Action::Remove(address) => {
                settings.servers.retain(|s| *s != address);
                settings.save();
                launcher.expanded = None;
            }
            Action::OpenAdd => {
                launcher.adding = true;
                launcher.focused = Some(Field::Add);
            }
            Action::ConfirmAdd => {
                let address = launcher.add.trim().to_string();
                if !address.is_empty() && !settings.servers.contains(&address) {
                    settings.servers.push(address.clone());
                    settings.save();
                    launcher.refresh(&[address]);
                }
                launcher.add.clear();
                launcher.adding = false;
                launcher.focused = None;
            }
            Action::OpenDirect => {
                launcher.direct_open = true;
                launcher.focused = Some(Field::Direct);
            }
            Action::ConfirmDirect => {
                let address = launcher.direct.clone();
                connect(&address, &mut settings, &mut state, &mut launcher);
            }
            Action::Cancel => {
                launcher.adding = false;
                launcher.direct_open = false;
                launcher.focused = None;
            }
            Action::News(index) => {
                launcher.news_open = if launcher.news_open == Some(index) {
                    None
                } else {
                    Some(index)
                };
            }
            Action::Url(url) => crate::lobby::open_url(url),
            Action::Focus(field) => launcher.focused = Some(field),
            Action::Toggle(toggle) => {
                match toggle {
                    Toggle::Fullscreen => {
                        settings.fullscreen = !settings.fullscreen;
                        if let Ok(mut window) = windows.single_mut() {
                            window.mode = if settings.fullscreen {
                                WindowMode::BorderlessFullscreen(MonitorSelection::Primary)
                            } else {
                                WindowMode::Windowed
                            };
                        }
                    }
                    Toggle::ShowFps => settings.show_fps = !settings.show_fps,
                    Toggle::FootWalk => {
                        settings.foot_walk_animation = !settings.foot_walk_animation;
                    }
                }
                settings.save();
            }
        }
    }
}

/// Ввод текста в поле в фокусе; Enter подтверждает, Esc — снимает фокус.
pub fn launcher_text_input(
    mut events: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut launcher: ResMut<Launcher>,
    mut settings: ResMut<crate::settings::Settings>,
    mut state: ResMut<LobbyState>,
) {
    use bevy::input::ButtonState;
    use bevy::input::keyboard::Key;
    if *state != LobbyState::Launcher {
        return;
    }
    let Some(field) = launcher.focused else {
        events.clear();
        return;
    };
    for event in events.read() {
        if event.state != ButtonState::Pressed {
            continue;
        }
        let edit = |launcher: &mut Launcher, settings: &mut crate::settings::Settings, f: &dyn Fn(&mut String)| {
            match field {
                Field::Search => f(&mut launcher.search),
                Field::Direct => f(&mut launcher.direct),
                Field::Add => f(&mut launcher.add),
                Field::Name => {
                    f(&mut settings.player_name);
                    settings.save();
                }
            }
        };
        match &event.logical_key {
            Key::Escape => launcher.focused = None,
            Key::Enter => match field {
                Field::Direct => {
                    let address = launcher.direct.clone();
                    connect(&address, &mut settings, &mut state, &mut launcher);
                }
                Field::Add => {
                    let address = launcher.add.trim().to_string();
                    if !address.is_empty() && !settings.servers.contains(&address) {
                        settings.servers.push(address.clone());
                        settings.save();
                        launcher.refresh(&[address]);
                    }
                    launcher.add.clear();
                    launcher.adding = false;
                    launcher.focused = None;
                }
                _ => launcher.focused = None,
            },
            Key::Backspace => edit(&mut launcher, &mut settings, &|s| {
                s.pop();
            }),
            Key::Space => {
                if matches!(field, Field::Name | Field::Search) {
                    edit(&mut launcher, &mut settings, &|s| s.push(' '));
                }
            }
            Key::Character(chars) => {
                let chars = chars.to_string();
                edit(&mut launcher, &mut settings, &|s| {
                    if s.chars().count() < 48 {
                        s.push_str(&chars);
                    }
                });
            }
            _ => {}
        }
    }
}

/// Колесо мыши — прокрутка списка (по строке).
pub fn launcher_scroll(mut wheel: MessageReader<MouseWheel>, mut launcher: ResMut<Launcher>, state: Res<LobbyState>) {
    if *state != LobbyState::Launcher {
        wheel.clear();
        return;
    }
    let mut delta = 0.0;
    for event in wheel.read() {
        delta += event.y;
    }
    if delta == 0.0 {
        return;
    }
    let scroll = launcher.scroll as i64 - delta.signum() as i64;
    launcher.scroll = scroll.max(0) as usize;
}

/// Тест `SSR_LAUNCHER_TAB=<servers|news|settings>`: открыть вкладку при старте
/// (для снимков), `SSR_LAUNCHER_NEWS=1` — раскрыть первую новость,
/// `SSR_LAUNCHER_CONNECT=1` — через 3 с «Подключиться» к локальному серверу.
pub fn launcher_test_mode(
    time: Res<Time>,
    mut launcher: ResMut<Launcher>,
    mut settings: ResMut<crate::settings::Settings>,
    mut state: ResMut<LobbyState>,
    mut done: Local<(bool, f32, bool)>,
) {
    if std::env::var_os("SSR_LAUNCHER_CONNECT").is_some() && !done.2 {
        done.1 += time.delta_secs();
        if done.1 >= 3.0 && *state == LobbyState::Launcher {
            done.2 = true;
            connect(&local_address(), &mut settings, &mut state, &mut launcher);
        }
    }
    if done.0 {
        return;
    }
    done.0 = true;
    if let Ok(tab) = std::env::var("SSR_LAUNCHER_TAB") {
        launcher.tab = match tab.as_str() {
            "servers" => Tab::Servers,
            "news" => Tab::News,
            "settings" => Tab::Settings,
            _ => Tab::Home,
        };
    }
    if std::env::var_os("SSR_LAUNCHER_NEWS").is_some() {
        launcher.news_open = Some(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn news_are_progress_sections_newest_first() {
        let news = parse_news("# Заголовок\nвступление\n## Первая\nтекст 1\n\n## Вторая\nтекст 2\n");
        assert_eq!(news.len(), 2);
        assert_eq!(news[0].title, "Вторая");
        assert_eq!(news[0].body, "текст 2");
        assert_eq!(news[1].body, "текст 1");
    }

    #[test]
    fn markdown_is_stripped() {
        assert_eq!(
            plain_markdown("- **Баг**: `Thrown` ок
### Осталось"),
            "- Баг: Thrown ок
Осталось"
        );
    }
}
