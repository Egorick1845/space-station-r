# Sqripts — быстрый запуск

| Скрипт | Что делает |
|---|---|
| `run_client.bat` / `run_client.sh` | сборка + запуск игрового клиента (окно 1280×720, WASD) |
| `run_server.bat` / `run_server.sh` | сборка + запуск headless-сервера (20 TPS, Ctrl+C для остановки) |

Скрипты сами переходят в корень репозитория и подкладывают `cargo` в PATH
(Windows-вариант также подкладывает w64devkit для сборки под windows-gnu).

Фильтр логов сервера: переменная `RUST_LOG` (по умолчанию `info`),
например: `set RUST_LOG=debug` перед `run_server.bat`.
