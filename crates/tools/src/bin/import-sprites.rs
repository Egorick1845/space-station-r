//! Копирование спрайтов из сборки SS14 в assets (SS14_IMPORT.md IMP-4):
//! читает список относительных путей (RSI-папки и файлы), копирует с
//! сохранением структуры, считает объём и пропуски.
//!
//! Запуск:
//! `cargo run -p ssr-tools --bin import-sprites -- <TexturesRoot> <list.txt> <assetsDest>`

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: import-sprites <TexturesRoot> <list.txt> <assetsDest>");
        std::process::exit(2);
    }
    let source = PathBuf::from(&args[1]);
    let list = PathBuf::from(&args[2]);
    let dest_root = PathBuf::from(&args[3]);

    let text = std::fs::read_to_string(&list).expect("read sprite list");
    let mut copied = 0u64;
    let mut missing = 0u64;
    let mut bytes = 0u64;

    for line in text.lines() {
        let rel = line.trim();
        if rel.is_empty() {
            continue;
        }
        let src = source.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !src.exists() {
            missing += 1;
            eprintln!("нет в источнике: {rel}");
            continue;
        }
        let dst = dest_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if src.is_dir() {
            if let Err(e) = copy_dir(&src, &dst, &mut bytes) {
                eprintln!("copy {rel}: {e}");
                missing += 1;
                continue;
            }
        } else {
            if let Some(parent) = dst.parent()
                && let Err(e) = std::fs::create_dir_all(parent)
            {
                eprintln!("mkdir {}: {e}", parent.display());
                missing += 1;
                continue;
            }
            match std::fs::copy(&src, &dst) {
                Ok(n) => bytes += n,
                Err(e) => {
                    eprintln!("copy {rel}: {e}");
                    missing += 1;
                    continue;
                }
            }
        }
        copied += 1;
    }

    println!(
        "скопировано: {copied}, пропущено: {missing}, объём: {:.1} МБ",
        bytes as f64 / (1024.0 * 1024.0)
    );
}

fn copy_dir(src: &PathBuf, dst: &PathBuf, bytes: &mut u64) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target, bytes)?;
        } else {
            *bytes += std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}
