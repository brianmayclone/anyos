//! Interrupt Descriptor Table (IDT) for x86-64 long mode.
//!
//! Sets up 256 entries: CPU exceptions (ISR 0-31), hardware IRQs remapped
//! to INT 32-55, and the `int 0x80` syscall trap gate (DPL 3).

use core::arch::asm;
use core::mem::size_of;

/// Total IDT entries (covers the full x86 interrupt vector range).
const IDT_ENTRIES: usize = 256;
/// GDT selector for Ring 0 code segment.
const KERNEL_CODE_SEG: u16 = 0x08;

/// x86-64 IDT entry (16 bytes).
#[repr(C, packed)]
#[derive(Copy, Clone)]
struct IdtEntry {
    offset_low: u16,     // Handler address bits 0-15
    selector: u16,       // Kernel code segment selector
    ist: u8,             // IST index (bits 0-2), zero (bits 3-7)
    type_attr: u8,       // Gate type and attributes
    offset_mid: u16,     // Handler address bits 16-31
    offset_high: u32,    // Handler address bits 32-63
    _reserved: u32,      // Must be zero
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
const GATE_TRAP: u8 = 0x8F;      // Present, DPL=0, 64-bit trap gate
const GATE_TRAP_DPL3: u8 = 0xEF; // Present, DPL=3, 64-bit trap gate (for syscalls)

fn set_gate(num: usize, handler: unsafe extern "C" fn(), selector: u16, type_attr: u8) {
    let handler = handler as *const () as u64;
    unsafe {
        IDT[num] = IdtEntry {
            offset_low: (handler & 0xFFFF) as u16,
            selector,
            ist: 0,
            type_attr,
            offset_mid: ((handler >> 16) & 0xFFFF) as u16,
            offset_high: ((handler >> 32) & 0xFFFFFFFF) as u32,
            _reserved: 0,
        };
    }
}

// External ISR/IRQ stubs from interrupts.asm
extern "C" {
    fn isr0();  fn isr1();  fn isr2();  fn isr3();
    fn isr4();  fn isr5();  fn isr6();  fn isr7();
    fn isr8();  fn isr9();  fn isr10(); fn isr11();
    fn isr12(); fn isr13(); fn isr14(); fn isr15();
    fn isr16(); fn isr17(); fn isr18(); fn isr19();
    fn isr20(); fn isr21(); fn isr22(); fn isr23();
    fn isr24(); fn isr25(); fn isr26(); fn isr27();
    fn isr28(); fn isr29(); fn isr30(); fn isr31();

    fn irq0();  fn irq1();  fn irq2();  fn irq3();
    fn irq4();  fn irq5();  fn irq6();  fn irq7();
    fn irq8();  fn irq9();  fn irq10(); fn irq11();
    fn irq12(); fn irq13(); fn irq14(); fn irq15();
    // LAPIC / APIC vectors
    fn irq16(); fn irq17(); fn irq18(); fn irq19();
    fn irq20(); fn irq21(); fn irq22(); fn irq23();

    fn syscall_entry();
}

/// Populate the IDT with exception, IRQ, and syscall gates, then load via `lidt`.
pub fn init() {
    // CPU Exceptions (ISR 0-31)
    set_gate(0,  isr0 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(1,  isr1 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(2,  isr2 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(3,  isr3 , KERNEL_CODE_SEG, GATE_TRAP);
    set_gate(4,  isr4 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(5,  isr5 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(6,  isr6 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(7,  isr7 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(8,  isr8 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(9,  isr9 , KERNEL_CODE_SEG, GATE_INTERRUPT);
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
    set_gate(32, irq0 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(33, irq1 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(34, irq2 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(35, irq3 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(36, irq4 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(37, irq5 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(38, irq6 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(39, irq7 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(40, irq8 , KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(41, irq9 , KERNEL_CODE_SEG, GATE_INTERRUPT);
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
    set_gate(54, irq22, KERNEL_CODE_SEG, GATE_INTERRUPT);
    set_gate(55, irq23, KERNEL_CODE_SEG, GATE_INTERRUPT); // Spurious

    // Syscall: int 0x80 - DPL=3 trap gate so Ring 3 code can invoke it
    set_gate(0x80, syscall_entry, KERNEL_CODE_SEG, GATE_TRAP_DPL3);

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

/// Try to recover from a fault by killing the current thread.
/// Returns true if the thread was killed (never actually returns to caller
/// because exit_current switches context). Returns false if unrecoverable.
fn try_kill_faulting_thread(signal: u32) -> bool {
    let tid = crate::task::scheduler::current_tid();
    // TID 0 = idle context, TID 1-2 are typically desktop/critical threads
    if tid == 0 {
        return false; // Can't kill idle context
    }
    if crate::task::scheduler::is_current_thread_user() {
        let name = crate::task::scheduler::current_thread_name();
        let name_len = name.iter().position(|&b| b == 0).unwrap_or(32);
        let name_str = core::str::from_utf8(&name[..name_len]).unwrap_or("???");
        crate::serial_println!("  Killing faulting thread '{}' (TID={}, signal={})", name_str, tid, signal);
        crate::task::scheduler::exit_current(signal);
        // exit_current never returns
    }
    false
}

/// High-level CPU exception handler called from assembly ISR stubs.
///
/// Handles division-by-zero, invalid opcode, double fault, GPF, and page
/// faults. For user-mode faults the offending thread is terminated; for
/// kernel faults the CPU is halted after diagnostic output.
#[no_mangle]
pub extern "C" fn isr_handler(frame: &InterruptFrame) {
    let is_user_mode = frame.cs & 3 != 0;

    match frame.int_no {
        0 => {
            crate::serial_println!("EXCEPTION: Division by zero at RIP={:#018x} CS={:#x}", frame.rip, frame.cs);
            if is_user_mode {
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(136);
            }
            if try_kill_faulting_thread(136) { return; }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        6 => {
            let dbg_tid = crate::task::scheduler::debug_current_tid();
            crate::serial_println!("EXCEPTION: Invalid opcode at RIP={:#018x} CS={:#x} (debug_tid={})", frame.rip, frame.cs, dbg_tid);
            crate::serial_println!(
                "  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                frame.rax, frame.rbx, frame.rcx, frame.rdx
            );
            crate::serial_println!(
                "  RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}",
                frame.rsi, frame.rdi, frame.rbp, frame.rsp
            );
            crate::serial_println!(
                "  R8={:#018x}  R9={:#018x}  R10={:#018x} R11={:#018x}",
                frame.r8, frame.r9, frame.r10, frame.r11
            );
            crate::serial_println!(
                "  R12={:#018x} R13={:#018x} R14={:#018x} R15={:#018x}",
                frame.r12, frame.r13, frame.r14, frame.r15
            );
            // Dump stack
            let stack_ptr = frame.rsp as *const u64;
            crate::serial_println!("  Stack dump (from RSP):");
            for i in 0..16 {
                let val = unsafe { stack_ptr.add(i as usize).read_volatile() };
                crate::serial_println!("    [RSP+{:#04x}] = {:#018x}", i * 8, val);
            }
            // Walk RBP chain for stack trace
            crate::serial_println!("  RBP chain:");
            let mut bp = frame.rbp;
            for _ in 0..8 {
                if bp < 0xFFFF_FFFF_8000_0000 || bp > 0xFFFF_FFFF_D100_0000 { break; }
                let ret_addr = unsafe { *((bp + 8) as *const u64) };
                let prev_bp = unsafe { *(bp as *const u64) };
                crate::serial_println!("    RBP={:#018x} RET={:#018x}", bp, ret_addr);
                bp = prev_bp;
            }
            if is_user_mode {
                crate::serial_println!("  User RSP={:#018x}", frame.rsp);
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(132);
            }
            if try_kill_faulting_thread(132) { return; }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        8 => {
            crate::serial_println!("EXCEPTION: Double fault!");
            crate::serial_println!("  FATAL: unrecoverable — halting");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        13 => {
            crate::serial_println!(
                "EXCEPTION: General Protection Fault err={:#x} RIP={:#018x} CS={:#x}",
                frame.err_code, frame.rip, frame.cs
            );
            if is_user_mode {
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(139);
            }
            if try_kill_faulting_thread(139) { return; }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        14 => {
            let cr2: u64;
            unsafe { core::arch::asm!("mov {}, cr2", out(reg) cr2); }
            crate::serial_println!(
                "EXCEPTION: Page Fault addr={:#018x} RIP={:#018x} err={:#x}",
                cr2, frame.rip, frame.err_code
            );
            crate::serial_println!(
                "  CS={:#x} RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                frame.cs, frame.rax, frame.rbx, frame.rcx, frame.rdx
            );
            crate::serial_println!(
                "  RSI={:#018x} RDI={:#018x} RBP={:#018x}",
                frame.rsi, frame.rdi, frame.rbp
            );
            if is_user_mode {
                crate::serial_println!("  User RSP={:#018x} SS={:#x}", frame.rsp, frame.ss);
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(139);
            }
            if try_kill_faulting_thread(139) { return; }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        _ => {
            crate::serial_println!("Unhandled exception #{} at RIP={:#018x}", frame.int_no, frame.rip);
            if is_user_mode {
                crate::task::scheduler::exit_current((128 + frame.int_no) as u32);
            }
            if try_kill_faulting_thread((128 + frame.int_no) as u32) { return; }
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

    // Send EOI before dispatch — handlers (e.g. scheduler) may context_switch
    // and never return, so EOI must be sent first to allow further interrupts.
    if crate::arch::x86::apic::is_initialized() {
        crate::arch::x86::apic::eoi();
    } else {
        crate::arch::x86::pic::send_eoi(irq);
    }

    // Dispatch to dynamically registered handler
    crate::arch::x86::irq::dispatch_irq(irq);
}
