# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build libgit2 as a static library for anyOS (cross-compiled with i686-elf-gcc)
#
# Output: third_party\libgit2\libgit2.a
# Usage: .\scripts\build_libgit2.ps1

$ErrorActionPreference = "Continue"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$LG2Dir = Join-Path $ProjectDir "third_party\libgit2"
$LibcDir = Join-Path $ProjectDir "libs\libc"
$ObjDir = Join-Path $LG2Dir "obj"
$Output = Join-Path $LG2Dir "libgit2.a"

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
        Write-Host "Install via MSYS2: pacman -S mingw-w64-x86_64-i686-elf-gcc"
        exit 1
    }
} else {
    $CC = $CC.Source
    $AR = $AR.Source
}

$CFLAGS = @(
    "-m32", "-O2", "-ffreestanding", "-nostdlib", "-nostdinc",
    "-fno-builtin", "-fno-stack-protector", "-fcommon", "-std=c99", "-w",
    "-I$LG2Dir\include",
    "-I$LG2Dir\src\libgit2",
    "-I$LG2Dir\src\util",
    "-I$LG2Dir\deps\xdiff",
    "-I$LG2Dir\deps\zlib",
    "-I$LG2Dir\deps\pcre",
    "-I$LG2Dir\deps\llhttp",
    "-I$LG2Dir\src\util\hash",
    "-I$LG2Dir\src\util\hash\sha1dc",
    "-I$LG2Dir\src\util\hash\rfc6234",
    "-I$LibcDir\include",
    "-DHAVE_STDINT_H", "-DHAVE_LIMITS_H",
    "-DPCRE_STATIC", "-DHAVE_CONFIG_H",
    "-DNO_READDIR_R"
)

New-Item -ItemType Directory -Force -Path $ObjDir | Out-Null

Write-Host "=== Building libgit2 for anyOS ===" -ForegroundColor Cyan

# Collect source files
$SRCS = @()

# Core libgit2
$SRCS += (Get-ChildItem -Path "$LG2Dir\src\libgit2" -Filter "*.c" -File).FullName

# Transports
$transportFiles = @(
    "local.c", "credential.c", "credential_helpers.c",
    "smart.c", "smart_pkt.c", "smart_protocol.c",
    "http.c", "httpclient.c", "httpparser.c", "auth.c", "git.c"
)
foreach ($f in $transportFiles) {
    $path = Join-Path $LG2Dir "src\libgit2\transports\$f"
    if (Test-Path $path) { $SRCS += $path }
}

# Streams
$streamFiles = @("socket.c", "registry.c", "tls.c")
foreach ($f in $streamFiles) {
    $path = Join-Path $LG2Dir "src\libgit2\streams\$f"
    if (Test-Path $path) { $SRCS += $path }
}

# Utility layer
$SRCS += (Get-ChildItem -Path "$LG2Dir\src\util" -Filter "*.c" -File).FullName

# Utility - allocators
$stdalloc = Join-Path $LG2Dir "src\util\allocators\stdalloc.c"
if (Test-Path $stdalloc) { $SRCS += $stdalloc }

# Utility - hash implementations
$hashFiles = @(
    "src\util\hash\collisiondetect.c",
    "src\util\hash\sha1dc\sha1.c",
    "src\util\hash\sha1dc\ubc_check.c",
    "src\util\hash\builtin.c",
    "src\util\hash\rfc6234\sha224-256.c"
)
foreach ($f in $hashFiles) {
    $path = Join-Path $LG2Dir $f
    if (Test-Path $path) { $SRCS += $path }
}

# Utility - unix stubs
$unixFiles = @("src\util\unix\map.c", "src\util\unix\realpath.c")
foreach ($f in $unixFiles) {
    $path = Join-Path $LG2Dir $f
    if (Test-Path $path) { $SRCS += $path }
}

# Deps - zlib, xdiff, pcre, llhttp
foreach ($dep in @("deps\zlib", "deps\xdiff", "deps\pcre", "deps\llhttp")) {
    $depDir = Join-Path $LG2Dir $dep
    if (Test-Path $depDir) {
        $SRCS += (Get-ChildItem -Path $depDir -Filter "*.c" -File).FullName
    }
}

# anyOS-specific stubs
$stubs = Join-Path $LG2Dir "anyos_stubs.c"
if (Test-Path $stubs) { $SRCS += $stubs }

# Compile all
$objs = @()
$errors = 0
foreach ($src in $SRCS) {
    $relPath = $src.Substring($LG2Dir.Length + 1)
    $objName = ($relPath -replace '[\\/]', '_') -replace '\.c$', '.o'
    $obj = Join-Path $ObjDir $objName

    $output = & $CC @CFLAGS -c $src -o $obj 2>&1
    if ($LASTEXITCODE -eq 0) {
        $objs += $obj
    } else {
        Write-Host "  FAILED: $relPath" -ForegroundColor Red
        $errors++
    }
}

if ($errors -gt 0) {
    Write-Host "=== libgit2: $errors files failed ===" -ForegroundColor Yellow
}

& $AR rcs $Output @objs
if ($LASTEXITCODE -ne 0) {
    Write-Host "Archive creation failed!" -ForegroundColor Red
    exit 1
}

$size = (Get-Item $Output).Length
$sizeKB = [math]::Round($size / 1024)
Write-Host "=== libgit2: $($objs.Count) objects, ${sizeKB} KiB ===" -ForegroundColor Green
