//! RSI-загрузчик (SS14_IMPORT.md §6, задача IMP.1).
//!
//! Чистый (без Bevy) разбор RSI: папка `Name.rsi/` или zip `Name.rsi`.
//! Раскладка направлений в SS14 — сетка, не полоса: 1→1×1, 2→2×1, 4→2×2,
//! 8→4×2; кадры анимации добавляются строками снизу.

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
    #[error("state {0}: PNG {1}x{2}, ожидалось {3}x{4} при сетке {5}x{6} и {7} кадрах")]
    SheetSize(String, u32, u32, u32, u32, u32, u32, u32),
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

/// Состояние RSI: `directions` направлений, `frames` кадров анимации.
///
/// `sheet` — целый PNG состояния; позиция (кадр, направление) — сетка [`dir_grid`].
#[derive(Debug)]
pub struct RsiState {
    pub name: String,
    pub directions: u32,
    pub frames: u32,
    /// `delays[frame][direction]` — длительности кадров анимации, сек.
    pub delays: Vec<Vec<f32>>,
    pub sheet: DynamicImage,
}

impl RsiState {
    /// Сетка направлений (колонок, строк) одного кадра.
    pub fn grid(&self) -> (u32, u32) {
        dir_grid(self.directions)
    }

    /// Индекс ячейки (кадр, направление) в атласе, собранном по порядку кадров.
    pub fn cell_index(&self, frame: u32, direction: u32) -> u32 {
        let (cols, rows) = self.grid();
        frame * rows * cols + direction
    }
}

/// Сетка направлений SS14: 1→1×1, 2→2×1, 3→3×1, 4→2×2, 8→4×2, остальное — квадрат.
pub fn dir_grid(directions: u32) -> (u32, u32) {
    match directions {
        1 => (1, 1),
        2 => (2, 1),
        3 => (3, 1),
        4 => (2, 2),
        8 => (4, 2),
        d => {
            let cols = (d as f64).sqrt().ceil() as u32;
            (cols, d.div_ceil(cols))
        }
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
        // delays[frame][direction]; отсутствует у статичных состояний → 1 кадр.
        let delays: Vec<Vec<f32>> = st["delays"]
            .as_array()
            .map(|frames| {
                frames
                    .iter()
                    .map(|f| {
                        f.as_array()
                            .map(|d| {
                                d.iter()
                                    .filter_map(|v| v.as_f64())
                                    .map(|v| v as f32)
                                    .collect()
                            })
                            .unwrap_or_default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        let frames = delays.len().max(1) as u32;

        let png_name = format!("{name}.png");
        let bytes = pngs
            .get(&png_name)
            .ok_or(RsiError::MissingPng(name.clone()))?;
        let sheet = image::load_from_memory(bytes).map_err(|e| RsiError::Png(name.clone(), e))?;

        let state = RsiState {
            name,
            directions,
            frames,
            delays,
            sheet,
        };
        verify_sheet_size(&state, size)?;
        states.push(state);
    }

    Ok(Rsi { size, states })
}

/// Проверка размера листа по сетке направлений и числу кадров.
fn verify_sheet_size(state: &RsiState, (w, h): (u32, u32)) -> Result<(), RsiError> {
    let (cols, rows) = state.grid();
    let expected = (w * cols, h * rows * state.frames);
    let (width, height) = (state.sheet.width(), state.sheet.height());
    if (width, height) != expected {
        return Err(RsiError::SheetSize(
            state.name.clone(),
            width,
            height,
            expected.0,
            expected.1,
            cols,
            rows,
            state.frames,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Реальный RSI из сборки мини-станции (см. ASSETS_LICENSES.md).
    fn crowbar_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("repo root")
            .join("assets/sprites/ss14/Objects/Tools/crowbar.rsi")
    }

    #[test]
    fn crowbar_folder_form() {
        let rsi = load_rsi(&crowbar_path()).expect("load crowbar.rsi");
        assert_eq!(rsi.size, (32, 32));
        // meta.json монтировки: 14 состояний.
        assert_eq!(rsi.states.len(), 14);

        let icon = rsi.state("icon").expect("icon state");
        assert_eq!(icon.directions, 1);
        assert_eq!(icon.frames, 1);
        assert_eq!(icon.sheet.width(), 32);

        let inhand = rsi.state("inhand-left").expect("inhand-left state");
        assert_eq!(inhand.directions, 4);
        assert_eq!(inhand.grid(), (2, 2));
        assert_eq!(inhand.sheet.width(), 64);
        assert_eq!(inhand.sheet.height(), 64);
        // Порядок направлений SS14: юг, восток, север, запад — индексы 0..3.
        assert_eq!(inhand.cell_index(0, 3), 3);
    }

    #[test]
    fn dir_grid_matches_ss14_layout() {
        assert_eq!(dir_grid(1), (1, 1));
        assert_eq!(dir_grid(2), (2, 1));
        assert_eq!(dir_grid(4), (2, 2));
        assert_eq!(dir_grid(8), (4, 2));
    }
}
