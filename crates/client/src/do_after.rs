//! Полоска DoAfter над персонажем — перенос `Content.Client/DoAfter/DoAfterOverlay.cs`:
//! фон `Interface/Misc/progress_bar.rsi#icon` (24×7), заливка от `StartX = 2`
//! до `EndX = 22` пикселей в строке y = 3..4, цвет —
//! `ProgressColorSystem.GetProgressColor` ([`ssr_core::damage::progress_color`]).
//! Сервер реплицирует `DoAfterBar { delay, id }`; отсчёт идёт на клиенте и
//! сбрасывается при смене `id` (повтор лечения).

use bevy::prelude::*;
use ssr_core::damage::{DoAfterBar, progress_color};

use crate::inventory_ui::{OwnPlayerEntity, RemotePlayerVisual};
use crate::rsi::RsiRegistry;

const BAR_RSI: &str = "sprites/ss14/Interface/Misc/progress_bar.rsi#icon";
const BAR_WIDTH: f32 = 24.0;
const START_X: f32 = 2.0;
const END_X: f32 = 22.0;
/// Над головой: верх спрайта тела (32 px) + половина полоски.
const BAR_Y: f32 = 20.0;

/// Полоска (фон) над владельцем.
#[derive(Component)]
pub struct DoAfterVisual {
    owner: Entity,
    id: u32,
    elapsed: f32,
}

/// Заливка прогресса (дочерняя к фону).
#[derive(Component)]
pub struct DoAfterFill;

#[allow(clippy::too_many_arguments)]
pub fn sync_do_after_bars(
    mut commands: Commands,
    time: Res<Time>,
    registry: Res<RsiRegistry>,
    own: Res<OwnPlayerEntity>,
    players: Query<Entity, With<crate::Player>>,
    visuals: Query<(Entity, &RemotePlayerVisual)>,
    bars_data: Query<&DoAfterBar>,
    mut bars: Query<(Entity, &mut DoAfterVisual, &Children)>,
    mut fills: Query<(&mut Sprite, &mut Transform), With<DoAfterFill>>,
) {
    let mut owners: Vec<(Entity, Entity)> = players
        .iter()
        .filter_map(|entity| own.0.map(|data| (entity, data)))
        .collect();
    owners.extend(visuals.iter().map(|(entity, visual)| (entity, visual.player)));
    let dt = time.delta_secs();

    // Полоски без владельца или без DoAfter — убрать.
    for (entity, visual, _) in bars.iter() {
        let active = owners
            .iter()
            .find(|(owner, _)| *owner == visual.owner)
            .is_some_and(|(_, data)| bars_data.get(*data).is_ok());
        if !active {
            commands.entity(entity).despawn();
        }
    }

    for (owner, data) in owners {
        let Ok(bar) = bars_data.get(data) else {
            continue;
        };
        let existing = bars.iter_mut().find(|(_, visual, _)| visual.owner == owner);
        match existing {
            Some((_, mut visual, children)) => {
                if visual.id != bar.id {
                    visual.id = bar.id;
                    visual.elapsed = 0.0;
                }
                visual.elapsed += dt;
                let ratio = (visual.elapsed / bar.delay.max(0.01)).min(1.0);
                let [r, g, b] = progress_color(ratio);
                let width = (END_X - START_X) * ratio;
                for child in children.iter() {
                    if let Ok((mut sprite, mut transform)) = fills.get_mut(child) {
                        sprite.color = Color::srgb(r, g, b);
                        sprite.custom_size = Some(Vec2::new(width.max(0.01), 1.0));
                        // Левый край заливки — StartX от левого края фона.
                        transform.translation.x = -BAR_WIDTH / 2.0 + START_X + width / 2.0;
                    }
                }
            }
            None => {
                let Some(sprite) = registry.get(BAR_RSI) else {
                    continue;
                };
                let mut background = Sprite::from_image(sprite.image.clone());
                background.texture_atlas = Some(TextureAtlas {
                    layout: sprite.layout.clone(),
                    index: sprite.index(0, 0),
                });
                commands.entity(owner).with_children(|parent| {
                    parent
                        .spawn((
                            DoAfterVisual {
                                owner,
                                id: bar.id,
                                elapsed: 0.0,
                            },
                            background,
                            Transform::from_xyz(0.0, BAR_Y, 0.6),
                        ))
                        .with_children(|bar_parent| {
                            bar_parent.spawn((
                                DoAfterFill,
                                Sprite {
                                    color: Color::srgb(0.75, 0.0, 0.0),
                                    custom_size: Some(Vec2::new(0.01, 1.0)),
                                    ..default()
                                },
                                // Строка y = 3..4 сверху у полоски 7 px — центр.
                                Transform::from_xyz(-BAR_WIDTH / 2.0 + START_X, 0.0, 0.01),
                            ));
                        });
                });
                tracing::info!(delay = bar.delay, "do-after bar shown");
            }
        }
    }
}
