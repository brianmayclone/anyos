use alloc::string::String;

pub struct LiveCheckState {
    pub timer_id: u32,
    pub debounce_ticks: u32,
    pub pending: Option<QueuedCheck>,
    pub running: Option<RunningCheck>,
    pub output_buffer: String,
    pub label: String,
}

#[derive(Clone, Copy)]
pub struct QueuedCheck {
    pub editor_index: usize,
    pub version: u32,
}

pub struct RunningCheck {
    pub file_path: String,
    pub version: u32,
}

impl LiveCheckState {
    pub fn new() -> Self {
        Self {
            timer_id: 0,
            debounce_ticks: 0,
            pending: None,
            running: None,
            output_buffer: String::new(),
            label: String::new(),
        }
    }

    pub fn queue(&mut self, editor_index: usize, version: u32, debounce_ticks: u32) {
        self.pending = Some(QueuedCheck {
            editor_index,
            version,
        });
        self.debounce_ticks = debounce_ticks;
    }

    pub fn requeue(&mut self, check: QueuedCheck, debounce_ticks: u32) {
        self.pending = Some(check);
        self.debounce_ticks = debounce_ticks;
    }

    pub fn take_pending(&mut self) -> Option<QueuedCheck> {
        self.pending.take()
    }

    pub fn begin_running(&mut self, file_path: &str, version: u32, label: &str) {
        self.output_buffer.clear();
        self.label = String::from(label);
        self.running = Some(RunningCheck {
            file_path: String::from(file_path),
            version,
        });
    }

    pub fn finish_running(&mut self) {
        self.output_buffer.clear();
        self.label.clear();
        self.running = None;
    }

    pub fn reset(&mut self) {
        self.debounce_ticks = 0;
        self.pending = None;
        self.running = None;
        self.output_buffer.clear();
        self.label.clear();
    }
}
