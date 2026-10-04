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

use bevy::prelude::*;

// ------------------------------------------------------------------ цвета

/// `StyleNano.ButtonColorDefault`.
pub const BUTTON_DEFAULT: Color = Color::srgb_u8(0x46, 0x49, 0x66);
/// `StyleNano.ButtonColorHovered`.
pub const BUTTON_HOVERED: Color = Color::srgb_u8(0x57, 0x5b, 0x7f);
/// `StyleNano.ButtonColorPressed`.
pub const BUTTON_PRESSED: Color = Color::srgb_u8(0x3e, 0x6c, 0x45);
/// `StyleNano.ButtonColorDisabled`.
#[allow(dead_code)]
pub const BUTTON_DISABLED: Color = Color::srgb_u8(0x30, 0x31, 0x3c);
/// `StyleNano.ButtonColorDefaultRed` (задел под опасные действия).
#[allow(dead_code)]
pub const BUTTON_RED: Color = Color::srgb_u8(0xd4, 0x3b, 0x3b);
/// `StyleNano.NanoGold` — заголовки окон.
pub const NANO_GOLD: Color = Color::srgb_u8(0xa8, 0x8b, 0x5e);
/// `StyleNano.PanelDark`.
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
/// Модуляция ячеек хранилища: в `StorageWindow.cs` таблица затемняется `#222222`,
/// но на светлой текстуре `tile_empty.png` этого мало — берём чуть светлее,
/// чтобы границы клеток читались (как на скриншотах SS14).
pub const GRID_BACKGROUND: Color = Color::srgb_u8(0x55, 0x55, 0x5c);

// ------------------------------------------------------------------ размеры

/// `SlotControl.DefaultButtonSize` — сторона слота руки/хотбара.
pub const SLOT_SIZE: f32 = 64.0;
/// `GridContainer.Separations` — зазор между слотами.
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
    /// `Nano/button.svg.96dpi` — текстура кнопок.
    pub button: Handle<Image>,
    /// `Nano/lineedit` — текстура поля ввода (поиск в спавн-меню).
    pub lineedit: Handle<Image>,
    /// `Nano/window_background_bordered` — фон окна.
    pub window_background: Handle<Image>,
    /// `Nano/cross.svg` — крестик закрытия окна.
    pub cross: Handle<Image>,
    /// Иконки верхней панели (MenuButton): [меню, гайд, персонаж, эмоции,
    /// молот (крафт), кулак (бой), молоток судьи (админ), песочница (спавн)].
    pub icons: Vec<Handle<Image>>,
}

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

/// Загружает текстуры темы при старте.
pub fn load_ui_theme(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(UiTheme {
        hand_l: assets.load("sprites/ss14/Interface/Default/Slots/hand_l.png"),
        hand_r: assets.load("sprites/ss14/Interface/Default/Slots/hand_r.png"),
        slot_highlight: assets.load("sprites/ss14/Interface/Default/Slots/slot_highlight.png"),
        slot_toggle: assets.load("sprites/ss14/Interface/Default/Slots/toggle.png"),
        status_left: assets.load("sprites/ss14/Interface/Default/item_status_left.png"),
        status_right: assets.load("sprites/ss14/Interface/Default/item_status_right.png"),
        storage_tile: assets.load("sprites/ss14/Interface/Default/Storage/tile_empty.png"),
        storage_exit: assets.load("sprites/ss14/Interface/Default/Storage/exit.png"),
        storage_sidebar: assets.load("sprites/ss14/Interface/Default/Storage/sidebar_fat.png"),
        button: assets.load("sprites/ss14/Interface/Nano/button.svg.96dpi.png"),
        lineedit: assets.load("sprites/ss14/Interface/Nano/lineedit.png"),
        window_background: assets
            .load("sprites/ss14/Interface/Nano/window_background_bordered.png"),
        cross: assets.load("sprites/ss14/Interface/Nano/cross.svg.png"),
        icons: TOP_ICONS.iter().map(|path| assets.load(*path)).collect(),
    });
}

/// 9-slice картинка с растянутым центром — аналог `StyleBoxTexture` движка.
pub fn nine_slice(handle: &Handle<Image>, border: f32) -> ImageNode {
    let mut node = ImageNode::new(handle.clone());
    node.image_mode = NodeImageMode::Sliced(TextureSlicer {
        border: border_rect(border),
        center_scale_mode: SliceScaleMode::Stretch,
        sides_scale_mode: SliceScaleMode::Stretch,
        max_corner_scale: 1.0,
    });
    node
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

fn border_rect(border: f32) -> BorderRect {
    BorderRect {
        min_inset: Vec2::splat(border),
        max_inset: Vec2::splat(border),
    }
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
