//! Атмосфера (T4.3, lite): газ как числа на тайле — давление (кПа) и доля
//! кислорода. Диффузия между соседними тайлами, космос — сток (пробой =
//! утечка), низкое давление/O₂ = урон (сервер).
//!
//! Полная модель (моли, температуры, смеси газов) — не цель: числа на тайле,
//! которых достаточно для утечек и удушья.

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Давление, ниже которого начинается урон (кПа).
pub const LOW_PRESSURE_KPA: f32 = 20.0;
/// Доля кислорода, ниже которой начинается урон.
pub const LOW_OXYGEN: f32 = 0.16;
/// Урон в секунду при разгерметизации (T4.3).
pub const VACUUM_DAMAGE_PER_SECOND: i32 = 5;

/// Газ тайла: давление (кПа) и доля кислорода (0..1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Gas {
    pub pressure: f32,
    pub oxygen: f32,
}

impl Gas {
    /// Космос: пустота.
    pub const VACUUM: Gas = Gas {
        pressure: 0.0,
        oxygen: 0.0,
    };

    /// Стандартная атмосфера станции: 101.3 кПа, 21% O₂.
    pub const STATION: Gas = Gas {
        pressure: 101.3,
        oxygen: 0.21,
    };

    /// Опасен ли тайл для дыхания (T4.3).
    pub fn is_breathable(self) -> bool {
        self.pressure >= LOW_PRESSURE_KPA && self.oxygen >= LOW_OXYGEN
    }
}

/// Атмосфера чанка (реплицируется, T4.3): давление в кПа и кислород в
/// процентах по тайлам, порядок — как у [`crate::tiles::TileChunkData`].
#[derive(Component, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChunkAtmosphere {
    pub coords: (i32, i32),
    /// Давление, кПа (целые).
    pub pressure: Vec<u16>,
    /// Кислород, проценты 0..100.
    pub oxygen: Vec<u8>,
}

impl ChunkAtmosphere {
    /// Упаковка газа чанка в реплицируемый вид (квантование экономит трафик).
    pub fn pack(coords: (i32, i32), gas: &[Gas]) -> Self {
        Self {
            coords,
            pressure: gas
                .iter()
                .map(|g| g.pressure.max(0.0).round() as u16)
                .collect(),
            oxygen: gas
                .iter()
                .map(|g| (g.oxygen.clamp(0.0, 1.0) * 100.0).round() as u8)
                .collect(),
        }
    }

    /// Газ тайла по локальным координатам (None — вне чанка).
    pub fn at(&self, lx: u32, ly: u32) -> Option<Gas> {
        let size = crate::tiles::CHUNK_TILES;
        if lx >= size || ly >= size {
            return None;
        }
        let index = (ly * size + lx) as usize;
        Some(Gas {
            pressure: f32::from(*self.pressure.get(index)?),
            oxygen: f32::from(*self.oxygen.get(index)?) / 100.0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_round_trip_keeps_values() {
        let gas = [Gas::STATION, Gas::VACUUM];
        let mut full =
            vec![Gas::VACUUM; (crate::tiles::CHUNK_TILES * crate::tiles::CHUNK_TILES) as usize];
        full[0] = gas[0];
        full[1] = gas[1];
        let packed = ChunkAtmosphere::pack((0, 0), &full);
        let restored = packed.at(0, 0).expect("tile");
        assert!((restored.pressure - 101.0).abs() < 1.0);
        assert!((restored.oxygen - 0.21).abs() < 0.01);
        assert_eq!(packed.at(0, 1), Some(Gas::VACUUM));
        assert_eq!(packed.at(99, 99), None);
    }

    #[test]
    fn breathable_thresholds() {
        assert!(Gas::STATION.is_breathable());
        assert!(!Gas::VACUUM.is_breathable());
        assert!(
            !Gas {
                pressure: 101.3,
                oxygen: 0.05
            }
            .is_breathable()
        );
    }
}
