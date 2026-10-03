//! Сетевой протокол Space Station R: сообщения клиент↔сервер, версионирование.
//!
//! ADR-4: репликация дельт; ADR-7: серверные ID сущностей.
//! Крейт зависит только от serde (PLAN.md §4); транспорт появится в T1.2.

use serde::{Deserialize, Serialize};

/// Версия протокола. Несовпадение при рукопожатии = отказ соединения (проверка в T1.2).
pub const PROTOCOL_VERSION: u32 = 1;

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
    },
    /// Взаимодействие с сущностью (система взаимодействия — T3.1).
    Interact {
        /// Идентификатор сущности на сервере.
        entity: u64,
    },
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
    /// Игровое событие для отображения (урон, взаимодействие и т.п.).
    Event {
        /// Тип события; типизируется в T3.1.
        kind: String,
    },
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
