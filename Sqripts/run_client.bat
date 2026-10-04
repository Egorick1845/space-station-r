@echo off
chcp 65001 >nul
rem Быстрый запуск игрового клиента Space Station R (T0.2)
setlocal
cd /d "%~dp0.."

where cargo >nul 2>nul || set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if exist "C:\ss14\tools\w64devkit\w64devkit\bin\dlltool.exe" set "PATH=C:\ss14\tools\w64devkit\w64devkit\bin;%PATH%"

echo [SSR] Сборка и запуск клиента...
cargo run -p ssr-client
if errorlevel 1 (
    echo [SSR] Ошибка сборки/запуска клиента.
    pause
)
endlocal
