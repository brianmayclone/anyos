# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build mini git CLI for anyOS (links against libgit2 + BearSSL)
#
# Output: bin\git\git.elf
# Usage: .\scripts\build_git.ps1

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$GitDir = Join-Path $ProjectDir "bin\git"
$LG2Dir = Join-Path $ProjectDir "third_party\libgit2"
$BearsslDir = Join-Path $ProjectDir "third_party\bearssl"
$LibcDir = Join-Path $ProjectDir "libs\libc"
$Output = Join-Path $GitDir "git.elf"

# ── Find cross-compiler ──────────────────────────────────────────────────────

$CC = Get-Command "i686-elf-gcc" -ErrorAction SilentlyContinue
if (-not $CC) {
    $mingw64Bin = "C:\msys64\mingw64\bin"
    if (Test-Path (Join-Path $mingw64Bin "i686-elf-gcc.exe")) {
        $env:Path = "$mingw64Bin;$env:Path"
        $CC = Join-Path $mingw64Bin "i686-elf-gcc.exe"
    } else {
        Write-Host "Error: i686-elf-gcc not found." -ForegroundColor Red
        Write-Host "Install via MSYS2: pacman -S mingw-w64-x86_64-i686-elf-gcc"
        exit 1
    }
} else {
    $CC = $CC.Source
}

$CFLAGS = @(
    "-m32", "-O2", "-ffreestanding", "-nostdlib", "-nostdinc",
    "-fno-builtin", "-fno-stack-protector", "-fcommon", "-std=c99", "-w",
    "-I$LG2Dir\include",
    "-I$BearsslDir\inc",
    "-I$LibcDir\include"
)

Write-Host "=== Building git CLI for anyOS ===" -ForegroundColor Cyan

# Compile source files
$mainSrc = Join-Path $GitDir "src\main.c"
$mainObj = Join-Path $GitDir "main.o"
& $CC @CFLAGS -c $mainSrc -o $mainObj
if ($LASTEXITCODE -ne 0) {
    Write-Host "Failed to compile main.c" -ForegroundColor Red
    exit 1
}

$bearsslStreamSrc = Join-Path $GitDir "src\bearssl_stream.c"
$bearsslStreamObj = Join-Path $GitDir "bearssl_stream.o"
& $CC @CFLAGS -c $bearsslStreamSrc -o $bearsslStreamObj
if ($LASTEXITCODE -ne 0) {
    Write-Host "Failed to compile bearssl_stream.c" -ForegroundColor Red
    exit 1
}

# Link
$crt0 = Join-Path $LibcDir "obj\crt0.o"
$linkLd = Join-Path $LibcDir "link.ld"
$libcA = Join-Path $LibcDir "libc.a"
$libgit2A = Join-Path $LG2Dir "libgit2.a"
$libbearsslA = Join-Path $BearsslDir "build\libbearssl.a"

& $CC -nostdlib -static -m32 `
    -T $linkLd `
    -o $Output `
    $crt0 `
    $mainObj `
    $bearsslStreamObj `
    $libgit2A `
    $libbearsslA `
    $libcA `
    -lgcc

if ($LASTEXITCODE -ne 0) {
    Write-Host "Linking failed!" -ForegroundColor Red
    exit 1
}

$size = (Get-Item $Output).Length
$sizeKB = [math]::Round($size / 1024)
Write-Host "=== Done: git.elf (${sizeKB} KiB) ===" -ForegroundColor Green
