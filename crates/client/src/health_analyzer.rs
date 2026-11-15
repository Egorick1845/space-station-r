//! Окно анализатора здоровья — `HealthAnalyzerWindow` (`Content.Client/
//! HealthAnalyzer/UI/HealthAnalyzerWindow.xaml(.cs)`): пациент, статус,
//! температура, уровень крови, общий урон, состояния (кровотечение,
//! низкий уровень крови) и урон по группам с типами. Вкладки органов и
//! химии Shitmed не перенесены. Данные — из реплицируемых `Damageable` и
//! `Bloodstream` пациента; сервер сообщает только, идёт ли скан.
//! Ещё здесь отрисовка луж крови (`PuddleSystem.UpdatePuddleSprite`).

use bevy::prelude::*;
use bevy_replicon::shared::server_entity_map::ServerEntityMap;
use ssr_core::blood::{BLOOD_COLOR, BLOODLOSS_THRESHOLD, Bloodstream, Puddle};
use ssr_core::damage::{DamageGroup, Damageable, MobState};

use crate::rsi::RsiRegistry;
use crate::ui_theme as ui;

/// Последний скан: пациент (серверные bits) и активен ли скан.
#[derive(Resource, Default, Debug)]
pub struct HealthScan {
    pub target: Option<u64>,
    pub active: bool,
}

impl HealthScan {
    /// Сообщение `HealthScan` от сервера.
    pub fn receive(&mut self, target: u64, active: bool, opened: bool) {
        if opened {
            self.target = Some(target);
            self.active = active;
        } else if self.target == Some(target) {
            self.active = active;
        }
    }
}

#[derive(Component)]
pub struct AnalyzerRoot;

#[derive(Component)]
pub struct AnalyzerClose;

/// Строка окна: текст, цвет, размер шрифта, отступ слева.
type Line = (String, Color, f32, f32);

const RED: Color = Color::srgb_u8(0xd4, 0x3b, 0x3b);
const GOLD: Color = Color::srgb_u8(0xe8, 0xc0, 0x4b);
const GREEN: Color = Color::srgb_u8(0x6c, 0xc0, 0x6c);

/// Число урона как в окне (`FixedPoint2`): без лишних нулей.
fn amount(value: f32) -> String {
    let rounded = (value * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded}")
    }
}

/// Содержимое окна по данным пациента.
pub fn analyzer_lines(
    name: &str,
    active: bool,
    damage: Option<&Damageable>,
    blood: Option<&Bloodstream>,
) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(if active {
        ("Сканирование: активно".into(), GREEN, ui::FONT_BASE, 0.0)
    } else {
        (
            "Сканирование: неактивно".into(),
            ui::TEXT_MUTED,
            ui::FONT_BASE,
            0.0,
        )
    });
    lines.push((format!("Пациент: {name}"), ui::TEXT, ui::FONT_LABEL, 0.0));
    let status = match damage.map(|d| d.state) {
        Some(MobState::Alive) => ("Жив", GREEN),
        Some(MobState::Critical) => ("В критическом состоянии", GOLD),
        Some(MobState::Dead) => ("Мёртв", RED),
        None => ("Неизвестно", ui::TEXT_MUTED),
    };
    lines.push((
        format!("Статус: {}", status.0),
        status.1,
        ui::FONT_BASE,
        0.0,
    ));
    // Температуры тела у нас пока нет — как `NaN` в сборке.
    lines.push(("Температура: Н/Д".into(), ui::TEXT, ui::FONT_BASE, 0.0));
    let blood_text = blood
        .map(|b| format!("{:.1} %", b.level() * 100.0))
        .unwrap_or_else(|| "Н/Д".into());
    lines.push((
        format!("Уровень крови: {blood_text}"),
        ui::TEXT,
        ui::FONT_BASE,
        0.0,
    ));
    let total = damage.map(|d| d.total()).unwrap_or(0.0);
    lines.push((
        format!("Общий урон: {}", amount(total)),
        ui::TEXT,
        ui::FONT_BASE,
        0.0,
    ));

    // Состояния (`AlertsContainer`).
    lines.push(("Состояния".into(), ui::ACCENT, ui::FONT_LABEL, 0.0));
    let mut conditions = 0;
    if let Some(blood) = blood {
        if blood.level() < BLOODLOSS_THRESHOLD {
            lines.push((
                format!("{name} имеет низкий уровень крови."),
                RED,
                ui::FONT_BASE,
                8.0,
            ));
            conditions += 1;
        }
        if blood.bleed > 0.0 {
            lines.push((format!("{name} кровоточит."), RED, ui::FONT_BASE, 8.0));
            conditions += 1;
        }
    }
    if conditions == 0 {
        lines.push(("Нет".into(), ui::TEXT_MUTED, ui::FONT_BASE, 8.0));
    }

    // Урон по группам (`DrawDiagnosticGroups`): группа, затем её типы.
    lines.push(("Повреждения".into(), ui::ACCENT, ui::FONT_LABEL, 0.0));
    let mut any = false;
    if let Some(damage) = damage {
        for group in DamageGroup::ALL {
            let group_total = damage.damage.group_total(group);
            if group_total <= 0.0 {
                continue;
            }
            any = true;
            lines.push((
                format!("{}: {}", group.name_ru(), amount(group_total)),
                ui::TEXT,
                ui::FONT_BASE,
                8.0,
            ));
            for damage_type in group.types() {
                let value = damage.damage.get(*damage_type);
                if value <= 0.0 {
                    continue;
                }
                lines.push((
                    format!("· {}: {}", damage_type.name_ru(), amount(value)),
                    ui::TEXT_MUTED,
                    ui::FONT_BASE,
                    20.0,
                ));
            }
        }
    }
    if !any {
        lines.push(("Нет".into(), ui::TEXT_MUTED, ui::FONT_BASE, 8.0));
    }
    lines
}

/// Перерисовка окна при изменении данных пациента.
#[allow(clippy::too_many_arguments)]
pub fn render_health_analyzer(
    mut commands: Commands,
    scan: Res<HealthScan>,
    entity_map: Option<Res<ServerEntityMap>>,
    damageables: Query<&Damageable>,
    bloods: Query<&Bloodstream>,
    names: Query<&ssr_core::mechanics::PlayerName>,
    theme: Res<ui::UiTheme>,
    roots: Query<Entity, With<AnalyzerRoot>>,
    mut last: Local<Option<Vec<Line>>>,
) {
    let patient = scan.target.and_then(|bits| {
        let server = Entity::try_from_bits(bits)?;
        entity_map.as_deref()?.to_client().get(&server).copied()
    });
    let lines = scan.target.map(|_| {
        let name = patient
            .and_then(|entity| names.get(entity).ok())
            .map(|name| name.0.clone())
            .unwrap_or_else(|| "Неизвестно".into());
        analyzer_lines(
            &name,
            scan.active,
            patient.and_then(|e| damageables.get(e).ok()),
            patient.and_then(|e| bloods.get(e).ok()),
        )
    });
    if *last == lines {
        return;
    }
    *last = lines.clone();
    for root in roots.iter() {
        commands.entity(root).despawn();
    }
    let Some(lines) = lines else {
        return;
    };
    commands
        .spawn((
            AnalyzerRoot,
            Node {
                position_type: PositionType::Absolute,
                left: px(90),
                top: px(90),
                width: px(320),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            GlobalZIndex(40),
        ))
        .with_children(|window| {
            window
                .spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: px(25.0),
                        align_items: AlignItems::Center,
                        padding: UiRect::new(px(6), px(6), px(2), px(2)),
                        border: UiRect::bottom(px(2)),
                        ..default()
                    },
                    BackgroundColor(ui::GLASS_HEADER),
                    BorderColor::from(ui::GLASS_HEADER_LINE),
                ))
                .with_children(|row| {
                    row.spawn((
                        Text::new("Анализатор здоровья"),
                        TextFont::from_font_size(ui::FONT_LABEL),
                        TextColor(ui::WINDOW_TITLE),
                    ));
                    row.spawn((
                        AnalyzerClose,
                        Button,
                        crate::hud::IconTint::cross(),
                        ImageNode::new(theme.cross.clone()),
                        Node {
                            width: px(22),
                            height: px(22),
                            margin: UiRect::left(Val::Auto),
                            ..default()
                        },
                    ));
                });
            let (body, background) = crate::hud::window_body();
            window.spawn((body, background)).with_children(|body| {
                for (text, color, size, indent) in lines {
                    body.spawn((
                        Text::new(text),
                        TextFont::from_font_size(size),
                        TextColor(color),
                        Node {
                            margin: UiRect::left(px(indent)),
                            ..default()
                        },
                    ));
                }
            });
        });
}

/// Крестик окна — закрыть (`StopAnalyzingEntity` при закрытии UI).
pub fn analyzer_close(
    buttons: Query<&Interaction, (Changed<Interaction>, With<AnalyzerClose>)>,
    mut scan: ResMut<HealthScan>,
) {
    if buttons.iter().any(|i| *i == Interaction::Pressed) {
        scan.target = None;
    }
}

/// Визуал лужи: спрайт на полу под предметами.
#[derive(Component)]
pub struct PuddleVisual {
    pub state: &'static str,
}

/// z луж: `DrawDepth.Puddles` — ниже предметов и структур, над полом.
const PUDDLE_Z: f32 = 0.2;

/// Лужи крови: спрайт `Fluids/puddle.rsi` по объёму, цвет реагента `Blood`.
pub fn render_puddles(
    mut commands: Commands,
    registry: Res<RsiRegistry>,
    puddles: Query<(
        Entity,
        &Puddle,
        &ssr_core::inventory::ItemPosition,
        Option<&PuddleVisual>,
    )>,
) {
    for (entity, puddle, position, visual) in puddles.iter() {
        let state = puddle.sprite_state();
        if visual.is_some_and(|v| v.state == state) {
            continue;
        }
        let key = format!("sprites/ss14/Fluids/puddle.rsi#{state}");
        let Some(sprite) = registry.get(&key) else {
            continue;
        };
        let mut component = Sprite::from_image(sprite.image.clone());
        component.texture_atlas = Some(TextureAtlas {
            layout: sprite.layout.clone(),
            index: sprite.index(0, 0),
        });
        component.color = Color::srgb(BLOOD_COLOR[0], BLOOD_COLOR[1], BLOOD_COLOR[2]);
        commands.entity(entity).insert((
            PuddleVisual { state },
            component,
            Transform::from_xyz(position.0[0], position.0[1], PUDDLE_Z).with_rotation(
                Quat::from_rotation_z(
                    puddle.rotation_quarters() as f32 * std::f32::consts::FRAC_PI_2,
                ),
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzer_shows_bleeding_and_groups() {
        let mut damage = Damageable::default();
        damage.damage =
            ssr_core::damage::DamageSpecifier::of(ssr_core::damage::DamageType::Slash, 12.5);
        let blood = Bloodstream {
            volume: 240.0,
            bleed: 3.0,
            ..Bloodstream::default()
        };
        let lines = analyzer_lines("Боб", true, Some(&damage), Some(&blood));
        let texts: Vec<&str> = lines.iter().map(|l| l.0.as_str()).collect();
        assert!(texts.contains(&"Уровень крови: 80.0 %"));
        assert!(texts.contains(&"Боб имеет низкий уровень крови."));
        assert!(texts.contains(&"Боб кровоточит."));
        assert!(
            texts.contains(&"Механические: 12.5") || texts.iter().any(|t| t.ends_with(": 12.5"))
        );
        assert!(texts.contains(&"Общий урон: 12.5"));
    }

    #[test]
    fn periodic_update_does_not_reopen_closed_window() {
        let mut scan = HealthScan::default();
        scan.receive(7, true, false);
        assert_eq!(scan.target, None, "без открытия окно не появляется");
        scan.receive(7, true, true);
        assert_eq!(scan.target, Some(7));
        scan.receive(7, false, false);
        assert!(!scan.active);
    }
}
