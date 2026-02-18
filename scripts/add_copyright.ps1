# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Script to add copyright headers to all source files in the anyOS project.
# Usage: .\scripts\add_copyright.ps1 [--check] [--remove]
#   --check   Only report files missing the header (no modifications)
#   --remove  Remove existing copyright headers

param(
    [switch]$Check,
    [switch]$Remove
)

$ErrorActionPreference = "Stop"

$ProjectRoot = Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)

# ── Copyright header templates ────────────────────────────────────────────────

$RustHeader = @"
// Copyright (c) 2024-2026 Christian Moeller
// Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
//
// This project is open source and community-driven.
// Contributions are welcome! See README.md for details.
//
// SPDX-License-Identifier: MIT
"@

$AsmHeader = @"
; Copyright (c) 2024-2026 Christian Moeller
; Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
;
; This project is open source and community-driven.
; Contributions are welcome! See README.md for details.
;
; SPDX-License-Identifier: MIT
"@

$CHeader = @"
/*
 * Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 *
 * This project is open source and community-driven.
 * Contributions are welcome! See README.md for details.
 *
 * SPDX-License-Identifier: MIT
 */
"@

$PythonHeader = @"
# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT
"@

$CmakeHeader = $PythonHeader

$LdHeader = @"
/* Copyright (c) 2024-2026 Christian Moeller
 * Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
 * SPDX-License-Identifier: MIT */
"@

# ── Determine mode ───────────────────────────────────────────────────────────

$Mode = "add"
if ($Check) { $Mode = "--check" }
if ($Remove) { $Mode = "--remove" }

$Added = 0
$Skipped = 0
$Checked = 0

# ── Helpers ──────────────────────────────────────────────────────────────────

function Test-HasCopyright {
    param([string]$FilePath)
    $lines = Get-Content -Path $FilePath -TotalCount 10 -ErrorAction SilentlyContinue
    if ($lines) {
        foreach ($line in $lines) {
            if ($line -match "(?i)copyright") { return $true }
        }
    }
    return $false
}

function Remove-CopyrightHeader {
    param([string]$FilePath, [string]$FileExt)
    $lines = Get-Content -Path $FilePath -Raw
    $lineArray = Get-Content -Path $FilePath

    switch ($FileExt) {
        { $_ -in "rs" } {
            # Remove leading // Copyright block + blank line
            $result = @()
            $inHeader = $true
            foreach ($line in $lineArray) {
                if ($inHeader -and $line -match '^// (Copyright|Email|This project|Contributions|SPDX)') { continue }
                if ($inHeader -and $line -match '^//$') { continue }
                if ($inHeader -and $line -eq '') { $inHeader = $false; continue }
                $inHeader = $false
                $result += $line
            }
            Set-Content -Path $FilePath -Value ($result -join "`n") -NoNewline
        }
        { $_ -in "asm", "inc" } {
            $result = @()
            $inHeader = $true
            foreach ($line in $lineArray) {
                if ($inHeader -and $line -match '^; (Copyright|Email|This project|Contributions|SPDX)') { continue }
                if ($inHeader -and $line -match '^;$') { continue }
                if ($inHeader -and $line -eq '') { $inHeader = $false; continue }
                $inHeader = $false
                $result += $line
            }
            Set-Content -Path $FilePath -Value ($result -join "`n") -NoNewline
        }
        { $_ -in "c", "h", "S" } {
            $result = @()
            $inHeader = $true
            $inBlock = $false
            foreach ($line in $lineArray) {
                if ($inHeader -and $line -match '^/\*' -and $line -match 'Copyright') { $inBlock = $true; continue }
                if ($inBlock -and $line -match '\*/') { $inBlock = $false; continue }
                if ($inBlock) { continue }
                if ($inHeader -and $line -eq '') { $inHeader = $false; continue }
                $inHeader = $false
                $result += $line
            }
            Set-Content -Path $FilePath -Value ($result -join "`n") -NoNewline
        }
        { $_ -in "py", "sh", "cmake", "ps1" } {
            $result = @()
            $inHeader = $true
            foreach ($i in 0..($lineArray.Count - 1)) {
                $line = $lineArray[$i]
                if ($inHeader -and $i -eq 0 -and $line -match '^#!') { $result += $line; continue }
                if ($inHeader -and $line -match '^# (Copyright|Email|This project|Contributions|SPDX)') { continue }
                if ($inHeader -and $line -match '^#$') { continue }
                if ($inHeader -and $line -eq '') { $inHeader = $false; continue }
                $inHeader = $false
                $result += $line
            }
            Set-Content -Path $FilePath -Value ($result -join "`n") -NoNewline
        }
        { $_ -in "ld" } {
            $result = @()
            $inHeader = $true
            $inBlock = $false
            foreach ($line in $lineArray) {
                if ($inHeader -and $line -match '^/\* Copyright') { $inBlock = $true; continue }
                if ($inBlock -and $line -match '\*/') { $inBlock = $false; continue }
                if ($inBlock) { continue }
                if ($inHeader -and $line -eq '') { $inHeader = $false; continue }
                $inHeader = $false
                $result += $line
            }
            Set-Content -Path $FilePath -Value ($result -join "`n") -NoNewline
        }
    }
}

function Add-CopyrightHeader {
    param([string]$FilePath, [string]$Header, [string]$FileExt)
    $content = Get-Content -Path $FilePath -Raw

    # For scripts with shebang, preserve it
    if ($FileExt -in "py", "sh" ) {
        $lines = Get-Content -Path $FilePath
        if ($lines.Count -gt 0 -and $lines[0] -match '^#!') {
            $shebang = $lines[0]
            $rest = ($lines | Select-Object -Skip 1) -join "`n"
            $newContent = "$shebang`n`n$Header`n`n$rest"
            Set-Content -Path $FilePath -Value $newContent -NoNewline
            return
        }
    }

    $newContent = "$Header`n`n$content"
    Set-Content -Path $FilePath -Value $newContent -NoNewline
}

# ── Process files ────────────────────────────────────────────────────────────

Write-Host "anyOS Copyright Header Tool" -ForegroundColor Cyan
Write-Host "==========================="
Write-Host "Mode: $Mode"
Write-Host ""

# Excluded directories
$excludeDirs = @("build", "third_party", ".git", "target", ".venv")

# File extensions to process
$extensions = @("*.rs", "*.asm", "*.inc", "*.c", "*.h", "*.S", "*.py", "*.sh", "*.ld", "*.ps1", "CMakeLists.txt")

$allFiles = @()
foreach ($ext in $extensions) {
    $allFiles += Get-ChildItem -Path $ProjectRoot -Filter $ext -Recurse -File -ErrorAction SilentlyContinue |
        Where-Object {
            $relPath = $_.FullName.Substring($ProjectRoot.Length + 1)
            $skip = $false
            foreach ($dir in $excludeDirs) {
                if ($relPath -like "$dir\*" -or $relPath -like "*\target\*") {
                    $skip = $true
                    break
                }
            }
            -not $skip
        }
}

foreach ($file in $allFiles) {
    $relPath = $file.FullName.Substring($ProjectRoot.Length + 1)
    $ext = $file.Extension.TrimStart(".")
    $basename = $file.Name

    $header = $null
    $fileExt = $null

    switch ($ext) {
        "rs"    { $header = $RustHeader;   $fileExt = "rs" }
        "asm"   { $header = $AsmHeader;    $fileExt = "asm" }
        "inc"   { $header = $AsmHeader;    $fileExt = "inc" }
        "c"     { $header = $CHeader;      $fileExt = "c" }
        "h"     { $header = $CHeader;      $fileExt = "h" }
        "S"     { $header = $CHeader;      $fileExt = "S" }
        "py"    { $header = $PythonHeader; $fileExt = "py" }
        "sh"    { $header = $CmakeHeader;  $fileExt = "sh" }
        "ld"    { $header = $LdHeader;     $fileExt = "ld" }
        "ps1"   { $header = $PythonHeader; $fileExt = "ps1" }
        "txt"   {
            if ($basename -eq "CMakeLists.txt") {
                $header = $CmakeHeader; $fileExt = "cmake"
            } else { continue }
        }
        default { continue }
    }

    if (-not $header) { continue }

    $Checked++

    if ($Mode -eq "--check") {
        if (-not (Test-HasCopyright $file.FullName)) {
            Write-Host "  MISSING: $relPath"
            $Added++
        }
        continue
    }

    if ($Mode -eq "--remove") {
        if (Test-HasCopyright $file.FullName) {
            Remove-CopyrightHeader -FilePath $file.FullName -FileExt $fileExt
            Write-Host "  REMOVED: $relPath"
            $Added++
        }
        continue
    }

    # Default: add mode
    if (Test-HasCopyright $file.FullName) {
        $Skipped++
        continue
    }

    Add-CopyrightHeader -FilePath $file.FullName -Header $header -FileExt $fileExt
    Write-Host "  ADDED:   $relPath"
    $Added++
}

Write-Host ""
Write-Host "Done! Checked: $Checked, Added/Matched: $Added, Skipped (already has): $Skipped"
