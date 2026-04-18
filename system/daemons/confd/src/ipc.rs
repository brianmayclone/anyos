use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use libdb_client::Database;

use crate::{schema, ConfState, ConfValue, NodeKind, RegistryEntry, Scope};

pub fn handle_requests(db: &Database, state: &mut ConfState, pipe_id: u32, buf: &mut [u8]) -> bool {
    let n = anyos_std::ipc::pipe_read(pipe_id, buf);
    if n == 0 || n == u32::MAX {
        return false;
    }

    let data = match core::str::from_utf8(&buf[..n as usize]) {
        Ok(text) => text,
        Err(_) => return true,
    };
    state.pending_request.push_str(data);

    while let Some(pos) = state.pending_request.find('\n') {
        let mut line = state.pending_request[..pos].to_string();
        state.pending_request.drain(..=pos);
        if line.ends_with('\r') {
            line.pop();
        }
        if !line.is_empty() {
            handle_single_request(db, state, &line);
        }
    }

    true
}

fn handle_single_request(db: &Database, state: &mut ConfState, line: &str) {
    let Some(tab_pos) = line.find('\t') else {
        return;
    };
    let Some(tid) = parse_u32(&line[..tab_pos]) else {
        return;
    };
    let cmd = line[tab_pos + 1..].trim();
    if cmd.is_empty() {
        return;
    }
    dispatch(db, state, tid, cmd);
}

fn dispatch(db: &Database, state: &mut ConfState, tid: u32, cmd: &str) {
    let (verb, rest) = split_first_word(cmd);
    match verb {
        "HELLO" | "hello" => cmd_hello(state, tid, rest),
        "PING" | "ping" => send_line(tid, "PONG"),
        "REGISTER" | "register" => cmd_register(db, state, tid, rest),
        "MKDIR" | "mkdir" => cmd_mkdir(db, state, tid, rest),
        "SET" | "set" => cmd_set(db, state, tid, rest),
        "GET" | "get" => cmd_get(state, tid, rest),
        "DEL" | "del" => cmd_del(db, state, tid, rest),
        "LIST" | "list" => cmd_list(state, tid, rest),
        "AUDIT" | "audit" => cmd_audit(db, state, tid, rest),
        "WATCH" | "watch" => cmd_watch(state, tid, rest),
        "UNWATCH" | "unwatch" => cmd_unwatch(state, tid, rest),
        _ => send_line(tid, "ERR unknown_command"),
    }
}

fn cmd_hello(state: &mut ConfState, tid: u32, rest: &str) {
    let uid = uid_for_tid(tid).unwrap_or(0);
    let mut name = rest.trim();
    if name.is_empty() {
        name = if uid == 0 { "service" } else { "app" };
    }
    if !is_valid_client_name(name) {
        send_line(tid, "ERR invalid_client_name");
        return;
    }
    state.set_client(tid, uid, name);
    let mut resp = String::from("OK hello ");
    push_u32(&mut resp, uid as u32);
    send_line(tid, &resp);
}

fn cmd_mkdir(db: &Database, state: &mut ConfState, tid: u32, rest: &str) {
    let Some((scope, logical_path, canonical_path, actor_uid, actor_name, owner_uid)) =
        parse_scope_and_path(state, tid, rest)
    else {
        return;
    };

    if !can_write(scope, actor_uid, owner_uid) {
        send_line(tid, "ERR forbidden");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "mkdir", scope, &logical_path, "forbidden", "", 0);
        return;
    }

    ensure_parent_dirs(db, state, scope, owner_uid, actor_uid, &logical_path, &actor_name);

    let now = now_ms();
    let (entry, changed) = state.upsert_dir(
        scope,
        owner_uid,
        &canonical_path,
        &logical_path,
        actor_uid,
        &actor_name,
        now,
    );
    if changed && schema::persist_entry(db, &entry).is_err() {
        send_line(tid, "ERR persist_failed");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "mkdir", scope, &logical_path, "persist_failed", "", entry.version);
        return;
    }

    let mut resp = String::from("OK mkdir ");
    resp.push_str(scope_name(scope));
    resp.push(' ');
    resp.push_str(&entry.logical_path);
    resp.push(' ');
    push_u64(&mut resp, entry.version);
    resp.push(' ');
    push_u64(&mut resp, entry.updated_at);
    send_line(tid, &resp);

    audit(state, db, actor_uid, owner_uid, &actor_name, tid, "mkdir", scope, &logical_path, "ok", "", entry.version);
    if changed {
        emit_change_events(state, &entry, "mkdir");
    }
}

fn cmd_register(db: &Database, state: &mut ConfState, tid: u32, rest: &str) {
    let mut parts = rest.splitn(4, ' ');
    let Some(scope_raw) = parts.next() else {
        send_line(tid, "ERR invalid_register");
        return;
    };
    let Some(namespace) = parts.next() else {
        send_line(tid, "ERR invalid_register");
        return;
    };
    let Some(version_raw) = parts.next() else {
        send_line(tid, "ERR invalid_register");
        return;
    };
    let manifest_text = parts.next().unwrap_or("");

    let Some((scope, requested_owner_uid)) = parse_scope_spec(scope_raw) else {
        send_line(tid, "ERR invalid_scope");
        return;
    };
    if !is_valid_logical_path(namespace) || namespace.is_empty() {
        send_line(tid, "ERR invalid_namespace");
        return;
    }
    let Some(schema_version) = parse_u32(version_raw) else {
        send_line(tid, "ERR invalid_schema_version");
        return;
    };

    let actor_uid = uid_for_tid(tid).unwrap_or(0);
    let actor_name = String::from(state.client_name(tid));
    let owner_uid = owner_uid_for_scope(scope, actor_uid, requested_owner_uid);
    if !can_write(scope, actor_uid, owner_uid) {
        send_line(tid, "ERR forbidden");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "register", scope, namespace, "forbidden", "", 0);
        return;
    }

    let namespace_root = canonical_path(scope, owner_uid, namespace);
    ensure_parent_dirs(db, state, scope, owner_uid, actor_uid, namespace, &actor_name);
    let now = now_ms();
    let (root_entry, root_changed) = state.upsert_dir(
        scope,
        owner_uid,
        &namespace_root,
        namespace,
        actor_uid,
        &actor_name,
        now,
    );
    if root_changed {
        let _ = schema::persist_entry(db, &root_entry);
    }

    let (_stored_schema_version, mut applied_version) =
        schema::load_schema_versions(db, scope, owner_uid, namespace);

    for op in manifest_text.split(';') {
        if op.is_empty() {
            continue;
        }
        let mut fields = op.split('|');
        match fields.next() {
            Some("D") => {
                let Some(rel_path) = fields.next() else { continue; };
                if !is_valid_logical_path(rel_path) {
                    continue;
                }
                let full_path = join_namespace(namespace, rel_path);
                ensure_parent_dirs(db, state, scope, owner_uid, actor_uid, &full_path, &actor_name);
                let canonical = canonical_path(scope, owner_uid, &full_path);
                let (entry, changed) = state.upsert_dir(
                    scope,
                    owner_uid,
                    &canonical,
                    &full_path,
                    actor_uid,
                    &actor_name,
                    now_ms(),
                );
                if changed {
                    let _ = schema::persist_entry(db, &entry);
                }
            }
            Some("K") => {
                let Some(rel_path) = fields.next() else { continue; };
                let Some(value_type) = fields.next() else { continue; };
                let Some(raw_value) = fields.next() else { continue; };
                if !is_valid_logical_path(rel_path) {
                    continue;
                }
                let Some(value) = decode_value(value_type, raw_value) else {
                    continue;
                };
                let full_path = join_namespace(namespace, rel_path);
                let canonical = canonical_path(scope, owner_uid, &full_path);
                if state.find_entry(&canonical).is_some() {
                    continue;
                }
                ensure_parent_dirs(db, state, scope, owner_uid, actor_uid, &full_path, &actor_name);
                let (entry, _) = state.upsert_value(
                    scope,
                    owner_uid,
                    &canonical,
                    &full_path,
                    value,
                    actor_uid,
                    &actor_name,
                    now_ms(),
                );
                let _ = schema::persist_entry(db, &entry);
            }
            Some("M") => {
                let Some(step_version_raw) = fields.next() else { continue; };
                let Some(rel_path) = fields.next() else { continue; };
                let Some(value_type) = fields.next() else { continue; };
                let Some(raw_value) = fields.next() else { continue; };
                let Some(step_version) = parse_u32(step_version_raw) else { continue; };
                if step_version <= applied_version || step_version > schema_version {
                    continue;
                }
                if !is_valid_logical_path(rel_path) {
                    continue;
                }
                let Some(value) = decode_value(value_type, raw_value) else {
                    continue;
                };
                let full_path = join_namespace(namespace, rel_path);
                ensure_parent_dirs(db, state, scope, owner_uid, actor_uid, &full_path, &actor_name);
                let canonical = canonical_path(scope, owner_uid, &full_path);
                let (entry, _) = state.upsert_value(
                    scope,
                    owner_uid,
                    &canonical,
                    &full_path,
                    value,
                    actor_uid,
                    &actor_name,
                    now_ms(),
                );
                let _ = schema::persist_entry(db, &entry);
                applied_version = step_version;
            }
            Some("X") => {
                let Some(step_version_raw) = fields.next() else { continue; };
                let Some(rel_path) = fields.next() else { continue; };
                let Some(step_version) = parse_u32(step_version_raw) else { continue; };
                if step_version <= applied_version || step_version > schema_version {
                    continue;
                }
                if !is_valid_logical_path(rel_path) {
                    continue;
                }
                let full_path = join_namespace(namespace, rel_path);
                delete_registry_subtree(db, state, scope, owner_uid, &full_path);
                applied_version = step_version;
            }
            Some("R") => {
                let Some(step_version_raw) = fields.next() else { continue; };
                let Some(from_rel) = fields.next() else { continue; };
                let Some(to_rel) = fields.next() else { continue; };
                let Some(step_version) = parse_u32(step_version_raw) else { continue; };
                if step_version <= applied_version || step_version > schema_version {
                    continue;
                }
                if !is_valid_logical_path(from_rel) || !is_valid_logical_path(to_rel) {
                    continue;
                }
                let from_full = join_namespace(namespace, from_rel);
                let to_full = join_namespace(namespace, to_rel);
                rename_registry_subtree(
                    db,
                    state,
                    scope,
                    owner_uid,
                    actor_uid,
                    &actor_name,
                    &from_full,
                    &to_full,
                );
                applied_version = step_version;
            }
            Some("C") => {
                let Some(step_version_raw) = fields.next() else { continue; };
                let Some(from_rel) = fields.next() else { continue; };
                let Some(to_rel) = fields.next() else { continue; };
                let Some(step_version) = parse_u32(step_version_raw) else { continue; };
                if step_version <= applied_version || step_version > schema_version {
                    continue;
                }
                if !is_valid_logical_path(from_rel) || !is_valid_logical_path(to_rel) {
                    continue;
                }
                let from_full = join_namespace(namespace, from_rel);
                let to_full = join_namespace(namespace, to_rel);
                copy_registry_subtree(
                    db,
                    state,
                    scope,
                    owner_uid,
                    actor_uid,
                    &actor_name,
                    &from_full,
                    &to_full,
                );
                applied_version = step_version;
            }
            _ => {}
        }
    }

    if applied_version < schema_version {
        applied_version = schema_version;
    }
    if schema::persist_schema(
        db,
        scope,
        owner_uid,
        namespace,
        schema_version,
        applied_version,
        manifest_text,
        now_ms(),
        &actor_name,
    )
    .is_err()
    {
        send_line(tid, "ERR persist_failed");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "register", scope, namespace, "persist_failed", "", schema_version as u64);
        return;
    }

    let mut resp = String::from("OK register ");
    resp.push_str(scope_name(scope));
    resp.push(' ');
    resp.push_str(namespace);
    resp.push(' ');
    push_u32(&mut resp, schema_version);
    resp.push(' ');
    push_u32(&mut resp, applied_version);
    send_line(tid, &resp);

    audit(
        state,
        db,
        actor_uid,
        owner_uid,
        &actor_name,
        tid,
        "register",
        scope,
        namespace,
        "ok",
        "",
        schema_version as u64,
    );
}

fn cmd_set(db: &Database, state: &mut ConfState, tid: u32, rest: &str) {
    let mut parts = rest.splitn(4, ' ');
    let Some(scope_name_raw) = parts.next() else {
        send_line(tid, "ERR invalid_set");
        return;
    };
    let Some(path) = parts.next() else {
        send_line(tid, "ERR invalid_set");
        return;
    };
    let Some(type_name) = parts.next() else {
        send_line(tid, "ERR invalid_set");
        return;
    };
    let Some(raw_value) = parts.next() else {
        send_line(tid, "ERR invalid_set");
        return;
    };

    let meta = format!("{} {}", scope_name_raw, path);
    let Some((scope, logical_path, canonical_path, actor_uid, actor_name, owner_uid)) =
        parse_scope_and_path(state, tid, &meta)
    else {
        return;
    };
    if !can_write(scope, actor_uid, owner_uid) {
        send_line(tid, "ERR forbidden");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "set", scope, &logical_path, "forbidden", "", 0);
        return;
    }

    let Some(value) = decode_value(type_name, raw_value) else {
        send_line(tid, "ERR invalid_value");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "set", scope, &logical_path, "invalid_value", type_name, 0);
        return;
    };

    ensure_parent_dirs(db, state, scope, owner_uid, actor_uid, &logical_path, &actor_name);

    let now = now_ms();
    let (entry, changed) = state.upsert_value(
        scope,
        owner_uid,
        &canonical_path,
        &logical_path,
        value,
        actor_uid,
        &actor_name,
        now,
    );
    if changed && schema::persist_entry(db, &entry).is_err() {
        send_line(tid, "ERR persist_failed");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "set", scope, &logical_path, "persist_failed", "", entry.version);
        return;
    }

    let mut resp = String::from("OK set ");
    resp.push_str(scope_name(scope));
    resp.push(' ');
    resp.push_str(&entry.logical_path);
    resp.push(' ');
    push_u64(&mut resp, entry.version);
    resp.push(' ');
    push_u64(&mut resp, entry.updated_at);
    send_line(tid, &resp);

    audit(state, db, actor_uid, owner_uid, &actor_name, tid, "set", scope, &logical_path, "ok", "", entry.version);
    if changed {
        emit_change_events(state, &entry, "set");
    }
}

fn cmd_get(state: &ConfState, tid: u32, rest: &str) {
    let Some((scope, _logical_path, canonical_path, _actor_uid, _actor_name, _owner_uid)) =
        parse_scope_and_path(state, tid, rest)
    else {
        return;
    };
    let Some(entry) = state.find_entry(&canonical_path) else {
        send_line(tid, "ERR not_found");
        return;
    };
    send_line(tid, &format_entry_line("ITEM", scope, entry));
}

fn cmd_del(db: &Database, state: &mut ConfState, tid: u32, rest: &str) {
    let Some((scope, logical_path, canonical_path, actor_uid, actor_name, owner_uid)) =
        parse_scope_and_path(state, tid, rest)
    else {
        return;
    };
    if !can_write(scope, actor_uid, owner_uid) {
        send_line(tid, "ERR forbidden");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "del", scope, &logical_path, "forbidden", "", 0);
        return;
    }

    let Some(old_entry) = state.remove_entry(&canonical_path) else {
        send_line(tid, "ERR not_found");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "del", scope, &logical_path, "not_found", "", 0);
        return;
    };

    if schema::delete_entry(db, &canonical_path).is_err() {
        state.entries.push(old_entry);
        send_line(tid, "ERR persist_failed");
        audit(state, db, actor_uid, owner_uid, &actor_name, tid, "del", scope, &logical_path, "persist_failed", "", 0);
        return;
    }

    let version = old_entry.version.saturating_add(1);
    let updated_at = now_ms();
    let mut resp = String::from("OK del ");
    resp.push_str(scope_name(scope));
    resp.push(' ');
    resp.push_str(&logical_path);
    resp.push(' ');
    push_u64(&mut resp, version);
    resp.push(' ');
    push_u64(&mut resp, updated_at);
    send_line(tid, &resp);

    audit(state, db, actor_uid, owner_uid, &actor_name, tid, "del", scope, &logical_path, "ok", "", version);
    emit_delete_events(state, scope, &logical_path, &canonical_path, version, updated_at);
}

fn cmd_list(state: &ConfState, tid: u32, rest: &str) {
    let Some((scope, _logical_path, canonical_path, _actor_uid, _actor_name, _owner_uid)) =
        parse_scope_and_path(state, tid, rest)
    else {
        return;
    };
    let items = state.list_prefix(&canonical_path);
    for entry in &items {
        send_line(tid, &format_entry_line("ITEM", scope, entry));
    }
    send_line(tid, "END");
}

fn cmd_audit(db: &Database, state: &ConfState, tid: u32, rest: &str) {
    let mut parts = rest.split_whitespace();
    let Some(scope_token) = parts.next() else {
        send_line(tid, "ERR invalid_audit");
        return;
    };
    let path = parts.next().unwrap_or("");
    let limit = parts.next().and_then(parse_u32).unwrap_or(100).max(1).min(500);

    if !is_valid_logical_path(path) {
        send_line(tid, "ERR invalid_path");
        return;
    }

    let Some((scope, requested_owner_uid)) = parse_scope_spec(scope_token) else {
        send_line(tid, "ERR invalid_scope");
        return;
    };
    let actor_uid = uid_for_tid(tid).unwrap_or(0);
    let owner_uid = owner_uid_for_scope(scope, actor_uid, requested_owner_uid);
    if !can_read(scope, actor_uid, owner_uid) {
        send_line(tid, "ERR forbidden");
        return;
    }

    let entries = schema::query_audit(db, scope, owner_uid, path, limit);
    for entry in &entries {
        send_line(tid, &format_audit_line(entry));
    }
    send_line(tid, "END");
}

fn cmd_watch(state: &mut ConfState, tid: u32, rest: &str) {
    let Some((scope, _logical_path, canonical_path, _actor_uid, _actor_name, _owner_uid)) =
        parse_scope_and_path(state, tid, rest)
    else {
        return;
    };
    let watch_id = state.add_watch(tid, scope, &canonical_path);
    let mut resp = String::from("OK watch ");
    push_u32(&mut resp, watch_id);
    send_line(tid, &resp);
}

fn cmd_unwatch(state: &mut ConfState, tid: u32, raw_id: &str) {
    let Some(watch_id) = parse_u32(raw_id) else {
        send_line(tid, "ERR invalid_watch_id");
        return;
    };
    if !state.remove_watch(tid, watch_id) {
        send_line(tid, "ERR not_found");
        return;
    }
    let mut resp = String::from("OK unwatch ");
    push_u32(&mut resp, watch_id);
    send_line(tid, &resp);
}

fn parse_scope_and_path(
    state: &ConfState,
    tid: u32,
    rest: &str,
) -> Option<(Scope, String, String, u16, String, u16)> {
    let (scope_token, path) = split_first_word(rest);
    let (scope, requested_owner_uid) = match parse_scope_spec(scope_token) {
        Some(v) => v,
        None => {
            send_line(tid, "ERR invalid_scope");
            return None;
        }
    };
    if !is_valid_logical_path(path) {
        send_line(tid, "ERR invalid_path");
        return None;
    }
    let actor_uid = uid_for_tid(tid).unwrap_or(0);
    let actor_name = String::from(state.client_name(tid));
    let owner_uid = owner_uid_for_scope(scope, actor_uid, requested_owner_uid);
    if !can_read(scope, actor_uid, owner_uid) {
        send_line(tid, "ERR forbidden");
        return None;
    }
    let logical_path = String::from(path);
    let canonical_path = canonical_path(scope, owner_uid, &logical_path);
    Some((scope, logical_path, canonical_path, actor_uid, actor_name, owner_uid))
}

fn ensure_parent_dirs(
    db: &Database,
    state: &mut ConfState,
    scope: Scope,
    owner_uid: u16,
    actor_uid: u16,
    logical_path: &str,
    actor_name: &str,
) {
    let mut current = String::new();
    for segment in logical_path.split('/') {
        if segment.is_empty() {
            continue;
        }
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(segment);
        let canonical = canonical_path(scope, actor_uid, &current);
        if state.find_entry(&canonical).is_some() {
            continue;
        }
        let now = now_ms();
        let (entry, _) = state.upsert_dir(
            scope,
            owner_uid,
            &canonical,
            &current,
            actor_uid,
            actor_name,
            now,
        );
        let _ = schema::persist_entry(db, &entry);
    }
}

fn delete_registry_subtree(
    db: &Database,
    state: &mut ConfState,
    scope: Scope,
    owner_uid: u16,
    logical_path: &str,
) {
    let canonical = canonical_path(scope, owner_uid, logical_path);
    let entries = collect_subtree_entries(state, &canonical);
    for entry in entries {
        state.remove_entry(&entry.canonical_path);
        let _ = schema::delete_entry(db, &entry.canonical_path);
    }
}

fn rename_registry_subtree(
    db: &Database,
    state: &mut ConfState,
    scope: Scope,
    owner_uid: u16,
    actor_uid: u16,
    actor_name: &str,
    from_logical_path: &str,
    to_logical_path: &str,
) {
    if from_logical_path == to_logical_path {
        return;
    }

    let from_canonical = canonical_path(scope, owner_uid, from_logical_path);
    let entries = collect_subtree_entries(state, &from_canonical);
    if entries.is_empty() {
        return;
    }

    ensure_parent_dirs(db, state, scope, owner_uid, actor_uid, to_logical_path, actor_name);
    delete_registry_subtree(db, state, scope, owner_uid, to_logical_path);

    for entry in &entries {
        state.remove_entry(&entry.canonical_path);
        let _ = schema::delete_entry(db, &entry.canonical_path);
    }

    let now = now_ms();
    for entry in entries {
        let moved = remap_entry(entry, scope, owner_uid, actor_uid, actor_name, now, from_logical_path, to_logical_path);
        state.entries.push(moved.clone());
        let _ = schema::persist_entry(db, &moved);
    }
}

fn copy_registry_subtree(
    db: &Database,
    state: &mut ConfState,
    scope: Scope,
    owner_uid: u16,
    actor_uid: u16,
    actor_name: &str,
    from_logical_path: &str,
    to_logical_path: &str,
) {
    if from_logical_path == to_logical_path {
        return;
    }

    let from_canonical = canonical_path(scope, owner_uid, from_logical_path);
    let entries = collect_subtree_entries(state, &from_canonical);
    if entries.is_empty() {
        return;
    }

    ensure_parent_dirs(db, state, scope, owner_uid, actor_uid, to_logical_path, actor_name);
    delete_registry_subtree(db, state, scope, owner_uid, to_logical_path);

    let now = now_ms();
    for entry in entries {
        let copied = remap_entry(entry, scope, owner_uid, actor_uid, actor_name, now, from_logical_path, to_logical_path);
        state.entries.push(copied.clone());
        let _ = schema::persist_entry(db, &copied);
    }
}

fn collect_subtree_entries(state: &ConfState, canonical_prefix: &str) -> Vec<RegistryEntry> {
    let mut entries = state.list_prefix(canonical_prefix);
    entries.retain(|entry| path_matches_prefix(&entry.canonical_path, canonical_prefix));
    entries.sort_by(|a, b| b.canonical_path.len().cmp(&a.canonical_path.len()));
    entries
}

fn remap_entry(
    mut entry: RegistryEntry,
    scope: Scope,
    owner_uid: u16,
    actor_uid: u16,
    actor_name: &str,
    now: u64,
    from_logical_path: &str,
    to_logical_path: &str,
) -> RegistryEntry {
    let suffix = entry
        .logical_path
        .strip_prefix(from_logical_path)
        .unwrap_or("");

    let mut new_logical_path = String::from(to_logical_path);
    new_logical_path.push_str(suffix);

    entry.scope = scope;
    entry.owner_uid = owner_uid;
    entry.logical_path = new_logical_path.clone();
    entry.canonical_path = canonical_path(scope, owner_uid, &new_logical_path);
    entry.writer_uid = actor_uid;
    entry.writer_name.clear();
    entry.writer_name.push_str(actor_name);
    entry.version = entry.version.saturating_add(1);
    entry.updated_at = now;
    entry
}

fn emit_change_events(state: &ConfState, entry: &RegistryEntry, action: &str) {
    let watchers = state.matching_watch_ids(entry.scope, &entry.canonical_path);
    if watchers.is_empty() {
        return;
    }

    let (type_name, value_str) = encode_value(entry.value.as_ref());
    for (tid, watch_id, scope) in watchers {
        let mut msg = String::from("EVENT ");
        push_u32(&mut msg, watch_id);
        msg.push(' ');
        msg.push_str(action);
        msg.push(' ');
        msg.push_str(scope_name(scope));
        msg.push(' ');
        msg.push_str(&entry.logical_path);
        msg.push(' ');
        msg.push_str(kind_name(entry.kind));
        msg.push(' ');
        msg.push_str(type_name);
        msg.push(' ');
        msg.push_str(&value_str);
        msg.push(' ');
        push_u64(&mut msg, entry.version);
        msg.push(' ');
        push_u64(&mut msg, entry.updated_at);
        send_line(tid, &msg);
    }
}

fn emit_delete_events(
    state: &ConfState,
    scope: Scope,
    logical_path: &str,
    canonical_path: &str,
    version: u64,
    updated_at: u64,
) {
    let watchers = state.matching_watch_ids(scope, canonical_path);
    if watchers.is_empty() {
        return;
    }

    for (tid, watch_id, event_scope) in watchers {
        let mut msg = String::from("EVENT ");
        push_u32(&mut msg, watch_id);
        msg.push_str(" delete ");
        msg.push_str(scope_name(event_scope));
        msg.push(' ');
        msg.push_str(logical_path);
        msg.push_str(" value none - ");
        push_u64(&mut msg, version);
        msg.push(' ');
        push_u64(&mut msg, updated_at);
        send_line(tid, &msg);
    }
}

fn audit(
    state: &mut ConfState,
    db: &Database,
    actor_uid: u16,
    owner_uid: u16,
    actor_name: &str,
    tid: u32,
    action: &str,
    scope: Scope,
    logical_path: &str,
    status: &str,
    detail: &str,
    version: u64,
) {
    let seq = state.next_audit_seq;
    state.next_audit_seq = state.next_audit_seq.saturating_add(1);
    schema::append_audit(
        db,
        seq,
        actor_uid,
        owner_uid,
        actor_name,
        tid,
        action,
        scope,
        logical_path,
        status,
        detail,
        version,
        now_ms(),
    );
}

fn parse_scope_spec(raw: &str) -> Option<(Scope, Option<u16>)> {
    match raw {
        "system" | "SYSTEM" => Some((Scope::System, None)),
        "user" | "USER" => Some((Scope::User, None)),
        _ => {
            if let Some(uid_raw) = raw.strip_prefix("user@").or_else(|| raw.strip_prefix("USER@")) {
                return parse_u32(uid_raw).map(|uid| (Scope::User, Some(uid as u16)));
            }
            None
        }
    }
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::System => "system",
        Scope::User => "user",
    }
}

fn kind_name(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Directory => "dir",
        NodeKind::Value => "value",
    }
}

fn can_read(scope: Scope, actor_uid: u16, owner_uid: u16) -> bool {
    match scope {
        Scope::System => true,
        Scope::User => actor_uid == 0 || actor_uid == owner_uid,
    }
}

fn can_write(scope: Scope, actor_uid: u16, owner_uid: u16) -> bool {
    match scope {
        Scope::System => actor_uid == 0,
        Scope::User => actor_uid == 0 || actor_uid == owner_uid,
    }
}

fn owner_uid_for_scope(scope: Scope, actor_uid: u16, requested_owner_uid: Option<u16>) -> u16 {
    match scope {
        Scope::System => 0,
        Scope::User => requested_owner_uid.unwrap_or(actor_uid),
    }
}

fn canonical_path(scope: Scope, uid: u16, logical_path: &str) -> String {
    let mut out = String::new();
    match scope {
        Scope::System => out.push_str("system"),
        Scope::User => {
            out.push_str("user/");
            push_u32(&mut out, uid as u32);
        }
    }
    if !logical_path.is_empty() {
        out.push('/');
        out.push_str(logical_path);
    }
    out
}

fn join_namespace(namespace: &str, rel_path: &str) -> String {
    if rel_path.is_empty() {
        return String::from(namespace);
    }
    let mut out = String::from(namespace);
    out.push('/');
    out.push_str(rel_path);
    out
}

fn path_matches_prefix(path: &str, prefix: &str) -> bool {
    if path == prefix {
        return true;
    }

    if let Some(rest) = path.strip_prefix(prefix) {
        return rest.starts_with('/');
    }

    false
}

fn format_entry_line(prefix: &str, scope: Scope, entry: &RegistryEntry) -> String {
    let (type_name, value_str) = encode_value(entry.value.as_ref());
    let mut line = String::from(prefix);
    line.push(' ');
    line.push_str(scope_name(scope));
    line.push(' ');
    line.push_str(&entry.logical_path);
    line.push(' ');
    line.push_str(kind_name(entry.kind));
    line.push(' ');
    line.push_str(type_name);
    line.push(' ');
    line.push_str(&value_str);
    line.push(' ');
    push_u64(&mut line, entry.version);
    line.push(' ');
    push_u64(&mut line, entry.updated_at);
    line
}

fn format_audit_line(entry: &schema::AuditEntry) -> String {
    let mut line = String::from("AUDIT ");
    push_u64(&mut line, entry.seq);
    line.push(' ');
    push_u32(&mut line, entry.actor_uid as u32);
    line.push(' ');
    push_u32(&mut line, entry.owner_uid as u32);
    line.push(' ');
    line.push_str(&escape_value(&entry.actor_name));
    line.push(' ');
    push_u32(&mut line, entry.tid);
    line.push(' ');
    line.push_str(&escape_value(&entry.action));
    line.push(' ');
    line.push_str(scope_name(entry.scope));
    line.push(' ');
    line.push_str(&entry.logical_path);
    line.push(' ');
    line.push_str(&escape_value(&entry.status));
    line.push(' ');
    line.push_str(&escape_value(&entry.detail));
    line.push(' ');
    push_u64(&mut line, entry.version);
    line.push(' ');
    push_u64(&mut line, entry.at_ms);
    line
}

fn encode_value(value: Option<&ConfValue>) -> (&'static str, String) {
    match value {
        Some(ConfValue::String(s)) => ("string", escape_value(s)),
        Some(ConfValue::Int(v)) => ("int", format!("{}", *v)),
        Some(ConfValue::Bool(v)) => ("bool", if *v { String::from("1") } else { String::from("0") }),
        None => ("none", String::from("-")),
    }
}

fn decode_value(type_name: &str, raw_value: &str) -> Option<ConfValue> {
    match type_name {
        "string" | "STRING" => Some(ConfValue::String(unescape_value(raw_value))),
        "int" | "INT" => parse_i64(raw_value).map(ConfValue::Int),
        "bool" | "BOOL" => match raw_value {
            "1" | "true" | "TRUE" => Some(ConfValue::Bool(true)),
            "0" | "false" | "FALSE" => Some(ConfValue::Bool(false)),
            _ => None,
        },
        _ => None,
    }
}

fn escape_value(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        match ch {
            ' ' => out.push_str("%20"),
            '%' => out.push_str("%25"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            _ => out.push(ch),
        }
    }
    out
}

fn unescape_value(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = String::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_nibble(bytes[i + 1]);
            let lo = hex_nibble(bytes[i + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
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

fn uid_for_tid(tid: u32) -> Option<u16> {
    const ENTRY_SIZE: usize = 80;
    const MAX_THREADS: usize = 256;

    let mut buf = [0u8; ENTRY_SIZE * MAX_THREADS];
    let count = anyos_std::sys::sysinfo(1, &mut buf);
    if count == u32::MAX {
        return None;
    }

    for i in 0..count as usize {
        let off = i * ENTRY_SIZE;
        if off + ENTRY_SIZE > buf.len() {
            break;
        }
        let entry_tid =
            u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        if entry_tid == tid {
            return Some(u16::from_le_bytes([buf[off + 56], buf[off + 57]]));
        }
    }
    None
}

fn now_ms() -> u64 {
    anyos_std::sys::uptime_ms() as u64
}

fn is_valid_client_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'/' | b'_' | b'-'))
}

fn is_valid_logical_path(path: &str) -> bool {
    if path.is_empty() {
        return true;
    }
    if path.starts_with('/') || path.ends_with('/') || path.contains("//") {
        return false;
    }
    path.split('/').all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
    })
}

fn send_line(tid: u32, line: &str) {
    let reply_pipe_name = format!("confd-{}", tid);
    let reply_pipe = anyos_std::ipc::pipe_open(&reply_pipe_name);
    if reply_pipe == 0 {
        return;
    }
    let mut msg = String::from(line);
    msg.push('\n');
    anyos_std::ipc::pipe_write(reply_pipe, msg.as_bytes());
}

fn split_first_word(input: &str) -> (&str, &str) {
    let trimmed = input.trim();
    if let Some(pos) = trimmed.find(' ') {
        (&trimmed[..pos], trimmed[pos + 1..].trim())
    } else {
        (trimmed, "")
    }
}

fn parse_u32(raw: &str) -> Option<u32> {
    if raw.is_empty() {
        return None;
    }
    let mut value = 0u32;
    for b in raw.bytes() {
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as u32)?;
    }
    Some(value)
}

fn parse_i64(raw: &str) -> Option<i64> {
    if raw.is_empty() {
        return None;
    }
    let bytes = raw.as_bytes();
    let mut idx = 0usize;
    let mut negative = false;
    if bytes[0] == b'-' {
        negative = true;
        idx = 1;
    }
    if idx >= bytes.len() {
        return None;
    }
    let mut value = 0i64;
    while idx < bytes.len() {
        let b = bytes[idx];
        if !b.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?.checked_add((b - b'0') as i64)?;
        idx += 1;
    }
    Some(if negative { -value } else { value })
}

fn push_u32(out: &mut String, value: u32) {
    out.push_str(format!("{}", value).as_str());
}

fn push_u64(out: &mut String, value: u64) {
    out.push_str(format!("{}", value).as_str());
}
