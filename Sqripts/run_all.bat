@echo off
chcp 65001 >nul
rem [SSR] Build and run everything: server (separate window) + client. ASCII only.
rem Test modes: set SSR_LOCK_INPUT=1 etc. before running.
setlocal
cd /d "%~dp0.."

call "%~dp0_build_env.bat"

echo [SSR] Building...
cargo build --workspace
if errorlevel 1 (
    rem Retry after cleaning the crate artifacts: heals
    rem "undefined reference to anon. ... llvm. ..." after an interrupted or
    rem parallel build.
    echo [SSR] Build failed - cleaning crate artifacts and retrying...
    cargo clean -p ssr-client
    cargo clean -p ssr-server
    cargo build --workspace
)
if errorlevel 1 (
    echo [SSR] Build failed.
    pause
    exit /b 1
)

echo [SSR] Starting server (separate window)...
start "SSR server" cmd /k "cd /d %~dp0.. && target\debug\ssr-server.exe"
timeout /t 2 /nobreak >nul

echo [SSR] Starting client...
target\debug\ssr-client.exe

echo [SSR] Client closed. Stopping server...
taskkill /IM ssr-server.exe /F >nul 2>nul
endlocal
