//! Генератор первой станции (PLAN.md T5.1): генерируется по шаблону, дальше
//! дорабатывается вручную прямо в .ron.
//!
//! Шаблон: корпус из стен, центральный коридор, четыре отсека на севере
//! (медотсек, инженерный, бар, мостик), техническая зона на юге с техполом,
//! генератором и видимой проводкой; кабели уходят от генератора к дверям и
//! лампам, двери инженерного и мостика требуют доступы из ролей.
//!
//! Запуск из корня workspace:
//! ```text
//! cargo run -p ssr-core --bin gen-station -- assets/maps/station.ron
//! ```

use ssr_core::tiles::{CHUNK_TILES, DoorAccess, MapFile, MapLayout, TileChunk, TileType};

/// Размер станции в тайлах (внутри корпуса).
const WIDTH: i32 = 98;
const HEIGHT: i32 = 60;

/// Виртуальная карта тайлов: (x, y) → TileType, вне — космос.
struct Grid {
    tiles: Vec<TileType>,
}

impl Grid {
    fn new() -> Self {
        Self {
            tiles: vec![TileType::Space; (WIDTH * HEIGHT) as usize],
        }
    }

    fn set(&mut self, x: i32, y: i32, tile: TileType) {
        if (0..WIDTH).contains(&x) && (0..HEIGHT).contains(&y) {
            self.tiles[(y * WIDTH + x) as usize] = tile;
        }
    }

    fn get(&self, x: i32, y: i32) -> TileType {
        if (0..WIDTH).contains(&x) && (0..HEIGHT).contains(&y) {
            self.tiles[(y * WIDTH + x) as usize]
        } else {
            TileType::Space
        }
    }

    /// Прямоугольник: пол (или техпол) внутри, рамка — стены.
    fn room(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, floor: TileType) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let border = x == x0 || y == y0 || x == x1 || y == y1;
                self.set(x, y, if border { TileType::Wall } else { floor });
            }
        }
    }

    /// Проём в стене (дверь).
    fn opening(&mut self, x: i32, y: i32) {
        self.set(x, y, TileType::Floor);
    }

    /// Заполняет прямоугольник тайлом (для техпола внутри комнаты).
    fn fill(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, tile: TileType) {
        for y in y0..=y1 {
            for x in x0..=x1 {
                self.set(x, y, tile);
            }
        }
    }
}

/// Позиция центра тайла в юнитах.
fn center(tx: i32, ty: i32) -> (f32, f32) {
    (tx as f32 * 32.0 + 16.0, ty as f32 * 32.0 + 16.0)
}

fn main() {
    let out = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "assets/maps/station.ron".to_string());

    let mut grid = Grid::new();

    // Корпус и центральный коридор (стены по краям, пол внутри).
    grid.room(1, 1, WIDTH - 2, HEIGHT - 2, TileType::Floor);
    // Коридор: полоса по центру станции.
    grid.fill(2, 26, WIDTH - 3, 31, TileType::Floor);
    grid.fill(2, 25, WIDTH - 3, 25, TileType::Wall);
    grid.fill(2, 32, WIDTH - 3, 32, TileType::Wall);

    // Северные отсеки: медотсек, инженерный, бар, мостик.
    // Координаты: (x0, y0, x1, y1), дверь — проём в южной стене.
    let rooms = [
        (4, 34, 26, 56),  // медотсек
        (30, 34, 52, 56), // инженерный
        (56, 34, 74, 56), // бар
        (78, 34, 96, 56), // мостик
    ];
    let _ = rooms;
    for (x0, y0, x1, y1) in rooms {
        grid.room(x0, y0, x1, y1, TileType::Floor);
    }
    let doors = [
        (15, 33), // медотсек
        (41, 33), // инженерный
        (65, 33), // бар
        (87, 33), // мостик
    ];
    for (x, y) in doors {
        // Проход сквозь стену коридора и стену отсека: дверь в середине.
        grid.opening(x, y - 1);
        grid.opening(x, y);
        grid.opening(x, y + 1);
    }

    // Техническая зона на юге: техпол, сквозь стены — коридор к двери.
    grid.room(4, 4, WIDTH - 5, 22, TileType::Plating);
    let technical_door = (48, 23);
    grid.opening(technical_door.0, 22);
    grid.opening(technical_door.0, 23);
    grid.opening(technical_door.0, 24);
    // Дверь техзоны — в общий список (без доступа, открывается всем).
    let mut all_doors = doors.to_vec();
    all_doors.push(technical_door);

    // Спавны: коридор и отсеки.
    let spawns: Vec<(i32, i32)> = vec![
        // Коридор — напротив дверей отсеков (удобно выходить к нужной двери).
        (15, 28),
        (41, 28),
        (65, 28),
        (87, 28),
        // Внутри отсеков.
        (15, 45),
        (41, 45),
        (65, 45),
        (87, 45),
    ];

    // Электрика: генератор в техзоне, кабели к дверям и лампам.
    let generator = (10, 12);
    let mut cables: Vec<(i32, i32)> = Vec::new();
    let mut lights: Vec<(i32, i32)> = Vec::new();

    // Магистраль по техзоне (видна — здесь техпол), затем подъём к каждой двери.
    for x in 10..=WIDTH - 6 {
        cables.push((x, 12));
    }
    for (index, (dx, _)) in doors.iter().enumerate() {
        let dx = *dx;
        // Подъём от магистрали к двери: по стене техзоны и дальше по коридору.
        let column = 6 + index as i32 * 24 + 9;
        let column = column.clamp(10, WIDTH - 6);
        for y in 12..=26 {
            cables.push((column, y));
        }
        // Горизонтальная подводка к двери по коридору.
        let (from, to) = if column < dx {
            (column, dx)
        } else {
            (dx, column)
        };
        for x in from..=to {
            cables.push((x, 27));
        }
        for y in 28..=33 {
            cables.push((dx, y));
        }
        // Лампа в отсеке на линии кабеля.
        lights.push((dx, 40));
        for y in 34..=40 {
            cables.push((dx, y));
        }
    }
    // Ветка к двери техзоны.
    for y in 12..=23 {
        cables.push((48, y));
    }
    // Лампы в техзоне (видимые провода).
    let _ = &grid;
    lights.push((30, 12));
    lights.push((60, 12));
    lights.push((80, 12));

    // Дедупликация кабелей и ламп.
    cables.sort();
    cables.dedup();
    lights.sort();
    lights.dedup();

    // Разбиение на чанки 32×32 (координаты чанков — мировые/32).
    let tile = CHUNK_TILES as i32;
    let chunks_x = (WIDTH + tile - 1) / tile;
    let chunks_y = (HEIGHT + tile - 1) / tile;
    let mut chunks: Vec<TileChunk> = Vec::new();
    for cy in 0..chunks_y {
        for cx in 0..chunks_x {
            let mut chunk = TileChunk::new(bevy::math::IVec2::new(cx, cy));
            for ly in 0..CHUNK_TILES as i32 {
                for lx in 0..CHUNK_TILES as i32 {
                    let tile = grid.get(cx * CHUNK_TILES as i32 + lx, cy * CHUNK_TILES as i32 + ly);
                    chunk.set_local(lx as u32, ly as u32, tile);
                }
            }
            chunks.push(chunk);
        }
    }

    let layout = MapLayout {
        name: "station".into(),
        spawn_points: spawns.iter().map(|(tx, ty)| center(*tx, *ty)).collect(),
        doors: all_doors.iter().map(|(tx, ty)| center(*tx, *ty)).collect(),
        door_access: vec![
            DoorAccess {
                position: center(41, 33),
                access: "engineering".into(),
            },
            DoorAccess {
                position: center(87, 33),
                access: "command".into(),
            },
        ],
        cables: cables.iter().map(|(tx, ty)| center(*tx, *ty)).collect(),
        generators: vec![{
            let (x, y) = center(generator.0, generator.1);
            (x, y, 40.0)
        }],
        lights: lights.iter().map(|(tx, ty)| center(*tx, *ty)).collect(),
    };

    MapFile::save(std::path::Path::new(&out), layout, &chunks).expect("save station");
    let floors = grid.tiles.iter().filter(|tile| tile.is_walkable()).count();
    println!(
        "станция записана: {out}\n  тайлов: {floors} ходибельных, {} чанков\n  \
         отсеки: медотсек, инженерный, бар, мостик + техзона\n  дверей: {}, кабелей: {}, ламп: {}",
        chunks.len(),
        all_doors.len(),
        cables.len(),
        lights.len()
    );
}
