//! Стопки предметов — перенос из сборки mini-station-goob:
//! - `Content.Shared/Stacks/StackComponent.cs` (`Count`, реплицируется);
//! - `Content.Shared/Stacks/SharedStackSystem.API.cs` (`TryMergeStacks`:
//!   переносится `min(count донора, свободное место получателя)`);
//! - `Content.Shared/Stacks/SharedStackSystem.cs` (`DefaultSplitAmounts`,
//!   вербы «Разделить пополам» и количества в категории «Разделить»);
//! - `Resources/Locale/ru-RU/stack/stack-component.ftl` (тексты).
//!
//! Тип стопки — id предмета (у нас прототип стопки и предмета совпадают);
//! максимум — [`crate::items::ItemProto::max_stack`].

use bevy::prelude::Component;
use serde::{Deserialize, Serialize};

/// Количество в стопке (`StackComponent.Count`).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stack {
    pub count: u32,
}

/// `SharedStackSystem.DefaultSplitAmounts`.
pub const DEFAULT_SPLIT_AMOUNTS: [u32; 6] = [1, 5, 10, 20, 30, 50];

/// Итог слияния (`TryMergeStacks`): сколько перенесено и новые количества.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Merge {
    pub transferred: u32,
    pub donor: u32,
    pub recipient: u32,
}

/// `TryMergeStacks`: из донора в получателя, не больше свободного места.
/// `None` — ничего не перенесено (тогда в сборке нет и всплывашки).
pub fn merge(donor: u32, recipient: u32, max: u32) -> Option<Merge> {
    let space = max.saturating_sub(recipient);
    let transferred = donor.min(space);
    if transferred == 0 {
        return None;
    }
    Some(Merge {
        transferred,
        donor: donor - transferred,
        recipient: recipient + transferred,
    })
}

/// Варианты разделения (`OnStackAlternativeInteract`): «пополам» и
/// количества меньше текущего. Пусто для стопки из одного предмета.
pub fn split_options(count: u32) -> Vec<(String, u32)> {
    if count <= 1 {
        return Vec::new();
    }
    let mut options = vec![("Разделить пополам".to_string(), count / 2)];
    for amount in DEFAULT_SPLIT_AMOUNTS {
        if amount < count {
            options.push((amount.to_string(), amount));
        }
    }
    options
}

/// Строка статуса предмета в руке (`comp-stack-status`).
pub fn status_text(count: u32) -> String {
    format!("Количество: {count}")
}

/// Строка осмотра (`comp-stack-examine-detail-count`, русские формы числа).
pub fn examine_text(count: u32) -> String {
    let word = match (count % 10, count % 100) {
        (1, n) if n != 11 => "предмет",
        (2..=4, n) if !(12..=14).contains(&n) => "предмета",
        _ => "предметов",
    };
    format!("В стопке {count} {word}.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_is_limited_by_free_space() {
        assert_eq!(
            merge(10, 25, 30),
            Some(Merge {
                transferred: 5,
                donor: 5,
                recipient: 30
            })
        );
        assert_eq!(merge(3, 4, 30).unwrap().recipient, 7);
        assert_eq!(merge(3, 30, 30), None, "получатель полон");
        assert_eq!(merge(0, 1, 30), None);
    }

    #[test]
    fn split_options_follow_default_amounts() {
        assert!(split_options(1).is_empty());
        let options = split_options(30);
        assert_eq!(options[0], ("Разделить пополам".to_string(), 15));
        let amounts: Vec<u32> = options[1..].iter().map(|(_, a)| *a).collect();
        assert_eq!(amounts, vec![1, 5, 10, 20]);
    }

    #[test]
    fn examine_text_uses_russian_plurals() {
        assert_eq!(examine_text(1), "В стопке 1 предмет.");
        assert_eq!(examine_text(3), "В стопке 3 предмета.");
        assert_eq!(examine_text(12), "В стопке 12 предметов.");
        assert_eq!(examine_text(21), "В стопке 21 предмет.");
        assert_eq!(examine_text(30), "В стопке 30 предметов.");
    }
}
