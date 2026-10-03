//! Генератор файла карты по умолчанию (T2.3, SS14_IMPORT.md IMP-4:
//! импорт/генерация — инструменты, не рантайм).
//!
//! Запуск из корня workspace:
//! ```text
//! cargo run -p ssr-core --bin gen-map -- assets/maps/test.ron
//! ```

use ssr_core::tiles::{MapFile, gen_test_map};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/maps/test.ron".to_string());
    let chunks = gen_test_map();
    // Точки спавна (T2.4): стартовый зал, достаточно далеко друг от друга,
    // чтобы коллайдеры игроков (r=16) не пересекались при спавне.
    let spawn_points = vec![(16.0, 16.0), (80.0, 16.0), (16.0, 80.0), (80.0, 80.0)];
    MapFile::save(std::path::Path::new(&out), "test", spawn_points, &chunks).expect("save map");
    println!(
        "map written: {out} ({} chunks, 128x128 tiles)",
        chunks.len()
    );
}
