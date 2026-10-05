@echo off
chcp 65001 >nul
rem [SSR] Build and run the headless server (T0.3). ASCII only.
setlocal
cd /d "%~dp0.."

call "%~dp0_build_env.bat"

echo [SSR] Building and starting server (20 TPS, Ctrl+C to stop)...
cargo run -p ssr-server
if errorlevel 1 (
    echo [SSR] Build failed - cleaning ssr-server artifacts and retrying...
    cargo clean -p ssr-server
    cargo run -p ssr-server
)
if errorlevel 1 (
    echo [SSR] Server build/run failed.
    pause
)
endlocal
