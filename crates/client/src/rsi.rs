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
    "Mobs/Animals/monkey.rsi",
    "Mobs/Ghosts/ghost_human.rsi",
    "Objects/Tools/crowbar.rsi",
    "Structures/Walls/solid.rsi",
    "Structures/Doors/Airlocks/Standard/basic.rsi",
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

/// Загружает лист состояния в GPU: один [`Image`] + [`TextureAtlasLayout`],
/// где ячейка i — это i-я клетка листа в порядке обхода движка.
fn upload_state(
    rsi: &Rsi,
    state: &rsi::RsiState,
    images: &mut Assets<Image>,
    layouts: &mut Assets<TextureAtlasLayout>,
) -> (Handle<Image>, Handle<TextureAtlasLayout>) {
    let (w, h) = rsi.size;
    let rgba = state.sheet.to_rgba8();
    let image = Image::new(
        Extent3d {
            width: state.sheet.width(),
            height: state.sheet.height(),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba.into_vec(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    let image_handle = images.add(image);

    let (cols, rows) = state.sheet_grid(rsi.size);
    let mut layout =
        TextureAtlasLayout::new_empty(UVec2::new(state.sheet.width(), state.sheet.height()));
    // Ячейки добавляем строго в порядке обхода листа: кадры каждого направления подряд.
    for row in 0..rows {
        for col in 0..cols {
            layout.add_texture(URect::new(col * w, row * h, col * w + w, row * h + h));
        }
    }
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
