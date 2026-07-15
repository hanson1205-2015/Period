use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Contents of `period.toml`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct PeriodToml {
    pub package: Package,
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Package {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub license: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum DependencySpec {
    Version(String),
    Detailed(DependencyDetail),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct DependencyDetail {
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub git: Option<String>,
}

impl DependencySpec {
    pub fn version(&self) -> Option<&str> {
        match self {
            DependencySpec::Version(v) => Some(v),
            DependencySpec::Detailed(d) => d.version.as_deref(),
        }
    }

    pub fn git_url(&self) -> Option<&str> {
        match self {
            DependencySpec::Version(_) => None,
            DependencySpec::Detailed(d) => d.git.as_deref(),
        }
    }
}

impl PeriodToml {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        toml::from_str(&text).map_err(|e| format!("invalid period.toml: {}", e))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self)
            .map_err(|e| format!("cannot serialize period.toml: {}", e))?;
        fs::write(path, text).map_err(|e| format!("cannot write {}: {}", path.display(), e))
    }
}

pub fn default_manifest(name: &str) -> PeriodToml {
    PeriodToml {
        package: Package {
            name: name.to_string(),
            version: "1.0.0".to_string(),
            authors: Vec::new(),
            license: None,
        },
        dependencies: BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_manifest() {
        let text = r#"
[package]
name = "demo"
version = "0.1.0"

[dependencies]
foo = "1.0.0"
"#;
        let manifest: PeriodToml = toml::from_str(text).expect("manifest should parse");
        assert_eq!(manifest.package.name, "demo");
        assert_eq!(manifest.package.version, "0.1.0");
        assert_eq!(manifest.dependencies.get("foo").expect("dependency foo should exist").version(), Some("1.0.0"));
    }

    #[test]
    fn parse_git_dependency() {
        let text = r#"
[package]
name = "demo"
version = "0.1.0"

[dependencies]
bar = { git = "https://github.com/user/bar", version = "2.1.0" }
"#;
        let manifest: PeriodToml = toml::from_str(text).expect("manifest should parse");
        let dep = manifest.dependencies.get("bar").expect("dependency bar should exist");
        assert_eq!(dep.version(), Some("2.1.0"));
        assert_eq!(dep.git_url(), Some("https://github.com/user/bar"));
    }
}
