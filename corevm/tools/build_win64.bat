@echo off
setlocal

REM Resolve script directory and navigate to vmmanager
set "SCRIPT_DIR=%~dp0"
cd /d "%SCRIPT_DIR%\..\vmmanager"
if errorlevel 1 (
    echo ERROR: Could not cd to vmmanager directory
    echo Trying via wslpath...
    exit /b 1
)

cargo.exe +stable build --release --target x86_64-pc-windows-msvc --features libcorevm/windows
if errorlevel 1 exit /b 1
echo Built: target\x86_64-pc-windows-msvc\release\corevm-vmmanager.exe
