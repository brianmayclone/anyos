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
 *   2. Sets up initial page tables (identity + higher-half)
 *   3. Enables the MMU
 *   4. Jumps to virtual (higher-half) address
 *   5. Sets up the boot stack, zeros BSS
 *   6. Jumps to kernel_main(dtb_addr)
 *
 * Page table structure (4 KiB granule):
 *   Level 0 (PGD): ONLY table descriptors allowed.
 *   Level 1 (PUD): 1 GiB block descriptors.
 *
 * MAIR_EL1 attribute indices:
 *   AttrIdx 0 = Device-nGnRnE  (0x00)
 *   AttrIdx 1 = Normal WB RAWA (0xFF)
 *
 * Linker places this section at VMA = LMA = 0x40400000 (physical)
 * so the ELF entry point is a physical address usable with MMU off.
 */

.section .text.boot, "ax"
.global _boot_start
.type _boot_start, %function

_boot_start:
    /* Save DTB address from x0 (callee-saved register) */
    mov     x19, x0

    /* Check current exception level */
    mrs     x1, CurrentEL
    and     x1, x1, #0xC       /* Extract EL bits [3:2] */
    cmp     x1, #0x8           /* EL2? */
    b.ne    .at_el1

    /* --- EL2 → EL1 transition --- */
    mov     x0, #(1 << 31)     /* HCR_EL2.RW = 1 (AArch64 at EL1) */
    msr     hcr_el2, x0

    mov     x0, #0x3C5         /* SPSR: EL1h, DAIF masked */
    msr     spsr_el2, x0

    adr     x0, .at_el1
    msr     elr_el2, x0

    eret

.at_el1:
    /* --- Now running at EL1, MMU off --- */

    /* Disable MMU and caches while setting up page tables */
    mrs     x0, sctlr_el1
    bic     x0, x0, #(1 << 0)  /* M: MMU disable */
    bic     x0, x0, #(1 << 2)  /* C: Data cache disable */
    bic     x0, x0, #(1 << 12) /* I: Instruction cache disable */
    msr     sctlr_el1, x0
    isb

    /* ---- MAIR_EL1 ----
     * Attr[0] = 0x00 : Device-nGnRnE (MMIO: UART, GIC)
     * Attr[1] = 0xFF : Normal, Inner/Outer Write-Back, Read-Allocate, Write-Allocate
     */
    mov     x0, #0xFF00
    msr     mair_el1, x0

    /* ---- TCR_EL1 ----
     * T0SZ  = 16  (48-bit VA for TTBR0)   bits[5:0]   = 0x10
     * T1SZ  = 16  (48-bit VA for TTBR1)   bits[21:16] = 0x10
     * TG0   = 0b00  (4 KiB for TTBR0)     bits[15:14] = 0b00
     * TG1   = 0b10  (4 KiB for TTBR1)     bits[31:30] = 0b10
     * SH0   = 0b11  (Inner Shareable)      bits[13:12] = 0b11
     * SH1   = 0b11  (Inner Shareable)      bits[29:28] = 0b11
     * ORGN0 = 0b01  (WB RA WA)            bits[11:10] = 0b01
     * IRGN0 = 0b01  (WB RA WA)            bits[9:8]   = 0b01
     * ORGN1 = 0b01                         bits[27:26] = 0b01
     * IRGN1 = 0b01                         bits[25:24] = 0b01
     * IPS   = 0b101  (48-bit PA)           bits[34:32] = 0b101
     */
    ldr     x0, =0x00000005B5103510
    msr     tcr_el1, x0
    isb

    /* ---- Get PHYSICAL address of boot page tables ----
     * adrp+add gives PC-relative address (physical, since MMU is off).
     * _boot_page_tables is in .boot_pt section at VMA=LMA (physical).
     */
    adrp    x8, _boot_page_tables
    add     x8, x8, :lo12:_boot_page_tables

    /* Clear page table area (16 KiB = 4 × 4 KiB pages) */
    mov     x0, x8
    mov     x1, #16384
.clear_pt:
    stp     xzr, xzr, [x0], #16
    sub     x1, x1, #16
    cbnz    x1, .clear_pt

    /* ================================================================
     * TTBR0 identity map: VA = PA (for boot code execution)
     *
     * Table layout:
     *   x8+0x0000 = L0 PGD for TTBR0
     *   x8+0x1000 = L1 PUD for TTBR0
     *
     * L0 PGD[0] → table → L1 PUD
     * L1 PUD[0] → 1 GiB block: PA 0x00000000 (Device, MMIO)
     * L1 PUD[1] → 1 GiB block: PA 0x40000000 (Normal, RAM)
     * ================================================================ */
    mov     x9, x8                  /* x9 = L0 PGD for TTBR0 */
    add     x10, x8, #4096         /* x10 = L1 PUD for TTBR0 */

    /* L0 PGD[0] = table descriptor → PUD */
    orr     x0, x10, #0x3          /* valid + table */
    str     x0, [x9, #0]

    /* L1 PUD[0]: PA 0x00000000, Device-nGnRnE (AttrIdx=0)
     * Descriptor: output_addr=0, AF=1, SH=ISH, AttrIdx=0, valid block (0x1) */
    mov     x0, #0
    orr     x0, x0, #(1 << 10)     /* AF (Access Flag) */
    orr     x0, x0, #(3 << 8)      /* SH = Inner Shareable */
    orr     x0, x0, #0x1           /* Valid block descriptor */
    str     x0, [x10, #0]          /* PUD[0] */

    /* L1 PUD[1]: PA 0x40000000, Normal WB (AttrIdx=1)
     * Descriptor: output_addr=0x40000000, AF=1, SH=ISH, AttrIdx=1 */
    ldr     x0, =0x40000000
    orr     x0, x0, #(1 << 10)     /* AF */
    orr     x0, x0, #(3 << 8)      /* SH = Inner Shareable */
    orr     x0, x0, #(1 << 2)      /* AttrIdx = 1 */
    orr     x0, x0, #0x1           /* Valid block descriptor */
    str     x0, [x10, #8]          /* PUD[1] */

    /* Set TTBR0_EL1 */
    msr     ttbr0_el1, x9

    /* ================================================================
     * TTBR1 kernel map: VA 0xFFFF_0000_8xxx_xxxx → PA 0x4xxx_xxxx
     *
     * Table layout:
     *   x8+0x2000 = L0 PGD for TTBR1
     *   x8+0x3000 = L1 PUD for TTBR1
     *
     * VA 0xFFFF_0000_8000_0000 with T1SZ=16 (48-bit VA):
     *   VA[47:0] = 0x0000_8000_0000
     *   L0 index = VA[47:39] = 0
     *   L1 index = VA[38:30] = 2
     *
     * VA 0xFFFF_0000_C000_0000 (MMIO):
     *   L0 index = 0, L1 index = 3
     *
     * L0 PGD[0] → table → L1 PUD
     * L1 PUD[2] → 1 GiB block: PA 0x40000000 (Normal, kernel + RAM)
     * L1 PUD[3] → 1 GiB block: PA 0x00000000 (Device, kernel MMIO)
     * ================================================================ */
    add     x11, x8, #(4096 * 2)   /* x11 = L0 PGD for TTBR1 */
    add     x12, x8, #(4096 * 3)   /* x12 = L1 PUD for TTBR1 */

    /* L0 PGD[0] = table descriptor → PUD */
    orr     x0, x12, #0x3
    str     x0, [x11, #0]          /* PGD entry index 0 */

    /* L1 PUD[2]: PA 0x40000000, Normal WB (AttrIdx=1) — kernel RAM */
    ldr     x0, =0x40000000
    orr     x0, x0, #(1 << 10)
    orr     x0, x0, #(3 << 8)
    orr     x0, x0, #(1 << 2)      /* AttrIdx = 1 */
    orr     x0, x0, #0x1
    str     x0, [x12, #16]         /* PUD[2] = offset 16 */

    /* L1 PUD[3]: PA 0x00000000, Device-nGnRnE (AttrIdx=0) — MMIO */
    mov     x0, #0
    orr     x0, x0, #(1 << 10)
    orr     x0, x0, #(3 << 8)
    orr     x0, x0, #0x1
    str     x0, [x12, #24]         /* PUD[3] = offset 24 */

    /* Set TTBR1_EL1 */
    msr     ttbr1_el1, x11

    /* Ensure page table writes are visible */
    dsb     ish
    isb

    /* Invalidate all TLB entries */
    tlbi    vmalle1
    dsb     ish
    isb

    /* ---- Enable MMU ---- */
    mrs     x0, sctlr_el1
    orr     x0, x0, #(1 << 0)      /* M: MMU enable */
    orr     x0, x0, #(1 << 2)      /* C: Data cache enable */
    orr     x0, x0, #(1 << 12)     /* I: Instruction cache enable */
    msr     sctlr_el1, x0
    isb

    /* ---- Jump to higher-half virtual address ----
     * We're executing at physical address (identity-mapped via TTBR0).
     * Load the virtual address of .in_virtual (in .text section,
     * VMA = higher-half) and branch there.
     */
    ldr     x0, =.in_virtual
    br      x0

.size _boot_start, . - _boot_start

/* ================================================================
 * Post-MMU code — in .text section at kernel virtual address.
 * ================================================================ */
.section .text
.global _in_virtual
_in_virtual:
.in_virtual:

    /* Set up boot stack (virtual address) */
    ldr     x0, =_boot_stack_top
    mov     sp, x0

    /* Zero BSS (virtual addresses, mapped via TTBR1) */
    ldr     x0, =_bss_start
    ldr     x1, =_bss_end
.zero_bss:
    cmp     x0, x1
    b.ge    .bss_done
    stp     xzr, xzr, [x0], #16
    b       .zero_bss
.bss_done:

    /* Enable FP/NEON — CPACR_EL1.FPEN = 0b11 (bits [21:20])
     * Without this, any FP register use (even callee-save spills) traps. */
    mrs     x0, cpacr_el1
    orr     x0, x0, #(3 << 20)
    msr     cpacr_el1, x0
    isb

    /* Jump to kernel_main with DTB physical address in x0 */
    mov     x0, x19
    bl      kernel_main

    /* Should never return */
.hang:
    wfi
    b       .hang
