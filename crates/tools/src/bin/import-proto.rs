//! Импорт прототипов SS14 (SS14_IMPORT.md §4, задачи IMP.2/IMP.3):
//! парсер YAML c наследованием (parent) и конвертер в наш формат `.ron`.
//!
//! Запуск:
//! `cargo run -p ssr-tools --bin import-proto -- <PrototypesDir> <out.ron> [sprites.txt]`
//!
//! Отчёт (IMP-5): сколько перенесено, сколько файлов/прототипов пропущено и почему;
//! список `sprites.txt` — RSI-пути, которые нужно скопировать в assets.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use serde_yaml_ng::Value;
use ssr_core::prototypes::{Proto, ProtoSet};

/// Максимум итераций разрешения наследования (защита от глубоких цепочек).
const MAX_DEPTH: usize = 64;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: import-proto <PrototypesDir> <out.ron> [sprites.txt]");
        std::process::exit(2);
    }
    let (input, output) = (PathBuf::from(&args[1]), PathBuf::from(&args[2]));
    let sprites_path = args.get(3).map(PathBuf::from);

    let mut raw: HashMap<String, RawProto> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut stats = Stats::default();

    for entry in walkdir::WalkDir::new(&input)
        .into_iter()
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        if entry.path().extension().and_then(|e| e.to_str()) != Some("yml") {
            continue;
        }
        let text = match std::fs::read_to_string(entry.path()) {
            Ok(t) => t,
            Err(e) => {
                stats.file_errors += 1;
                eprintln!("read {}: {e}", entry.path().display());
                continue;
            }
        };
        let docs: Vec<Value> = match serde_yaml_ng::from_str(&text) {
            Ok(docs) => docs,
            Err(e) => {
                // Кастомные теги (`!type:` и пр.) роняют парсер — пропуск с логом.
                stats.parse_skipped += 1;
                eprintln!("skip {}: {e}", entry.path().display());
                continue;
            }
        };
        for doc in docs {
            stats.docs += 1;
            let Some(id) = doc["id"].as_str() else {
                continue;
            };
            let kind = doc["type"].as_str().unwrap_or("entity").to_string();
            if kind != "entity" {
                stats.non_entities += 1;
                continue;
            }
            if raw.contains_key(id) {
                stats.duplicates += 1;
            }
            let parent = doc["parent"].as_str().map(str::to_string);
            let components: Vec<(String, Value)> = doc["components"]
                .as_sequence()
                .map(|list| {
                    list.iter()
                        .filter_map(|c| Some((c["type"].as_str()?.to_string(), c.clone())))
                        .collect()
                })
                .unwrap_or_default();
            let raw_proto = RawProto {
                id: id.to_string(),
                kind,
                parent,
                name: doc["name"].as_str().map(str::to_string),
                description: doc["description"].as_str().map(str::to_string),
                abstract_: doc["abstract"].as_bool().unwrap_or(false),
                components,
                file: entry.path().display().to_string(),
            };
            if raw.insert(id.to_string(), raw_proto).is_none() {
                order.push(id.to_string());
            }
        }
    }
    println!(
        "прочитано прототипов: {} (файлов пропущено: {}, дубликатов: {})",
        raw.len(),
        stats.parse_skipped,
        stats.duplicates
    );

    // Разрешение наследования (двухпроходный merge, ребёнок выигрывает).
    let mut resolved: HashMap<String, Proto> = HashMap::new();
    let mut resolving: Vec<String> = Vec::new();
    let mut cycles = 0usize;
    let ids: Vec<String> = order.clone();
    for id in &ids {
        if let Err(e) = resolve(id, &raw, &mut resolved, &mut resolving, 0) {
            cycles += 1;
            eprintln!("inheritance skip {id}: {e}");
        }
    }
    println!("разрешено: {} (циклов/ошибок: {cycles})", resolved.len());

    // Отсортированный вывод + сбор спрайт-референсов.
    let mut protos: Vec<Proto> = resolved.into_values().collect();
    protos.sort_by(|a, b| a.id.cmp(&b.id));
    let mut sprites: BTreeSet<String> = BTreeSet::new();
    for proto in &protos {
        if let Some(sprite) = &proto.sprite {
            // "path/name.rsi" → папка целиком (state отбрасываем).
            let dir = sprite.split('#').next().unwrap_or(sprite);
            if let Some(rsi) = dir.strip_suffix(".rsi") {
                sprites.insert(format!("{rsi}.rsi"));
            } else {
                sprites.insert(dir.to_string());
            }
        }
    }

    ProtoSet { protos }.save(&output).expect("save protos");
    println!(
        "сохранено: {} прототипов → {}\n  со спрайтом: {}, уникальных RSI: {}",
        ids.len(),
        output.display(),
        sprites.len(),
        sprites.len()
    );
    if let Some(path) = sprites_path {
        let text: String = sprites.iter().cloned().collect::<Vec<_>>().join("\n");
        std::fs::write(&path, text).expect("sprites list");
        println!("список спрайтов: {}", path.display());
    }
}

/// Сырой прототип до наследования.
struct RawProto {
    id: String,
    kind: String,
    parent: Option<String>,
    name: Option<String>,
    description: Option<String>,
    abstract_: bool,
    components: Vec<(String, Value)>,
    file: String,
}

#[derive(Default)]
struct Stats {
    docs: u64,
    non_entities: u64,
    duplicates: u64,
    parse_skipped: u64,
    file_errors: u64,
}

/// Рекурсивно сливает компоненты от родителей к ребёнку.
fn resolve(
    id: &str,
    raw: &HashMap<String, RawProto>,
    out: &mut HashMap<String, Proto>,
    stack: &mut Vec<String>,
    depth: usize,
) -> Result<(), String> {
    if out.contains_key(id) {
        return Ok(());
    }
    if depth > MAX_DEPTH {
        return Err("слишком глубокая цепочка наследования".into());
    }
    if stack.iter().any(|s| s == id) {
        return Err(format!("цикл наследования: {stack:?} -> {id}"));
    }
    let Some(proto) = raw.get(id) else {
        return Err(format!("нет такого прототипа: {id}"));
    };
    stack.push(id.to_string());

    // Сначала родитель — его компоненты станут базой.
    let mut merged: Vec<(String, Value)> = Vec::new();
    if let Some(parent) = &proto.parent
        && raw.contains_key(parent)
    {
        resolve(parent, raw, out, stack, depth + 1)?;
        // Компоненты родителя переносим по цепочке raw-прототипов.
        let mut chain: Vec<&RawProto> = Vec::new();
        let mut cursor = raw.get(parent);
        while let Some(p) = cursor {
            chain.push(p);
            cursor = p.parent.as_ref().and_then(|par| raw.get(par));
        }
        for p in chain.into_iter().rev() {
            for (name, value) in &p.components {
                upsert_component(&mut merged, name, value);
            }
        }
    } else if let Some(parent) = &proto.parent {
        eprintln!("unknown parent {parent} for {id}");
    }

    for (name, value) in &proto.components {
        upsert_component(&mut merged, name, value);
    }
    stack.pop();

    // Извлекаем нужное нам подмножество компонентов (таблица 4.4 документа).
    let mut sprite = None;
    let mut tags = Vec::new();
    let mut size = None;
    let mut equip = None;
    for (name, value) in &merged {
        match name.as_str() {
            "Sprite" => {
                if let Some(path) = value["sprite"].as_str() {
                    let state = value["state"].as_str().unwrap_or("0");
                    sprite = Some(format!("{path}#{state}"));
                }
            }
            "Tag" => {
                if let Some(list) = value["tags"].as_sequence() {
                    tags = list
                        .iter()
                        .filter_map(|t| t.as_str().map(str::to_string))
                        .collect();
                }
            }
            "Item" => {
                size = value["size"].as_str().map(str::to_string);
            }
            "Clothing" => {
                equip = value["slot"].as_str().map(str::to_string);
            }
            _ => {}
        }
    }

    out.insert(
        id.to_string(),
        Proto {
            id: proto.id.clone(),
            parent: proto.parent.clone(),
            name: proto.name.clone(),
            description: proto.description.clone(),
            kind: proto.kind.clone(),
            sprite,
            tags,
            size,
            equip,
            abstract_: proto.abstract_,
        },
    );
    let _ = proto.file.as_str();
    Ok(())
}

/// Merge компонента по имени: ребёнок переопределяет родителя.
fn upsert_component(components: &mut Vec<(String, Value)>, name: &str, value: &Value) {
    match components.iter_mut().find(|(n, _)| n == name) {
        Some(slot) => slot.1 = value.clone(),
        None => components.push((name.to_string(), value.clone())),
    }
}
