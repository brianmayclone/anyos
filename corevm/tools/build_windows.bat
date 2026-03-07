@echo off
cd /d "%~dp0\..\vmmanager"
cargo +stable build --release
if "%1"=="--run" (
    cargo +stable run --release -- %*
)
