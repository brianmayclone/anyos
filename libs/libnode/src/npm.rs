use alloc::format;
use alloc::string::String;

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
        Self { config }
    }

    pub fn package_metadata_url(&self, package_name: &str) -> String {
        self.config.package_url(package_name)
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
