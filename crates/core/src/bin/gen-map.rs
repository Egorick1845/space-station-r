//! Генератор файла карты по умолчанию (T2.3, SS14_IMPORT.md IMP-4:
//! импорт/генерация — инструменты, не рантайм).
//!
//! Запуск из корня workspace:
//! ```text
//! cargo run -p ssr-core --bin gen-map -- assets/maps/test.ron
//! ```

use ssr_core::tiles::{DoorAccess, MapFile, MapLayout, TileType, gen_test_map};

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/maps/test.ron".to_string());
    let mut chunks = gen_test_map();
    // Коридор электрики — техпол (плиты без покрытия): на нём видно кабели,
    // под обычным полом провод скрыт (T4.4+).
    for chunk in chunks.iter_mut() {
        for ly in 0..ssr_core::tiles::CHUNK_TILES {
            for lx in 0..ssr_core::tiles::CHUNK_TILES {
                let tx = chunk.coords.x * ssr_core::tiles::CHUNK_TILES as i32 + lx as i32;
                let ty = chunk.coords.y * ssr_core::tiles::CHUNK_TILES as i32 + ly as i32;
                let corridor = (0..=4).contains(&tx) && ty == 1 || tx == 4 && (0..=2).contains(&ty);
                if corridor && chunk.get_local(lx, ly) == TileType::Floor {
                    chunk.set_local(lx, ly, TileType::Plating);
                }
            }
        }
    }
    // Точки спавна (T2.4): стартовый зал, достаточно далеко друг от друга,
    // чтобы коллайдеры игроков (r=16) не пересекались при спавне.
    let spawn_points = vec![(16.0, 16.0), (80.0, 16.0), (16.0, 80.0), (80.0, 80.0)];
    // Двери (T3.1): первая стоит на пути от точки спавна (16,16) — упор в закрытую дверь.
    let doors = vec![(144.0, 16.0), (144.0, 48.0), (144.0, 80.0)];
    // Доступ (T4.2): первая дверь — инженерная, остальные открыты всем.
    let door_access = vec![DoorAccess {
        position: (144.0, 16.0),
        access: "engineering".into(),
    }];
    // Электрика (T4.4): генератор у коридора, кабель к дверям, лампы на кабеле.
    // Баланс: 20 кВт генерации против 3 двери × 4 + 2 лампы × 1 = 14 кВт.
    // Позиции — центры тайлов (как у дверей).
    let tile = |tx: f32, ty: f32| (tx * 32.0 + 16.0, ty * 32.0 + 16.0);
    let generators = vec![{
        let (x, y) = tile(1.0, 1.0);
        (x, y, 20.0)
    }];
    let cables: Vec<(f32, f32)> = [
        (0.0, 1.0),
        (1.0, 1.0),
        (2.0, 1.0),
        (3.0, 1.0),
        (4.0, 1.0),
        (4.0, 0.0),
        (4.0, 2.0),
    ]
    .iter()
    .map(|(tx, ty)| tile(*tx, *ty))
    .collect();
    let lights: Vec<(f32, f32)> = [(2.0, 1.0), (3.0, 1.0)]
        .iter()
        .map(|(tx, ty)| tile(*tx, *ty))
        .collect();
    MapFile::save(
        std::path::Path::new(&out),
        MapLayout {
            name: "test".into(),
            spawn_points,
            doors,
            door_access,
            cables,
            generators,
            lights,
        },
        &chunks,
    )
    .expect("save map");
    println!(
        "map written: {out} ({} chunks, 128x128 tiles)",
        chunks.len()
    );
}
