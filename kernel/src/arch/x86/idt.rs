//! Interrupt Descriptor Table (IDT) for x86-64 long mode.
//!
//! Sets up 256 entries: CPU exceptions (ISR 0-31), hardware IRQs remapped
//! to INT 32-55, and the `int 0x80` syscall trap gate (DPL 3).

use crate::serial_println;
use core::arch::asm;
use core::mem::size_of;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Total IDT entries (covers the full x86 interrupt vector range).
const IDT_ENTRIES: usize = 256;

/// Saved DS/ES from the ISR/IRQ stub entry — captured BEFORE the stub
/// overwrites them with kernel segment 0x10.  Used by the #GP handler to
/// diagnose null-DS faults in 32-bit compat mode.
#[no_mangle]
pub static mut SAVED_FAULT_DS: u16 = 0;
#[no_mangle]
pub static mut SAVED_FAULT_ES: u16 = 0;

/// Per-CPU re-entrancy guard for the fault handler.
/// Prevents recursive SIGSEGV crash loops when SCHEDULER data is corrupted:
/// try_exit_current accesses corrupted per_cpu Vec ptr → #PF → handler calls
/// try_exit_current again → same #PF → infinite recursion with decreasing RSP.
static FAULT_HANDLER_ACTIVE: [AtomicBool; crate::arch::x86::smp::MAX_CPUS] = {
    const INIT: AtomicBool = AtomicBool::new(false);
    [INIT; crate::arch::x86::smp::MAX_CPUS]
};

/// Write a single byte to COM1 (0x3F8) blocking until UART is ready.
/// Completely lock-free — safe to call from any context.
#[inline]
fn uart_putc(c: u8) {
    unsafe {
        // Wait for Transmit Holding Register Empty (bit 5 of LSR)
        while crate::arch::x86::port::inb(0x3FD) & 0x20 == 0 {
            core::hint::spin_loop();
        }
        crate::arch::x86::port::outb(0x3F8, c);
    }
}

/// Write a string directly to UART (lock-free).
#[inline]
fn uart_puts(s: &[u8]) {
    for &c in s {
        if c == b'\n' {
            uart_putc(b'\r');
        }
        uart_putc(c);
    }
}

/// Write a decimal number directly to UART (lock-free).
fn uart_put_dec(mut n: u32) {
    if n == 0 {
        uart_putc(b'0');
        return;
    }
    let mut buf = [0u8; 10];
    let mut i = 0;
    while n > 0 {
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        uart_putc(buf[i]);
    }
}

/// Write a 64-bit hex number directly to UART (lock-free).
fn uart_put_hex(n: u64) {
    uart_puts(b"0x");
    if n == 0 {
        uart_putc(b'0');
        return;
    }
    let mut buf = [0u8; 16];
    let mut val = n;
    let mut i = 0usize;
    while val > 0 && i < 16 {
        let d = (val & 0xF) as u8;
        buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        val >>= 4;
        i += 1;
    }
    while i > 0 {
        i -= 1;
        uart_putc(buf[i]);
    }
}

fn uart_put_labeled_hex(label: &[u8], value: u64) {
    uart_puts(label);
    uart_put_hex(value);
}

fn dump_reentrant_fault_context(frame: &InterruptFrame, cr2: Option<u64>) {
    uart_puts(b"  ctx: RIP=");
    uart_put_hex(frame.rip);
    uart_puts(b" RSP=");
    uart_put_hex(frame.rsp);
    uart_puts(b" RBP=");
    uart_put_hex(frame.rbp);
    uart_puts(b" ERR=");
    uart_put_hex(frame.err_code);
    uart_putc(b'\n');

    uart_put_labeled_hex(b"  CR2=", cr2.unwrap_or(0));
    uart_puts(b" RAX=");
    uart_put_hex(frame.rax);
    uart_puts(b" RBX=");
    uart_put_hex(frame.rbx);
    uart_puts(b" RCX=");
    uart_put_hex(frame.rcx);
    uart_puts(b" RDX=");
    uart_put_hex(frame.rdx);
    uart_putc(b'\n');

    uart_puts(b"  RSI=");
    uart_put_hex(frame.rsi);
    uart_puts(b" RDI=");
    uart_put_hex(frame.rdi);
    uart_puts(b" R8=");
    uart_put_hex(frame.r8);
    uart_puts(b" R9=");
    uart_put_hex(frame.r9);
    uart_putc(b'\n');

    let cpu = crate::arch::x86::smp::current_cpu_id() as usize;
    let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu);
    uart_puts(b"  kstack=[");
    uart_put_hex(stack_bottom);
    uart_puts(b"..");
    uart_put_hex(stack_top);
    uart_puts(b"]\n");

    if stack_bottom != 0
        && frame.rsp >= stack_bottom
        && frame.rsp.saturating_add(8 * 8) <= stack_top
        && frame.rsp & 7 == 0
    {
        uart_puts(b"  stack qwords:");
        let words = frame.rsp as *const u64;
        unsafe {
            for i in 0..8usize {
                uart_puts(b" ");
                uart_put_hex(core::ptr::read_unaligned(words.add(i)));
            }
        }
        uart_putc(b'\n');
    } else {
        uart_puts(b"  stack qwords: unavailable\n");
    }

    let mut bp = frame.rbp;
    let mut depth = 0usize;
    uart_puts(b"  rbp-chain:");
    while stack_bottom != 0
        && bp >= stack_bottom
        && bp.saturating_add(16) <= stack_top
        && bp & 7 == 0
        && depth < 8
    {
        let ptr = bp as *const u64;
        let next = unsafe { core::ptr::read_unaligned(ptr) };
        let ret = unsafe { core::ptr::read_unaligned(ptr.add(1)) };
        uart_puts(b" ");
        uart_put_hex(ret);
        if next <= bp || next > stack_top {
            break;
        }
        bp = next;
        depth += 1;
    }
    if depth == 0 {
        uart_puts(b" none");
    }
    uart_putc(b'\n');
}
/// Check if the instruction at `rip` is IRETQ (0x48 0xCF) and dump the 5-value
/// return frame from the stack. When IRETQ faults, the CPU restores RSP to the
/// pre-IRETQ value, so `saved_rsp` points to: [RIP, CS, RFLAGS, RSP, SS].
fn dump_iretq_frame(rip: u64, saved_rsp: u64) {
    // Only check kernel-space addresses
    if rip < 0xFFFF_FFFF_8000_0000 {
        return;
    }
    // Read the 2-byte opcode at fault RIP: REX.W prefix (0x48) + IRET (0xCF)
    let opcodes = unsafe { core::slice::from_raw_parts(rip as *const u8, 2) };
    if opcodes[0] != 0x48 || opcodes[1] != 0xCF {
        return;
    }
    crate::serial_println!(
        "  >>> IRETQ fault detected! Dumping return frame at RSP={:#018x}:",
        saved_rsp
    );
    // Validate saved_rsp is in kernel higher-half and aligned
    if saved_rsp < 0xFFFF_FFFF_8000_0000 || saved_rsp & 0x7 != 0 {
        crate::serial_println!("  >>> Invalid RSP for IRETQ frame (not kernel-aligned)");
        return;
    }
    let p = saved_rsp as *const u64;
    unsafe {
        let ret_rip = *p;
        let ret_cs = *p.add(1);
        let ret_rflags = *p.add(2);
        let ret_rsp = *p.add(3);
        let ret_ss = *p.add(4);
        crate::serial_println!("    Return RIP:    {:#018x}", ret_rip);
        crate::serial_println!("    Return CS:     {:#018x}", ret_cs);
        crate::serial_println!("    Return RFLAGS: {:#018x}", ret_rflags);
        crate::serial_println!("    Return RSP:    {:#018x}", ret_rsp);
        crate::serial_println!("    Return SS:     {:#018x}", ret_ss);
        // Flag suspicious values
        let cs_rpl = ret_cs & 3;
        let canonical_ok = ret_rip < 0x0000_8000_0000_0000 || ret_rip >= 0xFFFF_8000_0000_0000;
        if !canonical_ok {
            crate::serial_println!("    !!! Return RIP is NON-CANONICAL — stack corruption!");
        }
        if cs_rpl == 3 {
            // User-mode return — check RSP is in user range
            if ret_rsp >= 0x0000_8000_0000_0000 {
                crate::serial_println!("    !!! Return RSP is non-canonical for user mode!");
            }
            let valid_user_cs = matches!(ret_cs, 0x1B | 0x2B);
            if !valid_user_cs {
                crate::serial_println!("    !!! Return CS={:#x} is not a valid user CS!", ret_cs);
            }
            if ret_ss != 0x23 {
                crate::serial_println!("    !!! Return SS={:#x} is not 0x23 (user data)!", ret_ss);
            }
        }
    }
}

/// Called from the ISR/IRQ assembly stubs after the Rust handler returned but
/// before `iretq`, when a kernel-mode return frame points outside kernel text.
/// This catches corruption while the original IRETQ frame is still readable,
/// instead of letting the CPU jump to low memory and reporting only RIP=0x3.
#[no_mangle]
pub extern "C" fn bad_kernel_iret_recovery(frame_rsp: u64, source: u32) -> ! {
    uart_puts(b"\n!!! BAD KERNEL IRET frame before return (");
    if source == 0 {
        uart_puts(b"ISR");
    } else {
        uart_puts(b"IRQ");
    }
    uart_puts(b")\n");

    let tid = crate::task::scheduler::debug_current_tid();
    uart_puts(b"  TID=");
    uart_put_dec(tid);
    uart_puts(b" user=");
    uart_put_dec(crate::task::scheduler::debug_is_current_user() as u32);
    uart_puts(b" frame=");
    uart_put_hex(frame_rsp);
    uart_puts(b"\n");

    if frame_rsp >= 0xFFFF_FFFF_8000_0000 && frame_rsp & 7 == 0 {
        let words = frame_rsp as *const u64;
        unsafe {
            let labels: [&[u8]; 5] = [b"RIP", b"CS ", b"RFL", b"RSP", b"SS "];
            for i in 0..5 {
                uart_puts(b"  ");
                uart_puts(labels[i]);
                uart_puts(b"=");
                uart_put_hex(core::ptr::read_unaligned(words.add(i)));
                uart_puts(b"\n");
            }
        }
    } else {
        uart_puts(b"  frame pointer is not a valid kernel-aligned address\n");
    }

    // If this was a user thread executing in kernel context, park the CPU via
    // the same manual fault recovery used by normal syscall/IRQ faults. For
    // idle or pure kernel contexts, continuing would risk corrupting the
    // scheduler further, so halt after the exact frame has been printed.
    if tid != 0 && crate::task::scheduler::debug_is_current_user() {
        crate::task::scheduler::fault_kill_and_idle(132);
    }

    uart_puts(b"  unrecoverable kernel/idle IRET corruption; halting CPU\n");
    unsafe {
        loop {
            asm!("cli; hlt", options(nomem, nostack));
        }
    }
}

/// GDT selector for Ring 0 code segment.
const KERNEL_CODE_SEG: u16 = 0x08;

/// x86-64 IDT entry (16 bytes).
#[repr(C, packed)]
#[derive(Copy, Clone)]
struct IdtEntry {
    offset_low: u16,  // Handler address bits 0-15
    selector: u16,    // Kernel code segment selector
    ist: u8,          // IST index (bits 0-2), zero (bits 3-7)
    type_attr: u8,    // Gate type and attributes
    offset_mid: u16,  // Handler address bits 16-31
    offset_high: u32, // Handler address bits 32-63
    _reserved: u32,   // Must be zero
}

#[repr(C, packed)]
struct IdtDescriptor {
    size: u16,
    offset: u64,
}

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry {
    offset_low: 0,
    selector: 0,
    ist: 0,
    type_attr: 0,
    offset_mid: 0,
    offset_high: 0,
    _reserved: 0,
}; IDT_ENTRIES];

static mut IDT_DESC: IdtDescriptor = IdtDescriptor { size: 0, offset: 0 };

// Gate type attributes (interpreted as 64-bit gates in long mode)
const GATE_INTERRUPT: u8 = 0x8E; // Present, DPL=0, 64-bit interrupt gate
const GATE_TRAP: u8 = 0x8F; // Present, DPL=0, 64-bit trap gate
const GATE_TRAP_DPL3: u8 = 0xEF; // Present, DPL=3, 64-bit trap gate (for syscalls)

fn set_gate(num: usize, handler: unsafe extern "C" fn(), selector: u16, type_attr: u8) {
    set_gate_ist(num, handler, selector, type_attr, 0);
}

fn set_gate_ist(
    num: usize,
    handler: unsafe extern "C" fn(),
    selector: u16,
    type_attr: u8,
    ist: u8,
) {
    let handler = handler as *const () as u64;
    unsafe {
        IDT[num] = IdtEntry {
            offset_low: (handler & 0xFFFF) as u16,
            selector,
            ist: ist & 0x7, // IST index in bits 0-2
            type_attr,
            offset_mid: ((handler >> 16) & 0xFFFF) as u16,
            offset_high: ((handler >> 32) & 0xFFFFFFFF) as u32,
            _reserved: 0,
        };
    }
}

// External ISR/IRQ stubs from interrupts.asm
extern "C" {
    fn isr0();
    fn isr1();
    fn isr2();
    fn isr3();
    fn isr4();
    fn isr5();
    fn isr6();
    fn isr7();
    fn isr8();
    fn isr9();
    fn isr10();
    fn isr11();
    fn isr12();
    fn isr13();
    fn isr14();
    fn isr15();
    fn isr16();
    fn isr17();
    fn isr18();
    fn isr19();
    fn isr20();
    fn isr21();
    fn isr22();
    fn isr23();
    fn isr24();
    fn isr25();
    fn isr26();
    fn isr27();
    fn isr28();
    fn isr29();
    fn isr30();
    fn isr31();

    fn irq0();
    fn irq1();
    fn irq2();
    fn irq3();
    fn irq4();
    fn irq5();
    fn irq6();
    fn irq7();
    fn irq8();
    fn irq9();
    fn irq10();
    fn irq11();
    fn irq12();
    fn irq13();
    fn irq14();
    fn irq15();
    // LAPIC / APIC vectors
    fn irq16();
    fn irq17();
    fn irq18();
    fn irq19();
    fn irq20();
    fn irq21();
    fn irq22();
    fn irq23();
    // PCI MSI slots (vectors 56-87)
    fn irq24();
    fn irq25();
    fn irq26();
    fn irq27();
    fn irq28();
    fn irq29();
    fn irq30();
    fn irq31();
    fn irq32();
    fn irq33();
    fn irq34();
    fn irq35();
    fn irq36();
    fn irq37();
    fn irq38();
    fn irq39();
    fn irq40();
    fn irq41();
    fn irq42();
    fn irq43();
    fn irq44();
    fn irq45();
    fn irq46();
    fn irq47();
    fn irq48();
    fn irq49();
    fn irq50();
    fn irq51();
    fn irq52();
    fn irq53();
    fn irq54();
    fn irq55();
}

/// Populate the IDT with exception, IRQ, and syscall gates, then load via `lidt`.
pub fn init() {
    // CPU Exceptions (ISR 0-31)
    set_gate(0, isr0, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(1, isr1, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(2, isr2, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(3, isr3, KERNEL_CODE_SEG, GATE_TRAP);
    set_gate(4, isr4, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(5, isr5, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(6, isr6, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(7, isr7, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate_ist(8, isr8, KERNEL_CODE_SEG, GATE_INTERRUPT, 1); // #DF uses IST1
    set_gate(9, isr9, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(10, isr10, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(11, isr11, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(12, isr12, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(13, isr13, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(14, isr14, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(15, isr15, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(16, isr16, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(17, isr17, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(18, isr18, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(19, isr19, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(20, isr20, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(21, isr21, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(22, isr22, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(23, isr23, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(24, isr24, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(25, isr25, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(26, isr26, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(27, isr27, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(28, isr28, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(29, isr29, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(30, isr30, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(31, isr31, KERNEL_CODE_SEG, GATE_INTERRUPT);

    // Hardware IRQs (remapped to INT 32-47)
    set_gate(32, irq0, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(33, irq1, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(34, irq2, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(35, irq3, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(36, irq4, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(37, irq5, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(38, irq6, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(39, irq7, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(40, irq8, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(41, irq9, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(42, irq10, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(43, irq11, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(44, irq12, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(45, irq13, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(46, irq14, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(47, irq15, KERNEL_CODE_SEG, GATE_INTERRUPT);

    // LAPIC / APIC vectors (INT 48-55)
    set_gate(48, irq16, KERNEL_CODE_SEG, GATE_INTERRUPT); // LAPIC Timer
    set_gate(49, irq17, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(50, irq18, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(51, irq19, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(52, irq20, KERNEL_CODE_SEG, GATE_INTERRUPT); // IPI: TLB
    set_gate(53, irq21, KERNEL_CODE_SEG, GATE_INTERRUPT); // IPI: Halt
    set_gate(54, irq22, KERNEL_CODE_SEG, GATE_INTERRUPT); // IPI: Reschedule
    set_gate(55, irq23, KERNEL_CODE_SEG, GATE_INTERRUPT); // Reserved APIC slot
                                                          // PCI MSI slots (vectors 56-87)
    set_gate(56, irq24, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(57, irq25, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(58, irq26, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(59, irq27, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(60, irq28, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(61, irq29, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(62, irq30, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(63, irq31, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(64, irq32, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(65, irq33, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(66, irq34, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(67, irq35, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(68, irq36, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(69, irq37, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(70, irq38, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(71, irq39, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(72, irq40, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(73, irq41, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(74, irq42, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(75, irq43, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(76, irq44, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(77, irq45, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(78, irq46, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(79, irq47, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(80, irq48, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(81, irq49, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(82, irq50, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(83, irq51, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(84, irq52, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(85, irq53, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(86, irq54, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(87, irq55, KERNEL_CODE_SEG, GATE_INTERRUPT);

    // INT 0x80 vector is intentionally left unset — 64-bit user space uses
    // the SYSCALL fast path (kernel/asm/x86/syscall_fast.asm).

    // Load IDT
    unsafe {
        IDT_DESC = IdtDescriptor {
            size: (IDT_ENTRIES * size_of::<IdtEntry>() - 1) as u16,
            offset: (&raw const IDT) as *const _ as u64,
        };

        asm!(
            "lidt [{}]",
            in(reg) &raw const IDT_DESC,
            options(nostack, preserves_flags)
        );
    }
}

/// Reload the IDT on the current CPU.
/// Used by APs to load the kernel IDT after trampoline startup.
pub fn reload() {
    unsafe {
        asm!(
            "lidt [{}]",
            in(reg) &raw const IDT_DESC,
            options(nostack, preserves_flags)
        );
    }
}

/// Interrupt stack frame for x86-64 long mode.
///
/// In 64-bit mode the CPU always pushes SS and RSP (even for same-privilege
/// interrupts). Our assembly stubs push all 15 GPRs individually (no pushad
/// in 64-bit mode).
#[repr(C)]
pub struct InterruptFrame {
    // Pushed by stub (last push = lowest address = first field)
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rbp: u64,
    pub rdi: u64,
    pub rsi: u64,
    pub rdx: u64,
    pub rcx: u64,
    pub rbx: u64,
    pub rax: u64,
    // Pushed by stub
    pub int_no: u64,
    pub err_code: u64,
    // Pushed by CPU
    pub rip: u64,
    pub cs: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u64,
}

const LINUX_VSYSCALL_GETTIMEOFDAY: u64 = 0xffff_ffff_ff60_0000;
const LINUX_VSYSCALL_TIME: u64 = 0xffff_ffff_ff60_0400;
const LINUX_VSYSCALL_GETCPU: u64 = 0xffff_ffff_ff60_0800;

fn try_handle_linux_vsyscall_fault(frame: &InterruptFrame, cr2: u64) -> bool {
    if frame.rip != cr2 {
        return false;
    }
    if frame.rip != LINUX_VSYSCALL_GETTIMEOFDAY
        && frame.rip != LINUX_VSYSCALL_TIME
        && frame.rip != LINUX_VSYSCALL_GETCPU
    {
        return false;
    }
    if crate::task::scheduler::current_thread_abi() != crate::task::abi::AbiPersonality::LinuxX86_64
    {
        return false;
    }

    let Some(ret_bytes) = crate::syscall::handlers::helpers::copy_user_bytes(frame.rsp, 8, 8)
    else {
        return false;
    };
    let mut ret_arr = [0u8; 8];
    ret_arr.copy_from_slice(&ret_bytes[..8]);
    let return_rip = u64::from_le_bytes(ret_arr);
    let sec = linux_compat_now_seconds();
    let mut rax = 0u64;

    match frame.rip {
        LINUX_VSYSCALL_GETTIMEOFDAY => {
            if frame.rdi != 0 {
                if !copy_user_u64(frame.rdi, sec) || !copy_user_u64(frame.rdi + 8, 0) {
                    rax = (-14i64) as u64;
                }
            }
            if rax == 0 && frame.rsi != 0 {
                if !copy_user_u32(frame.rsi, 0) || !copy_user_u32(frame.rsi + 4, 0) {
                    rax = (-14i64) as u64;
                }
            }
        }
        LINUX_VSYSCALL_TIME => {
            rax = sec;
            if frame.rdi != 0 && !copy_user_u64(frame.rdi, sec) {
                rax = (-14i64) as u64;
            }
        }
        LINUX_VSYSCALL_GETCPU => {
            if frame.rdi != 0
                && !copy_user_u32(frame.rdi, crate::arch::x86::smp::current_cpu_id() as u32)
            {
                rax = (-14i64) as u64;
            }
            if rax == 0 && frame.rsi != 0 && !copy_user_u32(frame.rsi, 0) {
                rax = (-14i64) as u64;
            }
        }
        _ => return false,
    }

    let frame_mut = frame as *const InterruptFrame as *mut InterruptFrame;
    unsafe {
        (*frame_mut).rax = rax;
        (*frame_mut).rip = return_rip;
        (*frame_mut).rsp = frame.rsp + 8;
    }
    true
}

fn linux_compat_now_seconds() -> u64 {
    crate::time::wall_clock_unix_secs()
}

fn copy_user_u64(ptr: u64, value: u64) -> bool {
    crate::syscall::handlers::helpers::copy_to_user_bytes(ptr, &value.to_le_bytes(), 8)
}

fn copy_user_u32(ptr: u64, value: u32) -> bool {
    crate::syscall::handlers::helpers::copy_to_user_bytes(ptr, &value.to_le_bytes(), 4)
}

/// Try to recover from a fault by killing the current thread.
/// Returns true if the thread was killed (never actually returns to caller
/// because exit_current switches context). Returns false if unrecoverable.
///
/// Uses LOCK-FREE scheduler queries (debug_current_tid, debug_is_current_user)
/// to prevent deadlock: a page fault could fire while the scheduler lock is held
/// (e.g., demand paging on heap memory accessed from schedule_inner). Using
/// SCHEDULER.lock() in that scenario would deadlock on a single CPU with IF=0.
///
/// Also uses try_exit_current (try_lock) instead of exit_current to avoid
/// deadlock when the scheduler lock is already held by this CPU.
fn signal_name(signal: u32) -> &'static str {
    match signal {
        132 => "SIGILL (Invalid opcode)",
        135 => "SIGBUS (#NM Device not available)",
        136 => "SIGFPE (Floating-point exception)",
        139 => "SIGSEGV (Segmentation fault)",
        _ => "Unknown signal",
    }
}

fn last_syscall_diag(cpu: usize) -> (u32, &'static str, &'static str) {
    let num = crate::task::scheduler::get_last_syscall(cpu);
    match crate::task::scheduler::get_last_syscall_abi(cpu) {
        1 => (num, "linux", crate::syscall::linux::syscall_name(num)),
        _ => (num, "anyos", crate::syscall::table::syscall_name(num)),
    }
}

fn try_kill_faulting_thread(signal: u32, frame: &InterruptFrame) -> bool {
    let tid = crate::task::scheduler::debug_current_tid();
    // TID 0 = idle context
    if tid == 0 {
        return false; // Can't kill idle context
    }
    // Re-entrancy guard: if this CPU is already inside the fault handler
    // (e.g., try_exit_current faulted on corrupted SCHEDULER data), skip
    // the try_exit_current retry loop and go straight to manual recovery.
    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
    if cpu_id < crate::arch::x86::smp::MAX_CPUS {
        if FAULT_HANDLER_ACTIVE[cpu_id].swap(true, Ordering::Relaxed) {
            // Already active — re-entrant fault detected!
            uart_puts(b"\n!!! RE-ENTRANT FAULT on CPU");
            uart_put_dec(cpu_id as u32);
            uart_puts(b" TID=");
            uart_put_dec(tid);
            uart_puts(b" signal=");
            uart_put_dec(signal);
            uart_puts(b" RIP=");
            uart_put_hex(frame.rip);
            uart_puts(b" -- manual recovery\n");
            // Log CR2 for page faults to help diagnose the corruption
            let mut cr2_opt = None;
            if frame.int_no == 14 {
                let cr2: u64;
                unsafe {
                    core::arch::asm!("mov {}, cr2", out(reg) cr2);
                }
                cr2_opt = Some(cr2);
                uart_puts(b"  CR2=");
                uart_put_hex(cr2);
                uart_puts(b" (faulting address)\n");
            }
            dump_reentrant_fault_context(frame, cr2_opt);
            FAULT_HANDLER_ACTIVE[cpu_id].store(false, Ordering::Relaxed);
            crate::task::scheduler::fault_kill_tid_and_idle(tid, signal);
        }
    }
    if crate::task::scheduler::debug_is_current_user() {
        // Flush the async TX buffer and switch to blocking output so the crash
        // report is not interleaved with buffered bytes from other contexts.
        crate::drivers::serial::flush_blocking();
        crate::serial_println!("--- Thread Crash Report ---");
        crate::serial_println!("  TID:    {}", tid);
        {
            let name_buf = crate::task::scheduler::debug_current_thread_name();
            let name_len = name_buf
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(name_buf.len());
            let name = core::str::from_utf8(&name_buf[..name_len]).unwrap_or("<invalid>");
            crate::serial_println!("  Name:   \"{}\"", name);
        }
        crate::serial_println!("  Signal: {} ({})", signal, signal_name(signal));
        crate::serial_println!("  RIP:    {:#018x}", frame.rip);
        crate::serial_println!("  RSP:    {:#018x}  RBP: {:#018x}", frame.rsp, frame.rbp);
        crate::serial_println!("  RAX:    {:#018x}  RBX: {:#018x}", frame.rax, frame.rbx);
        crate::serial_println!("  RCX:    {:#018x}  RDX: {:#018x}", frame.rcx, frame.rdx);
        crate::serial_println!("  RSI:    {:#018x}  RDI: {:#018x}", frame.rsi, frame.rdi);
        crate::serial_println!("  R8:     {:#018x}  R9:  {:#018x}", frame.r8, frame.r9);
        crate::serial_println!("  R10:    {:#018x}  R11: {:#018x}", frame.r10, frame.r11);
        crate::serial_println!(
            "  CS:     {:#06x}  SS:  {:#06x}  RFLAGS: {:#018x}",
            frame.cs,
            frame.ss,
            frame.rflags
        );
        {
            let crash_cpu = crate::arch::x86::smp::current_cpu_id() as usize;
            let (last_sc, last_abi, last_name) = last_syscall_diag(crash_cpu);
            crate::serial_println!("  LastSC: {}:{} ({})", last_abi, last_sc, last_name);
            if last_sc == crate::syscall::SYS_GPU_COMMAND {
                let (driver_data, driver_vtable) = crate::drivers::gpu::last_driver_ptrs();
                let (virtio_ctrl, virtio_cursor) =
                    crate::drivers::gpu::virtio_gpu::VirtioGpu::last_command_types();
                crate::serial_println!(
                    "  GPU:    driver_data={:#010x}  driver_vtable={:#018x}",
                    driver_data,
                    driver_vtable
                );
                crate::serial_println!(
                    "  GPU:    virtio_last_ctrl={:#x}  virtio_last_cursor={:#x}",
                    virtio_ctrl,
                    virtio_cursor
                );
                let (controlq, cursorq) =
                    crate::drivers::gpu::virtio_gpu::VirtioGpu::queue_debug_info();
                crate::serial_println!(
                    "  GPU:    ctrlq qsz={} avail={} used={} free={} broken={}",
                    controlq.0,
                    controlq.1,
                    controlq.2,
                    controlq.3,
                    controlq.4 as u8
                );
                crate::serial_println!(
                    "  GPU:    cursq qsz={} avail={} used={} free={} broken={}",
                    cursorq.0,
                    cursorq.1,
                    cursorq.2,
                    cursorq.3,
                    cursorq.4 as u8
                );
                if virtio_ctrl == 0x105 {
                    let (res, x, y, w, h, offset) =
                        crate::drivers::gpu::virtio_gpu::VirtioGpu::last_transfer_info();
                    crate::serial_println!(
                        "  GPU:    transfer res={} x={} y={} w={} h={} off={:#x}",
                        res,
                        x,
                        y,
                        w,
                        h,
                        offset
                    );
                }
            }
        }
        if signal == 139 && frame.int_no == 14 {
            // Page fault — show CR2 (faulting address)
            let cr2: u64;
            unsafe {
                core::arch::asm!("mov {}, cr2", out(reg) cr2);
            }
            let reason = match frame.err_code & 0x7 {
                0b000 => "read from non-present page",
                0b001 => "read protection violation",
                0b010 => "write to non-present page",
                0b011 => "write protection violation",
                0b100 => "exec from non-present page",
                0b101 => "exec protection violation",
                _ => "unknown",
            };
            crate::serial_println!("  CR2:    {:#018x} ({})", cr2, reason);
        }
        // Detect IRETQ fault and dump the return frame
        dump_iretq_frame(frame.rip, frame.rsp);
        if frame.err_code != 0 && frame.int_no != 14 {
            crate::serial_println!("  Error:  {:#x}", frame.err_code);
        }
        if frame.cs == 0x08 {
            let cpu = crate::arch::x86::smp::current_cpu_id() as usize;
            let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu);
            let in_bounds =
                stack_bottom != 0 && frame.rsp >= stack_bottom && frame.rsp <= stack_top;
            crate::serial_println!(
                "  KStack: [{:#018x}..{:#018x}] rsp_in_bounds={}",
                stack_bottom,
                stack_top,
                in_bounds as u8
            );
            if stack_bottom != 0 {
                let used = stack_top.saturating_sub(frame.rsp);
                let total = stack_top.saturating_sub(stack_bottom);
                crate::serial_println!("  KStack used: {} / {}", used, total);
                if in_bounds {
                    let dump_start = frame.rsp;
                    let dump_end = dump_start.saturating_add(12 * 8);
                    if dump_end <= stack_top {
                        crate::serial_println!("  KStack qwords @RSP:");
                        let words = dump_start as *const u64;
                        unsafe {
                            for i in 0..12 {
                                crate::serial_println!(
                                    "    +{:#04x}: {:#018x}",
                                    i * 8,
                                    core::ptr::read_unaligned(words.add(i))
                                );
                            }
                        }
                    }
                }
            }
        }
        // User-space stack trace (walk RBP chain)
        crate::serial_println!("  Stack trace:");
        let mut bp = frame.rbp;
        let mut depth = 0;
        while bp > 0x1000 && bp < 0x0000_8000_0000_0000 && bp & 7 == 0 && depth < 16 {
            let ret_addr = unsafe { *((bp + 8) as *const u64) };
            if ret_addr == 0 || ret_addr >= 0x0000_8000_0000_0000 {
                break;
            }
            crate::serial_println!("    #{}: {:#018x}", depth, ret_addr);
            bp = unsafe { *(bp as *const u64) };
            depth += 1;
        }
        if depth == 0 {
            crate::serial_println!("    (no valid frames — RBP={:#018x})", frame.rbp);
        }
        crate::serial_println!("--- End Crash Report ---");

        // Store crash report for retrieval by compositor crash dialog
        crate::task::crash_info::store_crash(tid, signal, frame);

        // Force-release any kernel locks held by THIS CPU. A fault during a
        // syscall or demand-page handler can leave locks permanently held,
        // deadlocking the entire system. Check all critical locks:
        let cpu = crate::arch::x86::smp::current_cpu_id() as u32;
        let mut recovery_state_dirty = false;
        if crate::task::scheduler::is_scheduler_locked_by_cpu(cpu) {
            unsafe {
                crate::task::scheduler::force_unlock_scheduler();
            }
            recovery_state_dirty = true;
        }
        if crate::memory::physical::is_allocator_locked_by_cpu(cpu) {
            unsafe {
                crate::memory::physical::force_unlock_allocator();
            }
            crate::serial_println!("  RECOVERED: force-released physical allocator lock");
            recovery_state_dirty = true;
        }
        if crate::task::dll::is_dll_locked_by_cpu(cpu) {
            unsafe {
                crate::task::dll::force_unlock_dlls();
            }
            crate::serial_println!("  RECOVERED: force-released LOADED_DLLS lock");
            recovery_state_dirty = true;
        }
        // GPU Mutex: a fault inside a GPU syscall (e.g. SYS_GPU_COMMAND, DMA)
        // leaves the GPU Mutex held. Without release, every subsequent GPU call
        // from any thread blocks forever (yielding Mutex → schedule loop).
        if crate::drivers::gpu::is_gpu_locked() {
            unsafe {
                crate::drivers::gpu::force_unlock_gpu();
            }
            crate::serial_println!("  RECOVERED: force-released GPU mutex");
            recovery_state_dirty = true;
        }

        // Once we had to force-release kernel locks, we no longer trust the
        // global state enough to route the fault through the normal scheduler
        // path. In that case, skip straight to the dedicated fallback that
        // parks the CPU on the idle stack.
        //
        // Even on the clean path we must NOT call schedule() from the fault
        // handler. Re-entering the generic scheduler from an exception frame
        // is fragile and was a major source of corruption when the current
        // thread faulted during a syscall. Try a few non-blocking exit attempts,
        // then fall back to the dedicated recovery path.
        if !recovery_state_dirty {
            for _ in 0..8 {
                if crate::task::scheduler::try_exit_current(signal) {
                    if cpu_id < crate::arch::x86::smp::MAX_CPUS {
                        FAULT_HANDLER_ACTIVE[cpu_id].store(false, Ordering::Relaxed);
                    }
                    unreachable!();
                }
                for _ in 0..64 {
                    core::hint::spin_loop();
                }
            }
        }
        // Clean path failed — use manual fallback: kill thread, repair state,
        // enter idle loop. Does NOT call schedule()/context_switch, avoiding
        // the deadlock where schedule_inner's try_lock loop spins forever.
        crate::serial_println!(
            "  fault recovery using manual kill path (scheduler state dirty={}): falling back",
            recovery_state_dirty as u8
        );
        if cpu_id < crate::arch::x86::smp::MAX_CPUS {
            FAULT_HANDLER_ACTIVE[cpu_id].store(false, Ordering::Relaxed);
        }
        crate::task::scheduler::fault_kill_tid_and_idle(tid, signal);
    }
    // Clear re-entrancy guard on the normal return path
    if cpu_id < crate::arch::x86::smp::MAX_CPUS {
        FAULT_HANDLER_ACTIVE[cpu_id].store(false, Ordering::Relaxed);
    }
    false
}

/// High-level CPU exception handler called from assembly ISR stubs.
///
/// Handles division-by-zero, invalid opcode, double fault, GPF, and page
/// faults. For user-mode faults the offending thread is terminated; for
/// kernel faults panic mode is entered (halts other CPUs) and this CPU halts.
#[no_mangle]
pub extern "C" fn isr_handler(frame: &InterruptFrame) {
    // CRITICAL: Detect corrupt RSP before doing ANYTHING that uses the stack.
    // If TSS.RSP0 was 0/corrupt, `frame` points to near-zero addresses and
    // ANY function call or serial_println! will loop forever with IF=0.
    // Check the frame pointer itself — it must be in kernel higher-half.
    let frame_addr = frame as *const InterruptFrame as u64;
    if frame_addr < 0xFFFF_FFFF_8000_0000 || frame_addr > 0xFFFF_FFFF_F000_0000 {
        // Stack is garbage — print via lock-free direct UART and halt.
        // DO NOT call any Rust functions (they need a valid stack).
        unsafe {
            use crate::arch::x86::port::{inb, outb};
            let msg = b"\r\n!!! FATAL: ISR entered with corrupt RSP frame=";
            for &c in msg {
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, c);
            }
            // Print frame address in hex
            let mut n = frame_addr;
            let mut buf = [0u8; 16];
            let mut i = 0usize;
            loop {
                let d = (n & 0xF) as u8;
                buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
                n >>= 4;
                i += 1;
                if n == 0 || i >= 16 {
                    break;
                }
            }
            let prefix = b"0x";
            for &c in prefix {
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, c);
            }
            while i > 0 {
                i -= 1;
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, buf[i]);
            }
            let msg2 = b" - halting CPU\r\n";
            for &c in msg2 {
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, c);
            }
            // Print TSS.RSP0 for this CPU
            let rsp0 = crate::arch::x86::tss::get_kernel_stack_for_cpu(0);
            let msg3 = b"  TSS.RSP0=";
            for &c in msg3 {
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, c);
            }
            n = rsp0;
            i = 0;
            loop {
                let d = (n & 0xF) as u8;
                buf[i] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
                n >>= 4;
                i += 1;
                if n == 0 || i >= 16 {
                    break;
                }
            }
            for &c in prefix {
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, c);
            }
            while i > 0 {
                i -= 1;
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, buf[i]);
            }
            let nl = b"\r\n";
            for &c in nl {
                while inb(0x3FD) & 0x20 == 0 {}
                outb(0x3F8, c);
            }
            // Halt this CPU cleanly — let other CPUs continue
            loop {
                core::arch::asm!("cli; hlt");
            }
        }
    }

    // Garbled frame detection: CS must be a valid segment selector.
    // If CS contains a kernel heap address instead of 0x08/0x1B/0x2B, the
    // exception frame was pushed to wrong memory (stale TSS.RSP0 pointing
    // to freed/reused stack) or adjacent heap was corrupted by stack overflow.
    let cs_valid = matches!(frame.cs, 0x08 | 0x1B | 0x2B);
    if !cs_valid {
        let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
        let tss_rsp0 = crate::arch::x86::tss::get_kernel_stack_for_cpu(cpu_id);
        let percpu_krsp = crate::arch::x86::syscall_msr::get_kernel_rsp(cpu_id);
        let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
        let tid = crate::task::scheduler::debug_current_tid();
        uart_puts(b"\n!!! GARBLED InterruptFrame detected!\n");
        uart_puts(b"  CS=");
        uart_put_hex(frame.cs);
        uart_puts(b" SS=");
        uart_put_hex(frame.ss);
        uart_puts(b" RIP=");
        uart_put_hex(frame.rip);
        uart_puts(b" int=");
        uart_put_dec(frame.int_no as u32);
        uart_putc(b'\n');
        uart_puts(b"  frame_addr=");
        uart_put_hex(frame as *const InterruptFrame as u64);
        uart_puts(b" TSS.RSP0=");
        uart_put_hex(tss_rsp0);
        uart_puts(b" PERCPU.krsp=");
        uart_put_hex(percpu_krsp);
        uart_putc(b'\n');
        uart_puts(b"  stack=[");
        uart_put_hex(stack_bottom);
        uart_puts(b"..");
        uart_put_hex(stack_top);
        uart_puts(b"] cpu=");
        uart_put_dec(cpu_id as u32);
        uart_puts(b" tid=");
        uart_put_dec(tid);
        uart_putc(b'\n');
        // Check if frame is inside or outside expected stack bounds
        let frame_u64 = frame as *const InterruptFrame as u64;
        if stack_bottom != 0 && (frame_u64 < stack_bottom || frame_u64 > stack_top) {
            uart_puts(b"  >>> Frame is OUTSIDE kernel stack bounds!\n");
        }
        // Check if TSS.RSP0 is inside expected stack bounds
        if stack_bottom != 0 && (tss_rsp0 < stack_bottom || tss_rsp0 > stack_top) {
            uart_puts(b"  >>> TSS.RSP0 is OUTSIDE kernel stack bounds!\n");
        }
        // Check stack canary for current thread
        if stack_bottom != 0 && stack_bottom >= 0xFFFF_FFFF_8000_0000 {
            let canary = unsafe { *(stack_bottom as *const u64) };
            if canary != crate::task::thread::STACK_CANARY {
                uart_puts(b"  >>> STACK CANARY DEAD! canary=");
                uart_put_hex(canary);
                uart_puts(b" expected=");
                uart_put_hex(crate::task::thread::STACK_CANARY);
                uart_putc(b'\n');
            }
        }
        // Try to kill the faulting thread via the existing recovery path
        // (uses debug_is_current_user which doesn't depend on frame.cs)
    }

    let is_user_mode = frame.cs & 3 != 0;

    match frame.int_no {
        0 => {
            let dbg_tid = crate::task::scheduler::debug_current_tid();
            if is_user_mode {
                crate::serial_println!(
                    "EXCEPTION: Division by zero at RIP={:#018x} CS={:#x} (TID={})",
                    frame.rip,
                    frame.cs,
                    dbg_tid
                );
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::crash_info::store_crash(dbg_tid, 136, frame);
                if try_kill_faulting_thread(136, frame) {
                    return;
                }
            }
            if try_kill_faulting_thread(136, frame) {
                return;
            }
            // Fatal kernel fault — enter panic mode to halt other CPUs
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!(
                "EXCEPTION: Division by zero at RIP={:#018x} CS={:#x} (TID={})",
                frame.rip,
                frame.cs,
                dbg_tid
            );
            crate::serial_println!(
                "  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                frame.rax,
                frame.rbx,
                frame.rcx,
                frame.rdx
            );
            crate::serial_println!(
                "  RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}",
                frame.rsi,
                frame.rdi,
                frame.rbp,
                frame.rsp
            );
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "Division by Zero (#DE)");
            loop {
                unsafe {
                    core::arch::asm!("cli; hlt");
                }
            }
        }
        1 => {
            // #DB Debug Exception — check if hardware watchpoint (DR0) fired.
            // We use DR0 to watch TSS.RSP0 for corruption: ANY write to that
            // address (including wild pointers) triggers this handler.
            let dr6: u64;
            unsafe {
                core::arch::asm!("mov {}, dr6", out(reg) dr6, options(nostack, nomem));
            }

            if dr6 & 1 != 0 {
                // DR0 watchpoint hit — something wrote to TSS.RSP0.
                // frame.rip is the instruction AFTER the write (x86 data breakpoints
                // fire after the instruction completes).
                let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
                let tss_rsp0 = crate::arch::x86::tss::get_kernel_stack_for_cpu(cpu_id);
                let tid = crate::task::scheduler::debug_current_tid();

                // Check if the new RSP0 value is corrupt (user-space range or zero)
                let is_bad = tss_rsp0 == 0 || tss_rsp0 < 0xFFFF_FFFF_8000_0000;

                // Only log bad writes to reduce noise (legitimate writes happen on
                // every context switch). To see ALL writes, change to `true`.
                if is_bad {
                    crate::serial_println!(
                        "!!! TSS.RSP0 WATCHPOINT: BAD write detected! RIP={:#018x} RSP0={:#018x} CPU{} TID={}",
                        frame.rip, tss_rsp0, cpu_id, tid
                    );
                    crate::serial_println!(
                        "    RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                        frame.rax,
                        frame.rbx,
                        frame.rcx,
                        frame.rdx
                    );
                    crate::serial_println!(
                        "    RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}",
                        frame.rsi,
                        frame.rdi,
                        frame.rbp,
                        frame.rsp
                    );
                    crate::serial_println!(
                        "    R8={:#018x} R9={:#018x} R10={:#018x} R11={:#018x}",
                        frame.r8,
                        frame.r9,
                        frame.r10,
                        frame.r11
                    );
                    // Immediately repair TSS.RSP0 from per-CPU stack top
                    let (_, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                    if stack_top >= 0xFFFF_FFFF_8000_0000 {
                        crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, stack_top);
                        crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, stack_top);
                        crate::serial_println!("    REPAIRED RSP0={:#018x}", stack_top);
                    }
                }

                // Clear DR6 to acknowledge the breakpoint
                unsafe {
                    core::arch::asm!("xor {tmp}, {tmp}; mov dr6, {tmp}", tmp = out(reg) _, options(nostack));
                }
                return;
            }

            if dr6 & 2 != 0 {
                // DR1 watchpoint hit — something wrote to compositor's CpuContext.rip.
                // This fires on EVERY write (including legitimate context_switch saves).
                // We only log when the written value looks corrupt.
                let watch_addr = crate::task::scheduler::get_dr1_watch_addr();
                if watch_addr != 0 {
                    let written_val = unsafe { core::ptr::read_volatile(watch_addr as *const u64) };
                    let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
                    let tid = crate::task::scheduler::debug_current_tid();

                    // Check if the new RIP value is corrupt (not in kernel text range)
                    let is_bad =
                        written_val < 0xFFFF_FFFF_8010_0000 || written_val >= 0xFFFF_FFFF_C000_0000;

                    if is_bad {
                        crate::serial_println!(
                            "!!! DR1 WATCHPOINT: BAD write to CpuContext.rip! writer_RIP={:#018x} written_val={:#018x} CPU{} TID={}",
                            frame.rip, written_val, cpu_id, tid
                        );
                        crate::serial_println!(
                            "    RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                            frame.rax,
                            frame.rbx,
                            frame.rcx,
                            frame.rdx
                        );
                        crate::serial_println!(
                            "    RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}",
                            frame.rsi,
                            frame.rdi,
                            frame.rbp,
                            frame.rsp
                        );
                        crate::serial_println!(
                            "    R8={:#018x} R9={:#018x} R10={:#018x} R11={:#018x}",
                            frame.r8,
                            frame.r9,
                            frame.r10,
                            frame.r11
                        );
                    }
                }

                // Clear DR6 to acknowledge
                unsafe {
                    core::arch::asm!("xor {tmp}, {tmp}; mov dr6, {tmp}", tmp = out(reg) _, options(nostack));
                }
                return;
            }

            // Not a watchpoint — check for debug-attached thread (anyTrace)
            if is_user_mode {
                if crate::task::scheduler::is_debug_attached_current() {
                    // Single-step completed on a debug-attached thread.
                    // Suspend the thread and notify the debugger.
                    let tid = crate::task::scheduler::debug_current_tid();
                    unsafe {
                        core::arch::asm!("xor {tmp}, {tmp}; mov dr6, {tmp}", tmp = out(reg) _, options(nostack));
                    }
                    crate::task::scheduler::debug_auto_suspend(
                        tid,
                        crate::task::scheduler::DEBUG_EVENT_SINGLE_STEP,
                        frame.rip,
                    );
                    crate::task::scheduler::schedule();
                    return;
                }
                if try_kill_faulting_thread(129, frame) {
                    return;
                }
                return;
            }
            // Clear DR6 and continue (single-step or breakpoint)
            unsafe {
                core::arch::asm!("xor {tmp}, {tmp}; mov dr6, {tmp}", tmp = out(reg) _, options(nostack));
            }
            return;
        }
        3 => {
            // #BP Breakpoint (INT3) — check for debug-attached thread (anyTrace)
            if is_user_mode {
                if crate::task::scheduler::is_debug_attached_current() {
                    // INT3 hit on a debug-attached thread.
                    // RIP points to the byte AFTER INT3 (INT3 = 1 byte opcode 0xCC).
                    // Adjust RIP back by 1 so the debugger sees the breakpoint address.
                    let bp_addr = frame.rip - 1;
                    let tid = crate::task::scheduler::debug_current_tid();
                    // Adjust the saved RIP in the interrupt frame so the thread resumes
                    // at the breakpoint address (after the debugger restores the original byte).
                    let frame_mut = frame as *const InterruptFrame as *mut InterruptFrame;
                    unsafe {
                        (*frame_mut).rip = bp_addr;
                    }
                    crate::task::scheduler::debug_auto_suspend(
                        tid,
                        crate::task::scheduler::DEBUG_EVENT_BREAKPOINT,
                        bp_addr,
                    );
                    crate::task::scheduler::schedule();
                    return;
                }
                // Not debug-attached — kill the thread (unexpected INT3)
                let dbg_tid = crate::task::scheduler::debug_current_tid();
                crate::serial_println!(
                    "EXCEPTION: Breakpoint (INT3) at RIP={:#018x} (TID={})",
                    frame.rip,
                    dbg_tid
                );
                if try_kill_faulting_thread(133, frame) {
                    return;
                }
                return;
            }
            // Kernel breakpoint — ignore (shouldn't normally happen)
            return;
        }
        6 => {
            let dbg_tid = crate::task::scheduler::debug_current_tid();
            if is_user_mode {
                crate::serial_println!(
                    "EXCEPTION: Invalid opcode at RIP={:#018x} CS={:#x} (TID={})",
                    frame.rip,
                    frame.cs,
                    dbg_tid
                );
                crate::serial_println!("  User RSP={:#018x}", frame.rsp);
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::crash_info::store_crash(dbg_tid, 132, frame);
                if try_kill_faulting_thread(132, frame) {
                    return;
                }
            }
            if try_kill_faulting_thread(132, frame) {
                return;
            }
            // Fatal kernel fault — enter panic mode to halt other CPUs
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!(
                "EXCEPTION: Invalid opcode at RIP={:#018x} CS={:#x} (debug_tid={})",
                frame.rip,
                frame.cs,
                dbg_tid
            );
            crate::serial_println!(
                "  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                frame.rax,
                frame.rbx,
                frame.rcx,
                frame.rdx
            );
            crate::serial_println!(
                "  RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}",
                frame.rsi,
                frame.rdi,
                frame.rbp,
                frame.rsp
            );
            crate::serial_println!(
                "  R8={:#018x}  R9={:#018x}  R10={:#018x} R11={:#018x}",
                frame.r8,
                frame.r9,
                frame.r10,
                frame.r11
            );
            crate::serial_println!(
                "  R12={:#018x} R13={:#018x} R14={:#018x} R15={:#018x}",
                frame.r12,
                frame.r13,
                frame.r14,
                frame.r15
            );
            // Last syscall diagnostics
            {
                let crash_cpu = crate::arch::x86::smp::current_cpu_id() as usize;
                let (last_sc, last_abi, last_name) = last_syscall_diag(crash_cpu);
                crate::serial_println!("  LastSC={}:{} ({})", last_abi, last_sc, last_name);
            }
            // Stack location diagnostics
            {
                let frame_addr = frame as *const InterruptFrame as u64;
                let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
                let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                crate::serial_println!("  Frame addr={:#018x} CPU={}", frame_addr, cpu_id);
                crate::serial_println!(
                    "  Expected stack=[{:#018x}..{:#018x}]",
                    stack_bottom,
                    stack_top
                );
                if stack_bottom != 0 && (frame_addr < stack_bottom || frame_addr > stack_top) {
                    crate::serial_println!("  CRITICAL: Frame is OUTSIDE kernel stack bounds!");
                }
            }
            // Dump stack (only if RSP is aligned and in kernel range)
            if frame.rsp >= 0xFFFF_FFFF_8000_0000
                && frame.rsp < 0xFFFF_FFFF_F000_0000
                && frame.rsp & 7 == 0
            {
                let stack_ptr = frame.rsp as *const u64;
                crate::serial_println!("  Stack dump (from RSP):");
                for i in 0..16 {
                    let val = unsafe { stack_ptr.add(i as usize).read_volatile() };
                    crate::serial_println!("    [RSP+{:#04x}] = {:#018x}", i * 8, val);
                }
            }
            // Walk RBP chain for stack trace
            crate::serial_println!("  RBP chain:");
            let mut bp = frame.rbp;
            for _ in 0..8 {
                if bp < 0xFFFF_FFFF_8000_0000 || bp > 0xFFFF_FFFF_D100_0000 {
                    break;
                }
                if bp & 7 != 0 {
                    break;
                } // Misaligned — corrupt frame pointer
                let ret_addr = unsafe { *((bp + 8) as *const u64) };
                let prev_bp = unsafe { *(bp as *const u64) };
                crate::serial_println!("    RBP={:#018x} RET={:#018x}", bp, ret_addr);
                bp = prev_bp;
            }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "Invalid Opcode (#UD)");
            loop {
                unsafe {
                    core::arch::asm!("cli; hlt");
                }
            }
        }
        7 => {
            // #NM — Device Not Available (CR0.TS set)
            // Lazy FPU switching: user thread touched FPU/SSE, restore its state
            if is_user_mode {
                crate::task::scheduler::handle_device_not_available();
                return; // Retry the faulting FPU/SSE instruction
            }
            // Kernel never uses FPU (soft-float) — #NM in kernel is a bug
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!(
                "EXCEPTION: #NM Device Not Available at RIP={:#018x} CS={:#x}",
                frame.rip,
                frame.cs
            );
            crate::serial_println!("  FATAL: unexpected #NM in kernel — halting");
            crate::drivers::rsod::show_exception(frame, "Device Not Available (#NM)");
            loop {
                unsafe {
                    core::arch::asm!("cli; hlt");
                }
            }
        }
        8 => {
            crate::drivers::serial::enter_panic_mode();
            let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
            let tss_rsp0 = crate::arch::x86::tss::get_kernel_stack_for_cpu(cpu_id);
            let percpu_krsp = crate::arch::x86::syscall_msr::get_kernel_rsp(cpu_id);
            crate::serial_println!(
                "EXCEPTION: Double fault! CPU={} TSS.RSP0={:#018x} PERCPU.krsp={:#018x}",
                cpu_id,
                tss_rsp0,
                percpu_krsp,
            );
            crate::serial_println!(
                "  frame.RSP={:#018x} frame.RIP={:#018x} frame.CS={:#06x}",
                frame.rsp,
                frame.rip,
                frame.cs,
            );
            crate::serial_println!("  FATAL: unrecoverable — halting");
            crate::drivers::rsod::show_exception(frame, "Double Fault (#DF)");
            loop {
                unsafe {
                    core::arch::asm!("cli; hlt");
                }
            }
        }
        13 => {
            if is_user_mode {
                let (saved_ds, saved_es) = unsafe { (SAVED_FAULT_DS, SAVED_FAULT_ES) };
                if frame.err_code == 0 && (saved_ds == 0x10 || saved_es == 0x10) {
                    crate::serial_println!(
                        "RECOVER: user #GP with kernel DS/ES leaked (DS={:#06x} ES={:#06x}) - retrying RIP={:#018x}",
                        saved_ds,
                        saved_es,
                        frame.rip
                    );
                    unsafe {
                        SAVED_FAULT_DS = 0x23;
                        SAVED_FAULT_ES = 0x23;
                    }
                    return;
                }
                crate::serial_println!(
                    "EXCEPTION: General Protection Fault err={:#x} RIP={:#018x} CS={:#x}",
                    frame.err_code,
                    frame.rip,
                    frame.cs
                );
                crate::serial_println!(
                    "  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                    frame.rax,
                    frame.rbx,
                    frame.rcx,
                    frame.rdx
                );
                crate::serial_println!(
                    "  RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}",
                    frame.rsi,
                    frame.rdi,
                    frame.rbp,
                    frame.rsp
                );
                crate::serial_println!(
                    "  R8={:#018x} R9={:#018x} R10={:#018x} R11={:#018x}",
                    frame.r8,
                    frame.r9,
                    frame.r10,
                    frame.r11
                );
                crate::serial_println!(
                    "  R12={:#018x} R13={:#018x} R14={:#018x} R15={:#018x}",
                    frame.r12,
                    frame.r13,
                    frame.r14,
                    frame.r15
                );
                // Print DS/ES captured by ISR stub BEFORE overwrite
                crate::serial_println!(
                    "  DS={:#06x}  ES={:#06x} (at fault time)",
                    saved_ds,
                    saved_es
                );
                if saved_ds == 0 {
                    crate::serial_println!("  !!! DS is NULL — this is the cause of the #GP(0)!");
                    crate::serial_println!(
                        "  DS was not restored to 0x23 before returning to user mode."
                    );
                }
                {
                    let crash_cpu = crate::arch::x86::smp::current_cpu_id() as usize;
                    let (last_sc, last_abi, last_name) = last_syscall_diag(crash_cpu);
                    crate::serial_println!("  LastSC: {}:{} ({})", last_abi, last_sc, last_name);
                }
                dump_iretq_frame(frame.rip, frame.rsp);
                crate::serial_println!("  User process fault — terminating thread");
                let gpf_tid = crate::task::scheduler::debug_current_tid();
                crate::task::crash_info::store_crash(gpf_tid, 139, frame);
                if try_kill_faulting_thread(139, frame) {
                    return;
                }
            }
            if try_kill_faulting_thread(139, frame) {
                return;
            }
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!(
                "EXCEPTION: General Protection Fault err={:#x} RIP={:#018x} CS={:#x}",
                frame.err_code,
                frame.rip,
                frame.cs
            );
            crate::serial_println!(
                "  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                frame.rax,
                frame.rbx,
                frame.rcx,
                frame.rdx
            );
            crate::serial_println!(
                "  RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}",
                frame.rsi,
                frame.rdi,
                frame.rbp,
                frame.rsp
            );
            {
                let (saved_ds, saved_es) = unsafe { (SAVED_FAULT_DS, SAVED_FAULT_ES) };
                crate::serial_println!(
                    "  DS={:#06x}  ES={:#06x} (at fault time)",
                    saved_ds,
                    saved_es
                );
            }
            {
                let crash_cpu = crate::arch::x86::smp::current_cpu_id() as usize;
                let (last_sc, last_abi, last_name) = last_syscall_diag(crash_cpu);
                crate::serial_println!("  LastSC={}:{} ({})", last_abi, last_sc, last_name);
            }
            // Stack location diagnostics
            {
                let frame_addr = frame as *const InterruptFrame as u64;
                let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
                let tid = crate::task::scheduler::debug_current_tid();
                let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                crate::serial_println!(
                    "  Frame addr={:#018x} CPU={} TID={}",
                    frame_addr,
                    cpu_id,
                    tid
                );
                crate::serial_println!(
                    "  Expected stack=[{:#018x}..{:#018x}]",
                    stack_bottom,
                    stack_top
                );
                if stack_bottom != 0 && (frame_addr < stack_bottom || frame_addr > stack_top) {
                    crate::serial_println!("  CRITICAL: Frame is OUTSIDE kernel stack bounds!");
                }
            }
            dump_iretq_frame(frame.rip, frame.rsp);
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "General Protection Fault (#GP)");
            loop {
                unsafe {
                    core::arch::asm!("cli; hlt");
                }
            }
        }
        14 => {
            let cr2: u64;
            unsafe {
                core::arch::asm!("mov {}, cr2", out(reg) cr2);
            }

            // Demand paging: if page not present and address is in committed heap range,
            // allocate a frame and map it transparently, then retry the instruction.
            let err_not_present = (frame.err_code & 1) == 0;
            if err_not_present {
                if crate::memory::virtual_mem::handle_heap_demand_page(cr2) {
                    return; // Page mapped — retry faulting instruction via iretq
                }
            }

            // Userspace mmap repair/demand paging: if the fault falls inside a
            // live VMA but the leaf PTE is missing, map a zero page and retry.
            if err_not_present && is_user_mode {
                if crate::memory::virtual_mem::handle_user_mmap_demand_page(cr2) {
                    return;
                }
            }

            // DLL demand paging: if an unmapped DLL page is accessed, map the
            // shared physical frame on-demand and retry the instruction.
            // This must also handle kernel-mode faults (e.g. sys_write reading
            // from a user buffer whose DLL page hasn't been faulted in yet).
            if err_not_present {
                let is_dll_range = crate::memory::user_vmap::fault_diag::DLL_RANGE.contains(&cr2);
                if (is_user_mode || is_dll_range) && crate::task::dll::handle_dll_demand_page(cr2) {
                    return; // DLL page mapped — retry faulting instruction via iretq
                }
            }

            if err_not_present && is_user_mode && try_handle_linux_vsyscall_fault(frame, cr2) {
                return;
            }

            // User-mode page fault: print diagnostics and kill the thread
            if is_user_mode {
                let tid = crate::task::scheduler::current_tid();
                // Detect stack guard page hit (stack overflow)
                let is_stack_area =
                    crate::memory::user_vmap::fault_diag::STACK_RANGE.contains(&cr2);
                if is_stack_area && err_not_present {
                    crate::serial_println!(
                        "USER STACK OVERFLOW! TID={} addr={:#018x} RIP={:#018x} — killing thread",
                        tid,
                        cr2,
                        frame.rip
                    );
                } else {
                    crate::serial_println!(
                        "EXCEPTION: Page Fault addr={:#018x} RIP={:#018x} err={:#x} TID={}",
                        cr2,
                        frame.rip,
                        frame.err_code,
                        tid
                    );
                }
                crate::serial_println!(
                    "  CS={:#x} RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                    frame.cs,
                    frame.rax,
                    frame.rbx,
                    frame.rcx,
                    frame.rdx
                );
                crate::serial_println!(
                    "  RSI={:#018x} RDI={:#018x} RBP={:#018x} R8={:#018x}",
                    frame.rsi,
                    frame.rdi,
                    frame.rbp,
                    frame.r8
                );
                crate::serial_println!("  User RSP={:#018x} SS={:#x}", frame.rsp, frame.ss);
                crate::serial_println!("  User process fault — terminating thread (TID={})", tid);
                crate::task::crash_info::store_crash(tid, 139, frame);
                if try_kill_faulting_thread(139, frame) {
                    return;
                }
            }
            if try_kill_faulting_thread(139, frame) {
                return;
            }

            // Detect kernel stack guard-page overflow before general diagnostics.
            if err_not_present {
                let page_va = crate::memory::address::VirtAddr::new(cr2 & !0xFFF);
                if crate::memory::virtual_mem::read_pte(page_va)
                    & crate::memory::virtual_mem::PTE_GUARD
                    != 0
                {
                    crate::drivers::serial::enter_panic_mode();
                    crate::serial_println!(
                        "KERNEL STACK OVERFLOW! guard page hit at {:#018x} RIP={:#018x} TID={}",
                        cr2,
                        frame.rip,
                        crate::task::scheduler::debug_current_tid()
                    );
                    crate::drivers::rsod::show_exception(
                        frame,
                        "Kernel Stack Overflow (guard page)",
                    );
                    loop {
                        unsafe {
                            core::arch::asm!("cli; hlt");
                        }
                    }
                }
            }

            // Fatal kernel page fault — enter panic mode (halt other CPUs)
            // so diagnostics are not interleaved with other crashes
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!(
                "EXCEPTION: Page Fault addr={:#018x} RIP={:#018x} err={:#x}",
                cr2,
                frame.rip,
                frame.err_code
            );
            crate::serial_println!(
                "  CS={:#x} RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                frame.cs,
                frame.rax,
                frame.rbx,
                frame.rcx,
                frame.rdx
            );
            crate::serial_println!(
                "  RSI={:#018x} RDI={:#018x} RBP={:#018x}",
                frame.rsi,
                frame.rdi,
                frame.rbp
            );
            crate::serial_println!(
                "  R8={:#018x}  R9={:#018x}  R10={:#018x} R11={:#018x}",
                frame.r8,
                frame.r9,
                frame.r10,
                frame.r11
            );
            crate::serial_println!(
                "  R12={:#018x} R13={:#018x} R14={:#018x} R15={:#018x}",
                frame.r12,
                frame.r13,
                frame.r14,
                frame.r15
            );

            // Corruption diagnostics: detect if the interrupt frame is corrupt
            let valid_cs = matches!(frame.cs, 0x08 | 0x1B | 0x23 | 0x2B);
            if !valid_cs {
                crate::serial_println!(
                    "  WARNING: CS={:#018x} is NOT a valid segment selector!",
                    frame.cs
                );
                crate::serial_println!(
                    "  This indicates the kernel stack was corrupted when the CPU"
                );
                crate::serial_println!(
                    "  pushed the exception frame (stack overflow into adjacent heap?)"
                );
            }

            // Stack location diagnostics
            {
                let frame_addr = frame as *const InterruptFrame as u64;
                let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
                let tid = crate::task::scheduler::debug_current_tid();
                let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                crate::serial_println!(
                    "  Frame addr={:#018x} CPU={} TID={}",
                    frame_addr,
                    cpu_id,
                    tid
                );
                crate::serial_println!(
                    "  Expected stack=[{:#018x}..{:#018x}]",
                    stack_bottom,
                    stack_top
                );
                if stack_bottom != 0 {
                    if frame_addr < stack_bottom || frame_addr > stack_top {
                        crate::serial_println!("  CRITICAL: Frame is OUTSIDE kernel stack bounds!");
                        crate::serial_println!("  This confirms stack overflow or use-after-free.");
                    } else {
                        let used = stack_top - frame_addr;
                        let total = stack_top - stack_bottom;
                        let pct = if total > 0 { used * 100 / total } else { 0 };
                        crate::serial_println!(
                            "  Stack usage: {} / {} bytes ({}%)",
                            used,
                            total,
                            pct
                        );
                    }
                }
            }

            // Print CR3 and page table indices (no recursive mapping dereference —
            // accessing recursive addresses causes recursive faults if PML4[510]
            // is not set up in the current CR3, e.g. during early boot or CR3 corruption).
            {
                let cr3_val: u64;
                unsafe {
                    core::arch::asm!("mov {}, cr3", out(reg) cr3_val);
                }
                let pml4i = ((cr2 >> 39) & 0x1FF) as usize;
                let pdpti = ((cr2 >> 30) & 0x1FF) as usize;
                let pdi = ((cr2 >> 21) & 0x1FF) as usize;
                let pti = ((cr2 >> 12) & 0x1FF) as usize;
                crate::serial_println!(
                    "  CR3={:#018x} PML4[{}] PDPT[{}] PD[{}] PT[{}]",
                    cr3_val,
                    pml4i,
                    pdpti,
                    pdi,
                    pti
                );
            }
            dump_iretq_frame(frame.rip, frame.rsp);
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "Page Fault (#PF)");
            loop {
                unsafe {
                    core::arch::asm!("cli; hlt");
                }
            }
        }
        16 => {
            // #MF — x87 Floating-Point Exception
            if is_user_mode {
                crate::serial_println!(
                    "EXCEPTION: #MF x87 FP Exception at RIP={:#018x} CS={:#x}",
                    frame.rip,
                    frame.cs
                );
                crate::serial_println!("  User process fault — terminating thread");
                let mf_tid = crate::task::scheduler::debug_current_tid();
                crate::task::crash_info::store_crash(mf_tid, 136, frame);
                if try_kill_faulting_thread(136, frame) {
                    return;
                }
            }
            if try_kill_faulting_thread(136, frame) {
                return;
            }
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!(
                "EXCEPTION: #MF x87 FP Exception at RIP={:#018x} CS={:#x}",
                frame.rip,
                frame.cs
            );
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "x87 FP Exception (#MF)");
            loop {
                unsafe {
                    core::arch::asm!("cli; hlt");
                }
            }
        }
        19 => {
            // #XM — SIMD Floating-Point Exception
            let mxcsr: u32;
            unsafe {
                let mut tmp = [0u32; 1];
                core::arch::asm!("stmxcsr [{}]", in(reg) tmp.as_mut_ptr(), options(nostack, preserves_flags));
                mxcsr = tmp[0];
            }
            if is_user_mode {
                crate::serial_println!(
                    "EXCEPTION: #XM SIMD FP Exception at RIP={:#018x} CS={:#x} MXCSR={:#010x}",
                    frame.rip,
                    frame.cs,
                    mxcsr
                );
                crate::serial_println!("  User process fault — terminating thread");
                let xm_tid = crate::task::scheduler::debug_current_tid();
                crate::task::crash_info::store_crash(xm_tid, 136, frame);
                if try_kill_faulting_thread(136, frame) {
                    return;
                }
            }
            if try_kill_faulting_thread(136, frame) {
                return;
            }
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!(
                "EXCEPTION: #XM SIMD FP Exception at RIP={:#018x} CS={:#x} MXCSR={:#010x}",
                frame.rip,
                frame.cs,
                mxcsr
            );
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "SIMD FP Exception (#XM)");
            loop {
                unsafe {
                    core::arch::asm!("cli; hlt");
                }
            }
        }
        _ => {
            crate::serial_println!(
                "Unhandled exception #{} at RIP={:#018x}",
                frame.int_no,
                frame.rip
            );
            if is_user_mode {
                let def_tid = crate::task::scheduler::debug_current_tid();
                crate::task::crash_info::store_crash(def_tid, (128 + frame.int_no) as u32, frame);
                crate::task::scheduler::exit_current((128 + frame.int_no) as u32);
            }
            if try_kill_faulting_thread((128 + frame.int_no) as u32, frame) {
                return;
            }
        }
    }
}

/// Hardware IRQ dispatcher called from assembly IRQ stubs.
///
/// Sends EOI (to APIC or PIC) before dispatching to the registered
/// handler, since handlers like the scheduler may context-switch and
/// never return.
#[no_mangle]
pub extern "C" fn irq_handler(frame: &InterruptFrame) {
    let irq = (frame.int_no - 32) as u8;

    // LAPIC timer (IRQ 16): per-tick safety checks + debug tick-rate output.
    if irq == 16 {
        // Use current_cpu_id() for correctness — it maps the hardware LAPIC ID
        // to the logical CPU index via CPU_DATA lookup. LAPIC IDs may not be
        // contiguous on all hypervisors (e.g., VirtualBox can assign 0,2,4,6).
        let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;

        // Periodic uptime summary every 60s (CPU 0 only, LAPIC at 1000Hz)
        if cpu_id == 0 {
            static LAPIC_TICK_CTR: AtomicU32 = AtomicU32::new(0);
            let ctr = LAPIC_TICK_CTR.fetch_add(1, Ordering::Relaxed) + 1;
            if ctr % 60_000 == 0 {
                let uptime_ms = crate::arch::x86::pit::get_ticks();
                let secs = uptime_ms / 1000;
                let mins = secs / 60;
                let hrs = mins / 60;
                serial_println!("uptime: {}h {}m {}s", hrs, mins % 60, secs % 60);
            }
            // Pet the hardware watchdog every ~1s (1024 ticks at 1000 Hz).
            if ctr % 1024 == 0 {
                crate::drivers::watchdog::pet();
            }
        }
        if cpu_id < 8 {
            // TSS.RSP0 corruption check on EVERY timer tick.
            // When a user thread is running on this CPU, TSS.RSP0 must point to
            // a valid kernel stack. The CPU reads RSP0 on any ring 3→0 interrupt
            // transition — if it's 0 or corrupt, the CPU loads RSP=0, pushes the
            // interrupt frame to near-zero, and we get an unrecoverable #DF.
            // Checking here catches corruption BEFORE the next user-mode interrupt.
            if crate::task::scheduler::cpu_has_active_thread(cpu_id) {
                let tss_rsp0 = crate::arch::x86::tss::get_kernel_stack_for_cpu(cpu_id);
                let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                // Check 1: RSP0 must be in kernel higher-half
                let rsp0_range_bad = tss_rsp0 == 0 || tss_rsp0 < 0xFFFF_FFFF_8000_0000;
                // Check 2: RSP0 must be within the current thread's stack bounds.
                // If RSP0 points to a different thread's freed stack, the next
                // ring 3→0 transition pushes the frame to wrong memory → garbled.
                let rsp0_bounds_bad = stack_bottom != 0
                    && stack_top != 0
                    && (tss_rsp0 < stack_bottom || tss_rsp0 > stack_top);
                if rsp0_range_bad || rsp0_bounds_bad {
                    let tid = crate::task::scheduler::debug_current_tid();
                    if rsp0_range_bad {
                        uart_puts(b"\n!!!TSS.RSP0 CORRUPT cpu=");
                    } else {
                        uart_puts(b"\n!!!TSS.RSP0 OUT-OF-BOUNDS cpu=");
                    }
                    uart_put_dec(cpu_id as u32);
                    uart_puts(b" rsp0=");
                    uart_put_hex(tss_rsp0);
                    uart_puts(b" tid=");
                    uart_put_dec(tid);
                    uart_puts(b" stack=[");
                    uart_put_hex(stack_bottom);
                    uart_puts(b"..");
                    uart_put_hex(stack_top);
                    uart_puts(b"]\n");
                    // Attempt repair: use the per-CPU stack bounds (set by scheduler)
                    if stack_top >= 0xFFFF_FFFF_8000_0000 {
                        crate::arch::x86::tss::set_kernel_stack_for_cpu(cpu_id, stack_top);
                        crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, stack_top);
                        uart_puts(b"  REPAIRED RSP0=");
                        uart_put_hex(stack_top);
                        uart_putc(b'\n');
                    }
                }
            }

            // PERCPU.kernel_rsp corruption check (SYSCALL path uses this, NOT TSS.RSP0).
            // The SYSCALL fast entry loads RSP from [gs:0] = PERCPU.kernel_rsp.
            // If this is corrupt but TSS.RSP0 is fine, INT 0x80 works but SYSCALL crashes.
            // This closes the blind spot where set_kernel_rsp had no validation.
            if crate::task::scheduler::cpu_has_active_thread(cpu_id) {
                let percpu_rsp = crate::arch::x86::syscall_msr::get_kernel_rsp(cpu_id);
                if percpu_rsp != 0 && percpu_rsp < 0xFFFF_FFFF_8000_0000 {
                    let tid = crate::task::scheduler::debug_current_tid();
                    uart_puts(b"\n!!!PERCPU.kernel_rsp CORRUPT cpu=");
                    uart_put_dec(cpu_id as u32);
                    uart_puts(b" rsp=");
                    uart_put_hex(percpu_rsp);
                    uart_puts(b" tid=");
                    uart_put_dec(tid);
                    uart_putc(b'\n');
                    // Repair from per-CPU stack bounds
                    let (_, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                    if stack_top >= 0xFFFF_FFFF_8000_0000 {
                        crate::arch::x86::syscall_msr::set_kernel_rsp(cpu_id, stack_top);
                        uart_puts(b"  REPAIRED PERCPU.kernel_rsp=");
                        uart_put_hex(stack_top);
                        uart_putc(b'\n');
                    }
                }
            }

            // Stack canary check: detect kernel stack overflow on every timer tick.
            // The canary at the bottom of the current thread's stack is the earliest
            // indicator of overflow — fires before the overflow corrupts adjacent
            // heap memory (which causes the garbled InterruptFrame crashes).
            if crate::task::scheduler::cpu_has_active_thread(cpu_id) {
                let (stack_bottom, _) = crate::task::scheduler::get_stack_bounds(cpu_id);
                if stack_bottom >= 0xFFFF_FFFF_8000_0000 {
                    let canary = unsafe { *(stack_bottom as *const u64) };
                    if canary != crate::task::thread::STACK_CANARY {
                        let tid = crate::task::scheduler::debug_current_tid();
                        uart_puts(b"\n!!!STACK CANARY DEAD cpu=");
                        uart_put_dec(cpu_id as u32);
                        uart_puts(b" tid=");
                        uart_put_dec(tid);
                        uart_puts(b" canary=");
                        uart_put_hex(canary);
                        uart_putc(b'\n');
                        // Kill the thread to prevent further corruption
                        crate::task::scheduler::try_exit_current(139);
                    }
                }
            }

            // Historical note: a `refresh_kernel_gs_base()` call lived here
            // to compensate for QEMU TCG MSR leaks. Under the old invariant
            // (swapgs back to user inside the syscall handler, Phase 3) the
            // call was harmless: during kernel residency KERNEL_GS_BASE was
            // supposed to equal PERCPU, so overwriting it was a no-op.
            //
            // Post-commit 241b1475 the invariant flipped — GS.base stays at
            // PERCPU for the whole kernel residency and KERNEL_GS_BASE holds
            // the user's saved GS (typically 0). A refresh in that state
            // would clobber KERNEL_GS_BASE with PERCPU, making the final
            // Phase 6 `swapgs` a no-op and leaking GS.base=PERCPU to ring 3.
            // MSR-leak recovery now lives in the Phase 1b LAPIC-ID check
            // inside syscall_fast.asm, which is immune to this ambiguity.
        }
    }

    // Real-time stack overflow detection: check if the interrupted RSP is
    // within the current thread's kernel stack bounds. This catches overflows
    // BEFORE the stack corrupts adjacent heap memory (which causes mysterious
    // crashes with corrupted interrupt frames — CS/RIP contain heap data).
    // Only check when a kernel thread is active (not idle, not user mode).
    // Skip when CPU is idle (PER_CPU_HAS_THREAD=false) — the idle context
    // runs on the boot stack, not a heap-allocated thread stack.
    let is_kernel_thread = frame.cs & 3 == 0;
    let mut stack_overflow = false;
    if is_kernel_thread {
        let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
        if crate::task::scheduler::cpu_has_active_thread(cpu_id) {
            if !crate::task::scheduler::check_rsp_in_bounds(cpu_id, frame.rsp) {
                let (bottom, top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                let stack_size = top.wrapping_sub(bottom);
                // Only flag as real overflow if RSP is within one stack-size below
                // the bottom. If RSP is much further away (e.g., on the AP idle
                // stack after context_switch), the check is a false positive —
                // the CPU is running on a different stack, not an overflowed one.
                let delta_below = bottom.wrapping_sub(frame.rsp);
                if delta_below <= stack_size && frame.rsp < bottom {
                    let tid = crate::task::scheduler::debug_current_tid();
                    let tss_rsp0 = crate::arch::x86::tss::get_kernel_stack_for_cpu(cpu_id);
                    let percpu_krsp = crate::arch::x86::syscall_msr::get_kernel_rsp(cpu_id);
                    crate::serial_println!(
                        "STACK OVERFLOW in IRQ! CPU{} TID={} RSP={:#018x} stack=[{:#018x}..{:#018x}]",
                        cpu_id, tid, frame.rsp, bottom, top,
                    );
                    crate::serial_println!(
                        "  TSS.RSP0={:#018x} PERCPU[{}].krsp={:#018x} delta={}",
                        tss_rsp0,
                        cpu_id,
                        percpu_krsp,
                        bottom.wrapping_sub(frame.rsp),
                    );
                    stack_overflow = true;
                    crate::task::scheduler::try_exit_current(139);
                }
            }
        }
    }

    // Send EOI before dispatch — handlers (e.g. scheduler) may context_switch
    // and never return, so EOI must be sent first to allow further interrupts.
    if crate::arch::x86::apic::is_initialized() {
        crate::arch::x86::apic::eoi();
    } else {
        crate::arch::x86::pic::send_eoi(irq);
    }

    // CRITICAL: Skip dispatch when stack overflow detected. Continuing with
    // schedule() on a corrupted stack would write past the allocation boundary
    // and corrupt adjacent heap memory (other threads' contexts, page tables, etc).
    if stack_overflow {
        return;
    }

    // PCI MSI uses the CPU vector directly, not the legacy IRQ line number.
    if crate::drivers::pci_msi::dispatch_msi(frame.int_no as u8) {
        return;
    }

    // Dispatch to dynamically registered legacy/APIC IRQ handler.
    crate::arch::x86::irq::dispatch_irq(irq);
}
