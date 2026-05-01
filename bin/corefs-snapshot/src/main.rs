// Copyright (c) 2026 Mike Strathmann
// SPDX-License-Identifier: MIT

//! `corefs-snapshot` — snapshot lifecycle management for CoreFS volumes.
//!
//! ```text
//! corefs-snapshot --device <id> --capacity <bytes> list [--json]
//! corefs-snapshot --device <id> --capacity <bytes> create --name <name> [--scope <path>] [--json]
//! corefs-snapshot --device <id> --capacity <bytes> delete --id <n> [--json]
//! corefs-snapshot --device <id> --capacity <bytes> restore --id <n>
//! ```
//!
//! Operates über die service-freie `OdfDeviceSession` aus `corefs-core` und
//! mutiert direkt den `PersistedState`. **Inhaltsstandard ist
//! "metadata-only"**: `create` erfasst Inode-Metadaten (Pfad, Kind, Mode,
//! Timestamps), aber nicht den Datei-Body — das würde `read_file()` aus der
//! std-gebundenen `CoreFsService`-Schicht voraussetzen (decompress + decrypt).
//!
//! `restore` arbeitet gegen `PersistedState::restore_snapshot_at` und spielt
//! die erfassten Inode-Metadaten (plus optional file_data) zurück. Auf
//! metadata-only Snapshots verhält sich restore idempotent für die
//! block_records.

#![no_std]
#![no_main]

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use corefs_core::domain::snapshot::{Snapshot, SnapshotInode};
use corefs_core::platform::Timestamp;
use corefs_core::storage::ondisk::session::OdfDeviceSession;
use libcorefs_tools::args;
use libcorefs_tools::block_device::AnyOsBlockDevice;
use libcorefs_tools::error::{exit_code_for, ExitCode};
use libcorefs_tools::report::{JsonBuilder, Report};

anyos_std::entry!(main);

fn usage() {
    anyos_std::println!(
        "Usage:\n\
         corefs-snapshot --device <id> --capacity <bytes> list [--json]\n\
         corefs-snapshot --device <id> --capacity <bytes> create --name <name> [--scope <path>] [--json]\n\
         corefs-snapshot --device <id> --capacity <bytes> delete --id <n> [--json]\n\
         corefs-snapshot --device <id> --capacity <bytes> restore --id <n>"
    );
}

struct ListReport {
    snapshots: Vec<SnapshotEntry>,
}

struct SnapshotEntry {
    id: u64,
    name: String,
    scope_root: String,
    created_at_secs: u64,
    paths: usize,
    file_data_count: usize,
}

impl Report for ListReport {
    fn summary(&self) -> String {
        if self.snapshots.is_empty() {
            "snapshots: none".to_string()
        } else {
            format!("snapshots: {}", self.snapshots.len())
        }
    }
    fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str("snapshots\n---------\n");
        if self.snapshots.is_empty() {
            out.push_str("(none)\n");
        } else {
            for s in &self.snapshots {
                out.push_str(&format!(
                    "  id={:<4} name={:<20} scope={:<16} paths={:<5} files={}\n",
                    s.id, s.name, s.scope_root, s.paths, s.file_data_count
                ));
            }
        }
        out
    }
    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.key("snapshots");
        b.begin_array();
        for s in &self.snapshots {
            b.begin_object();
            b.kv_u64("id", s.id);
            b.kv_string("name", &s.name);
            b.kv_string("scope_root", &s.scope_root);
            b.kv_u64("created_at_secs", s.created_at_secs);
            b.kv_u64("paths", s.paths as u64);
            b.kv_u64("file_data_count", s.file_data_count as u64);
            b.end_object();
        }
        b.end_array();
        b.end_object();
        b.finish()
    }
}

struct CreateReport {
    id: u64,
    name: String,
    scope_root: String,
    paths: usize,
    metadata_only: bool,
}

impl Report for CreateReport {
    fn summary(&self) -> String {
        format!(
            "snapshot created: id={} name={} ({} paths{})",
            self.id,
            self.name,
            self.paths,
            if self.metadata_only {
                ", metadata-only"
            } else {
                ""
            }
        )
    }
    fn render_text(&self) -> String {
        format!(
            "snapshot created\n----------------\n\
             id          : {}\n\
             name        : {}\n\
             scope root  : {}\n\
             paths       : {}\n\
             content     : {}\n",
            self.id,
            self.name,
            self.scope_root,
            self.paths,
            if self.metadata_only {
                "metadata-only (file_data not captured — needs std-bound service layer)"
            } else {
                "full"
            }
        )
    }
    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("id", self.id);
        b.kv_string("name", &self.name);
        b.kv_string("scope_root", &self.scope_root);
        b.kv_u64("paths", self.paths as u64);
        b.kv_bool("metadata_only", self.metadata_only);
        b.end_object();
        b.finish()
    }
}

struct DeleteReport {
    id: u64,
}

struct RestoreReport {
    snapshot_id: u64,
    snapshot_name: String,
    restored_files: usize,
    restored_dirs: usize,
    skipped_paths: Vec<String>,
}

impl Report for RestoreReport {
    fn summary(&self) -> String {
        if self.skipped_paths.is_empty() {
            format!(
                "snapshot {} restored ({} files, {} dirs)",
                self.snapshot_id, self.restored_files, self.restored_dirs
            )
        } else {
            format!(
                "snapshot {} restored partially ({} files, {} dirs, {} skipped)",
                self.snapshot_id,
                self.restored_files,
                self.restored_dirs,
                self.skipped_paths.len()
            )
        }
    }
    fn render_text(&self) -> String {
        let mut out = String::new();
        out.push_str("snapshot restore\n----------------\n");
        out.push_str(&format!("id             : {}\n", self.snapshot_id));
        out.push_str(&format!("name           : {}\n", self.snapshot_name));
        out.push_str(&format!("restored files : {}\n", self.restored_files));
        out.push_str(&format!("restored dirs  : {}\n", self.restored_dirs));
        out.push_str(&format!("skipped paths  : {}\n", self.skipped_paths.len()));
        for p in &self.skipped_paths {
            out.push_str(&format!("  {}\n", p));
        }
        out
    }
    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("snapshot_id", self.snapshot_id);
        b.kv_string("snapshot_name", &self.snapshot_name);
        b.kv_u64("restored_files", self.restored_files as u64);
        b.kv_u64("restored_dirs", self.restored_dirs as u64);
        b.key("skipped_paths");
        b.begin_array();
        for p in &self.skipped_paths {
            b.string(p);
        }
        b.end_array();
        b.end_object();
        b.finish()
    }
}

impl Report for DeleteReport {
    fn summary(&self) -> String {
        format!("snapshot deleted: id={}", self.id)
    }
    fn render_text(&self) -> String {
        format!("snapshot deleted\n----------------\nid : {}\n", self.id)
    }
    fn render_json(&self) -> String {
        let mut b = JsonBuilder::new();
        b.begin_object();
        b.kv_u64("id", self.id);
        b.end_object();
        b.finish()
    }
}

fn main() -> u32 {
    let mut buf = [0u8; 256];
    let raw = anyos_std::process::args(&mut buf);
    let args = args::parse(raw);

    if args.has("help") {
        usage();
        return ExitCode::Success.as_u32();
    }

    let Some(device_id) = args::parse_device_id(&args) else {
        anyos_std::println!("corefs-snapshot: missing or invalid --device <id>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(capacity) = args::parse_capacity(&args) else {
        anyos_std::println!("corefs-snapshot: missing or invalid --capacity <bytes>");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let Some(sub) = args.positional_at(0) else {
        anyos_std::println!("corefs-snapshot: missing subcommand");
        usage();
        return ExitCode::InvalidArgument.as_u32();
    };
    let json = args::flag_present(&args, "json");

    let device = match AnyOsBlockDevice::open(device_id, capacity) {
        Ok(d) => d,
        Err(e) => {
            anyos_std::println!("corefs-snapshot: cannot open device {}: {}", device_id, e);
            return exit_code_for(&e).as_u32();
        }
    };
    let mut session = match OdfDeviceSession::open(Box::new(device)) {
        Ok(s) => s,
        Err(e) => {
            anyos_std::println!("corefs-snapshot: cannot hydrate volume: {}", e);
            return exit_code_for(&e).as_u32();
        }
    };

    match sub {
        "list" => {
            let entries: Vec<SnapshotEntry> = session
                .state()
                .snapshots
                .iter()
                .map(|s| SnapshotEntry {
                    id: s.id,
                    name: s.name.clone(),
                    scope_root: s.scope_root.clone(),
                    created_at_secs: s.created_at.as_secs(),
                    paths: s.paths.len(),
                    file_data_count: s.file_data.len(),
                })
                .collect();
            libcorefs_tools::report::print_report(&ListReport { snapshots: entries }, json);
            ExitCode::Success.as_u32()
        }
        "create" => {
            let Some(name) = args.get("name") else {
                anyos_std::println!("corefs-snapshot create: --name <name> is required");
                return ExitCode::InvalidArgument.as_u32();
            };
            let scope_root = args.get("scope").unwrap_or("/").to_string();
            let name_owned = name.to_string();
            let result = session.mutate(|state| {
                state.next_snapshot_id += 1;
                let snapshot_id = state.next_snapshot_id;
                let prefix = if scope_root == "/" {
                    String::new()
                } else {
                    format!("{}/", scope_root)
                };
                let paths: Vec<String> = state
                    .active_inodes
                    .iter()
                    .map(|i| i.path.clone())
                    .filter(|p| scope_root == "/" || *p == scope_root || p.starts_with(&prefix))
                    .collect();
                let mut snap_inodes: BTreeMap<String, SnapshotInode> = BTreeMap::new();
                for p in &paths {
                    if let Some(inode) = state.active_inodes.iter().find(|i| &i.path == p) {
                        snap_inodes.insert(
                            p.clone(),
                            SnapshotInode {
                                kind: inode.kind,
                                size: inode.size,
                                created_at: inode.created_at,
                                modified_at: inode.modified_at,
                                changed_at: inode.changed_at,
                                metadata: inode.metadata.clone(),
                                symlink_target: None,
                            },
                        );
                    }
                }
                let snapshot = Snapshot {
                    id: snapshot_id,
                    name: name_owned.clone(),
                    scope_root: scope_root.clone(),
                    created_at: Timestamp::EPOCH,
                    paths: paths.clone(),
                    file_data: BTreeMap::new(),
                    inodes: snap_inodes,
                };
                state.snapshots.push(snapshot);
                Ok((snapshot_id, paths.len()))
            });
            match result {
                Ok(((id, paths), _flush)) => {
                    libcorefs_tools::report::print_report(
                        &CreateReport {
                            id,
                            name: name.to_string(),
                            scope_root,
                            paths,
                            metadata_only: true,
                        },
                        json,
                    );
                    ExitCode::Success.as_u32()
                }
                Err(e) => {
                    anyos_std::println!("corefs-snapshot create: failed: {}", e);
                    exit_code_for(&e).as_u32()
                }
            }
        }
        "delete" => {
            let Some(id) = args.get_u64("id") else {
                anyos_std::println!("corefs-snapshot delete: --id <n> is required");
                return ExitCode::InvalidArgument.as_u32();
            };
            let result = session.mutate(|state| {
                let before = state.snapshots.len();
                state.snapshots.retain(|s| s.id != id);
                if state.snapshots.len() == before {
                    return Err(corefs_core::error::CoreFsError::NotFound(format!(
                        "snapshot id {id} does not exist"
                    )));
                }
                Ok(())
            });
            match result {
                Ok(((), _flush)) => {
                    libcorefs_tools::report::print_report(&DeleteReport { id }, json);
                    ExitCode::Success.as_u32()
                }
                Err(e) => {
                    anyos_std::println!("corefs-snapshot delete: {}", e);
                    exit_code_for(&e).as_u32()
                }
            }
        }
        "restore" => {
            let Some(id) = args.get_u64("id") else {
                anyos_std::println!("corefs-snapshot restore: --id <n> is required");
                return ExitCode::InvalidArgument.as_u32();
            };
            let result = session.mutate(|state| state.restore_snapshot_at(id, Timestamp::EPOCH));
            match result {
                Ok((report, _flush)) => {
                    libcorefs_tools::report::print_report(
                        &RestoreReport {
                            snapshot_id: report.snapshot_id,
                            snapshot_name: report.snapshot_name,
                            restored_files: report.restored_files,
                            restored_dirs: report.restored_dirs,
                            skipped_paths: report.skipped_paths,
                        },
                        json,
                    );
                    ExitCode::Success.as_u32()
                }
                Err(e) => {
                    anyos_std::println!("corefs-snapshot restore: {}", e);
                    exit_code_for(&e).as_u32()
                }
            }
        }
        other => {
            anyos_std::println!("corefs-snapshot: unknown subcommand '{}'", other);
            usage();
            ExitCode::InvalidArgument.as_u32()
        }
    }
}
