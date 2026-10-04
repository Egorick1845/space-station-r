//! Тема интерфейса по образцу SS14 (StyleNano + текстуры `Interface/*`).
//!
//! Значения взяты из сборки mini-station-goob:
//! * цвета кнопок — `Content.Client/Stylesheets/StyleNano.cs:246-256`;
//! * слоты — `SlotControl.cs` (64×64, сетка 4 px), подсветка активной руки —
//!   `HighlightRect` с текстурой `Slots/slot_highlight` (32×32, растянута ×2);
//! * панель статуса руки (`ItemStatusPanel`) — 125×60, текстуры
//!   `item_status_left/right.png` с patch margin top 6 / bottom 4 (×2);
//! * окно хранилища (`StorageWindow.cs`) — ячейки `Storage/tile_empty.png`
//!   16×16 при масштабе ×2 (32×32), сетка без зазоров, сайдбар и крестик
//!   `Storage/exit.png`;
//! * кнопки окон — `Nano/button.svg.96dpi.png` (patch margin 10), текст 13;
//! * верхняя панель — `MenuButton`: иконка 42×64 и подпись горячей клавиши
//!   (цвета #99a7b3 / #acbac6 / #75838e).

use bevy::image::ImageLoaderSettings;
use bevy::prelude::*;

// ------------------------------------------------------------------ цвета
//
// В сборке mini-station-goob активна не «классическая» StyleNano, а её
// стеклянный вариант (`StylesheetManager.UpdateMiniStyles` → StyleNano):
// панели и кнопки — плоские StyleBoxFlat, тонированные акцентом
// `ui.interface_accent_* = 127,183,255` (#7FB7FF). Значения ниже посчитаны
// той же формулой, что `StyleNano.Accent()` (alpha ≥ 0.92, mix × 0.85).

/// Акцент интерфейса (`CCVars.Interface`: 127,183,255).
#[allow(dead_code)]
pub const ACCENT: Color = Color::srgb_u8(0x7f, 0xb7, 0xff);
/// Кнопка в покое: `Accent("#20202ACC", 0.16)`.
pub const GLASS_BUTTON: Color = Color::srgba(0.188, 0.208, 0.267, 0.92);
/// Кнопка под курсором: `Accent("#30303CCC", 0.24)`.
pub const GLASS_BUTTON_HOVERED: Color = Color::srgba(0.267, 0.298, 0.373, 0.92);
/// Кнопка нажата: `Accent("#181822E6", 0.12)`.
pub const GLASS_BUTTON_PRESSED: Color = Color::srgba(0.145, 0.157, 0.212, 0.92);
/// Кнопка недоступна: `Accent("#14141C8A", 0.08)`.
#[allow(dead_code)]
pub const GLASS_BUTTON_DISABLED: Color = Color::srgba(0.114, 0.122, 0.165, 0.92);
/// Тело окна: `Accent("#14141CF0", 0.06)`.
pub const GLASS_PANEL: Color = Color::srgba(0.106, 0.110, 0.149, 0.94);
/// Шапка окна: `Accent("#2A2A38D9", 0.26)`.
pub const GLASS_HEADER: Color = Color::srgba(0.259, 0.290, 0.373, 0.92);
/// Поле ввода: `Accent("#12121CCF", 0.06)`.
pub const GLASS_LINEEDIT: Color = Color::srgba(0.098, 0.106, 0.149, 0.92);
/// Подчёркивание шапки — акцент с альфой 0.72 (`WindowHeadingBackground`).
pub const GLASS_HEADER_LINE: Color = Color::srgba(0.498, 0.718, 1.0, 0.72);
/// Заголовок окна (`DefaultWindow`: `windowTitle` = #EAF2FF).
pub const WINDOW_TITLE: Color = Color::srgb_u8(0xea, 0xf2, 0xff);
/// Отступ содержимого окна (`DefaultWindow.xaml`: `ContentsContainer Margin="10"`).
pub const WINDOW_CONTENT_MARGIN: f32 = 10.0;
/// Отступ кнопки по горизонтали (content margin H14 + padding 1).
pub const BUTTON_PADDING_H: f32 = 12.0;
/// Отступ кнопки по вертикали (content margin V2 + padding 1).
pub const BUTTON_PADDING_V: f32 = 4.0;

/// `StyleNano.ButtonColorDefault` (классическая нанопалитра — задел).
#[allow(dead_code)]
pub const BUTTON_DEFAULT: Color = Color::srgb_u8(0x46, 0x49, 0x66);
/// `StyleNano.ButtonColorHovered`.
#[allow(dead_code)]
pub const BUTTON_HOVERED: Color = Color::srgb_u8(0x57, 0x5b, 0x7f);
/// `StyleNano.ButtonColorPressed`.
#[allow(dead_code)]
pub const BUTTON_PRESSED: Color = Color::srgb_u8(0x3e, 0x6c, 0x45);
/// `StyleNano.ButtonColorDisabled`.
#[allow(dead_code)]
pub const BUTTON_DISABLED: Color = Color::srgb_u8(0x30, 0x31, 0x3c);
/// `StyleNano.ButtonColorDefaultRed` (задел под опасные действия).
#[allow(dead_code)]
pub const BUTTON_RED: Color = Color::srgb_u8(0xd4, 0x3b, 0x3b);
/// `StyleNano.NanoGold` — заголовки окон.
pub const NANO_GOLD: Color = Color::srgb_u8(0xa8, 0x8b, 0x5e);
/// `StyleNano.PanelDark` (задел: фон панелей без текстуры).
#[allow(dead_code)]
pub const PANEL_DARK: Color = Color::srgb_u8(0x1e, 0x1e, 0x22);
/// `themes.yml whiteText` — основной текст.
pub const TEXT: Color = Color::srgb_u8(0xff, 0xf5, 0xee);
/// `StyleNano.ItemStatusNotHeldColor` (`Color.Gray`).
pub const TEXT_MUTED: Color = Color::srgb_u8(0x80, 0x80, 0x80);
/// `MenuButton.ColorNormal` — иконки верхней панели.
pub const TOP_ICON: Color = Color::srgb_u8(0x99, 0xa7, 0xb3);
/// `MenuButton.ColorHovered`.
pub const TOP_ICON_HOVERED: Color = Color::srgb_u8(0xac, 0xba, 0xc6);
/// `MenuButton.ColorPressed`.
pub const TOP_ICON_PRESSED: Color = Color::srgb_u8(0x75, 0x83, 0x8e);
/// `StyleNano.ChatBackgroundColor`.
#[allow(dead_code)]
pub const CHAT_BACKGROUND: Color = Color::srgba(0.145, 0.145, 0.165, 0.86);
/// Модуляция ячеек хранилища — ровно как `StorageWindow.cs` (`#222222`):
/// светлая текстура `tile_empty.png` затемняется до тёмных ячеек SS14.
pub const GRID_BACKGROUND: Color = Color::srgb_u8(0x22, 0x22, 0x22);

// ------------------------------------------------------------------ размеры

/// `SlotControl.DefaultButtonSize` — сторона слота руки/хотбара.
pub const SLOT_SIZE: f32 = 64.0;
/// `GridContainer.Separations` — зазор между слотами.
#[allow(dead_code)]
pub const SLOT_GAP: f32 = 4.0;
/// Сторона ячейки хранилища (16 px × 2).
pub const STORAGE_CELL: f32 = 32.0;
/// Ширина панели статуса руки (`HotbarGui.xaml`: `SetWidth="125"`).
pub const STATUS_WIDTH: f32 = 125.0;
/// Высота панели статуса руки.
pub const STATUS_HEIGHT: f32 = 60.0;

// ------------------------------------------------------------------ шрифты

/// Базовый размер шрифта игровых панелей (`StyleBase.cs`, `GetStack("Regular", 13)`).
pub const FONT_BASE: f32 = 13.0;
/// `notoSans10` — панель статуса предмета.
pub const FONT_SMALL: f32 = 10.0;
/// `notoSansDisplayBold14` — подписи горячих клавиш и заголовки окон.
pub const FONT_LABEL: f32 = 14.0;

/// Текстуры интерфейса SS14.
#[derive(Resource, Default)]
pub struct UiTheme {
    /// `Slots/hand_l` — слот левой руки (с буквой).
    pub hand_l: Handle<Image>,
    /// `Slots/hand_r` — слот правой руки.
    pub hand_r: Handle<Image>,
    /// `Slots/slot_highlight` — рамка активного слота.
    pub slot_highlight: Handle<Image>,
    /// `Slots/toggle` — кнопка окна инвентаря.
    pub slot_toggle: Handle<Image>,
    /// `Slots/back` — слот рюкзака.
    pub slot_back: Handle<Image>,
    /// `Slots/belt` — слот пояса.
    pub slot_belt: Handle<Image>,
    /// `Slots/pocket` — слот кармана.
    pub slot_pocket: Handle<Image>,
    /// `Slots/id` — слот ID-карты.
    pub slot_id: Handle<Image>,
    /// `Slots/suit_storage` — слот разгрузки.
    pub slot_suit_storage: Handle<Image>,
    /// Слоты окна персонажа: [голова, комбинезон, куртка, перчатки, шея, маска,
    /// очки, уши, обувь] — текстуры `Slots/*` из сборки.
    pub character_slots: Vec<Handle<Image>>,
    /// `SlotBackground` — фон слота действия (`ActionButton` в SS14).
    pub slot_background: Handle<Image>,

    /// Иконки действий: [взгляд, бросок, удар, удар выключен].
    pub action_icons: Vec<Handle<Image>>,
    /// `item_status_left` — панель статуса слева от рук.
    pub status_left: Handle<Image>,
    /// `item_status_right` — панель статуса справа.
    pub status_right: Handle<Image>,
    /// `Storage/tile_empty` — ячейка хранилища.
    pub storage_tile: Handle<Image>,
    /// `Storage/exit` — красный крестик окна хранилища.
    pub storage_exit: Handle<Image>,
    /// `Storage/sidebar_fat` — сайдбар окна хранилища.
    pub storage_sidebar: Handle<Image>,
    /// `Nano/cross.svg` — крестик закрытия окна.
    pub cross: Handle<Image>,
    /// `Storage/piece_*` — рамки предметов в хранилище (центр, края, углы).
    #[allow(dead_code)]
    pub storage_pieces: Vec<Handle<Image>>,
    /// `Storage/sidebar_top|mid|bottom` — сегменты сайдбара хранилища.
    pub storage_sidebar_segments: Vec<Handle<Image>>,
    /// `item_status_*_highlight` — подсветка панели активной руки.
    pub status_highlights: Vec<Handle<Image>>,
    /// Иконки верхней панели (MenuButton): [меню, гайд, персонаж, эмоции,
    /// молот (крафт), кулак (бой), молоток судьи (админ), песочница (спавн)].
    pub icons: Vec<Handle<Image>>,
}

/// Рамки предметов в хранилище (`Storage/piece_*`, 8×8 при масштабе ×2).
pub const STORAGE_PIECES: [&str; 9] = [
    "sprites/ss14/Interface/Default/Storage/piece_center.png",
    "sprites/ss14/Interface/Default/Storage/piece_top.png",
    "sprites/ss14/Interface/Default/Storage/piece_bottom.png",
    "sprites/ss14/Interface/Default/Storage/piece_left.png",
    "sprites/ss14/Interface/Default/Storage/piece_right.png",
    "sprites/ss14/Interface/Default/Storage/piece_topLeft.png",
    "sprites/ss14/Interface/Default/Storage/piece_topRight.png",
    "sprites/ss14/Interface/Default/Storage/piece_bottomLeft.png",
    "sprites/ss14/Interface/Default/Storage/piece_bottomRight.png",
];

/// Текстуры слотов окна персонажа (порядок — как в [`UiTheme::character_slots`]).
pub const CHARACTER_SLOTS: [&str; 9] = [
    "sprites/ss14/Interface/Default/Slots/head.png",
    "sprites/ss14/Interface/Default/Slots/uniform.png",
    "sprites/ss14/Interface/Default/Slots/suit.png",
    "sprites/ss14/Interface/Default/Slots/gloves.png",
    "sprites/ss14/Interface/Default/Slots/neck.png",
    "sprites/ss14/Interface/Default/Slots/mask.png",
    "sprites/ss14/Interface/Default/Slots/glasses.png",
    "sprites/ss14/Interface/Default/Slots/ears.png",
    "sprites/ss14/Interface/Default/Slots/shoes.png",
];

/// Сегменты сайдбара хранилища: [верх, середина, низ].
pub const STORAGE_SIDEBAR: [&str; 3] = [
    "sprites/ss14/Interface/Default/Storage/sidebar_top.png",
    "sprites/ss14/Interface/Default/Storage/sidebar_mid.png",
    "sprites/ss14/Interface/Default/Storage/sidebar_bottom.png",
];

/// Подсветка панели статуса активной руки: [правая (слева на экране), левая].
pub const STATUS_HIGHLIGHTS: [&str; 2] = [
    "sprites/ss14/Interface/Default/item_status_right_highlight.png",
    "sprites/ss14/Interface/Default/item_status_left_highlight.png",
];

/// Пути иконок верхней панели (порядок — как в [`UiTheme::icons`]).
pub const TOP_ICONS: [&str; 8] = [
    "sprites/ss14/Interface/hamburger.svg.192dpi.png",
    "sprites/ss14/Interface/info.svg.192dpi.png",
    "sprites/ss14/Interface/character.svg.192dpi.png",
    "sprites/ss14/Interface/emotes.svg.192dpi.png",
    "sprites/ss14/Interface/hammer.svg.192dpi.png",
    "sprites/ss14/Interface/fist.svg.192dpi.png",
    "sprites/ss14/Interface/gavel.svg.192dpi.png",
    "sprites/ss14/Interface/sandbox.svg.192dpi.png",
];

/// Пути иконок действий (порядок — как в [`UiTheme::action_icons`]).
pub const ACTION_ICONS: [&str; 4] = [
    "sprites/ss14/Interface/Actions/eyeopen.png",
    "sprites/ss14/Interface/Actions/drop.png",
    "sprites/ss14/Interface/Actions/harm.png",
    "sprites/ss14/Interface/Actions/harmOff.png",
];

/// Грузит PNG интерфейса с пиксельной фильтрацией (`FilterMode.Point` в SS14):
/// без неё спрайты слотов и иконки размываются при масштабе ×2.
fn load_pixel(assets: &AssetServer, path: &'static str) -> Handle<Image> {
    assets
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.sampler = bevy::image::ImageSampler::nearest();
        })
        .load(path)
}

/// Грузит PNG интерфейса с линейной фильтрацией — как текстуры с
/// `sample.filter: true` в сборке (`cross.svg.png.yml`).
fn load_linear(assets: &AssetServer, path: &'static str) -> Handle<Image> {
    assets
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.sampler = bevy::image::ImageSampler::linear();
        })
        .load(path)
}

/// Загружает текстуры темы при старте.
pub fn load_ui_theme(mut commands: Commands, assets: Res<AssetServer>) {
    let load = |path: &'static str| load_pixel(&assets, path);
    commands.insert_resource(UiTheme {
        hand_l: load("sprites/ss14/Interface/Default/Slots/hand_l.png"),
        hand_r: load("sprites/ss14/Interface/Default/Slots/hand_r.png"),
        slot_highlight: load("sprites/ss14/Interface/Default/slot_highlight.png"),
        slot_toggle: load("sprites/ss14/Interface/Default/Slots/toggle.png"),
        slot_back: load("sprites/ss14/Interface/Default/Slots/back.png"),
        slot_belt: load("sprites/ss14/Interface/Default/Slots/belt.png"),
        slot_pocket: load("sprites/ss14/Interface/Default/Slots/pocket.png"),
        slot_id: load("sprites/ss14/Interface/Default/Slots/id.png"),
        slot_suit_storage: load("sprites/ss14/Interface/Default/Slots/suit_storage.png"),
        character_slots: CHARACTER_SLOTS.iter().map(|path| load(path)).collect(),
        slot_background: load("sprites/ss14/Interface/Default/SlotBackground.png"),
        status_left: load("sprites/ss14/Interface/Default/item_status_left.png"),
        status_right: load("sprites/ss14/Interface/Default/item_status_right.png"),
        storage_tile: load("sprites/ss14/Interface/Default/Storage/tile_empty.png"),
        storage_exit: load("sprites/ss14/Interface/Default/Storage/exit.png"),
        storage_sidebar: load("sprites/ss14/Interface/Default/Storage/sidebar_fat.png"),
        cross: load_linear(&assets, "sprites/ss14/Interface/Nano/cross.svg.png"),
        storage_pieces: STORAGE_PIECES.iter().map(|path| load(path)).collect(),
        storage_sidebar_segments: STORAGE_SIDEBAR.iter().map(|path| load(path)).collect(),
        status_highlights: STATUS_HIGHLIGHTS.iter().map(|path| load(path)).collect(),
        icons: TOP_ICONS.iter().map(|path| load(path)).collect(),
        action_icons: ACTION_ICONS.iter().map(|path| load(path)).collect(),
    });
}

/// 9-slice с разными отступами по сторонам (панель статуса: top 6 / bottom 4).
pub fn nine_slice_rect(
    handle: &Handle<Image>,
    left: f32,
    right: f32,
    top: f32,
    bottom: f32,
) -> ImageNode {
    let mut node = ImageNode::new(handle.clone());
    node.image_mode = NodeImageMode::Sliced(TextureSlicer {
        border: BorderRect {
            min_inset: Vec2::new(left, top),
            max_inset: Vec2::new(right, bottom),
        },
        center_scale_mode: SliceScaleMode::Stretch,
        sides_scale_mode: SliceScaleMode::Stretch,
        max_corner_scale: 1.0,
    });
    node
}

/// Картинка с текстурой, растянутой на весь узел (ячейки хранилища и иконки).
pub fn stretched(handle: &Handle<Image>) -> ImageNode {
    let mut node = ImageNode::new(handle.clone());
    node.image_mode = NodeImageMode::Stretch;
    node
}

/// Состояние кнопки для модуляции текстуры.
#[derive(Clone, Copy, PartialEq)]
pub enum UiButtonState {
    Normal,
    Hovered,
    Pressed,
}

impl UiButtonState {
    /// Состояние по компоненту `Interaction` (кнопка под курсором/нажата).
    pub fn from_interaction(interaction: &Interaction) -> Self {
        match interaction {
            Interaction::None => Self::Normal,
            Interaction::Hovered => Self::Hovered,
            Interaction::Pressed => Self::Pressed,
        }
    }
}

// ------------------------------------------------------------------ скроллбар

// Скроллбар SS14 (StyleBase/ScrollBar.cs): текстуры НЕТ — плоский StyleBoxFlat,
// дорожка не рисуется, стрелок нет. Граббер 10 px, минимум 10 px в длину.
/// `StyleBase.DefaultGrabberSize` — толщина полосы.
pub const SCROLLBAR_WIDTH: f32 = 10.0;
/// `ContentMarginTopOverride` — минимальная длина граббера.
pub const SCROLLBAR_MIN_GRABBER: f32 = 10.0;
/// Граббер в покое (`Color.Gray` с альфой 0.35 = `#80808059`).
pub const SCROLLBAR_GRABBER: Color = Color::srgba(0.502, 0.502, 0.502, 0.35);
/// Граббер под курсором (`#8C8C8C59`).
pub const SCROLLBAR_GRABBER_HOVERED: Color = Color::srgba(0.549, 0.549, 0.549, 0.35);
/// Граббер при перетаскивании (`#A0A0A059`) — задел под drag (T7.2).
#[allow(dead_code)]
pub const SCROLLBAR_GRABBER_GRABBED: Color = Color::srgba(0.627, 0.627, 0.627, 0.35);
/// `ScrollContainer.ScrollSpeedY` — шаг колеса, px за щелчок.
pub const SCROLLBAR_WHEEL_STEP: f32 = 50.0;
/// `LerpAnimate(rate: 15)` — скорость догоняния цели прокруткой.
pub const SCROLLBAR_ANIM_RATE: f32 = 15.0;
