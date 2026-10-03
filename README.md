# Space Station R (SSR)

Многопользовательская 2D-песочница в духе Space Station 14, с нуля на Rust + Bevy.

Единый источник правды — [PLAN.md](PLAN.md) (архитектура, стек, дорожная карта).
Текущий прогресс — [PROGRESS.md](PROGRESS.md).

## Быстрый старт

```bash
cargo run -p ssr-client   # игровой клиент (окно 1280×720, WASD/стрелки)
cargo run -p ssr-server   # headless-сервер
cargo build --workspace   # собрать всё
cargo test --workspace    # тесты
```

Требуется Rust (stable). Крейты: `ssr-core` (общая логика), `ssr-protocol` (сеть),
`ssr-server`, `ssr-client`.

## Ассеты

Спрайты — из сборки «Мини-станции» ([mini-station-goob](https://github.com/ministation/mini-station-goob)),
см. `assets/sprites/ss14/ATTRIBUTION.md`. Синхронизация: `tools/sync_sprites.ps1`.
