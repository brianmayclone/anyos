#![no_std]
#![no_main]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::{fs, i18n, process, sys};
use libanyui_client as ui;
use libconf_schema::{default_string, manifest, RegistryScope, ServiceSchema};
use libdb_client::Database;

anyos_std::entry!(main);

const WIN_W: u32 = 720;
const WIN_H: u32 = 360;
const STATUS_H: u32 = 28;

const CONF_DB_PATH: &str = "/System/sysdb/config.db";
const RESTORE_TMP_PATH: &str = "/System/sysdb/config.restore.tmp";
const PRE_RESTORE_PATH: &str = "/System/sysdb/config.db.pre-restore";
const RESTORE_STAGE_DIR: &str = "/System/sysdb/backuprestore-stage";
const PRE_RESTORE_EXTERNAL_ROOT: &str = "/System/sysdb/external.pre-restore";

const DB_ARCHIVE_ENTRY: &str = "config.db";
const MANIFEST_ARCHIVE_ENTRY: &str = "manifest.txt";
const EXTERNAL_ARCHIVE_ROOT: &str = "external";
const VALUE_TYPE_EXTERNAL_REF: i64 = 4;

const HISTORY_DIRS: &[&str] = &["history"];
const HISTORY_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("history/last_backup_at", ""),
    default_string("history/last_backup_path", ""),
    default_string("history/last_restore_at", ""),
    default_string("history/last_restore_path", ""),
];
const HISTORY_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "system/utilities/backuprestore",
    RegistryScope::User,
    1,
    HISTORY_DIRS,
    HISTORY_DEFAULTS,
    &[],
);

struct App {
    status: ui::Label,
    last_backup: ui::Label,
    last_restore: ui::Label,
}

struct ExternalRefEntry {
    logical_path: String,
    target_path: String,
    archive_path: String,
    is_dir: bool,
}

struct SkippedExternalRef {
    logical_path: String,
    target_path: String,
}

struct BackupOutcome {
    archive_path: String,
    exported_refs: usize,
    skipped_refs: Vec<SkippedExternalRef>,
}

struct AppliedRestore {
    target_path: String,
    backup_path: String,
    had_existing: bool,
}

anyos_std::global_app_state!(App);

fn history_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("backuprestore", &HISTORY_MANIFEST)
}

fn main() {
    if !ui::init() {
        anyos_std::println!("backuprestore: failed to load libanyui.so");
        return;
    }
    i18n::init();
    let _ = history_schema().register();

    let tc = ui::theme::colors();
    let win = ui::Window::new("Backup & Restore", -1, -1, WIN_W, WIN_H);
    win.set_color(tc.window_bg);

    let status_bar = ui::View::new();
    status_bar.set_dock(ui::DOCK_BOTTOM);
    status_bar.set_size(WIN_W, STATUS_H);
    status_bar.set_color(tc.toolbar_bg);
    status_bar.set_padding(10, 6, 10, 6);
    win.add(&status_bar);

    let status = ui::Label::new("Ready.");
    status.set_position(10, 6);
    status.set_size(WIN_W - 20, 16);
    status.set_text_color(tc.text_secondary);
    status.set_font_size(12);
    status_bar.add(&status);

    let root = ui::View::new();
    root.set_dock(ui::DOCK_FILL);
    root.set_color(tc.window_bg);
    win.add(&root);

    let title = ui::Label::new("Backup & Restore");
    title.set_position(28, 28);
    title.set_size(360, 34);
    title.set_font_size(24);
    title.set_text_color(tc.text);
    root.add(&title);

    let subtitle = ui::Label::new(
        "Create a backup of your configuration and app data references, or restore a previous one.",
    );
    subtitle.set_position(28, 68);
    subtitle.set_size(640, 22);
    subtitle.set_font_size(13);
    subtitle.set_text_color(tc.text_secondary);
    root.add(&subtitle);

    let note = ui::Label::new("Restoring will restart the system when finished.");
    note.set_position(28, 94);
    note.set_size(420, 18);
    note.set_font_size(12);
    note.set_text_color(tc.text_secondary);
    root.add(&note);

    let card = ui::View::new();
    card.set_position(28, 132);
    card.set_size(664, 150);
    card.set_color(tc.card_bg);
    root.add(&card);

    let card_title = ui::Label::new("Choose an action");
    card_title.set_position(24, 22);
    card_title.set_size(240, 24);
    card_title.set_font_size(16);
    card_title.set_text_color(tc.text);
    card.add(&card_title);

    let card_text = ui::Label::new(
        "Backups include the confd database and all reachable ExternalRef targets.",
    );
    card_text.set_position(24, 50);
    card_text.set_size(560, 20);
    card_text.set_font_size(12);
    card_text.set_text_color(tc.text_secondary);
    card.add(&card_text);

    let btn_backup = ui::Button::new("Create Backup");
    btn_backup.set_position(24, 90);
    btn_backup.set_size(184, 40);
    card.add(&btn_backup);

    let btn_restore = ui::Button::new("Restore Backup");
    btn_restore.set_position(222, 90);
    btn_restore.set_size(184, 40);
    card.add(&btn_restore);

    let last_backup = ui::Label::new("");
    last_backup.set_position(28, 302);
    last_backup.set_size(640, 18);
    last_backup.set_font_size(11);
    last_backup.set_text_color(tc.text_secondary);
    root.add(&last_backup);

    let last_restore = ui::Label::new("");
    last_restore.set_position(28, 322);
    last_restore.set_size(640, 18);
    last_restore.set_font_size(11);
    last_restore.set_text_color(tc.text_secondary);
    root.add(&last_restore);

    unsafe {
        APP = Some(App {
            status,
            last_backup,
            last_restore,
        });
    }

    refresh_ui();

    btn_backup.on_click(|_| do_backup());
    btn_restore.on_click(|_| do_restore());

    win.on_close(|_| ui::quit());
    ui::run();
}

fn set_status(text: &str) {
    app().status.set_text(text);
}

fn refresh_ui() {
    let mut stat = [0u32; 7];
    if fs::stat(CONF_DB_PATH, &mut stat) == 0 {
        set_status("Ready.");
    } else {
        set_status("Configuration database not found.");
    }

    let schema = history_schema();
    let backup_at = schema.read_string("history/last_backup_at").unwrap_or_default();
    let backup_path = schema.read_string("history/last_backup_path").unwrap_or_default();
    let restore_at = schema.read_string("history/last_restore_at").unwrap_or_default();
    let restore_path = schema.read_string("history/last_restore_path").unwrap_or_default();

    app().last_backup.set_text(&history_line("Last backup", &backup_at, &backup_path));
    app().last_restore.set_text(&history_line("Last restore", &restore_at, &restore_path));
}

fn history_line(prefix: &str, at: &str, path: &str) -> String {
    if at.is_empty() {
        format!("{}: Never", prefix)
    } else if path.is_empty() {
        format!("{}: {}", prefix, at)
    } else {
        format!("{}: {}  •  {}", prefix, at, path)
    }
}

fn do_backup() {
    if !ensure_libraries() {
        return;
    }

    let mut stat = [0u32; 7];
    if fs::stat(CONF_DB_PATH, &mut stat) != 0 {
        ui::MessageBox::show(
            ui::MessageBoxType::Warning,
            "The configuration database could not be found.",
            Some("OK"),
        );
        refresh_ui();
        return;
    }

    let Some(path) = ui::FileDialog::save_file(&default_backup_name()) else {
        return;
    };

    set_status("Creating backup...");
    match create_backup_archive(&path) {
        Ok(outcome) => {
            record_backup(&outcome.archive_path);
            refresh_ui();
            if outcome.skipped_refs.is_empty() {
                set_status("Backup completed.");
                ui::MessageBox::show(
                    ui::MessageBoxType::Info,
                    "Backup created successfully.",
                    Some("OK"),
                );
            } else {
                set_status("Backup completed with warnings.");
                ui::MessageBox::show(
                    ui::MessageBoxType::Warning,
                    &backup_warning_text(&outcome),
                    Some("OK"),
                );
            }
        }
        Err(err) => {
            set_status("Backup failed.");
            ui::MessageBox::show(
                ui::MessageBoxType::Alert,
                &format!("Backup failed.\n\n{}", err),
                Some("OK"),
            );
        }
    }
}

fn do_restore() {
    if !ensure_libraries() {
        return;
    }

    let Some(path) = ui::FileDialog::open_file() else {
        return;
    };

    set_status("Preparing restore...");
    if !run_svc_command("stop", "confd") {
        ui::MessageBox::show(
            ui::MessageBoxType::Alert,
            "Could not stop confd for restore.",
            Some("OK"),
        );
        return;
    }
    process::sleep(300);

    match restore_archive(&path) {
        Ok(summary) => {
            record_restore(&path);
            set_status("Restore completed. Restarting system...");
            ui::MessageBox::show(ui::MessageBoxType::Info, &summary, Some("Restart"));
            process::sleep(200);
            process::reboot();
        }
        Err(err) => {
            let _ = run_svc_command("start", "confd");
            process::sleep(300);
            refresh_ui();
            set_status("Restore failed.");
            ui::MessageBox::show(
                ui::MessageBoxType::Alert,
                &format!("Restore failed.\n\n{}", err),
                Some("OK"),
            );
        }
    }
}

fn record_backup(path: &str) {
    let schema = history_schema();
    let _ = schema.write_string("history/last_backup_at", &now_string());
    let _ = schema.write_string("history/last_backup_path", path);
}

fn record_restore(path: &str) {
    let schema = history_schema();
    let _ = schema.write_string("history/last_restore_at", &now_string());
    let _ = schema.write_string("history/last_restore_path", path);
}

fn now_string() -> String {
    let mut buf = [0u8; 8];
    sys::time(&mut buf);
    let year = buf[0] as u16 | ((buf[1] as u16) << 8);
    let month = buf[2];
    let day = buf[3];
    let hour = buf[4];
    let min = buf[5];
    format!("{:04}-{:02}-{:02} {:02}:{:02}", year, month, day, hour, min)
}

fn create_backup_archive(path: &str) -> Result<BackupOutcome, String> {
    let (refs, skipped_refs) = collect_external_refs()?;
    let db_bytes = fs::read_to_vec(CONF_DB_PATH)
        .map_err(|_| String::from("Could not read the configuration database."))?;
    let writer = libzip_client::TarWriter::new()
        .ok_or_else(|| String::from("Could not create the backup archive."))?;

    if !writer.add_file(DB_ARCHIVE_ENTRY, &db_bytes) {
        return Err(String::from("Could not add the configuration database to the backup."));
    }

    if !refs.is_empty() && !writer.add_dir("external/") {
        return Err(String::from("Could not prepare the external data section in the backup."));
    }

    for entry in &refs {
        add_external_ref_to_archive(&writer, entry)?;
    }

    let manifest = build_manifest(&refs);
    if !writer.add_file(MANIFEST_ARCHIVE_ENTRY, manifest.as_bytes()) {
        return Err(String::from("Could not add the backup manifest."));
    }

    if !writer.write_to_file(path, true) {
        return Err(String::from("Could not write the backup file."));
    }

    Ok(BackupOutcome {
        archive_path: String::from(path),
        exported_refs: refs.len(),
        skipped_refs,
    })
}

fn backup_warning_text(outcome: &BackupOutcome) -> String {
    let count = outcome.skipped_refs.len();
    let mut text = format!(
        "Backup created, but {} reference{} could not be exported.\n\nThis can lead to missing app data when restoring this backup.",
        count,
        if count == 1 { "" } else { "s" },
    );
    if let Some(first) = outcome.skipped_refs.first() {
        text.push_str("\n\nExample target:\n");
        text.push_str(&first.target_path);
    }
    text
}

fn restore_archive(archive_path: &str) -> Result<String, String> {
    cleanup_restore_artifacts();

    let reader = libzip_client::TarReader::open(archive_path)
        .ok_or_else(|| String::from("Could not open the selected backup."))?;
    let db_index = find_archive_entry(&reader, DB_ARCHIVE_ENTRY)
        .ok_or_else(|| String::from("The selected backup is incomplete."))?;
    let manifest_index = find_archive_entry(&reader, MANIFEST_ARCHIVE_ENTRY)
        .ok_or_else(|| String::from("The selected backup is incomplete."))?;

    let manifest_bytes = reader
        .extract(manifest_index)
        .ok_or_else(|| String::from("Could not read the backup manifest."))?;
    let manifest_text = core::str::from_utf8(&manifest_bytes)
        .map_err(|_| String::from("The backup manifest is invalid."))?;
    let refs = parse_manifest(manifest_text)?;

    if !reader.extract_to_file(db_index, RESTORE_TMP_PATH) {
        return Err(String::from("Could not extract the configuration database from the backup."));
    }

    extract_external_stage(&reader)?;

    let mut config_replaced = false;
    let mut had_existing_config = false;
    let mut applied = Vec::new();

    let result = (|| -> Result<(), String> {
        let mut stat = [0u32; 7];
        had_existing_config = fs::stat(CONF_DB_PATH, &mut stat) == 0;
        if had_existing_config && fs::rename(CONF_DB_PATH, PRE_RESTORE_PATH) != 0 {
            return Err(String::from("Could not prepare the current configuration for restore."));
        }

        if fs::rename(RESTORE_TMP_PATH, CONF_DB_PATH) != 0 {
            if had_existing_config {
                let _ = fs::rename(PRE_RESTORE_PATH, CONF_DB_PATH);
            }
            return Err(String::from("Could not replace the current configuration database."));
        }
        config_replaced = true;

        for entry in &refs {
            apply_external_restore(entry, &mut applied)?;
        }
        Ok(())
    })();

    if let Err(err) = result {
        rollback_external_restore(&applied);
        if config_replaced {
            let _ = fs::unlink(CONF_DB_PATH);
            if had_existing_config {
                let _ = fs::rename(PRE_RESTORE_PATH, CONF_DB_PATH);
            }
        }
        let _ = fs::unlink(RESTORE_TMP_PATH);
        let _ = remove_tree(RESTORE_STAGE_DIR);
        return Err(err);
    }

    let _ = remove_tree(RESTORE_STAGE_DIR);

    Ok(String::from(
        "Restore completed.\n\nThe selected backup was restored successfully.\nThe system will restart now.",
    ))
}

fn ensure_libraries() -> bool {
    if !libzip_client::init() {
        set_status("libzip.so unavailable.");
        ui::MessageBox::show(
            ui::MessageBoxType::Alert,
            "libzip.so could not be loaded.",
            Some("OK"),
        );
        return false;
    }
    if !libdb_client::init() {
        set_status("libdb.so unavailable.");
        ui::MessageBox::show(
            ui::MessageBoxType::Alert,
            "libdb.so could not be loaded.",
            Some("OK"),
        );
        return false;
    }
    true
}

fn collect_external_refs() -> Result<(Vec<ExternalRefEntry>, Vec<SkippedExternalRef>), String> {
    let db = Database::open(CONF_DB_PATH)
        .ok_or_else(|| String::from("Could not open the configuration database."))?;
    let result = db
        .query(&format!(
            "SELECT logical_path, value_text FROM registry WHERE value_type = {} ORDER BY logical_path",
            VALUE_TYPE_EXTERNAL_REF
        ))
        .map_err(|err| format!("Could not read ExternalRef entries: {}", err))?;

    let mut refs = Vec::new();
    let mut skipped = Vec::new();
    for row in 0..result.row_count() {
        let logical_path = result.get_text(row, 0).unwrap_or_default();
        let target_path = result.get_text(row, 1).unwrap_or_default();
        if target_path.is_empty() || refs.iter().any(|item: &ExternalRefEntry| item.target_path == target_path) {
            continue;
        }

        let mut stat = [0u32; 7];
        if fs::stat(&target_path, &mut stat) != 0 {
            skipped.push(SkippedExternalRef {
                logical_path,
                target_path,
            });
            continue;
        }

        let index = refs.len();
        refs.push(ExternalRefEntry {
            logical_path,
            target_path,
            archive_path: format!("{}/{:03}", EXTERNAL_ARCHIVE_ROOT, index),
            is_dir: stat[0] == 1,
        });
    }

    Ok((refs, skipped))
}

fn add_external_ref_to_archive(
    writer: &libzip_client::TarWriter,
    entry: &ExternalRefEntry,
) -> Result<(), String> {
    if entry.is_dir {
        add_dir_tree_to_archive(writer, &entry.target_path, &entry.archive_path)
    } else {
        let root_dir = format!("{}/", entry.archive_path);
        if !writer.add_dir(&root_dir) {
            return Err(String::from("Could not prepare a referenced file for backup."));
        }
        let bytes = fs::read_to_vec(&entry.target_path)
            .map_err(|_| format!("Could not read referenced file {}.", entry.target_path))?;
        let archive_file = format!("{}/payload", entry.archive_path);
        if !writer.add_file(&archive_file, &bytes) {
            return Err(format!("Could not add {} to the backup.", entry.target_path));
        }
        Ok(())
    }
}

fn add_dir_tree_to_archive(
    writer: &libzip_client::TarWriter,
    source_dir: &str,
    archive_dir: &str,
) -> Result<(), String> {
    let archive_entry = format!("{}/", archive_dir);
    if !writer.add_dir(&archive_entry) {
        return Err(String::from("Could not prepare a referenced folder for backup."));
    }

    let entries = fs::read_dir(source_dir)
        .map_err(|_| format!("Could not read referenced directory {}.", source_dir))?;
    for child in entries {
        if child.name == "." || child.name == ".." {
            continue;
        }
        let source_path = join_fs_path(source_dir, &child.name);
        let child_archive = format!("{}/{}", archive_dir, child.name);
        if child.is_dir() {
            add_dir_tree_to_archive(writer, &source_path, &child_archive)?;
        } else {
            let bytes = fs::read_to_vec(&source_path)
                .map_err(|_| format!("Could not read referenced file {}.", source_path))?;
            if !writer.add_file(&child_archive, &bytes) {
                return Err(format!("Could not add {} to the backup.", source_path));
            }
        }
    }
    Ok(())
}

fn build_manifest(refs: &[ExternalRefEntry]) -> String {
    let mut out = String::from("ANYOS-BACKUP-V2\n");
    for entry in refs {
        out.push_str("REF\t");
        out.push_str(&escape_manifest_field(&entry.logical_path));
        out.push('\t');
        out.push_str(if entry.is_dir { "dir" } else { "file" });
        out.push('\t');
        out.push_str(&escape_manifest_field(&entry.target_path));
        out.push('\t');
        out.push_str(&escape_manifest_field(&entry.archive_path));
        out.push('\n');
    }
    out
}

fn parse_manifest(text: &str) -> Result<Vec<ExternalRefEntry>, String> {
    let mut lines = text.lines();
    match lines.next() {
        Some("ANYOS-BACKUP-V2") => {}
        _ => return Err(String::from("The backup manifest has an unknown format.")),
    }

    let mut refs = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut fields = line.split('\t');
        if fields.next() != Some("REF") {
            continue;
        }
        let logical_path = fields
            .next()
            .map(unescape_manifest_field)
            .ok_or_else(|| String::from("The backup manifest is incomplete."))?;
        let kind = fields
            .next()
            .ok_or_else(|| String::from("The backup manifest is incomplete."))?;
        let target_path = fields
            .next()
            .map(unescape_manifest_field)
            .ok_or_else(|| String::from("The backup manifest is incomplete."))?;
        let archive_path = fields
            .next()
            .map(unescape_manifest_field)
            .ok_or_else(|| String::from("The backup manifest is incomplete."))?;

        refs.push(ExternalRefEntry {
            logical_path,
            target_path,
            archive_path,
            is_dir: kind == "dir",
        });
    }

    Ok(refs)
}

fn extract_external_stage(reader: &libzip_client::TarReader) -> Result<(), String> {
    let _ = remove_tree(RESTORE_STAGE_DIR);
    ensure_dir(RESTORE_STAGE_DIR);

    for index in 0..reader.entry_count() {
        let name = reader.entry_name(index);
        if !name.starts_with("external/") {
            continue;
        }
        let stage_path = join_fs_path(RESTORE_STAGE_DIR, &name);
        if reader.entry_is_dir(index) {
            ensure_dir(&stage_path);
            continue;
        }
        ensure_parent_dirs(&stage_path);
        if !reader.extract_to_file(index, &stage_path) {
            return Err(String::from("Could not extract external app data from the backup."));
        }
    }

    Ok(())
}

fn apply_external_restore(
    entry: &ExternalRefEntry,
    applied: &mut Vec<AppliedRestore>,
) -> Result<(), String> {
    let staged_path = if entry.is_dir {
        join_fs_path(RESTORE_STAGE_DIR, &entry.archive_path)
    } else {
        join_fs_path(RESTORE_STAGE_DIR, &format!("{}/payload", entry.archive_path))
    };
    let backup_path = backup_slot_for_target(&entry.target_path);

    let mut target_stat = [0u32; 7];
    let had_existing = fs::stat(&entry.target_path, &mut target_stat) == 0;
    if had_existing {
        let _ = remove_tree(&backup_path);
        ensure_parent_dirs(&backup_path);
        if fs::rename(&entry.target_path, &backup_path) != 0 {
            return Err(String::from("Could not prepare current app data for restore."));
        }
    }

    ensure_parent_dirs(&entry.target_path);
    if fs::rename(&staged_path, &entry.target_path) != 0 {
        if had_existing {
            let _ = fs::rename(&backup_path, &entry.target_path);
        }
        return Err(String::from("Could not restore referenced app data."));
    }

    applied.push(AppliedRestore {
        target_path: entry.target_path.clone(),
        backup_path,
        had_existing,
    });
    Ok(())
}

fn rollback_external_restore(applied: &[AppliedRestore]) {
    let mut index = applied.len();
    while index > 0 {
        index -= 1;
        let item = &applied[index];
        let _ = remove_tree(&item.target_path);
        if item.had_existing {
            let _ = fs::rename(&item.backup_path, &item.target_path);
        }
    }
}

fn cleanup_restore_artifacts() {
    let _ = fs::unlink(RESTORE_TMP_PATH);
    let _ = remove_tree(RESTORE_STAGE_DIR);
    let _ = remove_tree(PRE_RESTORE_EXTERNAL_ROOT);
}

fn backup_slot_for_target(target_path: &str) -> String {
    format!("{}/{}", PRE_RESTORE_EXTERNAL_ROOT, strip_leading_slash(target_path))
}

fn find_archive_entry(reader: &libzip_client::TarReader, wanted: &str) -> Option<u32> {
    for index in 0..reader.entry_count() {
        if reader.entry_name(index) == wanted {
            return Some(index);
        }
    }
    None
}

fn ensure_parent_dirs(path: &str) {
    let bytes = path.as_bytes();
    let mut idx = 1usize;
    while idx < bytes.len() {
        if bytes[idx] == b'/' {
            let prefix = &path[..idx];
            if !prefix.is_empty() {
                let _ = fs::mkdir(prefix);
            }
        }
        idx += 1;
    }
}

fn ensure_dir(path: &str) {
    ensure_parent_dirs(path);
    let _ = fs::mkdir(path);
}

fn remove_tree(path: &str) -> Result<(), String> {
    let mut stat = [0u32; 7];
    if fs::stat(path, &mut stat) != 0 {
        return Ok(());
    }

    if stat[0] == 1 {
        let entries = fs::read_dir(path).map_err(|_| format!("Could not read directory {}.", path))?;
        for child in entries {
            if child.name == "." || child.name == ".." {
                continue;
            }
            let child_path = join_fs_path(path, &child.name);
            remove_tree(&child_path)?;
        }
    }

    if fs::unlink(path) != 0 {
        return Err(format!("Could not remove {}.", path));
    }
    Ok(())
}

fn join_fs_path(base: &str, child: &str) -> String {
    if base.is_empty() {
        return child.to_string();
    }
    if child.is_empty() {
        return base.to_string();
    }
    if base.ends_with('/') {
        format!("{}{}", base, child)
    } else {
        format!("{}/{}", base, child)
    }
}

fn strip_leading_slash(path: &str) -> &str {
    path.strip_prefix('/').unwrap_or(path)
}

fn escape_manifest_field(value: &str) -> String {
    if value.is_empty() {
        return String::from("%empty");
    }
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            '\t' => out.push_str("%09"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape_manifest_field(value: &str) -> String {
    if value == "%empty" {
        return String::new();
    }
    let bytes = value.as_bytes();
    let mut out = String::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_nibble(bytes[i + 1]), hex_nibble(bytes[i + 2])) {
                out.push((hi << 4 | lo) as char);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(10 + (b - b'a')),
        b'A'..=b'F' => Some(10 + (b - b'A')),
        _ => None,
    }
}

fn run_svc_command(command: &str, name: &str) -> bool {
    let args = format!("svc {} {}", command, name);
    let tid = process::spawn("/System/svc", &args);
    if tid != 0 && tid != u32::MAX {
        let _ = process::detach(tid);
        true
    } else {
        false
    }
}

fn default_backup_name() -> String {
    let mut buf = [0u8; 8];
    sys::time(&mut buf);
    let year = buf[0] as u16 | ((buf[1] as u16) << 8);
    let month = buf[2];
    let day = buf[3];
    let hour = buf[4];
    let min = buf[5];
    format!(
        "anyos-config-{:04}{:02}{:02}-{:02}{:02}.confdb.tar.gz",
        year, month, day, hour, min
    )
}
