@echo off
cd /d "%~dp0\..\vmmanager"
cargo build --release --target x86_64-pc-windows-msvc -Zbuild-std=
echo Built: target\x86_64-pc-windows-msvc\release\corevm-vmmanager.exe
