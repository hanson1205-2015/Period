use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Contents of `period.lock`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Default)]
pub struct PeriodLock {
    #[serde(default, rename = "package")]
    pub packages: Vec<LockedPackage>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub source: String,
    pub checksum: String,
}

impl PeriodLock {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        toml::from_str(&text).map_err(|e| format!("invalid period.lock: {}", e))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = toml::to_string_pretty(self)
            .map_err(|e| format!("cannot serialize period.lock: {}", e))?;
        fs::write(path, text).map_err(|e| format!("cannot write {}: {}", path.display(), e))
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_lockfile() {
        let lock = PeriodLock {
            packages: vec![LockedPackage {
                name: "foo".to_string(),
                version: "1.0.0".to_string(),
                source: "registry+https://example.com/foo.period".to_string(),
                checksum: "sha256:abcd".to_string(),
            }],
        };
        let text = toml::to_string_pretty(&lock).unwrap();
        let parsed: PeriodLock = toml::from_str(&text).unwrap();
        assert_eq!(lock, parsed);
    }
}
