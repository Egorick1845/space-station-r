//! Речь над головой: реплики Runechat и облачко набора.
//!
//! Источники в сборке:
//! - `Content.Client/Chat/UI/SpeechBubble.cs` + `RunechatSpeechBubble.cs` +
//!   `RunechatStyle.cs` — текст без подложки, цвет по имени (`GetNameColor`),
//!   чёрная обводка, шёпот мельче, крик крупнее, эмоция в `*звёздочках*`,
//!   LOOC бирюзовым; жизнь 4 с, появление 0.15 с, затухание 0.25 с;
//! - `ChatUIController` — очередь на говорящего (`BubbleDelayBase` +
//!   `len · BubbleDelayFactor`), не больше 4 реплик (`SpeechBubbleCap`),
//!   старые поднимаются вверх на высоту новой;
//! - `TypingIndicatorVisualizerSystem` + `typing_indicator.yml` — слой
//!   `Effects/speech.rsi` (`default0` печатает, `default3` ждёт).
//!
//! Числа — в `ssr_core::speech`.

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use ssr_core::TILE_SIZE;
use ssr_core::speech::{self, TypingIndicator};
use ssr_protocol::{BubbleKind, SpeechBubble};

use crate::chat::ChatState;
use crate::inventory_ui::{OwnPlayerEntity, RemotePlayerVisual};
use crate::rsi::RsiRegistry;

/// Реплика на экране.
#[derive(Component)]
pub struct OverheadBubble {
    /// Серверная сущность говорящего.
    speaker: u64,
    /// Возраст, с.
    age: f32,
    /// Срок жизни, с (сокращается при вытеснении сверх лимита — `FadeNow`).
    life: f32,
    /// Цель подъёма над головой (сумма высот более новых реплик), px.
    target_offset: f32,
    /// Текущий подъём (плавно догоняет цель: `Lerp(…, 10·dt)`).
    offset: f32,
    /// Порядковый номер (новее — больше).
    serial: u64,
}

/// Текстовые узлы реплики: основной цвет и обводка (для прозрачности).
#[derive(Component)]
pub struct BubbleText {
    color: Color,
}

/// Очереди реплик по говорящим и счётчик.
#[derive(Resource, Default)]
pub struct BubbleQueues {
    queues: HashMap<u64, (VecDeque<SpeechBubble>, f32)>,
    serial: u64,
}

/// Облачко набора над головой.
#[derive(Component)]
pub struct TypingCloud {
    owner: Entity,
    state: TypingIndicator,
    frame: u32,
    elapsed: f32,
}

/// Смещения копий текста для обводки (8 направлений).
const OUTLINE_OFFSETS: [(f32, f32); 8] = [
    (-1.0, -1.0),
    (0.0, -1.0),
    (1.0, -1.0),
    (-1.0, 0.0),
    (1.0, 0.0),
    (-1.0, 1.0),
    (0.0, 1.0),
    (1.0, 1.0),
];

/// Текст реплики (`RunechatSpeechBubble.BuildBubble`).
fn bubble_display(bubble: &SpeechBubble) -> String {
    match bubble.kind {
        BubbleKind::Emote => format!("*{}*", bubble.text.trim()),
        _ => bubble.text.trim().to_string(),
    }
}

/// Цвет текста: по имени, у LOOC — `LoocColor`.
fn bubble_color(bubble: &SpeechBubble) -> Color {
    let [r, g, b] = if bubble.kind == BubbleKind::Looc {
        speech::LOOC_BUBBLE_COLOR
    } else {
        speech::name_color(&bubble.name)
    };
    Color::srgb(r, g, b)
}

/// Забирает реплики из чата в очереди и выпускает по одной на говорящего с
/// паузой `bubble_delay`; лишние сверх `SpeechBubbleCap` начинают гаснуть.
pub fn spawn_speech_bubbles(
    mut commands: Commands,
    time: Res<Time>,
    mut chat: ResMut<ChatState>,
    mut queues: ResMut<BubbleQueues>,
    mut bubbles: Query<&mut OverheadBubble>,
    settings: Res<crate::settings::Settings>,
) {
    for bubble in chat.pending_bubbles.drain(..) {
        // `hud.show_looc_above_head`: LOOC над головой можно выключить.
        if bubble.kind == BubbleKind::Looc && !settings.show_looc_overhead {
            continue;
        }
        queues
            .queues
            .entry(bubble.speaker)
            .or_insert_with(|| (VecDeque::new(), 0.0))
            .0
            .push_back(bubble);
    }
    let dt = time.delta_secs();
    let mut ready = Vec::new();
    queues.queues.retain(|_, (queue, timer)| {
        *timer -= dt;
        if *timer > 0.0 {
            return true;
        }
        let Some(bubble) = queue.pop_front() else {
            return false;
        };
        *timer += speech::bubble_delay(&bubble.text);
        ready.push(bubble);
        true
    });
    for bubble in ready {
        queues.serial += 1;
        let serial = queues.serial;
        let font_px = speech::bubble_font_px(bubble.kind == BubbleKind::Whisper, bubble.loud);
        let outline = speech::bubble_outline_px(font_px).max(1.0);
        let text = bubble_display(&bubble);
        let color = bubble_color(&bubble);
        let font = TextFont::from_font_size(font_px);
        commands
            .spawn((
                OverheadBubble {
                    speaker: bubble.speaker,
                    age: 0.0,
                    life: speech::BUBBLE_TOTAL_SECS,
                    target_offset: 0.0,
                    offset: 0.0,
                    serial,
                },
                Node {
                    position_type: PositionType::Absolute,
                    max_width: px(speech::BUBBLE_MAX_WIDTH),
                    padding: UiRect::all(px(outline.ceil())),
                    ..default()
                },
                Visibility::Hidden,
                GlobalZIndex(-5),
                Pickable::IGNORE,
            ))
            .with_children(|root| {
                // Обводка: 8 чёрных копий, сдвинутых на толщину.
                for (dx, dy) in OUTLINE_OFFSETS {
                    root.spawn((
                        Text::new(text.clone()),
                        font.clone(),
                        TextColor(Color::BLACK),
                        BubbleText {
                            color: Color::BLACK,
                        },
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(outline.ceil() + dx * outline),
                            top: px(outline.ceil() + dy * outline),
                            max_width: px(speech::BUBBLE_MAX_WIDTH),
                            ..default()
                        },
                    ));
                }
                root.spawn((
                    Text::new(text.clone()),
                    font,
                    TextColor(color),
                    BubbleText { color },
                ));
            });
        // `existing.Count > SpeechBubbleCap` → самая старая лишняя гаснет сразу.
        let mut same: Vec<_> = bubbles
            .iter_mut()
            .filter(|existing| existing.speaker == bubble.speaker)
            .collect();
        same.sort_by_key(|existing| std::cmp::Reverse(existing.serial));
        if same.len() >= speech::BUBBLE_CAP {
            for old in same.iter_mut().skip(speech::BUBBLE_CAP - 1) {
                let fade_end = old.age + speech::BUBBLE_FADE_OUT_SECS;
                if fade_end < old.life {
                    old.life = fade_end;
                }
            }
        }
        tracing::info!(speaker = bubble.speaker, %text, "speech bubble");
    }
}

/// Мировая точка говорящего: свой игрок — сущность `Player`, чужой — визуал
/// `RemotePlayerVisual` реплицированного тела.
fn speaker_world_position(
    speaker: u64,
    entity_map: &Option<Res<ServerEntityMap>>,
    own: &OwnPlayerEntity,
    players: &Query<&GlobalTransform, With<crate::Player>>,
    visuals: &Query<(&RemotePlayerVisual, &GlobalTransform)>,
) -> Option<Vec3> {
    let server = Entity::try_from_bits(speaker)?;
    let client = entity_map.as_deref()?.to_client().get(&server).copied()?;
    if own.0 == Some(client) {
        return players.iter().next().map(GlobalTransform::translation);
    }
    visuals
        .iter()
        .find(|(visual, _)| visual.player == client)
        .map(|(_, transform)| transform.translation())
}

/// Ставит реплики над головами (`SpeechBubble.FrameUpdate`): нижний край —
/// на `EntityVerticalOffset` (0.5 тайла) выше сущности, старые поднимаются
/// на высоту более новых, прозрачность — появление/затухание.
#[allow(clippy::too_many_arguments)]
pub fn position_speech_bubbles(
    mut commands: Commands,
    time: Res<Time>,
    own: Res<OwnPlayerEntity>,
    entity_map: Option<Res<ServerEntityMap>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera2d>>,
    players: Query<&GlobalTransform, With<crate::Player>>,
    visuals: Query<(&RemotePlayerVisual, &GlobalTransform)>,
    mut bubbles: Query<(
        Entity,
        &mut OverheadBubble,
        &mut Node,
        &mut Visibility,
        &ComputedNode,
        &Children,
    )>,
    mut texts: Query<(&BubbleText, &mut TextColor)>,
) {
    let dt = time.delta_secs();
    let Some((camera, camera_transform)) = cameras.iter().find(|(camera, _)| camera.is_active)
    else {
        return;
    };
    // Высоты (в логических px) по говорящим, от новых к старым.
    let mut stacks: HashMap<u64, Vec<(u64, f32)>> = HashMap::new();
    for (_, bubble, _, _, computed, _) in bubbles.iter() {
        let height = computed.size().y * computed.inverse_scale_factor();
        stacks
            .entry(bubble.speaker)
            .or_default()
            .push((bubble.serial, height));
    }
    for stack in stacks.values_mut() {
        stack.sort_by_key(|(serial, _)| std::cmp::Reverse(*serial));
    }
    for (entity, mut bubble, mut node, mut visibility, computed, children) in bubbles.iter_mut() {
        bubble.age += dt;
        if bubble.age >= bubble.life {
            commands.entity(entity).despawn();
            continue;
        }
        let above: f32 = stacks
            .get(&bubble.speaker)
            .map(|stack| {
                stack
                    .iter()
                    .take_while(|(serial, _)| *serial != bubble.serial)
                    .map(|(_, height)| height)
                    .sum()
            })
            .unwrap_or(0.0);
        bubble.target_offset = above;
        let lerp = (10.0 * dt).min(1.0);
        bubble.offset += (bubble.target_offset - bubble.offset) * lerp;
        if (bubble.offset - bubble.target_offset).abs() < 0.1 {
            bubble.offset = bubble.target_offset;
        }
        let Some(world) =
            speaker_world_position(bubble.speaker, &entity_map, &own, &players, &visuals)
        else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let anchor = world + Vec3::Y * speech::BUBBLE_VERTICAL_OFFSET * TILE_SIZE;
        let Ok(screen) = camera.world_to_viewport(camera_transform, anchor) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let size = computed.size() * computed.inverse_scale_factor();
        if size.x <= 0.0 {
            // Размер ещё не посчитан — покажем со следующего кадра.
            continue;
        }
        node.left = px((screen.x - size.x / 2.0).round());
        node.top = px((screen.y - size.y - bubble.offset).round());
        *visibility = Visibility::Visible;
        let alpha = speech::bubble_alpha(bubble.age, bubble.life);
        for child in children.iter() {
            if let Ok((text, mut color)) = texts.get_mut(child) {
                color.0 = text.color.with_alpha(alpha);
            }
        }
    }
}

/// Облачко набора (`TypingIndicatorVisualizerSystem`): слой `speech.rsi`
/// поверх тела, `default0` — печатает, `default3` — ждёт, нет — скрыт.
#[allow(clippy::type_complexity)]
pub fn sync_typing_clouds(
    mut commands: Commands,
    time: Res<Time>,
    registry: Res<RsiRegistry>,
    own: Res<OwnPlayerEntity>,
    players: Query<Entity, With<crate::Player>>,
    visuals: Query<(Entity, &RemotePlayerVisual)>,
    indicators: Query<&TypingIndicator>,
    mut clouds: Query<(Entity, &mut TypingCloud, &mut Sprite)>,
) {
    let mut owners: Vec<(Entity, Entity)> = players
        .iter()
        .filter_map(|entity| own.0.map(|data| (entity, data)))
        .collect();
    owners.extend(
        visuals
            .iter()
            .map(|(entity, visual)| (entity, visual.player)),
    );
    let dt = time.delta_secs();

    for (owner, data) in owners.iter().copied() {
        let state = indicators.get(data).copied().unwrap_or_default();
        let existing = clouds.iter_mut().find(|(_, cloud, _)| cloud.owner == owner);
        let state_name = match state {
            TypingIndicator::None => {
                if let Some((entity, _, _)) = existing {
                    commands.entity(entity).despawn();
                }
                continue;
            }
            TypingIndicator::Typing => speech::TYPING_STATE,
            TypingIndicator::Idle => speech::IDLE_STATE,
        };
        let key = format!("{}#{state_name}", speech::TYPING_RSI);
        let Some(sprite) = registry.get(&key) else {
            continue;
        };
        match existing {
            Some((_, mut cloud, mut image)) => {
                if cloud.state != state {
                    cloud.state = state;
                    cloud.frame = 0;
                    cloud.elapsed = 0.0;
                    image.image = sprite.image.clone();
                    image.texture_atlas = Some(TextureAtlas {
                        layout: sprite.layout.clone(),
                        index: sprite.index(0, 0),
                    });
                    continue;
                }
                // Кадры по `delays` из meta.json (флипбук как у алертов).
                let frames = sprite
                    .frames_per_direction
                    .first()
                    .copied()
                    .unwrap_or(1)
                    .max(1);
                let delays = sprite.delays.first().map(Vec::as_slice).unwrap_or(&[]);
                cloud.elapsed += dt;
                let (elapsed, frame) =
                    crate::hud::advance_alert_frame(cloud.elapsed, cloud.frame, delays, frames);
                cloud.elapsed = elapsed;
                if frame != cloud.frame {
                    cloud.frame = frame;
                    if let Some(atlas) = image.texture_atlas.as_mut() {
                        atlas.index = sprite.index(0, frame);
                    }
                }
            }
            None => {
                let mut image = Sprite::from_image(sprite.image.clone());
                image.texture_atlas = Some(TextureAtlas {
                    layout: sprite.layout.clone(),
                    index: sprite.index(0, 0),
                });
                commands.entity(owner).with_children(|parent| {
                    parent.spawn((
                        TypingCloud {
                            owner,
                            state,
                            frame: 0,
                            elapsed: 0.0,
                        },
                        image,
                        // Слой поверх тела со смещением прототипа (0, 0).
                        Transform::from_xyz(0.0, 0.0, 0.55),
                    ));
                });
                tracing::info!(?state, "typing cloud shown");
            }
        }
    }
}
