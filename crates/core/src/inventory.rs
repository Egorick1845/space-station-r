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

/// Размеры предметов в клетках инвентаря (тетрис-сетка, как в SS14):
/// (ширина, высота). Неизвестные предметы — 1×1.
pub fn item_size(name: &str) -> (u8, u8) {
    match name {
        // Лом в SS14 длинный: 2 клетки по горизонтали.
        "Crowbar" => (2, 1),
        "SteelSheet" => (1, 1),
        _ => (1, 1),
    }
}

/// Инвентарь-сетка (SS14-тетрис): в каждой клетке — bits предмета, который её
/// занимает; предмет размера w×h заполняет w·h клеток (ADR-3, реплицируется).
#[derive(Component, Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Inventory {
    /// Клетки построчно (7×4): None — пусто.
    pub cells: Vec<Option<u64>>,
}

#[allow(clippy::derivable_impls)]
impl Default for Inventory {
    fn default() -> Self {
        Self::new()
    }
}

impl Inventory {
    pub fn new() -> Self {
        Self {
            cells: vec![None; INVENTORY_SLOTS],
        }
    }

    pub fn contains(&self, item: u64) -> bool {
        self.cells.contains(&Some(item))
    }

    /// Клетки, занятые предметом.
    pub fn cells_of(&self, item: u64) -> Vec<usize> {
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| **cell == Some(item))
            .map(|(index, _)| index)
            .collect()
    }

    /// Клетка-якорь предмета (верхняя левая из занятых).
    pub fn anchor_of(&self, item: u64) -> Option<u8> {
        self.cells_of(item)
            .into_iter()
            .min()
            .map(|index| index as u8)
    }

    /// Подходит ли позиция для предмета w×h (все клетки пусты и внутри сетки).
    fn fits(&self, w: u8, h: u8, index: u8) -> bool {
        let x = index % INVENTORY_COLS;
        let y = index / INVENTORY_COLS;
        if x + w > INVENTORY_COLS || y + h > INVENTORY_ROWS {
            return false;
        }
        (0..h).all(|dy| {
            (0..w).all(|dx| {
                let cell = (y + dy) * INVENTORY_COLS + (x + dx);
                self.cells.get(cell as usize).copied().flatten().is_none()
            })
        })
    }

    /// Позиция для предмета w×h: конкретная клетка (если задана) или первая
    /// подходящая (перебор построчно).
    pub fn find_place(&self, w: u8, h: u8, at: Option<u8>) -> Option<u8> {
        match at {
            Some(index) => self.fits(w, h, index).then_some(index),
            None => (0..INVENTORY_SLOTS as u8).find(|index| self.fits(w, h, *index)),
        }
    }

    /// Кладёт предмет в клетку-якорь (занимает w×h клеток).
    pub fn place(&mut self, item: u64, w: u8, h: u8, index: u8) -> bool {
        if !self.fits(w, h, index) {
            return false;
        }
        let x = index % INVENTORY_COLS;
        let y = index / INVENTORY_COLS;
        for dy in 0..h {
            for dx in 0..w {
                let cell = (y + dy) * INVENTORY_COLS + (x + dx);
                self.cells[cell as usize] = Some(item);
            }
        }
        true
    }

    /// Убирает предмет со всех клеток.
    pub fn take(&mut self, item: u64) -> bool {
        let mut taken = false;
        for cell in self.cells.iter_mut() {
            if *cell == Some(item) {
                *cell = None;
                taken = true;
            }
        }
        taken
    }

    /// Кладёт предмет в первую подходящую позицию; возвращает клетку-якорь.
    pub fn put_first_fit(&mut self, item: u64, w: u8, h: u8) -> Option<u8> {
        let index = self.find_place(w, h, None)?;
        self.place(item, w, h, index).then_some(index)
    }
}

/// Раскладка предметов в сетке для отрисовки «тетрисом»: для каждого предмета
/// (bits, x, y, w, h) — ограничивающий прямоугольник занятых им клеток.
pub fn item_layout(cells: &[Option<u64>]) -> Vec<(u64, u8, u8, u8, u8)> {
    let mut seen: Vec<u64> = Vec::new();
    let mut layout = Vec::new();
    for cell in cells.iter().flatten() {
        if seen.contains(cell) {
            continue;
        }
        seen.push(*cell);
        let coords: Vec<(u8, u8)> = cells
            .iter()
            .enumerate()
            .filter(|(_, value)| **value == Some(*cell))
            .map(|(index, _)| {
                let index = index as u8;
                (index % INVENTORY_COLS, index / INVENTORY_COLS)
            })
            .collect();
        let x = coords.iter().map(|c| c.0).min().unwrap_or(0);
        let y = coords.iter().map(|c| c.1).min().unwrap_or(0);
        let w = coords.iter().map(|c| c.0).max().unwrap_or(0) - x + 1;
        let h = coords.iter().map(|c| c.1).max().unwrap_or(0) - y + 1;
        layout.push((*cell, x, y, w, h));
    }
    layout
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

    /// Полностью вылечить — дебаг-верб «Оживить» (`rejuvenate` в сборке лечит
    /// и снимает станы; у нас здоровье и `KnockedDown`).
    pub fn heal(&mut self) {
        self.current = self.max;
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
    fn inventory_grid_places_and_takes() {
        let mut inv = Inventory::new();
        assert_eq!(inv.cells.len(), INVENTORY_SLOTS);
        // Лом 2×1 занимает две клетки.
        assert_eq!(inv.put_first_fit(42, 2, 1), Some(0));
        assert!(inv.contains(42));
        assert_eq!(inv.cells_of(42), vec![0, 1]);
        assert_eq!(inv.anchor_of(42), Some(0));
        // Следующий предмет кладётся после лома.
        assert_eq!(inv.put_first_fit(7, 1, 1), Some(2));
        // Явная позиция занята — отказ.
        assert_eq!(inv.find_place(1, 1, Some(0)), None);
        assert!(inv.take(42));
        assert_eq!(inv.anchor_of(42), None);
        // После освобождения первая позиция снова доступна.
        assert_eq!(inv.put_first_fit(9, 1, 1), Some(0));
        assert!(!inv.take(42), "повторное изъятие не проходит");
    }

    #[test]
    fn inventory_grid_size_respected() {
        let mut inv = Inventory::new();
        // 2×1 не влезает в последний столбец, но влезает в первый.
        let last_in_row = INVENTORY_COLS - 1;
        assert_eq!(inv.find_place(2, 1, Some(last_in_row)), None);
        assert_eq!(inv.put_first_fit(5, 2, 1), Some(0));
        // 1×1 встаёт в оставшуюся клетку второго ряда? Нет — рядом с ломом.
        assert_eq!(inv.find_place(1, 1, Some(1)), None);
        assert_eq!(inv.find_place(8, 1, None), None, "шире сетки — не влезает");
        assert_eq!(inv.find_place(7, 5, None), None, "выше сетки — не влезает");
        assert_eq!(
            inv.find_place(7, 2, None),
            Some(7),
            "вся ширина во втором ряду"
        );
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
            assert!(inv.put_first_fit(i, 1, 1).is_some());
        }
        assert_eq!(inv.put_first_fit(1234, 1, 1), None);
        // Освободили две клетки подряд — лом снова влезает.
        assert!(inv.take(0));
        assert!(inv.take(1));
        assert_eq!(inv.put_first_fit(1234, 2, 1), Some(0));
    }

    #[test]
    fn item_sizes_from_names() {
        assert_eq!(item_size("Crowbar"), (2, 1));
        assert_eq!(item_size("SteelSheet"), (1, 1));
        assert_eq!(item_size("Unknown"), (1, 1));
    }
}
