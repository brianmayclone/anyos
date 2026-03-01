/*
 * ap_startup.S — ARM64 Application Processor entry point.
 *
 * Called by PSCI CPU_ON with:
 *   - x0 = context_id (CPU number, passed as x3 to PSCI)
 *   - MMU may or may not be enabled (depends on firmware)
 *
 * This code:
 *   1. Sets up the exception vector table
 *   2. Sets up a temporary stack
 *   3. Calls the Rust AP init function
 */

.section .text
.global _ap_entry
.type _ap_entry, %function

_ap_entry:
    /* x0 = cpu_id (context_id from PSCI CPU_ON) */
    mov     x19, x0             /* Save CPU ID */

    /* Install exception vector table */
    ldr     x1, =_exception_vector_table
    msr     vbar_el1, x1
    isb

    /* Set up per-CPU temporary stack.
     * Each AP gets a 16 KiB stack.
     * Stack base = _ap_stacks + (cpu_id + 1) * 16384
     * (cpu_id 0 is BSP, APs start from 1)
     */
    ldr     x1, =_ap_stacks
    add     x2, x19, #1
    lsl     x2, x2, #14        /* × 16384 */
    add     sp, x1, x2

    /* Enable FP/NEON (may be trapped by default) */
    mrs     x1, cpacr_el1
    orr     x1, x1, #(3 << 20) /* FPEN = 0b11 */
    msr     cpacr_el1, x1
    isb

    /* Call Rust AP init with cpu_id in x0 */
    mov     x0, x19
    bl      arm64_ap_init

    /* Enter idle loop */
.ap_idle:
    wfi
    b       .ap_idle

.size _ap_entry, . - _ap_entry


/* ====================================================================
 * AP stacks — 16 KiB per AP, supports up to 16 CPUs
 * ==================================================================== */
.section .bss
.balign 4096
.global _ap_stacks
_ap_stacks:
    .space 16384 * 16           /* 16 CPUs × 16 KiB */
