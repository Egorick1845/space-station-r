//! Headless-сервер Space Station R.
//!
//! T0.3: Bevy MinimalPlugins, фиксированный тик-рейт 20 TPS,
//! лог каждого тика через tracing.

use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use tracing_subscriber::EnvFilter;

/// Тик-рейт сервера (PLAN.md T0.3).
const TPS: f64 = 20.0;

fn main() {
    init_tracing();

    App::new()
        .add_plugins(
            MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(
                1.0 / TPS,
            ))),
        )
        .init_resource::<TickState>()
        .add_systems(Update, tick_logger)
        .run();
}

/// Фильтр логов берётся из RUST_LOG (например, `RUST_LOG=debug`), по умолчанию info.
///
/// Внимание: bevy_log включает у tracing-subscriber фичу env-filter, из-за чего
/// `fmt::try_init()` без явного фильтра строит пустой EnvFilter и молчит.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    if let Err(e) = tracing_subscriber::fmt().with_env_filter(filter).try_init() {
        eprintln!("tracing init failed: {e}");
    }
}

/// Счётчик тиков сервера с момента запуска.
#[derive(Resource, Default)]
struct TickState {
    tick: u64,
}

fn tick_logger(mut state: ResMut<TickState>) {
    state.tick += 1;
    tracing::info!(tick = state.tick, "server tick");
}
