@echo off
chcp 65001 >nul
rem [SSR] Build and run the game client. ASCII only (see _build_env.bat).
setlocal
cd /d "%~dp0.."

call "%~dp0_build_env.bat"

echo [SSR] Building and starting client...
cargo run -p ssr-client
if errorlevel 1 (
    rem Retry after a full clean of the client artifacts: this heals
    rem "undefined reference to anon. ... llvm. ..." after an interrupted or
    rem parallel build.
    echo [SSR] Build failed - cleaning ssr-client artifacts and retrying...
    cargo clean -p ssr-client
    cargo run -p ssr-client
)
if errorlevel 1 (
    echo [SSR] Client build/run failed.
    pause
)
endlocal
