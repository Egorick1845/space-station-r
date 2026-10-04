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

/// Реестр загруженных RSI-состояний.
#[derive(Resource, Default)]
pub struct RsiRegistry {
    sprites: HashMap<RsiKey, RsiSprite>,
}

impl RsiRegistry {
    pub fn get(&self, key: &str) -> Option<&RsiSprite> {
        self.sprites.get(key)
    }
}

/// RSI-наборы, которые грузятся при старте. В assets лежат 2500+ RSI из сборки,
/// грузить их все нельзя — расширяем список по мере использования (ленивая
/// загрузка по требованию — отдельная задача, T5.3).
pub const STARTUP_RSI: &[&str] = &[
    "Mobs/Ghosts/ghost_human.rsi",
    "Objects/Tools/crowbar.rsi",
    // Листы металла (T5.3): иконка и inhand для SteelSheet.
    "Objects/Materials/Sheets/metal.rsi",
    "Structures/Walls/solid.rsi",
    "Structures/Doors/Airlocks/Standard/basic.rsi",
    // Анимированный фон лобби мини-станции (64 кадра).
    "_Mini/Lobby/mars.rsi",
    // Ящик-контейнер (T3.4): состояния base/closed/open.
    "Structures/Storage/Crates/generic.rsi",
    // Глаза гуманоида (отдельный слой поверх головы, как в SS14).
    "Mobs/Customization/eyes.rsi",
    // Тела рас (T5.3): части гуманоидов, собираются в humanoid.rs.
    "Mobs/Species/Human/parts.rsi",
    "Mobs/Species/Skeleton/parts.rsi",
    "Mobs/Species/Arachnid/parts.rsi",
    "Mobs/Species/Diona/parts.rsi",
    "Mobs/Species/Gingerbread/parts.rsi",
    "Mobs/Species/Moth/parts.rsi",
    "Mobs/Species/Reptilian/parts.rsi",
    "Mobs/Species/Slime/parts.rsi",
    "Mobs/Species/Vox/parts.rsi",
];

/// Загружает RSI из [`STARTUP_RSI`] и строит реестр.
pub fn build_registry(
    images: &mut Assets<Image>,
    layouts: &mut Assets<TextureAtlasLayout>,
    root: &Path,
) -> RsiRegistry {
    let mut registry = RsiRegistry::default();

    for relative in STARTUP_RSI {
        let dir = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        let Ok(rsi) = rsi::load_rsi(&dir) else {
            tracing::warn!(path = %dir.display(), "rsi load failed");
            continue;
        };
        let prefix = format!("sprites/ss14/{relative}");

        for state in &rsi.states {
            let key = format!("{prefix}#{}", state.name);
            let (image, layout) = upload_state(&rsi, state, images, layouts);
            let sprite = RsiSprite {
                image,
                layout,
                directions: state.directions,
                frames_per_direction: state.frames_per_direction.clone(),
                delays: state.delays.clone(),
            };
            registry.sprites.insert(key, sprite);
        }
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
