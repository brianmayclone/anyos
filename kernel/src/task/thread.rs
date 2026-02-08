use crate::memory::address::PhysAddr;
use crate::task::context::CpuContext;
use alloc::boxed::Box;
use alloc::vec;

static mut NEXT_TID: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    Ready,
    Running,
    Blocked,
    Terminated,
}

pub struct Thread {
    pub tid: u32,
    pub state: ThreadState,
    pub context: CpuContext,
    pub kernel_stack: Box<[u8]>,
    pub priority: u8,
    pub name: [u8; 32],
    pub exit_code: Option<u32>,
    pub waiting_tid: Option<u32>,
    pub is_user: bool,
    /// Per-process page directory (None for kernel threads that share the kernel PD).
    pub page_directory: Option<PhysAddr>,
    /// Current program break (end of data/heap segment) for user processes.
    pub brk: u32,
    /// Command-line arguments (null-terminated string, set at spawn time).
    pub args: [u8; 256],
    /// Pipe ID for stdout redirection (0 = no pipe, write to serial).
    pub stdout_pipe: u32,
    /// CPU ticks consumed by this thread (incremented each scheduler tick while running).
    pub cpu_ticks: u32,
}

const KERNEL_STACK_SIZE: usize = 16 * 1024; // 16 KiB per thread

impl Thread {
    pub fn new(entry: extern "C" fn(), priority: u8, name: &str) -> Self {
        let tid = unsafe {
            let t = NEXT_TID;
            NEXT_TID += 1;
            t
        };

        // Allocate kernel stack on the heap directly (NOT via Box::new which
        // would create a 16 KiB temporary on the current stack — fatal when
        // called from a syscall where the kernel stack is only 16 KiB).
        let stack: Box<[u8]> = vec![0u8; KERNEL_STACK_SIZE].into_boxed_slice();
        let stack_top = stack.as_ptr() as u32 + KERNEL_STACK_SIZE as u32;

        // Set up initial context so that when we "switch" to this thread,
        // it starts executing at `entry`
        let mut context = CpuContext::default();
        context.eip = entry as *const () as u32;
        context.esp = stack_top;
        context.ebp = stack_top;
        context.eflags = 0x202; // IF (interrupts enabled) + reserved bit 1
        // Use the current page directory (all kernel threads share same address space)
        unsafe { core::arch::asm!("mov {}, cr3", out(reg) context.cr3); }

        // Copy name
        let mut name_buf = [0u8; 32];
        let bytes = name.as_bytes();
        let len = bytes.len().min(31);
        name_buf[..len].copy_from_slice(&bytes[..len]);

        Thread {
            tid,
            state: ThreadState::Ready,
            context,
            kernel_stack: stack as Box<[u8]>,
            priority,
            name: name_buf,
            exit_code: None,
            waiting_tid: None,
            is_user: false,
            page_directory: None,
            brk: 0,
            args: [0u8; 256],
            stdout_pipe: 0,
            cpu_ticks: 0,
        }
    }

    pub fn kernel_stack_top(&self) -> u32 {
        self.kernel_stack.as_ptr() as u32 + self.kernel_stack.len() as u32
    }

    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(32);
        core::str::from_utf8(&self.name[..len]).unwrap_or("???")
    }
}
