//! Serial port driver for debug output.
//!
//! On x86_64: COM1 (0x3F8) at 115200 baud 8N1, with async TX via IRQ 4.
//! On AArch64: PL011 UART (0x09000000) via MMIO.
//!
//! Both share a 32 KiB kernel log ring buffer and SMP-safe output locking.

#[cfg(target_arch = "x86_64")]
use crate::arch::x86::port::{inb, outb};
use core::fmt;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};

/// COM1 I/O base port address (x86 only).
#[cfg(target_arch = "x86_64")]
const COM1: u16 = 0x3F8;

/// Zero-sized type implementing `fmt::Write` for serial output.
pub struct SerialPort;

static mut SERIAL_INITIALIZED: bool = false;
/// When true, `write_byte` uses the async TX buffer + IRQ 4 path (x86 only).
#[cfg(target_arch = "x86_64")]
static ASYNC_TX: AtomicBool = AtomicBool::new(false);

// ── SMP output lock (ensures entire messages are atomic) ────────────────

/// Protects serial output so entire formatted messages from one CPU
/// are not interleaved with output from another CPU.
static OUTPUT_LOCK: AtomicBool = AtomicBool::new(false);
/// CPU that currently holds the output lock (0xFF = nobody).
static OUTPUT_LOCK_CPU: AtomicU8 = AtomicU8::new(0xFF);
/// When true, we're in a panic/fatal exception — skip lock contention,
/// other CPUs should already be halted.
static PANIC_MODE: AtomicBool = AtomicBool::new(false);

/// Acquire the serial output lock (IRQ-safe, reentrant per-CPU).
///
/// Returns `(saved_flags, was_reentrant)`. The caller MUST pass this to
/// [`output_lock_release`] when the message is complete.
///
/// Reentrancy: if the same CPU already holds the lock (e.g. an exception
/// fired inside `serial_println!`), the lock is NOT re-acquired — the
/// exception's output may corrupt the in-progress message, but this is
/// preferable to deadlocking and losing crash diagnostics entirely.
pub fn output_lock_acquire() -> (u64, bool) {
    let flags = crate::arch::hal::save_and_disable_interrupts();

    // Check panic mode BEFORE calling cpu_id(), because that reads
    // LAPIC MMIO which may not be mapped in the current CR3 during a crash.
    if PANIC_MODE.load(Ordering::Relaxed) {
        // Panic mode: force-take (other CPUs should be halted)
        OUTPUT_LOCK.store(true, Ordering::Relaxed);
        OUTPUT_LOCK_CPU.store(0, Ordering::Relaxed);
        return (flags, false);
    }

    let cpu = crate::arch::hal::cpu_id() as u8;

    // Reentrant: same CPU already holds the lock (exception during serial output)
    if OUTPUT_LOCK.load(Ordering::Relaxed) && OUTPUT_LOCK_CPU.load(Ordering::Relaxed) == cpu {
        return (flags, true);
    }

    let mut spin_count: u32 = 0;
    while OUTPUT_LOCK
        .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        core::hint::spin_loop();
        spin_count += 1;
        if spin_count == 10_000_000 {
            // Probable deadlock — print via direct UART (bypasses this lock)
            unsafe {
                let msg = b"\r\n!!! SER_LOCK TIMEOUT cpu=";
                for &c in msg { uart_direct_write_byte(c); }
                uart_direct_write_byte(b'0' + cpu);
                let msg2 = b" owner=";
                for &c in msg2 { uart_direct_write_byte(c); }
                let owner = OUTPUT_LOCK_CPU.load(Ordering::Relaxed);
                if owner == 0xFF {
                    let m = b"NONE";
                    for &c in m { uart_direct_write_byte(c); }
                } else {
                    uart_direct_write_byte(b'0' + owner);
                }
                uart_direct_write_byte(b'\r'); uart_direct_write_byte(b'\n');
            }
        }
    }
    OUTPUT_LOCK_CPU.store(cpu, Ordering::Relaxed);
    (flags, false)
}

/// Release the serial output lock and restore interrupt state.
pub fn output_lock_release(saved: (u64, bool)) {
    let (flags, reentrant) = saved;
    if !reentrant {
        OUTPUT_LOCK_CPU.store(0xFF, Ordering::Relaxed);
        OUTPUT_LOCK.store(false, Ordering::Release);
    }
    crate::arch::hal::restore_interrupt_state(flags);
}

/// Check if the serial output lock is currently held (lock-free diagnostic).
pub fn is_output_locked() -> bool {
    OUTPUT_LOCK.load(Ordering::Relaxed)
}

/// Enter panic mode: halt all other CPUs, switch to blocking serial TX,
/// and force-release the output lock so crash diagnostics can be printed.
///
/// Called by the panic handler and fatal exception handlers.
pub fn enter_panic_mode() {
    // Prevent reentrant calls
    if PANIC_MODE.swap(true, Ordering::SeqCst) {
        return; // Already in panic mode (another CPU got here first)
    }

    // Disable interrupts on this CPU
    crate::arch::hal::disable_interrupts();

    // Force-release the output lock (previous holder may be dead)
    OUTPUT_LOCK.store(false, Ordering::Release);
    OUTPUT_LOCK_CPU.store(0xFF, Ordering::Release);

    // Switch to blocking TX — async buffer may be lost during panic
    #[cfg(target_arch = "x86_64")]
    ASYNC_TX.store(false, Ordering::Release);

    // Halt all other CPUs via IPI so they stop outputting
    crate::arch::hal::halt_other_cpus();
}

// ── Direct UART I/O helpers ────────────────────────────────────────────────

/// Write one byte directly to the UART (blocking, bypasses all locks/buffers).
/// Used for emergency output in deadlock detection and panic.
#[inline]
unsafe fn uart_direct_write_byte(byte: u8) {
    #[cfg(target_arch = "x86_64")]
    {
        while inb(COM1 + 5) & 0x20 == 0 {}
        outb(COM1, byte);
    }
    #[cfg(target_arch = "aarch64")]
    {
        crate::arch::arm64::serial::write_byte(byte);
    }
}

// ── Kernel log ring buffer (pre-heap, interrupt-safe) ──────────────────────

/// Size of the kernel log ring buffer in bytes.
const LOG_BUF_SIZE: usize = 32 * 1024; // 32 KiB
static mut LOG_BUF: [u8; LOG_BUF_SIZE] = [0u8; LOG_BUF_SIZE];
static LOG_WRITE_POS: AtomicUsize = AtomicUsize::new(0);
static LOG_TOTAL_WRITTEN: AtomicUsize = AtomicUsize::new(0);

fn log_push_byte(byte: u8) {
    let pos = LOG_WRITE_POS.load(Ordering::Relaxed) & (LOG_BUF_SIZE - 1);
    unsafe { *LOG_BUF.as_mut_ptr().add(pos) = byte; }
    LOG_WRITE_POS.store((pos + 1) & (LOG_BUF_SIZE - 1), Ordering::Relaxed);
    LOG_TOTAL_WRITTEN.fetch_add(1, Ordering::Relaxed);
}

/// Copy kernel log into `dst`. Returns number of bytes written.
pub fn read_log(dst: &mut [u8]) -> usize {
    let total = LOG_TOTAL_WRITTEN.load(Ordering::Relaxed);
    if total == 0 || dst.is_empty() {
        return 0;
    }
    let available = total.min(LOG_BUF_SIZE);
    let write_pos = LOG_WRITE_POS.load(Ordering::Relaxed);
    let start = if total <= LOG_BUF_SIZE { 0 } else { write_pos };
    let copy_len = available.min(dst.len());

    for i in 0..copy_len {
        let idx = (start + i) & (LOG_BUF_SIZE - 1);
        dst[i] = unsafe { *LOG_BUF.as_ptr().add(idx) };
    }
    copy_len
}

// ── Async TX ring buffer (x86 only — COM1 IRQ 4 / THRE) ────────────────────

#[cfg(target_arch = "x86_64")]
const TX_BUF_SIZE: usize = 8 * 1024; // 8 KiB
#[cfg(target_arch = "x86_64")]
static mut TX_BUF: [u8; TX_BUF_SIZE] = [0u8; TX_BUF_SIZE];
/// Producer (write_byte) writes here.
#[cfg(target_arch = "x86_64")]
static TX_HEAD: AtomicUsize = AtomicUsize::new(0);
/// Consumer (ISR) reads from here.
#[cfg(target_arch = "x86_64")]
static TX_TAIL: AtomicUsize = AtomicUsize::new(0);

/// Push one byte into the TX ring buffer.  Returns false if full (byte dropped).
#[cfg(target_arch = "x86_64")]
#[inline]
fn tx_push(byte: u8) -> bool {
    let head = TX_HEAD.load(Ordering::Relaxed) & (TX_BUF_SIZE - 1);
    let next = (head + 1) & (TX_BUF_SIZE - 1);
    if next == TX_TAIL.load(Ordering::Acquire) & (TX_BUF_SIZE - 1) {
        return false; // full — drop
    }
    unsafe { *TX_BUF.as_mut_ptr().add(head) = byte; }
    TX_HEAD.store(next, Ordering::Release);
    true
}

/// Pop one byte from the TX ring buffer.  Returns None if empty.
#[cfg(target_arch = "x86_64")]
#[inline]
fn tx_pop() -> Option<u8> {
    let tail = TX_TAIL.load(Ordering::Relaxed) & (TX_BUF_SIZE - 1);
    if tail == TX_HEAD.load(Ordering::Acquire) & (TX_BUF_SIZE - 1) {
        return None;
    }
    let byte = unsafe { *TX_BUF.as_ptr().add(tail) };
    TX_TAIL.store((tail + 1) & (TX_BUF_SIZE - 1), Ordering::Release);
    Some(byte)
}

/// Atomic guard: only one CPU at a time may call `tx_pop()` (via `try_drain`).
/// Prevents the dual-consumer race between `kick_tx` and the ISR on different CPUs.
#[cfg(target_arch = "x86_64")]
static DRAINING: AtomicBool = AtomicBool::new(false);

/// Drain up to `max` bytes from the TX ring buffer into the UART FIFO.
/// Uses `DRAINING` to ensure only one CPU pops from the buffer at a time.
/// If another CPU is already draining, returns immediately.
#[cfg(target_arch = "x86_64")]
fn try_drain(max: usize) {
    if DRAINING
        .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        .is_err()
    {
        return; // another CPU is draining — skip
    }
    for _ in 0..max {
        if !is_transmit_empty() {
            break;
        }
        match tx_pop() {
            Some(b) => unsafe { outb(COM1, b); },
            None => {
                // Buffer empty — disable THRE interrupt until new data arrives
                unsafe {
                    let ier = inb(COM1 + 1);
                    outb(COM1 + 1, ier & !0x02);
                }
                DRAINING.store(false, Ordering::Release);
                return;
            }
        }
    }
    // More data remains — ensure THRE interrupt is enabled
    unsafe {
        let ier = inb(COM1 + 1);
        if ier & 0x02 == 0 {
            outb(COM1 + 1, ier | 0x02);
        }
    }
    DRAINING.store(false, Ordering::Release);
}

/// Kick-start the TX drain.  Sends up to 16 bytes directly (works even when
/// interrupts are disabled on this CPU), and ensures the THRE interrupt is
/// enabled so the ISR handles the rest.
#[cfg(target_arch = "x86_64")]
#[inline]
fn kick_tx() {
    try_drain(16);
    // Ensure THRE is enabled even if try_drain was skipped (another CPU had the lock)
    unsafe {
        let ier = inb(COM1 + 1);
        if ier & 0x02 == 0 {
            outb(COM1 + 1, ier | 0x02);
        }
    }
}

/// Initialize the serial port.
pub fn init() {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        outb(COM1 + 1, 0x00); // Disable all interrupts
        outb(COM1 + 3, 0x80); // Enable DLAB (set baud rate divisor)
        outb(COM1 + 0, 0x01); // Set divisor to 1 (115200 baud)
        outb(COM1 + 1, 0x00); //   hi byte
        outb(COM1 + 3, 0x03); // 8 bits, no parity, one stop bit (8N1)
        outb(COM1 + 2, 0xC7); // Enable FIFO, clear them, 14-byte threshold
        outb(COM1 + 4, 0x0B); // IRQs enabled, RTS/DSR set (OUT2 needed for ISA IRQ)

        SERIAL_INITIALIZED = true;
    }

    #[cfg(target_arch = "aarch64")]
    unsafe {
        crate::arch::arm64::serial::init();
        SERIAL_INITIALIZED = true;
    }
}

/// Switch from blocking to async TX mode.
/// Call once after the IRQ subsystem is ready (IRQ handler registered, IRQ 4 unmasked).
#[cfg(target_arch = "x86_64")]
pub fn enable_async() {
    // Register COM1 IRQ handler and unmask IRQ 4
    crate::arch::x86::irq::register_irq(4, serial_irq_handler);

    if crate::arch::x86::apic::is_initialized() {
        crate::arch::x86::ioapic::unmask_irq(4);
    } else {
        crate::arch::x86::pic::unmask(4);
    }

    ASYNC_TX.store(true, Ordering::Release);
}

/// No-op on ARM64 (PL011 uses blocking TX for now).
#[cfg(target_arch = "aarch64")]
pub fn enable_async() {
    // ARM64: PL011 uses blocking TX; async TX not implemented yet
}

#[cfg(target_arch = "x86_64")]
fn is_transmit_empty() -> bool {
    unsafe { inb(COM1 + 5) & 0x20 != 0 }
}

/// Write a single byte to the serial port, also capturing it in the log ring buffer.
pub fn write_byte(byte: u8) {
    unsafe {
        if !SERIAL_INITIALIZED {
            return;
        }
    }
    // Always capture to log ring buffer
    log_push_byte(byte);

    #[cfg(target_arch = "x86_64")]
    {
        if ASYNC_TX.load(Ordering::Acquire) {
            // Async path: push to TX buffer, kick UART if idle
            if tx_push(byte) {
                kick_tx();
            }
        } else {
            // Early boot: blocking poll
            while !is_transmit_empty() {
                core::hint::spin_loop();
            }
            unsafe { outb(COM1, byte); }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        crate::arch::arm64::serial::write_byte(byte);
    }
}

/// COM1 (IRQ 4) interrupt handler — drains the TX buffer into the UART FIFO.
#[cfg(target_arch = "x86_64")]
fn serial_irq_handler(_irq: u8) {
    // Read IIR to acknowledge the interrupt source
    let _iir = unsafe { inb(COM1 + 2) };
    // Drain up to 16 bytes (UART FIFO depth) per interrupt
    try_drain(16);
}

impl fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                write_byte(b'\r');
            }
            write_byte(byte);
        }
        Ok(())
    }
}

#[macro_export]
macro_rules! serial_print {
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _lock_state = $crate::drivers::serial::output_lock_acquire();
        let _ = write!($crate::drivers::serial::SerialPort, $($arg)*);
        $crate::drivers::serial::output_lock_release(_lock_state);
    }};
}

/// Helper for `serial_println!` — returns the current thread name as a printable wrapper.
/// Lock-free, safe to call from interrupt handlers and early boot.
pub fn caller_name() -> CallerName {
    let raw = crate::task::scheduler::debug_current_thread_name();
    CallerName(raw)
}

/// Wrapper for printing the caller thread name in `serial_println!`.
pub struct CallerName([u8; 32]);

impl fmt::Display for CallerName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let len = self.0.iter().position(|&b| b == 0).unwrap_or(32);
        if len == 0 {
            f.write_str("kernel")
        } else {
            let s = core::str::from_utf8(&self.0[..len]).unwrap_or("?");
            f.write_str(s)
        }
    }
}

#[macro_export]
macro_rules! serial_println {
    () => { $crate::serial_print!("\n") };
    ($($arg:tt)*) => {{
        use core::fmt::Write;
        let _lock_state = $crate::drivers::serial::output_lock_acquire();
        let _ticks = $crate::arch::hal::timer_current_ticks();
        let _ms = _ticks as u64 * 1000 / $crate::arch::hal::timer_frequency_hz();
        let _ = write!($crate::drivers::serial::SerialPort, "[{}] {}\n", _ms, format_args!($($arg)*));
        $crate::drivers::serial::output_lock_release(_lock_state);
    }};
}

#[cfg(feature = "debug_verbose")]
#[macro_export]
macro_rules! debug_println {
    () => { $crate::serial_print!("[DBG] \n") };
    ($($arg:tt)*) => { $crate::serial_print!("[DBG] {}\n", format_args!($($arg)*)) };
}

#[cfg(not(feature = "debug_verbose"))]
#[macro_export]
macro_rules! debug_println {
    ($($arg:tt)*) => {};
}
