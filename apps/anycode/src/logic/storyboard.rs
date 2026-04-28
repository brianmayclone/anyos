use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::logic::designer;
use crate::util::path;

const SCENE_W: u32 = 230;
const SCENE_H: u32 = 170;

#[derive(Clone, Debug)]
pub struct StoryboardScene {
    pub form_name: String,
    pub designer_path: String,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Debug)]
pub struct StoryboardSegue {
    pub id: String,
    pub from_form: String,
    pub from_control: String,
    pub to_form: String,
    pub condition: String,
    pub handler: String,
}

#[derive(Clone, Debug)]
pub struct StoryboardDocument {
    pub name: String,
    pub scenes: Vec<StoryboardScene>,
    pub segues: Vec<StoryboardSegue>,
}

impl StoryboardDocument {
    pub fn parse(file_path: &str, data: &str) -> Self {
        let mut doc = Self {
            name: storyboard_name(file_path),
            scenes: Vec::new(),
            segues: Vec::new(),
        };
        for line in data.split('\n') {
            let trimmed = line.trim();
            if trimmed.is_empty()
                || trimmed.starts_with('#')
                || trimmed == "anycode-storyboard-v1"
            {
                continue;
            }
            if trimmed.starts_with("scene ") {
                doc.scenes.push(StoryboardScene {
                    form_name: attr(trimmed, "form").unwrap_or_else(|| String::from("Form")),
                    designer_path: attr(trimmed, "designer").unwrap_or_default(),
                    x: attr_i32(trimmed, "x", 40),
                    y: attr_i32(trimmed, "y", 40),
                });
            } else if trimmed.starts_with("segue ") {
                doc.segues.push(StoryboardSegue {
                    id: attr(trimmed, "id").unwrap_or_default(),
                    from_form: attr(trimmed, "from_form").unwrap_or_default(),
                    from_control: attr(trimmed, "from_control").unwrap_or_default(),
                    to_form: attr(trimmed, "to_form").unwrap_or_default(),
                    condition: attr(trimmed, "condition").unwrap_or_default(),
                    handler: attr(trimmed, "handler").unwrap_or_default(),
                });
            }
        }
        doc
    }

    pub fn to_metadata(&self) -> String {
        let mut out = String::from("anycode-storyboard-v1\n");
        for scene in &self.scenes {
            out.push_str(&format!(
                "scene form=\"{}\" designer=\"{}\" x={} y={}\n",
                escape(&scene.form_name),
                escape(&scene.designer_path),
                scene.x,
                scene.y
            ));
        }
        for segue in &self.segues {
            out.push_str(&format!(
                "segue id=\"{}\" from_form=\"{}\" from_control=\"{}\" to_form=\"{}\" condition=\"{}\" handler=\"{}\"\n",
                escape(&segue.id),
                escape(&segue.from_form),
                escape(&segue.from_control),
                escape(&segue.to_form),
                escape(&segue.condition),
                escape(&segue.handler)
            ));
        }
        out
    }

    pub fn scene_at(&self, x: i32, y: i32) -> Option<usize> {
        self.scenes.iter().position(|scene| {
            x >= scene.x
                && y >= scene.y
                && x < scene.x + SCENE_W as i32
                && y < scene.y + SCENE_H as i32
        })
    }

    pub fn control_anchor_at(&self, x: i32, y: i32) -> Option<(String, String)> {
        for scene in &self.scenes {
            let Some(form) = designer::load_designer(&scene.designer_path) else {
                continue;
            };
            for control in &form.controls {
                let (ax, ay) = control_anchor(scene, control);
                let dx = x - ax;
                let dy = y - ay;
                if dx * dx + dy * dy <= 64 {
                    return Some((scene.form_name.clone(), control.name.clone()));
                }
            }
        }
        None
    }

    pub fn add_segue(
        &mut self,
        from_form: &str,
        from_control: &str,
        to_form: &str,
    ) -> Option<StoryboardSegue> {
        if from_form == to_form {
            return None;
        }
        if self.segues.iter().any(|segue| {
            segue.from_form == from_form
                && segue.from_control == from_control
                && segue.to_form == to_form
        }) {
            return None;
        }
        let id = format!(
            "{}_{}_to_{}",
            to_module_name(from_form),
            to_module_name(from_control),
            to_module_name(to_form)
        );
        let handler = format!("{}_navigate_to_{}", to_module_name(from_control), to_module_name(to_form));
        let segue = StoryboardSegue {
            id,
            from_form: String::from(from_form),
            from_control: String::from(from_control),
            to_form: String::from(to_form),
            condition: String::new(),
            handler,
        };
        self.segues.push(segue.clone());
        Some(segue)
    }
}

pub fn is_storyboard_file(file_path: &str) -> bool {
    file_path.ends_with(".Storyboard")
}

pub fn load_storyboard(file_path: &str) -> Option<StoryboardDocument> {
    let data = anyos_std::fs::read_to_string(file_path).ok()?;
    let mut doc = StoryboardDocument::parse(file_path, &data);
    if doc.scenes.is_empty() {
        if let Some(root) = project_root_for(file_path) {
            doc.scenes = discover_scenes(&root);
        }
    }
    Some(doc)
}

pub fn save_storyboard(file_path: &str, doc: &StoryboardDocument) -> Result<(), &'static str> {
    anyos_std::fs::write_bytes(file_path, doc.to_metadata().as_bytes())
        .map_err(|_| "Could not write storyboard")
}

pub fn apply_segue(
    storyboard_path: &str,
    doc: &mut StoryboardDocument,
    from_form: &str,
    from_control: &str,
    to_form: &str,
) -> Result<Option<StoryboardSegue>, &'static str> {
    let Some(segue) = doc.add_segue(from_form, from_control, to_form) else {
        return Ok(None);
    };
    save_storyboard(storyboard_path, doc)?;
    write_navigation_handler(doc, &segue)?;
    Ok(Some(segue))
}

pub fn scene_size() -> (u32, u32) {
    (SCENE_W, SCENE_H)
}

pub fn control_anchor(scene: &StoryboardScene, control: &designer::DesignerControl) -> (i32, i32) {
    let local_x = control.x / 3 + (control.width / 6) as i32;
    let local_y = control.y / 3 + (control.height / 6) as i32;
    (scene.x + 16 + local_x, scene.y + 36 + local_y)
}

fn write_navigation_handler(
    doc: &StoryboardDocument,
    segue: &StoryboardSegue,
) -> Result<(), &'static str> {
    let Some(scene) = doc
        .scenes
        .iter()
        .find(|scene| scene.form_name == segue.from_form)
    else {
        return Err("Source form not found");
    };
    let mut form = designer::load_designer(&scene.designer_path).ok_or("Could not load source form")?;
    form.update_control_property(&segue.from_control, "OnClick", &segue.handler)?;
    designer::save_designer(&scene.designer_path, &form)?;

    let events_path = designer::events_file_for_designer(&scene.designer_path);
    let signature = format!("pub fn {}()", segue.handler);
    let mut data = anyos_std::fs::read_to_string(&events_path).unwrap_or_default();
    if !data.contains(&signature) {
        if !data.ends_with('\n') {
            data.push('\n');
        }
        data.push_str(&format!(
            "\npub fn {}() {{\n    if storyboard_can_navigate(\"{}\") {{\n        storyboard_navigate(\"{}\");\n    }}\n}}\n",
            segue.handler,
            escape(&segue.id),
            escape(&segue.to_form)
        ));
        data.push_str(
            "\nfn storyboard_can_navigate(_segue_id: &str) -> bool {\n    true\n}\n\nfn storyboard_navigate(_form_name: &str) {\n    // TODO: connect this to the app navigation host.\n}\n",
        );
        anyos_std::fs::write_bytes(&events_path, data.as_bytes())
            .map_err(|_| "Could not update navigation event")?;
    }
    Ok(())
}

fn discover_scenes(project_root: &str) -> Vec<StoryboardScene> {
    let mut out = Vec::new();
    let ui_dir = format!("{}/src/ui", project_root);
    discover_designer_files(&ui_dir, &mut out, 0);
    for (idx, scene) in out.iter_mut().enumerate() {
        scene.x = 36 + ((idx as i32 % 3) * 280);
        scene.y = 42 + ((idx as i32 / 3) * 220);
    }
    out
}

fn discover_designer_files(dir: &str, out: &mut Vec<StoryboardScene>, depth: u32) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = anyos_std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let full = path::join(dir, &entry.name);
        if entry.is_dir() {
            discover_designer_files(&full, out, depth + 1);
        } else if designer::is_designer_file(&full) {
            if let Some(form) = designer::load_designer(&full) {
                out.push(StoryboardScene {
                    form_name: form.form_name,
                    designer_path: full,
                    x: 0,
                    y: 0,
                });
            }
        }
    }
}

fn project_root_for(file_path: &str) -> Option<String> {
    let mut dir = path::parent(file_path)?;
    for _ in 0..6 {
        if path::exists(&format!("{}/src/ui", dir)) {
            return Some(dir);
        }
        dir = path::parent(&dir)?;
    }
    None
}

fn storyboard_name(file_path: &str) -> String {
    let base = path::basename(file_path);
    base.strip_suffix(".Storyboard")
        .map(String::from)
        .unwrap_or_else(|| String::from(base))
}

fn attr(line: &str, key: &str) -> Option<String> {
    let pat = format!("{}=\"", key);
    let start = line.find(&pat)? + pat.len();
    let rest = &line[start..];
    let end = rest.find('"')?;
    Some(unescape(&rest[..end]))
}

fn attr_i32(line: &str, key: &str, default: i32) -> i32 {
    let pat = format!("{}=", key);
    let Some(start) = line.find(&pat).map(|idx| idx + pat.len()) else {
        return default;
    };
    let rest = &line[start..];
    let end = rest.find(' ').unwrap_or(rest.len());
    rest[..end].parse::<i32>().unwrap_or(default)
}

fn escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut escaped = false;
    for ch in value.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else {
            out.push(ch);
        }
    }
    out
}

fn to_module_name(name: &str) -> String {
    let mut out = String::new();
    for (idx, ch) in name.chars().enumerate() {
        if ch.is_ascii_alphanumeric() {
            if ch.is_ascii_uppercase() && idx > 0 && !out.ends_with('_') {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').into()
}
