//! std::process compatible process management.

use crate::io::{self, Read, Write};
use crate::path::Path;
use alloc::string::String;
use alloc::vec::Vec;

/// Exit the current process.
pub fn exit(code: i32) -> ! {
    anyos_std::process::exit(code as u32);
}

/// Abort the current process.
pub fn abort() -> ! {
    anyos_std::process::exit(255);
}

/// Get the current process ID.
pub fn id() -> u32 {
    anyos_std::process::getpid()
}

/// A child process.
pub struct Child {
    tid: u32,
    pub stdin: Option<ChildStdin>,
    pub stdout: Option<ChildStdout>,
    pub stderr: Option<ChildStderr>,
}

impl Child {
    /// Wait for the child to exit.
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let code = anyos_std::process::waitpid(self.tid);
        Ok(ExitStatus { code })
    }

    /// Try to wait without blocking.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let ret = anyos_std::process::try_waitpid(self.tid);
        if ret == anyos_std::process::STILL_RUNNING || ret == u32::MAX {
            Ok(None)
        } else {
            Ok(Some(ExitStatus { code: ret }))
        }
    }

    /// Kill the child process.
    pub fn kill(&mut self) -> io::Result<()> {
        let ret = anyos_std::process::kill(self.tid);
        if ret == u32::MAX {
            Err(io::Error::from_kind(io::ErrorKind::NotFound))
        } else {
            Ok(())
        }
    }

    /// Get the child's PID.
    pub fn id(&self) -> u32 {
        self.tid
    }

    /// Wait and collect output.
    pub fn wait_with_output(mut self) -> io::Result<Output> {
        let mut stdout_data = Vec::new();
        let mut stderr_data = Vec::new();

        if let Some(ref mut stdout) = self.stdout {
            stdout.read_to_end(&mut stdout_data)?;
        }
        if let Some(ref mut stderr) = self.stderr {
            stderr.read_to_end(&mut stderr_data)?;
        }

        let status = self.wait()?;
        Ok(Output {
            status,
            stdout: stdout_data,
            stderr: stderr_data,
        })
    }
}

/// Process exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitStatus {
    code: u32,
}

impl ExitStatus {
    pub fn success(&self) -> bool {
        self.code == 0
    }

    pub fn code(&self) -> Option<i32> {
        Some(self.code as i32)
    }
}

impl core::fmt::Display for ExitStatus {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "exit status: {}", self.code)
    }
}

/// Process exit code (for use as main return type).
#[derive(Debug, Clone, Copy)]
pub struct ExitCode(u8);

impl ExitCode {
    pub const SUCCESS: ExitCode = ExitCode(0);
    pub const FAILURE: ExitCode = ExitCode(1);
}

impl From<u8> for ExitCode {
    fn from(code: u8) -> Self {
        ExitCode(code)
    }
}

/// Output of a finished process.
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Builder for spawning processes.
pub struct Command {
    program: String,
    args: Vec<String>,
}

impl Command {
    pub fn new<S: AsRef<str>>(program: S) -> Self {
        Command {
            program: String::from(program.as_ref()),
            args: Vec::new(),
        }
    }

    pub fn arg<S: AsRef<str>>(&mut self, arg: S) -> &mut Self {
        self.args.push(String::from(arg.as_ref()));
        self
    }

    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for arg in args {
            self.args.push(String::from(arg.as_ref()));
        }
        self
    }

    pub fn spawn(&mut self) -> io::Result<Child> {
        let args_str = self.args.join(" ");
        let tid = anyos_std::process::spawn(&self.program, &args_str);
        if tid == 0 || tid == u32::MAX {
            return Err(io::Error::from_kind(io::ErrorKind::NotFound));
        }
        Ok(Child {
            tid,
            stdin: None,
            stdout: None,
            stderr: None,
        })
    }

    pub fn output(&mut self) -> io::Result<Output> {
        let child = self.spawn()?;
        child.wait_with_output()
    }

    pub fn status(&mut self) -> io::Result<ExitStatus> {
        let mut child = self.spawn()?;
        child.wait()
    }

    pub fn current_dir<P: AsRef<Path>>(&mut self, _dir: P) -> &mut Self {
        self // Not directly supported
    }

    pub fn env<K: AsRef<str>, V: AsRef<str>>(&mut self, _key: K, _val: V) -> &mut Self {
        self // Not directly supported per-child
    }

    pub fn stdin(&mut self, _cfg: Stdio) -> &mut Self {
        self
    }
    pub fn stdout(&mut self, _cfg: Stdio) -> &mut Self {
        self
    }
    pub fn stderr(&mut self, _cfg: Stdio) -> &mut Self {
        self
    }
}

/// Stdio configuration.
#[derive(Debug)]
pub struct Stdio(u8);

impl Stdio {
    pub fn piped() -> Self {
        Stdio(0)
    }
    pub fn inherit() -> Self {
        Stdio(1)
    }
    pub fn null() -> Self {
        Stdio(2)
    }
}

/// Child stdin handle.
pub struct ChildStdin {
    _private: (),
}
impl Write for ChildStdin {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Child stdout handle.
pub struct ChildStdout {
    _private: (),
}
impl Read for ChildStdout {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Ok(0)
    }
}

/// Child stderr handle.
pub struct ChildStderr {
    _private: (),
}
impl Read for ChildStderr {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        Ok(0)
    }
}
