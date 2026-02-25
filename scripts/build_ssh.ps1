# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build SSH library for anyOS (i686 freestanding cross-compile)
#
# Output: third_party/ssh/build/libssh.a
# Usage: .\scripts\build_ssh.ps1

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = ((Split-Path -Parent $ScriptDir) -replace '\\','/') -replace '^([A-Za-z]):','/$1'
$SshDir = "$ProjectDir/third_party/ssh"
$BearsslDir = "$ProjectDir/third_party/bearssl"
$LibcDir = "$ProjectDir/libs/libc"
$ObjDir = "$SshDir/build"
$Output = "$SshDir/build/libssh.a"

# ── Find cross-compiler ──────────────────────────────────────────────────────

$CC = Get-Command "i686-elf-gcc" -ErrorAction SilentlyContinue
$AR = Get-Command "i686-elf-ar" -ErrorAction SilentlyContinue

if (-not $CC -or -not $AR) {
    $mingw64Bin = "C:\msys64\mingw64\bin"
    if (Test-Path (Join-Path $mingw64Bin "i686-elf-gcc.exe")) {
        $env:Path = "$mingw64Bin;$env:Path"
        $CC = Join-Path $mingw64Bin "i686-elf-gcc.exe"
        $AR = Join-Path $mingw64Bin "i686-elf-ar.exe"
    } else {
        Write-Host "Error: i686-elf-gcc not found." -ForegroundColor Red
        exit 1
    }
} else {
    $CC = $CC.Source
    $AR = $AR.Source
}

$CFLAGS = @(
    "-O2", "-ffreestanding", "-nostdlib", "-nostdinc", "-fno-builtin", "-fno-stack-protector",
    "-m32", "-std=c99", "-w",
    "-I$SshDir/include",
    "-I$BearsslDir/inc",
    "-I$LibcDir/include"
)

New-Item -ItemType Directory -Force -Path $ObjDir | Out-Null

Write-Host "=== Building SSH library for anyOS (i686) ===" -ForegroundColor Cyan

& $CC @CFLAGS -c "$SshDir/src/ssh.c" -o "$ObjDir/ssh.o"
if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: ssh.c" -ForegroundColor Red; exit 1 }

Write-Host "  AR  libssh.a"
& $AR rcs $Output "$ObjDir/ssh.o"
if ($LASTEXITCODE -ne 0) { Write-Host "Archive creation failed!" -ForegroundColor Red; exit 1 }

$size = (Get-Item $Output).Length
$sizeKB = [math]::Round($size / 1024)
Write-Host "=== Done: $Output (${sizeKB} KiB) ===" -ForegroundColor Green
