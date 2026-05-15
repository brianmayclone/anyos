# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Set up the native Windows development toolchain for anyOS.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File .\scripts\setup_toolchain.ps1
#
# Optional:
#   -SkipPackageInstall       Only configure/check tools already installed
#   -NoPathPersist            Do not write PATH additions to the user profile
#   -InstallLegacyI686        Install old i686-elf GCC tools for legacy work
#   -EnableWhpx               Enable Windows Hypervisor Platform for QEMU -Kvm

param(
    [switch]$SkipPackageInstall,
    [switch]$NoPathPersist,
    [switch]$InstallLegacyI686,
    [switch]$EnableWhpx,
    [string]$I686ToolsVersion = "7.1.0"
)

$ErrorActionPreference = "Stop"
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

function Write-Section {
    param([string]$Title)
    Write-Host ""
    Write-Host "--- $Title ---" -ForegroundColor Cyan
}

function Test-Command {
    param([string]$Name)
    return $null -ne (Get-Command $Name -ErrorAction SilentlyContinue)
}

function Add-PathEntry {
    param(
        [string]$Path,
        [switch]$Persist
    )

    if ([string]::IsNullOrWhiteSpace($Path) -or -not (Test-Path $Path)) {
        return
    }

    $resolved = (Resolve-Path $Path).Path.TrimEnd('\')
    $sessionParts = $env:Path -split ';' | Where-Object { $_ -ne "" }
    $alreadyInSession = $false
    foreach ($part in $sessionParts) {
        if ($part.TrimEnd('\').Equals($resolved, [StringComparison]::OrdinalIgnoreCase)) {
            $alreadyInSession = $true
            break
        }
    }
    if (-not $alreadyInSession) {
        $env:Path = "$resolved;$env:Path"
        Write-Host "Added to current PATH: $resolved" -ForegroundColor DarkGray
    }

    if (-not $Persist) {
        return
    }

    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    $userParts = @()
    if (-not [string]::IsNullOrWhiteSpace($userPath)) {
        $userParts = $userPath -split ';' | Where-Object { $_ -ne "" }
    }

    foreach ($part in $userParts) {
        if ($part.TrimEnd('\').Equals($resolved, [StringComparison]::OrdinalIgnoreCase)) {
            return
        }
    }

    $newUserPath = if ([string]::IsNullOrWhiteSpace($userPath)) {
        $resolved
    } else {
        "$resolved;$userPath"
    }
    [Environment]::SetEnvironmentVariable("Path", $newUserPath, "User")
    Write-Host "Added to user PATH: $resolved" -ForegroundColor DarkGray
}

function Invoke-Native {
    param(
        [string]$FilePath,
        [string[]]$Arguments
    )

    & $FilePath @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$FilePath failed with exit code $LASTEXITCODE"
    }
}

function Install-WingetPackage {
    param(
        [string]$Name,
        [string]$Id,
        [string]$ProbeCommand,
        [string[]]$PathHints = @()
    )

    $wingetLinks = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Links" } else { "" }
    Add-PathEntry $wingetLinks -Persist:(!$NoPathPersist)

    foreach ($hint in $PathHints) {
        Add-PathEntry $hint -Persist:(!$NoPathPersist)
    }

    if (Test-Command $ProbeCommand) {
        Write-Host "$Name already installed." -ForegroundColor Green
        return
    }

    if ($SkipPackageInstall) {
        Write-Host "$Name not found; package installation skipped." -ForegroundColor Yellow
        return
    }

    if (-not (Test-Command "winget")) {
        throw "winget not found. Install App Installer from Microsoft Store or install $Name manually."
    }

    Write-Host "Installing $Name via winget ($Id)..."
    Invoke-Native "winget" @(
        "install",
        "--id", $Id,
        "--exact",
        "--accept-source-agreements",
        "--accept-package-agreements"
    )

    foreach ($hint in $PathHints) {
        Add-PathEntry $hint -Persist:(!$NoPathPersist)
    }
    Add-PathEntry $wingetLinks -Persist:(!$NoPathPersist)

    if (Test-Command $ProbeCommand) {
        Write-Host "$Name installed." -ForegroundColor Green
    } else {
        Write-Host "$Name was installed, but $ProbeCommand is not visible in this shell yet." -ForegroundColor Yellow
    }
}

function Install-Rustup {
    Write-Section "Rust nightly"

    $cargoBin = Join-Path $env:USERPROFILE ".cargo\bin"
    Add-PathEntry $cargoBin -Persist:(!$NoPathPersist)

    if (-not (Test-Command "rustup")) {
        if ($SkipPackageInstall) {
            throw "rustup not found and package installation is skipped."
        }

        if (Test-Command "winget") {
            Write-Host "Installing rustup via winget..."
            Invoke-Native "winget" @(
                "install",
                "--id", "Rustlang.Rustup",
                "--exact",
                "--accept-source-agreements",
                "--accept-package-agreements"
            )
        } else {
            Write-Host "Installing rustup from rustup.rs..."
            $rustupInit = Join-Path $env:TEMP "rustup-init.exe"
            Invoke-WebRequest "https://win.rustup.rs/x86_64" -OutFile $rustupInit
            Invoke-Native $rustupInit @("-y", "--default-toolchain", "none")
        }

        Add-PathEntry $cargoBin -Persist:(!$NoPathPersist)
    }

    if (-not (Test-Command "rustup")) {
        throw "rustup is still not available. Open a new PowerShell or add $cargoBin to PATH."
    }

    if ($SkipPackageInstall) {
        Write-Host "rustup found; nightly installation/update skipped." -ForegroundColor Yellow
        return
    }

    Invoke-Native "rustup" @("install", "nightly")
    Invoke-Native "rustup" @("component", "add", "rust-src", "llvm-tools-preview", "--toolchain", "nightly")
}

function Install-Msys2Packages {
    Write-Section "MSYS2 clang/make"

    $msysRoot = if ($env:MSYS2_ROOT) { $env:MSYS2_ROOT } else { "C:\msys64" }
    $bash = Join-Path $msysRoot "usr\bin\bash.exe"
    $clangBin = Join-Path $msysRoot "clang64\bin"

    if (-not (Test-Path $bash)) {
        if ($SkipPackageInstall) {
            Write-Host "MSYS2 not found at $msysRoot; package installation skipped." -ForegroundColor Yellow
            return
        }
        if (-not (Test-Command "winget")) {
            throw "winget not found. Install App Installer from Microsoft Store or install MSYS2 manually."
        }
        Write-Host "Installing MSYS2 via winget (MSYS2.MSYS2)..."
        Invoke-Native "winget" @(
            "install",
            "--id", "MSYS2.MSYS2",
            "--exact",
            "--accept-source-agreements",
            "--accept-package-agreements"
        )
    }

    if (-not (Test-Path $bash)) {
        throw "MSYS2 was not found at $msysRoot. Install MSYS2 or set MSYS2_ROOT."
    }

    Add-PathEntry $clangBin -Persist:(!$NoPathPersist)

    if ($SkipPackageInstall) {
        return
    }

    Write-Host "Refreshing MSYS2 package database..."
    Invoke-Native $bash @("-lc", "pacman -Sy --noconfirm")

    Write-Host "Installing MSYS2 build packages..."
    $packages = @(
        "base-devel",
        "make",
        "diffutils",
        "mingw-w64-clang-x86_64-clang",
        "mingw-w64-clang-x86_64-lld",
        "mingw-w64-clang-x86_64-llvm"
    ) -join " "
    Invoke-Native $bash @("-lc", "pacman -S --needed --noconfirm $packages")

    Add-PathEntry $clangBin -Persist:(!$NoPathPersist)
}

function Install-LegacyI686Toolchain {
    Write-Section "i686-elf-gcc legacy cross-compiler"

    if (Test-Command "i686-elf-gcc") {
        Write-Host "i686-elf-gcc already installed." -ForegroundColor Green
        return
    }

    if (-not $InstallLegacyI686) {
        Write-Host "Skipped. The current x86_64 build no longer requires i686-elf-gcc." -ForegroundColor Yellow
        Write-Host "Use -InstallLegacyI686 if you need old 32-bit libc/TCC work."
        return
    }

    if ($SkipPackageInstall) {
        Write-Host "i686-elf-gcc not found; package installation skipped." -ForegroundColor Yellow
        return
    }

    $prefix = Join-Path $env:USERPROFILE "opt\cross"
    $zipPath = Join-Path $env:TEMP "i686-elf-tools-windows-$I686ToolsVersion.zip"
    $url = "https://github.com/lordmilko/i686-elf-tools/releases/download/$I686ToolsVersion/i686-elf-tools-windows.zip"

    Write-Host "Downloading $url"
    New-Item -ItemType Directory -Path $prefix -Force | Out-Null
    Invoke-WebRequest $url -OutFile $zipPath
    Expand-Archive -Path $zipPath -DestinationPath $prefix -Force

    $gcc = Get-ChildItem -Path $prefix -Recurse -Filter "i686-elf-gcc.exe" -ErrorAction SilentlyContinue |
        Select-Object -First 1

    if (-not $gcc) {
        throw "Downloaded i686-elf tools, but i686-elf-gcc.exe was not found under $prefix."
    }

    Add-PathEntry $gcc.Directory.FullName -Persist:(!$NoPathPersist)
    Write-Host "Installed i686-elf tools at $($gcc.Directory.FullName)" -ForegroundColor Green
}

function Enable-WindowsWhpx {
    Write-Section "Windows Hypervisor Platform"

    if (-not $EnableWhpx) {
        Write-Host "Skipped. Use -EnableWhpx to enable QEMU WHPX acceleration." -ForegroundColor Yellow
        return
    }

    $isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).
        IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
    if (-not $isAdmin) {
        Write-Host "Run this script from an elevated PowerShell to enable WHPX." -ForegroundColor Yellow
        return
    }

    Enable-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform -NoRestart | Out-Null
    Write-Host "WHPX enabled. A reboot may be required." -ForegroundColor Green
}

function Get-VersionLine {
    param(
        [string]$Command,
        [string[]]$Arguments = @("--version"),
        [string]$FallbackPath = ""
    )

    $cmd = Get-Command $Command -ErrorAction SilentlyContinue
    $exe = if ($cmd) { $cmd.Source } elseif ($FallbackPath -and (Test-Path $FallbackPath)) { $FallbackPath } else { "" }
    if (-not $exe) {
        return "not found"
    }

    try {
        $out = & $exe @Arguments 2>$null | Select-Object -First 1
        if ($out) { return $out.ToString().Trim() }
        return "found at $exe"
    } catch {
        return "found at $exe"
    }
}

Write-Host "Setting up anyOS native Windows development toolchain..."

Write-Section "Host tools"
Install-Rustup
Install-WingetPackage "NASM" "NASM.NASM" "nasm" @("C:\Program Files\NASM")
Install-WingetPackage "CMake" "Kitware.CMake" "cmake" @("C:\Program Files\CMake\bin")
Install-WingetPackage "Ninja" "Ninja-build.Ninja" "ninja" @("C:\Program Files\Ninja")
Install-WingetPackage "QEMU" "SoftwareFreedomConservancy.QEMU" "qemu-system-x86_64" @("C:\Program Files\qemu")
Install-Msys2Packages
Install-LegacyI686Toolchain
Enable-WindowsWhpx

Write-Section "Toolchain versions"
$msysRootSummary = if ($env:MSYS2_ROOT) { $env:MSYS2_ROOT } else { "C:\msys64" }
$makeFallback = Join-Path $msysRootSummary "usr\bin\make.exe"
$clangFallback = Join-Path $msysRootSummary "clang64\bin\clang.exe"
$clangxxFallback = Join-Path $msysRootSummary "clang64\bin\clang++.exe"
$llvmArFallback = Join-Path $msysRootSummary "clang64\bin\llvm-ar.exe"

Write-Host "  rustc:         $(Get-VersionLine "rustc" @("+nightly", "--version"))"
Write-Host "  nasm:          $(Get-VersionLine "nasm")"
Write-Host "  cmake:         $(Get-VersionLine "cmake")"
Write-Host "  ninja:         $(Get-VersionLine "ninja" @("--version"))"
Write-Host "  qemu:          $(Get-VersionLine "qemu-system-x86_64")"
Write-Host "  make:          $(Get-VersionLine "make" @("--version") $makeFallback)"
Write-Host "  clang:         $(Get-VersionLine "clang" @("--version") $clangFallback)"
Write-Host "  clang++:       $(Get-VersionLine "clang++" @("--version") $clangxxFallback)"
Write-Host "  llvm-ar:       $(Get-VersionLine "llvm-ar" @("--version") $llvmArFallback)"

if (Test-Command "i686-elf-gcc") {
    Write-Host "  i686-elf-gcc:  $(Get-VersionLine "i686-elf-gcc")"
} else {
    Write-Host "  i686-elf-gcc:  not installed (legacy-only; use -InstallLegacyI686 if needed)"
}

Write-Host ""
Write-Host "Toolchain setup complete!" -ForegroundColor Green
Write-Host ""
Write-Host "Next steps:"
Write-Host "  .\scripts\build.ps1"
Write-Host "  .\scripts\run.ps1"
Write-Host ""
Write-Host "If a newly installed tool is still not visible, open a new PowerShell window."
