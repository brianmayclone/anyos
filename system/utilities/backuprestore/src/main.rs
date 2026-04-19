#![no_std]
#![no_main]

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::{fs, i18n, process, sys};
use libdb_client::Database;
use libanyui_client as ui;

anyos_std::entry!(main);

const WIN_W: u32 = 760;
const WIN_H: u32 = 420;
const TOOLBAR_H: u32 = 40;
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

struct App {
    status: ui::Label,
    detail: ui::TextEditor,
    db_path: ui::TextField,
}

struct ExternalRefEntry {
    logical_path: String,
    target_path: String,
    archive_path: String,
    is_dir: bool,
}

struct AppliedRestore {
    target_path: String,
    backup_path: String,
    had_existing: bool,
}

anyos_std::global_app_state!(App);

fn main() {
    if !ui::init() {
        anyos_std::println!("backuprestore: failed to load libanyui.so");
        return;
    }
    i18n::init();

    let tc = ui::theme::colors();
    let win = ui::Window::new("Backup & Restore", -1, -1, WIN_W, WIN_H);

    let toolbar = ui::Toolbar::new();
    toolbar.set_dock(ui::DOCK_TOP);
    toolbar.set_size(WIN_W, TOOLBAR_H);
    toolbar.set_padding(8, 5, 8, 5);
    win.add(&toolbar);

    let btn_backup = toolbar.add_button("Backup...");
    btn_backup.set_size(110, 28);

    let btn_restore = toolbar.add_button("Restore...");
    btn_restore.set_size(110, 28);

    toolbar.add_separator();
    let title = toolbar.add_label("Configuration Backup");
    title.set_font_size(15);
    title.set_text_color(tc.text);
    title.set_size(260, 26);

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
    root.set_color(tc.editor_bg);
    root.set_padding(16, 14, 16, 14);
    win.add(&root);

    let intro = ui::Label::new(
        "Creates a compressed backup of confd plus every ExternalRef target stored in the registry.",
    );
    intro.set_position(16, 14);
    intro.set_size(720, 22);
    intro.set_text_color(tc.text);
    root.add(&intro);

    let note = ui::Label::new(
        "Restore replaces config.db and referenced external files/directories, then restarts the system.",
    );
    note.set_position(16, 40);
    note.set_size(720, 20);
    note.set_text_color(tc.text_secondary);
    note.set_font_size(11);
    root.add(&note);

    let db_label = ui::Label::new("Current Database");
    db_label.set_position(16, 80);
    db_label.set_size(180, 16);
    root.add(&db_label);

    let db_path = ui::TextField::new();
    db_path.set_position(16, 102);
    db_path.set_size(710, 28);
    db_path.set_read_only(true);
    db_path.set_text(CONF_DB_PATH);
    root.add(&db_path);

    let detail_label = ui::Label::new("Details");
    detail_label.set_position(16, 146);
    detail_label.set_size(80, 16);
    root.add(&detail_label);

    let detail = ui::TextEditor::new(710, 190);
    detail.set_position(16, 168);
    detail.set_editor_font(4, 12);
    detail.set_read_only(true);
    root.add(&detail);

    unsafe {
        APP = Some(App {
            status,
            detail,
            db_path,
        });
    }

    set_detail(
        "Backups are written as compressed .tar.gz archives.\n\
         Contents:\n\
         - config.db\n\
         - manifest.txt with ExternalRef mappings\n\
         - every referenced external file or directory tree\n\n\
         Restore keeps rollback copies of config.db and replaced external targets, then performs a reboot.",
    );
    refresh_status();

    btn_backup.on_click(|_| do_backup());
    btn_restore.on_click(|_| do_restore());

    win.on_close(|_| ui::quit());
    ui::run();
}

fn set_status(text: &str) {
    app().status.set_text(text);
}

fn set_detail(text: &str) {
    app().detail.set_text(text);
}

fn refresh_status() {
    let mut stat = [0u32; 7];
    if fs::stat(CONF_DB_PATH, &mut stat) == 0 {
        let size = stat[1];
        set_status(&format!("Database present: {}", format_size(size)));
        app().db_path.set_text(CONF_DB_PATH);
    } else {
        set_status("Database not found.");
        app().db_path.set_text("(missing)");
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
            "The confd database was not found at /System/sysdb/config.db.",
            Some("OK"),
        );
        refresh_status();
        return;
    }

    let default_name = default_backup_name();
    let Some(path) = ui::FileDialog::save_file(&default_name) else {
        return;
    };

    set_status("Collecting ExternalRef targets...");
    match create_backup_archive(&path) {
        Ok(summary) => {
            set_detail(&summary);
            set_status("Backup completed.");
            ui::MessageBox::show(ui::MessageBoxType::Info, &summary, Some("OK"));
        }
        Err(err) => {
            let message = format!("Backup failed.\n\n{}", err);
            set_detail(&message);
            set_status("Backup failed.");
            ui::MessageBox::show(ui::MessageBoxType::Alert, &message, Some("OK"));
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

    set_status("Stopping confd...");
    if !run_svc_command("stop", "confd") {
        ui::MessageBox::show(
            ui::MessageBoxType::Alert,
            "Failed to launch /System/svc for stopping confd.",
            Some("OK"),
        );
        return;
    }
    process::sleep(300);

    let result = restore_archive(&path);

    match result {
        Ok(summary) => {
            set_detail(&summary);
            set_status("Restore completed. Restarting system...");
            ui::MessageBox::show(ui::MessageBoxType::Info, &summary, Some("Restart"));
            process::sleep(200);
            process::reboot();
        }
        Err(err) => {
            let _ = run_svc_command("start", "confd");
            process::sleep(300);
            refresh_status();
            let message = format!("Restore failed.\n\n{}", err);
            set_detail(&message);
            set_status("Restore failed.");
            ui::MessageBox::show(ui::MessageBoxType::Alert, &message, Some("OK"));
        }
    }
}

fn create_backup_archive(path: &str) -> Result<String, String> {
    let refs = collect_external_refs()?;
    let db_bytes = fs::read_to_vec(CONF_DB_PATH)
        .map_err(|_| format!("Could not read the confd database at {}.", CONF_DB_PATH))?;
    let writer = libzip_client::TarWriter::new()
        .ok_or_else(|| String::from("Could not create a tar writer."))?;
    if !writer.add_file(DB_ARCHIVE_ENTRY, &db_bytes) {
        return Err(String::from("Could not add config.db to the backup archive."));
    }

    if !refs.is_empty() && !writer.add_dir("external/") {
        return Err(String::from("Could not create the external/ archive root."));
    }

    for entry in &refs {
        add_external_ref_to_archive(&writer, entry)?;
    }

    let manifest = build_manifest(&refs);
    if !writer.add_file(MANIFEST_ARCHIVE_ENTRY, manifest.as_bytes()) {
        return Err(String::from("Could not add manifest.txt to the backup archive."));
    }

    if !writer.write_to_file(path, true) {
        return Err(format!("Could not write the backup archive to {}.", path));
    }

    Ok(format!(
        "Backup created successfully.\n\nArchive: {}\nDatabase: {}\nExternalRef targets included: {}",
        path,
        CONF_DB_PATH,
        refs.len()
    ))
}

fn restore_archive(archive_path: &str) -> Result<String, String> {
    cleanup_restore_artifacts();

    let reader = libzip_client::TarReader::open(archive_path)
        .ok_or_else(|| format!("Could not open backup archive {}.", archive_path))?;
    let db_index = find_archive_entry(&reader, DB_ARCHIVE_ENTRY)
        .ok_or_else(|| String::from("Backup archive does not contain config.db."))?;
    let manifest_index = find_archive_entry(&reader, MANIFEST_ARCHIVE_ENTRY)
        .ok_or_else(|| String::from("Backup archive does not contain manifest.txt."))?;

    let manifest_bytes = reader
        .extract(manifest_index)
        .ok_or_else(|| String::from("Could not extract manifest.txt from the backup archive."))?;
    let manifest_text = core::str::from_utf8(&manifest_bytes)
        .map_err(|_| String::from("manifest.txt is not valid UTF-8."))?;
    let refs = parse_manifest(manifest_text)?;

    if !reader.extract_to_file(db_index, RESTORE_TMP_PATH) {
        return Err(format!(
            "Could not extract config.db from {} to {}.",
            archive_path, RESTORE_TMP_PATH
        ));
    }

    extract_external_stage(&reader)?;

    let mut config_replaced = false;
    let mut had_existing_config = false;
    let mut applied = Vec::new();

    let result = (|| -> Result<(), String> {
        let mut stat = [0u32; 7];
        had_existing_config = fs::stat(CONF_DB_PATH, &mut stat) == 0;
        if had_existing_config && fs::rename(CONF_DB_PATH, PRE_RESTORE_PATH) != 0 {
            return Err(format!(
                "Could not move the current database out of the way.\nDatabase: {}\nBackup copy: {}",
                CONF_DB_PATH, PRE_RESTORE_PATH
            ));
        }

        if fs::rename(RESTORE_TMP_PATH, CONF_DB_PATH) != 0 {
            if had_existing_config {
                let _ = fs::rename(PRE_RESTORE_PATH, CONF_DB_PATH);
            }
            return Err(format!(
                "Could not replace the active database with the restored file.\nDatabase: {}",
                CONF_DB_PATH
            ));
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

    Ok(format!(
        "Restore completed.\n\nBackup: {}\nDatabase: {}\nRollback DB copy: {}\nExternalRef targets restored: {}\nExternal rollback root: {}\n\nThe system must restart now so all services reload the restored configuration state.",
        archive_path,
        CONF_DB_PATH,
        PRE_RESTORE_PATH,
        refs.len(),
        PRE_RESTORE_EXTERNAL_ROOT
    ))
}

fn ensure_libraries() -> bool {
    if !libzip_client::init() {
        set_status("libzip.so unavailable.");
        ui::MessageBox::show(
            ui::MessageBoxType::Alert,
            "libzip.so could not be loaded. Backup and restore need /Libraries/libzip.so.",
            Some("OK"),
        );
        return false;
    }
    if !libdb_client::init() {
        set_status("libdb.so unavailable.");
        ui::MessageBox::show(
            ui::MessageBoxType::Alert,
            "libdb.so could not be loaded. Backup and restore need /Libraries/libdb.so.",
            Some("OK"),
        );
        return false;
    }
    true
}

fn collect_external_refs() -> Result<Vec<ExternalRefEntry>, String> {
    let db = Database::open(CONF_DB_PATH)
        .ok_or_else(|| format!("Could not open confd database {}.", CONF_DB_PATH))?;
    let result = db
        .query(&format!(
            "SELECT logical_path, value_text FROM registry WHERE value_type = {} ORDER BY logical_path",
            VALUE_TYPE_EXTERNAL_REF
        ))
        .map_err(|err| format!("Could not query ExternalRef entries from config.db: {}", err))?;

    let mut refs = Vec::new();
    for row in 0..result.row_count() {
        let logical_path = result.get_text(row, 0).unwrap_or_default();
        let target_path = result.get_text(row, 1).unwrap_or_default();
        if target_path.is_empty() || refs.iter().any(|item: &ExternalRefEntry| item.target_path == target_path) {
            continue;
        }

        let mut stat = [0u32; 7];
        if fs::stat(&target_path, &mut stat) != 0 {
            return Err(format!(
                "ExternalRef target missing.\nRegistry: {}\nTarget: {}",
                logical_path, target_path
            ));
        }

        let index = refs.len();
        refs.push(ExternalRefEntry {
            logical_path,
            target_path,
            archive_path: format!("{}/{:03}", EXTERNAL_ARCHIVE_ROOT, index),
            is_dir: stat[0] == 1,
        });
    }

    Ok(refs)
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
            return Err(format!("Could not add archive directory {}.", root_dir));
        }
        let bytes = fs::read_to_vec(&entry.target_path)
            .map_err(|_| format!("Could not read referenced file {}.", entry.target_path))?;
        let archive_file = format!("{}/payload", entry.archive_path);
        if !writer.add_file(&archive_file, &bytes) {
            return Err(format!("Could not add {} to the backup archive.", entry.target_path));
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
        return Err(format!("Could not add archive directory {}.", archive_entry));
    }

    let entries = fs::read_dir(source_dir)
        .map_err(|_| format!("Could not read directory {}.", source_dir))?;
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
                return Err(format!("Could not add {} to the backup archive.", source_path));
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
        _ => return Err(String::from("Backup manifest has an unknown format.")),
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
            .ok_or_else(|| String::from("Manifest entry is missing a logical path."))?;
        let kind = fields
            .next()
            .ok_or_else(|| String::from("Manifest entry is missing a kind."))?;
        let target_path = fields
            .next()
            .map(unescape_manifest_field)
            .ok_or_else(|| String::from("Manifest entry is missing a target path."))?;
        let archive_path = fields
            .next()
            .map(unescape_manifest_field)
            .ok_or_else(|| String::from("Manifest entry is missing an archive path."))?;

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
            return Err(format!("Could not extract {} to {}.", name, stage_path));
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
            return Err(format!(
                "Could not move the current ExternalRef target out of the way.\nTarget: {}\nBackup: {}",
                entry.target_path, backup_path
            ));
        }
    }

    ensure_parent_dirs(&entry.target_path);
    if fs::rename(&staged_path, &entry.target_path) != 0 {
        if had_existing {
            let _ = fs::rename(&backup_path, &entry.target_path);
        }
        return Err(format!(
            "Could not restore the ExternalRef target.\nTarget: {}\nStaged: {}\nRegistry: {}",
            entry.target_path, staged_path, entry.logical_path
        ));
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
    format!(
        "{}/{}",
        PRE_RESTORE_EXTERNAL_ROOT,
        strip_leading_slash(target_path)
    )
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

fn format_size(bytes: u32) -> String {
    if bytes >= 1024 * 1024 {
        format!("{} MiB", bytes / (1024 * 1024))
    } else if bytes >= 1024 {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{} B", bytes)
    }
}
