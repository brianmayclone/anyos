# Copyright (c) 2024-2026 Christian Moeller
# Email: c.moeller.ffo@gmail.com, brianmayclone@googlemail.com
#
# This project is open source and community-driven.
# Contributions are welcome! See README.md for details.
#
# SPDX-License-Identifier: MIT

# Run anyOS in QEMU on Windows
# Usage: .\scripts\run.ps1 [-Vmware] [-Std] [-Virtio] [-Res "WxH"] [-Ide] [-Cdrom] [-Audio] [-Usb] [-Uefi] [-Kvm] [-Fwd "H:G","H:G"] [-VBox] [-VMwareWS]
#
#   -VBox     Start VirtualBox VM named 'anyos' and stream its COM1 serial output here
#   -VMwareWS Start VMware Workstation VM named 'anyos' and stream its COM1 serial output here
#   -Vmware   VMware SVGA II (2D acceleration, HW cursor)
#   -Std      Bochs VGA / Standard VGA (double-buffering, no accel) [default]
#   -Virtio   VirtIO GPU (modern transport, ARGB cursor)
#   -Res WxH  Set initial GPU resolution (VirtIO only). Example: -Res "1280x1024"
#   -Ide      Use legacy IDE (PIO) instead of AHCI (DMA) for disk I/O
#   -Cdrom    Boot from ISO image (CD-ROM) instead of hard drive
#   -Audio    Enable AC'97 audio device
#   -Usb      Enable USB controller with keyboard + mouse devices
#   -Uefi     Boot via UEFI (OVMF) instead of BIOS
#   -Kvm      Enable hardware virtualization (WHPX on Windows)
#   -Fwd H:G  Forward host port H to guest port G (TCP). Repeatable.
#             Example: -Fwd "2222:22","8080:8080"

param(
    [switch]$VBox,
    [switch]$VMwareWS,
    [switch]$Vmware,
    [switch]$Std,
    [switch]$Virtio,
    [switch]$Ide,
    [switch]$Cdrom,
    [switch]$Audio,
    [switch]$Usb,
    [switch]$Uefi,
    [switch]$Kvm,
    [string]$Res = "",
    [string[]]$Fwd = @()
)

$ErrorActionPreference = "Stop"

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$ProjectDir = Split-Path -Parent $ScriptDir
$BuildDir = Join-Path $ProjectDir "build"

# ── VirtualBox mode ───────────────────────────────────────────────────────────

if ($VBox) {
    # Find VBoxManage
    $vbm = Get-Command "VBoxManage" -ErrorAction SilentlyContinue
    if (-not $vbm) {
        $vbmDefault = "C:\Program Files\Oracle\VirtualBox\VBoxManage.exe"
        if (Test-Path $vbmDefault) {
            $vbm = $vbmDefault
        } else {
            Write-Host "Error: VBoxManage not found in PATH or '$vbmDefault'" -ForegroundColor Red
            Write-Host "Install VirtualBox from https://www.virtualbox.org/"
            exit 1
        }
    } else {
        $vbm = $vbm.Source
    }

    $vmName   = "anyos"
    $pipeName = "anyos-serial"
    $pipePath = "\\.\pipe\$pipeName"

    # Query current VM state (machinereadable output: VMState="running")
    $vmInfo  = & $vbm showvminfo $vmName --machinereadable 2>&1
    $stMatch = ($vmInfo | Select-String '^VMState="(\w+)"')
    $vmState = if ($stMatch) { $stMatch.Matches.Groups[1].Value } else { "unknown" }

    if ($vmState -eq "running" -or $vmState -eq "starting") {
        Write-Host "VM '$vmName' is already $vmState - skipping configuration." -ForegroundColor Yellow
    } else {
        # ── Refresh disk: detach old medium, reconvert, re-attach ────────────
        $imgPath  = Join-Path $BuildDir "anyos.img"
        $vmdkPath = Join-Path $BuildDir "anyos.vmdk"

        if (-not (Test-Path $imgPath)) {
            Write-Host "Error: $imgPath not found. Run .\scripts\build.ps1 first." -ForegroundColor Red
            exit 1
        }

        Write-Host "Refreshing disk image ..." -ForegroundColor Cyan

        # Collect all storage controller names from VM info
        $ctrlNames = @()
        foreach ($line in $vmInfo) {
            if ($line -match '^storagecontrollername\d+="(.+)"$') { $ctrlNames += $Matches[1] }
        }

        # Find which controller / port / device currently holds a HDD medium
        $diskCtrl = $null; $diskPort = 0; $diskDevice = 0; $diskMedium = $null
        foreach ($cn in $ctrlNames) {
            $pat = '^"' + [Regex]::Escape($cn) + '-(\d+)-(\d+)"="(.+\.(vmdk|vdi|vhd|img))"$'
            foreach ($line in $vmInfo) {
                if ($line -match $pat) {
                    $diskCtrl   = $cn
                    $diskPort   = [int]$Matches[1]
                    $diskDevice = [int]$Matches[2]
                    $diskMedium = $Matches[3]
                    break
                }
            }
            if ($diskCtrl) { break }
        }

        # Detach old medium and unregister it (frees the UUID).
        # Wrapped in try/catch because VBoxManage writes progress to stderr and
        # $ErrorActionPreference = "Stop" would treat that as a fatal error.
        if ($diskCtrl -and $diskMedium) {
            Write-Host "  Detaching: $diskMedium" -ForegroundColor DarkGray
            try { & $vbm storageattach $vmName --storagectl $diskCtrl --port $diskPort --device $diskDevice --medium none 2>&1 | Out-Null } catch {}
            try { & $vbm closemedium disk $diskMedium --delete 2>&1 | Out-Null } catch {}
        }

        # Remove stale VMDK file (closemedium --delete may already have done it)
        try { if (Test-Path $vmdkPath) { Remove-Item $vmdkPath -Force } } catch {}

        # Convert raw disk image to VMDK (VirtualBox assigns a fresh UUID)
        Write-Host "  Converting anyos.img -> anyos.vmdk ..." -ForegroundColor DarkGray
        & $vbm convertfromraw $imgPath $vmdkPath --format VMDK
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Error: VBoxManage convertfromraw failed." -ForegroundColor Red
            exit 1
        }

        # Determine controller / port to use for the new medium
        if (-not $diskCtrl) {
            $diskCtrl   = if ($ctrlNames.Count -gt 0) { $ctrlNames[0] } else { "SATA Controller" }
            $diskPort   = 0
            $diskDevice = 0
            if ($ctrlNames.Count -eq 0) {
                Write-Host "  Adding storage controller '$diskCtrl' ..." -ForegroundColor DarkGray
                & $vbm storagectl $vmName --name $diskCtrl --add sata --controller IntelAhci
            }
        }

        # Attach the freshly created VMDK
        Write-Host "  Attaching anyos.vmdk -> '$diskCtrl' port=$diskPort device=$diskDevice" -ForegroundColor DarkGray
        & $vbm storageattach $vmName --storagectl $diskCtrl --port $diskPort --device $diskDevice --type hdd --medium $vmdkPath
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Error: Could not attach VMDK to VM '$vmName'." -ForegroundColor Red
            exit 1
        }
        Write-Host "[OK] Disk refreshed." -ForegroundColor Green

        # ── Configure COM1 as named pipe (VirtualBox = server, we = client) ──
        Write-Host "Configuring VM '$vmName'  COM1 -> named pipe $pipePath" -ForegroundColor Cyan
        & $vbm modifyvm $vmName --uart1 0x3f8 4
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Error: Could not configure UART (VM locked or name wrong?)." -ForegroundColor Red
            exit 1
        }
        & $vbm modifyvm $vmName --uartmode1 server $pipePath
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Error: Could not set UART mode to named-pipe server." -ForegroundColor Red
            exit 1
        }

        # Start the VM with GUI so the user can also see the display
        Write-Host "Starting VirtualBox VM '$vmName'..." -ForegroundColor Cyan
        & $vbm startvm $vmName --type gui
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Error: Failed to start VM '$vmName'." -ForegroundColor Red
            exit 1
        }
    }

    # Wait for VirtualBox to create the named pipe (up to 20 s)
    Write-Host "Waiting for serial pipe $pipePath ..." -ForegroundColor Cyan
    $pipe    = $null
    $deadline = (Get-Date).AddSeconds(20)
    while ((Get-Date) -lt $deadline) {
        try {
            $pipe = New-Object System.IO.Pipes.NamedPipeClientStream(
                ".", $pipeName,
                [System.IO.Pipes.PipeDirection]::In,
                [System.IO.Pipes.PipeOptions]::None
            )
            $pipe.Connect(500)   # 0.5 s per attempt
            break
        } catch {
            if ($null -ne $pipe) { $pipe.Dispose(); $pipe = $null }
            Start-Sleep -Milliseconds 300
        }
    }

    if ($null -eq $pipe -or -not $pipe.IsConnected) {
        Write-Host "Error: Could not connect to $pipePath after 20 s." -ForegroundColor Red
        Write-Host "Make sure the VM is running and COM1 is enabled in VirtualBox settings."
        exit 1
    }

    Write-Host ""
    Write-Host ("=" * 60) -ForegroundColor Magenta
    Write-Host "  anyOS Serial Output  (Ctrl+C to disconnect)" -ForegroundColor Magenta
    Write-Host ("=" * 60) -ForegroundColor Magenta
    Write-Host ""

    try {
        $buf = New-Object byte[] 512
        while ($true) {
            $read = $pipe.Read($buf, 0, $buf.Length)
            if ($read -le 0) { break }
            # Decode ASCII; strip bare CR so lines render correctly in PowerShell
            $text = [System.Text.Encoding]::ASCII.GetString($buf, 0, $read)
            $text = $text -replace "`r`n", "`n" -replace "`r", ""
            Write-Host -NoNewline $text
        }
    } catch {
        # IOException = VM shut down (normal); anything else print message
        if ($_.Exception -isnot [System.IO.IOException]) {
            Write-Host "`nPipe error: $($_.Exception.Message)" -ForegroundColor Yellow
        }
    } finally {
        if ($null -ne $pipe) { $pipe.Dispose() }
    }

    Write-Host ""
    Write-Host ("=" * 60) -ForegroundColor Magenta
    Write-Host "  Serial session ended." -ForegroundColor Magenta
    Write-Host ("=" * 60) -ForegroundColor Magenta
    exit 0
}

# ── VMware Workstation mode ───────────────────────────────────────────────────

if ($VMwareWS) {
    # Find VBoxManage (for raw → VMDK conversion)
    $vbm = Get-Command "VBoxManage" -ErrorAction SilentlyContinue
    if (-not $vbm) {
        $vbmDefault = "C:\Program Files\Oracle\VirtualBox\VBoxManage.exe"
        if (Test-Path $vbmDefault) {
            $vbm = $vbmDefault
        } else {
            Write-Host "Error: VBoxManage not found (needed for VMDK conversion)" -ForegroundColor Red
            exit 1
        }
    } else {
        $vbm = $vbm.Source
    }

    # Find vmrun
    $vmrun = Get-Command "vmrun" -ErrorAction SilentlyContinue
    if (-not $vmrun) {
        $vmrunDefault = "C:\Program Files (x86)\VMware\VMware Workstation\vmrun.exe"
        if (Test-Path $vmrunDefault) {
            $vmrun = $vmrunDefault
        } else {
            Write-Host "Error: vmrun not found in PATH or '$vmrunDefault'" -ForegroundColor Red
            exit 1
        }
    } else {
        $vmrun = $vmrun.Source
    }

    $vmName   = "anyos"
    $pipeName = "anyos-serial"
    $pipePath = "\\.\pipe\$pipeName"

    # ── Locate .vmx file ─────────────────────────────────────────────────
    $vmxPath = $null

    # 1. Environment variable override
    if ($env:ANYOS_VMX -and (Test-Path $env:ANYOS_VMX)) {
        $vmxPath = $env:ANYOS_VMX
    }

    # 2. Parse VMware inventory
    if (-not $vmxPath) {
        $invPath = "$env:APPDATA\VMware\inventory.vmls"
        if (Test-Path $invPath) {
            foreach ($line in (Get-Content $invPath)) {
                if ($line -match '\.config\s*=\s*"(.+[/\\]anyos[^"]*\.vmx)"') {
                    $candidate = $Matches[1]
                    if (Test-Path $candidate) { $vmxPath = $candidate; break }
                }
            }
        }
    }

    # 3. Search default directories
    if (-not $vmxPath) {
        $searchPaths = @(
            "$env:USERPROFILE\Documents\Virtual Machines\$vmName\$vmName.vmx",
            "$env:USERPROFILE\Virtual Machines\$vmName\$vmName.vmx"
        )
        foreach ($p in $searchPaths) {
            if (Test-Path $p) { $vmxPath = $p; break }
        }
    }

    if (-not $vmxPath) {
        Write-Host "Error: Could not find '$vmName' VM." -ForegroundColor Red
        Write-Host "Set `$env:ANYOS_VMX to the path of your .vmx file."
        exit 1
    }

    $vmDir = Split-Path -Parent $vmxPath
    Write-Host "VMware VM: $vmxPath" -ForegroundColor Cyan

    # ── Check if VM is already running ───────────────────────────────────
    $runList = (& $vmrun list 2>&1) | Out-String
    $isRunning = $runList.ToLower().Contains($vmxPath.ToLower())

    if ($isRunning) {
        Write-Host "VM '$vmName' is already running - skipping configuration." -ForegroundColor Yellow
    } else {
        # ── Refresh disk image ───────────────────────────────────────────
        $imgPath = Join-Path $BuildDir "anyos.img"
        if (-not (Test-Path $imgPath)) {
            Write-Host "Error: $imgPath not found. Run .\scripts\build.ps1 first." -ForegroundColor Red
            exit 1
        }

        # Read .vmx to find the existing disk filename
        $vmxLines = Get-Content $vmxPath
        $diskFileName = $null
        foreach ($line in $vmxLines) {
            if ($line -match '^\s*(scsi|sata|ide|nvme)\d+:\d+\.fileName\s*=\s*"(.+\.vmdk)"') {
                $diskFileName = $Matches[2]
                break
            }
        }
        if (-not $diskFileName) { $diskFileName = "$vmName.vmdk" }

        if ([System.IO.Path]::IsPathRooted($diskFileName)) {
            $diskFullPath = $diskFileName
        } else {
            $diskFullPath = Join-Path $vmDir $diskFileName
        }

        Write-Host "Refreshing disk image ..." -ForegroundColor Cyan

        # Remove old VMDK files (flat, split, descriptor) and stale locks
        # Delete the exact target path first (may be outside $vmDir, e.g. build/)
        $diskDir = Split-Path -Parent $diskFullPath
        $baseName = [System.IO.Path]::GetFileNameWithoutExtension($diskFileName)
        foreach ($dir in @($diskDir, $vmDir) | Select-Object -Unique) {
            Get-ChildItem -Path $dir -Filter "$baseName*.vmdk" -ErrorAction SilentlyContinue |
                Remove-Item -Force -ErrorAction SilentlyContinue
        }
        Get-ChildItem -Path $vmDir -Directory -Filter "*.lck" -ErrorAction SilentlyContinue |
            Remove-Item -Recurse -Force -ErrorAction SilentlyContinue

        Write-Host "  Converting anyos.img -> $diskFullPath ..." -ForegroundColor DarkGray
        & $vbm convertfromraw $imgPath $diskFullPath --format VMDK
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Error: VBoxManage convertfromraw failed." -ForegroundColor Red
            exit 1
        }
        Write-Host "[OK] Disk refreshed." -ForegroundColor Green

        # ── Configure serial port as named pipe in .vmx ──────────────────
        Write-Host "Configuring COM1 -> named pipe $pipePath" -ForegroundColor Cyan
        $vmxLines = $vmxLines | Where-Object { $_ -notmatch '^\s*serial0\.' }
        $vmxLines += 'serial0.present = "TRUE"'
        $vmxLines += 'serial0.fileType = "pipe"'
        $vmxLines += "serial0.fileName = `"$pipePath`""
        $vmxLines += 'serial0.pipe.endPoint = "server"'
        $vmxLines += 'serial0.yieldOnMsrRead = "TRUE"'
        $vmxLines += 'serial0.startConnected = "TRUE"'
        $vmxLines | Set-Content $vmxPath -Encoding ASCII

        # ── Start the VM ─────────────────────────────────────────────────
        Write-Host "Starting VMware VM '$vmName' ..." -ForegroundColor Cyan
        & $vmrun start $vmxPath gui
        if ($LASTEXITCODE -ne 0) {
            Write-Host "Error: Failed to start VM." -ForegroundColor Red
            exit 1
        }
    }

    # ── Connect to serial named pipe ─────────────────────────────────────
    Write-Host "Waiting for serial pipe $pipePath ..." -ForegroundColor Cyan
    $pipe    = $null
    $deadline = (Get-Date).AddSeconds(30)
    while ((Get-Date) -lt $deadline) {
        try {
            $pipe = New-Object System.IO.Pipes.NamedPipeClientStream(
                ".", $pipeName,
                [System.IO.Pipes.PipeDirection]::InOut,
                [System.IO.Pipes.PipeOptions]::None
            )
            $pipe.Connect(500)
            break
        } catch {
            if ($null -ne $pipe) { $pipe.Dispose(); $pipe = $null }
            Start-Sleep -Milliseconds 300
        }
    }

    if ($null -eq $pipe -or -not $pipe.IsConnected) {
        Write-Host "Error: Could not connect to $pipePath after 30 s." -ForegroundColor Red
        exit 1
    }

    Write-Host ""
    Write-Host ("=" * 60) -ForegroundColor Magenta
    Write-Host "  anyOS Serial Output  (Ctrl+C to disconnect)" -ForegroundColor Magenta
    Write-Host ("=" * 60) -ForegroundColor Magenta
    Write-Host ""

    try {
        $buf = New-Object byte[] 512
        while ($true) {
            $read = $pipe.Read($buf, 0, $buf.Length)
            if ($read -le 0) { break }
            $text = [System.Text.Encoding]::ASCII.GetString($buf, 0, $read)
            $text = $text -replace "`r`n", "`n" -replace "`r", ""
            Write-Host -NoNewline $text
        }
    } catch {
        if ($_.Exception -isnot [System.IO.IOException]) {
            Write-Host "`nPipe error: $($_.Exception.Message)" -ForegroundColor Yellow
        }
    } finally {
        if ($null -ne $pipe) { $pipe.Dispose() }
    }

    Write-Host ""
    Write-Host ("=" * 60) -ForegroundColor Magenta
    Write-Host "  Serial session ended." -ForegroundColor Magenta
    Write-Host ("=" * 60) -ForegroundColor Magenta
    exit 0
}

# ── Find QEMU ────────────────────────────────────────────────────────────────

$qemu = Get-Command "qemu-system-x86_64" -ErrorAction SilentlyContinue
if (-not $qemu) {
    # Check default install location
    $qemuDefault = "C:\Program Files\qemu\qemu-system-x86_64.exe"
    if (Test-Path $qemuDefault) {
        $qemu = $qemuDefault
    } else {
        Write-Host "Error: qemu-system-x86_64 not found in PATH or $qemuDefault" -ForegroundColor Red
        Write-Host "Install with: winget install SoftwareFreedomConservancy.QEMU"
        exit 1
    }
} else {
    $qemu = $qemu.Source
}

# ── VGA selection ────────────────────────────────────────────────────────────

$vga = "std"
$vgaLabel = "Bochs VGA (standard)"

if ($Vmware) {
    $vga = "vmware"
    $vgaLabel = "VMware SVGA II (accelerated)"
} elseif ($Virtio) {
    $vga = "virtio"
    $vgaLabel = "Virtio GPU (paravirtualized)"
}

# ── Resolution validation ───────────────────────────────────────────────────

$minResW = 1024
$minResH = 768

if ($Res -ne "") {
    if (-not $Virtio) {
        Write-Host "Error: -Res is only supported with -Virtio (VirtIO GPU sets resolution via device properties)" -ForegroundColor Red
        Write-Host "Bochs VGA and VMware SVGA set resolution from the guest OS."
        exit 1
    }
    if ($Res -match '^(\d+)x(\d+)$') {
        $resW = [int]$Matches[1]
        $resH = [int]$Matches[2]
    } else {
        Write-Host "Error: Invalid -Res format '$Res'. Expected WIDTHxHEIGHT (e.g. 1280x1024)" -ForegroundColor Red
        exit 1
    }
}

# VirtIO GPU: default to 1024x768 if no -Res specified
if ($Virtio -and $Res -eq "") {
    $Res = "${minResW}x${minResH}"
    $resW = $minResW
    $resH = $minResH
}

# Enforce minimum resolution (1024x768)
if ($Res -ne "") {
    if ($resW -lt $minResW -or $resH -lt $minResH) {
        Write-Host "Error: Resolution ${resW}x${resH} is below minimum ${minResW}x${minResH}" -ForegroundColor Red
        exit 1
    }
}

# ── Build QEMU arguments ────────────────────────────────────────────────────

$args = @()

# Disk / boot mode
if ($Cdrom) {
    $image = Join-Path $BuildDir "anyos.iso"
    $driveLabel = "CD-ROM (ISO 9660)"
    $args += "-cdrom", $image, "-boot", "d"
} elseif ($Uefi) {
    $image = Join-Path $BuildDir "anyos-uefi.img"
    $driveLabel = "UEFI (GPT)"

    # Find OVMF firmware
    $ovmfPaths = @(
        "C:\Program Files\qemu\share\edk2-x86_64-code.fd",
        "C:\Program Files\qemu\share\OVMF_CODE.fd"
    )
    $ovmfFw = $null
    foreach ($p in $ovmfPaths) {
        if (Test-Path $p) { $ovmfFw = $p; break }
    }
    if (-not $ovmfFw) {
        Write-Host "Error: OVMF firmware not found." -ForegroundColor Red
        Write-Host "Searched:"
        foreach ($p in $ovmfPaths) { Write-Host "  $p" }
        exit 1
    }
    $args += "-drive", "if=pflash,format=raw,readonly=on,file=$ovmfFw"
    $args += "-drive", "format=raw,file=$image"
} else {
    $image = Join-Path $BuildDir "anyos.img"
    if ($Ide) {
        $driveLabel = "IDE (PIO)"
        $args += "-drive", "format=raw,file=$image"
    } else {
        $driveLabel = "AHCI (DMA)"
        $args += "-drive", "id=hd0,if=none,format=raw,file=$image"
        $args += "-device", "ich9-ahci,id=ahci"
        $args += "-device", "ide-hd,drive=hd0,bus=ahci.0"
    }
}

# Check image exists
if (-not (Test-Path $image)) {
    Write-Host "Error: Image not found at $image" -ForegroundColor Red
    if ($Cdrom) {
        Write-Host "Run: .\scripts\build.ps1 -Iso"
    } else {
        Write-Host "Run: .\scripts\build.ps1"
    }
    exit 1
}

# Core settings
$args += "-m", "1024M"
$args += "-smp", "cpus=4"
$args += "-serial", "stdio"

# VGA device: VirtIO always uses explicit -device with edid=on for reliable resolution
if ($Virtio) {
    $args += "-vga", "none"
    $args += "-device", "virtio-vga,edid=on,xres=$resW,yres=$resH"
    $vgaLabel = "Virtio GPU (${resW}x${resH})"
} else {
    $args += "-vga", $vga
}
# Port forwarding rules
$fwdRules = ""
foreach ($rule in $Fwd) {
    if ($rule -match '^(\d+):(\d+)$') {
        $fwdRules += ",hostfwd=tcp::$($Matches[1])-:$($Matches[2])"
    } else {
        Write-Host "Error: Invalid -Fwd format '$rule'. Expected HOST:GUEST (e.g. 2222:22)" -ForegroundColor Red
        exit 1
    }
}
$args += "-netdev", "user,id=net0$fwdRules"
$args += "-device", "e1000,netdev=net0"
$args += "-no-reboot"
$args += "-no-shutdown"

# Audio (Windows uses wasapi backend)
$audioLabel = ""
if ($Audio) {
    $args += "-device", "AC97,audiodev=audio0"
    $args += "-audiodev", "wasapi,id=audio0"
    $audioLabel = ", audio: AC'97"
}

# USB
$usbLabel = ""
if ($Usb) {
    $args += "-usb"
    $args += "-device", "usb-kbd"
    $args += "-device", "usb-mouse"
    $usbLabel = ", USB: keyboard + mouse"
}

# Hardware virtualization (WHPX)
$kvmLabel = ""
if ($Kvm) {
    # Check if Windows Hypervisor Platform is available
    $whpx = Get-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform -ErrorAction SilentlyContinue
    if ($whpx -and $whpx.State -eq "Enabled") {
        $args += "-accel", "whpx"
        $args += "-cpu", "max"
        $kvmLabel = ", WHPX enabled"
    } else {
        Write-Host "Error: Windows Hypervisor Platform (WHPX) is not enabled." -ForegroundColor Red
        Write-Host "Enable with: Enable-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform"
        Write-Host "A reboot is required after enabling."
        exit 1
    }
}

# VirtIO GPU: add USB tablet for absolute mouse (no VMware backdoor)
if ($Virtio -and -not $Usb) {
    $args += "-usb"
    $args += "-device", "usb-tablet"
}

# Resolution label
$resLabel = ""
if ($Res -ne "") {
    $resLabel = ", res: $Res"
}

# Port forwarding label
$fwdLabel = ""
if ($Fwd.Count -gt 0) {
    $fwdLabel = ", fwd: $($Fwd -join ',')"
}

# ── Launch ───────────────────────────────────────────────────────────────────

Write-Host "Starting anyOS with $vgaLabel (-vga $vga), disk: $driveLabel$audioLabel$usbLabel$kvmLabel$resLabel$fwdLabel" -ForegroundColor Cyan
& $qemu @args
