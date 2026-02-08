//! Interactive shell with built-in commands (help, ls, cat, ping, dhcp, etc.).
//! Shared by both the graphical terminal and the VGA text-mode fallback.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Abstraction over output targets (graphical terminal buffer, VGA text, etc.)
pub trait ShellOutput {
    fn write_str(&mut self, s: &str);
    fn write_line(&mut self, s: &str) {
        self.write_str(s);
        self.write_str("\n");
    }
    fn clear(&mut self);
}

/// Interactive command-line shell with input editing, command history, and
/// built-in commands for system inspection and network diagnostics.
pub struct Shell {
    input: String,
    cursor: usize,
    history: Vec<String>,
    history_index: Option<usize>,
}

impl Shell {
    /// Create a new shell with empty input and history.
    pub fn new() -> Self {
        Shell {
            input: String::new(),
            cursor: 0,
            history: Vec::new(),
            history_index: None,
        }
    }

    /// Return the current input line.
    pub fn input(&self) -> &str {
        &self.input
    }

    /// Return the current cursor position within the input line.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Return the shell prompt string.
    pub fn prompt() -> &'static str {
        "anyos> "
    }

    /// Insert a character at the cursor position and advance the cursor.
    pub fn insert_char(&mut self, c: char) {
        if self.cursor >= self.input.len() {
            self.input.push(c);
        } else {
            self.input.insert(self.cursor, c);
        }
        self.cursor += 1;
        self.history_index = None;
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.input.remove(self.cursor);
        }
    }

    /// Delete the character at the cursor position.
    pub fn delete(&mut self) {
        if self.cursor < self.input.len() {
            self.input.remove(self.cursor);
        }
    }

    /// Move cursor one position to the left.
    pub fn move_left(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    /// Move cursor one position to the right.
    pub fn move_right(&mut self) {
        if self.cursor < self.input.len() {
            self.cursor += 1;
        }
    }

    /// Move cursor to the beginning of the input line.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Move cursor to the end of the input line.
    pub fn end(&mut self) {
        self.cursor = self.input.len();
    }

    /// Navigate to the previous command in history.
    pub fn history_up(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let idx = match self.history_index {
            None => {
                if self.history.is_empty() { return; }
                self.history.len() - 1
            }
            Some(0) => return,
            Some(i) => i - 1,
        };
        self.history_index = Some(idx);
        self.input = self.history[idx].clone();
        self.cursor = self.input.len();
    }

    /// Navigate to the next command in history, or clear input if at the end.
    pub fn history_down(&mut self) {
        match self.history_index {
            None => return,
            Some(i) => {
                if i + 1 >= self.history.len() {
                    self.history_index = None;
                    self.input.clear();
                    self.cursor = 0;
                } else {
                    self.history_index = Some(i + 1);
                    self.input = self.history[i + 1].clone();
                    self.cursor = self.input.len();
                }
            }
        }
    }

    /// Execute the current input line. Returns false if the shell should exit.
    pub fn submit(&mut self, out: &mut dyn ShellOutput) -> bool {
        let line = self.input.trim().to_string();
        out.write_str("\n");

        if !line.is_empty() {
            // Add to history (avoid duplicates of last entry)
            if self.history.last().map_or(true, |last| *last != line) {
                self.history.push(line.clone());
                if self.history.len() > 64 {
                    self.history.remove(0);
                }
            }
        }

        self.input.clear();
        self.cursor = 0;
        self.history_index = None;

        if line.is_empty() {
            return true;
        }

        // Parse command and args
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let args = parts.next().unwrap_or("");

        match cmd {
            "help" => self.cmd_help(out),
            "echo" => out.write_line(args),
            "clear" => out.clear(),
            "uname" => out.write_line(".anyOS v0.1 i686"),
            "mem" => self.cmd_mem(out),
            "ps" => self.cmd_ps(out),
            "time" => self.cmd_time(out),
            "pci" => self.cmd_pci(out),
            "dev" => self.cmd_dev(out),
            "uptime" => self.cmd_uptime(out),
            "ls" => self.cmd_ls(args, out),
            "cat" => self.cmd_cat(args, out),
            "ping" => self.cmd_ping(args, out),
            "dhcp" => self.cmd_dhcp(out),
            "dns" => self.cmd_dns(args, out),
            "ifconfig" => self.cmd_ifconfig(out),
            "arp" => self.cmd_arp(out),
            "reboot" => {
                out.write_line("Rebooting...");
                unsafe { crate::arch::x86::port::outb(0x64, 0xFE); }
                loop { unsafe { core::arch::asm!("hlt"); } }
            }
            "exit" => return false,
            _ => {
                // Try to load from /bin/<cmd>
                use alloc::format;
                let path = format!("/bin/{}", cmd);
                match crate::task::loader::load_and_run_with_args(&path, cmd, args) {
                    Ok(tid) => {
                        let exit_code = crate::task::scheduler::waitpid(tid);
                        if exit_code != 0 {
                            out.write_line(&format!("Program exited with code {}", exit_code));
                        }
                    }
                    Err(_) => {
                        out.write_str("Unknown command: ");
                        out.write_line(cmd);
                        out.write_line("Type 'help' for a list of commands.");
                    }
                }
            }
        }

        true
    }

    fn cmd_help(&self, out: &mut dyn ShellOutput) {
        out.write_line(".anyOS Shell - Built-in Commands:");
        out.write_line("");
        out.write_line("  help     - Show this help");
        out.write_line("  echo     - Print text");
        out.write_line("  clear    - Clear screen");
        out.write_line("  uname    - System identification");
        out.write_line("  mem      - Memory statistics");
        out.write_line("  ps       - List running threads");
        out.write_line("  time     - Show current time");
        out.write_line("  uptime   - Show system uptime");
        out.write_line("  pci      - List PCI devices");
        out.write_line("  dev      - List HAL devices");
        out.write_line("  ls       - List directory contents");
        out.write_line("  cat      - Show file contents");
        out.write_line("  ping     - Ping an IP address");
        out.write_line("  dhcp     - Request IP via DHCP");
        out.write_line("  dns      - Resolve hostname");
        out.write_line("  ifconfig - Show network config");
        out.write_line("  arp      - Show ARP table");
        out.write_line("  reboot   - Restart the system");
        out.write_line("  exit     - Exit shell");
    }

    fn cmd_mem(&self, out: &mut dyn ShellOutput) {
        let free = crate::memory::physical::free_frames();
        let total = crate::memory::physical::total_frames();
        let used = total - free;

        use alloc::format;
        out.write_line(&format!(
            "Physical: {} / {} frames ({} / {} MiB)",
            used, total,
            used * 4 / 1024, total * 4 / 1024
        ));
        out.write_line(&format!(
            "Free: {} frames ({} MiB)",
            free, free * 4 / 1024
        ));
    }

    fn cmd_ps(&self, out: &mut dyn ShellOutput) {
        let threads = crate::task::scheduler::list_threads();
        use alloc::format;
        out.write_line(&format!("  TID  PRI  STATE    NAME"));
        for t in &threads {
            out.write_line(&format!(
                "  {:>3}  {:>3}  {:<8} {}",
                t.tid, t.priority, t.state, t.name
            ));
        }
    }

    fn cmd_time(&self, out: &mut dyn ShellOutput) {
        let (year, month, day, hour, min, sec) = crate::drivers::rtc::read_datetime();
        use alloc::format;
        out.write_line(&format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            year, month, day, hour, min, sec
        ));
    }

    fn cmd_uptime(&self, out: &mut dyn ShellOutput) {
        let ticks = crate::arch::x86::pit::get_ticks();
        let secs = ticks / 100;
        let mins = secs / 60;
        let hours = mins / 60;
        use alloc::format;
        out.write_line(&format!(
            "Uptime: {}h {}m {}s ({} ticks)",
            hours, mins % 60, secs % 60, ticks
        ));
    }

    fn cmd_pci(&self, out: &mut dyn ShellOutput) {
        let devices = crate::drivers::pci::devices();
        use alloc::format;
        for dev in &devices {
            out.write_line(&format!(
                "  {:02x}:{:02x}.{} {:04x}:{:04x} {}",
                dev.bus, dev.device, dev.function,
                dev.vendor_id, dev.device_id,
                crate::drivers::pci::class_name(dev.class_code, dev.subclass),
            ));
        }
    }

    fn cmd_dev(&self, out: &mut dyn ShellOutput) {
        let devices = crate::drivers::hal::list_devices();
        use alloc::format;
        for (path, name, dtype) in &devices {
            out.write_line(&format!("  {:<12} {:<30} {:?}", path, name, dtype));
        }
    }

    fn cmd_ls(&self, args: &str, out: &mut dyn ShellOutput) {
        use alloc::format;
        let path = if args.trim().is_empty() { "/" } else { args.trim() };

        match crate::fs::vfs::read_dir(path) {
            Ok(entries) => {
                for entry in &entries {
                    let type_char = match entry.file_type {
                        crate::fs::file::FileType::Directory => 'd',
                        crate::fs::file::FileType::Regular => '-',
                        crate::fs::file::FileType::Device => 'c',
                    };
                    if entry.file_type == crate::fs::file::FileType::Directory {
                        out.write_line(&format!("  {}  {:>8}  {}/", type_char, entry.size, entry.name));
                    } else {
                        out.write_line(&format!("  {}  {:>8}  {}", type_char, entry.size, entry.name));
                    }
                }
            }
            Err(_) => {
                out.write_str("ls: cannot access '");
                out.write_str(path);
                out.write_line("': No such file or directory");
            }
        }
    }

    fn cmd_cat(&self, args: &str, out: &mut dyn ShellOutput) {
        let path = args.trim();
        if path.is_empty() {
            out.write_line("Usage: cat <file>");
            return;
        }

        match crate::fs::vfs::read_file_to_vec(path) {
            Ok(data) => {
                if let Ok(text) = core::str::from_utf8(&data) {
                    out.write_str(text);
                    if !text.ends_with('\n') {
                        out.write_str("\n");
                    }
                } else {
                    use alloc::format;
                    out.write_line(&format!("(binary file, {} bytes)", data.len()));
                }
            }
            Err(_) => {
                out.write_str("cat: ");
                out.write_str(path);
                out.write_line(": No such file or directory");
            }
        }
    }

    fn cmd_ping(&self, args: &str, out: &mut dyn ShellOutput) {
        use alloc::format;
        let target = args.trim();
        if target.is_empty() {
            out.write_line("Usage: ping <ip>");
            return;
        }

        let ip = match crate::net::types::Ipv4Addr::parse(target) {
            Some(ip) => ip,
            None => {
                out.write_line("Invalid IP address");
                return;
            }
        };

        let cfg = crate::net::config();
        if cfg.ip == crate::net::types::Ipv4Addr::ZERO {
            out.write_line("No IP configured. Run 'dhcp' first.");
            return;
        }

        out.write_line(&format!("PING {} from {}", ip, cfg.ip));

        for seq in 0..4u16 {
            match crate::net::icmp::ping(ip, seq, 200) {
                Some((rtt, ttl)) => {
                    let ms = rtt * 10; // 100Hz timer, each tick is 10ms
                    out.write_line(&format!(
                        "Reply from {}: seq={} ttl={} time={}ms",
                        ip, seq, ttl, ms
                    ));
                }
                None => {
                    out.write_line(&format!("Request timeout for seq={}", seq));
                }
            }
        }
    }

    fn cmd_dhcp(&self, out: &mut dyn ShellOutput) {
        out.write_line("Running DHCP...");

        match crate::net::dhcp::discover() {
            Ok(result) => {
                use alloc::format;
                crate::net::set_config(result.ip, result.mask, result.gateway, result.dns);
                out.write_line(&format!("  IP:      {}", result.ip));
                out.write_line(&format!("  Mask:    {}", result.mask));
                out.write_line(&format!("  Gateway: {}", result.gateway));
                out.write_line(&format!("  DNS:     {}", result.dns));
                out.write_line("DHCP configuration applied.");
            }
            Err(e) => {
                out.write_str("DHCP failed: ");
                out.write_line(e);
            }
        }
    }

    fn cmd_dns(&self, args: &str, out: &mut dyn ShellOutput) {
        use alloc::format;
        let hostname = args.trim();
        if hostname.is_empty() {
            out.write_line("Usage: dns <hostname>");
            return;
        }

        let cfg = crate::net::config();
        if cfg.dns == crate::net::types::Ipv4Addr::ZERO {
            out.write_line("No DNS server configured. Run 'dhcp' first.");
            return;
        }

        match crate::net::dns::resolve(hostname) {
            Ok(ip) => {
                out.write_line(&format!("{} -> {}", hostname, ip));
            }
            Err(e) => {
                out.write_str("DNS resolve failed: ");
                out.write_line(e);
            }
        }
    }

    fn cmd_ifconfig(&self, out: &mut dyn ShellOutput) {
        use alloc::format;
        let cfg = crate::net::config();
        let link = if crate::drivers::network::e1000::is_link_up() { "UP" } else { "DOWN" };

        out.write_line(&format!("eth0: link {}", link));
        out.write_line(&format!("  MAC:     {}", cfg.mac));
        out.write_line(&format!("  IP:      {}", cfg.ip));
        out.write_line(&format!("  Mask:    {}", cfg.mask));
        out.write_line(&format!("  Gateway: {}", cfg.gateway));
        out.write_line(&format!("  DNS:     {}", cfg.dns));
    }

    fn cmd_arp(&self, out: &mut dyn ShellOutput) {
        use alloc::format;
        let entries = crate::net::arp::entries();
        if entries.is_empty() {
            out.write_line("ARP table is empty");
        } else {
            out.write_line("  IP Address        MAC Address");
            for (ip, mac) in &entries {
                out.write_line(&format!("  {:<16}  {}", ip, mac));
            }
        }
    }
}
