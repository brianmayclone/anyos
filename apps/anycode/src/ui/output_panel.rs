use libanyui_client as ui;

/// The bottom panel with Output and Terminal tabs.
pub struct OutputPanel {
    pub panel: ui::View,
    pub tab_bar: ui::TabBar,
    // Output sub-panel
    output_panel: ui::View,
    output_area: ui::TextArea,
    // Terminal sub-panel
    terminal_panel: ui::View,
    terminal_area: ui::TextArea,
    pub terminal_input: ui::TextField,
    // Shell process state
    pub shell_stdout_pipe: u32,
    pub shell_stdin_pipe: u32,
    pub shell_tid: u32,
}

impl OutputPanel {
    /// Create the bottom panel with Output + Terminal tabs.
    pub fn new() -> Self {
        let panel = ui::View::new();
        panel.set_color(0xFF1E1E1E);

        // Tab bar for switching Output / Terminal
        let tab_bar = ui::TabBar::new("Output|Terminal");
        tab_bar.set_dock(ui::DOCK_TOP);
        tab_bar.set_size(400, 24);
        tab_bar.set_color(0xFF252526);
        panel.add(&tab_bar);

        // ── Output sub-panel ──
        let output_panel = ui::View::new();
        output_panel.set_dock(ui::DOCK_FILL);
        output_panel.set_color(0xFF1E1E1E);
        panel.add(&output_panel);

        let output_area = ui::TextArea::new();
        output_area.set_dock(ui::DOCK_FILL);
        output_area.set_font(4); // monospace
        output_area.set_font_size(12);
        output_area.set_color(0xFF1E1E1E);
        output_area.set_text_color(0xFFCCCCCC);
        output_panel.add(&output_area);

        // ── Terminal sub-panel ──
        let terminal_panel = ui::View::new();
        terminal_panel.set_dock(ui::DOCK_FILL);
        terminal_panel.set_color(0xFF1E1E1E);
        terminal_panel.set_visible(false);
        panel.add(&terminal_panel);

        let terminal_area = ui::TextArea::new();
        terminal_area.set_dock(ui::DOCK_FILL);
        terminal_area.set_font(4);
        terminal_area.set_font_size(12);
        terminal_area.set_color(0xFF1E1E1E);
        terminal_area.set_text_color(0xFF33FF33);
        terminal_panel.add(&terminal_area);

        let terminal_input = ui::TextField::new();
        terminal_input.set_dock(ui::DOCK_BOTTOM);
        terminal_input.set_size(400, 24);
        terminal_input.set_font(4);
        terminal_input.set_font_size(12);
        terminal_input.set_color(0xFF2D2D2D);
        terminal_input.set_text_color(0xFFCCCCCC);
        terminal_input.set_placeholder("$ ");
        terminal_panel.add(&terminal_input);

        // Wire tab switching
        tab_bar.connect_panels(&[&output_panel, &terminal_panel]);

        Self {
            panel,
            tab_bar,
            output_panel,
            output_area,
            terminal_panel,
            terminal_area,
            terminal_input,
            shell_stdout_pipe: 0,
            shell_stdin_pipe: 0,
            shell_tid: 0,
        }
    }

    // ── Output methods ──

    /// Clear all output.
    pub fn clear(&self) {
        self.output_area.set_text("");
    }

    /// Append text to the output (read existing + concat).
    pub fn append(&self, text: &str) {
        let mut buf = [0u8; 32768];
        let existing = self.output_area.get_text(&mut buf) as usize;
        let add = text.len().min(buf.len() - existing);
        buf[existing..existing + add].copy_from_slice(&text.as_bytes()[..add]);
        let total = existing + add;
        if let Ok(full) = core::str::from_utf8(&buf[..total]) {
            self.output_area.set_text(full);
        }
    }

    /// Append a line to the output (with trailing newline).
    pub fn append_line(&self, text: &str) {
        let mut buf = [0u8; 32768];
        let existing = self.output_area.get_text(&mut buf) as usize;
        let add = text.len().min(buf.len() - existing - 1);
        buf[existing..existing + add].copy_from_slice(&text.as_bytes()[..add]);
        let mut total = existing + add;
        if total < buf.len() {
            buf[total] = b'\n';
            total += 1;
        }
        if let Ok(full) = core::str::from_utf8(&buf[..total]) {
            self.output_area.set_text(full);
        }
    }

    // ── Terminal methods ──

    /// Start the shell process (/System/bin/sh) with piped I/O.
    pub fn start_shell(&mut self, working_dir: &str) {
        // Kill existing shell if running
        if self.shell_tid != 0 {
            anyos_std::process::kill(self.shell_tid);
            anyos_std::ipc::pipe_close(self.shell_stdout_pipe);
            anyos_std::ipc::pipe_close(self.shell_stdin_pipe);
            self.shell_tid = 0;
            self.shell_stdout_pipe = 0;
            self.shell_stdin_pipe = 0;
        }
        let stdout_pipe = anyos_std::ipc::pipe_create("anycode:term:out");
        let stdin_pipe = anyos_std::ipc::pipe_create("anycode:term:in");
        if stdout_pipe == 0 || stdin_pipe == 0 {
            return;
        }

        anyos_std::fs::chdir(working_dir);
        let tid = anyos_std::process::spawn_piped_full(
            "/System/bin/sh", "sh -i", stdout_pipe, stdin_pipe,
        );
        if tid == u32::MAX {
            anyos_std::ipc::pipe_close(stdout_pipe);
            anyos_std::ipc::pipe_close(stdin_pipe);
            return;
        }

        self.shell_stdout_pipe = stdout_pipe;
        self.shell_stdin_pipe = stdin_pipe;
        self.shell_tid = tid;
    }

    /// Send a command to the shell.
    pub fn send_to_shell(&self, cmd: &str) {
        if self.shell_stdin_pipe == 0 {
            return;
        }
        // Echo command to terminal area
        self.append_terminal("$ ");
        self.append_terminal(cmd);
        self.append_terminal("\n");
        // Send command + newline to shell
        let mut buf = [0u8; 512];
        let len = cmd.len().min(buf.len() - 1);
        buf[..len].copy_from_slice(&cmd.as_bytes()[..len]);
        buf[len] = b'\n';
        anyos_std::ipc::pipe_write(self.shell_stdin_pipe, &buf[..len + 1]);
    }

    /// Poll for output from the shell process.
    pub fn poll_shell_output(&self) {
        if self.shell_stdout_pipe == 0 {
            return;
        }
        let mut buf = [0u8; 1024];
        loop {
            let n = anyos_std::ipc::pipe_read(self.shell_stdout_pipe, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            if let Ok(text) = core::str::from_utf8(&buf[..n as usize]) {
                self.append_terminal(text);
            }
        }
    }

    /// Append text to the terminal output area.
    fn append_terminal(&self, text: &str) {
        let mut buf = [0u8; 32768];
        let existing = self.terminal_area.get_text(&mut buf) as usize;
        let add = text.len().min(buf.len() - existing);
        buf[existing..existing + add].copy_from_slice(&text.as_bytes()[..add]);
        let total = existing + add;
        if let Ok(full) = core::str::from_utf8(&buf[..total]) {
            self.terminal_area.set_text(full);
        }
    }

    /// Check if the shell is still running.
    pub fn is_shell_running(&self) -> bool {
        if self.shell_tid == 0 {
            return false;
        }
        let status = anyos_std::process::try_waitpid(self.shell_tid);
        status == anyos_std::process::STILL_RUNNING
    }
}
