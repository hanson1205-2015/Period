use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Default registry URL.
pub fn default_registry() -> String {
    std::env::var("PERIOD_REGISTRY")
        .unwrap_or_else(|_| "https://period-lang.github.io/registry".to_string())
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RegistryIndex {
    pub schema_version: String,
    #[serde(default)]
    pub packages: BTreeMap<String, BTreeMap<String, RegistryVersion>>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct RegistryVersion {
    pub url: String,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

impl RegistryIndex {
    pub fn fetch(registry: &str) -> Result<Self, String> {
        let url = format!("{}/registry.json", registry.trim_end_matches('/'));
        let text = ureq::get(&url)
            .call()
            .map_err(|e| format!("failed to fetch registry from '{}': {}", url, e))?
            .into_string()
            .map_err(|e| format!("failed to read registry from '{}': {}", url, e))?;
        serde_json::from_str(&text)
            .map_err(|e| format!("invalid registry at '{}': {}", url, e))
    }
}

/// Select the latest version from `available` that satisfies `constraint`.
/// For now supports exact matches (`=1.2.3`) and caret (`^1.2.3`) / wildcard
/// patterns, plus plain version strings treated as "at least this version".
pub fn select_version(constraint: &str, available: &BTreeMap<String, RegistryVersion>) -> Result<String, String> {
    let constraint = constraint.trim();

    // Wildcard: accept any version.
    if constraint == "*" || constraint == "x" || constraint == "X" {
        let mut best: Option<(Version, String)> = None;
        for version in available.keys() {
            let parts = parse_version(version)?;
            if best.as_ref().is_none_or(|(b, _)| is_greater(&parts, b)) {
                best = Some((parts, version.clone()));
            }
        }
        return best
            .map(|(_, v)| v)
            .ok_or_else(|| "no versions available".to_string());
    }

    // Exact constraint.
    if let Some(version) = constraint.strip_prefix('=') {
        let version = version.trim();
        if available.contains_key(version) {
            return Ok(version.to_string());
        }
        return Err(format!("no version {} found", version));
    }

    // Strip optional leading ^ or ~; for the initial implementation treat them
    // as "latest compatible" which simply means the highest available version.
    let cleaned = constraint
        .strip_prefix('^')
        .or_else(|| constraint.strip_prefix('~'))
        .unwrap_or(constraint)
        .trim();

    // Parse major.minor.patch for cleaned constraint.
    let constraint_parts = parse_version(cleaned)?;

    let mut best: Option<(Version, String)> = None;
    for version in available.keys() {
        let parts = parse_version(version)?;
        if !is_compatible(cleaned, &constraint_parts, version, &parts) {
            continue;
        }
        if best.as_ref().is_none_or(|(b, _)| is_greater(&parts, b)) {
            best = Some((parts, version.clone()));
        }
    }

    best.map(|(_, v)| v)
        .ok_or_else(|| format!("no version matching '{}' found", constraint))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Version(Vec<u64>);

fn parse_version(s: &str) -> Result<Version, String> {
    let mut parts = Vec::new();
    for chunk in s.split('.') {
        let num: u64 = chunk
            .parse()
            .map_err(|_| format!("invalid version segment '{}' in '{}'", chunk, s))?;
        parts.push(num);
    }
    if parts.is_empty() {
        return Err(format!("empty version '{}'", s));
    }
    Ok(Version(parts))
}

fn is_greater(a: &Version, b: &Version) -> bool {
    let max_len = a.0.len().max(b.0.len());
    for i in 0..max_len {
        let av = a.0.get(i).copied().unwrap_or(0);
        let bv = b.0.get(i).copied().unwrap_or(0);
        if av != bv {
            return av > bv;
        }
    }
    false
}

fn is_compatible(
    _constraint_str: &str,
    constraint: &Version,
    _version_str: &str,
    version: &Version,
) -> bool {
    let max_len = constraint.0.len().max(version.0.len());
    for i in 0..max_len {
        let cv = constraint.0.get(i).copied().unwrap_or(0);
        let vv = version.0.get(i).copied().unwrap_or(0);
        if vv < cv {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_latest_version() {
        let mut versions = BTreeMap::new();
        versions.insert("1.0.0".to_string(), RegistryVersion {
            url: "a".to_string(),
            checksum: None,
            dependencies: BTreeMap::new(),
        });
        versions.insert("1.2.0".to_string(), RegistryVersion {
            url: "b".to_string(),
            checksum: None,
            dependencies: BTreeMap::new(),
        });
        versions.insert("2.0.0".to_string(), RegistryVersion {
            url: "c".to_string(),
            checksum: None,
            dependencies: BTreeMap::new(),
        });
        assert_eq!(select_version("1.0.0", &versions).expect("should select latest version"), "2.0.0");
        assert_eq!(select_version("=1.2.0", &versions).expect("should select exact version"), "1.2.0");
        assert_eq!(select_version("^1.0.0", &versions).expect("should select caret version"), "2.0.0");
        assert_eq!(select_version("*", &versions).expect("should select wildcard version"), "2.0.0");
    }

    #[test]
    fn select_wildcard_with_single_version() {
        let mut versions = BTreeMap::new();
        versions.insert("0.5.1".to_string(), RegistryVersion {
            url: "a".to_string(),
            checksum: None,
            dependencies: BTreeMap::new(),
        });
        assert_eq!(select_version("*", &versions).expect("should select wildcard version"), "0.5.1");
    }

    #[test]
    fn deserialize_registry_index() {
        let json = r#"{
            "schema_version": "1",
            "packages": {
                "list": {
                    "1.0.0": {
                        "url": "https://github.com/period-lang/registry/releases/download/list-1.0.0/list-1.0.0.period",
                        "checksum": "sha256:abcd",
                        "dependencies": {}
                    }
                }
            }
        }"#;
        let index: RegistryIndex = serde_json::from_str(json).expect("registry JSON should deserialize");
        assert_eq!(index.schema_version, "1");
        let list = index.packages.get("list").expect("list package");
        let version = list.get("1.0.0").expect("1.0.0 version");
        assert_eq!(
            version.url,
            "https://github.com/period-lang/registry/releases/download/list-1.0.0/list-1.0.0.period"
        );
        assert_eq!(version.checksum.as_deref(), Some("sha256:abcd"));
    }
}
