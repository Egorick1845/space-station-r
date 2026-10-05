//! Вербы — контекстные действия по ПКМ, перенос `Content.Shared/Verbs` из сборки.
//!
//! Источники: `Content.Shared/Verbs/Verb.cs` (поля и порядок сортировки
//! `CompareTo:169-207`, типы `VerbTypes:218-229`), `VerbCategory.cs:47-107`
//! (категории: текст локализации, иконка, `IconsOnly`, `Columns`),
//! `SharedVerbSystem.cs` (сбор вербов и исполнение).

use serde::{Deserialize, Serialize};

/// Тип верба — задаёт `TypePriority` и стиль шрифта в меню
/// (`Verb.cs:218-364`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VerbType {
    /// `InnateVerb` = −1 (правка Goobstation: «EE Verb on the Bottom»).
    Innate,
    /// Обычный `Verb` = 0.
    #[default]
    Verb,
    /// `ExamineVerb` = 0, `CloseMenuDefault = false`.
    Examine,
    /// `ActivationVerb` = 1 (стиль `ActivationVerb`).
    Activation,
    /// `AlternativeVerb` = 2 (стиль `AlternativeVerb`).
    Alternative,
    /// `UtilityVerb` = 3 (стиль `InteractionVerb`).
    Utility,
    /// `InteractionVerb` = 4 (стиль `InteractionVerb`, contact interaction).
    Interaction,
    /// `EquipmentVerb` = 5.
    Equipment,
    /// `VvVerb` = `int.MaxValue` — всегда первый.
    ViewVariables,
}

impl VerbType {
    /// `TypePriority` из `Verb.cs` (больше — выше в меню).
    pub fn priority(self) -> i32 {
        match self {
            VerbType::Innate => -1,
            VerbType::Verb | VerbType::Examine => 0,
            VerbType::Activation => 1,
            VerbType::Alternative => 2,
            VerbType::Utility => 3,
            VerbType::Interaction => 4,
            VerbType::Equipment => 5,
            VerbType::ViewVariables => i32::MAX,
        }
    }

    /// Класс стиля текста (`TextStyleClass`): различие типов видно только
    /// шрифтом (`ContextMenuSheetlet.cs:55-65`).
    pub fn style_class(self) -> &'static str {
        match self {
            VerbType::Interaction | VerbType::Utility => "InteractionVerb",
            VerbType::Activation => "ActivationVerb",
            VerbType::Alternative => "AlternativeVerb",
            _ => "Verb",
        }
    }

    /// Имя типа как в `Verb.VerbTypes` сборки (для сети и консольных команд).
    pub fn name(self) -> &'static str {
        match self {
            VerbType::Innate => "InnateVerb",
            VerbType::Verb => "Verb",
            VerbType::Examine => "ExamineVerb",
            VerbType::Activation => "ActivationVerb",
            VerbType::Alternative => "AlternativeVerb",
            VerbType::Utility => "UtilityVerb",
            VerbType::Interaction => "InteractionVerb",
            VerbType::Equipment => "EquipmentVerb",
            VerbType::ViewVariables => "VvVerb",
        }
    }
}

/// Категория вербов (`VerbCategory.cs:47-107`): текст, иконка, `IconsOnly`
/// (рисуются только иконки) и число колонок в подменю.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerbCategory {
    Admin,
    Antag,
    Examine,
    Debug,
    Eject,
    Insert,
    Buckle,
    Unbuckle,
    Rotate,
    Smite,
    Tricks,
    SelectType,
    Switch,
    Interaction,
    SetTransferAmount,
}

impl VerbCategory {
    pub fn text(self) -> &'static str {
        match self {
            VerbCategory::Admin => "Админ",
            VerbCategory::Antag => "Антаг",
            VerbCategory::Examine => "Осмотреть",
            VerbCategory::Debug => "Дебаг",
            VerbCategory::Eject => "Извлечь",
            VerbCategory::Insert => "Вставить",
            VerbCategory::Buckle => "Пристегнуть",
            VerbCategory::Unbuckle => "Отстегнуть",
            VerbCategory::Rotate => "Повернуть",
            VerbCategory::Smite => "Кара",
            VerbCategory::Tricks => "Трюки",
            VerbCategory::SelectType => "Выбрать тип",
            VerbCategory::Switch => "Переключить",
            VerbCategory::Interaction => "Взаимодействие",
            VerbCategory::SetTransferAmount => "Объём переноса",
        }
    }

    /// `IconsOnly` — члены категории рисуются только иконками (`VerbCategory.cs:31`).
    pub fn icons_only(self) -> bool {
        matches!(
            self,
            VerbCategory::Antag | VerbCategory::Rotate | VerbCategory::Smite | VerbCategory::Tricks
        )
    }

    /// `Columns` — число колонок подменю.
    pub fn columns(self) -> u32 {
        match self {
            VerbCategory::Antag | VerbCategory::Rotate | VerbCategory::Tricks => 5,
            VerbCategory::Smite => 6,
            _ => 1,
        }
    }

    /// Иконка категории (RSI `#state` или PNG) — как в `VerbCategory.cs:47-107`.
    pub fn icon(self) -> Option<&'static str> {
        match self {
            VerbCategory::Admin => Some("Interface/VerbIcons/character.svg.192dpi.png"),
            VerbCategory::Examine => Some("Interface/VerbIcons/examine.svg.192dpi.png"),
            VerbCategory::Debug => Some("Interface/VerbIcons/debug.svg.192dpi.png"),
            VerbCategory::Rotate => Some("Interface/VerbIcons/refresh.svg.192dpi.png"),
            VerbCategory::SetTransferAmount => Some("Interface/VerbIcons/spill.svg.192dpi.png"),
            VerbCategory::Switch => Some("Interface/VerbIcons/group.svg.192dpi.png"),
            VerbCategory::Tricks => Some("Interface/AdminActions/tricks.png"),
            _ => None,
        }
    }

    /// Все категории (для меню админа/дебага — `VerbCategory.cs:47-107`).
    pub const ALL: [VerbCategory; 15] = [
        VerbCategory::Admin,
        VerbCategory::Antag,
        VerbCategory::Examine,
        VerbCategory::Debug,
        VerbCategory::Eject,
        VerbCategory::Insert,
        VerbCategory::Buckle,
        VerbCategory::Unbuckle,
        VerbCategory::Rotate,
        VerbCategory::Smite,
        VerbCategory::Tricks,
        VerbCategory::SelectType,
        VerbCategory::Switch,
        VerbCategory::Interaction,
        VerbCategory::SetTransferAmount,
    ];
}

/// Один верб — контекстное действие. Поля и дефолты из `Verb.cs:19-151`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verb {
    /// Текст (`verb.Text`).
    pub text: String,
    /// Тип верба (`TypePriority` + стиль).
    #[serde(default)]
    pub kind: VerbType,
    /// Категория (подменю); `None` — верхний уровень.
    #[serde(default)]
    pub category: Option<VerbCategory>,
    /// Иконка (`SpriteSpecifier`: `path#state` или `.png`).
    #[serde(default)]
    pub icon: Option<String>,
    /// Иконка-сущность (`IconEntity`) — рисуется спрайтом сущности.
    #[serde(default)]
    pub icon_entity: Option<u64>,
    /// Приоритет внутри типа: больше — выше (`Priority`, `Verb.cs:112`).
    #[serde(default)]
    pub priority: i32,
    /// Верб недоступен: серый элемент, причина в `message` (`Disabled`).
    #[serde(default)]
    pub disabled: bool,
    /// Причина недоступности / подсказка (`Message`, `Verb.cs:103`).
    #[serde(default)]
    pub message: Option<String>,
    /// Закрывать ли меню после исполнения (`CloseMenu`; у `ExamineVerb`
    /// `CloseMenuDefault = false`, `Verb.cs:127-129,348`).
    #[serde(default)]
    pub close_menu: Option<bool>,
    /// Выполняется на клиенте (`ClientExclusive`, `Verb.cs:68`).
    #[serde(default)]
    pub client_exclusive: bool,
    /// Требуется подтверждение (`ConfirmationPopup`, `Verb.cs:142`).
    #[serde(default)]
    pub confirmation_popup: bool,
    /// Действие (индекс в списке действий сервера) — по нему сервер исполняет верб.
    #[serde(default)]
    pub action_index: Option<usize>,
}

impl Verb {
    pub fn new(text: impl Into<String>, kind: VerbType) -> Self {
        Self {
            text: text.into(),
            kind,
            category: None,
            icon: None,
            icon_entity: None,
            priority: 0,
            disabled: false,
            message: None,
            close_menu: None,
            client_exclusive: false,
            confirmation_popup: false,
            action_index: None,
        }
    }

    pub fn category(mut self, category: VerbCategory) -> Self {
        self.category = Some(category);
        self
    }

    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }

    pub fn disabled(mut self, message: impl Into<String>) -> Self {
        self.disabled = true;
        self.message = Some(message.into());
        self
    }

    pub fn action(mut self, index: usize) -> Self {
        self.action_index = Some(index);
        self
    }

    /// Закрывать ли меню: `CloseMenu ?? CloseMenuDefault`
    /// (`ExamineVerb.CloseMenuDefault => false`, `Verb.cs:129,348`).
    pub fn closes_menu(&self) -> bool {
        self.close_menu
            .unwrap_or(!matches!(self.kind, VerbType::Examine))
    }

    /// Стиль текста (`TextStyleClass`).
    pub fn style_class(&self) -> &'static str {
        self.kind.style_class()
    }

    /// Порядок как `Verb.CompareTo` (`Verb.cs:169-207`): тип (убыв.), приоритет
    /// (убыв.), категория (без категории — ПЕРВЫМИ, затем по алфавиту), текст,
    /// иконка-сущность, иконка.
    pub fn sort_key(&self) -> (i32, i32, u8, &str, Option<u64>, &str) {
        (
            -self.kind.priority(),
            -self.priority,
            u8::from(self.category.is_some()),
            self.category.map(VerbCategory::text).unwrap_or(""),
            self.icon_entity.map(|_| 1),
            self.icon.as_deref().unwrap_or(""),
        )
    }
}

/// Сортирует вербы в порядке движка (эквивалент `SortedSet<Verb>`).
pub fn sort_verbs(verbs: &mut [Verb]) {
    verbs.sort_by(|left, right| {
        left.sort_key()
            .cmp(&right.sort_key())
            .then_with(|| left.text.cmp(&right.text))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Приоритеты типов — ровно как в `Verb.cs:218-364`.
    #[test]
    fn type_priorities_match_engine() {
        assert_eq!(VerbType::Equipment.priority(), 5);
        assert_eq!(VerbType::Interaction.priority(), 4);
        assert_eq!(VerbType::Utility.priority(), 3);
        assert_eq!(VerbType::Alternative.priority(), 2);
        assert_eq!(VerbType::Activation.priority(), 1);
        assert_eq!(VerbType::Verb.priority(), 0);
        assert_eq!(VerbType::Examine.priority(), 0);
        assert_eq!(VerbType::Innate.priority(), -1);
        assert_eq!(VerbType::ViewVariables.priority(), i32::MAX);
    }

    /// Порядок меню: тип → приоритет → категория (None первыми) → текст.
    #[test]
    fn sorting_matches_engine_compare_to() {
        let mut verbs = vec![
            Verb::new("Осмотреть", VerbType::Examine).category(VerbCategory::Examine),
            Verb::new("Взять", VerbType::Interaction).priority(0),
            Verb::new("Тянуть", VerbType::Verb),
            Verb::new("Vault", VerbType::Alternative),
            Verb::new("Открыть", VerbType::Activation).priority(3),
            Verb::new("View Variables", VerbType::ViewVariables),
        ];
        sort_verbs(&mut verbs);
        let texts: Vec<&str> = verbs.iter().map(|verb| verb.text.as_str()).collect();
        assert_eq!(
            texts,
            vec![
                "View Variables", // VvVerb — всегда первый
                "Взять",          // Interaction (4)
                "Vault",          // Alternative (2)
                "Открыть",        // Activation (1), priority 3
                "Тянуть",         // Verb (0), без категории
                "Осмотреть",      // Examine (0), но в категории → после «без категории»
            ]
        );
    }

    /// Стили текста различают типы (`ContextMenuSheetlet.cs:55-65`).
    #[test]
    fn style_classes_match_engine() {
        assert_eq!(VerbType::Interaction.style_class(), "InteractionVerb");
        assert_eq!(VerbType::Utility.style_class(), "InteractionVerb");
        assert_eq!(VerbType::Activation.style_class(), "ActivationVerb");
        assert_eq!(VerbType::Alternative.style_class(), "AlternativeVerb");
        assert_eq!(VerbType::Verb.style_class(), "Verb");
    }

    /// `CloseMenu`: у `ExamineVerb` по умолчанию меню остаётся открытым.
    #[test]
    fn examine_keeps_menu_open() {
        assert!(!Verb::new("Basic", VerbType::Examine).closes_menu());
        assert!(Verb::new("Pick Up", VerbType::Interaction).closes_menu());
        let mut verb = Verb::new("Pick Up", VerbType::Interaction);
        verb.close_menu = Some(false);
        assert!(!verb.closes_menu());
    }

    /// Категории: `IconsOnly` и колонки — из `VerbCategory.cs:47-107`.
    #[test]
    fn categories_match_engine() {
        assert!(VerbCategory::Rotate.icons_only());
        assert_eq!(VerbCategory::Rotate.columns(), 5);
        assert_eq!(VerbCategory::Smite.columns(), 6);
        assert!(!VerbCategory::Examine.icons_only());
        assert_eq!(VerbCategory::Examine.columns(), 1);
        assert_eq!(VerbCategory::Admin.text(), "Админ");
        assert_eq!(VerbCategory::Debug.text(), "Дебаг");
    }
}
