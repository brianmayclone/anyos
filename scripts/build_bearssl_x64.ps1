# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build BearSSL for anyOS (x86_64 freestanding cross-compile using clang)
#
# Uses libs\libc64\include for standard headers.
# Output: third_party\bearssl\build_x64\libbearssl_x64.a
# Usage: .\scripts\build_bearssl_x64.ps1

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BearsslDir = Join-Path $ProjectDir "third_party\bearssl"
$Libc64Inc = Join-Path $ProjectDir "libs\libc64\include"
$ObjDir = Join-Path $BearsslDir "build_x64\obj"
$Output = Join-Path $BearsslDir "build_x64\libbearssl_x64.a"

# Skip if already built
if (Test-Path $Output) {
    Write-Host "=== BearSSL x64 already built: $Output ===" -ForegroundColor Green
    exit 0
}

# ── Find clang and llvm-ar ────────────────────────────────────────────────────

$CC = Get-Command "clang" -ErrorAction SilentlyContinue
$AR = Get-Command "llvm-ar" -ErrorAction SilentlyContinue

if (-not $CC) {
    Write-Host "Error: clang not found in PATH." -ForegroundColor Red
    Write-Host "Install LLVM: winget install LLVM.LLVM"
    exit 1
}
$CC = $CC.Source

if (-not $AR) {
    # Try to find llvm-ar alongside clang
    $llvmDir = Split-Path -Parent $CC
    $llvmAr = Join-Path $llvmDir "llvm-ar.exe"
    if (Test-Path $llvmAr) {
        $AR = $llvmAr
    } else {
        Write-Host "Error: llvm-ar not found in PATH." -ForegroundColor Red
        exit 1
    }
} else {
    $AR = $AR.Source
}

# Disable HW intrinsics (AES-NI, SSE2, PCLMUL) — software fallbacks used instead.
# Enable BR_64 (64-bit registers) and BR_LE_UNALIGNED (x86 tolerates unaligned).
# Disable RDRAND, /dev/urandom, time — not available in freestanding.
$CFLAGS = @(
    "--target=x86_64-unknown-none-elf",
    "-ffreestanding", "-nostdlib", "-fno-builtin", "-nostdinc", "-O2", "-w",
    "-I$Libc64Inc",
    "-I$($BearsslDir)\inc",
    "-I$($BearsslDir)\src",
    "-DBR_AES_X86NI=0", "-DBR_SSE2=0", "-DBR_RDRAND=0",
    "-DBR_64=1", "-DBR_LE_UNALIGNED=1",
    "-DBR_USE_URANDOM=0", "-DBR_USE_UNIX_TIME=0", "-DBR_USE_GETENTROPY=0"
)

New-Item -ItemType Directory -Force -Path $ObjDir | Out-Null

Write-Host "=== Building BearSSL for anyOS (x86_64) ===" -ForegroundColor Cyan

# Compile libc64 stubs
$libc64Src = Join-Path $ProjectDir "libs\libc64\src"
if (Test-Path $libc64Src) {
    $libc64Files = Get-ChildItem -Path $libc64Src -Filter "*.c"
    foreach ($src in $libc64Files) {
        $name = [System.IO.Path]::GetFileNameWithoutExtension($src.Name)
        $obj = Join-Path $ObjDir "libc64_$name.o"
        & $CC @CFLAGS -c $src.FullName -o $obj
        if ($LASTEXITCODE -ne 0) {
            Write-Host "  FAILED: libc64/$($src.Name)" -ForegroundColor Red
            exit 1
        }
    }
}

# Compile all BearSSL .c files
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

Write-Host "  AR  libbearssl_x64.a"
$objFiles = Get-ChildItem -Path $ObjDir -Filter "*.o"
& $AR rcs $Output ($objFiles | ForEach-Object { $_.FullName })
if ($LASTEXITCODE -ne 0) {
    Write-Host "Archive creation failed!" -ForegroundColor Red
    exit 1
}

$size = (Get-Item $Output).Length
$sizeKB = [math]::Round($size / 1024)
Write-Host "=== Done: $Output (${sizeKB} KiB, $count objects) ===" -ForegroundColor Green
