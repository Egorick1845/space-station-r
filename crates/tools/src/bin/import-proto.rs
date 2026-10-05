//! Импорт прототипов SS14 (SS14_IMPORT.md §4, задачи IMP.2/IMP.3):
//! парсер YAML c наследованием (`parent`, включая МУЛЬТИнаследование) и
//! конвертер в наш формат `.ron`.
//!
//! Запуск:
//! `cargo run -p ssr-tools --bin import-proto -- <PrototypesDir> <out.ron> [sprites.txt]`
//!
//! Семантика наследования — как в RobustToolbox (`PrototypeManager.YamlLoad.cs` +
//! `SerializationManager.Composition.cs:179-223`): сливаются YAML-МАППИНГИ, а не
//! объекты. Ключи компонента наследуются поштучно (ребёнок перекрывает только те
//! поля, что указал сам), маппинги внутри поля сливаются по ключам, списки —
//! заменяются целиком (кроме `components`, который в движке помечен
//! `AlwaysPushInheritance`). Приоритет: ребёнок > parent[0] > parent[1] > …
//!
//! Отчёт (IMP-5): сколько перенесено, сколько файлов/прототипов пропущено и почему;
//! список `sprites.txt` — RSI-пути, которые нужно скопировать в assets.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use serde_yaml_ng::Value;
use ssr_core::prototypes::{
    Proto, ProtoAmmoProvider, ProtoCartridge, ProtoFixture, ProtoGun, ProtoItemSlot, ProtoLight,
    ProtoMagazineVisuals, ProtoMelee, ProtoProjectile, ProtoSet, ProtoSmooth, ProtoStack,
    ProtoStorage,
};

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
    // 5-й аргумент `components` — дописать сырой дамп неразобранных компонентов
    // (файл вырастает в ~40 раз; нужно только инструментам).
    let with_components = args.iter().any(|arg| arg == "components");

    let mut raw: HashMap<String, RawProto> = HashMap::new();
    // Прототипы стека (`type: stack`) — из них берём `maxCount` для `Stack`.
    let mut stacks: HashMap<String, u32> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut stats = Stats::default();
    let mut files: Vec<PathBuf> = Vec::new();

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
        files.push(entry.path().to_path_buf());
    }
    // Детерминированный порядок (в движке файлы шаффлятся, но дубликат id —
    // ошибка: `PrototypeManager.YamlLoad.cs:409-428`; нам нужен воспроизводимый
    // результат, поэтому сортируем пути).
    files.sort();

    for path in files {
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                stats.file_errors += 1;
                eprintln!("read {}: {e}", path.display());
                continue;
            }
        };
        let docs: Vec<Value> = match serde_yaml_ng::from_str(&text) {
            Ok(docs) => docs,
            Err(e) => {
                // Кастомные теги (`!type:` и пр.) роняют парсер — пропуск с логом.
                stats.parse_skipped += 1;
                eprintln!("skip {}: {e}", path.display());
                continue;
            }
        };
        for doc in docs {
            stats.docs += 1;
            let Some(id) = doc["id"].as_str() else {
                continue;
            };
            let kind = doc["type"].as_str().unwrap_or("entity").to_string();
            if kind == "stack" {
                // `StackPrototype.MaxCount` (`Content.Shared/Stacks/StackPrototype.cs:52`).
                if let Some(max) = doc["maxCount"].as_u64() {
                    stacks.insert(id.to_string(), max as u32);
                }
                continue;
            }
            if kind != "entity" {
                stats.non_entities += 1;
                continue;
            }
            if raw.contains_key(id) {
                stats.duplicates += 1;
                eprintln!("duplicate id {id} ({}), оставляем первый", path.display());
                continue;
            }
            let parents: Vec<String> = match &doc["parent"] {
                Value::String(single) => vec![single.clone()],
                Value::Sequence(list) => list
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect(),
                _ => Vec::new(),
            };
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
                parents,
                name: doc["name"].as_str().map(str::to_string),
                description: doc["description"].as_str().map(str::to_string),
                suffix: doc["suffix"].as_str().map(str::to_string),
                categories: doc["categories"]
                    .as_sequence()
                    .map(|list| {
                        list.iter()
                            .filter_map(|value| value.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default(),
                abstract_: doc["abstract"].as_bool().unwrap_or(false),
                components,
                file: path.display().to_string(),
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

    // Разрешение наследования (мерж YAML-маппингов, ребёнок выигрывает).
    let mut resolved: HashMap<String, Proto> = HashMap::new();
    let mut resolving: Vec<String> = Vec::new();
    let mut cycles = 0usize;
    let ids: Vec<String> = order.clone();
    for id in &ids {
        if let Err(e) = resolve(
            id,
            &raw,
            &stacks,
            &mut resolved,
            &mut resolving,
            0,
            with_components,
        ) {
            cycles += 1;
            eprintln!("inheritance skip {id}: {e}");
        }
    }
    println!("разрешено: {} (циклов/ошибок: {cycles})", resolved.len());

    // Сортированный вывод + сбор спрайт-референсов.
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

    let with_sprite = protos.iter().filter(|proto| proto.sprite.is_some()).count();
    let total = protos.len();
    ProtoSet { protos }.save(&output).expect("save protos");
    println!(
        "сохранено: {} прототипов → {}\n  со спрайтом: {}, уникальных RSI: {}",
        total,
        output.display(),
        with_sprite,
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
    parents: Vec<String>,
    name: Option<String>,
    description: Option<String>,
    suffix: Option<String>,
    categories: Vec<String>,
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
    stacks: &HashMap<String, u32>,
    out: &mut HashMap<String, Proto>,
    stack: &mut Vec<String>,
    depth: usize,
    with_components: bool,
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

    // Сначала родители (слева направо), затем сам прототип: приоритет
    // «ребёнок > parent[0] > parent[1] > …» (`Composition.cs:40-58`).
    let mut merged: Vec<(String, Value)> = Vec::new();
    for parent in &proto.parents {
        if !raw.contains_key(parent) {
            eprintln!("unknown parent {parent} for {id}");
            continue;
        }
        resolve(parent, raw, stacks, out, stack, depth + 1, with_components)?;
        // Компоненты родителя берём по его raw-цепочке (уже слитой для него).
        let parent_merged = merged_components(parent, raw, depth + 1)?;
        merge_components(&mut merged, &parent_merged);
    }
    merge_components(&mut merged, &proto.components);
    stack.pop();

    // Извлекаем нужное нам подмножество компонентов (таблица 4.4 документа).
    let mut sprite = None;
    let mut tags = Vec::new();
    let mut size = None;
    let mut equip = None;
    let mut storage = None;
    let mut clothing_slots = Vec::new();
    let mut tools = Vec::new();
    let mut stack_component = None;
    let mut pullable = false;
    let mut body_type = None;
    let mut light = None;
    let mut components = Vec::new();
    let mut is_item = false;
    let mut smooth = None;
    let mut surface = false;
    let mut fixtures: Vec<ProtoFixture> = Vec::new();
    let mut climbable = false;
    let mut icon_state = None;
    let mut gun = None;
    let mut cartridge = None;
    let mut projectile = None;
    let mut ammo_provider = None;
    let mut melee = None;
    let mut item_slots: Vec<ProtoItemSlot> = Vec::new();
    let mut ammo_counter = false;
    let mut magazine_visuals = None;
    let mut projectile_lifetime = 10.0f32;
    for (name, value) in &merged {
        match name.as_str() {
            "Sprite" => {
                // Спрайт может быть задан и на компоненте, и в первом слое
                // (`layers[0]`), и прямой текстурой (`texture`).
                let layer = value["layers"]
                    .as_sequence()
                    .and_then(|layers| layers.first());
                let path = value["sprite"]
                    .as_str()
                    .or_else(|| layer.and_then(|layer| layer["sprite"].as_str()))
                    .or_else(|| value["texture"].as_str())
                    .or_else(|| layer.and_then(|layer| layer["texture"].as_str()));
                let state = value["state"]
                    .as_str()
                    .or_else(|| layer.and_then(|layer| layer["state"].as_str()))
                    .unwrap_or("0");
                if let Some(path) = path {
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
                // `ItemComponent.Size` по умолчанию `Small` (`ItemComponent.cs:23`).
                size = Some(value["size"].as_str().unwrap_or("Small").to_string());
                is_item = true;
            }
            "Icon" => {
                // Состояние иконки для меню спавна (у столов `full`, тогда как в
                // мире состояние даёт `IconSmooth`).
                if let Some(state) = value["state"].as_str() {
                    icon_state = Some(state.to_string());
                }
            }
            "IconSmooth" => {
                if let Some(key) = value["key"].as_str() {
                    smooth = Some(ProtoSmooth {
                        key: key.to_string(),
                        base: value["base"].as_str().unwrap_or_default().to_string(),
                    });
                }
            }
            "PlaceableSurface" => surface = true,
            "Climbable" => climbable = true,
            "Fixtures" => {
                if let Some(map) = value["fixtures"].as_mapping() {
                    for (_name, fixture) in map {
                        // `shape: !type:PhysShapeAabb { bounds: "-0.45,-0.45,0.45,0.45" }`
                        let shape = untag(&fixture["shape"]);
                        let bounds = shape["bounds"]
                            .as_str()
                            .and_then(parse_bounds)
                            .unwrap_or((-0.5, -0.5, 0.5, 0.5));
                        fixtures.push(ProtoFixture {
                            bounds,
                            // `Fixture.Hard` по умолчанию `true` (`Fixture.cs:85`).
                            hard: fixture["hard"].as_bool().unwrap_or(true),
                            density: fixture["density"].as_f64().unwrap_or(0.0) as f32,
                            layer: string_list(&fixture["layer"]),
                            mask: string_list(&fixture["mask"]),
                        });
                    }
                }
            }
            "Clothing" => {
                equip = value["slot"].as_str().map(str::to_string);
                if let Some(flags) = value["slots"].as_str() {
                    clothing_slots = flags
                        .split(',')
                        .map(|flag| flag.trim().to_string())
                        .filter(|flag| !flag.is_empty())
                        .collect();
                }
            }
            "Storage" => {
                let grid = value["grid"]
                    .as_sequence()
                    .map(|list| {
                        list.iter()
                            .filter_map(|box_value| {
                                let text = box_value.as_str()?;
                                let parts: Vec<i32> = text
                                    .split(',')
                                    .filter_map(|part| part.trim().parse().ok())
                                    .collect();
                                (parts.len() == 4).then(|| (parts[0], parts[1], parts[2], parts[3]))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                storage = Some(ProtoStorage {
                    grid,
                    max_item_size: value["maxItemSize"].as_str().map(str::to_string),
                });
            }
            "Tool" => {
                if let Some(list) = value["qualities"].as_sequence() {
                    tools = list
                        .iter()
                        .filter_map(|quality| quality.as_str().map(str::to_string))
                        .collect();
                }
            }
            "Stack" => {
                let kind = value["stackType"].as_str().unwrap_or_default().to_string();
                if !kind.is_empty() {
                    let count = value["count"].as_u64().unwrap_or(30) as u32;
                    let max_count = value["maxCountOverride"]
                        .as_u64()
                        .map(|max| max as u32)
                        .or_else(|| stacks.get(&kind).copied());
                    stack_component = Some(ProtoStack {
                        kind,
                        count,
                        max_count,
                    });
                }
            }
            "Pullable" => pullable = true,
            // --- Оружие, патроны, снаряды, ближний бой (W-план) ---
            "Gun" => {
                gun = Some(ProtoGun {
                    fire_rate: value["fireRate"].as_f64().unwrap_or(5.0) as f32,
                    selected_mode: value["selectedMode"]
                        .as_str()
                        .unwrap_or("SemiAuto")
                        .to_string(),
                    modes: string_list(&value["availableModes"]),
                    sound: sound_path(&value["soundGunshot"]),
                    projectile_speed: value["projectileSpeed"].as_f64().map(|v| v as f32),
                    angle_increase: value["angleIncrease"].as_f64().map(|v| v as f32),
                    angle_decay: value["angleDecay"].as_f64().map(|v| v as f32),
                    min_angle: value["minAngle"].as_f64().map(|v| v as f32),
                    max_angle: value["maxAngle"].as_f64().map(|v| v as f32),
                });
            }
            "CartridgeAmmo" => {
                if let Some(proto) = value["proto"].as_str() {
                    cartridge = Some(ProtoCartridge {
                        proto: proto.to_string(),
                        spent: value["spent"].as_bool().unwrap_or(false),
                    });
                }
            }
            "Projectile" => {
                projectile = Some(ProtoProjectile {
                    damage: damage_types(&value["damage"]),
                    impact_effect: value["impactEffect"].as_str().map(str::to_string),
                    sound_hit: sound_path(&value["soundHit"]),
                    lifetime: projectile_lifetime,
                    delete_on_hit: value["deleteOnHit"].as_bool().unwrap_or(true),
                });
            }
            "BallisticAmmoProvider" | "MagazineAmmoProvider" => {
                ammo_provider = Some(ProtoAmmoProvider {
                    capacity: value["capacity"].as_u64().unwrap_or(0) as u32,
                    proto: value["proto"].as_str().map(str::to_string),
                    sound_insert: sound_path(&value["soundInsert"]),
                    sound_eject: sound_path(&value["soundEject"]),
                });
            }
            "MeleeWeapon" => {
                melee = Some(ProtoMelee {
                    damage: damage_types(&value["damage"]),
                    // Дефолты `MeleeWeaponComponent`: range 1.5, attackRate 1.5.
                    range: value["range"].as_f64().unwrap_or(1.5) as f32,
                    attack_rate: value["attackRate"].as_f64().unwrap_or(1.5) as f32,
                    sound_hit: sound_path(&value["soundHit"]),
                });
            }
            "ItemSlots" => {
                if let Some(map) = value["slots"].as_mapping() {
                    for (slot_id, slot) in map {
                        let Some(slot_id) = slot_id.as_str() else {
                            continue;
                        };
                        item_slots.push(ProtoItemSlot {
                            id: slot_id.to_string(),
                            starting_item: slot["startingItem"].as_str().map(str::to_string),
                            priority: slot["priority"].as_i64().unwrap_or(0) as i32,
                            whitelist_tags: string_list(&slot["whitelist"]["tags"]),
                        });
                    }
                }
            }
            "AmmoCounter" => ammo_counter = true,
            "MagazineVisuals" => {
                magazine_visuals = Some(ProtoMagazineVisuals {
                    mag_state: value["magState"].as_str().unwrap_or("mag").to_string(),
                    steps: value["steps"].as_u64().unwrap_or(1) as u32,
                    zero_visible: value["zeroVisible"].as_bool().unwrap_or(true),
                });
            }
            "TimedDespawn" => {
                // Время жизни снаряда (`BaseBullet` — 10 с).
                projectile_lifetime = value["lifetime"].as_f64().unwrap_or(10.0) as f32;
            }
            "Physics" => {
                body_type = value["bodyType"].as_str().map(str::to_string);
            }
            "PointLight" => {
                light = Some(ProtoLight {
                    radius: value["radius"].as_f64().unwrap_or(5.0) as f32,
                    energy: value["energy"].as_f64().unwrap_or(1.0) as f32,
                    color: value["color"].as_str().map(str::to_string),
                    enabled: value["enabled"].as_bool().unwrap_or(true),
                });
            }
            _ => {}
        }
        // Неразобранные компоненты сохраняем ТОЛЬКО по флагу `components`:
        // сырой дамп всех компонентов раздувает файл в 40 раз (125 МБ против
        // 3 МБ), а нужен он лишь инструментам/отладке. Полный дамп всегда можно
        // пересобрать этим же импортёром из сборки.
        if with_components
            && !matches!(
                name.as_str(),
                "Sprite" | "Tag" | "Item" | "Clothing" | "Storage" | "Tool" | "Stack" | "Pullable"
            )
        {
            components.push((name.clone(), yaml_to_ron(value)));
        }
    }

    out.insert(
        id.to_string(),
        Proto {
            id: proto.id.clone(),
            parent: proto.parents.first().cloned(),
            name: proto.name.clone(),
            description: proto.description.clone(),
            kind: proto.kind.clone(),
            sprite,
            tags,
            size,
            equip,
            abstract_: proto.abstract_,
            storage,
            clothing_slots,
            tools,
            stack: stack_component,
            pullable,
            categories: proto.categories.clone(),
            suffix: proto.suffix.clone(),
            body_type,
            light,
            components,
            is_item,
            smooth,
            surface,
            fixtures,
            climbable,
            icon_state,
            gun,
            cartridge,
            projectile,
            ammo_provider,
            melee,
            item_slots,
            ammo_counter,
            magazine_visuals,
        },
    );
    let _ = proto.file.as_str();
    Ok(())
}

/// Компоненты прототипа, слитые по его цепочке наследования.
fn merged_components(
    id: &str,
    raw: &HashMap<String, RawProto>,
    depth: usize,
) -> Result<Vec<(String, Value)>, String> {
    if depth > MAX_DEPTH {
        return Err("слишком глубокая цепочка наследования".into());
    }
    let Some(proto) = raw.get(id) else {
        return Ok(Vec::new());
    };
    let mut merged: Vec<(String, Value)> = Vec::new();
    for parent in &proto.parents {
        if !raw.contains_key(parent) {
            continue;
        }
        let parent_merged = merged_components(parent, raw, depth + 1)?;
        merge_components(&mut merged, &parent_merged);
    }
    merge_components(&mut merged, &proto.components);
    Ok(merged)
}

/// SS14-мерж компонентов (`ComponentRegistrySerializer.PushInheritance`): компонент
/// того же типа сливается ПО КЛЮЧАМ (ребёнок перекрывает свои поля), отсутствующий
/// у ребёнка компонент наследуется целиком. Маппинги внутри поля сливаются по
/// ключам (`CombineMappings`), списки заменяются целиком (Default-поведение).
fn merge_components(merged: &mut Vec<(String, Value)>, child: &[(String, Value)]) {
    for (name, value) in child {
        match merged.iter_mut().find(|(existing, _)| existing == name) {
            Some(slot) => slot.1 = merge_value(&slot.1, value),
            None => merged.push((name.clone(), value.clone())),
        }
    }
}

/// Мерж значений одного компонента: маппинги — по ключам, остальное — ребёнок
/// побеждает целиком.
fn merge_value(parent: &Value, child: &Value) -> Value {
    match (parent, child) {
        (Value::Mapping(parent_map), Value::Mapping(child_map)) => {
            let mut result = parent_map.clone();
            for (key, value) in child_map {
                match result.get(key) {
                    Some(existing) => {
                        let merged = merge_value(existing, value);
                        result.insert(key.clone(), merged);
                    }
                    None => {
                        result.insert(key.clone(), value.clone());
                    }
                }
            }
            Value::Mapping(result)
        }
        _ => child.clone(),
    }
}

/// Снимает YAML-тег (`!type:PhysShapeAabb`) — значения читаются из `.value`.
fn untag(value: &Value) -> &Value {
    match value {
        Value::Tagged(tagged) => &tagged.value,
        other => other,
    }
}

/// Строковый список значения (`layer: [TableLayer]` или одна строка).
fn string_list(value: &Value) -> Vec<String> {
    match value {
        Value::String(text) => vec![text.clone()],
        Value::Sequence(list) => list
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// `"-0.45,-0.45,0.45,0.45"` → кортеж границ фикстуры.
fn parse_bounds(text: &str) -> Option<(f32, f32, f32, f32)> {
    let parts: Vec<f32> = text
        .split(',')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    (parts.len() == 4).then(|| (parts[0], parts[1], parts[2], parts[3]))
}

/// Путь звука из значения прототипа: строка, `{ path: … }` или `{ collection: … }`
/// (коллекцию оставляем как есть — клиент ищет её в `SoundCollections`).
fn sound_path(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    value["path"].as_str().map(str::to_string)
}

/// Урон по типам: `{ types: { Piercing: 16, Heat: 2 } }` → `[("Piercing", 16), …]`.
fn damage_types(value: &Value) -> Vec<(String, f32)> {
    let Some(map) = value["types"].as_mapping() else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(kind, amount)| Some((kind.as_str()?.to_string(), amount.as_f64()? as f32)))
        .collect()
}

/// `serde_yaml_ng::Value` → `ron::Value` (чтобы сохранить неразобранные
/// компоненты в наш `.ron`).
fn yaml_to_ron(value: &Value) -> ron::Value {
    match value {
        Value::Null => ron::Value::Unit,
        Value::Bool(value) => ron::Value::Bool(*value),
        Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                ron::Value::Number(ron::value::Number::new(int))
            } else {
                ron::Value::Number(ron::value::Number::new(number.as_f64().unwrap_or(0.0)))
            }
        }
        Value::String(text) => ron::Value::String(text.clone()),
        Value::Sequence(list) => ron::Value::Seq(list.iter().map(yaml_to_ron).collect()),
        Value::Mapping(map) => ron::Value::Map(
            map.iter()
                .map(|(key, value)| (yaml_to_ron(key), yaml_to_ron(value)))
                .collect(),
        ),
        Value::Tagged(tagged) => yaml_to_ron(&tagged.value),
    }
}
