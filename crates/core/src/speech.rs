//! Речь персонажа (IC): «говорит», шёпот и эмоции — перенос из сборки.
//!
//! Источники:
//! - `Content.Server/Chat/Systems/ChatSystem.cs` — дальности (`VoiceRange = 10`,
//!   `WhisperClearRange = 2`, `WhisperMuffledRange = 5`), зашумление шёпота
//!   (`ObfuscateMessageReadability`, `DefaultObfuscationFactor = 0.2`), правила
//!   видимости (`EmoteRespectsLOS = true`, речь и шёпот слышны сквозь стены);
//! - `Content.Shared/Chat/SharedChatSystem.cs` — `GetSpeechVerb` (суффиксы),
//!   `SanitizeMessageCapital`;
//! - `Content.Shared/Speech/SpeechComponent.cs` — `SuffixSpeechVerbs`;
//! - `Resources/Prototypes/Voice/speech_verbs.yml` — приоритеты глаголов;
//! - `Resources/Locale/ru-RU/chat/managers/chat-manager.ftl` — тексты обёрток.
//!
//! Модуль чистый (без Bevy): сервер решает, кто что услышит, клиент только
//! показывает готовую строку.

/// Дальность речи и эмоций в тайлах (`ChatSystem.VoiceRange`).
pub const VOICE_RANGE: f32 = 10.0;
/// До этой дальности шёпот слышен разборчиво (`ChatSystem.WhisperClearRange`).
pub const WHISPER_CLEAR_RANGE: f32 = 2.0;
/// Дальше этого шёпот не слышен вовсе (`ChatSystem.WhisperMuffledRange`).
pub const WHISPER_MUFFLED_RANGE: f32 = 5.0;
/// Доля символов дальнего шёпота, которая остаётся видна
/// (`ChatSystem.DefaultObfuscationFactor`).
pub const OBFUSCATION_FACTOR: f32 = 0.2;

/// Глагол речи (`SpeechVerbPrototype`): текст и жирность (`bold`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechVerb {
    pub text: &'static str,
    pub bold: bool,
}

/// `chat-speech-verb-default` — «говорит» (прототип `Default`).
pub const VERB_DEFAULT: SpeechVerb = SpeechVerb {
    text: "говорит",
    bold: false,
};

/// Суффиксные глаголы в порядке `SpeechComponent.SuffixSpeechVerbs`:
/// (суффикс, глагол, приоритет из `speech_verbs.yml`). У
/// `DefaultExclamationStrong` приоритет 10, у остальных — 0.
const SUFFIX_VERBS: [(&str, SpeechVerb, i32); 5] = [
    (
        "!!",
        SpeechVerb {
            text: "кричит",
            bold: true,
        },
        10,
    ),
    (
        "!",
        SpeechVerb {
            text: "восклицает",
            bold: false,
        },
        0,
    ),
    (
        "?",
        SpeechVerb {
            text: "спрашивает",
            bold: false,
        },
        0,
    ),
    (
        "-",
        SpeechVerb {
            text: "запинается",
            bold: false,
        },
        0,
    ),
    (
        "..",
        SpeechVerb {
            text: "бубнит",
            bold: false,
        },
        0,
    ),
];

/// Глагол по окончанию реплики — `SharedChatSystem.GetSpeechVerb`: проходим
/// суффиксы по порядку, берём подходящий с приоритетом `>=` текущего
/// (поэтому при равных приоритетах побеждает последний подошедший).
pub fn speech_verb(message: &str) -> SpeechVerb {
    let mut current: Option<(SpeechVerb, i32)> = None;
    for (suffix, verb, priority) in SUFFIX_VERBS {
        if message.ends_with(suffix) && priority >= current.map(|(_, p)| p).unwrap_or(0) {
            current = Some((verb, priority));
        }
    }
    current.map(|(verb, _)| verb).unwrap_or(VERB_DEFAULT)
}

/// `SanitizeMessageCapital`: первая буква — заглавная. Эмоции не капитализируются
/// (`shouldCapitalize = desiredType != Emote`).
pub fn capitalize(message: &str) -> String {
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `ObfuscateMessageReadability`: каждый непробельный символ с вероятностью
/// `1 − chance` заменяется на `~`. `roll` — равномерное число в `[0, 1)`.
pub fn obfuscate(message: &str, chance: f32, mut roll: impl FnMut() -> f32) -> String {
    message
        .chars()
        .map(|ch| {
            if ch.is_whitespace() {
                ch
            } else if roll() < 1.0 - chance {
                '~'
            } else {
                ch
            }
        })
        .collect()
}

/// Как слушатель воспринимает шёпот (`SendEntityWhisper`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperHearing {
    /// Ближе `WhisperClearRange` (сквозь стены тоже) или призрак — текст целиком.
    Clear,
    /// В зоне `WhisperMuffledRange` и в прямой видимости — имя видно, текст зашумлён.
    Muffled,
    /// В зоне, но за препятствием — «Кто-то шепчет», текст зашумлён.
    MuffledUnknown,
    /// Дальше `WhisperMuffledRange` — не слышно.
    None,
}

/// Восприятие шёпота по дистанции в тайлах и прямой видимости. Призраки
/// (`GhostHearing`) получают `Range = -1` и потому всегда слышат разборчиво.
pub fn whisper_hearing(distance: f32, line_of_sight: bool, observer: bool) -> WhisperHearing {
    if observer || distance <= WHISPER_CLEAR_RANGE {
        WhisperHearing::Clear
    } else if distance > WHISPER_MUFFLED_RANGE {
        WhisperHearing::None
    } else if line_of_sight {
        WhisperHearing::Muffled
    } else {
        WhisperHearing::MuffledUnknown
    }
}

/// Слышна ли речь (`SendInVoiceRange`, `SpeakRespectsLOS = false`): только
/// дистанция, призраки слышат всё.
pub fn hears_say(distance: f32, observer: bool) -> bool {
    observer || distance <= VOICE_RANGE
}

/// Видна ли эмоция (`EmoteRespectsLOS = true`): дистанция и прямая видимость,
/// призраки видят всё.
pub fn sees_emote(distance: f32, line_of_sight: bool, observer: bool) -> bool {
    observer || (distance <= VOICE_RANGE && line_of_sight)
}

/// `chat-manager-entity-say-wrap-message`: `Имя говорит, "текст"`.
pub fn wrap_say(name: &str, verb: SpeechVerb, message: &str) -> String {
    format!("{name} {}, \"{message}\"", verb.text)
}

/// `chat-manager-entity-whisper-wrap-message`: `Имя шепчет, "текст"`.
pub fn wrap_whisper(name: &str, message: &str) -> String {
    format!("{name} шепчет, \"{message}\"")
}

/// `chat-manager-entity-whisper-unknown-wrap-message`: `Кто-то шепчет, "текст"`.
pub fn wrap_whisper_unknown(message: &str) -> String {
    format!("Кто-то шепчет, \"{message}\"")
}

/// `chat-manager-entity-me-wrap-message`: `{ CAPITALIZE($entityName) } текст`.
pub fn wrap_emote(name: &str, message: &str) -> String {
    format!("{} {message}", capitalize(name))
}

/// Прямая видимость между двумя точками в тайловых координатах (аналог
/// `ExamineSystem.InRangeUnOccluded` для тайловой карты): отрезок проходит
/// с шагом 1/4 тайла, препятствием считается любой тайл, для которого
/// `solid(x, y)` истинно, кроме тайлов самих источника и слушателя.
pub fn line_of_sight(from: [f32; 2], to: [f32; 2], solid: impl Fn(i32, i32) -> bool) -> bool {
    let start = (from[0].floor() as i32, from[1].floor() as i32);
    let end = (to[0].floor() as i32, to[1].floor() as i32);
    let dx = to[0] - from[0];
    let dy = to[1] - from[1];
    let steps = ((dx.abs().max(dy.abs())) * 4.0).ceil().max(1.0) as i32;
    for step in 1..steps {
        let t = step as f32 / steps as f32;
        let tile = (
            (from[0] + dx * t).floor() as i32,
            (from[1] + dy * t).floor() as i32,
        );
        if tile != start && tile != end && solid(tile.0, tile.1) {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_match_chat_system() {
        assert_eq!(VOICE_RANGE, 10.0);
        assert_eq!(WHISPER_CLEAR_RANGE, 2.0);
        assert_eq!(WHISPER_MUFFLED_RANGE, 5.0);
        assert_eq!(OBFUSCATION_FACTOR, 0.2);
    }

    #[test]
    fn speech_verb_follows_suffixes_and_priority() {
        assert_eq!(speech_verb("привет").text, "говорит");
        assert_eq!(speech_verb("привет!").text, "восклицает");
        // "!!" тоже оканчивается на "!", но у крика приоритет 10.
        let shout = speech_verb("помогите!!");
        assert_eq!(shout.text, "кричит");
        assert!(shout.bold);
        assert_eq!(speech_verb("кто здесь?").text, "спрашивает");
        assert_eq!(speech_verb("я-я-").text, "запинается");
        assert_eq!(speech_verb("ну...").text, "бубнит");
    }

    #[test]
    fn capitalize_handles_cyrillic_and_empty() {
        assert_eq!(capitalize("привет"), "Привет");
        assert_eq!(capitalize("hello"), "Hello");
        assert_eq!(capitalize(""), "");
    }

    #[test]
    fn obfuscation_keeps_whitespace_and_masks_by_roll() {
        // roll = 0.0 < 0.8 — всё непробельное заменяется.
        assert_eq!(obfuscate("а б", OBFUSCATION_FACTOR, || 0.0), "~ ~");
        // roll = 0.9 >= 0.8 — символ остаётся.
        assert_eq!(obfuscate("а б", OBFUSCATION_FACTOR, || 0.9), "а б");
    }

    #[test]
    fn whisper_hearing_by_distance_and_sight() {
        assert_eq!(whisper_hearing(1.5, false, false), WhisperHearing::Clear);
        assert_eq!(whisper_hearing(2.0, false, false), WhisperHearing::Clear);
        assert_eq!(whisper_hearing(4.0, true, false), WhisperHearing::Muffled);
        assert_eq!(
            whisper_hearing(4.0, false, false),
            WhisperHearing::MuffledUnknown
        );
        assert_eq!(whisper_hearing(5.5, true, false), WhisperHearing::None);
        assert_eq!(whisper_hearing(50.0, false, true), WhisperHearing::Clear);
    }

    #[test]
    fn say_ignores_walls_emote_needs_sight() {
        assert!(hears_say(10.0, false));
        assert!(!hears_say(10.5, false));
        assert!(hears_say(99.0, true));
        assert!(sees_emote(5.0, true, false));
        assert!(!sees_emote(5.0, false, false));
        assert!(!sees_emote(11.0, true, false));
        assert!(sees_emote(99.0, false, true));
    }

    #[test]
    fn wraps_match_locale() {
        assert_eq!(
            wrap_say("Иван", speech_verb("привет?"), "Привет?"),
            "Иван спрашивает, \"Привет?\""
        );
        assert_eq!(wrap_whisper("Иван", "тсс"), "Иван шепчет, \"тсс\"");
        assert_eq!(wrap_whisper_unknown("~~~"), "Кто-то шепчет, \"~~~\"");
        assert_eq!(wrap_emote("иван", "машет рукой"), "Иван машет рукой");
    }

    #[test]
    fn line_of_sight_blocked_by_wall_between() {
        // Стена на x = 2 по всей высоте.
        let wall = |x: i32, _y: i32| x == 2;
        assert!(!line_of_sight([0.5, 0.5], [4.5, 0.5], wall));
        assert!(line_of_sight([0.5, 0.5], [1.5, 3.5], wall));
        // Тайлы источника и слушателя не считаются препятствием.
        assert!(line_of_sight([2.5, 0.5], [3.5, 0.5], wall));
    }
}
