ENTRY(_boot_start)
OUTPUT_FORMAT(elf64-littleaarch64)

/*
 * Memory layout for QEMU virt machine:
 *   Physical RAM starts at 0x40000000.
 *   Kernel loaded at KERNEL_LMA = 0x40400000 (1 MiB into RAM).
 *
 * The boot page tables use 1 GiB block mapping:
 *   VA 0xFFFF000080000000 → PA 0x40000000
 *
 * Therefore:  KERNEL_VMA = 0xFFFF000080000000 + (KERNEL_LMA - 0x40000000)
 *           = 0xFFFF000080400000
 */
KERNEL_VMA = 0xFFFF000080400000;
KERNEL_LMA = 0x40400000;

SECTIONS
{
    /* ================================================================
     * Boot code — runs at PHYSICAL address with MMU off.
     * VMA = LMA so the ELF entry point is a physical address that
     * QEMU can jump to directly.
     * ================================================================ */
    . = KERNEL_LMA;

    .text.boot ALIGN(4K) : {
        *(.text.boot)
    }

    /* Boot page tables: 16 KiB (4 × 4 KiB), zeroed by boot code.
     * Placed at physical address right after boot code so adrp can reach. */
    .boot_pt ALIGN(4K) (NOLOAD) : {
        _boot_page_tables = .;
        . += 16384;
    }

    _boot_phys_end = .;

    /* ================================================================
     * Kernel — runs at VIRTUAL (higher-half) address after MMU enable.
     * LMA is contiguous in physical memory after the boot area.
     * ================================================================ */
    . = KERNEL_VMA + (_boot_phys_end - KERNEL_LMA);

    .text ALIGN(4K) : AT(ADDR(.text) - KERNEL_VMA + KERNEL_LMA)
    {
        _kernel_start = .;
        *(.text .text.*)
    }

    .rodata ALIGN(4K) : AT(ADDR(.rodata) - KERNEL_VMA + KERNEL_LMA)
    {
        *(.rodata .rodata.*)
    }

    .data ALIGN(4K) : AT(ADDR(.data) - KERNEL_VMA + KERNEL_LMA)
    {
        *(.data .data.*)
    }

    .bss ALIGN(4K) (NOLOAD) : AT(ADDR(.bss) - KERNEL_VMA + KERNEL_LMA)
    {
        _bss_start = .;
        *(.bss .bss.*)
        *(COMMON)
        _bss_end = .;
    }

    /* Boot stack: 512 KiB dedicated region above BSS. */
    .boot_stack (NOLOAD) : AT(ADDR(.boot_stack) - KERNEL_VMA + KERNEL_LMA)
    {
        _boot_stack_bottom = .;
        . += 0x80000;    /* 512 KiB */
        _boot_stack_top = .;
    }

    _kernel_end = .;

    /DISCARD/ :
    {
        *(.comment)
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note.*)
    }
}
