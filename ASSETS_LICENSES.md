# ASSETS_LICENSES.md — реестр происхождения ассетов

Правило IMP-6 (SS14_IMPORT.md §8): каждый скопированный из SS14/форка файл
регистрируется здесь. Контент SS14 — CC BY-SA 3.0; перед коммерческим релизом
заменить на CC0/собственный.

Источник по умолчанию: `C:\ss14\mini-station-goob` (github.com/ministation/mini-station-goob),
`Resources/Textures` — см. также assets/sprites/ss14/ATTRIBUTION.md.

| Путь в assets/ | Источник (Resources/Textures/…) | Лицензия | Скопировано |
|---|---|---|---|
| sprites/ss14/Mobs/Animals/monkey.rsi/ | Mobs/Animals/monkey.rsi | CC BY-SA 3.0 (апстрим SS14) | 03.10.2026 |
| sprites/ss14/Mobs/Ghosts/ghost_human.rsi/ | Mobs/Ghosts/ghost_human.rsi | CC BY-SA 3.0 (апстрим SS14) | 03.10.2026 |
| sprites/ss14/Objects/Tools/crowbar.rsi/ | Objects/Tools/crowbar.rsi | CC BY-SA 3.0 — tgstation (см. copyright в meta.json) | 03.10.2026 |
| sprites/ss14/Tiles/steel.png | Tiles/steel.png | CC BY-SA 3.0 (апстрим SS14) | 03.10.2026 |
| sprites/ss14/Tiles/dark.png | Tiles/dark.png | CC BY-SA 3.0 (апстрим SS14) | 03.10.2026 |
| sprites/ss14/Tiles/blue.png | Tiles/blue.png | CC BY-SA 3.0 (апстрим SS14) | 03.10.2026 |
| sprites/ss14/Structures/Walls/solid.rsi/ | Structures/Walls/solid.rsi | CC BY-SA 3.0 — TauCetiStation (см. copyright в meta.json) | 04.10.2026 |
| sprites/ss14/Structures/Doors/Airlocks/Standard/basic.rsi/ | Structures/Doors/Airlocks/Standard/basic.rsi | CC BY-SA 3.0 (апстрим SS14) | 04.10.2026 |
| maps/ss14/empty.yml | Maps/Test/empty.yml | CC BY-SA 3.0 (апстрим SS14) | 04.10.2026 |
| maps/ss14/floor3x3.yml | Maps/Test/floor3x3.yml | CC BY-SA 3.0 (апстрим SS14) | 04.10.2026 |
| maps/ss14/admin_test_arena.yml | Maps/Test/admin_test_arena.yml | CC BY-SA 3.0 (апстрим SS14) | 04.10.2026 |
| maps/ss14/aspid.yml | Maps/_Mini/aspid.yml (карта «Мини-станции») | CC BY-SA 3.0 | 04.10.2026 |

## Шрифты

- `assets/fonts/NotoSans-Regular.ttf`, `NotoSans-Bold.ttf` — из сборки
  мини-станции (`Resources/Fonts/NotoSans`), лицензия SIL OFL 1.1
  (`LICENSE-NotoSans.txt`); ставится шрифтом по умолчанию в клиенте (кириллица).

## Массовый импорт (04.10.2026, инструменты ssr-tools)

- `assets/prototypes_ss14.ron` — 9258 прототипов сущностей, сконвертированы из
  `Resources/Prototypes/Entities` сборки (`import-proto`), CC BY-SA 3.0.
- `assets/sprites/ss14/**` — 2533 RSI-набора, на которые ссылаются прототипы
  (список `tools/sprites_ss14.txt`, копирование `import-sprites`), 41 МБ, CC BY-SA 3.0;
  полный перечень авторов — в `meta.json` каждого RSI и `attributions.yml` сборки.
- `assets/sprites/ss14/Tiles/**` — тайлы пола (`import-map` для карт), CC BY-SA 3.0.
- `assets/sprites/ss14/LobbyScreens/SpaceStation64.webp`, `JustAnotherShift.webp` —
  экраны лобби сборки, CC BY-SA 3.0.
- `assets/maps/imported_aspid.ron`, `assets/maps/imported_floor3x3.ron` —
  сконвертированные карты (источники: `_Mini/aspid.yml`, `Test/floor3x3.yml`).

## Карты SS14 (staging для IMP.4)

`assets/maps/ss14/*.yml` — оригинальные карты формата 7 из сборки. Прямой
импорт в игровой формат (`assets/maps/*.ron`) — задача IMP.4 (SS14_IMPORT.md §5):
формат 7 уже содержит словарь `tilemap` (id → имя тайла), что упрощает парсер;
дальше — экспорт тайлов чанков и сущностей, как описано в документе.
