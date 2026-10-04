//! Bevy-интеграция RSI (SS14_IMPORT.md §6.4, задача IMP.1).
//!
//! [`build_registry`] при старте сканирует каталог ассетов: каждый PNG-лист
//! состояния заливается в [`Image`], клетки листа (в порядке обхода движка:
//! построчно, кадры направления подряд) регистрируются в [`TextureAtlasLayout`].
//! Соглашение ссылок — как в SS14: `path/to/name.rsi#state`.

use std::collections::HashMap;
use std::path::Path;

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ssr_core::rsi::{self, Rsi};

/// Ключ ссылки на состояние: `"sprites/ss14/Objects/Tools/crowbar.rsi#icon"`.
pub type RsiKey = String;

/// Одно состояние RSI, загруженное в GPU.
#[derive(Clone)]
pub struct RsiSprite {
    pub image: Handle<Image>,
    pub layout: Handle<TextureAtlasLayout>,
    /// 1, 4 или 8.
    pub directions: u32,
    /// Кадров на каждое направление (порядок обхода движка).
    pub frames_per_direction: Vec<u32>,
    /// Длительности кадров по направлениям (сек) — для анимаций (двери).
    pub delays: Vec<Vec<f32>>,
}

impl RsiSprite {
    /// Индекс ячейки атласа для направления/кадра (правило движка).
    pub fn index(&self, direction: u32, frame: u32) -> usize {
        let mut index = 0u32;
        for d in 0..direction.min(self.directions) {
            index += self
                .frames_per_direction
                .get(d as usize)
                .copied()
                .unwrap_or(1);
        }
        (index + frame) as usize
    }
}

/// Реестр RSI: грузится ЛЕНИВО (PLAN.md T5.3) — в assets 2500+ RSI из сборки,
/// поэтому набор состояния подгружается при первом обращении к нему.
#[derive(Resource)]
pub struct RsiRegistry {
    root: std::path::PathBuf,
    sprites: HashMap<RsiKey, RsiSprite>,
    /// Заявки на подгрузку папок RSI (`sprites/ss14/...rsi`), из `get`.
    pending: std::sync::Mutex<Vec<String>>,
    /// Счётчик загрузок: потребители по нему перерисовывают UI (T5.3).
    generation: u32,
}

impl Default for RsiRegistry {
    fn default() -> Self {
        Self {
            root: std::path::PathBuf::new(),
            sprites: HashMap::new(),
            pending: std::sync::Mutex::new(Vec::new()),
            generation: 0,
        }
    }
}

impl RsiRegistry {
    /// Реестр с корнем ассетов (`assets/sprites/ss14`).
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
            ..default()
        }
    }

    /// Сколько раз подгружались спрайты: потребители по изменению числа
    /// перерисовывают UI (иконка могла появиться позже первого кадра).
    pub fn generation(&self) -> u32 {
        self.generation
    }

    /// Спрайт состояния. Если RSI ещё не загружен — ставит его в очередь и
    /// возвращает None: спрайт появится через несколько кадров (T5.3).
    pub fn get(&self, key: &str) -> Option<&RsiSprite> {
        if let Some(sprite) = self.sprites.get(key) {
            return Some(sprite);
        }
        let folder = key.split('#').next()?;
        if let Ok(mut pending) = self.pending.lock()
            && !pending.iter().any(|item| item == folder)
        {
            pending.push(folder.to_string());
        }
        None
    }

    /// Загружает одну папку RSI (вызывается системой подгрузки).
    fn load_folder(
        &mut self,
        folder: &str,
        images: &mut Assets<Image>,
        layouts: &mut Assets<TextureAtlasLayout>,
    ) -> usize {
        let Some(relative) = folder.strip_prefix("sprites/ss14/") else {
            return 0;
        };
        let dir = self
            .root
            .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        let Ok(rsi) = rsi::load_rsi(&dir) else {
            tracing::warn!(path = %dir.display(), "rsi load failed");
            return 0;
        };
        let prefix = format!("sprites/ss14/{relative}");
        let mut loaded = 0;
        for state in &rsi.states {
            let key = format!("{prefix}#{}", state.name);
            if self.sprites.contains_key(&key) {
                continue;
            }
            let (image, layout) = upload_state(&rsi, state, images, layouts);
            self.sprites.insert(
                key,
                RsiSprite {
                    image,
                    layout,
                    directions: state.directions,
                    frames_per_direction: state.frames_per_direction.clone(),
                    delays: state.delays.clone(),
                },
            );
            loaded += 1;
        }
        loaded
    }
}

/// Система ленивой подгрузки: обрабатывает заявки из [`RsiRegistry::get`].
/// За кадр грузится не больше [`RSI_BUDGET_PER_FRAME`] папок (без фризов).
pub fn load_requested_rsi(
    mut registry: ResMut<RsiRegistry>,
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let requests: Vec<String> = match registry.pending.lock() {
        Ok(mut pending) => std::mem::take(&mut pending),
        Err(_) => return,
    };
    if requests.is_empty() {
        return;
    }
    let mut remaining = Vec::new();
    for (index, folder) in requests.into_iter().enumerate() {
        if index >= RSI_BUDGET_PER_FRAME {
            remaining.push(folder);
            continue;
        }
        let states = registry.load_folder(&folder, &mut images, &mut layouts);
        if states > 0 {
            registry.generation += 1;
            tracing::debug!(folder = %folder, states, "rsi loaded lazily");
        }
    }
    if !remaining.is_empty()
        && let Ok(mut pending) = registry.pending.lock()
    {
        pending.extend(remaining);
    }
}

/// Структурные RSI, которые нужны в первом же кадре (комната, двери, ящики,
/// проводка): остальное подгружается лениво по обращению (T5.3).
pub const STARTUP_RSI: &[&str] = &[
    "Structures/Walls/solid.rsi",
    "Structures/Doors/Airlocks/Standard/basic.rsi",
    "Structures/Storage/Crates/generic.rsi",
    "_Mini/Lobby/mars.rsi",
    "Structures/Power/Cables/lv_cable.rsi",
    "Structures/Power/Generation/portable_generator.rsi",
    "Structures/Wallmounts/Lighting/light_tube.rsi",
];

/// Сколько папок RSI подгружаем за кадр (ленивая загрузка).
const RSI_BUDGET_PER_FRAME: usize = 3;

/// Загружает структурные RSI из [`STARTUP_RSI`] (остальное — лениво).
pub fn build_registry(
    images: &mut Assets<Image>,
    layouts: &mut Assets<TextureAtlasLayout>,
    root: &Path,
) -> RsiRegistry {
    let mut registry = RsiRegistry::new(root);
    for relative in STARTUP_RSI {
        let folder = format!("sprites/ss14/{relative}");
        registry.load_folder(&folder, images, layouts);
    }
    registry
}

/// Предел размера текстуры на типичном GPU: листы шире (например, лента из
/// 64 кадров 320×180 = 20480 px) нужно перепаковать в сетку поближе к квадрату.
const MAX_TEXTURE_DIM: u32 = 16384;

/// Загружает лист состояния в GPU: один [`Image`] + [`TextureAtlasLayout`],
/// где ячейка i — это i-я клетка листа в порядке обхода движка.
///
/// Если лист не влезает в лимит GPU по ширине/высоте — клетки перепаковываются
/// в квадратную сетку (порядок клеток в атласе сохраняется).
fn upload_state(
    rsi: &Rsi,
    state: &rsi::RsiState,
    images: &mut Assets<Image>,
    layouts: &mut Assets<TextureAtlasLayout>,
) -> (Handle<Image>, Handle<TextureAtlasLayout>) {
    let (w, h) = rsi.size;
    let sheet = state.sheet.to_rgba8();
    let (src_cols, src_rows) = state.sheet_grid(rsi.size);
    let cells = src_cols * src_rows;

    let max_cols = (MAX_TEXTURE_DIM / w).max(1);
    let max_rows = (MAX_TEXTURE_DIM / h).max(1);
    let fits = src_cols <= max_cols && src_rows <= max_rows;

    let (pack_cols, pack_rows) = if fits {
        (src_cols, src_rows)
    } else {
        // Перепаковка: сетка поближе к квадрату в пределах лимитов.
        let mut cols = (cells as f32).sqrt().ceil() as u32;
        cols = cols.clamp(1, max_cols);
        let mut rows = cells.div_ceil(cols);
        if rows > max_rows {
            rows = max_rows;
            cols = cells.div_ceil(rows);
        }
        (cols, rows)
    };

    // Итоговое изображение атласа.
    let mut packed = image::RgbaImage::new(pack_cols * w, pack_rows * h);
    let mut layout = TextureAtlasLayout::new_empty(UVec2::new(pack_cols * w, pack_rows * h));
    for cell in 0..cells {
        let src_col = cell % src_cols;
        let src_row = cell / src_cols;
        let dst_col = cell % pack_cols;
        let dst_row = cell / pack_cols;
        let view = image::imageops::crop_imm(&sheet, src_col * w, src_row * h, w, h);
        image::imageops::overlay(
            &mut packed,
            &view.to_image(),
            (dst_col * w) as i64,
            (dst_row * h) as i64,
        );
        layout.add_texture(URect::new(
            dst_col * w,
            dst_row * h,
            dst_col * w + w,
            dst_row * h + h,
        ));
    }
    if !fits {
        tracing::debug!(
            sheet = ?(src_cols, src_rows),
            packed = ?(pack_cols, pack_rows),
            "rsi sheet repacked for GPU limits"
        );
    }

    let image = Image::new(
        Extent3d {
            width: packed.width(),
            height: packed.height(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        packed.into_raw(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    let image_handle = images.add(image);
    let layout_handle = layouts.add(layout);
    (image_handle, layout_handle)
}

/// Спавнит сущность со спрайтом из RSI: `key` = `"path/to/name.rsi#state"`.
pub fn spawn_rsi_sprite(
    commands: &mut Commands,
    registry: &RsiRegistry,
    key: &str,
    direction: u32,
    position: Vec3,
) -> Option<Entity> {
    let sprite = registry.get(key)?;
    let sprite_component = Sprite {
        image: sprite.image.clone(),
        texture_atlas: Some(TextureAtlas {
            layout: sprite.layout.clone(),
            index: sprite.index(direction, 0),
        }),
        ..default()
    };
    Some(
        commands
            .spawn((sprite_component, Transform::from_translation(position)))
            .id(),
    )
}
