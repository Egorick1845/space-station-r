//! Импорт русских имён сущностей из локали сборки (`Resources/Locale/ru-RU`):
//! ключи `ent-{Id} = {имя}` (4700+ файлов .ftl) → `assets/prototypes/names_ru.ron`
//! (`HashMap<Id, Имя>`). Запуск:
//! `cargo run -p ssr-tools --bin import-names -- <locale/ru-RU> <out.ron>`

use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: import-names <locale/ru-RU dir> <out.ron>");
        std::process::exit(2);
    }
    let (dir, output) = (&args[1], &args[2]);

    let mut files: Vec<PathBuf> = Vec::new();
    collect_ftl(Path::new(dir), &mut files);
    files.sort();

    let mut names: HashMap<String, String> = HashMap::new();
    let mut multiline = 0u64;
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let mut current: Option<String> = None;
        for line in text.lines() {
            // Комментарии и пустые строки рвут накопление продолжений.
            if line.starts_with('#') || line.trim().is_empty() {
                current = None;
                continue;
            }
            if let Some(rest) = line.strip_prefix('-') {
                // Продолжение многострочного значения (отступ обычно есть,
                // но .ftl допускает и `-` в начале строки).
                let _ = rest;
                if let Some(key) = &current {
                    names.entry(key.clone()).or_default();
                    multiline += 1;
                }
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim().to_string();
                let value = value.trim().to_string();
                // Нас интересуют ТОЛЬКО имена сущностей `ent-{ProtoId}`.
                if key.starts_with("ent-") && !value.is_empty() {
                    let id = key.trim_start_matches("ent-").to_string();
                    names.insert(id.clone(), value);
                    current = Some(id);
                } else {
                    current = None;
                }
            } else if current.is_some() {
                // Строка-продолжение значения: имя сущности обычно однострочное,
                // многострочные пропускаем (это описания/тексты).
                multiline += 1;
            }
        }
    }

    let ron = ron::ser::to_string_pretty(&names, ron::ser::PrettyConfig::default())
        .expect("serialize names");
    std::fs::write(output, ron).expect("write names");
    println!(
        "русских имён: {} (файлов .ftl: {}, многострочных пропущено: {}) → {output}",
        names.len(),
        files.len(),
        multiline
    );
}

/// Рекурсивно собирает все `.ftl` в каталоге.
fn collect_ftl(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ftl(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "ftl") {
            out.push(path);
        }
    }
}
