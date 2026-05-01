use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::{format, fs};

use crate::util::{
    append_line, copy_file, ensure_dir, ensure_parent_dirs, join_root, path_exists,
    remove_tree_files, strip_leading_slash,
};

const STATE_DIR: &str = "/.anyos-upgrade";
const PENDING_FILE: &str = "/.anyos-upgrade/pending";
const CHANGED_LOG: &str = "/.anyos-upgrade/changed.log";
const STAGED_LOG: &str = "/.anyos-upgrade/staged.log";

pub fn has_pending_upgrade(root: &str) -> bool {
    path_exists(&join_root(root, PENDING_FILE))
}

pub fn recover_pending_upgrade(root: &str) -> Result<u32, String> {
    if !has_pending_upgrade(root) {
        return Ok(0);
    }

    let state_root = join_root(root, STATE_DIR);
    let backup_root = join_root(root, "/.anyos-upgrade/backup");
    let staging_root = join_root(root, "/.anyos-upgrade/staging");
    let changed_log = join_root(root, CHANGED_LOG);
    let staged_log = join_root(root, STAGED_LOG);
    let marker_path = join_root(root, PENDING_FILE);

    let mut restored = 0u32;

    let changed = fs::read_to_string(&changed_log).unwrap_or_default();
    let mut lines: Vec<&str> = changed.lines().filter(|line| !line.is_empty()).collect();
    lines.reverse();

    for target in lines {
        let backup = format!(
            "{}/{}",
            backup_root.trim_end_matches('/'),
            strip_leading_slash(target)
        );
        if path_exists(&backup) {
            ensure_parent_dirs(target);
            copy_file(&backup, target)?;
            restored += 1;
        } else if path_exists(target) {
            let _ = fs::unlink(target);
            restored += 1;
        }
    }

    let staged = fs::read_to_string(&staged_log).unwrap_or_default();
    for staged_path in staged.lines().filter(|line| !line.is_empty()) {
        let _ = fs::unlink(staged_path);
    }

    remove_tree_files(&staging_root);
    remove_tree_files(&backup_root);
    remove_tree_files(&state_root);
    let _ = fs::unlink(&changed_log);
    let _ = fs::unlink(&staged_log);
    let _ = fs::unlink(&marker_path);
    Ok(restored)
}

pub struct UpgradeTransaction {
    state_root: String,
    backup_root: String,
    staging_root: String,
    marker_path: String,
    changed_log: String,
    staged_log: String,
    changed_files: Vec<String>,
    pending_boot_files: Vec<(String, String)>,
}

impl UpgradeTransaction {
    pub fn new(root: &str) -> Self {
        let state_root = join_root(root, STATE_DIR);
        let backup_root = join_root(root, "/.anyos-upgrade/backup");
        let staging_root = join_root(root, "/.anyos-upgrade/staging");
        let marker_path = join_root(root, PENDING_FILE);
        let changed_log = join_root(root, CHANGED_LOG);
        let staged_log = join_root(root, STAGED_LOG);
        Self {
            state_root,
            backup_root,
            staging_root,
            marker_path,
            changed_log,
            staged_log,
            changed_files: Vec::new(),
            pending_boot_files: Vec::new(),
        }
    }

    pub fn begin(&mut self) -> Result<(), String> {
        ensure_dir(&self.state_root);
        ensure_dir(&self.backup_root);
        ensure_dir(&self.staging_root);
        let _ = fs::unlink(&self.changed_log);
        let _ = fs::unlink(&self.staged_log);
        fs::write_bytes(&self.marker_path, b"pending\n")
            .map_err(|_| format!("failed to write {}", self.marker_path))
    }

    pub fn prepare_target(&mut self, target: &str) -> Result<(), String> {
        if self.backup_exists(target) || self.changed_files.iter().any(|p| p == target) {
            return Ok(());
        }
        if path_exists(target) {
            let backup = self.backup_path(target);
            ensure_parent_dirs(&backup);
            copy_file(target, &backup)?;
        }
        Ok(())
    }

    pub fn replace_file_from_path(&mut self, src: &str, dst: &str) -> Result<(), String> {
        self.prepare_target(dst)?;
        let staged = self.stage_path(dst);
        ensure_parent_dirs(&staged);
        copy_file(src, &staged)?;
        append_line(&self.staged_log, &staged)?;
        self.finish_staged_file(dst)
    }

    pub fn replace_file_with_bytes(&mut self, dst: &str, data: &[u8]) -> Result<(), String> {
        self.prepare_target(dst)?;
        let staged = self.stage_path(dst);
        ensure_parent_dirs(&staged);
        fs::write_bytes(&staged, data).map_err(|_| format!("failed to write {}", staged))?;
        append_line(&self.staged_log, &staged)?;
        self.finish_staged_file(dst)
    }

    pub fn commit(&mut self) -> Result<(), String> {
        let pending = core::mem::take(&mut self.pending_boot_files);
        for (staged, target) in pending {
            self.activate_staged(&staged, &target)?;
        }
        Ok(())
    }

    pub fn rollback(&mut self) -> u32 {
        let mut restored = 0u32;

        for (staged, _) in self.pending_boot_files.drain(..) {
            let _ = fs::unlink(&staged);
        }

        while let Some(target) = self.changed_files.pop() {
            let backup = self.backup_path(&target);
            if path_exists(&backup) {
                let _ = copy_file(&backup, &target);
                restored += 1;
            } else if path_exists(&target) {
                let _ = fs::unlink(&target);
                restored += 1;
            }
        }

        restored
    }

    pub fn finish(&self) {
        let _ = fs::unlink(&self.marker_path);
    }

    fn finish_staged_file(&mut self, target: &str) -> Result<(), String> {
        let staged = self.stage_path(target);
        if crate::util::is_boot_critical_path(target) {
            self.pending_boot_files.push((staged, String::from(target)));
            return Ok(());
        }
        self.activate_staged(&staged, target)
    }

    fn activate_staged(&mut self, staged: &str, target: &str) -> Result<(), String> {
        if fs::rename(staged, target) != 0 {
            return Err(format!("failed to activate {}", target));
        }
        if !self.changed_files.iter().any(|p| p == target) {
            self.changed_files.push(String::from(target));
            append_line(&self.changed_log, target)?;
        }
        Ok(())
    }

    fn backup_exists(&self, target: &str) -> bool {
        path_exists(&self.backup_path(target))
    }

    fn backup_path(&self, target: &str) -> String {
        format!(
            "{}/{}",
            self.backup_root.trim_end_matches('/'),
            strip_leading_slash(target)
        )
    }

    fn stage_path(&self, target: &str) -> String {
        format!(
            "{}/{}",
            self.staging_root.trim_end_matches('/'),
            strip_leading_slash(target)
        )
    }
}
