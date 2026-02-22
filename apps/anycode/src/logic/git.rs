use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;

use crate::util::path;

/// Status of a file tracked by git.
#[derive(Clone, Copy, PartialEq)]
pub enum FileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

/// A single changed file entry from `git status --porcelain`.
#[derive(Clone)]
pub struct ChangedFile {
    pub status: FileStatus,
    pub staged: bool,
    pub path: String,
}

/// Aggregated state of a git repository.
pub struct GitState {
    pub is_repo: bool,
    pub branch: String,
    pub changed_files: Vec<ChangedFile>,
}

impl GitState {
    pub fn empty() -> Self {
        Self {
            is_repo: false,
            branch: String::new(),
            changed_files: Vec::new(),
        }
    }

    pub fn staged_count(&self) -> usize {
        self.changed_files.iter().filter(|f| f.staged).count()
    }

    pub fn unstaged_count(&self) -> usize {
        self.changed_files.iter().filter(|f| !f.staged).count()
    }
}

/// The type of git operation currently running.
#[derive(Clone, Copy, PartialEq)]
pub enum GitOp {
    Status,
    Branch,
    Add,
    Commit,
    Push,
    Pull,
}

/// A running git command with pipe output capture.
/// Mirrors the BuildProcess pattern from build.rs.
pub struct GitProcess {
    pub tid: u32,
    pub pipe_id: u32,
    pub finished: bool,
    pub output: Vec<u8>,
}

impl GitProcess {
    /// Spawn a git command with stdout piped.
    pub fn spawn(git_path: &str, args: &str) -> Option<Self> {
        if git_path.is_empty() {
            return None;
        }
        let pipe_id = anyos_std::ipc::pipe_create("anycode:git");
        if pipe_id == 0 {
            return None;
        }
        let full_args = format!("git {}", args);
        let tid = anyos_std::process::spawn_piped(git_path, &full_args, pipe_id);
        if tid == u32::MAX {
            anyos_std::ipc::pipe_close(pipe_id);
            return None;
        }
        Some(Self {
            tid,
            pipe_id,
            finished: false,
            output: Vec::new(),
        })
    }

    /// Poll for new output. Accumulates into self.output.
    pub fn poll(&mut self) {
        if self.finished {
            return;
        }
        let mut buf = [0u8; 1024];
        loop {
            let n = anyos_std::ipc::pipe_read(self.pipe_id, &mut buf);
            if n == 0 || n == u32::MAX {
                break;
            }
            self.output.extend_from_slice(&buf[..n as usize]);
        }
    }

    /// Check if the git command has finished. Returns Some(exit_code) if done.
    pub fn check_finished(&mut self) -> Option<u32> {
        if self.finished {
            return Some(0);
        }
        let status = anyos_std::process::try_waitpid(self.tid);
        if status != anyos_std::process::STILL_RUNNING && status != u32::MAX {
            self.finished = true;
            self.poll();
            anyos_std::ipc::pipe_close(self.pipe_id);
            Some(status)
        } else {
            None
        }
    }

    /// Get the collected output as a string.
    pub fn output_str(&self) -> &str {
        core::str::from_utf8(&self.output).unwrap_or("")
    }
}

/// Check if a directory is a git repository by looking for `.git/`.
pub fn is_git_repo(root: &str) -> bool {
    let git_dir = format!("{}/.git", root);
    path::is_directory(&git_dir)
}

/// Parse the output of `git status --porcelain` into a list of changed files.
pub fn parse_status_porcelain(output: &str) -> Vec<ChangedFile> {
    let mut files = Vec::new();
    for line in output.split('\n') {
        if line.len() < 4 {
            continue;
        }
        let bytes = line.as_bytes();
        let index_status = bytes[0];
        let worktree_status = bytes[1];
        let file_path = String::from(&line[3..]);

        // Staged change (index column has a non-space, non-? character)
        if index_status != b' ' && index_status != b'?' {
            files.push(ChangedFile {
                status: char_to_status(index_status),
                staged: true,
                path: file_path.clone(),
            });
        }
        // Working tree change
        if worktree_status != b' ' {
            files.push(ChangedFile {
                status: if worktree_status == b'?' {
                    FileStatus::Untracked
                } else {
                    char_to_status(worktree_status)
                },
                staged: false,
                path: file_path,
            });
        }
    }
    files
}

fn char_to_status(c: u8) -> FileStatus {
    match c {
        b'M' => FileStatus::Modified,
        b'A' => FileStatus::Added,
        b'D' => FileStatus::Deleted,
        b'R' => FileStatus::Renamed,
        b'U' => FileStatus::Conflicted,
        b'?' => FileStatus::Untracked,
        _ => FileStatus::Modified,
    }
}

/// Parse the output of `git branch --show-current`.
pub fn parse_branch(output: &str) -> String {
    let trimmed = output.trim();
    String::from(trimmed)
}
