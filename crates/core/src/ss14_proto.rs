//! Парсер прототипов SS14 (SS14_IMPORT.md §4.2–4.3, задача IMP.2): читает
//! `.yml` из `Resources/Prototypes/**` сборки, разрешает наследование `parent`
//! и отдаёт готовые прототипы сущностей — чтобы спавнить ВСЕ сущности сборки,
//! а не только наш ручной набор в `.ron`.
//!
//! Живёт в ядре: нужно и серверу (команда спавна, валидация), и клиенту
//! (список в меню спавна), и инструментам (`ssr-tools import-proto`, IMP.3).
//!
//! Ограничения осознанные: файлы с YAML-тегами (`!type:`, `!del:`) парсер
//! пропускает с логом — их немного, и они не нужны для спавна; поля, которых
//! мы не знаем, сохраняются в `Value` и не теряются.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;
use serde_yaml_ng::Value;

/// Прототип SS14 (элемент верхнего уровня в файле прототипов).
#[derive(Debug, Clone, Deserialize)]
pub struct Ss14Proto {
    /// `type:` — `entity`, `job`, `recipe` и т.д. Нас интересуют `entity`.
    #[serde(rename = "type", default)]
    pub kind: String,
    /// `id:` — идентификатор прототипа.
    pub id: String,
    /// `parent:` — прототип-родитель (наследование).
    #[serde(default)]
    pub parent: Option<String>,
    /// `abstract: true` — шаблон, не спавнится.
    #[serde(default)]
    pub r#abstract: bool,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub components: Vec<Ss14Comp>,
    /// Неизвестные поля не теряем (пригодятся конвертеру).
    #[serde(flatten)]
    pub rest: Value,
}

/// Компонент прототипа: `- type: Sprite` + произвольные поля.
#[derive(Debug, Clone, Deserialize)]
pub struct Ss14Comp {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(flatten)]
    pub data: Value,
}

impl Ss14Proto {
    /// Компонент по имени типа (последний в списке — как в движке: побеждает
    /// объявленный позже).
    pub fn component(&self, kind: &str) -> Option<&Ss14Comp> {
        self.components.iter().rev().find(|c| c.kind == kind)
    }

    /// Спавнится ли прототип (сущность и не abstract).
    pub fn is_spawnable(&self) -> bool {
        self.kind == "entity" && !self.r#abstract
    }
}

/// Итог загрузки: прототипы и счётчики (лог делает вызывающий — ядро молчит,
/// PLAN.md §6: core без лишних зависимостей).
#[derive(Debug, Default)]
pub struct LoadReport {
    pub protos: Vec<Ss14Proto>,
    /// Файлов не прочитано (теги/битый YAML) — с причиной.
    pub skipped: Vec<(String, String)>,
}

/// Читает все `.yml` под `dir` (обычно `Resources/Prototypes`).
pub fn load_ss14_prototypes(dir: &Path) -> LoadReport {
    let mut report = LoadReport::default();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yml") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            report
                .skipped
                .push((path.display().to_string(), "не читается".into()));
            continue;
        };
        match serde_yaml_ng::from_str::<Vec<Ss14Proto>>(&text) {
            Ok(list) => report.protos.extend(list),
            Err(err) => report
                .skipped
                .push((path.display().to_string(), err.to_string())),
        }
    }
    report
}

/// Разрешает наследование: сливает цепочки `parent` в один прототип.
///
/// Правила движка, которые нам важны: `name`/`description` наследуются, если не
/// переопределены; компоненты ребёнка **дополняют** родительские (совпадение по
/// `type` — побеждает ребёнок); циклы отбрасываются. Возвращает индекс `id →
/// прототип` со разрешёнными (у которых `parent` уже слит).
pub fn resolve_inheritance(protos: &[Ss14Proto]) -> HashMap<String, Ss14Proto> {
    let by_id: HashMap<&str, &Ss14Proto> = protos.iter().map(|p| (p.id.as_str(), p)).collect();
    let mut resolved: HashMap<String, Ss14Proto> = HashMap::new();
    for proto in protos {
        let merged = resolve_one(proto, &by_id, &mut Vec::new());
        resolved.insert(merged.id.clone(), merged);
    }
    resolved
}

/// Сливает один прототип с его предками (рекурсивно, с защитой от циклов).
fn resolve_one(
    proto: &Ss14Proto,
    by_id: &HashMap<&str, &Ss14Proto>,
    chain: &mut Vec<String>,
) -> Ss14Proto {
    if chain.contains(&proto.id) {
        // Цикл: отдаём как есть, без родителя.
        let mut flat = proto.clone();
        flat.parent = None;
        return flat;
    }
    let Some(parent_id) = proto.parent.clone() else {
        return proto.clone();
    };
    let Some(parent) = by_id.get(parent_id.as_str()) else {
        // Родителя нет в наборе — оставляем как есть.
        return proto.clone();
    };
    chain.push(proto.id.clone());
    let base = resolve_one(parent, by_id, chain);
    chain.pop();
    merge(base, proto)
}

/// Дополняет родителя ребёнком: скалярные поля ребёнка важнее, компоненты
/// складываются (по типу побеждает ребёнок).
fn merge(base: Ss14Proto, child: &Ss14Proto) -> Ss14Proto {
    let mut components = base.components;
    for comp in &child.components {
        if let Some(existing) = components.iter_mut().find(|c| c.kind == comp.kind) {
            *existing = comp.clone();
        } else {
            components.push(comp.clone());
        }
    }
    Ss14Proto {
        kind: if child.kind.is_empty() {
            base.kind
        } else {
            child.kind.clone()
        },
        id: child.id.clone(),
        parent: None,
        r#abstract: child.r#abstract,
        name: child.name.clone().or(base.name),
        description: child.description.clone().or(base.description),
        components,
        rest: base.rest,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = r#"
- type: entity
  id: BaseTool
  abstract: true
  name: "базовый инструмент"
  components:
  - type: Item
    size: Normal
  - type: Sprite
    sprite: Objects/Tools/base.rsi

- type: entity
  id: Crowbar
  parent: BaseTool
  name: "лом"
  components:
  - type: Sprite
    sprite: Objects/Tools/crowbar.rsi
    state: icon
"#;

    fn parse() -> Vec<Ss14Proto> {
        serde_yaml_ng::from_str::<Vec<Ss14Proto>>(YAML).expect("yaml")
    }

    #[test]
    fn parses_components_and_flags() {
        let protos = parse();
        assert_eq!(protos.len(), 2);
        let base = &protos[0];
        assert!(base.r#abstract, "abstract читается");
        assert!(!base.is_spawnable(), "абстракт не спавнится");
        assert_eq!(
            base.component("Item").map(|c| c.kind.as_str()),
            Some("Item"),
            "компонент Item на месте"
        );
    }

    #[test]
    fn child_inherits_parent_and_overrides_component() {
        let resolved = resolve_inheritance(&parse());
        let crowbar = resolved.get("Crowbar").expect("Crowbar разрешён");
        assert!(crowbar.is_spawnable(), "лом спавнится");
        assert_eq!(crowbar.name.as_deref(), Some("лом"));
        // Item унаследован от родителя, Sprite — переопределён ребёнком.
        let item = crowbar.component("Item").expect("Item унаследован");
        assert!(item.data.get("size").is_some(), "поля Item дошли");
        let sprite = crowbar.component("Sprite").expect("Sprite есть");
        assert_eq!(
            sprite.data.get("state").and_then(Value::as_str),
            Some("icon"),
            "ребёнок переопределил Sprite"
        );
    }

    /// Прогон по реальной сборке (запускается вручную):
    /// `SSR_SS14_PROTOS=C:\ss14\mini-station-goob\Resources\Prototypes cargo test -p ssr-core --lib -- --ignored counts_real_prototypes --nocapture`
    #[test]
    #[ignore = "нужна сборка SS14 на диске"]
    fn counts_real_prototypes() {
        let Ok(dir) = std::env::var("SSR_SS14_PROTOS") else {
            return;
        };
        let report = load_ss14_prototypes(std::path::Path::new(&dir));
        let resolved = resolve_inheritance(&report.protos);
        let spawnable = resolved.values().filter(|p| p.is_spawnable()).count();
        let entities = resolved.values().filter(|p| p.kind == "entity").count();
        println!(
            "прототипов всего: {}, сущностей: {}, спавнится: {}, файлов пропущено: {}",
            report.protos.len(),
            entities,
            spawnable,
            report.skipped.len()
        );
        for (file, why) in report.skipped.iter().take(5) {
            println!("  пропущен {file}: {}", why.lines().next().unwrap_or(""));
        }
    }

    #[test]
    fn cycle_does_not_hang() {
        let protos: Vec<Ss14Proto> = serde_yaml_ng::from_str(
            r#"
- type: entity
  id: A
  parent: B
- type: entity
  id: B
  parent: A
"#,
        )
        .expect("yaml");
        let resolved = resolve_inheritance(&protos);
        assert_eq!(resolved.len(), 2, "цикл разрешён без зависания");
    }
}
