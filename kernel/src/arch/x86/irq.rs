/// Dynamic IRQ handler registration.
/// Uses AtomicPtr for lock-free access from interrupt context.

use core::sync::atomic::{AtomicPtr, Ordering};

/// IRQ handler function type. Takes the IRQ number as parameter.
pub type IrqHandler = fn(irq: u8);

const MAX_IRQS: usize = 32;

/// One AtomicPtr per IRQ line. Null means no handler registered.
/// IRQ 0-15: legacy PIC / I/O APIC (INT 32-47)
/// IRQ 16+:  LAPIC vectors (INT 48+): 16=LAPIC timer, etc.
static IRQ_HANDLERS: [AtomicPtr<()>; MAX_IRQS] = {
    const NULL: AtomicPtr<()> = AtomicPtr::new(core::ptr::null_mut());
    [NULL; MAX_IRQS]
};

/// Register an IRQ handler. Replaces any previous handler for that IRQ.
pub fn register_irq(irq: u8, handler: IrqHandler) {
    if (irq as usize) < MAX_IRQS {
        IRQ_HANDLERS[irq as usize].store(handler as *mut (), Ordering::SeqCst);
    }
}

/// Unregister an IRQ handler.
pub fn unregister_irq(irq: u8) {
    if (irq as usize) < MAX_IRQS {
        IRQ_HANDLERS[irq as usize].store(core::ptr::null_mut(), Ordering::SeqCst);
    }
}

/// Dispatch an IRQ to its registered handler.
/// Returns true if a handler was found and called.
pub fn dispatch_irq(irq: u8) -> bool {
    if (irq as usize) >= MAX_IRQS {
        return false;
    }
    let handler = IRQ_HANDLERS[irq as usize].load(Ordering::SeqCst);
    if handler.is_null() {
        false
    } else {
        let func: IrqHandler = unsafe { core::mem::transmute(handler) };
        func(irq);
        true
    }
}
