# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build dash 0.5.12 for anyOS (cross-compilation)
#
# Output: third_party/dash-0.5.12/dash.a
# Usage: .\scripts\build_dash.ps1

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$DashDir = Join-Path $ProjectDir "third_party\dash-0.5.12"
$ObjDir = Join-Path $DashDir "obj"
$Output = Join-Path $DashDir "dash.a"

# GCC paths (MSYS-style forward slashes, /c/ prefix for -include)
$GccProjectDir = ($ProjectDir -replace '\\','/') -replace '^([A-Za-z]):','/$1'
$GccDashDir = "$GccProjectDir/third_party/dash-0.5.12"
$GccLibcDir = "$GccProjectDir/libs/libc"

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

# ── Prepare ──────────────────────────────────────────────────────────────────

if (Test-Path $ObjDir) { Remove-Item -Recurse -Force $ObjDir }
New-Item -ItemType Directory -Force -Path $ObjDir | Out-Null
New-Item -ItemType Directory -Force -Path "$ObjDir/bltin" | Out-Null

$CFLAGS = @(
    "-ffreestanding", "-nostdlib", "-nostdinc", "-fno-builtin", "-fno-stack-protector",
    "-O2", "-m32", "-Wall", "-Wno-unused-but-set-variable", "-Wno-unused-parameter",
    "-include", "$GccDashDir/config.h",
    "-DBSD=1", "-DSHELL",
    "-I$GccDashDir/generated",
    "-I$GccDashDir/src",
    "-I$GccLibcDir/include"
)

Write-Host "=== Compiling dash ===" -ForegroundColor Cyan

# Source files from src/
$SRC_FILES = @(
    "alias", "arith_yacc", "arith_yylex", "cd", "error", "eval", "exec", "expand",
    "histedit", "input", "jobs", "mail", "main", "memalloc", "miscbltin",
    "mystring", "options", "output", "parser", "redir", "show", "system", "trap", "var"
)

foreach ($f in $SRC_FILES) {
    Write-Host "  CC ${f}.c"
    & $CC @CFLAGS -c "$DashDir/src/${f}.c" -o "$ObjDir/${f}.o"
    if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: ${f}.c" -ForegroundColor Red; exit 1 }
}

# Builtin files from src/bltin/
foreach ($f in @("printf", "test", "times")) {
    Write-Host "  CC bltin/${f}.c"
    & $CC @CFLAGS -c "$DashDir/src/bltin/${f}.c" -o "$ObjDir/bltin/${f}.o"
    if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: bltin/${f}.c" -ForegroundColor Red; exit 1 }
}

# Generated files
$GEN_FILES = @("builtins", "init", "nodes", "signames", "syntax")
foreach ($f in $GEN_FILES) {
    Write-Host "  CC generated/${f}.c"
    & $CC @CFLAGS -c "$DashDir/generated/${f}.c" -o "$ObjDir/${f}.o"
    if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: generated/${f}.c" -ForegroundColor Red; exit 1 }
}

Write-Host "=== Creating dash.a ===" -ForegroundColor Cyan

$allObjs = @()
foreach ($f in $SRC_FILES) { $allObjs += "$ObjDir/${f}.o" }
$allObjs += "$ObjDir/bltin/printf.o"
$allObjs += "$ObjDir/bltin/test.o"
$allObjs += "$ObjDir/bltin/times.o"
foreach ($f in $GEN_FILES) { $allObjs += "$ObjDir/${f}.o" }

& $AR rcs $Output @allObjs
if ($LASTEXITCODE -ne 0) { Write-Host "Archive creation failed!" -ForegroundColor Red; exit 1 }

$size = (Get-Item $Output).Length
$sizeKB = [math]::Round($size / 1024)
Write-Host "=== Done: $Output (${sizeKB} KiB) ===" -ForegroundColor Green
