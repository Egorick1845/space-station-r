//! Одежда на персонаже (SS14-модель): слоты инвентаря-одежды и слои отрисовки.
//!
//! Порядок слоёв взят из `Resources/Prototypes/Entities/Mobs/Species/base.yml`
//! (снизу вверх), имена состояний — из `ClientClothingSystem.TemporarySlotMap`
//! (`equipped-INNERCLOTHING`, `equipped-FEET`, `equipped-HAND`, ...).

use serde::{Deserialize, Serialize};

/// Слот одежды: соответствует слоту инвентаря человека в SS14.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClothingSlot {
    Back,
    Pocket1,
    Pocket2,
    SuitStorage,
    Belt,
    Ears,
    Eyes,
    Gloves,
    Head,
    Id,
    Jumpsuit,
    Mask,
    Neck,
    OuterClothing,
    Shoes,
}

impl ClothingSlot {
    /// Все слоты (порядок — для UI и сериализации).
    pub const ALL: [ClothingSlot; 15] = [
        ClothingSlot::Pocket1,
        ClothingSlot::Pocket2,
        ClothingSlot::SuitStorage,
        ClothingSlot::Jumpsuit,
        ClothingSlot::Shoes,
        ClothingSlot::Gloves,
        ClothingSlot::Head,
        ClothingSlot::Mask,
        ClothingSlot::Eyes,
        ClothingSlot::Ears,
        ClothingSlot::Id,
        ClothingSlot::Belt,
        ClothingSlot::Back,
        ClothingSlot::OuterClothing,
        ClothingSlot::Neck,
    ];

    /// Имя слота строкой (как в каталоге предметов).
    pub fn id(self) -> &'static str {
        match self {
            ClothingSlot::Pocket1 => "pocket1",
            ClothingSlot::Pocket2 => "pocket2",
            ClothingSlot::SuitStorage => "suitstorage",
            ClothingSlot::Back => "back",
            ClothingSlot::Belt => "belt",
            ClothingSlot::Ears => "ears",
            ClothingSlot::Eyes => "eyes",
            ClothingSlot::Gloves => "gloves",
            ClothingSlot::Head => "head",
            ClothingSlot::Id => "id",
            ClothingSlot::Jumpsuit => "jumpsuit",
            ClothingSlot::Mask => "mask",
            ClothingSlot::Neck => "neck",
            ClothingSlot::OuterClothing => "outerClothing",
            ClothingSlot::Shoes => "shoes",
        }
    }

    /// Слот по строке каталога.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|slot| slot.id() == id)
    }

    /// Карман: в него влезают только мелкие предметы (`PocketableItemSize` = Small).
    pub fn is_pocket(self) -> bool {
        matches!(self, ClothingSlot::Pocket1 | ClothingSlot::Pocket2)
    }

    /// Разгрузка требует верхней одежды (`dependsOn: outerClothing` в SS14).
    pub fn needs_outer(self) -> bool {
        matches!(self, ClothingSlot::SuitStorage)
    }

    /// Слоты, которые ЗАВИСЯТ от этого (`dependsOn` в
    /// `human_inventory_template.yml`): снятие комбинезона стягивает карманы и
    /// ID, снятие верхней одежды — разгрузку. В сборке этот каскад делает
    /// `InventorySystem.TryUnequip` (`Equip.cs:458-465`).
    pub fn dependents(self) -> &'static [ClothingSlot] {
        match self {
            ClothingSlot::Jumpsuit => &[
                ClothingSlot::Pocket1,
                ClothingSlot::Pocket2,
                ClothingSlot::Id,
            ],
            ClothingSlot::OuterClothing => &[ClothingSlot::SuitStorage],
            _ => &[],
        }
    }

    /// Слой отрисовки: чем больше, тем выше (порядок `base.yml`).
    /// Части тела идут с шагом 1 (1.00 Groin … 1.22 Head), одежда вклинивается
    /// между ними как в движке.
    pub fn layer_z(self) -> f32 {
        match self {
            // Карманы и разгрузка — невидимые слоты: спрайта на теле не имеют.
            ClothingSlot::Pocket1 | ClothingSlot::Pocket2 | ClothingSlot::SuitStorage => 0.0,
            ClothingSlot::Jumpsuit => 1.08,
            ClothingSlot::Gloves => 1.13,
            ClothingSlot::Shoes => 1.14,
            ClothingSlot::Ears => 1.15,
            ClothingSlot::Eyes => 1.16,
            ClothingSlot::Id => 1.17,
            ClothingSlot::OuterClothing => 1.18,
            ClothingSlot::Belt => 1.19,
            ClothingSlot::Back => 1.20,
            ClothingSlot::Neck => 1.21,
            ClothingSlot::Head => 1.22,
            ClothingSlot::Mask => 1.225,
        }
    }

    /// Состояние RSI «надетым» (`equipped-INNERCLOTHING` и т.п.).
    pub fn equipped_state(self) -> &'static str {
        match self {
            ClothingSlot::Pocket1 | ClothingSlot::Pocket2 | ClothingSlot::SuitStorage => "",
            ClothingSlot::Head => "equipped-HELMET",
            ClothingSlot::Eyes => "equipped-EYES",
            ClothingSlot::Ears => "equipped-EARS",
            ClothingSlot::Mask => "equipped-MASK",
            ClothingSlot::OuterClothing => "equipped-OUTERCLOTHING",
            ClothingSlot::Jumpsuit => "equipped-INNERCLOTHING",
            ClothingSlot::Neck => "equipped-NECK",
            ClothingSlot::Back => "equipped-BACKPACK",
            ClothingSlot::Belt => "equipped-BELT",
            ClothingSlot::Gloves => "equipped-HAND",
            ClothingSlot::Shoes => "equipped-FEET",
            ClothingSlot::Id => "equipped-IDCARD",
        }
    }
}

/// Надетая одежда игрока: слот → bits предмета (реплицируется).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, bevy::prelude::Component)]
pub struct Clothing {
    /// Предметы по слотам (None — пусто).
    pub slots: Vec<(ClothingSlot, u64)>,
}

impl Clothing {
    /// Предмет в слоте.
    pub fn get(&self, slot: ClothingSlot) -> Option<u64> {
        self.slots
            .iter()
            .find(|(worn, _)| *worn == slot)
            .map(|(_, item)| *item)
    }

    /// Надеть предмет в слот (старый возвращается).
    pub fn equip(&mut self, slot: ClothingSlot, item: u64) -> Option<u64> {
        let previous = self.get(slot);
        self.slots.retain(|(worn, _)| *worn != slot);
        self.slots.push((slot, item));
        previous
    }

    /// Снять предмет из слота.
    pub fn unequip(&mut self, slot: ClothingSlot) -> Option<u64> {
        let item = self.get(slot);
        self.slots.retain(|(worn, _)| *worn != slot);
        item
    }

    /// Есть ли рюкзак (инвентарь доступен только с ним — правило владельца).
    pub fn has_backpack(&self) -> bool {
        self.get(ClothingSlot::Back).is_some()
    }

    /// Надет ли этот предмет (bits) в любой слот: нужно, чтобы понять, доступно
    /// ли хранилище предмета (`StorageComponent`) — носитель может открыть его
    /// без проверки расстояния (`StorageSystem`: владелец рядом всегда).
    pub fn contains(&self, item: u64) -> bool {
        self.slots.iter().any(|(_, worn)| *worn == item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Зависимые слоты — из `dependsOn` шаблона человека: карманы и ID держатся
    /// на комбинезоне, разгрузка — на верхней одежде. Снятие каскадом
    /// (`TryUnequip`) обязано опустошать и их.
    #[test]
    fn dependents_match_human_template() {
        assert_eq!(
            ClothingSlot::Jumpsuit.dependents(),
            &[
                ClothingSlot::Pocket1,
                ClothingSlot::Pocket2,
                ClothingSlot::Id
            ]
        );
        assert_eq!(
            ClothingSlot::OuterClothing.dependents(),
            &[ClothingSlot::SuitStorage]
        );
        for slot in [
            ClothingSlot::Head,
            ClothingSlot::Mask,
            ClothingSlot::Shoes,
            ClothingSlot::Back,
            ClothingSlot::Gloves,
            ClothingSlot::Pocket1,
        ] {
            assert!(slot.dependents().is_empty(), "{slot:?} не имеет зависимых");
        }
    }
}
