//! Атмосфера (T4.3, lite): газ как числа на тайле — давление (кПа) и доля
//! кислорода. Диффузия между соседними тайлами, космос — сток (пробой =
//! утечка), низкое давление/O₂ = урон (сервер).
//!
//! Полная модель (моли, температуры, смеси газов) — не цель: числа на тайле,
//! которых достаточно для утечек и удушья.

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Пороги алертов давления (`Content.Shared/Atmos/Atmospherics.cs`).
/// В сборке алерт показывается только в опасной зоне: `BarotraumaSystem`
/// ставит severity 1 при предупреждении и 2 при уроне, иначе снимает категорию
/// «Pressure» целиком — постоянного индикатора давления в HUD нет.
pub mod pressure_alerts {
    /// Предупреждение о низком давлении: 2.5 × опасного порога.
    pub const WARNING_LOW_KPA: f32 = 2.5 * HAZARD_LOW_KPA;
    /// Опасное низкое давление (`HazardLowPressure`).
    pub const HAZARD_LOW_KPA: f32 = 20.0;
    /// Предупреждение о высоком давлении: 0.7 × опасного порога.
    pub const WARNING_HIGH_KPA: f32 = 0.7 * HAZARD_HIGH_KPA;
    /// Опасное высокое давление (`HazardHighPressure`).
    pub const HAZARD_HIGH_KPA: f32 = 550.0;

    /// Алерт давления для значения: `None` — безопасно (иконка не показывается),
    /// иначе `(высокое?, уровень 1..2)` — состояние иконки low/high pressure1|2.
    pub fn alert_for(pressure: f32) -> Option<(bool, u8)> {
        if pressure <= HAZARD_LOW_KPA {
            Some((false, 2))
        } else if pressure >= HAZARD_HIGH_KPA {
            Some((true, 2))
        } else if pressure <= WARNING_LOW_KPA {
            Some((false, 1))
        } else if pressure >= WARNING_HIGH_KPA {
            Some((true, 1))
        } else {
            None
        }
    }
}

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

    #[test]
    fn pressure_alerts_follow_barotrauma_thresholds() {
        use pressure_alerts::{
            HAZARD_HIGH_KPA, HAZARD_LOW_KPA, WARNING_HIGH_KPA, WARNING_LOW_KPA, alert_for,
        };
        // Норма станции (101.3 кПа) — иконки нет; сразу за предупреждающей
        // полосой (50+) и до высокой (385) — тоже безопасно.
        assert_eq!(alert_for(101.3), None);
        assert_eq!(alert_for(WARNING_LOW_KPA + 1.0), None);
        assert_eq!(alert_for(WARNING_HIGH_KPA - 1.0), None);
        // За опасным низким порогом (20+), но ниже предупреждения — уровень 1.
        assert_eq!(alert_for(HAZARD_LOW_KPA + 1.0), Some((false, 1)));
        // Предупреждения (уровень 1) и урон (уровень 2).
        assert_eq!(alert_for(WARNING_LOW_KPA), Some((false, 1)));
        assert_eq!(alert_for(HAZARD_LOW_KPA), Some((false, 2)));
        assert_eq!(alert_for(WARNING_HIGH_KPA), Some((true, 1)));
        assert_eq!(alert_for(HAZARD_HIGH_KPA), Some((true, 2)));
        // Космос.
        assert_eq!(alert_for(0.0), Some((false, 2)));
    }
}
