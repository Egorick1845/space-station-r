//! Звуки игры: двери, урон, работа инструментом. Общий том — из настроек
//! (`GlobalVolume`, settings.rs). Звуки — из сборки мини-станции.

use bevy::audio::{AudioPlayer, AudioSource, PlaybackSettings};
use bevy::prelude::*;
use ssr_core::Door;
use ssr_core::inventory::Health;
use ssr_core::tiles::TileChunkData;

/// Загруженные звуки (грузятся один раз при старте).
#[derive(Resource, Default)]
pub struct Sounds {
    door_open: Handle<AudioSource>,
    door_close: Handle<AudioSource>,
    hit: Handle<AudioSource>,
    build: Handle<AudioSource>,
}

/// Грузит звуки из `assets/sounds/ss14`.
pub fn load_sounds(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Sounds {
        door_open: assets.load("sounds/ss14/airlock_open.ogg"),
        door_close: assets.load("sounds/ss14/airlock_close.ogg"),
        hit: assets.load("sounds/ss14/genhit1.ogg"),
        build: assets.load("sounds/ss14/box_deploy.ogg"),
    });
}

/// Проигрывает звук и сразу отпускает сущность.
fn play(commands: &mut Commands, handle: &Handle<AudioSource>) {
    commands.spawn((AudioPlayer::new(handle.clone()), PlaybackSettings::DESPAWN));
}

/// Двери: звук открытия/закрытия. Первое появление двери в кадре (репликация)
/// звуком не считается — только реальные переключения.
pub fn door_sounds(
    mut commands: Commands,
    sounds: Res<Sounds>,
    doors: Query<(Entity, &Door), Changed<Door>>,
    mut seen: Local<std::collections::HashSet<Entity>>,
) {
    if seen.len() > 4096 {
        seen.clear();
    }
    for (entity, door) in doors.iter() {
        if !seen.insert(entity) {
            let handle = if door.open {
                &sounds.door_open
            } else {
                &sounds.door_close
            };
            play(&mut commands, handle);
        }
    }
}

/// Свой игрок получил урон — звук удара.
pub fn damage_sound(
    mut commands: Commands,
    sounds: Res<Sounds>,
    own: Res<crate::inventory_ui::OwnPlayerEntity>,
    healths: Query<&Health>,
    mut last: Local<Option<i32>>,
) {
    let Some(entity) = own.0 else {
        return;
    };
    let Ok(health) = healths.get(entity) else {
        return;
    };
    if let Some(previous) = *last
        && health.current < previous
    {
        play(&mut commands, &sounds.hit);
    }
    *last = Some(health.current);
}

/// Правка тайлов (стройка/разбор) — рабочий звук.
pub fn build_sound(
    mut commands: Commands,
    sounds: Res<Sounds>,
    chunks: Query<(), Changed<TileChunkData>>,
) {
    if chunks.iter().next().is_some() {
        play(&mut commands, &sounds.build);
    }
}
