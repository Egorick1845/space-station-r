#!/usr/bin/env bash
# Быстрый запуск всей игры: сервер в фоне + клиент на переднем плане.
set -euo pipefail
cd "$(dirname "$0")/.."

export PATH="$HOME/.cargo/bin:$PATH"

echo "[SSR] Сборка..."
cargo build --workspace

echo "[SSR] Запуск сервера (фон)..."
./target/debug/ssr-server.exe &
SERVER_PID=$!
trap 'kill $SERVER_PID 2>/dev/null || true' EXIT

sleep 2
echo "[SSR] Запуск клиента..."
./target/debug/ssr-client.exe

echo "[SSR] Клиент закрыт, останавливаю сервер..."
