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

/// Рук у персонажа (как в SS14: две руки, активная переключается).
pub const HAND_SLOTS: usize = 2;

/// Руки персонажа (SS14-модель): активная рука + предметы в руках.
/// Действия (атака, инструменты, стройка) идут через АКТИВНУЮ руку (ADR-3).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hands {
    pub active: u8,
    pub slots: Vec<Option<u64>>,
}

impl Default for Hands {
    fn default() -> Self {
        Self {
            active: 0,
            slots: vec![None; HAND_SLOTS],
        }
    }
}

impl Hands {
    /// Предмет в активной руке.
    pub fn active_item(&self) -> Option<u64> {
        self.slots.get(self.active as usize).copied().flatten()
    }

    /// Переключить активную руку.
    pub fn switch(&mut self) {
        self.active = (self.active + 1) % HAND_SLOTS as u8;
    }

    /// Взять предмет в активную руку (если рука свободна).
    pub fn take_in_active(&mut self, item: u64) -> bool {
        let index = self.active as usize;
        if self.slots.get(index).copied().flatten().is_some() {
            return false;
        }
        self.slots[index] = Some(item);
        true
    }

    /// Забрать предмет из любой руки.
    pub fn take(&mut self, item: u64) -> bool {
        match self.slots.iter().position(|s| *s == Some(item)) {
            Some(index) => {
                self.slots[index] = None;
                true
            }
            None => false,
        }
    }

    pub fn has(&self, item: u64) -> bool {
        self.slots.contains(&Some(item))
    }

    pub fn item_in_hand(&self, hand: u8) -> Option<u64> {
        self.slots.get(hand as usize).copied().flatten()
    }
}

/// Здоровье (T4.1, минимум для атаки): урон снижает, смерть — возврат на спавн.
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Health {
    pub current: i32,
    pub max: i32,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            current: 100,
            max: 100,
        }
    }
}

impl Health {
    /// Нанести урон; true — цель погибла.
    pub fn damage(&mut self, amount: i32) -> bool {
        self.current = (self.current - amount).max(0);
        self.current == 0
    }
}

/// Предмет удерживается игроком (руки/рюкзак): реплицируется, чтобы клиент
/// рисовал предмет в руке владельца (SS14-модель, T3.3+).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct HeldBy {
    /// bits серверной сущности игрока; `0` — предмет ничей (в мире).
    pub player: u64,
}

/// Мировая позиция предмета или контейнера (реплицируется; T3.4).
#[derive(Component, Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq)]
pub struct ItemPosition(pub [f32; 2]);

/// Контейнер (ящик/шкаф, T3.4): состояние «открыт» меняет только сервер (ADR-3).
#[derive(Component, Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Container {
    pub open: bool,
    pub name: String,
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
    fn hands_basic() {
        let mut hands = Hands::default();
        assert_eq!(hands.active_item(), None);
        assert!(hands.take_in_active(1));
        assert!(!hands.take_in_active(2), "активная рука занята");
        hands.switch();
        assert!(hands.take_in_active(2));
        assert_eq!(hands.active_item(), Some(2));
        assert!(hands.has(1) && hands.has(2));
        assert!(hands.take(1));
        assert!(!hands.has(1));
        hands.switch();
        assert_eq!(hands.active_item(), None);
    }

    #[test]
    fn health_damage() {
        let mut health = Health::default();
        assert!(!health.damage(40));
        assert_eq!(health.current, 60);
        assert!(health.damage(999));
        assert_eq!(health.current, 0);
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
