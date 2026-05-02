use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::fs;
use anyos_std::json::Value;

use crate::DEFAULT_NPM_REGISTRY;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageSpec {
    pub name: String,
    pub version: String,
}

impl PackageSpec {
    pub fn parse(spec: &str) -> Self {
        if let Some(pos) = spec.rfind('@') {
            if pos > 0 {
                return Self {
                    name: String::from(&spec[..pos]),
                    version: String::from(&spec[pos + 1..]),
                };
            }
        }
        Self {
            name: String::from(spec),
            version: String::from("latest"),
        }
    }

    fn registry_spec(&self) -> Self {
        self.version
            .strip_prefix("npm:")
            .map(Self::parse)
            .unwrap_or_else(|| self.clone())
    }
}

#[derive(Clone, Debug)]
pub struct RegistryConfig {
    pub url: String,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            url: String::from(DEFAULT_NPM_REGISTRY),
        }
    }
}

impl RegistryConfig {
    pub fn normalized_url(&self) -> String {
        normalize_registry_url(&self.url)
    }

    pub fn package_url(&self, package_name: &str) -> String {
        format!(
            "{}{}",
            self.normalized_url(),
            encode_package_name(package_name)
        )
    }
}

pub struct PackageManifest {
    data: String,
}

impl PackageManifest {
    pub fn new_app(name: &str) -> Self {
        Self {
            data: format!(
                "{{\n  \"name\": \"{}\",\n  \"version\": \"0.1.0\",\n  \"type\": \"module\",\n  \"scripts\": {{\n    \"start\": \"node src/main.js\"\n  }},\n  \"dependencies\": {{}}\n}}\n",
                name
            ),
        }
    }

    pub fn parse_or_new(data: Option<String>) -> Self {
        data.map(|data| Self { data })
            .unwrap_or_else(|| Self::new_app("anyos-js-app"))
    }

    pub fn add_dependency(&mut self, spec: &PackageSpec) {
        self.add_dependency_to("\"dependencies\"", spec);
    }

    pub fn add_dev_dependency(&mut self, spec: &PackageSpec) {
        self.add_dependency_to("\"devDependencies\"", spec);
    }

    fn add_dependency_to(&mut self, field: &str, spec: &PackageSpec) {
        if self.data.contains(&format!("\"{}\"", spec.name)) {
            return;
        }
        let dep_line = format!("    \"{}\": \"{}\"", spec.name, spec.version);
        if let Some(pos) = self.data.find(field) {
            if let Some(open_rel) = self.data[pos..].find('{') {
                let open = pos + open_rel + 1;
                let close = self.data[open..]
                    .find('}')
                    .map(|idx| open + idx)
                    .unwrap_or(open);
                let existing = self.data[open..close].trim();
                let mut out = String::new();
                out.push_str(&self.data[..open]);
                out.push('\n');
                if !existing.is_empty() {
                    out.push_str(&self.data[open..close]);
                    if !self.data[open..close].trim_end().ends_with(',') {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str(&dep_line);
                out.push('\n');
                out.push_str(&self.data[close..]);
                self.data = out;
                return;
            }
        }
        self.insert_dependency_group(field, &dep_line);
    }

    fn insert_dependency_group(&mut self, field: &str, dep_line: &str) {
        let key = field.trim_matches('"');
        let group = format!("  \"{}\": {{\n{}\n  }}", key, dep_line);
        let trimmed = self.data.trim();
        if trimmed.starts_with('{') && trimmed.ends_with('}') {
            let close = self.data.rfind('}').unwrap_or(self.data.len());
            let before_close = self.data[..close].trim_end();
            let needs_comma = before_close.len() > 1 && !before_close.ends_with('{');
            let mut out = String::new();
            out.push_str(before_close);
            if needs_comma {
                out.push(',');
            }
            out.push('\n');
            out.push_str(&group);
            out.push('\n');
            out.push_str(&self.data[close..]);
            self.data = out;
        } else {
            self.data = format!("{{\n{}\n}}\n", group);
        }
    }

    pub fn remove_dependency(&mut self, name: &str) -> bool {
        let mut changed = self.remove_dependency_from("\"dependencies\"", name);
        changed = self.remove_dependency_from("\"devDependencies\"", name) || changed;
        changed
    }

    fn remove_dependency_from(&mut self, field: &str, name: &str) -> bool {
        let Some(pos) = self.data.find(field) else {
            return false;
        };
        let Some(open_rel) = self.data[pos..].find('{') else {
            return false;
        };
        let open = pos + open_rel + 1;
        let close = self.data[open..]
            .find('}')
            .map(|idx| open + idx)
            .unwrap_or(open);
        let deps = parse_dependency_object(&self.data[open..close]);
        if !deps.iter().any(|dep| dep.name == name) {
            return false;
        }
        let remaining: Vec<PackageSpec> = deps.into_iter().filter(|dep| dep.name != name).collect();
        let mut replacement = String::new();
        if !remaining.is_empty() {
            replacement.push('\n');
            for (idx, dep) in remaining.iter().enumerate() {
                replacement.push_str(&format!("    \"{}\": \"{}\"", dep.name, dep.version));
                if idx + 1 < remaining.len() {
                    replacement.push(',');
                }
                replacement.push('\n');
            }
        }
        let mut out = String::new();
        out.push_str(&self.data[..open]);
        out.push_str(&replacement);
        out.push_str(&self.data[close..]);
        self.data = out;
        true
    }

    pub fn dependencies(&self) -> Vec<PackageSpec> {
        self.dependency_group("\"dependencies\"")
    }

    pub fn dev_dependencies(&self) -> Vec<PackageSpec> {
        self.dependency_group("\"devDependencies\"")
    }

    pub fn manifest_dependencies(&self, include_dev: bool) -> Vec<PackageSpec> {
        let mut deps = self.dependencies();
        if include_dev {
            for dep in self.dev_dependencies() {
                if !deps.iter().any(|existing| existing.name == dep.name) {
                    deps.push(dep);
                }
            }
        }
        deps
    }

    fn dependency_group(&self, field: &str) -> Vec<PackageSpec> {
        self.data
            .find(field)
            .and_then(|pos| json_object_field(&self.data[pos..], field))
            .map(parse_dependency_object)
            .unwrap_or_default()
    }

    pub fn script(&self, name: &str) -> Option<String> {
        let value = Value::parse(&self.data).ok()?;
        value["scripts"][name].as_str().map(String::from)
    }

    pub fn as_str(&self) -> &str {
        &self.data
    }
}

pub struct RegistryClient {
    config: RegistryConfig,
}

impl RegistryClient {
    pub fn new(config: RegistryConfig) -> Self {
        let _ = libhttp_client::init();
        Self { config }
    }

    pub fn package_metadata_url(&self, package_name: &str) -> String {
        self.config.package_url(package_name)
    }

    pub fn package_tarball_url(&self, package_name: &str, version: &str) -> String {
        let basename = package_binary_name(package_name);
        format!(
            "{}{}/-/{}-{}.tgz",
            self.config.normalized_url(),
            encode_package_name(package_name),
            basename,
            version
        )
    }

    pub fn fetch_metadata(&self, package_name: &str) -> Option<PackageMetadata> {
        let url = self.package_metadata_url(package_name);
        let text = match libhttp_client::get(&url).and_then(|data| String::from_utf8(data).ok()) {
            Some(text) if metadata_looks_complete(&text, package_name) => text,
            _ => {
                let path = metadata_cache_path(package_name);
                mkdir_p(&dirname(&path));
                if !libhttp_client::download(&url, &path) {
                    return None;
                }
                fs::read_to_string(&path).ok()?
            }
        };
        Some(PackageMetadata {
            package_name: String::from(package_name),
            raw_json: text,
        })
    }

    pub fn fetch_tarball(&self, url: &str) -> Option<Vec<u8>> {
        libhttp_client::get(url)
    }

    pub fn download_tarball(&self, url: &str, path: &str) -> bool {
        libhttp_client::download(url, path)
    }
}

pub struct PackageMetadata {
    pub package_name: String,
    pub raw_json: String,
}

impl PackageMetadata {
    pub fn resolve_version(&self, requested: &str) -> Option<String> {
        if requested == "latest" || requested == "*" {
            json_nested_string(&self.raw_json, "\"dist-tags\"", "\"latest\"")
        } else if self.has_version(requested) {
            Some(String::from(requested))
        } else {
            self.best_matching_version(requested)
        }
    }

    pub fn tarball_url(&self, version: &str) -> Option<String> {
        let version_object = self.version_object(version)?;
        let dist_object = json_object_field(version_object, "\"dist\"")?;
        json_string_field(dist_object, "\"tarball\"")
    }

    pub fn dependencies(&self, version: &str) -> Vec<PackageSpec> {
        let Some(version_object) = self.version_object(version) else {
            return Vec::new();
        };
        let Some(deps) = json_object_field(version_object, "\"dependencies\"") else {
            return Vec::new();
        };
        parse_dependency_object(deps)
    }

    pub fn optional_dependencies(&self, version: &str) -> Vec<PackageSpec> {
        let Some(version_object) = self.version_object(version) else {
            return Vec::new();
        };
        let Some(deps) = json_object_field(version_object, "\"optionalDependencies\"") else {
            return Vec::new();
        };
        parse_dependency_object(deps)
    }

    pub fn bin_entries(&self, version: &str) -> Vec<(String, String)> {
        let Some(version_object) = self.version_object(version) else {
            return Vec::new();
        };
        let package_bin_name = package_binary_name(&self.package_name);
        if let Some(bin_object) = json_object_field(version_object, "\"bin\"") {
            return parse_string_object(bin_object);
        }
        json_string_field(version_object, "\"bin\"")
            .map(|path| alloc::vec![(package_bin_name, path)])
            .unwrap_or_default()
    }

    fn has_version(&self, version: &str) -> bool {
        self.version_object(version).is_some()
    }

    fn version_object(&self, version: &str) -> Option<&str> {
        let versions = json_object_field(&self.raw_json, "\"versions\"")?;
        json_object_field(versions, &format!("\"{}\"", version))
    }

    fn best_matching_version(&self, requested: &str) -> Option<String> {
        let versions = json_object_field(&self.raw_json, "\"versions\"")?;
        let mut best: Option<Semver> = None;
        for version in collect_version_keys(versions) {
            let Some(parsed) = Semver::parse(&version) else {
                continue;
            };
            if !version_satisfies(&parsed, requested) {
                continue;
            }
            if best.map(|current| parsed > current).unwrap_or(true) {
                best = Some(parsed);
            }
        }
        best.map(|version| version.to_string())
    }
}

#[derive(Clone, Debug)]
pub struct InstallReport {
    pub installed: Vec<PackageSpec>,
}

#[derive(Clone, Copy, Debug)]
pub struct InstallManifestOptions {
    pub include_dev: bool,
}

impl Default for InstallManifestOptions {
    fn default() -> Self {
        Self { include_dev: true }
    }
}

struct InstallLayout {
    node_modules_dir: String,
    bin_dir: String,
    global_bin_dir: Option<String>,
}

impl InstallLayout {
    fn local(root: &str) -> Self {
        let node_modules_dir = join_path(root, "node_modules");
        let bin_dir = join_path(&node_modules_dir, ".bin");
        Self {
            node_modules_dir,
            bin_dir,
            global_bin_dir: None,
        }
    }

    fn global(prefix: &str) -> Self {
        let normalized = normalize_path(prefix);
        let node_modules_dir = join_path(&join_path(&normalized, "Library"), "node_modules");
        let bin_dir = join_path(&node_modules_dir, ".bin");
        let global_bin_dir = Some(join_path(&normalized, "bin"));
        Self {
            node_modules_dir,
            bin_dir,
            global_bin_dir,
        }
    }
}

pub struct PackageInstaller {
    client: RegistryClient,
    max_depth: usize,
}

impl PackageInstaller {
    pub fn new(config: RegistryConfig) -> Self {
        let _ = libzip_client::init();
        Self {
            client: RegistryClient::new(config),
            max_depth: 24,
        }
    }

    pub fn install_package(&self, root: &str, spec: &PackageSpec) -> Option<InstallReport> {
        self.install_package_result(root, spec).ok()
    }

    pub fn install_package_result(
        &self,
        root: &str,
        spec: &PackageSpec,
    ) -> Result<InstallReport, String> {
        let layout = InstallLayout::local(root);
        self.install_package_with_layout(&layout, spec)
    }

    pub fn install_global_package_result(
        &self,
        prefix: &str,
        spec: &PackageSpec,
    ) -> Result<InstallReport, String> {
        let layout = InstallLayout::global(prefix);
        self.install_package_with_layout(&layout, spec)
    }

    fn install_package_with_layout(
        &self,
        layout: &InstallLayout,
        spec: &PackageSpec,
    ) -> Result<InstallReport, String> {
        let mut report = InstallReport {
            installed: Vec::new(),
        };
        let mut seen = Vec::new();
        self.install_recursive(layout, spec, 0, &mut seen, &mut report)?;
        Ok(report)
    }

    pub fn install_manifest_dependencies(&self, root: &str) -> Option<InstallReport> {
        self.install_manifest_dependencies_result(root).ok()
    }

    pub fn install_manifest_dependencies_result(
        &self,
        root: &str,
    ) -> Result<InstallReport, String> {
        self.install_manifest_dependencies_with_options_result(
            root,
            InstallManifestOptions::default(),
        )
    }

    pub fn install_manifest_dependencies_with_options_result(
        &self,
        root: &str,
        options: InstallManifestOptions,
    ) -> Result<InstallReport, String> {
        let manifest_path = join_path(root, "package.json");
        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|_| format!("could not read {}", manifest_path))?;
        let deps = PackageManifest::parse_or_new(Some(manifest))
            .manifest_dependencies(options.include_dev);
        let mut report = InstallReport {
            installed: Vec::new(),
        };
        let mut seen = Vec::new();
        let layout = InstallLayout::local(root);
        for dep in deps {
            self.install_recursive(&layout, &dep, 0, &mut seen, &mut report)?;
        }
        Ok(report)
    }

    fn install_recursive(
        &self,
        layout: &InstallLayout,
        spec: &PackageSpec,
        depth: usize,
        seen: &mut Vec<String>,
        report: &mut InstallReport,
    ) -> Result<(), String> {
        if depth > self.max_depth {
            return Err(format!(
                "dependency graph is deeper than {}",
                self.max_depth
            ));
        }

        let registry_spec = spec.registry_spec();
        if self.install_system_package(layout, &registry_spec, seen, report)? {
            return Ok(());
        }
        let metadata = self
            .client
            .fetch_metadata(&registry_spec.name)
            .ok_or_else(|| format!("could not fetch metadata for {}", registry_spec.name))?;
        let version = metadata
            .resolve_version(&registry_spec.version)
            .ok_or_else(|| {
                format!(
                    "could not resolve {}@{}",
                    registry_spec.name, registry_spec.version
                )
            })?;
        let key = format!("{}@{}", spec.name, version);
        if seen.iter().any(|entry| entry == &key) {
            return Ok(());
        }
        seen.push(key);

        let resolved = PackageSpec {
            name: spec.name.clone(),
            version,
        };
        let install_dir = package_install_dir(&layout.node_modules_dir, &resolved.name);
        if !installed_version_matches(&install_dir, &resolved.version) {
            let tarball = metadata.tarball_url(&resolved.version).unwrap_or_else(|| {
                self.client
                    .package_tarball_url(&registry_spec.name, &resolved.version)
            });
            let cache_path = package_cache_path(&layout.node_modules_dir, &resolved);
            mkdir_p(&dirname(&cache_path));
            if !self.client.download_tarball(&tarball, &cache_path) {
                return Err(format!("could not download {}", tarball));
            }
            extract_npm_tarball(&cache_path, &install_dir)
                .ok_or_else(|| format!("could not extract {} into {}", cache_path, install_dir))?;
            mark_package_executables(&install_dir);
            install_bin_entries(layout, &resolved, &metadata);
            report.installed.push(resolved.clone());
        } else {
            install_bin_entries(layout, &resolved, &metadata);
        }

        for dep in metadata.dependencies(&resolved.version) {
            self.install_recursive(layout, &dep, depth + 1, seen, report)?;
        }
        for dep in metadata.optional_dependencies(&resolved.version) {
            let _ = self.install_recursive(layout, &dep, depth + 1, seen, report);
        }
        Ok(())
    }

    fn install_system_package(
        &self,
        layout: &InstallLayout,
        spec: &PackageSpec,
        seen: &mut Vec<String>,
        report: &mut InstallReport,
    ) -> Result<bool, String> {
        let Some(source_dir) = find_system_package_dir(&spec.name) else {
            return Ok(false);
        };
        let package_json_path = join_path(&source_dir, "package.json");
        let package_json = fs::read_to_string(&package_json_path)
            .map_err(|_| format!("could not read {}", package_json_path))?;
        let version = json_string_field(&package_json, "\"version\"")
            .unwrap_or_else(|| String::from("0.0.0"));
        let key = format!("{}@{}", spec.name, version);
        if seen.iter().any(|entry| entry == &key) {
            return Ok(true);
        }
        seen.push(key);

        let install_dir = package_install_dir(&layout.node_modules_dir, &spec.name);
        if !installed_version_matches(&install_dir, &version) {
            copy_package_tree(&source_dir, &install_dir)
                .ok_or_else(|| format!("could not install system package {}", spec.name))?;
            mark_package_executables(&install_dir);
            report.installed.push(PackageSpec {
                name: spec.name.clone(),
                version: version.clone(),
            });
        }

        let manifest = PackageManifest::parse_or_new(Some(package_json));
        for dep in manifest.dependencies() {
            self.install_recursive(layout, &dep, 1, seen, report)?;
        }
        Ok(true)
    }
}

pub fn normalize_registry_url(registry: &str) -> String {
    if registry.ends_with('/') {
        String::from(registry)
    } else {
        format!("{}/", registry)
    }
}

pub fn encode_package_name(package_name: &str) -> String {
    if package_name.starts_with('@') {
        package_name.replace("/", "%2f")
    } else {
        String::from(package_name)
    }
}

fn json_nested_string(source: &str, outer_key: &str, inner_key: &str) -> Option<String> {
    let outer_pos = find_json_key(source, outer_key)?;
    json_string_field(&source[outer_pos..], inner_key)
}

fn json_object_field<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    let key_pos = find_json_key(source, key)?;
    let after_key = &source[key_pos + key.len()..];
    let colon = after_key.find(':')?;
    let rest = &after_key[colon + 1..];
    let open_rel = rest.find('{')?;
    let open = key_pos + key.len() + colon + 1 + open_rel;
    let close = matching_brace(source, open)?;
    Some(&source[open + 1..close])
}

fn matching_brace(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open) != Some(&b'{') {
        return None;
    }
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (idx, &byte) in bytes.iter().enumerate().skip(open) {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(idx);
                }
            }
            _ => {}
        }
    }
    None
}

fn json_string_field(source: &str, key: &str) -> Option<String> {
    let key_pos = find_json_key(source, key)?;
    let after_key = &source[key_pos + key.len()..];
    let colon = after_key.find(':')?;
    let mut rest = after_key[colon + 1..].trim_start();
    if !rest.starts_with('"') {
        return None;
    }
    rest = &rest[1..];
    let mut out = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(out);
        } else {
            out.push(ch);
        }
    }
    None
}

fn find_json_key(source: &str, key: &str) -> Option<usize> {
    let needle = key.trim_matches('"');
    let bytes = source.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let quote_start = i;
        let start = i + 1;
        i = start;
        while i < bytes.len() && bytes[i] != b'"' {
            if bytes[i] == b'\\' {
                i += 1;
            }
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let end = i;
        i += 1;
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b':' {
            continue;
        }
        let Ok(candidate) = core::str::from_utf8(&bytes[start..end]) else {
            continue;
        };
        if candidate == needle {
            return Some(quote_start);
        }
    }
    None
}

fn parse_dependency_object(source: &str) -> Vec<PackageSpec> {
    parse_string_object(source)
        .into_iter()
        .map(|(name, version)| PackageSpec { name, version })
        .collect()
}

fn parse_string_object(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for pair in source.split(',') {
        let Some(colon) = pair.find(':') else {
            continue;
        };
        let key = pair[..colon].trim().trim_matches('"');
        let value = pair[colon + 1..].trim().trim_matches('"');
        if !key.is_empty() && !value.is_empty() {
            out.push((key.to_string(), value.to_string()));
        }
    }
    out
}

fn collect_version_keys(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'"' {
            i += 1;
            continue;
        }
        let start = i + 1;
        i = start;
        while i < bytes.len() && bytes[i] != b'"' {
            if bytes[i] == b'\\' {
                i += 1;
            }
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let end = i;
        i += 1;
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j + 1 >= bytes.len() || bytes[j] != b':' {
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'{' {
            continue;
        }
        let Ok(version) = core::str::from_utf8(&bytes[start..end]) else {
            continue;
        };
        if Semver::parse(version).is_some() {
            out.push(String::from(version));
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Semver {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Semver {
    fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim().trim_start_matches('v').trim_start_matches('=');
        let mut parts = trimmed.split('.');
        let major = parse_numeric_part(parts.next()?)?;
        let minor = parse_numeric_part(parts.next().unwrap_or("0"))?;
        let patch = parse_numeric_part(parts.next().unwrap_or("0"))?;
        Some(Self {
            major,
            minor,
            patch,
        })
    }

    fn to_string(self) -> String {
        format!("{}.{}.{}", self.major, self.minor, self.patch)
    }
}

fn parse_numeric_part(input: &str) -> Option<u32> {
    let mut value = 0u32;
    let mut seen_digit = false;
    for byte in input.as_bytes() {
        if byte.is_ascii_digit() {
            seen_digit = true;
            value = value
                .saturating_mul(10)
                .saturating_add((byte - b'0') as u32);
        } else {
            break;
        }
    }
    seen_digit.then_some(value)
}

fn version_satisfies(version: &Semver, requested: &str) -> bool {
    let requested = requested.trim();
    if requested.is_empty() || requested == "*" || requested == "latest" {
        return true;
    }
    if requested.contains("||") {
        return requested
            .split("||")
            .any(|part| version_satisfies(version, part));
    }
    let mut pending_operator: Option<&str> = None;
    for token in requested.split_whitespace() {
        let token = token.trim_matches(',');
        if token.is_empty() || token == "*" {
            continue;
        }
        if matches!(token, ">" | ">=" | "<" | "<=") {
            pending_operator = Some(token);
            continue;
        }
        let combined;
        let token = if let Some(operator) = pending_operator.take() {
            combined = format!("{}{}", operator, token);
            combined.as_str()
        } else {
            token
        };
        if !version_satisfies_token(version, token) {
            return false;
        }
    }
    true
}

fn version_satisfies_token(version: &Semver, token: &str) -> bool {
    if let Some(rest) = token.strip_prefix(">=") {
        return Semver::parse(rest)
            .map(|min| *version >= min)
            .unwrap_or(false);
    }
    if let Some(rest) = token.strip_prefix('>') {
        return Semver::parse(rest)
            .map(|min| *version > min)
            .unwrap_or(false);
    }
    if let Some(rest) = token.strip_prefix("<=") {
        return Semver::parse(rest)
            .map(|max| *version <= max)
            .unwrap_or(false);
    }
    if let Some(rest) = token.strip_prefix('<') {
        return Semver::parse(rest)
            .map(|max| *version < max)
            .unwrap_or(false);
    }
    if let Some(rest) = token.strip_prefix('~') {
        let Some(base) = Semver::parse(rest) else {
            return false;
        };
        return *version >= base && version.major == base.major && version.minor == base.minor;
    }
    if let Some(rest) = token.strip_prefix('^') {
        let Some(base) = Semver::parse(rest) else {
            return false;
        };
        if *version < base {
            return false;
        }
        if base.major > 0 {
            return version.major == base.major;
        }
        if base.minor > 0 {
            return version.major == 0 && version.minor == base.minor;
        }
        return version.major == 0 && version.minor == 0 && version.patch == base.patch;
    }
    if token.contains('x') || token.contains('X') || token.contains('*') {
        return wildcard_version_matches(version, token);
    }
    Semver::parse(token)
        .map(|exact| *version == exact)
        .unwrap_or(false)
}

fn wildcard_version_matches(version: &Semver, token: &str) -> bool {
    let mut parts = token.trim_start_matches('v').split('.');
    let major = parts.next().unwrap_or("*");
    let minor = parts.next().unwrap_or("*");
    let patch = parts.next().unwrap_or("*");
    numeric_or_wildcard_matches(version.major, major)
        && numeric_or_wildcard_matches(version.minor, minor)
        && numeric_or_wildcard_matches(version.patch, patch)
}

fn numeric_or_wildcard_matches(value: u32, token: &str) -> bool {
    matches!(token, "*" | "x" | "X") || parse_numeric_part(token) == Some(value)
}

fn package_install_dir(node_modules_dir: &str, package_name: &str) -> String {
    if let Some(stripped) = package_name.strip_prefix('@') {
        if let Some(slash) = stripped.find('/') {
            return join_path(
                &join_path(node_modules_dir, &format!("@{}", &stripped[..slash])),
                &stripped[slash + 1..],
            );
        }
    }
    join_path(node_modules_dir, package_name)
}

fn find_system_package_dir(package_name: &str) -> Option<String> {
    for base in system_package_bases() {
        let candidate = package_install_dir(&base, package_name);
        if fs::read_to_string(&join_path(&candidate, "package.json")).is_ok() {
            return Some(candidate);
        }
    }
    None
}

fn system_package_bases() -> Vec<String> {
    let mut bases = Vec::new();
    #[cfg(feature = "host")]
    if let Ok(value) = std::env::var("ANYOS_NODE_SYSTEM_PACKAGES") {
        for base in value.split(':') {
            let base = base.trim();
            if !base.is_empty() {
                bases.push(normalize_path(base));
            }
        }
    }
    bases.push(String::from("/System/Library/node_modules"));
    bases
}

fn copy_package_tree(source_dir: &str, install_dir: &str) -> Option<()> {
    mkdir_p(install_dir);
    copy_dir_recursive(source_dir, install_dir, 0)
}

fn copy_dir_recursive(source_dir: &str, install_dir: &str, depth: usize) -> Option<()> {
    if depth > 32 {
        return None;
    }
    let mut buf = alloc::vec![0u8; 8192];
    let count = fs::readdir(source_dir, &mut buf);
    if count == u32::MAX {
        return None;
    }
    for index in 0..count as usize {
        let base = index * 64;
        if base + 64 > buf.len() {
            break;
        }
        let entry_type = buf[base];
        let name_len = buf[base + 1] as usize;
        let name_start = base + 8;
        let name_end = (name_start + name_len).min(base + 64);
        let name = core::str::from_utf8(&buf[name_start..name_end]).ok()?;
        if name.is_empty() || name == "." || name == ".." {
            continue;
        }
        let source = join_path(source_dir, name);
        let target = join_path(install_dir, name);
        if entry_type == 1 {
            mkdir_p(&target);
            copy_dir_recursive(&source, &target, depth + 1)?;
        } else {
            let data = read_file_bytes(&source)?;
            mkdir_p(&dirname(&target));
            fs::write_bytes(&target, &data).ok()?;
        }
    }
    Some(())
}

fn read_file_bytes(path: &str) -> Option<Vec<u8>> {
    #[cfg(feature = "host")]
    {
        fs::read_to_vec(path)
    }
    #[cfg(not(feature = "host"))]
    {
        fs::read_to_vec(path).ok()
    }
}

fn package_cache_path(node_modules_dir: &str, spec: &PackageSpec) -> String {
    let safe_name = spec.name.replace('/', "__");
    join_path(
        &join_path(node_modules_dir, ".cache/anyos-npm"),
        &format!("{}-{}.tgz", safe_name, spec.version),
    )
}

fn metadata_cache_path(package_name: &str) -> String {
    let safe_name = package_name.replace('@', "_at_").replace('/', "__");
    join_path(
        &join_path("node_modules", ".cache/anyos-npm/metadata"),
        &format!("{}.json", safe_name),
    )
}

fn metadata_looks_complete(source: &str, package_name: &str) -> bool {
    source.contains("\"dist-tags\"")
        && source.contains("\"versions\"")
        && source.contains(&format!("\"name\":\"{}\"", package_name))
        && source.trim_end().ends_with('}')
}

fn package_binary_name(package_name: &str) -> String {
    package_name
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(package_name)
        .to_string()
}

fn install_bin_entries(layout: &InstallLayout, spec: &PackageSpec, metadata: &PackageMetadata) {
    let entries = metadata.bin_entries(&spec.version);
    if entries.is_empty() {
        return;
    }
    mkdir_p(&layout.bin_dir);
    if let Some(global_bin_dir) = &layout.global_bin_dir {
        mkdir_p(global_bin_dir);
    }
    for (name, target) in entries {
        let clean_target = target.trim_start_matches("./");
        if clean_target.is_empty()
            || clean_target.starts_with('/')
            || clean_target.contains("../")
            || clean_target == ".."
        {
            continue;
        }
        let package_path = format!("../{}/{}", spec.name, clean_target);
        let shim_path = join_path(&layout.bin_dir, &name);
        let source = format!("require({:?});\n", package_path);
        let _ = fs::write_bytes(&shim_path, source.as_bytes());
        if let Some(global_bin_dir) = &layout.global_bin_dir {
            let global_shim_path = join_path(global_bin_dir, &name);
            let _ = fs::unlink(&global_shim_path);
            if fs::symlink(&shim_path, &global_shim_path) != 0 {
                let _ = fs::write_bytes(&global_shim_path, source.as_bytes());
            }
        }
    }
}

fn installed_version_matches(package_dir: &str, version: &str) -> bool {
    let package_json = join_path(package_dir, "package.json");
    let Ok(data) = fs::read_to_string(&package_json) else {
        return false;
    };
    json_string_field(&data, "\"version\"")
        .map(|installed| installed == version)
        .unwrap_or(false)
}

fn extract_npm_tarball(cache_path: &str, package_dir: &str) -> Option<()> {
    let reader = libzip_client::TarReader::open(cache_path)?;
    mkdir_p(package_dir);
    for index in 0..reader.entry_count() {
        let name = reader.entry_name(index);
        let Some(path) = npm_tar_entry_path(&name) else {
            continue;
        };
        let out_path = join_path(package_dir, &path);
        if reader.entry_is_dir(index) {
            mkdir_p(&out_path);
            continue;
        }
        let parent = dirname(&out_path);
        mkdir_p(&parent);
        let data = reader.extract(index)?;
        fs::write_bytes(&out_path, &data).ok()?;
    }
    Some(())
}

fn mark_package_executables(package_dir: &str) {
    mark_executables_recursive(package_dir, 0);
}

fn mark_executables_recursive(path: &str, depth: usize) {
    if depth > 16 {
        return;
    }
    let mut buf = alloc::vec![0u8; 8192];
    let count = fs::readdir(path, &mut buf);
    if count == u32::MAX {
        return;
    }
    for index in 0..count as usize {
        let base = index * 64;
        if base + 64 > buf.len() {
            break;
        }
        let entry_type = buf[base] as u32;
        let name_len = buf[base + 1] as usize;
        let name_start = base + 8;
        let name_end = (name_start + name_len).min(base + 64);
        let Ok(name) = core::str::from_utf8(&buf[name_start..name_end]) else {
            continue;
        };
        if name == "." || name == ".." || name.is_empty() {
            continue;
        }
        let full = join_path(path, name);
        if entry_type == 1 {
            mark_executables_recursive(&full, depth + 1);
        } else if should_mark_executable(&full, name) {
            let _ = fs::chmod(&full, 0o755);
        }
    }
}

fn should_mark_executable(path: &str, name: &str) -> bool {
    path.contains("/bin/")
        || path.contains("/vendor/")
        || matches!(name, "codex" | "codex.exe" | "rg" | "node" | "npm")
}

fn npm_tar_entry_path(name: &str) -> Option<String> {
    let stripped = name.strip_prefix("package/").unwrap_or(name);
    if stripped.is_empty()
        || stripped.starts_with('/')
        || stripped.contains("../")
        || stripped == ".."
        || stripped.starts_with("./")
    {
        return None;
    }
    Some(String::from(stripped))
}

fn mkdir_p(path: &str) {
    if path.is_empty() || path == "." {
        return;
    }
    let normalized = normalize_path(path);
    let mut current = String::new();
    for part in normalized.split('/') {
        if part.is_empty() {
            current.push('/');
            continue;
        }
        if current.is_empty() || current == "/" {
            current.push_str(part);
        } else {
            current.push('/');
            current.push_str(part);
        }
        let _ = fs::mkdir(&current);
    }
}

fn dirname(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(0) => String::from("/"),
        Some(idx) => String::from(&trimmed[..idx]),
        None => String::from("."),
    }
}

fn join_path(left: &str, right: &str) -> String {
    if right.starts_with('/') {
        return normalize_path(right);
    }
    if left.is_empty() || left == "." {
        normalize_path(right)
    } else if left.ends_with('/') {
        normalize_path(&format!("{}{}", left, right))
    } else {
        normalize_path(&format!("{}/{}", left, right))
    }
}

fn normalize_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(part),
        }
    }
    let mut out = String::new();
    if absolute {
        out.push('/');
    }
    out.push_str(&parts.join("/"));
    if out.is_empty() {
        String::from(if absolute { "/" } else { "." })
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_scoped_and_unscoped_specs() {
        assert_eq!(
            PackageSpec::parse("left-pad@1.3.0"),
            PackageSpec {
                name: String::from("left-pad"),
                version: String::from("1.3.0")
            }
        );
        assert_eq!(
            PackageSpec::parse("@scope/pkg@2.0.0"),
            PackageSpec {
                name: String::from("@scope/pkg"),
                version: String::from("2.0.0")
            }
        );
    }

    #[test]
    fn dependency_is_inserted_into_manifest() {
        let mut manifest = PackageManifest::parse_or_new(None);
        manifest.add_dependency(&PackageSpec::parse("left-pad@1.3.0"));
        assert!(manifest.as_str().contains("\"left-pad\": \"1.3.0\""));
    }

    #[test]
    fn semver_ranges_resolve_to_highest_matching_version() {
        let metadata = PackageMetadata {
            package_name: String::from("demo"),
            raw_json: String::from(
                r#"{"dist-tags":{"latest":"1.3.0"},"versions":{"1.0.0":{},"1.2.3":{},"1.2.9":{},"1.3.0":{},"2.0.0":{}}}"#,
            ),
        };

        assert_eq!(metadata.resolve_version("*").as_deref(), Some("1.3.0"));
        assert_eq!(metadata.resolve_version("~1.2.0").as_deref(), Some("1.2.9"));
        assert_eq!(metadata.resolve_version("^1.2.0").as_deref(), Some("1.3.0"));
        assert_eq!(
            metadata.resolve_version(">=1.2.3 <2.0.0").as_deref(),
            Some("1.3.0")
        );
    }

    #[test]
    fn scoped_registry_url_is_encoded_like_npm() {
        let client = RegistryClient::new(RegistryConfig::default());
        assert_eq!(
            client.package_metadata_url("@scope/pkg"),
            "https://registry.npmjs.org/@scope%2fpkg"
        );
    }

    #[test]
    fn scoped_tarball_url_matches_npm_registry_shape() {
        let client = RegistryClient::new(RegistryConfig::default());
        assert_eq!(
            client.package_tarball_url("@types/node", "25.6.0"),
            "https://registry.npmjs.org/@types%2fnode/-/node-25.6.0.tgz"
        );
    }
}
