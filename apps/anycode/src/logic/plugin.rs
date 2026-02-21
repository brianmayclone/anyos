use alloc::string::String;
use alloc::vec::Vec;
use alloc::format;
use anyos_std::json::Value;

/// A loaded plugin with optional syntax and build configuration.
pub struct Plugin {
    pub name: String,
    pub extensions: Vec<String>,
    pub syntax_path: Option<String>,
    pub build_cmd: Option<String>,
    pub build_args: Option<String>,
    pub run_cmd: Option<String>,
    pub run_args: Option<String>,
}

const PLUGIN_DIR: &str = "/Libraries/anycode/plugins";

/// Scan the plugin directory and load all plugins.
pub fn load_plugins() -> Vec<Plugin> {
    let mut plugins = Vec::new();

    let entries = match anyos_std::fs::read_dir(PLUGIN_DIR) {
        Ok(rd) => rd,
        Err(_) => return plugins,
    };

    for entry in entries {
        if !entry.is_dir() || entry.name == "." || entry.name == ".." {
            continue;
        }
        if let Some(p) = load_single_plugin(&entry.name) {
            plugins.push(p);
        }
    }

    plugins
}

fn load_single_plugin(dir_name: &str) -> Option<Plugin> {
    let plugin_dir = format!("{}/{}", PLUGIN_DIR, dir_name);
    let json_path = format!("{}/plugin.json", plugin_dir);

    let data = anyos_std::fs::read_to_string(&json_path).ok()?;
    let val = Value::parse(&data).ok()?;

    let name = String::from(val["name"].as_str()?);

    let mut extensions = Vec::new();
    if let Some(arr) = val["extensions"].as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                extensions.push(String::from(s));
            }
        }
    }

    let syntax_path = val["syntax"].as_str().map(|s| format!("{}/{}", plugin_dir, s));

    let mut build_cmd = None;
    let mut build_args = None;
    let mut run_cmd = None;
    let mut run_args = None;

    let build_json_path = format!("{}/build.json", plugin_dir);
    if let Ok(build_data) = anyos_std::fs::read_to_string(&build_json_path) {
        if let Ok(bval) = Value::parse(&build_data) {
            build_cmd = bval["build"]["command"].as_str().map(String::from);
            build_args = bval["build"]["args"].as_str().map(String::from);
            run_cmd = bval["run"]["command"].as_str().map(String::from);
            run_args = bval["run"]["args"].as_str().map(String::from);
        }
    }

    Some(Plugin {
        name,
        extensions,
        syntax_path,
        build_cmd,
        build_args,
        run_cmd,
        run_args,
    })
}

/// Look up a syntax file path from loaded plugins by file extension.
pub fn syntax_for_extension<'a>(plugins: &'a [Plugin], ext: &str) -> Option<&'a str> {
    for plugin in plugins {
        for pext in &plugin.extensions {
            if pext == ext {
                return plugin.syntax_path.as_deref();
            }
        }
    }
    None
}
