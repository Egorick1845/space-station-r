# SS14_IMPORT.md — импорт контента из SS14/форка

> **Назначение:** инструкция по использованию YAML-прототипов, карт и RSI-спрайтов из SS14/форка в Rust-проекте.
> **Референс кода:** разделы 2 и 6 PLAN.md (стек, конвенции).
> **Нейминг:** в исходной редакции документа фигурировало кодовое имя «NEXUS» — в этом репозитории оно заменено на SSR (крейт утилит — `ssr-tools`).

## Общая схема

| Тип ресурса | Источник в SS14 | Куда в SSR | Стратегия |
|---|---|---|---|
| Прототипы | `Resources/Prototypes/Entities/**/*.yml` | `assets/prototypes/` | Парсер + конвертер в наш YAML |
| Карты | `Resources/Maps/*.yml` | `assets/maps/` | Экспорт через форк → JSON (рекомендуется) |
| Спрайты | `Resources/Textures/**/*.rsi` | `assets/textures/` | Загрузчик RSI → атлас Bevy |
| Звуки | `Resources/Audio/**/*.ogg` | `assets/audio/` | Простое копирование |

---

## 1. Что берём и во что конвертируем

Источник в SS14 / что это / куда в SSR / стратегия:

- `Resources/Prototypes/Entities/**/*.yml` — сущности: предметы, стены, роли → `assets/prototypes/` — парсер + конвертер в наш YAML
- `Resources/Maps/*.yml` — карты станций → `assets/maps/` — экспорт через форк → JSON (рекомендуется)
- `Resources/Textures/**/*.rsi` — спрайты → `assets/textures/` — загрузчик RSI → атлас Bevy
- `Resources/Audio/**/*.ogg` — звуки → `assets/audio/` — простое копирование

## 2. Лицензия — прочитать до копирования любого файла

| Часть SS14 | Лицензия | Правило для SSR |
|---|---|---|
| Код (Robust Toolbox) | MIT | Свободно |
| Контент (спрайты, карты, звуки, тексты) | CC BY-SA 3.0 | Указать авторство («Art from Space Station 14, CC BY-SA 3.0»); производный контент — тоже CC BY-SA; перед коммерческим релизом заменить на CC0/свой |

Контент SS14 разрешён на стадии прототипа и закрытой альфы. В реестр `ASSETS_LICENSES.md` (в корне) записывать: какой файл откуда взят.

## 3. Откуда брать контент

```bash
# апстрим (свежий контент)
git clone https://github.com/space-wizards/space-station-14

# или форк «Мини-станция» (контент вашей аудитории)
C:\ss14\mini-station-goob   # локальный чекаут
```

Структура `Resources/`:

| Путь | Содержимое |
|---|---|
| `Prototypes/Entities/` | Сущности по категориям (`Objects/Tools/`, `Structures/Walls/`…) |
| `Prototypes/Roles/Jobs/` | Профессии |
| `Prototypes/Catalog/` | Магазины/категории спавна |
| `Prototypes/Recipes/` | Рецепты крафта |
| `Maps/` | Карты |
| `Textures/` | Папки `*.rsi` с `meta.json` + PNG внутри |
| `Audio/` | OGG-файлы |

## 4. YAML-прототипы сущностей

### 4.1 Анатомия формата

```yaml
# Resources/Prototypes/Entities/Objects/Tools/toolbox.yml (упрощённо)
- type: entity            # вид прототипа: entity / job / reagent / recipe...
  id: Crowbar             # уникальный id (строка)
  parent: BaseItem        # наследование от родителя
  name: crowbar
  description: My trusty old crowbar.
  components:             # компоненты — список, каждый начинается с type:
  - type: Sprite
    sprite: Objects/Tools/Crowbar.rsi   # путь к RSI
    state: crowbar                      # имя состояния внутри RSI
  - type: Item
    size: Small
```

Важные факты:

- Файл = YAML-список прототипов.
- `parent` — цепочки глубиной 5–10 уровней (`Crowbar → BaseItem → BaseSmallItem → ...`). Корневые часто `abstract: true` (шаблоны, не для спавна).
- Компоненты в ребёнке переопределяют одноимённые компоненты родителя, недостающие — наследуются.

### 4.2 Rust-парсер

Крейты: `serde_yaml_ng` (форк архивированного serde_yaml, API идентичен), `walkdir`.

```rust
use serde::Deserialize;
use serde_yaml::Value;

/// Один прототип SS14 (элемент верхнего уровня списка)
#[derive(Debug, Deserialize)]
pub struct Ss14Proto {
    #[serde(rename = "type")]
    pub kind: String,                 // "entity", "job", "recipe"...
    pub id: String,
    pub parent: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub abstract_: bool,              // поле "abstract"
    #[serde(default)]
    pub components: Vec<Ss14Comp>,
    #[serde(flatten)]
    pub rest: Value,                  // неизвестные поля — не теряем
}

#[derive(Debug, Deserialize)]
pub struct Ss14Comp {
    #[serde(rename = "type")]
    pub kind: String,                 // "Sprite", "Item"...
    #[serde(flatten)]
    pub data: Value,                  // поля компонента
}

/// Загрузка всех .yml из дерева Prototypes/.
/// Файлы с кастомными YAML-тегами (!type:, !del:) упадут — пропускаем с логом.
pub fn load_ss14_prototypes(dir: &Path) -> Vec<Ss14Proto> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.path().extension().map_or(false, |e| e == "yml") {
            let text = match std::fs::read_to_string(entry.path()) {
                Ok(t) => t,
                Err(e) => { tracing::warn!("read {}: {e}", entry.path().display()); continue; }
            };
            match serde_yaml_ng::from_str::<Vec<Ss14Proto>>(&text) {
                Ok(list) => out.extend(list),
                Err(e) => tracing::warn!("skip {}: {e}", entry.path().display()),
            }
        }
    }
    out
}
```

### 4.3 Разрешение наследования (parent)

Двухпроходный алгоритм: индекс по id → рекурсивный merge с детекцией циклов.

```rust
use std::collections::HashMap;

pub fn resolve_inheritance(
    protos: Vec<Ss14Proto>,
) -> Result<HashMap<String, Ss14Proto>, ImportError> {
    let mut map: HashMap<String, Ss14Proto> =
        protos.into_iter().map(|p| (p.id.clone(), p)).collect();

    fn resolve(id: &str, map: &mut HashMap<String, Ss14Proto>,
               stack: &mut Vec<String>) -> Result<(), ImportError> {
        if stack.iter().any(|s| s == id) {
            return Err(ImportError::Cycle(id.into()));
        }
        let parent_id = match map.get(id).and_then(|p| p.parent.clone()) {
            Some(p) if map.contains_key(&p) => p,
            Some(p) => { tracing::warn!("unknown parent {p} for {id}"); return Ok(()); }
            None => return Ok(()),
        };
        stack.push(id.into());
        resolve(&parent_id, map, stack)?;
        stack.pop();

        let parent = map[&parent_id].clone();
        let child = map.get_mut(id).expect("exists");
        if child.name.is_none()        { child.name = parent.name; }
        if child.description.is_none() { child.description = parent.description; }
        // компоненты: merge по kind, ребёнок выигрывает
        let mut comps = parent.components;
        for c in child.components.drain(..) {
            match comps.iter_mut().find(|p| p.kind == c.kind) {
                Some(slot) => slot.data = c.data,
                None => comps.push(c),
            }
        }
        child.components = comps;
        child.parent = None;            // резолв выполнен
        Ok(())
    }

    let ids: Vec<String> = map.keys().cloned().collect();
    for id in &ids { resolve(id, &mut map, &mut Vec::new())?; }
    Ok(map)
}
```

Дубликат id (форки иногда переопределяют): правило last-wins + `warn!`.

### 4.4 Маппинг компонентов SS14 → SSR

Нам нужны не все ~300 компонентов SS14. Белый список:

| Компонент SS14 | Что читаем | Компонент SSR |
|---|---|---|
| `Sprite` | sprite (путь к RSI), state | `SpriteRef { rsi, state }` |
| `Item` | size (Small/Medium/Large) | `ItemSlot` |
| Description/name | текст | `Description` |
| `Clothing` | slot (Belt, Head...) | `EquipSlot` |
| `Tag` | список тегов | `Tags` |
| `Tool` | qualities | `ToolKind` |
| `Stack` | count, max | `Stackable` |
| остальные (Physics, AtmosDevice, PowerNetwork...) | — | `warn!` + пропуск |

### 4.5 Целевой формат SSR (наш YAML)

```yaml
# assets/prototypes/tools/crowbar.yml
- id: Crowbar                       # сохраняем оригинальный id для трассировки
  name: Лом
  description: Мой старый надёжный лом.
  kind: Tool
  sprite: objects/tools/crowbar.rsi#crowbar
  size: small
  equip: belt
  tags: [melee, tool]
```

Конвертер: SS14-прототип (после resolve) → NexusProto → запись в наш YAML. Запускается один раз как CLI-утилита (`cargo run -p ssr-tools -- import-proto <ss14_dir> <out_dir>`), дальше живём своим форматом.

## 5. Карты SS14

### 5.1 Анатомия формата

```yaml
meta:
  format: 2               # ВАЖНО: версия формата; у форков может отличаться
  name: MiniStation
grids:
- "1":
    settings:
      chunksize: 8        # чанк в тайлах
    chunks:
      "0,0":
        ind: [0, 0]
        version: 1
        tiles: "AAAA..."  # тайлы, base64-упакованные байты
entities:                 # сущности, сгруппированные по прототипу
- proto: WallSolid
  entities:
  - 42
    42:
      parent: 1
      type: Transform
      pos: 12.5,7.5
```

Тайлы в формате 2+ — бинарный блоб (base64), типичная раскладка на тайл — 4 байта: `u16` id типа, `u8` вариант, `u8` флаги. Имена ключей и битовая раскладка зависят от версии формата — у форка могут отличаться от апстрима. Поэтому:

### 5.2 Стратегия A — экспорт через форк (РЕКОМЕНДУЕТСЯ)

Есть работающий C#-форк → использовать родной загрузчик SS14 для идеальной точности и экспортировать в простой JSON. Обратный инжиниринг битовой упаковки не нужен.

```csharp
// Эскиз консольной утилиты внутри solution форка (Tools/MapExporter).
// Адаптируйте под API вашей версии Robust Toolbox.
var result = mapLoader.LoadMap(new ResPath("/Maps/mini.yml"));  // родной загрузчик

var export = new ExportMap { Name = "mini" };
foreach (var grid in result.Grids)
{
    // тайлы: обходим чанки -> (x, y, tileDefinition.ID, variant)
    // сущности: EntityManager -> protoId, worldPosition, rotation
}
File.WriteAllText("mini.json", JsonSerializer.Serialize(export, jsonOpts));
```

Промежуточный JSON (наш формат обмена):

```json
{
  "name": "mini",
  "tile_types": { "0": "Space", "5": "FloorSteel", "12": "WallSolid" },
  "grid": { "width": 128, "height": 128, "tiles": [0, 0, 5, 5, 12] },
  "entities": [
    { "proto": "SpawnPointAssistant", "x": 12.5, "y": 7.5, "rot": 0 }
  ]
}
```

Rust-сторона: тривиальный `serde_json` → `NexusMap` (чанки 32×32 из PLAN.md, T2.3).

### 5.3 Стратегия B — прямой парс YAML в Rust

Если трогать C# нельзя. Два шага:

1. **Dumper** — CLI-утилита, которая печатает дерево `serde_yaml::Value` из реальной карты форка. Так вы узнаёте точные имена ключей вашей версии формата.
2. Парсер под выявленную структуру + декодер тайлов:

```rust
let bytes = base64::engine::general_purpose::STANDARD.decode(&tiles_b64)?;
for chunk4 in bytes.chunks_exact(4) {
    let packed = u32::from_le_bytes(chunk4.try_into().unwrap());
    let type_id = (packed & 0xFFFF) as u16;          // сверить с dumper!
    let variant = ((packed >> 16) & 0xFF) as u8;
    let _flags  = (packed >> 24) as u8;
    // type_id -> имя тайла через словарь формата карты
}
```

⚠️ Порядок битов обязательно сверить с dumper-выводом и исходником MapSerializer в форке. Стратегия A избавляет от этого шага целиком.

### 5.4 Маппинг прототипов карты

Сущности карты ссылаются на id прототипов (`WallSolid`, `SpawnPointAssistant`). Конвертер карты использует ту же таблицу маппинга, что и 4.4. Неизвестный прототип → пустой тайл/ничего + `warn!` со счётчиком (по окончании импорта — список «топ-20 неперенесённых»).

## 6. RSI-спрайты

### 6.1 Две формы одного формата

| Форма | Где встречается | Структура |
|---|---|---|
| Директория `Name.rsi/` | Репозиторий SS14/форка | папка с `meta.json` + PNG |
| ZIP `Name.rsi` | Упакованные ресурс-паки | тот же контент внутри zip |

Загрузчик обязан поддерживать обе.

### 6.2 meta.json

```json
{
  "version": 1,
  "size": { "x": 32, "y": 32 },
  "states": [
    { "name": "crowbar", "directions": 1, "delays": [[1.0]] },
    { "name": "equipped-BELT", "directions": 4, "delays": [[1.0]] }
  ]
}
```

- `directions`: 1, 4 или 8 — количество ракурсов.
- `delays`: длительность кадров — анимации задаются здесь.
- Кадры всех направлений лежат в одном PNG горизонтальной полосой (`size.x` × кадры × направления). Порядок (кадр↔направление) проверить визуально, разрезав один файл.

### 6.3 Загрузчик (обе формы)

```rust
pub struct Rsi {
    pub size: (u32, u32),
    pub states: Vec<RsiState>,
}

pub struct RsiState {
    pub name: String,
    pub directions: u32,
    pub frames: Vec<image::DynamicImage>, // кадры (направления внутри кадра)
    pub delays: Vec<f32>,
}

pub fn load_rsi(path: &Path) -> Result<Rsi, ImportError> {
    let (meta_json, pngs): (serde_json::Value, HashMap<String, Vec<u8>>) = match path {
        p if p.is_dir() => read_dir_rsi(p)?,                 // форма 1: папка
        p => read_zip_rsi(p)?,                               // форма 2: zip (крейт `zip`)
    };

    let size = (
        meta_json["size"]["x"].as_u64().unwrap_or(32) as u32,
        meta_json["size"]["y"].as_u64().unwrap_or(32) as u32,
    );
    let mut states = Vec::new();
    for st in meta_json["states"].as_array().cloned().unwrap_or_default() {
        let name = st["name"].as_str().unwrap_or_default().to_string();
        let directions = st["directions"].as_u64().unwrap_or(1) as u32;
        let delays = st["delays"].as_array()
            .map(|d| d.iter().filter_map(|f| f.as_f64()).map(|f| f as f32).collect())
            .unwrap_or_default();
        // PNG состояния: "<name>.png" (или "<name>-0.png"... — учесть варианты)
        let frames = decode_state_frames(&pngs, &name, size, directions)?;
        states.push(RsiState { name, directions, frames, delays });
    }
    Ok(Rsi { size, states })
}
```

### 6.4 Интеграция с Bevy

- Упаковка кадров всех состояний RSI в один атлас (`TextureAtlasLayout`) — 1 draw call на RSI, критично для Android (задача MA.5 в мастер-плане).
- Индексация: `rsi_path#state` → (атлас, индекс) — строится ресурсом `RsiRegistry` при старте.
- Соглашение ссылок: как в SS14 — `objects/tools/crowbar.rsi#crowbar`.

## 7. Аудио и прочее

| Ресурс | Действие | Код |
|---|---|---|
| `.ogg` звуки | Копирование как есть | Bevy (bevy_audio/rodio) играет OGG нативно |
| `.ftl` локализация | Игнорировать | Тексты берём из name/description прототипов |
| Шейдеры `.swsl` | Игнорировать | Движок другой |

## 8. Соглашения импорта (ADR)

| # | Правило | Причина |
|---|---|---|
| IMP-1 | 1 тайл SS14 = 1.0 юнита мира = 1 м. | Позиции карт импортируются без пересчёта |
| IMP-2 | Оригинальные id прототипов SS14 сохраняются при импорте | Трассировка, репорты багов («сломан Crowbar») |
| IMP-3 | Пути RSI сохраняются в нижнем регистре, как в SS14 | Linux регистрозависим; SS14-пути уже каноничны |
| IMP-4 | Импорт — CLI-инструмент (`ssr-tools`), не рантайм | Импортированное один раз попадает в `assets/` и версионироваться git'ом |
| IMP-5 | Каждый запуск импорта пишет отчёт: сколько перенесено, сколько пропущено и почему | Контроль качества конвертера |
| IMP-6 | Всё скопированное из SS14 регистрируется в ASSETS_LICENSES.md | CC BY-SA |

## 9. Задачи для ИИ

Выполнять по одной, после T0.4 из PLAN.md. Шаблон промпта — раздел 7 PLAN.md.

### IMP.1 — RSI-загрузчик

Задача: IMP.1 RSI-загрузчик (см. раздел 6).

- Крейты: `zip`, `image`, `serde_json`, `walkdir`.
- Реализовать `load_rsi` с поддержкой папка- и zip-формы RSI.
- Интеграция Bevy: `RsiRegistry` (Resource), упаковка в атлас, lookup по `"path/to/name.rsi#state"`.
- Тест: реальный `Crowbar.rsi` в `tests/fixtures` (скопировать из SS14), проверка size, числа состояний, раскладки кадров.
- Критерий приёмки: спрайт ломика отображается в окне Bevy.

### IMP.2 — Парсер прототипов с наследованием

Задача: IMP.2 Prototype loader (раздел 4).

- Крейты: `serde_yaml_ng`, `walkdir`, `tracing`.
- Структуры `Ss14Proto`/`Ss14Comp`, `load_ss14_prototypes`, `resolve_inheritance` с merge компонентов и детекцией циклов.
- Тесты: цепочка parent из 3 уровней, переопределение компонента, циклическая ссылка → `ImportError::Cycle`, abstract не спавнится.

### IMP.3 — Конвертер прототипов

Задача: IMP.3 Proto converter (разделы 4.4–4.5, 8).

- Маппинг-таблица компонентов SS14 → SSR (таблица 4.4).
- CLI: `cargo run -p ssr-tools -- import-proto <ss14_dir> <out_dir>`.
- Выход: YAML формата 4.5 + отчёт IMP-5 (перенесено/пропущено).
- Запуск на 3 папках SS14: `Objects/Tools`, `Structures/Walls`, `Objects/Containers`.

### IMP.4 — Экспорт карты

Задача: IMP.4 Map import (раздел 5).

- Стратегия A: человек добавляет в форк C#-утилиту MapExporter по эскизу 5.2 — ИИ генерирует ПОЛНЫЙ C#-файл с TODO-метками под API форка.
- Rust-сторона: `serde_json` → NexusMap (чанки 32×32, задача T2.3 PLAN.md).
- Критерий: карта из mini.json рендерится в клиенте, спавн-точки найдены.

## 10. Чек-лист подводных камней

- Кастомные YAML-теги (`!type:`, `!del:`) роняют serde-парсер → пропуск файла + лог, не падение.
- `abstract: true` — не спавнить, использовать только как родителя.
- Дубликаты id в форках → last-wins + warn.
- `parent` ссылается на отсутствующий id → warn, пропустить связь.
- Формат карты (`meta.format`) у форка ≠ апстриму → сначала dumper/экспорт через форк.
- Битовая раскладка тайлов — сверять с MapSerializer форка, не доверять документации наизусть.
- Позиции сущностей — относительно грида; chunksize брать из settings карты.
- RSI: направления 1/4/8; анимации — через delays; порядок кадр/направление проверить визуально.
- Variant тайлов (один тип — разные спрайты) — хранить и рандомизировать при рендере.
- Регистр путей — Linux-сервер регистрозависим, сохранять пути SS14 как есть.
- Лицензия CC BY-SA: ASSETS_LICENSES.md с первого же скопированного файла.

**Первый шаг: IMP.1 (RSI-загрузчик).** Критерий успеха: лом из SS14 в окне Bevy. После этого весь остальной пайплайн (прототипы → карты) встаёт на рельсы.
