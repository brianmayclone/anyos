/// Dynamic IRQ handler registration.
/// Uses AtomicPtr for lock-free access from interrupt context.
/// Supports shared IRQs: up to 2 handlers per IRQ line (primary + chained).
use core::sync::atomic::{AtomicPtr, Ordering};

/// IRQ handler function type. Takes the IRQ number as parameter.
pub type IrqHandler = fn(irq: u8);

const MAX_IRQS: usize = 32;

/// Primary handler per IRQ line. Null means no handler registered.
/// IRQ 0-15: legacy PIC / I/O APIC (INT 32-47)
/// IRQ 16+:  LAPIC vectors (INT 48+): 16=LAPIC timer, etc.
static IRQ_HANDLERS: [AtomicPtr<()>; MAX_IRQS] = {
    const NULL: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
    [NULL; MAX_IRQS]
};

/// Secondary (chained) handler per IRQ line, for shared PCI interrupts.
static IRQ_CHAIN: [AtomicPtr<()>; MAX_IRQS] = {
    const NULL: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
    [NULL; MAX_IRQS]
};

/// Register an IRQ handler. If the primary slot is empty, claims it; otherwise
/// falls through to the chained slot. This auto-chaining lets multiple PCI
/// devices share the same legacy IRQ line without silently overwriting each
/// other's handlers (which would deliver interrupts only to the last
/// registrant). Returns true if a slot was claimed, false if both slots were
/// already in use.
pub fn register_irq(irq: u8, handler: IrqHandler) -> bool {
    if (irq as usize) >= MAX_IRQS {
        return false;
    }
    let primary = &IRQ_HANDLERS[irq as usize];
    if primary
        .compare_exchange(
            core::ptr::null_mut(),
            handler as *mut (),
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_ok()
    {
        return true;
    }
    let chain = &IRQ_CHAIN[irq as usize];
    if chain
        .compare_exchange(
            core::ptr::null_mut(),
            handler as *mut (),
            Ordering::SeqCst,
            Ordering::SeqCst,
        )
        .is_ok()
    {
        return true;
    }
    crate::serial_println!(
        "[irq] WARNING: IRQ {} already has primary + chained handlers; cannot register more",
        irq
    );
    false
}

/// Register a chained IRQ handler for shared IRQ lines.
/// Both the primary and chained handlers are called on each interrupt.
pub fn register_irq_chain(irq: u8, handler: IrqHandler) {
    if (irq as usize) < MAX_IRQS {
        IRQ_CHAIN[irq as usize].store(handler as *mut (), Ordering::SeqCst);
    }
}

/// Unregister an IRQ handler.
pub fn unregister_irq(irq: u8) {
    if (irq as usize) < MAX_IRQS {
        IRQ_HANDLERS[irq as usize].store(core::ptr::null_mut(), Ordering::SeqCst);
    }
}

/// Dispatch an IRQ to its registered handler(s).
/// Returns true if at least one handler was found and called.
pub fn dispatch_irq(irq: u8) -> bool {
    if (irq as usize) >= MAX_IRQS {
        return false;
    }

    let mut handled = false;

    let primary = IRQ_HANDLERS[irq as usize].load(Ordering::SeqCst);
    if !primary.is_null() {
        let func: IrqHandler = unsafe { core::mem::transmute(primary) };
        func(irq);
        handled = true;
    }

    let chained = IRQ_CHAIN[irq as usize].load(Ordering::SeqCst);
    if !chained.is_null() {
        let func: IrqHandler = unsafe { core::mem::transmute(chained) };
        func(irq);
        handled = true;
    }

    handled
}
