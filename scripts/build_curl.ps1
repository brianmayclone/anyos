# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Build script for curl (libcurl + curl CLI) for anyOS (i686-elf target)
# Produces: libcurl.a (static library) + curl.o objects
#
# Usage: .\scripts\build_curl.ps1

$ErrorActionPreference = "Continue"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = ((Split-Path -Parent $ScriptDir) -replace '\\','/') -replace '^([A-Za-z]):','/$1'
$CurlDir = "$ProjectDir/third_party/curl"
$ObjDir = "$CurlDir/obj"

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

$LibcInclude = "$ProjectDir/libs/libc/include"
$BearsslInc = "$ProjectDir/third_party/bearssl/inc"
$ConfigAnyos = "$CurlDir/lib/config-anyos.h"

$CFLAGS = @(
    "-O2", "-ffreestanding", "-nostdlib", "-fno-builtin", "-m32", "-std=gnu99", "-w",
    "-isystem", $LibcInclude,
    "-DHAVE_CONFIG_H",
    "-include", "stdbool.h",
    "-include", $ConfigAnyos,
    "-DCURL_STATICLIB",
    "-I$CurlDir/include",
    "-I$CurlDir/lib",
    "-I$CurlDir/src",
    "-I$BearsslInc"
)

# ── Create output directories ────────────────────────────────────────────────

New-Item -ItemType Directory -Force -Path "$ObjDir\lib" | Out-Null
New-Item -ItemType Directory -Force -Path "$ObjDir\lib\vauth" | Out-Null
New-Item -ItemType Directory -Force -Path "$ObjDir\lib\vtls" | Out-Null
New-Item -ItemType Directory -Force -Path "$ObjDir\lib\vquic" | Out-Null
New-Item -ItemType Directory -Force -Path "$ObjDir\src" | Out-Null

# ── Create curl_config.h redirect ────────────────────────────────────────────

$configH = Join-Path $CurlDir "lib\curl_config.h"
Set-Content -Path $configH -Value @"
/* Auto-generated - redirects to config-anyos.h */
#include "config-anyos.h"
"@

# ── Error tracking ───────────────────────────────────────────────────────────

$Success = 0
$Fail = 0
$Errors = @()

function Compile-File {
    param(
        [string]$Src,
        [string]$Obj,
        [string[]]$ExtraFlags = @()
    )
    $allFlags = $script:CFLAGS + $ExtraFlags
    $output = & $script:CC @allFlags -c $Src -o $Obj 2>&1
    if ($LASTEXITCODE -ne 0) {
        $script:Fail++
        $fname = Split-Path -Leaf $Src
        $errLine = ($output | Select-String "error:" | Select-Object -First 1)
        if ($errLine) {
            $script:Errors += "${fname}: $errLine"
        } else {
            $script:Errors += "${fname}: UNKNOWN ERROR"
        }
    } else {
        $script:Success++
    }
}

# ===========================================================================
# Compile libcurl (library)
# ===========================================================================
Write-Host "=== Compiling libcurl ===" -ForegroundColor Cyan

# Core library files
$LIB_CORE_FILES = @(
    "base64.c", "bufq.c", "bufref.c", "cf-https-connect.c", "cf-socket.c",
    "cfilters.c", "conncache.c", "connect.c", "content_encoding.c", "cookie.c",
    "curl_addrinfo.c", "curl_sha512_256.c", "curl_endian.c", "curl_fnmatch.c",
    "curl_get_line.c", "curl_gethostname.c", "curl_memrchr.c", "curl_multibyte.c",
    "curl_range.c", "curl_sasl.c", "curl_trc.c", "cw-out.c",
    "dynbuf.c", "dynhds.c", "easy.c", "easygetopt.c", "easyoptions.c",
    "escape.c", "file.c", "fileinfo.c", "fopen.c", "formdata.c",
    "ftp.c", "ftplistparser.c", "getenv.c", "getinfo.c", "hash.c",
    "headers.c", "hmac.c", "hostasyn.c", "hostip.c", "hostip4.c",
    "hostsyn.c", "http.c", "http1.c", "http_chunks.c", "http_digest.c",
    "idn.c", "if2ip.c", "inet_ntop.c", "inet_pton.c", "llist.c",
    "md5.c", "mime.c", "mprintf.c", "multi.c", "nonblock.c",
    "noproxy.c", "parsedate.c", "pingpong.c", "progress.c", "rand.c",
    "rename.c", "request.c", "select.c", "sendf.c", "setopt.c",
    "sha256.c", "share.c", "slist.c", "speedcheck.c", "splay.c",
    "strcase.c", "strdup.c", "strerror.c", "strparse.c", "strtok.c",
    "strtoofft.c", "timediff.c", "timeval.c", "transfer.c", "url.c",
    "urlapi.c", "version.c", "warnless.c"
)

$LIB_VAUTH_FILES = @(
    "vauth/cleartext.c", "vauth/cram.c", "vauth/digest.c",
    "vauth/oauth2.c", "vauth/vauth.c"
)

$LIB_VTLS_FILES = @(
    "vtls/bearssl.c", "vtls/cipher_suite.c", "vtls/hostcheck.c",
    "vtls/keylog.c", "vtls/vtls.c", "vtls/vtls_scache.c"
)

Write-Host "  [lib core]"
foreach ($f in $LIB_CORE_FILES) {
    $src = Join-Path $CurlDir "lib\$f"
    $name = [System.IO.Path]::GetFileNameWithoutExtension($f)
    $obj = Join-Path $ObjDir "lib\$name.o"
    if (-not (Test-Path $obj) -or ((Get-Item $src).LastWriteTime -gt (Get-Item $obj).LastWriteTime)) {
        Compile-File -Src $src -Obj $obj -ExtraFlags @("-DBUILDING_LIBCURL")
    } else {
        $Success++
    }
}

Write-Host "  [lib vauth]"
foreach ($f in $LIB_VAUTH_FILES) {
    $src = Join-Path $CurlDir "lib\$f"
    $name = [System.IO.Path]::GetFileNameWithoutExtension((Split-Path -Leaf $f))
    $obj = Join-Path $ObjDir "lib\vauth\$name.o"
    if (-not (Test-Path $obj) -or ((Get-Item $src).LastWriteTime -gt (Get-Item $obj).LastWriteTime)) {
        Compile-File -Src $src -Obj $obj -ExtraFlags @("-DBUILDING_LIBCURL")
    } else {
        $Success++
    }
}

Write-Host "  [lib vtls]"
foreach ($f in $LIB_VTLS_FILES) {
    $src = Join-Path $CurlDir "lib\$f"
    $name = [System.IO.Path]::GetFileNameWithoutExtension((Split-Path -Leaf $f))
    $obj = Join-Path $ObjDir "lib\vtls\$name.o"
    if (-not (Test-Path $obj) -or ((Get-Item $src).LastWriteTime -gt (Get-Item $obj).LastWriteTime)) {
        Compile-File -Src $src -Obj $obj -ExtraFlags @("-DBUILDING_LIBCURL")
    } else {
        $Success++
    }
}

Write-Host "  [lib vquic]"
Compile-File -Src (Join-Path $CurlDir "lib\vquic\vquic.c") -Obj (Join-Path $ObjDir "lib\vquic\vquic.o") -ExtraFlags @("-DBUILDING_LIBCURL")

# ===========================================================================
# Compile curl CLI tool
# ===========================================================================
Write-Host "  [curl tool]"

$TOOL_FILES = @(
    "terminal.c", "slist_wc.c", "tool_bname.c", "tool_cb_dbg.c",
    "tool_cb_hdr.c", "tool_cb_prg.c", "tool_cb_rea.c", "tool_cb_see.c",
    "tool_cb_soc.c", "tool_cb_wrt.c", "tool_cfgable.c", "tool_dirhie.c",
    "tool_doswin.c", "tool_easysrc.c", "tool_filetime.c", "tool_findfile.c",
    "tool_formparse.c", "tool_getparam.c", "tool_getpass.c", "tool_help.c",
    "tool_helpers.c", "tool_ipfs.c", "tool_libinfo.c", "tool_listhelp.c",
    "tool_main.c", "tool_msgs.c", "tool_operate.c", "tool_operhlp.c",
    "tool_paramhlp.c", "tool_parsecfg.c", "tool_progress.c", "tool_setopt.c",
    "tool_sleep.c", "tool_ssls.c", "tool_stderr.c", "tool_strdup.c",
    "tool_urlglob.c", "tool_util.c", "tool_vms.c", "tool_writeout.c",
    "tool_writeout_json.c", "tool_xattr.c", "var.c"
)

foreach ($f in $TOOL_FILES) {
    $src = Join-Path $CurlDir "src\$f"
    $name = [System.IO.Path]::GetFileNameWithoutExtension($f)
    $obj = Join-Path $ObjDir "src\$name.o"
    if (-not (Test-Path $obj) -or ((Get-Item $src).LastWriteTime -gt (Get-Item $obj).LastWriteTime)) {
        Compile-File -Src $src -Obj $obj
    } else {
        $Success++
    }
}

# Shared lib files compiled for tool (without BUILDING_LIBCURL for curlx_ names)
Write-Host "  [tool shared libs]"
$TOOL_LIB_FILES = @("dynbuf.c", "warnless.c", "base64.c")
foreach ($f in $TOOL_LIB_FILES) {
    $src = Join-Path $CurlDir "lib\$f"
    $name = [System.IO.Path]::GetFileNameWithoutExtension($f)
    $obj = Join-Path $ObjDir "src\tool_$name.o"
    if (-not (Test-Path $obj) -or ((Get-Item $src).LastWriteTime -gt (Get-Item $obj).LastWriteTime)) {
        Compile-File -Src $src -Obj $obj
    } else {
        $Success++
    }
}

# ===========================================================================
# Results
# ===========================================================================
Write-Host ""
Write-Host "=== Build Results ===" -ForegroundColor Cyan
Write-Host "SUCCESS: $Success, FAIL: $Fail"

if ($Fail -gt 0) {
    Write-Host ""
    Write-Host "=== Errors ===" -ForegroundColor Red
    foreach ($err in $Errors) { Write-Host "  $err" -ForegroundColor Red }
    exit 1
}

# ===========================================================================
# Create static library
# ===========================================================================
Write-Host ""
Write-Host "Creating libcurl.a..."

$allObjs = @()
$allObjs += (Get-ChildItem -Path "$ObjDir\lib" -Filter "*.o" -File).FullName
$allObjs += (Get-ChildItem -Path "$ObjDir\lib\vauth" -Filter "*.o" -File).FullName
$allObjs += (Get-ChildItem -Path "$ObjDir\lib\vtls" -Filter "*.o" -File).FullName
$allObjs += (Get-ChildItem -Path "$ObjDir\lib\vquic" -Filter "*.o" -File).FullName
$allObjs += (Get-ChildItem -Path "$ObjDir\src" -Filter "*.o" -File).FullName

$libcurlA = Join-Path $CurlDir "libcurl.a"
& $AR rcs $libcurlA @allObjs
if ($LASTEXITCODE -ne 0) {
    Write-Host "Archive creation failed!" -ForegroundColor Red
    exit 1
}

$size = (Get-Item $libcurlA).Length
$sizeKB = [math]::Round($size / 1024)
Write-Host "=== Done: $libcurlA (${sizeKB} KiB) ===" -ForegroundColor Green
