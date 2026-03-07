@echo off
cd /d "%~dp0\..\vmmanager"
cargo +stable build --release --target x86_64-pc-windows-msvc
echo Built: target\x86_64-pc-windows-msvc\release\corevm-vmmanager.exe
