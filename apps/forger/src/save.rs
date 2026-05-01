use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use anyos_std::env;
use anyos_std::json::Value;

use crate::block;
use crate::inventory::{Inventory, HOTBAR_SLOTS};
use crate::world::World;

#[derive(Clone)]
pub struct WorldSummary {
    pub id: String,
    pub name: String,
    pub seed: u32,
}

pub struct PlayerSnapshot {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub flying: bool,
}

pub struct WorldSnapshot {
    pub summary: WorldSummary,
    pub player: PlayerSnapshot,
    pub inventory_counts: [u16; block::BLOCK_COUNT],
    pub inventory_hotbar: [u8; HOTBAR_SLOTS],
    pub inventory_selected_slot: usize,
    pub modifications: BTreeMap<(i32, i32, i32), u8>,
}

pub fn data_root() -> String {
    let mut home_buf = [0u8; 256];
    let len = env::get("HOME", &mut home_buf);
    if len != u32::MAX && (len as usize) < home_buf.len() {
        if let Ok(home) = core::str::from_utf8(&home_buf[..len as usize]) {
            if !home.is_empty() {
                return format!("{}/.forger", home);
            }
        }
    }
    String::from("/Users/.forger")
}

pub fn mkdir_p(path: &str) {
    if path.is_empty() {
        return;
    }
    let mut built = String::new();
    if path.starts_with('/') {
        built.push('/');
    }
    for part in path.split('/') {
        if part.is_empty() {
            continue;
        }
        if !built.is_empty() && !built.ends_with('/') {
            built.push('/');
        }
        built.push_str(part);
        let _ = anyos_std::fs::mkdir(&built);
    }
}

pub fn load_world_summaries() -> Vec<WorldSummary> {
    let path = index_path();
    let Ok(text) = anyos_std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let Ok(root) = Value::parse(&text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    if let Some(items) = root["worlds"].as_array() {
        for item in items {
            let id = item["id"].as_str().unwrap_or("");
            let name = item["name"].as_str().unwrap_or("");
            let seed = item["seed"].as_u64().unwrap_or(42) as u32;
            if !id.is_empty() && !name.is_empty() {
                out.push(WorldSummary {
                    id: String::from(id),
                    name: String::from(name),
                    seed,
                });
            }
        }
    }
    out
}

pub fn create_world(name: &str, seed: u32) -> Option<WorldSummary> {
    let trimmed = name.trim();
    let display_name = if trimmed.is_empty() {
        "Neue Welt"
    } else {
        trimmed
    };
    mkdir_p(&worlds_root());

    let existing = load_world_summaries();
    let id = unique_world_id(display_name, &existing);
    let summary = WorldSummary {
        id,
        name: String::from(display_name),
        seed,
    };

    let snapshot = WorldSnapshot {
        summary: summary.clone(),
        player: PlayerSnapshot {
            x: 0.0,
            y: 80.0,
            z: 0.0,
            yaw: 0.0,
            pitch: 0.0,
            flying: false,
        },
        inventory_counts: [0; block::BLOCK_COUNT],
        inventory_hotbar: [block::AIR; HOTBAR_SLOTS],
        inventory_selected_slot: 0,
        modifications: BTreeMap::new(),
    };

    if !save_world_snapshot(&snapshot) {
        return None;
    }

    let mut updated = existing;
    updated.insert(0, summary.clone());
    save_world_index(&updated);
    Some(summary)
}

pub fn load_world(id: &str) -> Option<WorldSnapshot> {
    let Ok(text) = anyos_std::fs::read_to_string(&world_file_path(id)) else {
        return None;
    };
    let Ok(root) = Value::parse(&text) else {
        return None;
    };

    let name = root["name"].as_str().unwrap_or(id);
    let seed = root["seed"].as_u64().unwrap_or(42) as u32;
    let player = PlayerSnapshot {
        x: root["player"]["x"].as_f64().unwrap_or(0.0) as f32,
        y: root["player"]["y"].as_f64().unwrap_or(80.0) as f32,
        z: root["player"]["z"].as_f64().unwrap_or(0.0) as f32,
        yaw: root["player"]["yaw"].as_f64().unwrap_or(0.0) as f32,
        pitch: root["player"]["pitch"].as_f64().unwrap_or(0.0) as f32,
        flying: root["player"]["flying"].as_bool().unwrap_or(false),
    };

    let mut inventory_counts = [0u16; block::BLOCK_COUNT];
    if let Some(arr) = root["inventory"]["counts"].as_array() {
        for (i, value) in arr.iter().enumerate().take(block::BLOCK_COUNT) {
            inventory_counts[i] = value.as_u64().unwrap_or(0).min(u16::MAX as u64) as u16;
        }
    }

    let mut inventory_hotbar = [block::AIR; HOTBAR_SLOTS];
    if let Some(arr) = root["inventory"]["hotbar"].as_array() {
        for (i, value) in arr.iter().enumerate().take(HOTBAR_SLOTS) {
            inventory_hotbar[i] = value.as_u64().unwrap_or(block::AIR as u64) as u8;
        }
    }

    let mut modifications = BTreeMap::new();
    if let Some(arr) = root["modifications"].as_array() {
        for item in arr {
            let x = item["x"].as_i64().unwrap_or(0) as i32;
            let y = item["y"].as_i64().unwrap_or(0) as i32;
            let z = item["z"].as_i64().unwrap_or(0) as i32;
            let id = item["id"].as_u64().unwrap_or(0) as u8;
            modifications.insert((x, y, z), id);
        }
    }

    Some(WorldSnapshot {
        summary: WorldSummary {
            id: String::from(id),
            name: String::from(name),
            seed,
        },
        player,
        inventory_counts,
        inventory_hotbar,
        inventory_selected_slot: root["inventory"]["selected_slot"]
            .as_u64()
            .unwrap_or(0)
            .min((HOTBAR_SLOTS - 1) as u64) as usize,
        modifications,
    })
}

pub fn save_runtime_world(
    world_id: &str,
    world_name: &str,
    world: &World,
    player: &PlayerSnapshot,
    inventory: &Inventory,
) -> bool {
    let snapshot = WorldSnapshot {
        summary: WorldSummary {
            id: String::from(world_id),
            name: String::from(world_name),
            seed: world.seed,
        },
        player: PlayerSnapshot {
            x: player.x,
            y: player.y,
            z: player.z,
            yaw: player.yaw,
            pitch: player.pitch,
            flying: player.flying,
        },
        inventory_counts: inventory.counts_snapshot(),
        inventory_hotbar: inventory.hotbar_snapshot(),
        inventory_selected_slot: inventory.selected_slot(),
        modifications: world.modifications.clone(),
    };
    save_world_snapshot(&snapshot)
}

fn save_world_snapshot(snapshot: &WorldSnapshot) -> bool {
    let world_dir = world_dir_path(&snapshot.summary.id);
    mkdir_p(&world_dir);

    let mut root = Value::new_object();
    root.set("id", snapshot.summary.id.clone().into());
    root.set("name", snapshot.summary.name.clone().into());
    root.set("seed", snapshot.summary.seed.into());

    let mut player = Value::new_object();
    player.set("x", (snapshot.player.x as f64).into());
    player.set("y", (snapshot.player.y as f64).into());
    player.set("z", (snapshot.player.z as f64).into());
    player.set("yaw", (snapshot.player.yaw as f64).into());
    player.set("pitch", (snapshot.player.pitch as f64).into());
    player.set("flying", snapshot.player.flying.into());
    root.set("player", player);

    let mut inventory = Value::new_object();
    let mut counts = Value::new_array();
    for count in snapshot.inventory_counts {
        counts.push((count as u32).into());
    }
    let mut hotbar = Value::new_array();
    for block_id in snapshot.inventory_hotbar {
        hotbar.push((block_id as u32).into());
    }
    inventory.set("counts", counts);
    inventory.set("hotbar", hotbar);
    inventory.set(
        "selected_slot",
        (snapshot.inventory_selected_slot as u32).into(),
    );
    root.set("inventory", inventory);

    let mut modifications = Value::new_array();
    for (&(x, y, z), &block_id) in &snapshot.modifications {
        let mut item = Value::new_object();
        item.set("x", x.into());
        item.set("y", y.into());
        item.set("z", z.into());
        item.set("id", (block_id as u32).into());
        modifications.push(item);
    }
    root.set("modifications", modifications);

    anyos_std::fs::write_bytes(
        &world_file_path(&snapshot.summary.id),
        root.to_json_string_pretty().as_bytes(),
    )
    .is_ok()
}

fn save_world_index(items: &[WorldSummary]) {
    mkdir_p(&worlds_root());
    let mut root = Value::new_object();
    let mut worlds = Value::new_array();
    for summary in items {
        let mut item = Value::new_object();
        item.set("id", summary.id.clone().into());
        item.set("name", summary.name.clone().into());
        item.set("seed", summary.seed.into());
        worlds.push(item);
    }
    root.set("worlds", worlds);
    let _ = anyos_std::fs::write_bytes(&index_path(), root.to_json_string_pretty().as_bytes());
}

fn worlds_root() -> String {
    format!("{}/worlds", data_root())
}

fn index_path() -> String {
    format!("{}/index.json", worlds_root())
}

fn world_dir_path(id: &str) -> String {
    format!("{}/{}", worlds_root(), id)
}

fn world_file_path(id: &str) -> String {
    format!("{}/world.json", world_dir_path(id))
}

fn unique_world_id(name: &str, existing: &[WorldSummary]) -> String {
    let mut base = slugify(name);
    if base.is_empty() {
        base = String::from("welt");
    }
    let mut candidate = base.clone();
    let mut suffix = 2u32;
    while existing.iter().any(|item| item.id == candidate) {
        candidate = format!("{}-{}", base, suffix);
        suffix += 1;
    }
    candidate
}

fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else if ch == ' ' || ch == '-' || ch == '_' {
            '-'
        } else {
            continue;
        };
        if mapped == '-' {
            if last_dash || out.is_empty() {
                continue;
            }
            last_dash = true;
        } else {
            last_dash = false;
        }
        out.push(mapped);
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}
