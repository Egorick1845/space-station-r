//! RSI-загрузчик (SS14_IMPORT.md §6, задача IMP.1).
//!
//! Модель раскладки — как в движке (RobustToolbox `RsiLoading.cs`):
//! лист состоит из ячеек `size × size`, которые обходятся построчно
//! (слева-вправо, сверху-вниз): сначала все кадры нулевого направления,
//! затем следующего. `delays` в meta.json — по одному массиву на направление
//! (`delays[dir]` = длительности кадров этого направления); без `delays` —
//! 1 кадр на направление. Размер листа обязан быть кратен размеру кадра.

use std::collections::HashMap;
use std::path::Path;

use image::DynamicImage;
use thiserror::Error;

/// Ошибки загрузки RSI (PLAN.md §6: thiserror в core).
#[derive(Debug, Error)]
pub enum RsiError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("meta.json: {0}")]
    Meta(#[from] serde_json::Error),
    #[error("zip: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("png {0}: {1}")]
    Png(String, image::ImageError),
    #[error("state {0}: нет PNG в RSI")]
    MissingPng(String),
    #[error("state {0}: недопустимое число направлений {1} (ожидается 1, 4 или 8)")]
    Directions(String, u32),
    #[error("state {0}: PNG {1}x{2} не кратен размеру кадра {3}x{4}")]
    SheetSize(String, u32, u32, u32, u32),
    #[error("state {0}: ячеек {1}, а кадров требуется {2} (направлений {3}, кадров {4:?})")]
    CellCount(String, u32, u32, u32, Vec<u32>),
}

/// Загруженный RSI.
#[derive(Debug)]
pub struct Rsi {
    /// Размер одной ячейки-кадра (обычно 32×32).
    pub size: (u32, u32),
    pub states: Vec<RsiState>,
}

impl Rsi {
    pub fn state(&self, name: &str) -> Option<&RsiState> {
        self.states.iter().find(|s| s.name == name)
    }
}

/// Состояние RSI: лист-изображение с ячейками (направление × кадр).
#[derive(Debug)]
pub struct RsiState {
    pub name: String,
    /// 1, 4 или 8 (как в движке).
    pub directions: u32,
    /// Кадров на каждое направление (из `delays`, минимум 1).
    pub frames_per_direction: Vec<u32>,
    /// Длительности кадров по направлениям (для анимации, сек).
    pub delays: Vec<Vec<f32>>,
    pub sheet: DynamicImage,
}

impl RsiState {
    /// Наибольшее число кадров среди направлений.
    pub fn frames(&self) -> u32 {
        self.frames_per_direction.iter().copied().max().unwrap_or(1)
    }

    /// Индекс ячейки (направление, кадр) в порядке обхода движка.
    pub fn cell_index(&self, direction: u32, frame: u32) -> u32 {
        let mut index = 0u32;
        for d in 0..direction.min(self.directions) {
            index += self
                .frames_per_direction
                .get(d as usize)
                .copied()
                .unwrap_or(1);
        }
        index + frame
    }

    /// Сетка ячеек листа: (колонок, строк).
    pub fn sheet_grid(&self, size: (u32, u32)) -> (u32, u32) {
        (self.sheet.width() / size.0, self.sheet.height() / size.1)
    }
}

/// Загрузка RSI: папка (`Foo.rsi/`) или zip (`Foo.rsi`).
pub fn load_rsi(path: &Path) -> Result<Rsi, RsiError> {
    let (meta_json, pngs) = if path.is_dir() {
        read_dir_rsi(path)?
    } else {
        read_zip_rsi(path)?
    };

    let size = (
        meta_json["size"]["x"].as_u64().unwrap_or(32) as u32,
        meta_json["size"]["y"].as_u64().unwrap_or(32) as u32,
    );

    let mut states = Vec::new();
    for st in meta_json["states"].as_array().cloned().unwrap_or_default() {
        let Some(name) = st["name"].as_str().map(str::to_string) else {
            continue;
        };
        let directions = st["directions"].as_u64().unwrap_or(1) as u32;
        if !matches!(directions, 1 | 4 | 8) {
            return Err(RsiError::Directions(name, directions));
        }
        // delays[dir] = длительности кадров направления; пусто/нет — 1 кадр.
        let mut delays: Vec<Vec<f32>> = st["delays"]
            .as_array()
            .map(|dirs| {
                dirs.iter()
                    .map(|d| {
                        d.as_array()
                            .map(|f| {
                                f.iter()
                                    .filter_map(|v| v.as_f64())
                                    .map(|v| v as f32)
                                    .collect()
                            })
                            .unwrap_or_else(|| vec![1.0])
                    })
                    .collect()
            })
            .unwrap_or_default();
        if delays.is_empty() {
            delays = vec![vec![1.0]; directions as usize];
        }
        for d in delays.iter_mut() {
            if d.is_empty() {
                d.push(1.0);
            }
        }
        let frames_per_direction: Vec<u32> = delays.iter().map(|d| d.len() as u32).collect();

        let png_name = format!("{name}.png");
        let bytes = pngs
            .get(&png_name)
            .ok_or(RsiError::MissingPng(name.clone()))?;
        let sheet = image::load_from_memory(bytes).map_err(|e| RsiError::Png(name.clone(), e))?;

        let state = RsiState {
            name,
            directions,
            frames_per_direction,
            delays,
            sheet,
        };
        verify_sheet(&state, size)?;
        states.push(state);
    }

    Ok(Rsi { size, states })
}

/// Проверка: лист кратен кадру и содержит ЯЧЕЕК НЕ МЕНЬШЕ (направления × кадры).
/// Движок допускает лишние ячейки в листе (они просто не используются,
/// например `deny_unlit` в basic.rsi: 6 ячеек при 5 кадрах).
fn verify_sheet(state: &RsiState, (w, h): (u32, u32)) -> Result<(), RsiError> {
    let (width, height) = (state.sheet.width(), state.sheet.height());
    if width % w != 0 || height % h != 0 {
        return Err(RsiError::SheetSize(state.name.clone(), width, height, w, h));
    }
    let cells = (width / w) * (height / h);
    let expected: u32 = state.frames_per_direction.iter().sum();
    if cells < expected {
        return Err(RsiError::CellCount(
            state.name.clone(),
            cells,
            expected,
            state.directions,
            state.frames_per_direction.clone(),
        ));
    }
    Ok(())
}

/// Форма 1: папка `Name.rsi/` c meta.json и PNG.
fn read_dir_rsi(path: &Path) -> Result<(serde_json::Value, HashMap<String, Vec<u8>>), RsiError> {
    let mut meta_json = None;
    let mut pngs = HashMap::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match path.extension().and_then(|e| e.to_str()) {
            Some("json") if path.file_name().is_some_and(|n| n == "meta.json") => {
                meta_json = Some(serde_json::from_slice(&std::fs::read(&path)?)?);
            }
            Some("png") => {
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default()
                    .to_string();
                pngs.insert(name, std::fs::read(&path)?);
            }
            _ => {}
        }
    }
    let meta_json = meta_json.unwrap_or_else(|| serde_json::from_str("{}").expect("empty json"));
    Ok((meta_json, pngs))
}

/// Форма 2: zip `Name.rsi` с тем же содержимым.
fn read_zip_rsi(path: &Path) -> Result<(serde_json::Value, HashMap<String, Vec<u8>>), RsiError> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut meta_json = None;
    let mut pngs = HashMap::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry
            .name()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_string();
        match name.rsplit('.').next() {
            Some("json") if name == "meta.json" => {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut bytes)?;
                meta_json = Some(serde_json::from_slice(&bytes)?);
            }
            Some("png") => {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut bytes)?;
                pngs.insert(name, bytes);
            }
            _ => {}
        }
    }
    let meta_json = meta_json.unwrap_or_else(|| serde_json::from_str("{}").expect("empty json"));
    Ok((meta_json, pngs))
}

/// Имена состояний RSI без чтения PNG (только meta.json) — источник списков
/// причёсок и бород. В SS14 списки дают прототипы маркингов, у нас — сам RSI,
/// поэтому новый стиль в ассетах сразу виден и клиенту, и серверу.
pub fn state_names(path: &Path) -> Result<Vec<String>, RsiError> {
    let meta_json = if path.is_dir() {
        serde_json::from_slice(&std::fs::read(path.join("meta.json"))?)?
    } else {
        read_zip_meta(path)?
    };
    Ok(meta_json["states"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|state| state["name"].as_str().map(str::to_string))
        .collect())
}

/// meta.json из zip-архива RSI (для [`state_names`]).
fn read_zip_meta(path: &Path) -> Result<serde_json::Value, RsiError> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if entry.is_dir() {
            continue;
        }
        if entry.name().rsplit('/').next() == Some("meta.json") {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut bytes)?;
            return Ok(serde_json::from_slice(&bytes)?);
        }
    }
    Ok(serde_json::from_str("{}").expect("empty json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset_rsi(rel: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root")
            .join("assets/sprites/ss14")
            .join(rel)
    }

    #[test]
    fn crowbar_folder_form() {
        let rsi = load_rsi(&asset_rsi("Objects/Tools/crowbar.rsi")).expect("load crowbar.rsi");
        assert_eq!(rsi.size, (32, 32));
        assert_eq!(rsi.states.len(), 14);

        let icon = rsi.state("icon").expect("icon state");
        assert_eq!(icon.directions, 1);
        assert_eq!(icon.frames(), 1);
        assert_eq!(icon.sheet_grid(rsi.size), (1, 1));

        let inhand = rsi.state("inhand-left").expect("inhand-left state");
        assert_eq!(inhand.directions, 4);
        assert_eq!(inhand.frames_per_direction, vec![1, 1, 1, 1]);
        assert_eq!(inhand.sheet_grid(rsi.size), (2, 2));
        // Порядок направлений SS14: юг, восток, север, запад — индексы 0..3.
        assert_eq!(inhand.cell_index(0, 0), 0);
        assert_eq!(inhand.cell_index(3, 0), 3);
    }

    #[test]
    fn monkey_and_animated_airlock() {
        let monkey = load_rsi(&asset_rsi("Mobs/Animals/monkey.rsi")).expect("load monkey.rsi");
        let state = monkey.state("monkey").expect("monkey state");
        assert_eq!(state.directions, 4);
        assert_eq!(state.sheet_grid(monkey.size), (2, 2));

        // Дверь: анимированные состояния (delays) — кадры идут подряд в листе.
        let door = load_rsi(&asset_rsi("Structures/Doors/Airlocks/Standard/basic.rsi"))
            .expect("load basic.rsi");
        let closed = door.state("closed").expect("closed");
        assert_eq!(closed.directions, 1);
        assert_eq!(closed.frames(), 1);
        let closing = door.state("closing").expect("closing");
        assert_eq!(closing.frames(), 6);
        assert_eq!(closing.cell_index(0, 0), 0);
        assert_eq!(closing.cell_index(0, 5), 5);
        let sparks = door.state("sparks_broken").expect("sparks_broken");
        assert_eq!(sparks.frames(), 7);
        // Лишние ячейки в листе допустимы (движок их игнорирует).
        let deny = door.state("deny_unlit").expect("deny_unlit");
        assert_eq!(deny.frames(), 5);
    }
}
