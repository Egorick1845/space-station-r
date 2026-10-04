@echo off
chcp 65001 >nul
rem Быстрый запуск всей игры: сервер (в отдельном окне) + клиент.
rem Для тестовых режимов: set SSR_LOCK_INPUT=1 перед запуском и т.п.
setlocal
cd /d "%~dp0.."

where cargo >nul 2>nul || set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if exist "C:\ss14\tools\w64devkit\w64devkit\bin\dlltool.exe" set "PATH=C:\ss14\tools\w64devkit\w64devkit\bin;%PATH%"

echo [SSR] Сборка...
cargo build --workspace
if errorlevel 1 (
    echo [SSR] Ошибка сборки.
    pause
    exit /b 1
)

echo [SSR] Запуск сервера (отдельное окно)...
start "SSR server" cmd /k "cd /d %~dp0.. && target\debug\ssr-server.exe"
timeout /t 2 /nobreak >nul

echo [SSR] Запуск клиента...
target\debug\ssr-client.exe

echo [SSR] Клиент закрыт. Сервер останавливается...
taskkill /IM ssr-server.exe /F >nul 2>nul
endlocal
