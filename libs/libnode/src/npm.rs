use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use anyos_std::fs;

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
        if self.data.contains(&format!("\"{}\"", spec.name)) {
            return;
        }
        let dep_line = format!("    \"{}\": \"{}\"", spec.name, spec.version);
        if let Some(pos) = self.data.find("\"dependencies\"") {
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
        self.data = format!("{{\n  \"dependencies\": {{\n{}\n  }}\n}}\n", dep_line);
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

    pub fn fetch_metadata(&self, package_name: &str) -> Option<PackageMetadata> {
        let url = self.package_metadata_url(package_name);
        let data = libhttp_client::get(&url)?;
        let text = String::from_utf8(data).ok()?;
        Some(PackageMetadata {
            package_name: String::from(package_name),
            raw_json: text,
        })
    }

    pub fn fetch_tarball(&self, url: &str) -> Option<Vec<u8>> {
        libhttp_client::get(url)
    }
}

pub struct PackageMetadata {
    pub package_name: String,
    pub raw_json: String,
}

impl PackageMetadata {
    pub fn resolve_version(&self, requested: &str) -> Option<String> {
        if requested == "latest" {
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
        let mut report = InstallReport {
            installed: Vec::new(),
        };
        let mut seen = Vec::new();
        self.install_recursive(root, spec, 0, &mut seen, &mut report)?;
        Ok(report)
    }

    pub fn install_manifest_dependencies(&self, root: &str) -> Option<InstallReport> {
        self.install_manifest_dependencies_result(root).ok()
    }

    pub fn install_manifest_dependencies_result(&self, root: &str) -> Result<InstallReport, String> {
        let manifest_path = join_path(root, "package.json");
        let manifest = fs::read_to_string(&manifest_path)
            .map_err(|_| format!("could not read {}", manifest_path))?;
        let deps = json_object_field(&manifest, "\"dependencies\"")
            .map(parse_dependency_object)
            .unwrap_or_default();
        let mut report = InstallReport {
            installed: Vec::new(),
        };
        let mut seen = Vec::new();
        for dep in deps {
            self.install_recursive(root, &dep, 0, &mut seen, &mut report)?;
        }
        Ok(report)
    }

    fn install_recursive(
        &self,
        root: &str,
        spec: &PackageSpec,
        depth: usize,
        seen: &mut Vec<String>,
        report: &mut InstallReport,
    ) -> Result<(), String> {
        if depth > self.max_depth {
            return Err(format!("dependency graph is deeper than {}", self.max_depth));
        }

        let metadata = self
            .client
            .fetch_metadata(&spec.name)
            .ok_or_else(|| format!("could not fetch metadata for {}", spec.name))?;
        let version = metadata
            .resolve_version(&spec.version)
            .ok_or_else(|| format!("could not resolve {}@{}", spec.name, spec.version))?;
        let key = format!("{}@{}", spec.name, version);
        if seen.iter().any(|entry| entry == &key) {
            return Ok(());
        }
        seen.push(key);

        let resolved = PackageSpec {
            name: spec.name.clone(),
            version,
        };
        let install_dir = package_install_dir(root, &resolved.name);
        if !installed_version_matches(&install_dir, &resolved.version) {
            let tarball = metadata
                .tarball_url(&resolved.version)
                .ok_or_else(|| format!("missing tarball for {}@{}", spec.name, resolved.version))?;
            let data = self
                .client
                .fetch_tarball(&tarball)
                .ok_or_else(|| format!("could not download {}", tarball))?;
            let cache_path = package_cache_path(root, &resolved);
            mkdir_p(&dirname(&cache_path));
            fs::write_bytes(&cache_path, &data)
                .map_err(|_| format!("could not write {}", cache_path))?;
            extract_npm_tarball(&cache_path, &install_dir).ok_or_else(|| {
                format!("could not extract {} into {}", cache_path, install_dir)
            })?;
            report.installed.push(resolved.clone());
        }

        for dep in metadata.dependencies(&resolved.version) {
            self.install_recursive(root, &dep, depth + 1, seen, report)?;
        }
        Ok(())
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
    let mut out = Vec::new();
    for pair in source.split(',') {
        let Some(colon) = pair.find(':') else {
            continue;
        };
        let name = pair[..colon].trim().trim_matches('"');
        let version = pair[colon + 1..].trim().trim_matches('"');
        if !name.is_empty() && !version.is_empty() {
            out.push(PackageSpec {
                name: name.to_string(),
                version: version.to_string(),
            });
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
            value = value.saturating_mul(10).saturating_add((byte - b'0') as u32);
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
        return Semver::parse(rest).map(|min| *version >= min).unwrap_or(false);
    }
    if let Some(rest) = token.strip_prefix('>') {
        return Semver::parse(rest).map(|min| *version > min).unwrap_or(false);
    }
    if let Some(rest) = token.strip_prefix("<=") {
        return Semver::parse(rest).map(|max| *version <= max).unwrap_or(false);
    }
    if let Some(rest) = token.strip_prefix('<') {
        return Semver::parse(rest).map(|max| *version < max).unwrap_or(false);
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

fn package_install_dir(root: &str, package_name: &str) -> String {
    if let Some(stripped) = package_name.strip_prefix('@') {
        if let Some(slash) = stripped.find('/') {
            return join_path(
                &join_path(&join_path(root, "node_modules"), &format!("@{}", &stripped[..slash])),
                &stripped[slash + 1..],
            );
        }
    }
    join_path(&join_path(root, "node_modules"), package_name)
}

fn package_cache_path(root: &str, spec: &PackageSpec) -> String {
    let safe_name = spec.name.replace('/', "__");
    join_path(
        &join_path(&join_path(root, "node_modules"), ".cache/anyos-npm"),
        &format!("{}-{}.tgz", safe_name, spec.version),
    )
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
                r#"{"versions":{"1.0.0":{},"1.2.3":{},"1.2.9":{},"1.3.0":{},"2.0.0":{}}}"#,
            ),
        };

        assert_eq!(metadata.resolve_version("~1.2.0").as_deref(), Some("1.2.9"));
        assert_eq!(metadata.resolve_version("^1.2.0").as_deref(), Some("1.3.0"));
        assert_eq!(metadata.resolve_version(">=1.2.3 <2.0.0").as_deref(), Some("1.3.0"));
    }

    #[test]
    fn scoped_registry_url_is_encoded_like_npm() {
        let client = RegistryClient::new(RegistryConfig::default());
        assert_eq!(
            client.package_metadata_url("@scope/pkg"),
            "https://registry.npmjs.org/@scope%2fpkg"
        );
    }
}
