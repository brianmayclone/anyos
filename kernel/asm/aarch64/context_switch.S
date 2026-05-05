/*
 * context_switch.S — ARM64 cooperative/preemptive context switch.
 *
 * void context_switch(CpuContext *old, const CpuContext *new);
 *
 * CpuContext layout (must match Rust struct in arch::arm64::context):
 *   Offset  Field
 *   0       x[0..31]   (31 × 8 = 248 bytes)
 *   248     d[0..7]    (D8-D15 low 64-bit lanes)
 *   312     sp
 *   320     pc         (ELR_EL1)
 *   328     pstate     (SPSR_EL1)
 *   336     ttbr0      (TTBR0_EL1)
 *   344     tpidr      (TPIDR_EL0)
 *   352     save_complete
 *   360     canary
 *   368     checksum
 */

.section .text
.global context_switch
.type context_switch, %function

context_switch:
    /* x0 = old CpuContext*, x1 = new CpuContext* */

    /* ---- Save current context to old ---- */

    /* Save callee-saved registers x19-x30 plus x18 (platform register). */
    str     x18,      [x0, #(18 * 8)]
    stp     x19, x20, [x0, #(19 * 8)]
    stp     x21, x22, [x0, #(21 * 8)]
    stp     x23, x24, [x0, #(23 * 8)]
    stp     x25, x26, [x0, #(25 * 8)]
    stp     x27, x28, [x0, #(27 * 8)]
    stp     x29, x30, [x0, #(29 * 8)]
    stp     d8,  d9,  [x0, #248]
    stp     d10, d11, [x0, #264]
    stp     d12, d13, [x0, #280]
    stp     d14, d15, [x0, #296]

    /* Save SP */
    mov     x9, sp
    str     x9, [x0, #312]

    /* Save return address as PC (LR = x30, already saved above) */
    /* For cooperative switch, PC = LR (return address) */
    str     x30, [x0, #320]

    /* Save TTBR0_EL1 */
    mrs     x9, ttbr0_el1
    str     x9, [x0, #336]

    /* Save TPIDR_EL0 */
    mrs     x9, tpidr_el0
    str     x9, [x0, #344]

    /* Write canary */
    ldr     x9, =0xCAFEBABEDEADBEEF
    str     x9, [x0, #360]

    /* Compute checksum (XOR of first 44 u64 fields) */
    mov     x9, xzr             /* accumulator */
    mov     x10, x0             /* pointer */
    mov     x11, #44            /* field count */
.save_checksum:
    ldr     x12, [x10], #8
    eor     x9, x9, x12
    sub     x11, x11, #1
    cbnz    x11, .save_checksum
    str     x9, [x0, #368]      /* store checksum */

    /* Ensure context fields/canary/checksum are globally visible before
     * save_complete becomes observable by another CPU. */
    dmb     ishst

    /* Set save_complete = 1 only after the full save is visible. */
    mov     x9, #1
    str     x9, [x0, #352]

    /* ---- Validate and restore new context ---- */

    /* Verify save_complete. */
    ldr     x9, [x1, #352]
    cmp     x9, #1
    b.ne    .bad_context

    /* Verify canary. */
    ldr     x9, =0xCAFEBABEDEADBEEF
    ldr     x10, [x1, #360]
    cmp     x10, x9
    b.ne    .bad_context

    /* Verify checksum (XOR of first 44 u64 fields). */
    mov     x9, xzr
    mov     x10, x1
    mov     x11, #44
.load_checksum:
    ldr     x12, [x10], #8
    eor     x9, x9, x12
    sub     x11, x11, #1
    cbnz    x11, .load_checksum
    ldr     x10, [x1, #368]
    cmp     x9, x10
    b.ne    .bad_context

    /* Range checks: scheduler continuations must resume in kernel text on
     * kernel stacks. User execution resumes via a kernel trampoline. */
    ldr     x9, [x1, #312]      /* sp */
    ldr     x10, =0xFFFF000080000000
    cmp     x9, x10
    b.lo    .bad_context

    ldr     x9, [x1, #320]      /* pc */
    ldr     x10, =0xFFFF000080400000
    cmp     x9, x10
    b.lo    .bad_context
    ldr     x10, =0xFFFF0000C0000000
    cmp     x9, x10
    b.hs    .bad_context

    /* Switch TTBR0_EL1 if different */
    ldr     x9, [x1, #336]
    mrs     x10, ttbr0_el1
    cmp     x9, x10
    b.eq    .skip_ttbr
    msr     ttbr0_el1, x9
    isb
    tlbi    vmalle1is
    dsb     ish
    isb
.skip_ttbr:

    /* Restore TPIDR_EL0 */
    ldr     x9, [x1, #344]
    msr     tpidr_el0, x9

    /* Restore SP */
    ldr     x9, [x1, #312]
    mov     sp, x9

    /* Load the target PC from the saved context.
     * Use a dedicated register so the UART diagnostics below don't clobber it. */
    ldr     x11, [x1, #320]

    /* Restore callee-saved registers plus x18. */
    ldr     x18,      [x1, #(18 * 8)]
    ldp     x19, x20, [x1, #(19 * 8)]
    ldp     x21, x22, [x1, #(21 * 8)]
    ldp     x23, x24, [x1, #(23 * 8)]
    ldp     x25, x26, [x1, #(25 * 8)]
    ldp     x27, x28, [x1, #(27 * 8)]
    ldp     x29, x30, [x1, #(29 * 8)]
    ldp     d8,  d9,  [x1, #248]
    ldp     d10, d11, [x1, #264]
    ldp     d12, d13, [x1, #280]
    ldp     d14, d15, [x1, #296]

    /* Branch to the saved PC. */
    br      x11

.bad_context:
    /* The Rust scheduler performs the same checks before calling this routine
     * and should normally catch corruption first. This hard stop is the final
     * guard against branching to a bad return address. */
    wfe
    b       .bad_context

.size context_switch, . - context_switch
