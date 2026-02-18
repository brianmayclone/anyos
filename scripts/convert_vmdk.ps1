# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Convert anyOS disk image to VMDK for VirtualBox
# Usage: .\scripts\convert_vmdk.ps1 [-Out <path\to\anyos.vmdk>]

param(
    [string]$Out = ""
)

$ErrorActionPreference = "Stop"

$ScriptDir  = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BuildDir   = Join-Path $ProjectDir "build"
$ImgPath    = Join-Path $BuildDir "anyos.img"
$VmdkPath   = if ($Out) { $Out } else { Join-Path $BuildDir "anyos.vmdk" }

# Verify the raw image exists
if (-not (Test-Path $ImgPath)) {
    Write-Host "ERROR: Disk image not found: $ImgPath" -ForegroundColor Red
    Write-Host "Run .\scripts\build.ps1 first." -ForegroundColor Yellow
    exit 1
}

# Locate VBoxManage
$VBoxManage = Get-Command VBoxManage -ErrorAction SilentlyContinue
if (-not $VBoxManage) {
    $DefaultPath = "C:\Program Files\Oracle\VirtualBox\VBoxManage.exe"
    if (Test-Path $DefaultPath) {
        $VBoxManage = $DefaultPath
    } else {
        Write-Host "ERROR: VBoxManage not found. Install VirtualBox from https://www.virtualbox.org" -ForegroundColor Red
        exit 1
    }
} else {
    $VBoxManage = $VBoxManage.Source
}

# Remove existing VMDK so VBoxManage doesn't refuse to overwrite
if (Test-Path $VmdkPath) {
    Remove-Item $VmdkPath -Force
}

Write-Host "Converting $ImgPath -> $VmdkPath ..." -ForegroundColor Cyan
& $VBoxManage convertfromraw $ImgPath $VmdkPath --format VMDK
if ($LASTEXITCODE -ne 0) {
    Write-Host "Conversion failed!" -ForegroundColor Red
    exit $LASTEXITCODE
}

Write-Host "Done: $VmdkPath" -ForegroundColor Green
Write-Host ""
Write-Host "To use in VirtualBox:" -ForegroundColor Yellow
Write-Host "  1. Create a new VM (Type: Other, Version: Other/Unknown 64-bit)"
Write-Host "  2. Under Storage, add the VMDK as an existing hard disk"
Write-Host "  3. Set firmware to BIOS and boot from disk"
