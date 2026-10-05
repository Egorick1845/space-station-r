@echo off
chcp 65001 >nul
rem [SSR] Shared build preparation for all launch scripts. ASCII only:
rem non-ASCII text in .bat breaks cmd parsing on some codepages.
rem
rem Why: parallel builds (several agents/windows) or an interrupted build leave
rem stale objects in target\debug\incremental; linking then fails with
rem "undefined reference to anon. ... llvm. ..." (symptoms from
rem core::ub_checks). Fix: clear incremental and disable incremental builds.
rem Also stop running exe files: otherwise the linker cannot overwrite
rem target\debug\ssr-*.exe ("Access is denied, os error 5").
rem
rem Usage: call "%~dp0_build_env.bat"

cd /d "%~dp0.."

rem 1) Stop running processes (silently if none).
taskkill /IM ssr-client.exe /F >nul 2>nul
taskkill /IM ssr-server.exe /F >nul 2>nul

rem 2) Incremental off: with it linking breaks on parallel builds, and a full
rem    build takes about a minute anyway.
set "CARGO_INCREMENTAL=0"

rem 3) Remove stale incremental artifacts from an interrupted build.
if exist "target\debug\incremental" rd /s /q "target\debug\incremental" >nul 2>nul

rem 4) PATH: cargo and dlltool (w64devkit) for linking windows dependencies.
where cargo >nul 2>nul || set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
if exist "C:\ss14\tools\w64devkit\w64devkit\bin\dlltool.exe" set "PATH=C:\ss14\tools\w64devkit\w64devkit\bin;%PATH%"
