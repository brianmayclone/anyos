/*
 * context_switch.S — ARM64 cooperative/preemptive context switch.
 *
 * void context_switch(CpuContext *old, const CpuContext *new);
 *
 * CpuContext layout (must match Rust struct in arch::arm64::context):
 *   Offset  Field
 *   0       x[0..31]   (31 × 8 = 248 bytes)
 *   248     sp
 *   256     pc         (ELR_EL1)
 *   264     pstate     (SPSR_EL1)
 *   272     ttbr0      (TTBR0_EL1)
 *   280     tpidr      (TPIDR_EL0)
 *   288     save_complete
 *   296     canary
 *   304     checksum
 */

.section .text
.global context_switch
.type context_switch, %function

context_switch:
    /* x0 = old CpuContext*, x1 = new CpuContext* */

    /* ---- Save current context to old ---- */

    /* Save callee-saved registers x19-x30 */
    stp     x19, x20, [x0, #(19 * 8)]
    stp     x21, x22, [x0, #(21 * 8)]
    stp     x23, x24, [x0, #(23 * 8)]
    stp     x25, x26, [x0, #(25 * 8)]
    stp     x27, x28, [x0, #(27 * 8)]
    stp     x29, x30, [x0, #(29 * 8)]

    /* Save SP */
    mov     x9, sp
    str     x9, [x0, #248]

    /* Save return address as PC (LR = x30, already saved above) */
    /* For cooperative switch, PC = LR (return address) */
    str     x30, [x0, #256]

    /* Save TTBR0_EL1 */
    mrs     x9, ttbr0_el1
    str     x9, [x0, #272]

    /* Save TPIDR_EL0 */
    mrs     x9, tpidr_el0
    str     x9, [x0, #280]

    /* Set save_complete = 1 */
    mov     x9, #1
    str     x9, [x0, #288]

    /* Write canary */
    ldr     x9, =0xCAFEBABEDEADBEEF
    str     x9, [x0, #296]

    /* Compute checksum (XOR of first 36 u64 fields) */
    mov     x9, xzr             /* accumulator */
    mov     x10, x0             /* pointer */
    mov     x11, #36            /* field count */
.save_checksum:
    ldr     x12, [x10], #8
    eor     x9, x9, x12
    sub     x11, x11, #1
    cbnz    x11, .save_checksum
    str     x9, [x0, #304]      /* store checksum */


    /* ---- Restore new context ---- */

    /* Switch TTBR0_EL1 if different */
    ldr     x9, [x1, #272]
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
    ldr     x9, [x1, #280]
    msr     tpidr_el0, x9

    /* Restore SP */
    ldr     x9, [x1, #248]
    mov     sp, x9

    /* Restore callee-saved registers */
    ldp     x19, x20, [x1, #(19 * 8)]
    ldp     x21, x22, [x1, #(21 * 8)]
    ldp     x23, x24, [x1, #(23 * 8)]
    ldp     x25, x26, [x1, #(25 * 8)]
    ldp     x27, x28, [x1, #(27 * 8)]
    ldp     x29, x30, [x1, #(29 * 8)]

    /* Return to saved PC (which is in x30/LR for cooperative switch) */
    ret

.size context_switch, . - context_switch
