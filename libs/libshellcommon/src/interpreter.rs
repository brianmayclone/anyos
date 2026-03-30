//! Shell interpreter — shared command execution engine for Terminal and Shell apps.

use anyos_std::{String, Vec, format};
use anyos_std::{fs, ipc, process, env};
use anyos_std::shell;

use crate::{InputLine, History, read_file_to_buf, make_prompt};

// ─── Output abstraction ─────────────────────────────────────────────────────

/// Trait for shell output.  Implemented by both Terminal (writes to TerminalBuffer)
/// and Shell (writes to window pixel buffer or pipe).
pub trait ShellOutput {
    fn write_str(&mut self, s: &str);
    fn write_char(&mut self, ch: char);
    fn write_line(&mut self, s: &str) {
        self.write_str(s);
        self.write_char('\n');
    }
}

// ─── Action returned by execute() ──────────────────────────────────────────

/// What the caller (Terminal/Shell app) should do after execute().
pub enum ShellAction {
    /// Nothing special — command was handled internally.
    Done,
    /// Exit the shell.
    Exit,
    /// Run an external command in the foreground (caller creates pipes + spawns).
    RunForeground {
        path: String,
        args: String,
        command: String,
        redirect: Option<shell::Redirect>,
        input_data: Option<String>,
    },
    /// Run an external command in the background.
    RunBackground {
        path: String,
        args: String,
        command: String,
    },
    /// Run a pipeline (caller uses anyos_std::shell::run_pipeline).
    RunPipeline {
        line: String,
        redirect: Option<shell::Redirect>,
    },
    /// Prompt the user for a password (su command).
    PromptPassword {
        username: String,
    },
    /// Reboot the system.
    Reboot,
    /// Shutdown the system.
    Shutdown,
}

fn actions_one(a: ShellAction) -> Vec<ShellAction> {
    let mut v = Vec::new();
    v.push(a);
    v
}

// ─── Logical Operators ──────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum LogicalOp {
    None,
    And,
    Or,
    Semicolon,
}

/// Split a command line on `&&`, `||`, and `;` operators.
fn split_logical_operators(line: &str) -> Option<Vec<(LogicalOp, String)>> {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut result: Vec<(LogicalOp, String)> = Vec::new();
    let mut current_op = LogicalOp::None;
    let mut start = 0;
    let mut i = 0;
    let mut found_op = false;
    let mut in_single_quote = false;
    let mut in_double_quote = false;

    while i < len {
        match bytes[i] {
            b'\'' if !in_double_quote => { in_single_quote = !in_single_quote; i += 1; }
            b'"' if !in_single_quote => { in_double_quote = !in_double_quote; i += 1; }
            b'\\' if !in_single_quote => { i += 2; }
            b'&' if !in_single_quote && !in_double_quote && i + 1 < len && bytes[i + 1] == b'&' => {
                let cmd = String::from(line[start..i].trim());
                result.push((current_op, cmd));
                current_op = LogicalOp::And;
                i += 2; start = i; found_op = true;
            }
            b'|' if !in_single_quote && !in_double_quote && i + 1 < len && bytes[i + 1] == b'|' => {
                let cmd = String::from(line[start..i].trim());
                result.push((current_op, cmd));
                current_op = LogicalOp::Or;
                i += 2; start = i; found_op = true;
            }
            b';' if !in_single_quote && !in_double_quote => {
                let cmd = String::from(line[start..i].trim());
                result.push((current_op, cmd));
                current_op = LogicalOp::Semicolon;
                i += 1; start = i; found_op = true;
            }
            _ => { i += 1; }
        }
    }

    if !found_op { return None; }
    let cmd = String::from(line[start..].trim());
    if !cmd.is_empty() { result.push((current_op, cmd)); }
    Some(result)
}

// ─── Background Job ─────────────────────────────────────────────────────────

pub struct BackgroundJob {
    pub job_id: u32,
    pub tid: u32,
    pub command: String,
    pub finished: bool,
    pub exit_code: u32,
    pub stopped: bool,
    pub pipe_id: u32,
    pub stdin_pipe: u32,
    pub extra_pipes: Vec<u32>,
}

// ─── Shell Interpreter ──────────────────────────────────────────────────────

/// Callback trait for app-specific builtins (e.g. Terminal's `help`, `profile`, `theme`).
pub trait AppBuiltins {
    /// Try to handle a builtin command.  Return true if handled.
    fn handle(&mut self, cmd: &str, args: &str, shell: &mut ShellState, out: &mut dyn ShellOutput) -> bool;
    /// Called after `cd` changes directory.
    fn on_cd(&mut self, new_cwd: &str);
    /// Called to clear the screen (the `clear` builtin).
    fn clear_screen(&mut self);
}

/// Minimal AppBuiltins that does nothing (for Shell app).
pub struct NoAppBuiltins;
impl AppBuiltins for NoAppBuiltins {
    fn handle(&mut self, _cmd: &str, _args: &str, _shell: &mut ShellState, _out: &mut dyn ShellOutput) -> bool { false }
    fn on_cd(&mut self, _new_cwd: &str) {}
    fn clear_screen(&mut self) {}
}

/// The shell interpreter state (decoupled from any GUI).
pub struct ShellState {
    pub line: InputLine,
    pub history: History,
    pub cwd: String,
    pub aliases: Vec<(String, String)>,
    pub bg_jobs: Vec<BackgroundJob>,
    pub next_job_id: u32,
    pub pipe_counter: u32,
}

impl ShellState {
    pub fn new() -> Self {
        ShellState {
            line: InputLine::new(),
            history: History::new(),
            cwd: String::from("/"),
            aliases: Vec::new(),
            bg_jobs: Vec::new(),
            next_job_id: 1,
            pipe_counter: 0,
        }
    }

    pub fn prompt(&self) -> String {
        make_prompt(&self.cwd)
    }

    // ── Delegation wrappers ─────────────────────────────────────────────

    pub fn cursor_byte_pos(&self) -> usize { self.line.cursor_byte_pos() }
    pub fn char_count(&self) -> usize { self.line.char_count() }

    pub fn insert_char(&mut self, c: char) {
        self.line.insert_char(c);
        self.history.reset_index();
    }

    pub fn backspace(&mut self) { self.line.backspace(); }

    pub fn history_up(&mut self) {
        if let Some(entry) = self.history.up() {
            let s = String::from(entry);
            self.line.set_text(&s);
        }
    }

    pub fn history_down(&mut self) {
        match self.history.down() {
            Some(entry) => {
                let s = String::from(entry);
                self.line.set_text(&s);
            }
            None => self.line.clear(),
        }
    }

    pub fn cursor_left(&mut self) { self.line.cursor_left(); }
    pub fn cursor_right(&mut self) { self.line.cursor_right(); }
    pub fn cursor_home(&mut self) { self.line.cursor_home(); }
    pub fn cursor_end(&mut self) { self.line.cursor_end(); }
    pub fn delete_at_cursor(&mut self) { self.line.delete(); }

    // ── Command execution ───────────────────────────────────────────────

    /// Execute the current input line.  Returns a list of actions for the caller.
    pub fn execute(&mut self, out: &mut dyn ShellOutput, app: &mut dyn AppBuiltins) -> Vec<ShellAction> {
        let raw_line = String::from(self.line.text.trim_matches(|c: char| c == ' '));
        out.write_char('\n');

        if !raw_line.is_empty() {
            self.history.push(&raw_line);
        }
        self.line.clear();
        self.history.reset_index();

        if raw_line.is_empty() {
            return Vec::new();
        }

        // Logical operators (&&, ||, ;)
        if let Some(chain) = split_logical_operators(&raw_line) {
            let mut actions = Vec::new();
            let mut last_success = true;
            for (op, cmd_str) in chain {
                let should_run = match op {
                    LogicalOp::None | LogicalOp::Semicolon => true,
                    LogicalOp::And => last_success,
                    LogicalOp::Or => !last_success,
                };
                if !should_run { continue; }
                let cmd_str = String::from(cmd_str.trim());
                if cmd_str.is_empty() { continue; }
                let saved_input = core::mem::replace(&mut self.line.text, cmd_str);
                let saved_cursor = self.line.cursor;
                self.line.cursor_end();
                let sub_actions = self.execute(out, app);
                self.line.text = saved_input;
                self.line.cursor = saved_cursor;
                // If sub-action requires caller intervention, return immediately
                for a in &sub_actions {
                    match a {
                        ShellAction::Done => { last_success = true; }
                        _ => {
                            actions.extend(sub_actions);
                            return actions;
                        }
                    }
                }
            }
            return actions;
        }

        let result = self.execute_single(&raw_line, out, app);

        // Report finished background jobs
        self.check_bg_jobs(out);

        result
    }

    /// Execute a single command (no logical operators).
    fn execute_single(&mut self, raw_line: &str, out: &mut dyn ShellOutput, app: &mut dyn AppBuiltins) -> Vec<ShellAction> {
        // Expand aliases
        let expanded_line = {
            let first_word = raw_line.split_whitespace().next().unwrap_or("");
            if let Some((_, val)) = self.aliases.iter().find(|(n, _)| n == first_word) {
                let rest = String::from(&raw_line[first_word.len()..]);
                format!("{}{}", val, rest)
            } else {
                String::from(raw_line)
            }
        };

        // Parse redirects
        let (expanded_line, input_redir) = shell::parse_input_redirect(&expanded_line, &self.cwd);
        let (line, redirect) = shell::parse_redirects(&expanded_line, &self.cwd);

        // Read input redirect file
        let input_data = if let Some(ref ir) = input_redir {
            fs::read_to_string(&ir.source).ok()
        } else {
            None
        };

        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let raw_args = parts.next().unwrap_or("");

        // POSIX expansions
        let tokens = shell::expand_args(raw_args, &self.cwd);
        let expanded_args_buf = tokens.join(" ");
        let args = expanded_args_buf.as_str();

        // Capture builtin output for redirect
        let mut capture_buf: Option<String> = if redirect.is_some() { Some(String::new()) } else { None };
        let use_capture = capture_buf.is_some();

        // Try app-specific builtins first
        if app.handle(cmd, args, self, out) {
            self.flush_capture(capture_buf, redirect);
            return actions_one(ShellAction::Done);
        }

        match cmd {
            "echo" => {
                if use_capture {
                    if let Some(ref mut buf) = capture_buf { buf.push_str(args); buf.push('\n'); }
                } else {
                    out.write_line(args);
                }
            }
            "clear" => app.clear_screen(),
            "uname" => {
                let msg = format!(".anyOS x86_64");
                if use_capture {
                    if let Some(ref mut buf) = capture_buf { buf.push_str(&msg); buf.push('\n'); }
                } else {
                    out.write_line(&msg);
                }
            }
            "cd" => self.cmd_cd(args, out, app),
            "pwd" => {
                if use_capture {
                    if let Some(ref mut buf) = capture_buf { buf.push_str(&self.cwd); buf.push('\n'); }
                } else {
                    out.write_line(&self.cwd);
                }
            }
            "set" => self.cmd_set(args, out),
            "export" => self.cmd_export(args, out),
            "unset" => self.cmd_unset(args, out),
            "alias" => self.cmd_alias(args, out),
            "unalias" => self.cmd_unalias(args, out),
            "eval" => {
                let eval_line = String::from(args.trim());
                if !eval_line.is_empty() {
                    let saved_input = core::mem::replace(&mut self.line.text, eval_line);
                    let saved_cursor = self.line.cursor;
                    self.line.cursor_end();
                    let result = self.execute(out, app);
                    self.line.text = saved_input;
                    self.line.cursor = saved_cursor;
                    return result;
                }
                self.flush_capture(capture_buf, redirect);
                return actions_one(ShellAction::Done);
            }
            "source" | "." => self.cmd_source(args, out, app),
            "su" => {
                self.flush_capture(capture_buf, redirect);
                let parts_vec: Vec<&str> = args.split_whitespace().collect();
                let username = if parts_vec.is_empty() { "root" } else { parts_vec[0] };
                if parts_vec.len() > 1 {
                    Self::do_su(username, parts_vec[1], out);
                    return actions_one(ShellAction::Done);
                } else {
                    out.write_str("Password: ");
                    return actions_one(ShellAction::PromptPassword { username: String::from(username) });
                }
            }
            "jobs" => self.cmd_jobs(out),
            "fg" => {
                self.flush_capture(capture_buf, redirect);
                return self.cmd_fg(out);
            }
            "bg" => self.cmd_bg(out),
            "exit" => {
                return actions_one(ShellAction::Exit);
            }
            "reboot" => {
                out.write_line("Rebooting...");
                return actions_one(ShellAction::Reboot);
            }
            "shutdown" | "poweroff" => {
                out.write_line("Shutting down...");
                return actions_one(ShellAction::Shutdown);
            }
            _ => {
                // External command — no capture needed, pass redirect to caller
                return self.execute_external(cmd, &line, redirect, input_data, out);
            }
        }

        self.flush_capture(capture_buf, redirect);
        actions_one(ShellAction::Done)
    }

    fn flush_capture(&self, capture_buf: Option<String>, redirect: Option<shell::Redirect>) {
        if let Some(captured) = capture_buf {
            if let Some(mut redir) = redirect {
                shell::write_redirect(&mut redir, &captured);
            }
        }
    }

    /// Execute an external command (not a builtin).
    fn execute_external(&mut self, _cmd: &str, line: &str, redirect: Option<shell::Redirect>,
                        input_data: Option<String>, _out: &mut dyn ShellOutput) -> Vec<ShellAction> {
        // Pipeline
        if shell::has_pipe(line) {
            return actions_one(ShellAction::RunPipeline {
                line: String::from(line),
                redirect,
            });
        }

        // Background suffix
        let (cmd_line, background) = if line.ends_with(" &") || line.ends_with("\t&") {
            (&line[..line.len() - 2], true)
        } else if line.ends_with('&') && line.len() > 1 {
            (&line[..line.len() - 1], true)
        } else {
            (line, false)
        };

        let mut bg_parts = cmd_line.splitn(2, ' ');
        let bg_cmd = bg_parts.next().unwrap_or("");
        let raw_bg_args = bg_parts.next().unwrap_or("");
        let mut bg_tokens = shell::expand_args(raw_bg_args, &self.cwd);

        // Default args for ls
        if bg_cmd == "ls" {
            if bg_tokens.is_empty() {
                bg_tokens.push(String::from("--color"));
                bg_tokens.push(String::from(self.cwd.as_str()));
            } else if bg_tokens.iter().all(|t| t.starts_with('-')) {
                bg_tokens.insert(0, String::from("--color"));
                bg_tokens.push(String::from(self.cwd.as_str()));
            } else {
                bg_tokens.insert(0, String::from("--color"));
            }
        }

        let path = shell::resolve_cmd_path(bg_cmd, &self.cwd);
        let bg_args_quoted = shell::join(&bg_tokens);
        let full_args = if bg_args_quoted.is_empty() {
            String::from(bg_cmd)
        } else {
            format!("{} {}", bg_cmd, bg_args_quoted)
        };

        if background {
            return actions_one(ShellAction::RunBackground {
                path,
                args: full_args,
                command: String::from(cmd_line),
            });
        }

        actions_one(ShellAction::RunForeground {
            path,
            args: full_args,
            command: String::from(cmd_line),
            redirect,
            input_data,
        })
    }

    // ── Builtin: cd ─────────────────────────────────────────────────────

    fn cmd_cd(&mut self, args: &str, out: &mut dyn ShellOutput, app: &mut dyn AppBuiltins) {
        let target = args.trim();
        if target.is_empty() || target == "/" {
            self.cwd = String::from("/");
            env::set("PWD", "/");
            fs::chdir("/");
            app.on_cd("/");
            return;
        }

        let new_path = if target.starts_with('/') {
            String::from(target)
        } else if target == "~" {
            let mut home_buf = [0u8; 128];
            let hlen = env::get("HOME", &mut home_buf);
            if hlen != u32::MAX && hlen > 0 {
                String::from(core::str::from_utf8(&home_buf[..hlen as usize]).unwrap_or("/"))
            } else {
                String::from("/")
            }
        } else if target == ".." {
            if self.cwd == "/" { return; }
            let trimmed = self.cwd.trim_end_matches('/');
            match trimmed.rfind('/') {
                Some(0) => String::from("/"),
                Some(pos) => String::from(&trimmed[..pos]),
                None => String::from("/"),
            }
        } else {
            if self.cwd == "/" {
                format!("/{}", target)
            } else {
                format!("{}/{}", self.cwd, target)
            }
        };

        let mut stat_buf = [0u32; 7];
        if fs::stat(&new_path, &mut stat_buf) != 0 {
            out.write_str("cd: ");
            out.write_str(&new_path);
            out.write_line(": No such directory");
            return;
        }
        if stat_buf[0] != 1 {
            out.write_str("cd: ");
            out.write_str(&new_path);
            out.write_line(": Not a directory");
            return;
        }

        self.cwd = new_path;
        env::set("PWD", &self.cwd);
        fs::chdir(&self.cwd);
        app.on_cd(&self.cwd);
    }

    // ── Builtin: set/export/unset ───────────────────────────────────────

    fn cmd_set(&self, args: &str, out: &mut dyn ShellOutput) {
        let args = args.trim();
        if args.is_empty() {
            let mut env_buf = [0u8; 4096];
            let total = env::list(&mut env_buf);
            let len = (total as usize).min(env_buf.len());
            let mut offset = 0;
            while offset < len {
                let end = env_buf[offset..len].iter().position(|&b| b == 0).unwrap_or(len - offset);
                if end == 0 { break; }
                if let Ok(entry) = core::str::from_utf8(&env_buf[offset..offset + end]) {
                    out.write_line(entry);
                }
                offset += end + 1;
            }
            return;
        }
        if let Some(eq_pos) = args.find('=') {
            let key = &args[..eq_pos];
            let value = &args[eq_pos + 1..];
            if key.is_empty() {
                out.write_line("set: invalid variable name");
                return;
            }
            env::set(key, value);
        } else {
            let mut val_buf = [0u8; 256];
            let len = env::get(args, &mut val_buf);
            if len != u32::MAX {
                let val = core::str::from_utf8(&val_buf[..len as usize]).unwrap_or("");
                out.write_str(args);
                out.write_char('=');
                out.write_line(val);
            } else {
                out.write_str("set: '");
                out.write_str(args);
                out.write_line("' not set");
            }
        }
    }

    fn cmd_export(&self, args: &str, out: &mut dyn ShellOutput) {
        let args = args.trim();
        if args.is_empty() {
            let mut env_buf = [0u8; 4096];
            let total = env::list(&mut env_buf);
            let len = (total as usize).min(env_buf.len());
            let mut offset = 0;
            while offset < len {
                let end = env_buf[offset..len].iter().position(|&b| b == 0).unwrap_or(len - offset);
                if end == 0 { break; }
                if let Ok(entry) = core::str::from_utf8(&env_buf[offset..offset + end]) {
                    out.write_str("export ");
                    out.write_line(entry);
                }
                offset += end + 1;
            }
            return;
        }
        if let Some(eq_pos) = args.find('=') {
            let key = &args[..eq_pos];
            let value = &args[eq_pos + 1..];
            if !key.is_empty() { env::set(key, value); }
        } else {
            let mut val_buf = [0u8; 256];
            let len = env::get(args, &mut val_buf);
            if len == u32::MAX { env::set(args, ""); }
        }
    }

    fn cmd_unset(&self, args: &str, out: &mut dyn ShellOutput) {
        let key = args.trim();
        if key.is_empty() {
            out.write_line("Usage: unset VARIABLE");
            return;
        }
        env::unset(key);
    }

    // ── Builtin: alias/unalias ──────────────────────────────────────────

    fn cmd_alias(&mut self, args: &str, out: &mut dyn ShellOutput) {
        let args = args.trim();
        if args.is_empty() {
            if self.aliases.is_empty() {
                out.write_line("No aliases defined.");
            } else {
                for (name, val) in &self.aliases {
                    out.write_str("alias ");
                    out.write_str(name);
                    out.write_str("='");
                    out.write_str(val);
                    out.write_line("'");
                }
            }
            return;
        }
        if let Some(eq) = args.find('=') {
            let name = args[..eq].trim();
            let mut val = args[eq + 1..].trim();
            if (val.starts_with('\'') && val.ends_with('\''))
                || (val.starts_with('"') && val.ends_with('"'))
            {
                val = &val[1..val.len() - 1];
            }
            if name.is_empty() {
                out.write_line("alias: invalid name");
                return;
            }
            if let Some(existing) = self.aliases.iter_mut().find(|(n, _)| n == name) {
                existing.1 = String::from(val);
            } else {
                self.aliases.push((String::from(name), String::from(val)));
            }
        } else {
            if let Some((_, val)) = self.aliases.iter().find(|(n, _)| n == args) {
                out.write_str("alias ");
                out.write_str(args);
                out.write_str("='");
                out.write_str(val);
                out.write_line("'");
            } else {
                out.write_str("alias: '");
                out.write_str(args);
                out.write_line("' not found");
            }
        }
    }

    fn cmd_unalias(&mut self, args: &str, out: &mut dyn ShellOutput) {
        let name = args.trim();
        if name.is_empty() {
            out.write_line("usage: unalias <name>");
            return;
        }
        if name == "-a" {
            self.aliases.clear();
            return;
        }
        let before = self.aliases.len();
        self.aliases.retain(|(n, _)| n != name);
        if self.aliases.len() == before {
            out.write_str("unalias: '");
            out.write_str(name);
            out.write_line("' not found");
        }
    }

    // ── Builtin: source ─────────────────────────────────────────────────

    fn cmd_source(&mut self, args: &str, out: &mut dyn ShellOutput, app: &mut dyn AppBuiltins) {
        let path = args.trim();
        if path.is_empty() {
            out.write_line("usage: source <file>");
            return;
        }
        let mut data = [0u8; 4096];
        let total = read_file_to_buf(path, &mut data);
        if total == 0 {
            out.write_str("source: cannot read '");
            out.write_str(path);
            out.write_line("'");
            return;
        }
        let text = match core::str::from_utf8(&data[..total]) {
            Ok(s) => s,
            Err(_) => { out.write_line("source: invalid UTF-8"); return; }
        };
        for line in text.split('\n') {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') { continue; }
            let mut parts = line.splitn(2, ' ');
            let cmd = parts.next().unwrap_or("");
            let cmd_args = parts.next().unwrap_or("");
            match cmd {
                "export" => self.cmd_export(cmd_args, out),
                "set" => self.cmd_set(cmd_args, out),
                "unset" => self.cmd_unset(cmd_args, out),
                "alias" => self.cmd_alias(cmd_args, out),
                "unalias" => self.cmd_unalias(cmd_args, out),
                "cd" => self.cmd_cd(cmd_args, out, app),
                "echo" => out.write_line(cmd_args),
                "source" | "." => self.cmd_source(cmd_args, out, app),
                _ => {
                    if let Some(eq) = line.find('=') {
                        if !line[..eq].contains(' ') {
                            let key = line[..eq].trim();
                            let val = line[eq + 1..].trim();
                            if !key.is_empty() { env::set(key, val); }
                            continue;
                        }
                    }
                    let resolved = shell::resolve_cmd_path(cmd, &self.cwd);
                    let full_args = if cmd_args.is_empty() {
                        String::from(cmd)
                    } else {
                        format!("{} {}", cmd, cmd_args)
                    };
                    let tid = process::spawn(&resolved, &full_args);
                    if tid != u32::MAX { process::waitpid(tid); }
                }
            }
        }
    }

    // ── Builtin: su ─────────────────────────────────────────────────────

    pub fn do_su(username: &str, password: &str, out: &mut dyn ShellOutput) {
        if process::authenticate(username, password) {
            env::set("USER", username);
            let uid = process::getuid();
            if uid == 0 {
                env::set("HOME", "/");
            } else {
                let home = format!("/Users/{}", username);
                env::set("HOME", &home);
            }
            out.write_str("Switched to user '");
            out.write_str(username);
            out.write_line("'.");
        } else {
            out.write_str("su: authentication failed for '");
            out.write_str(username);
            out.write_line("'");
        }
    }

    // ── Builtin: jobs/fg/bg ─────────────────────────────────────────────

    fn cmd_jobs(&mut self, out: &mut dyn ShellOutput) {
        for job in &mut self.bg_jobs {
            if !job.finished {
                let status = process::try_waitpid(job.tid);
                if status == process::STOPPED {
                    job.stopped = true;
                } else if status != process::STILL_RUNNING {
                    job.finished = true;
                    job.exit_code = status;
                    if job.pipe_id != 0 { ipc::pipe_close(job.pipe_id); job.pipe_id = 0; }
                    if job.stdin_pipe != 0 { ipc::pipe_close(job.stdin_pipe); job.stdin_pipe = 0; }
                    for &p in &job.extra_pipes { ipc::pipe_close(p); }
                    job.extra_pipes.clear();
                }
            }
        }
        if self.bg_jobs.is_empty() {
            out.write_line("No jobs");
            return;
        }
        let last_id = self.bg_jobs.iter().rev().find(|j| !j.finished).map(|j| j.job_id).unwrap_or(0);
        for job in &self.bg_jobs {
            let marker = if job.job_id == last_id { "+" } else { "-" };
            let status = if job.finished {
                if job.exit_code == 0 { "Done" } else { "Exit" }
            } else if job.stopped {
                "Stopped"
            } else {
                "Running"
            };
            let suffix = if !job.finished && !job.stopped { " &" } else { "" };
            let cmd_display = if job.command.is_empty() {
                format!("TID {}", job.tid)
            } else {
                format!("{}{}", job.command, suffix)
            };
            let line = format!("[{}]{}  {:<20}{}\n", job.job_id, marker, status, cmd_display);
            out.write_str(&line);
        }
        self.bg_jobs.retain(|j| !j.finished);
    }

    fn cmd_fg(&mut self, out: &mut dyn ShellOutput) -> Vec<ShellAction> {
        if let Some(job) = self.bg_jobs.iter().rev().find(|j| !j.finished) {
            let tid = job.tid;
            let cmd = job.command.clone();
            let was_stopped = job.stopped;

            if !cmd.is_empty() { out.write_line(&cmd); }
            else { out.write_line(&format!("TID {}", tid)); }

            if was_stopped {
                process::send_signal(tid, process::SIGCONT);
            }
            for job in &mut self.bg_jobs {
                if job.tid == tid { job.finished = true; }
            }
            return actions_one(ShellAction::RunForeground {
                path: String::new(), // already running
                args: String::new(),
                command: cmd,
                redirect: None,
                input_data: None,
            });
        }
        out.write_line("fg: no current job");
        actions_one(ShellAction::Done)
    }

    fn cmd_bg(&mut self, out: &mut dyn ShellOutput) {
        if let Some(job) = self.bg_jobs.iter_mut().rev().find(|j| j.stopped && !j.finished) {
            let tid = job.tid;
            let job_id = job.job_id;
            job.stopped = false;
            process::send_signal(tid, process::SIGCONT);
            let msg = format!("[{}]  {} &\n", job_id, tid);
            out.write_str(&msg);
        } else {
            out.write_line("bg: no stopped job");
        }
    }

    // ── Background job reporting ────────────────────────────────────────

    pub fn check_bg_jobs(&mut self, out: &mut dyn ShellOutput) {
        for job in &mut self.bg_jobs {
            if !job.finished {
                let status = process::try_waitpid(job.tid);
                if status != process::STILL_RUNNING {
                    job.finished = true;
                    job.exit_code = status;
                    let done_str = if status == 0 { "Done" } else { "Exit" };
                    let msg = format!("[{}]  {}  {}\n", job.job_id, done_str, job.command);
                    out.write_str(&msg);
                }
            }
        }
        self.bg_jobs.retain(|j| !j.finished);
    }

    /// Register a background job spawned by the caller.
    pub fn add_bg_job(&mut self, tid: u32, command: &str) {
        let job_id = self.next_job_id;
        self.next_job_id += 1;
        self.bg_jobs.push(BackgroundJob {
            job_id, tid,
            command: String::from(command),
            finished: false, exit_code: 0, stopped: false,
            pipe_id: 0, stdin_pipe: 0, extra_pipes: Vec::new(),
        });
    }

    /// Register a stopped foreground process as a background job.
    pub fn add_stopped_job(&mut self, tid: u32, command: &str, pipe_id: u32, stdin_pipe: u32, extra_pipes: Vec<u32>) {
        let job_id = self.next_job_id;
        self.next_job_id += 1;
        self.bg_jobs.push(BackgroundJob {
            job_id, tid,
            command: String::from(command),
            finished: false, exit_code: 0, stopped: true,
            pipe_id, stdin_pipe, extra_pipes,
        });
    }
}
