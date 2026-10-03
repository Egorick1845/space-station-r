@echo off
rem Быстрый запуск headless-сервера Space Station R (T0.3)
setlocal
cd /d "%~dp0.."

where cargo >nul 2>nul || set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if exist "C:\ss14\tools\w64devkit\w64devkit\bin\dlltool.exe" set "PATH=C:\ss14\tools\w64devkit\w64devkit\bin;%PATH%"

echo [SSR] Сборка и запуск сервера (20 TPS, Ctrl+C для остановки)...
cargo run -p ssr-server
if errorlevel 1 (
    echo [SSR] Ошибка сборки/запуска сервера.
    pause
)
endlocal
