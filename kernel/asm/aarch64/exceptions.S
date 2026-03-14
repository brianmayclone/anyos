/*
 * exceptions.S — ARM64 exception vector table (VBAR_EL1).
 *
 * The vector table has 16 entries, each 128 bytes (32 instructions).
 * Layout:
 *   - Entries 0-3:  Current EL with SP_EL0 (not used in kernel)
 *   - Entries 4-7:  Current EL with SP_ELx (kernel exceptions)
 *   - Entries 8-11: Lower EL using AArch64 (user space exceptions)
 *   - Entries 12-15: Lower EL using AArch32 (not supported)
 */

.section .text
.balign 2048
.global _exception_vector_table

_exception_vector_table:

/* ---- Current EL with SP_EL0 (unused) ---- */
/* 0x000: Synchronous */
.balign 128
    b       _unhandled_exception

/* 0x080: IRQ */
.balign 128
    b       _unhandled_exception

/* 0x100: FIQ */
.balign 128
    b       _unhandled_exception

/* 0x180: SError */
.balign 128
    b       _unhandled_exception

/* ---- Current EL with SP_ELx (kernel mode) ---- */
/* 0x200: Synchronous (kernel fault) */
.balign 128
    b       _kernel_sync_handler

/* 0x280: IRQ (kernel IRQ) */
.balign 128
    b       _kernel_irq_handler

/* 0x300: FIQ */
.balign 128
    b       _unhandled_exception

/* 0x380: SError */
.balign 128
    b       _unhandled_exception

/* ---- Lower EL using AArch64 (user space) ---- */
/* 0x400: Synchronous (syscall, page fault, etc.) */
.balign 128
    b       _user_sync_handler

/* 0x480: IRQ (user interrupted by IRQ) */
.balign 128
    b       _user_irq_handler

/* 0x500: FIQ */
.balign 128
    b       _unhandled_exception

/* 0x580: SError */
.balign 128
    b       _unhandled_exception

/* ---- Lower EL using AArch32 (not supported) ---- */
/* 0x600-0x780 */
.balign 128
    b       _unhandled_exception
.balign 128
    b       _unhandled_exception
.balign 128
    b       _unhandled_exception
.balign 128
    b       _unhandled_exception


/* ====================================================================
 * Exception handlers
 * ==================================================================== */

/*
 * Save all general-purpose registers to the stack.
 * After this macro, SP points to the saved register frame.
 */
.macro SAVE_REGS
    sub     sp, sp, #(34 * 8)   /* 34 registers: x0-x30, sp_el0, elr, spsr */
    stp     x0,  x1,  [sp, #(0  * 8)]
    stp     x2,  x3,  [sp, #(2  * 8)]
    stp     x4,  x5,  [sp, #(4  * 8)]
    stp     x6,  x7,  [sp, #(6  * 8)]
    stp     x8,  x9,  [sp, #(8  * 8)]
    stp     x10, x11, [sp, #(10 * 8)]
    stp     x12, x13, [sp, #(12 * 8)]
    stp     x14, x15, [sp, #(14 * 8)]
    stp     x16, x17, [sp, #(16 * 8)]
    stp     x18, x19, [sp, #(18 * 8)]
    stp     x20, x21, [sp, #(20 * 8)]
    stp     x22, x23, [sp, #(22 * 8)]
    stp     x24, x25, [sp, #(24 * 8)]
    stp     x26, x27, [sp, #(26 * 8)]
    stp     x28, x29, [sp, #(28 * 8)]
    /* Save x30 (LR), SP_EL0, ELR_EL1, SPSR_EL1 */
    mrs     x10, sp_el0
    mrs     x11, elr_el1
    mrs     x12, spsr_el1
    stp     x30, x10, [sp, #(30 * 8)]
    stp     x11, x12, [sp, #(32 * 8)]
.endm

/*
 * Restore all general-purpose registers from the stack.
 */
.macro RESTORE_REGS
    /* Restore ELR_EL1, SPSR_EL1, SP_EL0, x30 */
    ldp     x11, x12, [sp, #(32 * 8)]
    ldp     x30, x10, [sp, #(30 * 8)]
    msr     elr_el1, x11
    msr     spsr_el1, x12
    msr     sp_el0, x10
    /* Restore x0-x29 */
    ldp     x28, x29, [sp, #(28 * 8)]
    ldp     x26, x27, [sp, #(26 * 8)]
    ldp     x24, x25, [sp, #(24 * 8)]
    ldp     x22, x23, [sp, #(22 * 8)]
    ldp     x20, x21, [sp, #(20 * 8)]
    ldp     x18, x19, [sp, #(18 * 8)]
    ldp     x16, x17, [sp, #(16 * 8)]
    ldp     x14, x15, [sp, #(14 * 8)]
    ldp     x12, x13, [sp, #(12 * 8)]
    ldp     x10, x11, [sp, #(10 * 8)]
    ldp     x8,  x9,  [sp, #(8  * 8)]
    ldp     x6,  x7,  [sp, #(6  * 8)]
    ldp     x4,  x5,  [sp, #(4  * 8)]
    ldp     x2,  x3,  [sp, #(2  * 8)]
    ldp     x0,  x1,  [sp, #(0  * 8)]
    add     sp, sp, #(34 * 8)
.endm


/* --- Kernel synchronous exception --- */
_kernel_sync_handler:
    SAVE_REGS
    mrs     x0, esr_el1         /* ESR */
    mrs     x1, far_el1         /* FAR */
    mrs     x2, elr_el1         /* ELR */
    bl      arm64_sync_handler
    RESTORE_REGS
    eret

/* --- Kernel IRQ --- */
_kernel_irq_handler:
    SAVE_REGS
    bl      arm64_irq_handler
    RESTORE_REGS
    eret

/* --- User synchronous exception (syscall or fault) --- */
_user_sync_handler:
    SAVE_REGS
    mrs     x0, esr_el1
    /* Check if this is an SVC (syscall) */
    lsr     x1, x0, #26        /* Extract EC (Exception Class) */
    cmp     x1, #0x15           /* EC_SVC_AARCH64 */
    b.ne    .user_sync_not_svc

    /* Syscall: x8=nr, x0-x5=args */
    /* Reload args from saved frame (x0 was clobbered by ESR read) */
    ldp     x1, x2, [sp, #(0 * 8)]   /* saved x0, x1 → arg0, arg1 */
    ldp     x3, x4, [sp, #(2 * 8)]   /* saved x2, x3 → arg2, arg3 */
    ldp     x5, x6, [sp, #(4 * 8)]   /* saved x4, x5 → arg4, arg5 */
    ldr     x0, [sp, #(8 * 8)]       /* saved x8 → syscall nr */
    /* x0=nr, x1=arg0, x2=arg1, x3=arg2, x4=arg3, x5=arg4, x6=arg5 */
    bl      arm64_syscall_dispatch
    /* Store return value in saved x0 position */
    str     x0, [sp, #(0 * 8)]
    RESTORE_REGS
    eret

.user_sync_not_svc:
    /* Non-syscall synchronous exception from user */
    mrs     x0, esr_el1
    mrs     x1, far_el1
    mrs     x2, elr_el1
    bl      arm64_sync_handler
    RESTORE_REGS
    eret

/* --- User IRQ --- */
_user_irq_handler:
    SAVE_REGS
    bl      arm64_irq_handler
    RESTORE_REGS
    eret

/* --- Unhandled exception --- */
_unhandled_exception:
    SAVE_REGS
    mrs     x0, esr_el1
    mrs     x1, far_el1
    mrs     x2, elr_el1
    bl      arm64_sync_handler
    RESTORE_REGS
    eret
