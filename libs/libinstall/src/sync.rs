use alloc::string::String;
use anyos_std::format;
use anyos_std::fs;
use anyos_std::json::Value;

use crate::merge::merge_config_file;
use crate::model::{ApplyStats, UpgradeList, UpgradeOperation};
use crate::transaction::UpgradeTransaction;
use crate::util::{ensure_dir, ensure_parent_dirs, is_config_path, join_root, path_exists, read_dir};

pub fn load_upgrade_list(path: &str) -> Option<UpgradeList> {
    let text = fs::read_to_string(path).ok()?;
    let value = Value::parse(&text).ok()?;
    let ops = value["operations"].as_array()?;
    let mut operations = alloc::vec::Vec::new();

    for op in ops {
        let action = op["action"].as_str().unwrap_or("");
        match action {
            "ensure_dir" => {
                let path = op["path"].as_str()?;
                operations.push(UpgradeOperation::EnsureDir {
                    path: String::from(path),
                });
            }
            "sync_tree" => {
                let source = op["source"].as_str()?;
                let target = op["target"].as_str()?;
                operations.push(UpgradeOperation::SyncTree {
                    source: String::from(source),
                    target: String::from(target),
                    merge_configs: op["merge_configs"].as_bool().unwrap_or(false),
                    preserve_existing: op["preserve_existing"].as_bool().unwrap_or(false),
                });
            }
            _ => {}
        }
    }

    if operations.is_empty() {
        return None;
    }

    Some(UpgradeList {
        from_version: String::from(value["from_version"].as_str().unwrap_or("0.0.0")),
        to_version: String::from(value["to_version"].as_str().unwrap_or("0.0.0")),
        operations,
    })
}

pub fn apply_upgrade_list(root: &str, list: &UpgradeList) -> Result<ApplyStats, String> {
    let mut stats = ApplyStats::default();
    let mut tx = UpgradeTransaction::new(root);
    tx.begin()?;

    let result: Result<(), String> = (|| {
        for op in &list.operations {
            match op {
                UpgradeOperation::EnsureDir { path } => {
                    ensure_dir(&join_root(root, path));
                    stats.dirs_created += 1;
                }
                UpgradeOperation::SyncTree {
                    source,
                    target,
                    merge_configs,
                    preserve_existing,
                } => {
                    let dst = join_root(root, target);
                    ensure_dir(&dst);
                    sync_tree(source, &dst, *merge_configs, *preserve_existing, &mut stats, &mut tx)?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    })();

    match result {
        Ok(()) => {
            tx.finish();
            Ok(stats)
        }
        Err(err) => {
            let rolled_back = tx.rollback();
            tx.finish();
            Err(format!("{} (rolled back {} files)", err, rolled_back))
        }
    }
}

fn sync_tree(
    src: &str,
    dst: &str,
    merge_configs: bool,
    preserve_existing: bool,
    stats: &mut ApplyStats,
    tx: &mut UpgradeTransaction,
) -> Result<(), String> {
    for entry in read_dir(src)? {
        let child_src = format!("{}/{}", src.trim_end_matches('/'), entry.name);
        let child_dst = format!("{}/{}", dst.trim_end_matches('/'), entry.name);
        if entry.entry_type == 1 {
            ensure_dir(&child_dst);
            stats.dirs_created += 1;
            sync_tree(&child_src, &child_dst, merge_configs, preserve_existing, stats, tx)?;
            continue;
        }

        if path_exists(&child_dst) {
            if preserve_existing {
                stats.files_preserved += 1;
                continue;
            }
            if merge_configs && is_config_path(&child_dst) {
                merge_config_file(&child_src, &child_dst, tx)?;
                stats.configs_merged += 1;
                continue;
            }
        }

        ensure_parent_dirs(&child_dst);
        tx.replace_file_from_path(&child_src, &child_dst)?;
        stats.files_copied += 1;
    }

    Ok(())
}
