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
    pub const ALL: [ClothingSlot; 12] = [
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

    /// Слой отрисовки: чем больше, тем выше (порядок `base.yml`).
    /// Части тела идут с шагом 1 (1.00 Groin … 1.22 Head), одежда вклинивается
    /// между ними как в движке.
    pub fn layer_z(self) -> f32 {
        match self {
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
}
