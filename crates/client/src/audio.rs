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
    /// Шаги (варианты) и разовые звуки действий (T5.3).
    steps: Vec<Handle<AudioSource>>,
    deny: Handle<AudioSource>,
    punch: Handle<AudioSource>,
    /// Ящик-`EntityStorage`: `OpenSound`/`CloseSound` из
    /// `EntityStorageComponent` сборки (`closetopen.ogg` / `closetclose.ogg`).
    closet_open: Handle<AudioSource>,
    closet_close: Handle<AudioSource>,
    /// Падение тела: `BodyFall` = `/Audio/Effects/bodyfall1..4.ogg`
    /// (`StandingStateComponent.DownSound`, играется в `Down()` при нокдауне,
    /// в том числе от стамина-крита).
    body_fall: Vec<Handle<AudioSource>>,
}

/// Заявки на разовые звуки от других систем (T5.3).
#[derive(Resource, Default)]
pub struct SoundRequests {
    /// Отказ доступа (красная лампа двери).
    pub deny: u32,
    /// Подтверждённый удар по цели.
    pub punch: u32,
}

/// Грузит звуки из `assets/sounds/ss14`.
pub fn load_sounds(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(Sounds {
        door_open: assets.load("sounds/ss14/airlock_open.ogg"),
        door_close: assets.load("sounds/ss14/airlock_close.ogg"),
        hit: assets.load("sounds/ss14/genhit1.ogg"),
        build: assets.load("sounds/ss14/box_deploy.ogg"),
        steps: vec![
            assets.load("sounds/ss14/floor1.ogg"),
            assets.load("sounds/ss14/floor2.ogg"),
            assets.load("sounds/ss14/floor3.ogg"),
        ],
        deny: assets.load("sounds/ss14/airlock_deny.ogg"),
        punch: assets.load("sounds/ss14/boxingpunch1.ogg"),
        closet_open: assets.load("sounds/ss14/closetopen.ogg"),
        closet_close: assets.load("sounds/ss14/closetclose.ogg"),
        body_fall: vec![
            assets.load("sounds/ss14/Effects/bodyfall1.ogg"),
            assets.load("sounds/ss14/Effects/bodyfall2.ogg"),
            assets.load("sounds/ss14/Effects/bodyfall3.ogg"),
            assets.load("sounds/ss14/Effects/bodyfall4.ogg"),
        ],
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

/// Ящики (`EntityStorage` в сборке): `OpenSound` / `CloseSound` из
/// `EntityStorageComponent` — `closetopen.ogg` / `closetclose.ogg`. Первое
/// появление ящика в кадре (репликация) звуком не считается, как у дверей.
pub fn container_sounds(
    mut commands: Commands,
    sounds: Res<Sounds>,
    containers: Query<
        (Entity, &ssr_core::inventory::Container),
        Changed<ssr_core::inventory::Container>,
    >,
    mut seen: Local<std::collections::HashSet<Entity>>,
) {
    if seen.len() > 4096 {
        seen.clear();
    }
    for (entity, container) in containers.iter() {
        if !seen.insert(entity) {
            let handle = if container.open {
                &sounds.closet_open
            } else {
                &sounds.closet_close
            };
            play(&mut commands, handle);
        }
    }
}

/// Падение тела: `BodyFall` (`bodyfall1..4.ogg`) играется в сборке в `Down()`
/// (`StandingStateSystem`), то есть при нокдауне — включая стамина-крит.
/// Звук играется и для своего игрока, и для чужих (как в движке: `PlayPredicted`
/// по сущности).
pub fn knockdown_sounds(
    mut commands: Commands,
    sounds: Res<Sounds>,
    knocked: Query<
        (Entity, &ssr_core::mechanics::KnockedDown),
        Added<ssr_core::mechanics::KnockedDown>,
    >,
) {
    if sounds.body_fall.is_empty() {
        return;
    }
    for (entity, _) in knocked.iter() {
        // Случайный вариант из четырёх, как `SoundCollectionSpecifier("BodyFall")`.
        let pick = (entity.to_bits() as usize) % sounds.body_fall.len();
        play(&mut commands, &sounds.body_fall[pick]);
        tracing::info!(?entity, "body fall sound");
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

/// Шаги своего игрока: звук каждые ~28 юнитов пути (примерно шаг по тайлу).
pub fn footsteps(
    mut commands: Commands,
    sounds: Res<Sounds>,
    own: Res<crate::inventory_ui::OwnPlayerEntity>,
    positions: Query<&ssr_core::PlayerPosition>,
    mut last: Local<(Option<[f32; 2]>, f32, usize)>,
) {
    let Some(position) = own.0.and_then(|entity| positions.get(entity).ok()) else {
        return;
    };
    let current = position.0;
    let previous = last.0;
    last.0 = Some(current);
    let Some(previous) = previous else {
        return;
    };
    let distance = ((current[0] - previous[0]).powi(2) + (current[1] - previous[1]).powi(2)).sqrt();
    // Телепорт (спавн/респавн) шагом не считаем.
    if distance > 200.0 {
        return;
    }
    last.1 += distance;
    if last.1 < 28.0 || sounds.steps.is_empty() {
        return;
    }
    last.1 = 0.0;
    last.2 = (last.2 + 1) % sounds.steps.len();
    if let Some(step) = sounds.steps.get(last.2) {
        play(&mut commands, step);
        tracing::debug!("footstep");
    }
}

/// Разовые звуки по заявкам (отказ доступа, удар).
pub fn play_requested_sounds(
    mut commands: Commands,
    sounds: Res<Sounds>,
    mut requests: ResMut<SoundRequests>,
) {
    if requests.deny > 0 {
        requests.deny = 0;
        play(&mut commands, &sounds.deny);
    }
    if requests.punch > 0 {
        requests.punch = 0;
        play(&mut commands, &sounds.punch);
    }
}
