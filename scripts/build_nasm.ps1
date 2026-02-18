# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build NASM assembler for anyOS (cross-compiled with i686-elf-gcc)
#
# Output: third_party\nasm\nasm.elf
# Usage: .\scripts\build_nasm.ps1

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$NasmDir = Join-Path $ProjectDir "third_party\nasm"
$LibcDir = Join-Path $ProjectDir "libs\libc"
$ObjDir = Join-Path $NasmDir "obj"
$Output = Join-Path $NasmDir "nasm.elf"

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
    "-fno-builtin", "-fno-stack-protector", "-fcommon", "-w",
    "-DHAVE_CONFIG_H",
    "-I$NasmDir", "-I$NasmDir\include", "-I$NasmDir\x86",
    "-I$NasmDir\asm", "-I$NasmDir\output", "-I$NasmDir\nasmlib",
    "-I$NasmDir\macros", "-I$NasmDir\common", "-I$NasmDir\disasm",
    "-I$LibcDir\include"
)

New-Item -ItemType Directory -Force -Path $ObjDir | Out-Null

# Source files
$LIBSRCS = @(
    "stdlib/snprintf.c", "stdlib/vsnprintf.c", "stdlib/strlcpy.c",
    "stdlib/strnlen.c", "stdlib/strrchrnul.c",
    "nasmlib/ver.c", "nasmlib/alloc.c", "nasmlib/asprintf.c",
    "nasmlib/errfile.c", "nasmlib/crc32.c", "nasmlib/crc64.c",
    "nasmlib/md5c.c", "nasmlib/string.c", "nasmlib/nctype.c",
    "nasmlib/file.c", "nasmlib/mmap.c", "nasmlib/ilog2.c",
    "nasmlib/realpath.c", "nasmlib/path.c", "nasmlib/filename.c",
    "nasmlib/rlimit.c", "nasmlib/readnum.c", "nasmlib/numstr.c",
    "nasmlib/zerobuf.c", "nasmlib/bsi.c", "nasmlib/rbtree.c",
    "nasmlib/hashtbl.c", "nasmlib/raa.c", "nasmlib/saa.c",
    "nasmlib/strlist.c", "nasmlib/perfhash.c", "nasmlib/badenum.c",
    "common/common.c",
    "x86/insnsa.c", "x86/insnsb.c", "x86/insnsd.c", "x86/insnsn.c",
    "x86/regs.c", "x86/regvals.c", "x86/regflags.c", "x86/regdis.c",
    "x86/disp8.c", "x86/iflag.c",
    "asm/error.c", "asm/floats.c", "asm/directiv.c", "asm/directbl.c",
    "asm/pragma.c", "asm/assemble.c", "asm/labels.c", "asm/parser.c",
    "asm/preproc.c", "asm/quote.c", "asm/pptok.c", "asm/listing.c",
    "asm/eval.c", "asm/exprlib.c", "asm/exprdump.c", "asm/stdscan.c",
    "asm/strfunc.c", "asm/tokhash.c", "asm/segalloc.c", "asm/rdstrnum.c",
    "asm/srcfile.c", "asm/warnings.c",
    "macros/macros.c",
    "output/outform.c", "output/outlib.c", "output/legacy.c",
    "output/nulldbg.c", "output/nullout.c", "output/outbin.c",
    "output/outaout.c", "output/outcoff.c", "output/outelf.c",
    "output/outobj.c", "output/outas86.c", "output/outdbg.c",
    "output/outieee.c", "output/outmacho.c", "output/codeview.c",
    "disasm/disasm.c", "disasm/sync.c"
)

# NASM main
$ALLSRCS = $LIBSRCS + @("asm/nasm.c")

Write-Host "=== Compiling NASM for anyOS ===" -ForegroundColor Cyan

$objs = @()
foreach ($src in $ALLSRCS) {
    $objName = ($src -replace '/', '_') -replace '\.c$', '.o'
    $obj = Join-Path $ObjDir $objName
    $srcPath = Join-Path $NasmDir $src
    Write-Host "  CC $src"
    & $CC @CFLAGS -c $srcPath -o $obj
    if ($LASTEXITCODE -ne 0) {
        Write-Host "  FAILED: $src" -ForegroundColor Red
        exit 1
    }
    $objs += $obj
}

Write-Host "=== Linking NASM ===" -ForegroundColor Cyan
$crt0 = Join-Path $LibcDir "obj\crt0.o"
$linkLd = Join-Path $LibcDir "link.ld"
$libcA = Join-Path $LibcDir "libc.a"

& $CC -nostdlib -static -m32 -T $linkLd -o $Output $crt0 @objs $libcA -lgcc
if ($LASTEXITCODE -ne 0) {
    Write-Host "Linking failed!" -ForegroundColor Red
    exit 1
}

$size = (Get-Item $Output).Length
$sizeKB = [math]::Round($size / 1024)
Write-Host "=== Done: $Output (${sizeKB} KiB) ===" -ForegroundColor Green
