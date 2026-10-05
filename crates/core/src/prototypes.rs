//! Прототипы контента (ADR-6): наши структуры + загрузка из .ron.
//!
//! Импорт из SS14 (парсер с наследованием и конвертер) — IMP.2/IMP.3
//! (SS14_IMPORT.md §4), инструмент `ssr-tools --bin import-proto`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Один прототип: сущность/предмет/роль и т.п.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Proto {
    /// Оригинальный id SS14 (IMP-2: сохраняем для трассировки).
    pub id: String,
    /// Родитель (в уже сконвертированном наборе отсутствует — слит).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Вид прототипа SS14: entity/job/reagent/...
    #[serde(default)]
    pub kind: String,
    /// Спрайт: `"path/to/name.rsi#state"` (или `.png`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprite: Option<String>,
    /// Теги (для рецептов/взаимодействий).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Размер предмета (Small/Medium/Large).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    /// `Item.shape` — ЯВНАЯ форма предмета в клетках инвентаря (`Box2i` со
    /// включительными границами). Перекрывает `defaultShape` размера: у лома
    /// `Normal` + `shape: [0,0,0,1]` = 1×2, у стали `Normal` без формы = 2×2.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shape: Vec<(i32, i32, i32, i32)>,
    /// Слот экипировки (belt/head/...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equip: Option<String>,
    /// Абстрактный прототип — только шаблон, не спавнить.
    #[serde(default)]
    pub abstract_: bool,
    /// `Storage` — сетка (Box2i со включительными границами) и максимальный
    /// размер вкладываемого предмета (`maxItemSize`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storage: Option<ProtoStorage>,
    /// `Clothing.slots` — флаги слотов (`INNERCLOTHING`, `HEAD`, `POCKET`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub clothing_slots: Vec<String>,
    /// `Tool.qualities` — качества инструмента (`Prying`, `Anchoring`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
    /// `Stack` — тип стека и количество.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<ProtoStack>,
    /// Есть ли компонент `Pullable` (`BaseItem`/`BaseStructure`).
    #[serde(default)]
    pub pullable: bool,
    /// `categories` (в т.ч. `HideSpawnMenu` — прототип не показывать в меню).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// `suffix` (подпись в редакторских меню).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suffix: Option<String>,
    /// `Physics.bodyType` (`Static`/`Dynamic`/`Kinematic`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_type: Option<String>,
    /// `PointLight` — радиус/энергия/цвет, если компонент есть.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<ProtoLight>,
    /// Компоненты, которые мы ещё не разобрали по полям: `(тип, поля)` —
    /// чтобы ничего не терять при импорте (IMP.3, задача конвертера).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<(String, ron::Value)>,
    /// Есть ли компонент `Item` (предмет). Иначе прототип — структура
    /// (`TableSteel`, `Airlock`, …), и спавнить его нужно как структуру.
    #[serde(default)]
    pub is_item: bool,
    /// `IconSmooth` — соединение соседних структур (`key`, `base` состояния).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smooth: Option<ProtoSmooth>,
    /// `PlaceableSurface` — на структуру можно класть предметы.
    #[serde(default)]
    pub surface: bool,
    /// `Fixtures` — коллизия структуры (границы в тайлах, слои, плотность).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fixtures: Vec<ProtoFixture>,
    /// `Climbable` — через структуру можно перелезть (в сборке — `Climbable`).
    #[serde(default)]
    pub climbable: bool,
    /// `Icon.state` — состояние спрайта для иконки в меню спавна (`full` у столов).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_state: Option<String>,
    /// `Gun` — параметры стрельбы (скорострельность, режимы, звук).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gun: Option<ProtoGun>,
    /// `CartridgeAmmo` — патрон: какой снаряд порождает и «стреляный» ли он.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cartridge: Option<ProtoCartridge>,
    /// `Projectile` — снаряд: урон, эффект попадания, звук, время жизни.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projectile: Option<ProtoProjectile>,
    /// `BallisticAmmoProvider` — ёмкость магазина/коробки и звуки.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ammo_provider: Option<ProtoAmmoProvider>,
    /// `MeleeWeapon` — урон и скорость удара в ближнем бою.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub melee: Option<ProtoMelee>,
    /// `ItemSlots` — слоты оружия (`gun_magazine`, `gun_chamber`, …).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub item_slots: Vec<ProtoItemSlot>,
    /// `AmmoCounter` — счётчик патронов в HUD.
    #[serde(default)]
    pub ammo_counter: bool,
    /// `MagazineVisuals` — состояние спрайта магазина по числу патронов.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magazine_visuals: Option<ProtoMagazineVisuals>,
}

/// `Gun` (`Content.Shared/Weapons/Ranged/Components/GunComponent.cs`): числа из
/// прототипов (у пистолета `fireRate: 6`, режимы `SemiAuto`/`FullAuto`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoGun {
    /// Выстрелов в секунду (`fireRate`).
    pub fire_rate: f32,
    /// Выбранный режим (`selectedMode`).
    pub selected_mode: String,
    /// Доступные режимы (`availableModes`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<String>,
    /// Звук выстрела (`soundGunshot.path`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<String>,
    /// Скорость снаряда в единицах/с (`projectileSpeed`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projectile_speed: Option<f32>,
    /// Рост угла за выстрел (`angleIncrease`), спад (`angleDecay`) и границы.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle_increase: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub angle_decay: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_angle: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_angle: Option<f32>,
}

/// `CartridgeAmmo` (`Content.Shared/Weapons/Ranged/Components/CartridgeAmmoComponent.cs`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoCartridge {
    /// Прототип снаряда (`proto: BulletPistol`).
    pub proto: String,
    /// Стреляная гильза (`spent`) — патрон уже нельзя выстрелить.
    #[serde(default)]
    pub spent: bool,
}

/// `Projectile` (`Content.Shared/Projectiles/ProjectileComponent.cs`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProtoProjectile {
    /// Урон по типам (`damage.types`): `[("Piercing", 16)]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub damage: Vec<(String, f32)>,
    /// Эффект попадания (`impactEffect`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub impact_effect: Option<String>,
    /// Звук попадания (`soundHit.path`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound_hit: Option<String>,
    /// Время жизни снаряда, с (`TimedDespawn.lifetime`, у `BaseBullet` — 10).
    #[serde(default)]
    pub lifetime: f32,
    /// Исчезает при попадании (`deleteOnHit`, по умолчанию true).
    #[serde(default)]
    pub delete_on_hit: bool,
}

/// `BallisticAmmoProvider` (`.../Components/BallisticAmmoProviderComponent.cs`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoAmmoProvider {
    /// Ёмкость (`capacity`, у `MagazinePistol` — 12).
    pub capacity: u32,
    /// Прототип, которым наполняется (`proto`), если задан.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proto: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound_insert: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound_eject: Option<String>,
}

/// `MeleeWeapon` (`Content.Shared/Weapons/Melee/MeleeWeaponComponent.cs`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ProtoMelee {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub damage: Vec<(String, f32)>,
    /// Дальность удара (`range`, по умолчанию 1.5).
    #[serde(default)]
    pub range: f32,
    /// Ударов в секунду (`attackRate`, по умолчанию 1.5).
    #[serde(default)]
    pub attack_rate: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound_hit: Option<String>,
}

/// Слот оружия из `ItemSlots` (магазин, патронник).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoItemSlot {
    /// Имя слота (`gun_magazine`, `gun_chamber`).
    pub id: String,
    /// Стартовый предмет в слоте (`startingItem: MagazinePistol`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub starting_item: Option<String>,
    /// Приоритет (`priority`, у магазина 2, у патронника 1).
    #[serde(default)]
    pub priority: i32,
    /// Теги, которые принимает слот (`whitelist.tags`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub whitelist_tags: Vec<String>,
}

/// `MagazineVisuals` — состояние спрайта магазина (`mag-0`, `mag-1`, …).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoMagazineVisuals {
    /// База состояния (`magState: mag`).
    pub mag_state: String,
    /// Число ступеней (`steps`).
    pub steps: u32,
    /// Показывать пустой магазин (`zeroVisible`).
    pub zero_visible: bool,
}

/// `IconSmooth` прототипа: соседние структуры с тем же `key` соединяются
/// спрайтами `{base}{маска}` (у столов `key: state`, `base: "state_"`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoSmooth {
    pub key: String,
    pub base: String,
}

/// Фикстура прототипа (`Fixtures.fixtures.<id>`): форма и слои коллизии.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoFixture {
    /// Границы `PhysShapeAabb.bounds` в тайлах (`-0.45,-0.45,0.45,0.45` у стола).
    pub bounds: (f32, f32, f32, f32),
    /// `hard: true` — твёрдая фикстура (блокирует). В сборке по умолчанию `true`.
    pub hard: bool,
    #[serde(default)]
    pub density: f32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub layer: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mask: Vec<String>,
}

/// `Storage` прототипа: сетка и максимальный размер вкладываемого.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoStorage {
    /// Боксы сетки `(left, bottom, right, top)` — границы включительно.
    pub grid: Vec<(i32, i32, i32, i32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_item_size: Option<String>,
}

/// `Stack` прототипа: тип стека, количество и максимум из прототипа стека.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoStack {
    pub kind: String,
    #[serde(default)]
    pub count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_count: Option<u32>,
}

/// `PointLight` прототипа (числа как в сборке).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProtoLight {
    pub radius: f32,
    pub energy: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default)]
    pub enabled: bool,
}

/// Набор прототипов (`assets/prototypes/*.ron`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProtoSet {
    pub protos: Vec<Proto>,
}

impl ProtoSet {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("parse {}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
            .map_err(|e| format!("serialize: {e}"))?;
        std::fs::write(path, text).map_err(|e| format!("write {}: {e}", path.display()))
    }

    /// Индекс id → прототип.
    pub fn by_id(&self) -> HashMap<&str, &Proto> {
        self.protos.iter().map(|p| (p.id.as_str(), p)).collect()
    }

    /// Прототип по ЛОКАЛИЗОВАННОМУ ИМЕНИ предмета (`Item.name` у сущностей —
    /// это `name` прототипа; `id` клиенту не известен).
    pub fn by_name(&self, name: &str) -> Option<&Proto> {
        self.protos
            .iter()
            .find(|proto| proto.name.as_deref() == Some(name))
    }

    /// `Storage` прототипа по имени предмета (`StorageComponent`: сетка и
    /// максимальный размер вкладываемого).
    pub fn storage_of(&self, name: &str) -> Option<&ProtoStorage> {
        self.by_name(name)?.storage.as_ref()
    }

    /// Габариты предмета по его размеру-прототипу (`Item.size` → `itemSize`).
    pub fn size_cells(&self, id: &str) -> Option<(u8, u8)> {
        let proto = self.protos.iter().find(|proto| proto.id == id)?;
        let size_id = proto.size.as_deref()?;
        crate::item_size::cells_of(size_id)
    }

    /// Показывать ли прототип в меню спавна: не abstract и нет категории
    /// `HideSpawnMenu` (`EntitySpawningUIController.BuildEntityList:203-211`).
    pub fn spawnable(&self, id: &str) -> bool {
        self.protos
            .iter()
            .find(|proto| proto.id == id)
            .is_some_and(|proto| {
                !proto.abstract_
                    && !proto
                        .categories
                        .iter()
                        .any(|category| category == "HideSpawnMenu")
            })
    }
}
