#!/usr/bin/env bash
# Быстрый запуск headless-сервера Space Station R (T0.3)
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"

echo "[SSR] Сборка и запуск сервера (20 TPS, Ctrl+C для остановки)..."
exec cargo run -p ssr-server
