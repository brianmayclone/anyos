# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build BearSSL for anyOS (i686 freestanding cross-compile)
#
# Output: third_party\bearssl\build\libbearssl.a
# Usage: .\scripts\build_bearssl.ps1

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BearsslDir = Join-Path $ProjectDir "third_party\bearssl"
$ObjDir = Join-Path $BearsslDir "build\obj"
$Output = Join-Path $BearsslDir "build\libbearssl.a"

# ── Find cross-compiler ──────────────────────────────────────────────────────

$CC = Get-Command "i686-elf-gcc" -ErrorAction SilentlyContinue
$AR = Get-Command "i686-elf-ar" -ErrorAction SilentlyContinue

if (-not $CC -or -not $AR) {
    # Try MSYS2 MinGW64 default location
    $mingw64Bin = "C:\msys64\mingw64\bin"
    if (Test-Path (Join-Path $mingw64Bin "i686-elf-gcc.exe")) {
        $env:Path = "$mingw64Bin;$env:Path"
        $CC = Join-Path $mingw64Bin "i686-elf-gcc.exe"
        $AR = Join-Path $mingw64Bin "i686-elf-ar.exe"
    } else {
        Write-Host "Error: i686-elf-gcc not found." -ForegroundColor Red
        Write-Host "Install via MSYS2: pacman -S mingw-w64-x86_64-i686-elf-gcc"
        exit 1
    }
} else {
    $CC = $CC.Source
    $AR = $AR.Source
}

$LibcInclude = Join-Path $ProjectDir "libs\libc\include"
$CFLAGS = @(
    "-O2", "-ffreestanding", "-nostdlib", "-fno-builtin", "-m32", "-w",
    "-I$($BearsslDir)\inc",
    "-I$($BearsslDir)\src",
    "-I$LibcInclude"
)

New-Item -ItemType Directory -Force -Path $ObjDir | Out-Null

Write-Host "=== Building BearSSL for anyOS (i686) ===" -ForegroundColor Cyan

# Find all .c files in src/
$srcFiles = Get-ChildItem -Path (Join-Path $BearsslDir "src") -Filter "*.c" -Recurse
$count = 0
foreach ($src in $srcFiles) {
    $name = [System.IO.Path]::GetFileNameWithoutExtension($src.Name)
    $obj = Join-Path $ObjDir "$name.o"
    & $CC @CFLAGS -c $src.FullName -o $obj
    if ($LASTEXITCODE -ne 0) {
        Write-Host "  FAILED: $($src.Name)" -ForegroundColor Red
        exit 1
    }
    $count++
}

Write-Host "  AR  libbearssl.a"
$objFiles = Get-ChildItem -Path $ObjDir -Filter "*.o"
& $AR rcs $Output ($objFiles | ForEach-Object { $_.FullName })
if ($LASTEXITCODE -ne 0) {
    Write-Host "Archive creation failed!" -ForegroundColor Red
    exit 1
}

$size = (Get-Item $Output).Length
$sizeKB = [math]::Round($size / 1024)
Write-Host "=== Done: $Output (${sizeKB} KiB, $count objects) ===" -ForegroundColor Green
