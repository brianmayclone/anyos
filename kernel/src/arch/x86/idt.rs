use core::arch::asm;
use core::mem::size_of;

const IDT_ENTRIES: usize = 256;
const KERNEL_CODE_SEG: u16 = 0x08;

#[repr(C, packed)]
#[derive(Copy, Clone)]
struct IdtEntry {
    offset_low: u16,
    selector: u16,
    zero: u8,
    type_attr: u8,
    offset_high: u16,
}

#[repr(C, packed)]
struct IdtDescriptor {
    size: u16,
    offset: u32,
}

static mut IDT: [IdtEntry; IDT_ENTRIES] = [IdtEntry {
    offset_low: 0,
    selector: 0,
    zero: 0,
    type_attr: 0,
    offset_high: 0,
}; IDT_ENTRIES];

static mut IDT_DESC: IdtDescriptor = IdtDescriptor { size: 0, offset: 0 };

// Type attributes
const GATE_INTERRUPT_32: u8 = 0x8E; // Present, DPL=0, 32-bit interrupt gate
const GATE_TRAP_32: u8 = 0x8F;      // Present, DPL=0, 32-bit trap gate
const GATE_TRAP_32_DPL3: u8 = 0xEF; // Present, DPL=3, 32-bit trap gate (for syscalls)

fn set_gate(num: usize, handler: unsafe extern "C" fn(), selector: u16, type_attr: u8) {
    let handler = handler as *const () as u32;
    unsafe {
        IDT[num] = IdtEntry {
            offset_low: (handler & 0xFFFF) as u16,
            selector,
            zero: 0,
            type_attr,
            offset_high: ((handler >> 16) & 0xFFFF) as u16,
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

pub fn init() {
    // CPU Exceptions (ISR 0-31)
    set_gate(0,  isr0 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(1,  isr1 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(2,  isr2 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(3,  isr3 , KERNEL_CODE_SEG, GATE_TRAP_32);
    set_gate(4,  isr4 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(5,  isr5 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(6,  isr6 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(7,  isr7 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(8,  isr8 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(9,  isr9 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(10, isr10, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(11, isr11, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(12, isr12, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(13, isr13, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(14, isr14, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(15, isr15, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(16, isr16, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(17, isr17, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(18, isr18, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(19, isr19, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(20, isr20, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(21, isr21, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(22, isr22, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(23, isr23, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(24, isr24, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(25, isr25, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(26, isr26, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(27, isr27, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(28, isr28, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(29, isr29, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(30, isr30, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(31, isr31, KERNEL_CODE_SEG, GATE_INTERRUPT_32);

    // Hardware IRQs (remapped to INT 32-47)
    set_gate(32, irq0 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(33, irq1 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(34, irq2 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(35, irq3 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(36, irq4 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(37, irq5 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(38, irq6 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(39, irq7 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(40, irq8 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(41, irq9 , KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(42, irq10, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(43, irq11, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(44, irq12, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(45, irq13, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(46, irq14, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(47, irq15, KERNEL_CODE_SEG, GATE_INTERRUPT_32);

    // LAPIC / APIC vectors (INT 48-55)
    set_gate(48, irq16, KERNEL_CODE_SEG, GATE_INTERRUPT_32); // LAPIC Timer
    set_gate(49, irq17, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(50, irq18, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(51, irq19, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(52, irq20, KERNEL_CODE_SEG, GATE_INTERRUPT_32); // IPI: TLB
    set_gate(53, irq21, KERNEL_CODE_SEG, GATE_INTERRUPT_32); // IPI: Halt
    set_gate(54, irq22, KERNEL_CODE_SEG, GATE_INTERRUPT_32);
    set_gate(55, irq23, KERNEL_CODE_SEG, GATE_INTERRUPT_32); // Spurious

    // Syscall: int 0x80 - DPL=3 trap gate so Ring 3 code can invoke it
    set_gate(0x80, syscall_entry, KERNEL_CODE_SEG, GATE_TRAP_32_DPL3);

    // Load IDT
    unsafe {
        IDT_DESC = IdtDescriptor {
            size: (IDT_ENTRIES * size_of::<IdtEntry>() - 1) as u16,
            offset: (&raw const IDT) as *const _ as u32,
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

/// Interrupt stack frame passed from assembly stubs
#[repr(C)]
pub struct InterruptFrame {
    // Pushed by our stub (push gs/fs/es/ds)
    pub gs: u32,
    pub fs: u32,
    pub es: u32,
    pub ds: u32,
    // Pushed by pushad
    pub edi: u32,
    pub esi: u32,
    pub ebp: u32,
    pub esp: u32, // ESP before pushad
    pub ebx: u32,
    pub edx: u32,
    pub ecx: u32,
    pub eax: u32,
    // Pushed by our stub
    pub int_no: u32,
    pub err_code: u32,
    // Pushed by CPU on interrupt
    pub eip: u32,
    pub cs: u32,
    pub eflags: u32,
}

/// Try to recover from a fault by killing the current thread.
/// Returns true if the thread was killed (never actually returns to caller
/// because exit_current switches context). Returns false if unrecoverable.
fn try_kill_faulting_thread(signal: u32) -> bool {
    let tid = crate::task::scheduler::current_tid();
    // TID 0 = idle context, TID 1-2 are typically desktop/critical threads
    // We can kill any thread that's a user process (even if temporarily in kernel mode)
    // or any non-critical kernel thread (TID > 2)
    if tid == 0 {
        return false; // Can't kill idle context
    }
    // User processes (their trampoline/syscall code runs in kernel mode too)
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

#[no_mangle]
pub extern "C" fn isr_handler(frame: &InterruptFrame) {
    let is_user_mode = frame.cs & 3 != 0;

    match frame.int_no {
        0 => {
            crate::serial_println!("EXCEPTION: Division by zero at EIP={:#010x} CS={:#x}", frame.eip, frame.cs);
            if is_user_mode {
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(136);
            }
            // Try to recover by killing the faulting thread
            if try_kill_faulting_thread(136) { return; }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        6 => {
            let dbg_tid = crate::task::scheduler::debug_current_tid();
            crate::serial_println!("EXCEPTION: Invalid opcode at EIP={:#010x} CS={:#x} (debug_tid={})", frame.eip, frame.cs, dbg_tid);
            crate::serial_println!(
                "  EAX={:#010x} EBX={:#010x} ECX={:#010x} EDX={:#010x}",
                frame.eax, frame.ebx, frame.ecx, frame.edx
            );
            crate::serial_println!(
                "  ESI={:#010x} EDI={:#010x} EBP={:#010x} ESP={:#010x}",
                frame.esi, frame.edi, frame.ebp, frame.esp
            );
            // Dump stack to find return addresses
            let stack_ptr = frame.esp as *const u32;
            crate::serial_println!("  Stack dump (from ESP):");
            for i in 0..16 {
                let val = unsafe { stack_ptr.add(i as usize).read_volatile() };
                crate::serial_println!("    [ESP+{:#04x}] = {:#010x}", i * 4, val);
            }
            // Walk EBP chain for stack trace
            crate::serial_println!("  EBP chain:");
            let mut bp = frame.ebp;
            for _ in 0..8 {
                if bp < 0xC000_0000 || bp > 0xD100_0000 { break; }
                let ret_addr = unsafe { *((bp + 4) as *const u32) };
                let prev_bp = unsafe { *(bp as *const u32) };
                crate::serial_println!("    EBP={:#010x} RET={:#010x}", bp, ret_addr);
                bp = prev_bp;
            }
            if is_user_mode {
                let frame_ptr = frame as *const InterruptFrame as *const u32;
                let esp3 = unsafe { frame_ptr.add(17).read() };
                crate::serial_println!("  User ESP={:#010x}", esp3);
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
                "EXCEPTION: General Protection Fault err={:#x} EIP={:#010x} CS={:#x}",
                frame.err_code, frame.eip, frame.cs
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
            let cr2: u32;
            unsafe { core::arch::asm!("mov {}, cr2", out(reg) cr2); }
            crate::serial_println!(
                "EXCEPTION: Page Fault addr={:#010x} EIP={:#010x} err={:#x}",
                cr2, frame.eip, frame.err_code
            );
            crate::serial_println!(
                "  CS={:#x} EAX={:#010x} EBX={:#010x} ECX={:#010x} EDX={:#010x}",
                frame.cs, frame.eax, frame.ebx, frame.ecx, frame.edx
            );
            crate::serial_println!(
                "  ESI={:#010x} EDI={:#010x} EBP={:#010x}",
                frame.esi, frame.edi, frame.ebp
            );
            if is_user_mode {
                let frame_ptr = frame as *const InterruptFrame as *const u32;
                let esp3 = unsafe { frame_ptr.add(17).read() };
                let ss3 = unsafe { frame_ptr.add(18).read() };
                crate::serial_println!("  User ESP={:#010x} SS={:#x}", esp3, ss3);
                crate::serial_println!("  User process fault — terminating thread");
                crate::task::scheduler::exit_current(139);
            }
            if try_kill_faulting_thread(139) { return; }
            crate::serial_println!("  FATAL: unrecoverable kernel fault — halting");
            loop { unsafe { core::arch::asm!("cli; hlt"); } }
        }
        _ => {
            crate::serial_println!("Unhandled exception #{} at EIP={:#010x}", frame.int_no, frame.eip);
            if is_user_mode {
                crate::task::scheduler::exit_current(128 + frame.int_no);
            }
            if try_kill_faulting_thread(128 + frame.int_no) { return; }
        }
    }
}

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
