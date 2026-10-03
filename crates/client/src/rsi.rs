//! Bevy-интеграция RSI (SS14_IMPORT.md §6.4, задача IMP.1).
//!
//! [`build_registry`] при старте сканирует каталог ассетов: каждый PNG-лист
//! состояния заливается в [`Image`], ячейки (кадр × направление) регистрируются
//! в [`TextureAtlasLayout`]. Соглашение ссылок — как в SS14: `path/to/name.rsi#state`.

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
    /// (колонок, строк) сетки направлений одного кадра.
    pub grid: (u32, u32),
    /// Число кадров анимации; понадобится в фазе 2 (анимации по delays).
    #[allow(dead_code)]
    pub frames: u32,
}

impl RsiSprite {
    /// Индекс ячейки атласа для направления/кадра.
    pub fn index(&self, direction: u32, frame: u32) -> usize {
        let (cols, rows) = self.grid;
        (frame * rows * cols + direction) as usize
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

/// Сканирует все `*.rsi`-папки под `root` и строит реестр.
pub fn build_registry(
    images: &mut Assets<Image>,
    layouts: &mut Assets<TextureAtlasLayout>,
    root: &Path,
) -> RsiRegistry {
    let mut registry = RsiRegistry::default();

    for entry in walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_dir() || !entry.file_name().to_string_lossy().ends_with(".rsi") {
            continue;
        }
        let dir = entry.path();
        let Ok(rsi) = rsi::load_rsi(dir) else {
            tracing::warn!(path = %dir.display(), "rsi load failed");
            continue;
        };
        let relative = dir.strip_prefix(root).unwrap_or(dir).to_string_lossy();
        let prefix = format!("sprites/ss14/{}", relative.replace('\\', "/"));

        for state in &rsi.states {
            let key = format!("{prefix}#{}", state.name);
            let (image, layout) = upload_state(&rsi, state, images, layouts);
            let sprite = RsiSprite {
                image,
                layout,
                grid: state.grid(),
                frames: state.frames,
            };
            registry.sprites.insert(key, sprite);
        }
    }

    registry
}

/// Загружает лист состояния в GPU: один [`Image`] + [`TextureAtlasLayout`]
/// с ячейками по порядку кадров×направлений.
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

    let (cols, rows) = state.grid();
    let cells = state.frames * rows * cols;
    let mut layout = TextureAtlasLayout::new_empty(UVec2::new(w * cols, h * rows * state.frames));
    // Ячейка i: кадр = i / directions, направление = i % directions;
    // направление d занимает колонку d % cols и строку d / cols внутри кадра.
    let directions = state.directions;
    for i in 0..cells {
        let frame = i / directions;
        let dir = i % directions;
        let col = dir % cols;
        let row = dir / cols + frame * rows;
        layout.add_texture(URect::new(col * w, row * h, col * w + w, row * h + h));
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
