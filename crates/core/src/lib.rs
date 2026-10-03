//! Общий игровой код Space Station R: компоненты, события, чистые системы,
//! загрузка прототипов.
//!
//! Правила (PLAN.md §6): крейт не знает о сети (`ssr-protocol`) и о движковых
//! плагинах клиента/сервера; компоненты — только данные, логика — в системах.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub const GAME_NAME: &str = "Space Station R";

pub mod rsi;

/// Скорость игрока, пикселей в секунду (сервер применяет ввод по ADR-3).
pub const PLAYER_MOVE_SPEED: f32 = 300.0;

/// Сервер-авторитарная позиция игрока (ADR-3). Реплицируется клиентам (T1.3).
///
/// Внутренний тип — массив, чтобы сериализация не зависела от фич bevy;
/// координаты сетки тайлов (`IVec2`, ADR-8) появятся в фазе 2.
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerPosition(pub [f32; 2]);
