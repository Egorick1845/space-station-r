//! Сетевой протокол Space Station R: сообщения клиент↔сервер, версионирование.
//!
//! ADR-4: репликация дельт; ADR-7: серверные ID сущностей.
//! Крейт зависит только от serde (PLAN.md §4); транспорт появится в T1.2.

use serde::{Deserialize, Serialize};

/// Версия протокола. Несовпадение при рукопожатии = отказ соединения (проверка в T1.2).
pub const PROTOCOL_VERSION: u32 = 13;

/// Порт игрового сервера по умолчанию (T1.2).
pub const DEFAULT_SERVER_PORT: u16 = 7777;

#[cfg(feature = "net")]
pub mod net;

/// Регистрация протокола в Bevy-приложении: сообщения и канал.
#[cfg(feature = "net")]
pub use net::ProtocolPlugin;

/// Сообщения клиент → сервер.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    /// Первый пакет после установления соединения.
    Connect {
        /// Версия протокола клиента — [`PROTOCOL_VERSION`].
        protocol_version: u32,
        /// Отображаемое имя игрока.
        name: String,
    },
    /// Направление ввода движения; нормализуется на сервере (репликация позиций — T1.3).
    Input {
        /// Ввод по осям: `[x, y]` — x вправо, y вверх.
        movement: [f32; 2],
        /// Бег: Shift удерживается (скорость выше, T-мех).
        #[serde(default)]
        running: bool,
        /// Боевой режим включён (клики бьют, а не используют, T-мех).
        #[serde(default)]
        combat: bool,
    },
    /// Взаимодействие с сущностью (система взаимодействия — T3.1).
    Interact {
        /// Идентификатор сущности на сервере.
        entity: u64,
    },
    /// Применить предмет к тайлу (T3.3): лист строит стену, лом разбирает.
    /// Решение принимает сервер по имени предмета (клиент имён не знает).
    UseItem {
        /// Идентификатор предмета (bits серверной сущности).
        item: u64,
        /// Координаты тайла.
        tx: i32,
        ty: i32,
    },
    /// Переключить активную руку (SS14-модель, T3.3+).
    SwitchHand,
    /// Скрафтить предмет по рецепту из `assets/prototypes/recipes.ron` (T5.2).
    Craft { recipe: String },
    /// Админ-команда серверу (T5.5): tp/spawn/kick/heal/ghost — проверяет сервер.
    Admin { command: String },
    /// Осмотр объекта или тайла (T-мех): клиент получает описание в ответе.
    Examine { entity: u64, tx: i32, ty: i32 },
    /// Поднять предмет с пола (лежит с ItemPosition, T-мех).
    Pickup { item: u64 },
    /// Сообщение в чат (ввод по T). LOOC слышат только рядом стоящие.
    Chat { channel: ChatChannel, text: String },
    /// Бросить предмет из рюкзака на пол (перетаскивание из окна в мир, SS14).
    DropItem { item: u64 },
    /// Надеть предмет: `slot` — целевой слот (одежда, карман, разгрузка).
    Equip { item: u64, slot: String },
    /// Сменить внешность (что передано — то и меняется; выдаётся на спавне
    /// случайно, а игрок может поправить: SS14 даёт это в лобби).
    SetAppearance {
        /// Пол (`male` / `female` / `unsexed`).
        sex: Option<String>,
        /// Стиль причёски (состояние `human_hair.rsi`).
        hair: Option<String>,
        /// Стиль бороды (состояние `human_facial_hair.rsi`), пусто — сбрить.
        beard: Option<String>,
        /// Цвет волос и бороды.
        hair_color: Option<[u8; 3]>,
    },
    /// Снять одежду из слота (имя слота как в каталоге: `jumpsuit`, `shoes`).
    Unequip { slot: String },
    /// Переключить боевой режим: обычные клики начинают бить, а не использовать.
    SetCombat { combat: bool },
    /// Переключить спринт (Space у человека, как `Sprint` в `keybinds.yml`).
    /// Сервер проверяет запреты (лежание, наручники, невесомость) и кулдаун.
    ToggleSprint { sprint: bool },
    /// Взять предмет из рюкзака в активную руку.
    TakeInHand {
        /// Слот рюкзака.
        slot: u8,
    },
    /// Убрать предмет из руки в рюкзак.
    MoveHandToInventory {
        /// Идентификатор предмета.
        item: u64,
    },
    /// Выбросить предмет из активной руки. В сборке (`SharedHandsSystem`) Q
    /// кладёт предмет К КУРСОРУ, но не дальше InteractionRange (1.5 тайла) —
    /// дальше сервер зажимает точку к игроку; Ctrl+Q (`ThrowItemInHand`) —
    /// БРОСОК: предмет летит к курсору и гасится о стены/игроков.
    DropHand {
        /// Мировая позиция курсора.
        target: [f32; 2],
        /// Ctrl+Q: бросок, а не аккуратная укладка.
        throw: bool,
    },
    /// Атаковать сущность (игрока) предметом из активной руки или кулаком (T4.1-мини).
    Attack {
        /// Цель (bits серверной сущности).
        target: u64,
    },
    /// Выстрел из оружия в активной руке (W-план): направление прицела.
    /// Сервер сам считает разброс (`GunSystem.GetAngle`), тратит патрон и
    /// порождает снаряд — клиент лишь сообщает, куда смотрит игрок.
    Shoot {
        /// Направление (единичный вектор) от игрока к точке прицела.
        dir: [f32; 2],
    },
    /// Перезарядка: вынуть магазин (если есть) или вставить магазин/патрон из
    /// активной руки (R в сборке — `SharedGunSystem.Interactions`).
    Reload,
    /// Запросить список контекстных действий (verbs) для цели.
    RequestActions {
        /// Сущность-цель; `0` — тайл по координатам.
        entity: u64,
        /// Координаты тайла.
        tx: i32,
        ty: i32,
    },
    /// Выполнить выбранное действие из меню.
    PerformAction {
        /// Выбранное действие.
        action: ActionKind,
    },
    /// Перенос предмета между инвентарями (T3.2).
    TransferItem {
        /// Идентификатор предмета (bits серверной сущности).
        item: u64,
        /// Целевой слот; 255 (`SLOT_ANY`) — первый свободный.
        to_slot: u8,
        /// bits игрока-получателя; `0` — свой инвентарь.
        target_player: u64,
    },
    /// Положить предмет из активной руки в хранилище предмета (окно пояса/
    /// рюкзака — `StorageWindow` в сборке).
    StoragePut {
        /// bits сущности хранилища (пояс и т.п.).
        container: u64,
        /// bits предмета из активной руки.
        item: u64,
    },
    /// Взять предмет из хранилища предмета в активную руку (клик по ячейке
    /// окна; при полной руке — в рюкзак, иначе под ноги).
    StorageTake {
        /// bits сущности хранилища.
        container: u64,
        /// Слот (якорь) в сетке хранилища.
        slot: u8,
    },
}

/// Контекстное действие (verbs в духе SS14, T3.3+): то, что можно сделать
/// с целью; клиент показывает список, выбранное шлёт обратно как PerformAction.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ActionKind {
    /// Ударить сущность (игрока).
    Attack { target: u64 },
    /// Взаимодействовать (двери и т.п.).
    Interact { entity: u64 },
    /// Применить предмет из руки к тайлу (стройка/разборка).
    UseItem { item: u64, tx: i32, ty: i32 },
    /// Взять предмет с пола (верб предмета, как ПКМ → «Взять» в SS14).
    Pickup { item: u64 },
    /// Осмотреть объект: сервер отвечает описанием (верб «Осмотреть»).
    Examine { entity: u64 },
    /// Тянуть сущность за собой (верб «Тянуть», `PullMessage` в сборке).
    /// Повторное действие по той же цели отпускает её.
    Pull { target: u64 },
    /// Админ-верб «Удалить» (`delete-verb-get-data-text`, категория Debug).
    Delete { entity: u64 },
    /// Дебаг-верб «Оживить» (`rejuvenate-verb-get-data-text`).
    Rejuvenate { entity: u64 },
    /// Админ-верб «Стать призраком» (`aghost`).
    AdminGhost,
    /// Верб «View Variables» (`VvVerb`): открыть окно переменных сущности.
    ViewVariables { entity: u64 },
    /// Снять надетый предмет (верб на носителе: ПКМ по себе/игроку — вербы
    /// его экипировки, как `StrippingSystem`/`InventorySystem` в сборке).
    Unequip { slot: String },
}

/// Пункт меню действий (верб): подпись, действие и метаданные из
/// `Content.Shared/Verbs/Verb.cs` — тип (шрифт и `TypePriority`), категория
/// (подменю), иконка, приоритет, недоступность с причиной.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionOption {
    /// Текст верба (`Verb.Text`).
    pub label: String,
    /// Исполняемое действие.
    pub action: ActionKind,
    /// Тип верба: задаёт `TypePriority` и стиль шрифта (`Verb.cs:218-364`).
    #[serde(default)]
    pub kind: ssr_core::verbs::VerbType,
    /// Категория (подменю); `None` — верхний уровень (`VerbCategory.cs`).
    #[serde(default)]
    pub category: Option<ssr_core::verbs::VerbCategory>,
    /// Иконка (`SpriteSpecifier`).
    #[serde(default)]
    pub icon: Option<String>,
    /// Приоритет внутри типа (`Verb.Priority`, больше — выше).
    #[serde(default)]
    pub priority: i32,
    /// Недоступен: серый пункт, причина в `message` (`Verb.Disabled`).
    #[serde(default)]
    pub disabled: bool,
    /// Причина недоступности / подсказка (`Verb.Message`).
    #[serde(default)]
    pub message: Option<String>,
    /// Закрывать ли меню после исполнения (`Verb.CloseMenu`).
    #[serde(default)]
    pub close_menu: Option<bool>,
    /// Верб исполняется на клиенте (`Verb.ClientExclusive`).
    #[serde(default)]
    pub client_exclusive: bool,
    /// Требуется подтверждение (`Verb.ConfirmationPopup`).
    #[serde(default)]
    pub confirmation_popup: bool,
}

impl ActionOption {
    /// Ключ сортировки — как `Verb.CompareTo` (`Verb.cs:169-207`): тип (убыв.),
    /// приоритет (убыв.), категория (без категории — ПЕРВЫМИ), затем текст.
    pub fn sort_key(&self) -> (i32, i32, u8, &str, &str) {
        (
            -self.kind.priority(),
            -self.priority,
            u8::from(self.category.is_some()),
            self.category
                .map(ssr_core::verbs::VerbCategory::text)
                .unwrap_or(""),
            self.label.as_str(),
        )
    }

    /// Помещает верб в категорию (подменю).
    pub fn in_category(mut self, category: ssr_core::verbs::VerbCategory) -> Self {
        self.category = Some(category);
        self
    }

    /// Закрывать ли меню (`CloseMenu ?? CloseMenuDefault`; у `ExamineVerb` — нет).
    pub fn closes_menu(&self) -> bool {
        self.close_menu
            .unwrap_or(!matches!(self.kind, ssr_core::verbs::VerbType::Examine))
    }

    /// Стиль текста (`TextStyleClass`) — различие типов вербов в меню.
    pub fn style_class(&self) -> &'static str {
        self.kind.style_class()
    }
}

/// Сообщения сервер → клиент.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ServerMessage {
    /// Ответ на [`ClientMessage::Connect`]: назначенная сервером сущность игрока.
    Welcome {
        /// Идентификатор сущности игрока (ADR-7).
        player_entity: u64,
        /// Версия протокола сервера — [`PROTOCOL_VERSION`].
        protocol_version: u32,
    },
    /// Полный снимок видимых сущностей (при спавне и смене интерес-чанков, T1.4).
    WorldState {
        /// Сущности, видимые игроку.
        entities: Vec<EntitySpawn>,
    },
    /// Дельта позиции сущности (ADR-4: шлём изменения, не снапшоты).
    EntityDelta {
        /// Идентификатор сущности на сервере.
        entity: u64,
        /// Новая позиция `[x, y]`.
        position: [f32; 2],
    },
    /// Список контекстных действий (ответ на [`ClientMessage::RequestActions`]).
    Actions { options: Vec<ActionOption> },
    /// Игровое событие для отображения (урон, взаимодействие и т.п.).
    Event {
        /// Тип события; типизируется в T3.1.
        kind: String,
    },
    /// Сообщение чата для панели (SS14: `ChatBox` справа вверху).
    Chat {
        /// Канал сообщения.
        channel: ChatChannel,
        /// Имя отправителя (пусто — системное сообщение).
        from: String,
        /// Текст сообщения.
        text: String,
    },
}

/// Канал чата. Цвета — `ChatChannelExtensions.TextColor` сборки
/// (`Content.Shared/Chat/ChatChannelExtensions.cs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatChannel {
    /// Общий чат (OOC) — видят все.
    Ooc,
    /// Локальный чат (LOOC) — видят только рядом стоящие.
    Looc,
    /// Системное сообщение сервера (`ChatChannel.Server` — Orange).
    System,
    /// Чат мёртвых (`ChatChannel.Dead`, MediumPurple) — пишут только
    /// призраки, читают призраки и админы (`GetDeadChatClients`).
    Dead,
    /// Админ-чат (`ChatChannel.AdminChat`, HotPink) — только для админов.
    AdminChat,
}

/// Сущность в снимке мира.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntitySpawn {
    /// Идентификатор сущности на сервере.
    pub entity: u64,
    /// Позиция `[x, y]`.
    pub position: [f32; 2],
}

/// Проверка совместимости версии протокола удалённой стороны.
pub fn is_compatible(peer_protocol_version: u32) -> bool {
    peer_protocol_version == PROTOCOL_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Критерий T1.1: сериализация → десериализация → равенство.
    fn round_trip<T>(value: T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug,
    {
        let bytes = postcard::to_allocvec(&value).expect("serialize");
        postcard::from_bytes(&bytes).expect("deserialize")
    }

    #[test]
    fn client_messages_round_trip() {
        let messages = [
            ClientMessage::Connect {
                protocol_version: PROTOCOL_VERSION,
                name: "Тест_игрок".into(),
            },
            ClientMessage::Input {
                movement: [0.5, -1.0],
                running: true,
                combat: false,
            },
            ClientMessage::Interact { entity: 42 },
        ];
        for msg in messages {
            assert_eq!(round_trip(msg.clone()), msg);
        }
    }

    #[test]
    fn server_messages_round_trip() {
        let messages = [
            ServerMessage::Welcome {
                player_entity: 1,
                protocol_version: PROTOCOL_VERSION,
            },
            ServerMessage::WorldState {
                entities: vec![EntitySpawn {
                    entity: 1,
                    position: [0.0, 0.0],
                }],
            },
            ServerMessage::EntityDelta {
                entity: 1,
                position: [12.5, 3.25],
            },
            ServerMessage::Event {
                kind: "damage".into(),
            },
        ];
        for msg in messages {
            assert_eq!(round_trip(msg.clone()), msg);
        }
    }

    #[test]
    fn version_check() {
        assert!(is_compatible(PROTOCOL_VERSION));
        assert!(!is_compatible(PROTOCOL_VERSION + 1));
        assert!(!is_compatible(0));
    }
}
