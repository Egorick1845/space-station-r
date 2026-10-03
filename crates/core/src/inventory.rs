//! Инвентарь (PLAN.md T3.2, ADR-3: содержимое меняет только сервер).
//!
//! Предметы — сущности с компонентом [`Item`]; инвентарь хранит их
//! идентификаторы (bits серверной сущности, ADR-7), поэтому состояние
//! реплицируется как простой список без ссылок на сетевые сущности.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

/// Колонок в сетке инвентаря (PLAN.md T3.2: слоты 7×4).
pub const INVENTORY_COLS: u8 = 7;
/// Строк в сетке инвентаря.
pub const INVENTORY_ROWS: u8 = 4;
/// Всего слотов.
pub const INVENTORY_SLOTS: usize = INVENTORY_COLS as usize * INVENTORY_ROWS as usize;

/// Слот «первый свободный» для запросов перемещения.
pub const SLOT_ANY: u8 = u8::MAX;

/// Инвентарь: по слоту на предмет; None — пусто (ADR-3, реплицируется).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inventory {
    pub slots: Vec<Option<u64>>,
}

impl Default for Inventory {
    fn default() -> Self {
        Self::new()
    }
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            slots: vec![None; INVENTORY_SLOTS],
        }
    }

    /// Первый свободный слот.
    pub fn first_empty(&self) -> Option<u8> {
        self.slots
            .iter()
            .position(|s| s.is_none())
            .map(|index| index as u8)
    }

    /// Слот, в котором лежит предмет.
    pub fn slot_of(&self, item: u64) -> Option<u8> {
        self.slots
            .iter()
            .position(|s| *s == Some(item))
            .map(|index| index as u8)
    }

    pub fn contains(&self, item: u64) -> bool {
        self.slot_of(item).is_some()
    }

    /// Забрать предмет (для переноса/выброса).
    pub fn take(&mut self, item: u64) -> bool {
        match self.slot_of(item) {
            Some(slot) => {
                self.slots[slot as usize] = None;
                true
            }
            None => false,
        }
    }

    /// Положить предмет в конкретный слот (только в пустой и в пределах сетки).
    pub fn put(&mut self, slot: u8, item: u64) -> bool {
        let index = slot as usize;
        if index >= self.slots.len() || self.slots[index].is_some() {
            return false;
        }
        self.slots[index] = Some(item);
        true
    }

    /// Положить в первый свободный слот; возвращает номер слота.
    pub fn put_first_empty(&mut self, item: u64) -> Option<u8> {
        let slot = self.first_empty()?;
        self.put(slot, item).then_some(slot)
    }
}

/// Предмет-сущность (T3.2): имя до появления прототипов в игре (T5.2).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Item {
    pub name: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_basic_ops() {
        let mut inv = Inventory::new();
        assert_eq!(inv.slots.len(), INVENTORY_SLOTS);
        assert_eq!(inv.first_empty(), Some(0));

        assert!(inv.put(3, 42));
        assert!(inv.contains(42));
        assert_eq!(inv.slot_of(42), Some(3));
        // Занятый слот не перезаписывается.
        assert!(!inv.put(3, 99));
        // Вне сетки — отказ.
        assert!(!inv.put(INVENTORY_SLOTS as u8, 99));

        assert_eq!(inv.put_first_empty(7), Some(0));
        assert_eq!(inv.first_empty(), Some(1));

        assert!(inv.take(42));
        assert!(!inv.contains(42));
        assert_eq!(inv.first_empty(), Some(1));
        assert!(!inv.take(42), "повторное изъятие не проходит");
    }

    #[test]
    fn inventory_full() {
        let mut inv = Inventory::new();
        for i in 0..INVENTORY_SLOTS as u64 {
            assert!(inv.put_first_empty(i).is_some());
        }
        assert_eq!(inv.first_empty(), None);
        assert_eq!(inv.put_first_empty(1234), None);
    }
}
