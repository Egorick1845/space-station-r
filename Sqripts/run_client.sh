#!/usr/bin/env bash
# Быстрый запуск игрового клиента Space Station R (T0.2)
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"

echo "[SSR] Сборка и запуск клиента..."
exec cargo run -p ssr-client
