/*
 * boot.S — ARM64 kernel entry point for QEMU virt machine.
 *
 * Entry conditions (QEMU -kernel or U-Boot):
 *   - CPU in EL1 or EL2 (QEMU: EL1, U-Boot: typically EL2)
 *   - X0 = physical address of DTB
 *   - MMU off, caches may be on or off
 *   - BSP only; APs are parked via PSCI
 *
 * This code:
 *   1. Drops from EL2 to EL1 if needed
 *   2. Sets up initial identity-mapped page tables
 *   3. Enables the MMU
 *   4. Sets up the boot stack
 *   5. Zeros BSS
 *   6. Jumps to kernel_main(dtb_addr)
 */

.section .text.boot, "ax"
.global _boot_start
.type _boot_start, %function

_boot_start:
    /* Save DTB address from x0 */
    mov     x19, x0

    /* Check current exception level */
    mrs     x1, CurrentEL
    and     x1, x1, #0xC       /* Extract EL bits [3:2] */
    cmp     x1, #0x8           /* EL2? */
    b.ne    .at_el1

    /* --- EL2 → EL1 transition --- */
    /* Configure EL2 for EL1 entry */
    mov     x0, #(1 << 31)     /* HCR_EL2.RW = 1 (AArch64 at EL1) */
    msr     hcr_el2, x0

    /* SPSR_EL2: return to EL1h with interrupts masked */
    mov     x0, #0x3C5         /* EL1h, DAIF masked */
    msr     spsr_el2, x0

    /* ELR_EL2: return address */
    adr     x0, .at_el1
    msr     elr_el2, x0

    eret

.at_el1:
    /* --- Now running at EL1 --- */

    /* Disable MMU and caches while setting up page tables */
    mrs     x0, sctlr_el1
    bic     x0, x0, #(1 << 0)  /* M: MMU disable */
    bic     x0, x0, #(1 << 2)  /* C: Data cache disable */
    bic     x0, x0, #(1 << 12) /* I: Instruction cache disable */
    msr     sctlr_el1, x0
    isb

    /* Set up MAIR_EL1 */
    ldr     x0, =0x000000000000FF44
    msr     mair_el1, x0

    /* Set up TCR_EL1 for 48-bit VA, 4 KiB granule */
    ldr     x0, =0x00000005B5103510
    msr     tcr_el1, x0
    isb

    /* Set up initial identity-mapped page tables */
    /* We use 1 GiB block mappings at PGD level for simplicity:
     *   PGD[0] → 0x00000000 - 0x3FFFFFFF (MMIO: UART, GIC, etc.)
     *   PGD[1] → 0x40000000 - 0x7FFFFFFF (RAM, where kernel is loaded)
     *
     * For the kernel higher-half mapping (0xFFFF0000_80000000):
     *   We'll set TTBR1_EL1 to a separate page table with:
     *   PGD[0] → maps 0xFFFF000000000000 (not needed yet)
     *   PGD[2] → maps 0xFFFF000080000000 → physical 0x40000000
     *
     * NOTE: For initial boot, we use identity mapping only via TTBR0.
     * Higher-half mapping will be set up by the Rust memory init code.
     */

    /* Clear page table area (16 KiB at a known location) */
    ldr     x0, =_boot_page_tables
    mov     x1, #4096           /* 4 KiB per table, 4 tables = 16 KiB */
    lsl     x1, x1, #2
    mov     x2, #0
.clear_pt:
    stp     xzr, xzr, [x0], #16
    sub     x1, x1, #16
    cbnz    x1, .clear_pt

    /* Build PGD (Level 0) for TTBR0 (identity map) */
    ldr     x0, =_boot_page_tables
    /* Entry 0: 0x00000000 - 0x3FFFFFFF → Device memory (UART, GIC) */
    /* 1 GiB block descriptor: AF=1, SH=ISH, AP=RW, AttrIdx=0 (Device) */
    ldr     x1, =(0x00000000 | (1 << 10) | (3 << 8) | (0 << 2) | 0x1)
    str     x1, [x0, #0]

    /* Entry 1: 0x40000000 - 0x7FFFFFFF → Normal memory (RAM) */
    /* 1 GiB block descriptor: AF=1, SH=ISH, AP=RW, AttrIdx=2 (Normal WB) */
    ldr     x1, =(0x40000000 | (1 << 10) | (3 << 8) | (2 << 2) | 0x1)
    str     x1, [x0, #8]

    /* Set TTBR0_EL1 to identity-map PGD */
    ldr     x0, =_boot_page_tables
    msr     ttbr0_el1, x0

    /* For TTBR1 (kernel higher-half), build a separate PGD */
    /* PGD is at _boot_page_tables + 4096 */
    ldr     x4, =_boot_page_tables
    add     x4, x4, #4096

    /* Entry 2 of PGD1: maps 0xFFFF_0000_8000_0000 → phys 0x4000_0000
     * Index in PGD for VA 0xFFFF000080000000:
     *   VA bits [47:39] = (0x80000000 >> 39) & 0x1FF = 0x1 (index 1 at level 0)
     *   Wait — with T1SZ=16, the top bits are sign-extended.
     *   VA = 0xFFFF_0000_8000_0000
     *   Significant bits = VA[47:0] = 0x0000_8000_0000
     *   PGD index = VA[47:39] = (0x0000_8000_0000 >> 39) & 0x1FF = 0x1
     *
     * So PGD[1] → points to a PUD (Level 1) table
     */
    /* PUD at _boot_page_tables + 8192 */
    ldr     x5, =_boot_page_tables
    add     x5, x5, #8192
    /* PGD[1] = table descriptor pointing to PUD */
    orr     x6, x5, #0x3       /* Table descriptor: valid + table */
    str     x6, [x4, #8]       /* PGD entry 1 */

    /* PUD[0]: maps VA 0xFFFF_0000_8000_0000 → PA 0x4000_0000 (1 GiB block) */
    ldr     x1, =(0x40000000 | (1 << 10) | (3 << 8) | (2 << 2) | 0x1)
    str     x1, [x5, #0]

    /* PUD[1]: identity for MMIO access from kernel
     * maps VA 0xFFFF_0000_C000_0000 → PA 0x0000_0000 (Device, for UART/GIC) */
    /* Actually, let's also map 0x00000000 range for MMIO */
    ldr     x1, =(0x00000000 | (1 << 10) | (3 << 8) | (0 << 2) | 0x1)
    str     x1, [x5, #8]

    /* Set TTBR1_EL1 to kernel PGD */
    msr     ttbr1_el1, x4

    /* Ensure page table writes are visible */
    dsb     ish
    isb

    /* Enable MMU */
    mrs     x0, sctlr_el1
    orr     x0, x0, #(1 << 0)  /* M: MMU enable */
    orr     x0, x0, #(1 << 2)  /* C: Data cache enable */
    orr     x0, x0, #(1 << 12) /* I: Instruction cache enable */
    msr     sctlr_el1, x0
    isb

    /* Set up boot stack (identity-mapped address for now) */
    ldr     x0, =_boot_stack_top
    mov     sp, x0

    /* Zero BSS */
    ldr     x0, =_bss_start
    ldr     x1, =_bss_end
.zero_bss:
    cmp     x0, x1
    b.ge    .bss_done
    stp     xzr, xzr, [x0], #16
    b       .zero_bss
.bss_done:

    /* Jump to kernel_main with DTB address in x0 */
    mov     x0, x19
    bl      kernel_main

    /* Should never return */
.hang:
    wfi
    b       .hang

.size _boot_start, . - _boot_start

/* ====================================================================
 * Boot page tables — 16 KiB aligned area in BSS
 * Layout:
 *   +0x0000: PGD for TTBR0 (identity map)
 *   +0x1000: PGD for TTBR1 (kernel higher-half)
 *   +0x2000: PUD for kernel mapping
 *   +0x3000: (reserved)
 * ==================================================================== */
.section .bss
.balign 4096
.global _boot_page_tables
_boot_page_tables:
    .space 16384
