use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use anyos_std::json::Value;
use libconf_schema::{default_string, manifest, RegistryScope, ServiceSchema};

// ════════════════════════════════════════════════════════════════
//  Plugin System — extensible IDE architecture
// ════════════════════════════════════════════════════════════════

/// Plugin capability flags — what a plugin provides.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PluginCapability {
    Syntax,      // Provides syntax highlighting (.syn files)
    Language,    // Provides language support (keywords, snippets)
    Build,       // Provides build/run commands
    Theme,       // Provides color theme
    Formatter,   // Provides code formatting
    Linter,      // Provides linting/diagnostics
    Snippets,    // Provides code snippets
    ProjectType, // Recognizes a project type
    Command,     // Adds commands to the command palette
    Panel,       // Adds a native IDE panel
    CodeAction,  // Adds quick fixes/refactorings
    Test,        // Discovers or runs tests
    AiTool,      // Safe tool callable by Codex
}

/// Plugin state in the extension manager.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PluginState {
    Installed,
    Active,
    Disabled,
    Error,
}

impl PluginState {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Installed => "Installed",
            Self::Active => "Active",
            Self::Disabled => "Disabled",
            Self::Error => "Error",
        }
    }
}

// ════════════════════════════════════════════════════════════════
//  Plugin manifest (plugin.json)
// ════════════════════════════════════════════════════════════════

/// A snippet provided by a plugin.
#[derive(Clone, Debug)]
pub struct PluginSnippet {
    pub prefix: String,
    pub body: String,
    pub description: String,
}

/// Build configuration from a plugin.
#[derive(Clone, Debug)]
pub struct PluginBuildConfig {
    pub build_cmd: String,
    pub build_args: String,
    pub run_cmd: String,
    pub run_args: String,
}

#[derive(Clone, Debug)]
pub struct PluginAbi {
    pub sdk_version: u32,
    pub library_path: Option<String>,
    pub entry_init: String,
    pub entry_register: String,
    pub entry_shutdown: String,
}

#[derive(Clone, Debug)]
pub struct PluginPermissions {
    pub filesystem: bool,
    pub process: bool,
    pub network: bool,
    pub ai_context: bool,
    pub ui: bool,
}

/// A loaded plugin with its manifest and resources.
pub struct Plugin {
    pub name: String,
    pub display_name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub dir_path: String,
    pub bundle_path: String,

    // Capabilities
    pub capabilities: Vec<PluginCapability>,
    pub state: PluginState,
    pub abi: PluginAbi,
    pub permissions: PluginPermissions,

    // Language support
    pub extensions: Vec<String>,
    pub syntax_path: Option<String>,
    pub keywords: Vec<String>,
    pub snippets: Vec<PluginSnippet>,

    // Build support
    pub build_config: Option<PluginBuildConfig>,

    // Theme support
    pub theme_path: Option<String>,

    // Project detection
    pub project_files: Vec<String>, // files that indicate this project type (e.g. "Cargo.toml")
    pub project_type_name: Option<String>,
}

impl Plugin {
    /// Check if the plugin has a specific capability.
    pub fn has_capability(&self, cap: PluginCapability) -> bool {
        self.capabilities.contains(&cap)
    }
}

// ════════════════════════════════════════════════════════════════
//  Plugin manager
// ════════════════════════════════════════════════════════════════

pub struct PluginManager {
    pub plugins: Vec<Plugin>,
    pub plugin_dir: String,
    pub system_plugin_dir: String,
    pub user_plugin_dir: String,
}

impl PluginManager {
    pub fn new(plugin_dir: &str) -> Self {
        Self {
            plugins: Vec::new(),
            plugin_dir: String::from(plugin_dir),
            system_plugin_dir: system_plugin_dir(),
            user_plugin_dir: user_plugin_dir(),
        }
    }

    /// Scan the plugin directory and load all plugins.
    pub fn scan_and_load(&mut self) {
        self.plugins.clear();

        let dirs = plugin_dirs(
            &self.plugin_dir,
            &self.system_plugin_dir,
            &self.user_plugin_dir,
        );
        for dir in dirs {
            self.scan_dir(&dir);
        }
    }

    fn scan_dir(&mut self, dir: &str) {
        let entries = match anyos_std::fs::read_dir(dir) {
            Ok(rd) => rd,
            Err(_) => return,
        };
        for entry in entries {
            if !entry.is_dir() || entry.name == "." || entry.name == ".." {
                continue;
            }
            if let Some(plugin) = load_plugin(dir, &entry.name) {
                self.plugins.push(plugin);
            }
        }
    }

    /// Get all active plugins.
    pub fn active_plugins(&self) -> Vec<&Plugin> {
        self.plugins
            .iter()
            .filter(|p| p.state == PluginState::Active || p.state == PluginState::Installed)
            .collect()
    }

    /// Find a plugin that handles a specific file extension.
    pub fn plugin_for_extension(&self, ext: &str) -> Option<&Plugin> {
        self.active_plugins()
            .into_iter()
            .find(|p| p.extensions.iter().any(|e| e == ext))
    }

    /// Get syntax file path from plugins for a given extension.
    pub fn syntax_for_extension(&self, ext: &str) -> Option<&str> {
        self.plugin_for_extension(ext)?.syntax_path.as_deref()
    }

    /// Get build configuration from a plugin for a given extension.
    pub fn build_config_for_extension(&self, ext: &str) -> Option<&PluginBuildConfig> {
        self.plugin_for_extension(ext)?.build_config.as_ref()
    }

    /// Get all snippets for a file extension (merged from all active plugins).
    pub fn snippets_for_extension(&self, ext: &str) -> Vec<&PluginSnippet> {
        let mut result = Vec::new();
        for plugin in self.active_plugins() {
            if plugin.extensions.iter().any(|e| e == ext) {
                for snippet in &plugin.snippets {
                    result.push(snippet);
                }
            }
        }
        result
    }

    /// Get the number of installed plugins.
    pub fn count(&self) -> usize {
        self.plugins.len()
    }

    /// Enable a plugin by name.
    pub fn enable(&mut self, name: &str) {
        if let Some(p) = self.plugins.iter_mut().find(|p| p.name == name) {
            p.state = PluginState::Active;
        }
    }

    /// Disable a plugin by name.
    pub fn disable(&mut self, name: &str) {
        if let Some(p) = self.plugins.iter_mut().find(|p| p.name == name) {
            p.state = PluginState::Disabled;
        }
    }

    /// Build display labels for extension list.
    pub fn display_labels(&self) -> Vec<String> {
        self.plugins
            .iter()
            .map(|p| format!("{} v{} - {}", p.display_name, p.version, p.description))
            .collect()
    }
}

const PLUGIN_DEFAULTS: &[libconf_schema::DefaultEntry<'static>] = &[
    default_string("config/system_dir", "/System/Library/anycode/plugins"),
    default_string("config/user_dir", "/Users/Shared/Library/anycode/plugins"),
    default_string("config/enabled_csv", ""),
    default_string("config/disabled_csv", ""),
    default_string("config/trust_decisions_json", "{}"),
];
const PLUGIN_MANIFEST: libconf_schema::RegistryManifest<'static> = manifest(
    "apps/anycode_plugins",
    RegistryScope::User,
    1,
    &["config"],
    PLUGIN_DEFAULTS,
    &[],
);

fn plugin_schema() -> ServiceSchema<'static> {
    ServiceSchema::new("anycode", &PLUGIN_MANIFEST)
}

fn system_plugin_dir() -> String {
    let _ = plugin_schema().register();
    plugin_schema()
        .read_string("config/system_dir")
        .unwrap_or_else(|| String::from("/System/Library/anycode/plugins"))
}

fn user_plugin_dir() -> String {
    let _ = plugin_schema().register();
    plugin_schema()
        .read_string("config/user_dir")
        .unwrap_or_else(|| String::from("/Users/Shared/Library/anycode/plugins"))
}

fn plugin_dirs(configured: &str, system_dir: &str, user_dir: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    for dir in configured.split('|') {
        let dir = dir.trim();
        if !dir.is_empty() && !dirs.iter().any(|existing| existing == dir) {
            dirs.push(String::from(dir));
        }
    }
    for dir in [system_dir, user_dir] {
        if !dir.is_empty() && !dirs.iter().any(|existing| existing == dir) {
            dirs.push(String::from(dir));
        }
    }
    dirs
}

// ════════════════════════════════════════════════════════════════
//  Plugin loading
// ════════════════════════════════════════════════════════════════

fn load_plugin(base_dir: &str, dir_name: &str) -> Option<Plugin> {
    let plugin_dir = format!("{}/{}", base_dir, dir_name);
    let json_path = format!("{}/plugin.json", plugin_dir);

    let data = anyos_std::fs::read_to_string(&json_path).ok()?;
    let val = Value::parse(&data).ok()?;

    let name = String::from(val["name"].as_str().unwrap_or(dir_name));
    let display_name = String::from(
        val["displayName"]
            .as_str()
            .unwrap_or(val["name"].as_str().unwrap_or(dir_name)),
    );
    let version = String::from(val["version"].as_str().unwrap_or("1.0.0"));
    let description = String::from(val["description"].as_str().unwrap_or(""));
    let author = String::from(val["author"].as_str().unwrap_or(""));

    // Parse extensions
    let mut extensions = Vec::new();
    if let Some(arr) = val["extensions"].as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                extensions.push(String::from(s));
            }
        }
    }

    // Parse capabilities
    let mut capabilities = Vec::new();
    if let Some(arr) = val["capabilities"].as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                match s {
                    "syntax" => capabilities.push(PluginCapability::Syntax),
                    "language" => capabilities.push(PluginCapability::Language),
                    "build" => capabilities.push(PluginCapability::Build),
                    "theme" => capabilities.push(PluginCapability::Theme),
                    "formatter" => capabilities.push(PluginCapability::Formatter),
                    "linter" => capabilities.push(PluginCapability::Linter),
                    "snippets" => capabilities.push(PluginCapability::Snippets),
                    "projectType" => capabilities.push(PluginCapability::ProjectType),
                    "command" => capabilities.push(PluginCapability::Command),
                    "panel" => capabilities.push(PluginCapability::Panel),
                    "code_action" | "codeAction" => capabilities.push(PluginCapability::CodeAction),
                    "test" => capabilities.push(PluginCapability::Test),
                    "ai_tool" | "aiTool" => capabilities.push(PluginCapability::AiTool),
                    _ => {}
                }
            }
        }
    }
    // Auto-detect capabilities from available files
    let syntax_path = val["syntax"]
        .as_str()
        .map(|s| format!("{}/{}", plugin_dir, s));
    if syntax_path.is_some() && !capabilities.contains(&PluginCapability::Syntax) {
        capabilities.push(PluginCapability::Syntax);
    }

    // Parse keywords
    let mut keywords = Vec::new();
    if let Some(arr) = val["keywords"].as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                keywords.push(String::from(s));
            }
        }
    }

    // Parse snippets
    let mut snippets = Vec::new();
    let snippets_path = format!("{}/snippets.json", plugin_dir);
    if let Ok(snippets_data) = anyos_std::fs::read_to_string(&snippets_path) {
        if let Ok(snippets_val) = Value::parse(&snippets_data) {
            if let Some(arr) = snippets_val.as_array() {
                for item in arr {
                    let prefix = item["prefix"].as_str().unwrap_or("");
                    let body = item["body"].as_str().unwrap_or("");
                    let desc = item["description"].as_str().unwrap_or("");
                    if !prefix.is_empty() {
                        snippets.push(PluginSnippet {
                            prefix: String::from(prefix),
                            body: String::from(body),
                            description: String::from(desc),
                        });
                    }
                }
            }
        }
    }
    if !snippets.is_empty() && !capabilities.contains(&PluginCapability::Snippets) {
        capabilities.push(PluginCapability::Snippets);
    }

    // Parse build config
    let mut build_config = None;
    let build_json_path = format!("{}/build.json", plugin_dir);
    if let Ok(build_data) = anyos_std::fs::read_to_string(&build_json_path) {
        if let Ok(bval) = Value::parse(&build_data) {
            build_config = Some(PluginBuildConfig {
                build_cmd: String::from(bval["build"]["command"].as_str().unwrap_or("")),
                build_args: String::from(bval["build"]["args"].as_str().unwrap_or("")),
                run_cmd: String::from(bval["run"]["command"].as_str().unwrap_or("")),
                run_args: String::from(bval["run"]["args"].as_str().unwrap_or("")),
            });
            if !capabilities.contains(&PluginCapability::Build) {
                capabilities.push(PluginCapability::Build);
            }
        }
    }

    // Theme support
    let theme_path = val["theme"]
        .as_str()
        .map(|s| format!("{}/{}", plugin_dir, s));
    if theme_path.is_some() && !capabilities.contains(&PluginCapability::Theme) {
        capabilities.push(PluginCapability::Theme);
    }

    // Project detection
    let mut project_files = Vec::new();
    if let Some(arr) = val["projectFiles"].as_array() {
        for item in arr {
            if let Some(s) = item.as_str() {
                project_files.push(String::from(s));
            }
        }
    }
    let project_type_name = val["projectTypeName"].as_str().map(String::from);
    let native_lib = val["native"]["library"]
        .as_str()
        .or_else(|| val["library"].as_str())
        .map(|s| format!("{}/{}", plugin_dir, s));
    let sdk_version = val["sdkVersion"].as_i64().unwrap_or(1).max(0) as u32;
    let abi = PluginAbi {
        sdk_version,
        library_path: native_lib,
        entry_init: String::from(
            val["native"]["entryInit"]
                .as_str()
                .unwrap_or("anycode_plugin_init"),
        ),
        entry_register: String::from(
            val["native"]["entryRegister"]
                .as_str()
                .unwrap_or("anycode_plugin_register"),
        ),
        entry_shutdown: String::from(
            val["native"]["entryShutdown"]
                .as_str()
                .unwrap_or("anycode_plugin_shutdown"),
        ),
    };
    let permissions = PluginPermissions {
        filesystem: json_bool_path(&val, "filesystem"),
        process: json_bool_path(&val, "process"),
        network: json_bool_path(&val, "network"),
        ai_context: json_bool_path(&val, "aiContext"),
        ui: json_bool_path(&val, "ui"),
    };

    Some(Plugin {
        name,
        display_name,
        version,
        description,
        author,
        dir_path: plugin_dir.clone(),
        bundle_path: plugin_dir,
        capabilities,
        state: PluginState::Active,
        abi,
        permissions,
        extensions,
        syntax_path,
        keywords,
        snippets,
        build_config,
        theme_path,
        project_files,
        project_type_name,
    })
}

fn json_bool_path(val: &Value, key: &str) -> bool {
    val["permissions"][key].as_bool().unwrap_or(false)
}

/// Legacy compat: load_plugins returns a Vec<Plugin> via PluginManager.
pub fn load_plugins(plugin_dir: &str) -> PluginManager {
    let mut mgr = PluginManager::new(plugin_dir);
    mgr.scan_and_load();
    mgr
}
