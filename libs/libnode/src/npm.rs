use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

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
}

pub struct PackageMetadata {
    pub package_name: String,
    pub raw_json: String,
}

impl PackageMetadata {
    pub fn resolve_version(&self, requested: &str) -> Option<String> {
        if requested == "latest" {
            json_nested_string(&self.raw_json, "\"dist-tags\"", "\"latest\"")
        } else if self.raw_json.contains(&format!("\"{}\":{{", requested))
            || self.raw_json.contains(&format!("\"{}\": {{", requested))
        {
            Some(String::from(requested))
        } else {
            None
        }
    }

    pub fn tarball_url(&self, version: &str) -> Option<String> {
        let version_key = format!("\"{}\"", version);
        let version_pos = self.raw_json.find(&version_key)?;
        let tail = &self.raw_json[version_pos..];
        json_string_field(tail, "\"tarball\"")
    }

    pub fn dependencies(&self, version: &str) -> Vec<PackageSpec> {
        let version_key = format!("\"{}\"", version);
        let Some(version_pos) = self.raw_json.find(&version_key) else {
            return Vec::new();
        };
        let tail = &self.raw_json[version_pos..];
        let Some(dep_pos) = tail.find("\"dependencies\"") else {
            return Vec::new();
        };
        let deps = &tail[dep_pos..];
        let Some(open) = deps.find('{') else {
            return Vec::new();
        };
        let Some(close) = deps[open + 1..].find('}') else {
            return Vec::new();
        };
        parse_dependency_object(&deps[open + 1..open + 1 + close])
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
    let outer_pos = source.find(outer_key)?;
    json_string_field(&source[outer_pos..], inner_key)
}

fn json_string_field(source: &str, key: &str) -> Option<String> {
    let key_pos = source.find(key)?;
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
    fn scoped_registry_url_is_encoded_like_npm() {
        let client = RegistryClient::new(RegistryConfig::default());
        assert_eq!(
            client.package_metadata_url("@scope/pkg"),
            "https://registry.npmjs.org/@scope%2fpkg"
        );
    }
}
