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

// ---------------------------------------------------------------- индикатор набора

/// Состояние индикатора набора (`TypingIndicatorState`): облачко над головой.
#[derive(
    bevy::prelude::Component,
    serde::Serialize,
    serde::Deserialize,
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
)]
pub enum TypingIndicator {
    /// Чат не в фокусе — облачка нет.
    #[default]
    None,
    /// Чат открыт, но текст давно не менялся — `idleState: default3`.
    Idle,
    /// Игрок печатает — `typingState: default0`.
    Typing,
}

/// Через сколько секунд без нажатий «печатает» сменяется на «ждёт»
/// (`TypingIndicatorSystem._typingTimeout`).
pub const TYPING_TIMEOUT: f32 = 2.0;
/// RSI и состояния прототипа `default` (`typing_indicator.yml`).
pub const TYPING_RSI: &str = "sprites/ss14/Effects/speech.rsi";
pub const TYPING_STATE: &str = "default0";
pub const IDLE_STATE: &str = "default3";

/// Состояние для отправки (`ClientUpdateTyping`): фокус чата + недавний ввод.
pub fn typing_state(chat_focused: bool, typing: bool) -> TypingIndicator {
    match (chat_focused, typing) {
        (false, _) => TypingIndicator::None,
        (true, true) => TypingIndicator::Typing,
        (true, false) => TypingIndicator::Idle,
    }
}

// ---------------------------------------------------------------- Runechat

/// Сколько живёт реплика над головой (`SpeechBubble.TotalTime`).
pub const BUBBLE_TOTAL_SECS: f32 = 4.0;
/// Затухание в конце (`SpeechBubble.FadeTime`).
pub const BUBBLE_FADE_OUT_SECS: f32 = 0.25;
/// Появление (`RunechatSpeechBubble.FadeInTime`).
pub const BUBBLE_FADE_IN_SECS: f32 = 0.15;
/// Сколько реплик одновременно над головой (`ChatUIController.SpeechBubbleCap`).
pub const BUBBLE_CAP: usize = 4;
/// Максимальная ширина текста (`RunechatMaxWidth`), px.
pub const BUBBLE_MAX_WIDTH: f32 = 200.0;
/// Высота над точкой сущности (`EntityVerticalOffset`), тайлов.
pub const BUBBLE_VERTICAL_OFFSET: f32 = 0.5;
/// Цвет LOOC над головой (`RunechatSpeechBubble.LoocColor` = #48d1cc).
pub const LOOC_BUBBLE_COLOR: [f32; 3] = [
    0x48 as f32 / 255.0,
    0xd1 as f32 / 255.0,
    0xcc as f32 / 255.0,
];

/// Пауза до следующей реплики в очереди: `BubbleDelayBase + len * BubbleDelayFactor`
/// (0.2 + len · 0.8/100).
pub fn bubble_delay(message: &str) -> f32 {
    0.2 + message.chars().count() as f32 * (0.8 / 100.0)
}

/// Размер шрифта Runechat в пикселях: базовый 12 pt (`chat.runechat_font_size`),
/// шёпот `ScaleSize(9)`, крик `GetLoudSize` = 18 pt; pt → px ×96/72.
pub fn bubble_font_px(whisper: bool, loud: bool) -> f32 {
    let points = if whisper {
        9.0
    } else if loud {
        18.0
    } else {
        12.0
    };
    points * 96.0 / 72.0
}

/// Толщина чёрной обводки (`RunechatStyle.GetOutline`): pt·96/72/8 · scale(1).
pub fn bubble_outline_px(font_px: f32) -> f32 {
    font_px / 8.0
}

/// Цвет имени (`RunechatSpeechBubble.GetNameColor`): FNV-1a по UTF-16 кодам
/// символов → HSL. Возвращает sRGB 0..1.
pub fn name_color(name: &str) -> [f32; 3] {
    let mut hash: u32 = 2_166_136_261;
    for unit in name.encode_utf16() {
        hash ^= unit as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    let hue = (hash % 360) as f32 / 360.0;
    let saturation = 0.55 + ((hash >> 9) % 30) as f32 / 100.0;
    let lightness = 0.65 + ((hash >> 17) % 15) as f32 / 100.0;
    hsl_to_rgb(hue, saturation, lightness)
}

/// `Color.FromHsl` (h, s, l в 0..1).
fn hsl_to_rgb(h: f32, s: f32, l: f32) -> [f32; 3] {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h * 6.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    [r + m, g + m, b + m]
}

/// Прозрачность реплики по возрасту: появление 0.15 с, затухание 0.25 с до
/// конца жизни `life` (4 с, либо раньше — при вытеснении сверх лимита).
pub fn bubble_alpha(age: f32, life: f32) -> f32 {
    let fade_in = (age / BUBBLE_FADE_IN_SECS).clamp(0.0, 1.0);
    let left = life - age;
    let fade_out = if left <= BUBBLE_FADE_OUT_SECS {
        (left / BUBBLE_FADE_OUT_SECS).clamp(0.0, 1.0)
    } else {
        1.0
    };
    fade_in * fade_out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_state_follows_focus_and_input() {
        assert_eq!(typing_state(false, true), TypingIndicator::None);
        assert_eq!(typing_state(true, true), TypingIndicator::Typing);
        assert_eq!(typing_state(true, false), TypingIndicator::Idle);
    }

    #[test]
    fn runechat_numbers_match_engine() {
        assert!((bubble_delay("") - 0.2).abs() < 1e-6);
        assert!((bubble_delay(&"a".repeat(100)) - 1.0).abs() < 1e-5);
        assert_eq!(bubble_font_px(false, false), 16.0);
        assert_eq!(bubble_font_px(true, true), 12.0);
        assert_eq!(bubble_font_px(false, true), 24.0);
        assert_eq!(bubble_outline_px(16.0), 2.0);
        assert_eq!(bubble_alpha(0.0, 4.0), 0.0);
        assert_eq!(bubble_alpha(1.0, 4.0), 1.0);
        assert!((bubble_alpha(3.875, 4.0) - 0.5).abs() < 1e-4);
    }

    #[test]
    fn name_color_is_stable_and_light() {
        let a = name_color("Иван");
        assert_eq!(a, name_color("Иван"));
        assert_ne!(a, name_color("Пётр"));
        // Светлота 0.65..0.80 — цвет всегда светлый.
        let max = a.iter().cloned().fold(0.0, f32::max);
        assert!(max > 0.6 && a.iter().all(|c| (0.0..=1.0).contains(c)));
    }

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
