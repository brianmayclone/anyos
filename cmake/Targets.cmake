# ============================================================
# Aggregate Programs Target
# ============================================================
add_custom_target(programs DEPENDS
  ${RUST_USER_BINS}
  ${SYSTEM_BINS}
  ${APP_BINS}
  ${DLL_BINS}
  ${DRIVER_BINS}
  ${SYSROOT_DIR}/.stamp
  ${SELFHOST_SYSROOT_DEPS}
  ${SYSROOT_DIR}/Libraries/system/buildsystem/.stamp
  ${C_TOOLCHAIN_DEPS}
  ${CXX_TOOLCHAIN_DEPS}
  bootloader_sysroot
)

# ============================================================
# 6. Disk Image
# ============================================================
set(MKIMAGE_RESET_FLAG "")
if(ANYOS_RESET)
  set(MKIMAGE_RESET_FLAG "--reset")
endif()

# Generate boot font for bootloader splash screen
set(BOOT_FONT_BIN ${CMAKE_BINARY_DIR}/boot_font.bin)
add_custom_command(
  OUTPUT ${BOOT_FONT_BIN}
  COMMAND ${PYTHON_EXECUTABLE} ${CMAKE_SOURCE_DIR}/tools/generate_boot_font.py ${BOOT_FONT_BIN}
  DEPENDS ${CMAKE_SOURCE_DIR}/tools/generate_boot_font.py
  COMMENT "Generating boot font (8x16 bitmap)"
)

# Boot assets
set(BOOT_CFG ${CMAKE_SOURCE_DIR}/sysroot/boot/boot.cfg)
set(BOOT_LOGO ${CMAKE_SOURCE_DIR}/kernel/src/graphics/boot_logo.bin)

# Optional post-processing step: append a CoreFS system partition to the
# exFAT-only image produced by mkimage.  Controlled by the top-level
# ANYOS_SYSTEM_FS cache variable — defaults to "exfat" (no-op) for full
# backwards compatibility.
if(ANYOS_SYSTEM_FS STREQUAL "corefs")
  # Where the CoreFS partition descriptor lands in the MBR depends on
  # whether the base image is single-partition (slot 0 = exFAT, CoreFS
  # goes into slot 1) or dual-partition (slot 0 = /boot, slot 1 = /,
  # CoreFS goes into slot 2).
  #
  # In dual-partition layouts we also populate the CoreFS volume with
  # the contents of sysroot/System, so that once the kernel mounts
  # the partition under /System (via the auto-activate path in Phase
  # 5c/1) every binary, library and app bundle is served from CoreFS.
  # In single-partition layouts we leave the CoreFS partition empty —
  # it exists as a sandbox/test target next to the main exFAT root.
  if(ANYOS_DUAL_PARTITION)
    set(COREFS_MBR_SLOT "2")
    set(COREFS_POPULATE_ARGS --populate ${SYSROOT_DIR}/System)
  else()
    set(COREFS_MBR_SLOT "1")
    set(COREFS_POPULATE_ARGS)
  endif()
  set(COREFS_POST_CMD
    COMMAND ${PYTHON_EXECUTABLE}
      ${CMAKE_SOURCE_DIR}/tools/anyos_append_corefs.py
      --image ${DISK_IMAGE}
      --size ${ANYOS_SYSTEM_FS_SIZE_MIB}M
      --slot ${COREFS_MBR_SLOT}
      --mkfs-corefs-host ${MKFS_COREFS_HOST_EXECUTABLE}
      --label system
      ${COREFS_POPULATE_ARGS})
  set(COREFS_POST_DEPS ${MKFS_COREFS_HOST_EXECUTABLE})
else()
  set(COREFS_POST_CMD "")
  set(COREFS_POST_DEPS "")
endif()

# Image layout:
#
#   * Single-partition (default)  — built by the fast C mkimage
#     tool.  Kernel + all sysroot files end up on one exFAT
#     filesystem starting at sector 128.  Optional CoreFS post-
#     processing (see above) appends a CoreFS partition at the end.
#
#   * Dual-partition (ANYOS_DUAL_PARTITION=ON)  — built by the
#     Python tools/__mkimage.py.  Partition 1 (exFAT /boot, default
#     32 MiB) contains Stage2's required files + /System/krnl64;
#     Partition 2 (exFAT /) holds everything else — /System/bin,
#     /Applications, /Users, /Libraries, …  The kernel mounts them
#     as / and /boot respectively (see kernel/src/boot/x86/storage.rs).
if(ANYOS_DUAL_PARTITION)
  # Dual-partition path uses the same C mkimage, with --dual-partition
  # + --boot-partition-size-mib switching it to the two-partition
  # layout (see buildsystem/mkimage/src/mkimage.c:create_bios_image_dual).
  #
  # When ANYOS_SYSTEM_FS=corefs, mkimage skips the exFAT format on
  # Partition 2, marks the MBR slot as 0xCF (CoreFS), and invokes
  # mkfs-corefs-host as a subprocess after the image is on disk.  The
  # legacy COREFS_POST_CMD (anyos_append_corefs.py, which appended a
  # 3rd partition) is suppressed in this branch — Partition 2 *is* the
  # CoreFS root.
  set(DUAL_ROOT_FS_ARGS "")
  if(ANYOS_SYSTEM_FS STREQUAL "corefs")
    set(DUAL_ROOT_FS_ARGS
      --root-fs corefs
      --mkfs-corefs-host ${MKFS_COREFS_HOST_EXECUTABLE})
    # Suppress the legacy 3rd-partition appender — Partition 2 carries
    # CoreFS now, an additional CoreFS region would be redundant.
    set(COREFS_POST_CMD "")
    set(COREFS_POST_DEPS ${MKFS_COREFS_HOST_EXECUTABLE})
  endif()

  set(IMAGE_BUILD_CMD
    ${MKIMAGE_EXECUTABLE}
      --stage1 ${CMAKE_BINARY_DIR}/stage1.bin
      --stage2 ${CMAKE_BINARY_DIR}/stage2.bin
      --kernel ${KERNEL_ELF}
      --output ${DISK_IMAGE}
      --image-size 512
      --sysroot ${SYSROOT_DIR}
      --fs-start 128
      --boot-cfg ${BOOT_CFG}
      --boot-logo ${BOOT_LOGO}
      --boot-font ${BOOT_FONT_BIN}
      --dual-partition
      --boot-partition-size-mib ${ANYOS_BOOT_PARTITION_SIZE_MIB}
      ${DUAL_ROOT_FS_ARGS})
  set(IMAGE_BUILD_DEP ${MKIMAGE_EXECUTABLE})
  if(ANYOS_SYSTEM_FS STREQUAL "corefs")
    set(IMAGE_COMMENT "Creating dual-partition image (/boot ${ANYOS_BOOT_PARTITION_SIZE_MIB} MiB exFAT + / CoreFS)")
  else()
    set(IMAGE_COMMENT "Creating dual-partition disk image (/boot ${ANYOS_BOOT_PARTITION_SIZE_MIB} MiB + /)")
  endif()
else()
  set(IMAGE_BUILD_CMD
    ${MKIMAGE_EXECUTABLE}
      --stage1 ${CMAKE_BINARY_DIR}/stage1.bin
      --stage2 ${CMAKE_BINARY_DIR}/stage2.bin
      --kernel ${KERNEL_ELF}
      --output ${DISK_IMAGE}
      --image-size 512
      --sysroot ${SYSROOT_DIR}
      --fs-start 128
      --boot-cfg ${BOOT_CFG}
      --boot-logo ${BOOT_LOGO}
      --boot-font ${BOOT_FONT_BIN}
      ${MKIMAGE_RESET_FLAG})
  set(IMAGE_BUILD_DEP ${MKIMAGE_EXECUTABLE})
  set(IMAGE_COMMENT "Creating bootable disk image (512 MiB, exFAT filesystem)")
endif()

add_custom_command(
  OUTPUT ${DISK_IMAGE}
  COMMAND ${IMAGE_BUILD_CMD}
  ${COREFS_POST_CMD}
  DEPENDS
    ${CMAKE_BINARY_DIR}/stage1.bin
    ${CMAKE_BINARY_DIR}/stage2.bin
    ${KERNEL_ELF}
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${APP_BINS}
    ${DLL_BINS}
    ${DRIVER_BINS}
    ${SYSROOT_DIR}/.stamp
    ${SELFHOST_SYSROOT_DEPS}
    ${C_TOOLCHAIN_DEPS}
    ${IMAGE_BUILD_DEP}
    ${PROVISION_DEPS}
    ${BOOT_FONT_BIN}
    ${BOOT_CFG}
    ${BOOT_LOGO}
    ${COREFS_POST_DEPS}
  COMMENT "${IMAGE_COMMENT}"
)

# ============================================================
# QEMU CPU model — expose SSE3/SSSE3/SSE4.1/SSE4.2/POPCNT
# ============================================================
set(QEMU_CPU_FLAGS -cpu qemu64,+sse3,+ssse3,+sse4.1,+sse4.2,+popcnt)

# ============================================================
# Targets
# ============================================================
add_custom_target(bootloader DEPENDS
  ${CMAKE_BINARY_DIR}/stage1.bin
  ${CMAKE_BINARY_DIR}/stage2.bin
)

add_custom_target(kernel DEPENDS ${KERNEL_ELF})

# ============================================================
# System Manifest (for delta updates)
# ============================================================
set(SYSTEM_MANIFEST "${CMAKE_BINARY_DIR}/system-manifest.json")
add_custom_command(
  OUTPUT ${SYSTEM_MANIFEST}
  COMMAND ${MKMANIFEST_EXECUTABLE}
    --sysroot ${SYSROOT_DIR}
    --version ${ANYOS_VERSION}
    -o ${SYSTEM_MANIFEST}
  DEPENDS
    ${MKMANIFEST_EXECUTABLE}
    programs
  COMMENT "Generating system-manifest.json for update system"
)

add_custom_target(manifest DEPENDS ${SYSTEM_MANIFEST})

add_custom_target(image ALL DEPENDS ${DISK_IMAGE} programs)

add_custom_target(run
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU"
)

add_custom_target(run-vmware
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with VMware SVGA II"
)

add_custom_target(run-vmware-debug
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
    -s
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with VMware SVGA II + GDB server on :1234"
)

add_custom_target(run-ahci
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive id=hd0,if=none,format=raw,file=${DISK_IMAGE}
    -device ich9-ahci,id=ahci -device ide-hd,drive=hd0,bus=ahci.0
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with AHCI (SATA DMA)"
)

add_custom_target(run-ahci-vmware
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive id=hd0,if=none,format=raw,file=${DISK_IMAGE}
    -device ich9-ahci,id=ahci -device ide-hd,drive=hd0,bus=ahci.0
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with AHCI + VMware SVGA II"
)

add_custom_target(run-audio
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -device AC97,audiodev=audio0 -audiodev coreaudio,id=audio0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with AC'97 audio"
)

add_custom_target(run-usb
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -usb -device usb-kbd -device usb-mouse
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with UHCI USB keyboard + mouse"
)

add_custom_target(run-usb-ehci
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -device ich9-usb-ehci1,id=ehci
    -device ich9-usb-uhci1,masterbus=ehci.0,firstport=0,multifunction=on
    -device usb-kbd,bus=ehci.0,port=1
    -device usb-mouse,bus=ehci.0,port=2
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with EHCI USB keyboard + mouse"
)

add_custom_target(debug
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive format=raw,file=${DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -s -S
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU (debug mode, GDB on :1234)"
)

# ============================================================
# UEFI Boot Support
# ============================================================
set(UEFI_BOOTLOADER_EFI "${CMAKE_BINARY_DIR}/bootx64.efi")
set(UEFI_DISK_IMAGE "${CMAKE_BINARY_DIR}/anyos-uefi.img")
set(OVMF_FW "/opt/homebrew/share/qemu/edk2-x86_64-code.fd")

# Build UEFI bootloader
add_custom_command(
  OUTPUT ${UEFI_BOOTLOADER_EFI}
  COMMAND ${CMAKE_COMMAND} -E env "RUSTFLAGS=-Awarnings"
    ${CARGO_EXECUTABLE} build --release --quiet
    --manifest-path ${CMAKE_SOURCE_DIR}/bootloader/uefi/Cargo.toml
    --target-dir ${CMAKE_BINARY_DIR}/uefi-boot
    --target x86_64-unknown-uefi
    -Zbuild-std=core,alloc
    -Zbuild-std-features=compiler-builtins-mem
  COMMAND ${CMAKE_COMMAND} -E copy
    ${CMAKE_BINARY_DIR}/uefi-boot/x86_64-unknown-uefi/release/anyos-uefi-boot.efi
    ${UEFI_BOOTLOADER_EFI}
  DEPENDS
    ${CMAKE_SOURCE_DIR}/bootloader/uefi/Cargo.toml
    ${CMAKE_SOURCE_DIR}/bootloader/uefi/src/main.rs
  COMMENT "Building UEFI bootloader"
)

# Create UEFI disk image (GPT + ESP + exFAT data partition)
add_custom_command(
  OUTPUT ${UEFI_DISK_IMAGE}
  COMMAND ${MKIMAGE_EXECUTABLE} --uefi
    --bootloader ${UEFI_BOOTLOADER_EFI}
    --kernel ${KERNEL_ELF}
    --output ${UEFI_DISK_IMAGE}
    --image-size 512
    --sysroot ${SYSROOT_DIR}
    ${MKIMAGE_RESET_FLAG}
  DEPENDS
    ${UEFI_BOOTLOADER_EFI}
    ${KERNEL_ELF}
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${DLL_BINS}
    ${SYSROOT_DIR}/.stamp
    ${SELFHOST_SYSROOT_DEPS}
    ${C_TOOLCHAIN_DEPS}
    ${MKIMAGE_EXECUTABLE}
  COMMENT "Creating UEFI bootable disk image (512 MiB, GPT + ESP + exFAT)"
)

add_custom_target(uefi-bootloader DEPENDS ${UEFI_BOOTLOADER_EFI})
add_custom_target(uefi-image DEPENDS ${UEFI_DISK_IMAGE} programs)

add_custom_target(run-uefi
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive if=pflash,format=raw,readonly=on,file=${OVMF_FW}
    -device ahci,id=ahci0
    -drive id=disk0,file=${UEFI_DISK_IMAGE},format=raw,if=none
    -device ide-hd,drive=disk0,bus=ahci0.0
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${UEFI_DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with UEFI (OVMF + AHCI + VMware SVGA)"
)

add_custom_target(run-uefi-std
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive if=pflash,format=raw,readonly=on,file=${OVMF_FW}
    -device ahci,id=ahci0
    -drive id=disk0,file=${UEFI_DISK_IMAGE},format=raw,if=none
    -device ide-hd,drive=disk0,bus=ahci0.0
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${UEFI_DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with UEFI (OVMF + AHCI + Bochs VGA)"
)

# ============================================================
# ISO 9660 Live CD Support (El Torito BIOS boot)
# ============================================================
set(ISO_IMAGE "${CMAKE_BINARY_DIR}/anyos.iso")

add_custom_command(
  OUTPUT ${ISO_IMAGE}
  COMMAND ${MKIMAGE_EXECUTABLE} --iso
    --stage1 ${CMAKE_BINARY_DIR}/stage1.bin
    --stage2 ${CMAKE_BINARY_DIR}/stage2.bin
    --kernel ${KERNEL_ELF}
    --output ${ISO_IMAGE}
    --sysroot ${SYSROOT_DIR}
  DEPENDS
    ${CMAKE_BINARY_DIR}/stage1.bin
    ${CMAKE_BINARY_DIR}/stage2.bin
    ${KERNEL_ELF}
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${DLL_BINS}
    ${SYSROOT_DIR}/.stamp
    ${ISO_SYSROOT_DEPS}
    ${C_TOOLCHAIN_DEPS}
    ${MKIMAGE_EXECUTABLE}
  COMMENT "Creating bootable ISO 9660 image (El Torito, BIOS boot)"
)

add_custom_target(iso DEPENDS ${ISO_IMAGE} programs)

add_custom_target(run-cdrom
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -cdrom ${ISO_IMAGE}
    -boot d
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${ISO_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS from ISO (CD-ROM boot with VMware SVGA)"
)

add_custom_target(run-cdrom-std
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -cdrom ${ISO_IMAGE}
    -boot d
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${ISO_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS from ISO (CD-ROM boot with Bochs VGA)"
)

add_custom_target(run-cdrom-with-disk
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive format=raw,file=${DISK_IMAGE}
    -cdrom ${ISO_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${DISK_IMAGE} ${ISO_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS with HDD + CD-ROM (VMware SVGA)"
)

# ============================================================
# ARM64 Disk Image + QEMU Targets
# ============================================================
set(KERNEL_ARM64_ELF "${CMAKE_BINARY_DIR}/kernel/aarch64-anyos/release/anyos_kernel.elf")
set(ARM64_DISK_IMAGE "${CMAKE_BINARY_DIR}/anyos-arm64.img")

# Create ARM64 disk image (GPT + exFAT data partition, no bootloader)
add_custom_command(
  OUTPUT ${ARM64_DISK_IMAGE}
  COMMAND ${MKIMAGE_EXECUTABLE} --arm64
    --output ${ARM64_DISK_IMAGE}
    --image-size 512
    --sysroot ${SYSROOT_DIR}
    ${MKIMAGE_RESET_FLAG}
  DEPENDS
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${APP_BINS}
    ${DLL_BINS}
    ${SYSROOT_DIR}/.stamp
    ${SELFHOST_SYSROOT_DEPS}
    ${MKIMAGE_EXECUTABLE}
  COMMENT "Creating ARM64 disk image (512 MiB, GPT + exFAT)"
)

add_custom_target(arm64-image DEPENDS ${ARM64_DISK_IMAGE} programs)

add_custom_target(run-arm64
  COMMAND qemu-system-aarch64
    -M virt,gic-version=3 -cpu cortex-a72
    -m 512M -nographic
    -kernel ${KERNEL_ARM64_ELF}
    -drive if=none,format=raw,file=${ARM64_DISK_IMAGE},id=hd0
    -device virtio-blk-device,drive=hd0
    -device virtio-gpu-device
    -device virtio-keyboard-device
    -device virtio-mouse-device
    -serial stdio
    -no-reboot -no-shutdown
  DEPENDS kernel-arm64 ${ARM64_DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU (AArch64, virt machine + disk)"
)

add_custom_target(run-arm64-debug
  COMMAND qemu-system-aarch64
    -M virt,gic-version=3 -cpu cortex-a72
    -m 512M -nographic
    -kernel ${KERNEL_ARM64_ELF}
    -drive if=none,format=raw,file=${ARM64_DISK_IMAGE},id=hd0
    -device virtio-blk-device,drive=hd0
    -device virtio-gpu-device
    -device virtio-keyboard-device
    -device virtio-mouse-device
    -serial stdio
    -s -S
    -no-reboot -no-shutdown
  DEPENDS kernel-arm64 ${ARM64_DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU (AArch64, debug mode, GDB on :1234)"
)
