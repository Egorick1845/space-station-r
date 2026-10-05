//! Оружие на клиенте (W-план): ввод выстрела, спрайты снарядов, звуки выстрела
//! и попаданий, счётчик патронов в HUD.
//!
//! Источники: `Content.Client/Weapons/Ranged/Systems/GunSystem.cs:171-234`
//! (стрельба по ЛКМ в боевом режиме, `RequestShootEvent`),
//! `.../GunSystem.AmmoCounter.cs:156` (текст счётчика — `x{count:00}`),
//! `Content.Client/Projectiles/ProjectileSystem.cs` (спрайт снаряда).

use bevy::prelude::*;
use lightyear::prelude::{Connected, MessageSender};

use ssr_core::inventory::{Hands, ItemPosition};
use ssr_core::weapons::{AmmoProvider, Gun, Projectile, WorldSound};

use crate::content::ClientContent;
use crate::hud::HudState;
use crate::inventory_ui::OwnPlayerEntity;
use crate::rsi::RsiRegistry;
use ssr_protocol::ClientMessage;

/// z снарядов: в сборке `drawdepth: Effects` — выше пола, ниже мобов.
const PROJECTILE_Z: f32 = 0.9;

/// Клиентский ввод стрельбы: ЛКМ в боевом режиме с оружием в активной руке.
/// Направление — от игрока к курсору (как `ShootCoordinates` в движке).
#[allow(clippy::too_many_arguments)]
pub fn shoot_input(
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    own: Res<OwnPlayerEntity>,
    hands: Query<&Hands>,
    guns: Query<&Gun>,
    positions: Query<&ItemPosition>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    combat: Res<HudState>,
) {
    if !combat.combat || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Some(player) = own.0 else {
        return;
    };
    // Оружие в активной руке — иначе стрелять нечем (ЛКМ бьёт/использует).
    let Some(gun_entity) = hands
        .get(player)
        .ok()
        .and_then(|hands| hands.active_item())
        .and_then(Entity::try_from_bits)
    else {
        return;
    };
    if guns.get(gun_entity).is_err() {
        return;
    }
    let Ok(reference) = positions.get(gun_entity) else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = cameras.single() else {
        return;
    };
    let Ok(world) = camera.viewport_to_world_2d(camera_transform, cursor) else {
        return;
    };
    let dir = Vec2::new(world.x - reference.0[0], world.y - reference.0[1]);
    let dir = if dir.length_squared() > 1e-6 {
        dir.normalize()
    } else {
        Vec2::NEG_Y
    };
    for mut sender in senders.iter_mut() {
        sender.send::<ssr_protocol::net::GameChannel>(ClientMessage::Shoot {
            dir: [dir.x, dir.y],
        });
    }
}

/// Перезарядка по R (в сборке — верб/`Z`, у нас клавиша R как в других шутерах;
/// механика та же: вынуть/вставить магазин).
pub fn reload_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    for mut sender in senders.iter_mut() {
        sender.send::<ssr_protocol::net::GameChannel>(ClientMessage::Reload);
    }
}

/// Спрайты снарядов: пуля из прототипа (`Bullets` — `projectiles2.rsi#bullet`).
pub fn sync_projectiles(
    mut commands: Commands,
    content: Res<ClientContent>,
    registry: Res<RsiRegistry>,
    projectiles: Query<(Entity, &Projectile, &ItemPosition), Added<Projectile>>,
    mut existing: Query<&mut Transform, With<ProjectileSprite>>,
) {
    for (entity, projectile, position) in projectiles.iter() {
        let Some(sprite_key) = content.proto_sprites.get(&projectile.proto) else {
            continue;
        };
        let (path, state) = match sprite_key.split_once('#') {
            Some((path, state)) => (path, state),
            None => (sprite_key.as_str(), "0"),
        };
        let key = format!("sprites/ss14/{path}#{state}");
        let Some(sprite) = registry.get(&key) else {
            continue;
        };
        let _ = &mut existing;
        commands.entity(entity).insert((
            ProjectileSprite,
            Sprite {
                image: sprite.image.clone(),
                texture_atlas: Some(TextureAtlas {
                    layout: sprite.layout.clone(),
                    index: sprite.index(0, 0),
                }),
                ..default()
            },
            Transform::from_xyz(position.0[0], position.0[1], PROJECTILE_Z),
        ));
    }
}

/// Позиция снаряда в мире — каждый кадр (сервер двигает его на своей стороне).
pub fn move_projectile_sprites(
    projectiles: Query<(&ItemPosition, &mut Transform), With<ProjectileSprite>>,
) {
    for (position, mut transform) in projectiles {
        transform.translation.x = position.0[0];
        transform.translation.y = position.0[1];
    }
}

/// Маркер спрайта снаряда.
#[derive(Component)]
pub struct ProjectileSprite;

/// Звуки из мира: выстрел, попадание, пустой магазин (`WorldSound` с сервера).
pub fn play_world_sounds(
    mut commands: Commands,
    sounds: Query<(Entity, &WorldSound), Added<WorldSound>>,
    assets: Res<AssetServer>,
    audio: Option<Res<crate::audio::Sounds>>,
) {
    let _ = audio;
    for (entity, sound) in sounds.iter() {
        let path = sound_path(&sound.path);
        let handle: Handle<AudioSource> = assets.load(path);
        commands.spawn((
            AudioPlayer::new(handle),
            PlaybackSettings::DESPAWN,
            Transform::from_xyz(sound.position[0], sound.position[1], 0.0),
        ));
        // Сущность-метка нужна только чтобы не проиграть звук дважды.
        commands.entity(entity).remove::<WorldSound>();
    }
}

/// `/Audio/Weapons/Guns/...` → `sounds/ss14/Weapons/Guns/...` (наш корень ассетов).
fn sound_path(path: &str) -> String {
    let trimmed = path.trim_start_matches('/');
    let trimmed = trimmed.strip_prefix("Audio/").unwrap_or(trimmed);
    format!("sounds/ss14/{trimmed}")
}

/// Счётчик патронов: `AmmoCounter` в сборке показывает `x{count:00}`.
/// Здесь — текст над слотами рук: патрон в патроннике + патроны магазина.
#[derive(Component)]
pub struct AmmoCounterText;

/// Спавн текста счётчика (один раз).
pub fn spawn_ammo_counter(mut commands: Commands, existing: Query<&AmmoCounterText>) {
    if !existing.is_empty() {
        return;
    }
    commands.spawn((
        AmmoCounterText,
        Text::new(""),
        TextFont {
            font_size: bevy::text::FontSize::Px(18.0),
            ..default()
        },
        TextColor(Color::srgb(0.9, 0.9, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(24.0),
            bottom: Val::Px(215.0),
            ..default()
        },
    ));
}

/// Обновление счётчика: `x{патроны:02}` + патрон в патроннике (как в сборке
/// `ChamberMagazineStatusControl`: магазин + `chambered`).
pub fn update_ammo_counter(
    own: Res<OwnPlayerEntity>,
    hands: Query<&Hands>,
    guns: Query<&Gun>,
    providers: Query<&AmmoProvider>,
    mut texts: Query<&mut Text, With<AmmoCounterText>>,
) {
    let mut label = String::new();
    if let Some(player) = own.0
        && let Some(gun_entity) = hands
            .get(player)
            .ok()
            .and_then(|hands| hands.active_item())
            .and_then(Entity::try_from_bits)
        && let Ok(gun) = guns.get(gun_entity)
    {
        let rounds = gun
            .magazine
            .and_then(Entity::try_from_bits)
            .and_then(|magazine| providers.get(magazine).ok())
            .map(|provider| provider.rounds.len())
            .unwrap_or(0);
        let chambered = gun.chamber.is_some();
        label = format!("x{:02}{}", rounds, if chambered { " +1" } else { "" });
    }
    for mut text in texts.iter_mut() {
        if text.0 != label {
            text.0 = label.clone();
        }
    }
}

/// Тест-режим `SSR_SHOOT_TEST=<сек>`: клиент стреляет вверх через N секунд после
/// подключения (3 выстрела с шагом 0.6 с) — проверка цепочки без мыши.
pub fn shoot_test_mode(
    time: Res<Time>,
    mut state: Local<(f32, u32)>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
) {
    let Ok(value) = std::env::var("SSR_SHOOT_TEST") else {
        return;
    };
    let Ok(delay) = value.parse::<f32>() else {
        return;
    };
    state.0 += time.delta_secs();
    if state.0 < delay || state.1 >= 3 {
        return;
    }
    // Три выстрела подряд с шагом 0.6 с — видно и разброс, и расход патронов.
    if state.1 > 0 && state.0 < delay + state.1 as f32 * 0.6 {
        return;
    }
    state.1 += 1;
    for mut sender in senders.iter_mut() {
        sender.send::<ssr_protocol::net::GameChannel>(ClientMessage::Shoot { dir: [0.0, -1.0] });
    }
    tracing::info!(shot = state.1, "shoot test: выстрел отправлен");
}

/// Тест-режим `SSR_SPAWN_MENU_TEST=1`: клиент открывает панель спавна через
/// 3 секунды после подключения (проверка списка и кликов без мыши).
pub fn spawn_menu_test(
    time: Res<Time>,
    mut state: Local<(f32, bool)>,
    mut hud: ResMut<crate::hud::HudState>,
) {
    if std::env::var_os("SSR_SPAWN_MENU_TEST").is_none() || state.1 {
        return;
    }
    state.0 += time.delta_secs();
    if state.0 < 3.0 {
        return;
    }
    state.1 = true;
    hud.spawn_open = true;
    tracing::info!("spawn menu test: панель спавна открыта");
}

/// Тест-режим `SSR_VERB_TEST=1`: клиент через 4 с запрашивает вербы для СВОЕЙ
/// сущности — проверка меню (Админ/Дебаг/View Variables) скриншотом.
pub fn verb_test_mode(
    time: Res<Time>,
    own: Res<crate::inventory_ui::OwnPlayerEntity>,
    mut menu: ResMut<crate::inventory_ui::ActionMenu>,
    mut state: Local<(f32, bool)>,
    mut senders: Query<&mut MessageSender<ClientMessage>, With<Connected>>,
    windows: Query<&Window>,
) {
    if std::env::var_os("SSR_VERB_TEST").is_none() || state.1 {
        return;
    }
    state.0 += time.delta_secs();
    if state.0 < 4.0 {
        return;
    }
    state.1 = true;
    let Some(player) = own.0 else {
        return;
    };
    let cursor = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position())
        .unwrap_or(Vec2::new(400.0, 300.0));
    menu.cursor = cursor;
    for mut sender in senders.iter_mut() {
        sender.send::<ssr_protocol::net::GameChannel>(ClientMessage::RequestActions {
            entity: player.to_bits(),
            tx: 0,
            ty: 0,
        });
    }
    tracing::info!("verb test: вербы запрошены");
}
