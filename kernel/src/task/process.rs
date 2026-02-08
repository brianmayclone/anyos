use alloc::vec::Vec;

static mut NEXT_PID: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Active,
    Zombie,
}

pub struct Process {
    pub pid: u32,
    pub parent_pid: u32,
    pub state: ProcessState,
    pub page_directory: u32, // Physical address of page directory
    pub thread_ids: Vec<u32>,
    pub name: [u8; 64],
}

impl Process {
    pub fn new(name: &str, page_directory: u32) -> Self {
        let pid = unsafe {
            let p = NEXT_PID;
            NEXT_PID += 1;
            p
        };

        let mut name_buf = [0u8; 64];
        let bytes = name.as_bytes();
        let len = bytes.len().min(63);
        name_buf[..len].copy_from_slice(&bytes[..len]);

        Process {
            pid,
            parent_pid: 0,
            state: ProcessState::Active,
            page_directory,
            thread_ids: Vec::new(),
            name: name_buf,
        }
    }

    pub fn name_str(&self) -> &str {
        let len = self.name.iter().position(|&b| b == 0).unwrap_or(64);
        core::str::from_utf8(&self.name[..len]).unwrap_or("???")
    }
}
