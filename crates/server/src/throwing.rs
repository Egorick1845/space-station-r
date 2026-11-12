//! Полёт брошенного предмета на сервере (Ctrl+Q, `ThrownItemSystem` сборки):
//! числа и формулы — в [`ssr_core::throwing`]. У предметов нет физических тел,
//! поэтому движение интегрируется здесь с подшагами (без проскока сквозь
//! стену): в полёте предмет останавливается о твёрдый тайл, закрытую дверь
//! или моба (кроме бросившего — `PreventCollision`), после `LandTime` —
//! скользит по полу с трением тайла и останавливается у курсора.

use bevy::prelude::*;
use ssr_core::inventory::ItemPosition;
use ssr_core::throwing::{self, Throw};
use ssr_core::tiles::TileChunkData;
use ssr_core::{Door, PlayerPosition};

use crate::{MapIndex, TILE_SIZE};

/// Брошенный предмет (`ThrownItemComponent`): только сервер, клиент видит
/// полёт по обновлениям `ItemPosition`.
#[derive(Component, Debug)]
pub struct Thrown {
    /// Направление (единичное).
    dir: [f32; 2],
    /// Скорость, тайлов/с.
    speed: f32,
    /// Осталось лететь до `LandTime`, с.
    fly_left: f32,
    /// Бросивший — его предмет не задевает.
    thrower: Entity,
}

impl Thrown {
    pub fn new(plan: Throw, thrower: Entity) -> Self {
        Self {
            dir: plan.dir,
            speed: plan.speed,
            fly_left: plan.fly_time,
            thrower,
        }
    }

    fn landed(&self) -> bool {
        self.fly_left <= 0.0
    }
}

/// Пауза между бросками (`HandsComponent.NextThrowTime`): игрок → время, с
/// которого можно бросать снова.
#[derive(Resource, Default)]
pub struct ThrowCooldowns(pub std::collections::HashMap<Entity, f32>);

/// Твёрдый ли тайл в точке мира (живые чанки: построенные стены тоже).
fn solid_at(index: &MapIndex, chunks: &Query<&TileChunkData>, x: f32, y: f32) -> bool {
    let size = ssr_core::tiles::CHUNK_TILES as i32;
    let tx = (x / TILE_SIZE).floor() as i32;
    let ty = (y / TILE_SIZE).floor() as i32;
    let coords = (tx.div_euclid(size), ty.div_euclid(size));
    let Some(chunk) = index.chunks.get(&coords).and_then(|e| chunks.get(*e).ok()) else {
        return true; // за картой — препятствие
    };
    let lx = tx.rem_euclid(size) as usize;
    let ly = ty.rem_euclid(size) as usize;
    ssr_core::occluders::is_solid(chunk.tiles[ly * size as usize + lx])
}

/// Подшаги и столкновения брошенных предметов.
pub fn move_thrown(
    mut commands: Commands,
    time: Res<Time>,
    index: Res<MapIndex>,
    chunks: Query<&TileChunkData>,
    doors: Query<&Door>,
    mobs: Query<(Entity, &PlayerPosition)>,
    mut thrown: Query<(Entity, &mut Thrown, &mut ItemPosition)>,
) {
    let dt = time.delta_secs();
    for (entity, mut item, mut position) in thrown.iter_mut() {
        // Подшаги не длиннее четверти тайла — предмет не проскакивает стену.
        let step_len = item.speed * TILE_SIZE * dt;
        let steps = (step_len / (TILE_SIZE * 0.25)).ceil().max(1.0) as u32;
        let sub_dt = dt / steps as f32;
        let mut stopped = false;
        for _ in 0..steps {
            let landed = item.landed();
            let next = [
                position.0[0] + item.dir[0] * item.speed * TILE_SIZE * sub_dt,
                position.0[1] + item.dir[1] * item.speed * TILE_SIZE * sub_dt,
            ];
            let blocked = solid_at(&index, &chunks, next[0], next[1])
                || doors.iter().any(|door| {
                    !door.open
                        && (door.position[0] - next[0]).abs() < TILE_SIZE * 0.5
                        && (door.position[1] - next[1]).abs() < TILE_SIZE * 0.5
                });
            if blocked {
                stopped = true;
                tracing::info!(item = ?entity, "thrown item hit an obstacle");
                break;
            }
            position.0 = next;
            // Моб (не бросивший) — только в полёте (`StartCollideEvent`
            // приходит, пока висит `ThrownItemComponent`).
            if !landed
                && let Some((target, _)) = mobs.iter().find(|(mob, at)| {
                    *mob != item.thrower
                        && (at.0[0] - next[0]).powi(2) + (at.0[1] - next[1]).powi(2)
                            < (TILE_SIZE * 0.5).powi(2)
                })
            {
                stopped = true;
                tracing::info!(item = ?entity, ?target, "thrown item hit a mob");
                break;
            }
            item.speed = throwing::damp(item.speed, landed, sub_dt);
            item.fly_left -= sub_dt;
            if item.speed <= 0.0 {
                stopped = true;
                break;
            }
        }
        if stopped {
            commands.entity(entity).remove::<Thrown>();
            tracing::info!(item = ?entity, at = ?position.0, "thrown item stopped");
        }
    }
}

/// Тест `SSR_THROW_TEST=1`: у первого игрока в активной руке всегда лом
/// (клиентский тест того же имени бросает его каждые 2 с), раз в 0.25 с —
/// лог позиции и скорости игрока: видно, мешает ли бросок движению.
pub fn throw_test(
    mut commands: Commands,
    time: Res<Time>,
    mut next: Local<f32>,
    players: Res<crate::Players>,
    mut hands: Query<&mut ssr_core::inventory::Hands>,
    bodies: Query<(&PlayerPosition, &avian2d::prelude::LinearVelocity)>,
) {
    if std::env::var_os("SSR_THROW_TEST").is_none() {
        return;
    }
    let Some(player) = players.entries.first().map(|entry| entry.player) else {
        return;
    };
    if let Ok(mut hand) = hands.get_mut(player)
        && hand.active_item().is_none()
    {
        let crowbar = commands
            .spawn((
                ssr_core::inventory::Item {
                    name: "Crowbar".to_string(),
                },
                ssr_core::inventory::HeldBy {
                    player: player.to_bits(),
                },
                lightyear::prelude::Replicate::to_clients(lightyear::prelude::NetworkTarget::All),
                lightyear::prelude::Rooms::default(),
            ))
            .id();
        hand.take_in_active(crowbar.to_bits());
    }
    *next += time.delta_secs();
    if *next < 0.25 {
        return;
    }
    *next = 0.0;
    if let Ok((position, velocity)) = bodies.get(player) {
        tracing::info!(position = ?position.0, velocity = ?[velocity.x, velocity.y], "throw-test: player");
    }
}
