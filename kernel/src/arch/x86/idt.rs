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
///
/// Uses LOCK-FREE scheduler queries (debug_current_tid, debug_is_current_user)
/// to prevent deadlock: a page fault could fire while the scheduler lock is held
/// (e.g., demand paging on heap memory accessed from schedule_inner). Using
/// SCHEDULER.lock() in that scenario would deadlock on a single CPU with IF=0.
///
/// Also uses try_exit_current (try_lock) instead of exit_current to avoid
/// deadlock when the scheduler lock is already held by this CPU.
fn try_kill_faulting_thread(signal: u32) -> bool {
    let tid = crate::task::scheduler::debug_current_tid();
    // TID 0 = idle context
    if tid == 0 {
        return false; // Can't kill idle context
    }
    if crate::task::scheduler::debug_is_current_user() {
        crate::serial_println!("  Killing faulting user thread (TID={}, signal={})", tid, signal);
        // Use try_exit to avoid deadlock if scheduler lock is held by this CPU
        if crate::task::scheduler::try_exit_current(signal) {
            // try_exit_current calls schedule() and never returns on success
            unreachable!();
        }
        // Couldn't acquire scheduler lock — can't kill, fall through to kernel panic
        crate::serial_println!("  WARNING: scheduler lock held — cannot kill thread");
        return false;
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
    let is_user_mode = frame.cs & 3 != 0;

    match frame.int_no {
        0 => {
            let dbg_tid = crate::task::scheduler::debug_current_tid();
            if is_user_mode {
                crate::serial_println!("EXCEPTION: Division by zero at RIP={:#018x} CS={:#x} (TID={})", frame.rip, frame.cs, dbg_tid);
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(136);
            }
            if try_kill_faulting_thread(136) { return; }
            // Fatal kernel fault — enter panic mode to halt other CPUs
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!("EXCEPTION: Division by zero at RIP={:#018x} CS={:#x} (TID={})", frame.rip, frame.cs, dbg_tid);
            crate::serial_println!(
                "  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                frame.rax, frame.rbx, frame.rcx, frame.rdx
            );
            crate::serial_println!(
                "  RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}",
                frame.rsi, frame.rdi, frame.rbp, frame.rsp
            );
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "Division by Zero (#DE)");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        6 => {
            let dbg_tid = crate::task::scheduler::debug_current_tid();
            if is_user_mode {
                crate::serial_println!("EXCEPTION: Invalid opcode at RIP={:#018x} CS={:#x} (TID={})", frame.rip, frame.cs, dbg_tid);
                crate::serial_println!("  User RSP={:#018x}", frame.rsp);
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(132);
            }
            if try_kill_faulting_thread(132) { return; }
            // Fatal kernel fault — enter panic mode to halt other CPUs
            crate::drivers::serial::enter_panic_mode();
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
            // Stack location diagnostics
            {
                let frame_addr = frame as *const InterruptFrame as u64;
                let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
                let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                crate::serial_println!("  Frame addr={:#018x} CPU={}", frame_addr, cpu_id);
                crate::serial_println!("  Expected stack=[{:#018x}..{:#018x}]", stack_bottom, stack_top);
                if stack_bottom != 0 && (frame_addr < stack_bottom || frame_addr > stack_top) {
                    crate::serial_println!("  CRITICAL: Frame is OUTSIDE kernel stack bounds!");
                }
            }
            // Dump stack (only if RSP is aligned and in kernel range)
            if frame.rsp >= 0xFFFF_FFFF_8000_0000 && frame.rsp < 0xFFFF_FFFF_F000_0000 && frame.rsp & 7 == 0 {
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
                if bp < 0xFFFF_FFFF_8000_0000 || bp > 0xFFFF_FFFF_D100_0000 { break; }
                if bp & 7 != 0 { break; } // Misaligned — corrupt frame pointer
                let ret_addr = unsafe { *((bp + 8) as *const u64) };
                let prev_bp = unsafe { *(bp as *const u64) };
                crate::serial_println!("    RBP={:#018x} RET={:#018x}", bp, ret_addr);
                bp = prev_bp;
            }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "Invalid Opcode (#UD)");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        7 => {
            // #NM — Device Not Available (CR0.TS set or no FPU)
            if is_user_mode {
                crate::serial_println!("EXCEPTION: #NM Device Not Available at RIP={:#018x} CS={:#x}", frame.rip, frame.cs);
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(135);
            }
            if try_kill_faulting_thread(135) { return; }
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!("EXCEPTION: #NM Device Not Available at RIP={:#018x} CS={:#x}", frame.rip, frame.cs);
            crate::serial_println!("  FATAL: unexpected #NM in kernel — halting");
            crate::drivers::rsod::show_exception(frame, "Device Not Available (#NM)");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        8 => {
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!("EXCEPTION: Double fault!");
            crate::serial_println!("  FATAL: unrecoverable — halting");
            crate::drivers::rsod::show_exception(frame, "Double Fault (#DF)");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        13 => {
            if is_user_mode {
                crate::serial_println!(
                    "EXCEPTION: General Protection Fault err={:#x} RIP={:#018x} CS={:#x}",
                    frame.err_code, frame.rip, frame.cs
                );
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(139);
            }
            if try_kill_faulting_thread(139) { return; }
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!(
                "EXCEPTION: General Protection Fault err={:#x} RIP={:#018x} CS={:#x}",
                frame.err_code, frame.rip, frame.cs
            );
            crate::serial_println!(
                "  RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                frame.rax, frame.rbx, frame.rcx, frame.rdx
            );
            crate::serial_println!(
                "  RSI={:#018x} RDI={:#018x} RBP={:#018x} RSP={:#018x}",
                frame.rsi, frame.rdi, frame.rbp, frame.rsp
            );
            // Stack location diagnostics
            {
                let frame_addr = frame as *const InterruptFrame as u64;
                let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
                let tid = crate::task::scheduler::debug_current_tid();
                let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                crate::serial_println!("  Frame addr={:#018x} CPU={} TID={}", frame_addr, cpu_id, tid);
                crate::serial_println!("  Expected stack=[{:#018x}..{:#018x}]", stack_bottom, stack_top);
                if stack_bottom != 0 && (frame_addr < stack_bottom || frame_addr > stack_top) {
                    crate::serial_println!("  CRITICAL: Frame is OUTSIDE kernel stack bounds!");
                }
            }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "General Protection Fault (#GP)");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        14 => {
            let cr2: u64;
            unsafe { core::arch::asm!("mov {}, cr2", out(reg) cr2); }

            // Demand paging: if page not present and address is in committed heap range,
            // allocate a frame and map it transparently, then retry the instruction.
            let err_not_present = (frame.err_code & 1) == 0;
            if err_not_present {
                if crate::memory::virtual_mem::handle_heap_demand_page(cr2) {
                    return; // Page mapped — retry faulting instruction via iretq
                }
            }

            // DLL demand paging: if a user process accesses an unmapped DLL page,
            // map the shared physical frame on-demand and retry the instruction.
            if err_not_present && is_user_mode {
                if crate::task::dll::handle_dll_demand_page(cr2) {
                    return; // DLL page mapped — retry faulting instruction via iretq
                }
            }

            // User-mode page fault: print diagnostics and kill the thread
            if is_user_mode {
                crate::serial_println!(
                    "EXCEPTION: Page Fault addr={:#018x} RIP={:#018x} err={:#x}",
                    cr2, frame.rip, frame.err_code
                );
                crate::serial_println!(
                    "  CS={:#x} RAX={:#018x} RBX={:#018x} RCX={:#018x} RDX={:#018x}",
                    frame.cs, frame.rax, frame.rbx, frame.rcx, frame.rdx
                );
                crate::serial_println!("  User RSP={:#018x} SS={:#x}", frame.rsp, frame.ss);
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(139);
            }
            if try_kill_faulting_thread(139) { return; }

            // Fatal kernel page fault — enter panic mode (halt other CPUs)
            // so diagnostics are not interleaved with other crashes
            crate::drivers::serial::enter_panic_mode();
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
            crate::serial_println!(
                "  R8={:#018x}  R9={:#018x}  R10={:#018x} R11={:#018x}",
                frame.r8, frame.r9, frame.r10, frame.r11
            );
            crate::serial_println!(
                "  R12={:#018x} R13={:#018x} R14={:#018x} R15={:#018x}",
                frame.r12, frame.r13, frame.r14, frame.r15
            );

            // Corruption diagnostics: detect if the interrupt frame is corrupt
            let valid_cs = matches!(frame.cs, 0x08 | 0x1B | 0x23 | 0x2B);
            if !valid_cs {
                crate::serial_println!("  WARNING: CS={:#018x} is NOT a valid segment selector!", frame.cs);
                crate::serial_println!("  This indicates the kernel stack was corrupted when the CPU");
                crate::serial_println!("  pushed the exception frame (stack overflow into adjacent heap?)");
            }

            // Stack location diagnostics
            {
                let frame_addr = frame as *const InterruptFrame as u64;
                let cpu_id = crate::arch::x86::smp::current_cpu_id() as usize;
                let tid = crate::task::scheduler::debug_current_tid();
                let (stack_bottom, stack_top) = crate::task::scheduler::get_stack_bounds(cpu_id);
                crate::serial_println!("  Frame addr={:#018x} CPU={} TID={}", frame_addr, cpu_id, tid);
                crate::serial_println!("  Expected stack=[{:#018x}..{:#018x}]", stack_bottom, stack_top);
                if stack_bottom != 0 {
                    if frame_addr < stack_bottom || frame_addr > stack_top {
                        crate::serial_println!("  CRITICAL: Frame is OUTSIDE kernel stack bounds!");
                        crate::serial_println!("  This confirms stack overflow or use-after-free.");
                    } else {
                        let used = stack_top - frame_addr;
                        let total = stack_top - stack_bottom;
                        let pct = if total > 0 { used * 100 / total } else { 0 };
                        crate::serial_println!("  Stack usage: {} / {} bytes ({}%)",
                            used, total, pct);
                    }
                }
            }

            // Print CR3 and page table indices (no recursive mapping dereference —
            // accessing recursive addresses causes recursive faults if PML4[510]
            // is not set up in the current CR3, e.g. during early boot or CR3 corruption).
            {
                let cr3_val: u64;
                unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3_val); }
                let pml4i = ((cr2 >> 39) & 0x1FF) as usize;
                let pdpti = ((cr2 >> 30) & 0x1FF) as usize;
                let pdi   = ((cr2 >> 21) & 0x1FF) as usize;
                let pti   = ((cr2 >> 12) & 0x1FF) as usize;
                crate::serial_println!("  CR3={:#018x} PML4[{}] PDPT[{}] PD[{}] PT[{}]",
                    cr3_val, pml4i, pdpti, pdi, pti);
            }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "Page Fault (#PF)");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        16 => {
            // #MF — x87 Floating-Point Exception
            if is_user_mode {
                crate::serial_println!("EXCEPTION: #MF x87 FP Exception at RIP={:#018x} CS={:#x}", frame.rip, frame.cs);
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(136);
            }
            if try_kill_faulting_thread(136) { return; }
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!("EXCEPTION: #MF x87 FP Exception at RIP={:#018x} CS={:#x}", frame.rip, frame.cs);
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "x87 FP Exception (#MF)");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
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
                    frame.rip, frame.cs, mxcsr
                );
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(136);
            }
            if try_kill_faulting_thread(136) { return; }
            crate::drivers::serial::enter_panic_mode();
            crate::serial_println!(
                "EXCEPTION: #XM SIMD FP Exception at RIP={:#018x} CS={:#x} MXCSR={:#010x}",
                frame.rip, frame.cs, mxcsr
            );
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            crate::drivers::rsod::show_exception(frame, "SIMD FP Exception (#XM)");
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
                        tss_rsp0, cpu_id, percpu_krsp, bottom.wrapping_sub(frame.rsp),
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

    // Dispatch to dynamically registered handler
    crate::arch::x86::irq::dispatch_irq(irq);
}
