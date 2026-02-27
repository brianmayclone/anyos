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
  ${SYSROOT_DIR}/Libraries/system/buildsystem/.stamp
  ${C_TOOLCHAIN_DEPS}
  ${CXX_TOOLCHAIN_DEPS}
)

# ============================================================
# 6. Disk Image
# ============================================================
set(MKIMAGE_RESET_FLAG "")
if(ANYOS_RESET)
  set(MKIMAGE_RESET_FLAG "--reset")
endif()

add_custom_command(
  OUTPUT ${DISK_IMAGE}
  COMMAND ${MKIMAGE_EXECUTABLE}
    --stage1 ${CMAKE_BINARY_DIR}/stage1.bin
    --stage2 ${CMAKE_BINARY_DIR}/stage2.bin
    --kernel ${KERNEL_ELF}
    --output ${DISK_IMAGE}
    --image-size 256
    --sysroot ${SYSROOT_DIR}
    --fs-start 8192
    ${MKIMAGE_RESET_FLAG}
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
    ${C_TOOLCHAIN_DEPS}
    ${MKIMAGE_EXECUTABLE}
    ${PROVISION_DEPS}
  COMMENT "Creating bootable disk image (256 MiB, exFAT filesystem)"
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
    --image-size 64
    --sysroot ${SYSROOT_DIR}
    ${MKIMAGE_RESET_FLAG}
  DEPENDS
    ${UEFI_BOOTLOADER_EFI}
    ${KERNEL_ELF}
    ${RUST_USER_BINS}
    ${SYSTEM_BINS}
    ${DLL_BINS}
    ${SYSROOT_DIR}/.stamp
    ${C_TOOLCHAIN_DEPS}
    ${MKIMAGE_EXECUTABLE}
  COMMENT "Creating UEFI bootable disk image (GPT + ESP + exFAT)"
)

add_custom_target(uefi-bootloader DEPENDS ${UEFI_BOOTLOADER_EFI})
add_custom_target(uefi-image DEPENDS ${UEFI_DISK_IMAGE} programs)

add_custom_target(run-uefi
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive if=pflash,format=raw,readonly=on,file=${OVMF_FW}
    -drive format=raw,file=${UEFI_DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga vmware
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${UEFI_DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with UEFI (OVMF + VMware SVGA)"
)

add_custom_target(run-uefi-std
  COMMAND ${QEMU_EXECUTABLE}
    ${QEMU_CPU_FLAGS}
    -drive if=pflash,format=raw,readonly=on,file=${OVMF_FW}
    -drive format=raw,file=${UEFI_DISK_IMAGE}
    -m 1024M
    -smp cpus=4
    -serial stdio
    -vga std
    -netdev user,id=net0 -device e1000,netdev=net0
    -no-reboot -no-shutdown
  DEPENDS ${UEFI_DISK_IMAGE}
  USES_TERMINAL
  COMMENT "Launching anyOS in QEMU with UEFI (OVMF + Bochs VGA)"
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
