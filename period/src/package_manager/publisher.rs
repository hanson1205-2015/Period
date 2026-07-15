use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::package_manager::downloader::sha256_hex;
use crate::package_manager::manifest::PeriodToml;
use crate::package_manager::registry::{RegistryIndex, RegistryVersion};

/// Options for publishing a package.
pub struct PublishOptions<'a> {
    pub file: &'a Path,
    pub name: Option<&'a str>,
    pub version: Option<&'a str>,
    pub registry_file: Option<&'a Path>,
    pub base_url: Option<&'a str>,
}

/// Publish a `.period` file and produce a registry entry.
///
/// If `version` is `None`, tries to read it from `period.toml` in the current
/// directory, falling back to `"1.0.0"`.
///
/// The file is read, its SHA256 checksum is computed, and a registry entry is
/// built. If `registry_file` is provided, the entry is merged into that file
/// (creating it if necessary). Otherwise the entry JSON is printed to stdout.
pub fn publish(options: PublishOptions<'_>) -> Result<(), String> {
    let package_name = determine_package_name(options.file, options.name)?;
    let package_version = determine_version(options.version)?;

    let bytes = fs::read(options.file)
        .map_err(|e| format!("cannot read '{}': {}", options.file.display(), e))?;
    let checksum = format!("sha256:{}", sha256_hex(&bytes));

    let registry_url = options
        .base_url
        .map(|u| format!("{}/{}-{}.period", u.trim_end_matches('/'), package_name, package_version))
        .unwrap_or_else(|| default_registry_url_for(&package_name, &package_version));

    let entry = RegistryVersion {
        url: registry_url,
        checksum: Some(checksum),
        dependencies: BTreeMap::new(),
    };

    if let Some(registry_file) = options.registry_file {
        let mut index = load_or_create_registry(registry_file)?;
        let package_entry = index
            .packages
            .entry(package_name.clone())
            .or_default();
        if package_entry.contains_key(&package_version) {
            return Err(format!(
                "package '{} {}' already exists in registry",
                package_name, package_version
            ));
        }
        package_entry.insert(package_version.clone(), entry);

        let index_json = serde_json::to_string_pretty(&index)
            .map_err(|e| format!("cannot serialize registry: {}", e))?;
        fs::write(registry_file, index_json)
            .map_err(|e| format!("cannot write {}: {}", registry_file.display(), e))?;

        println!(
            "Updated {} with {} {}",
            registry_file.display(),
            package_name,
            package_version
        );
    } else {
        let snippet = serde_json::to_string_pretty(&BTreeMap::from([(
            package_version.clone(),
            entry,
        )]))
        .map_err(|e| format!("cannot serialize entry: {}", e))?;
        println!(
            "Add the following entry for package '{}' to your registry.json:\n{}",
            package_name, snippet
        );
    }

    Ok(())
}

fn determine_package_name(file: &Path, override_name: Option<&str>) -> Result<String, String> {
    if let Some(name) = override_name {
        return Ok(name.to_string());
    }

    // Try period.toml in the file's directory or current directory.
    let candidates = [
        file.parent().map(|p| p.join("period.toml")),
        Some(PathBuf::from("period.toml")),
    ];
    for candidate in candidates.into_iter().flatten() {
        if candidate.exists()
            && let Ok(manifest) = PeriodToml::load(&candidate)
        {
            return Ok(manifest.package.name);
        }
    }

    // Fall back to the file stem.
    file.file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .ok_or_else(|| format!("cannot determine package name from '{}'", file.display()))
}

fn determine_version(override_version: Option<&str>) -> Result<String, String> {
    if let Some(v) = override_version {
        return Ok(v.to_string());
    }

    let manifest_path = PathBuf::from("period.toml");
    if manifest_path.exists()
        && let Ok(manifest) = PeriodToml::load(&manifest_path)
    {
        return Ok(manifest.package.version);
    }

    Ok("1.0.0".to_string())
}

fn default_registry_url_for(name: &str, version: &str) -> String {
    format!(
        "https://github.com/period-lang/registry/releases/download/{}-{}/{}-{}.period",
        name, version, name, version
    )
}

fn load_or_create_registry(path: &Path) -> Result<RegistryIndex, String> {
    if path.exists() {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        serde_json::from_str(&text)
            .map_err(|e| format!("invalid registry {}: {}", path.display(), e))
    } else {
        Ok(RegistryIndex {
            schema_version: "1".to_string(),
            packages: BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn publish_prints_entry_without_registry_file() {
        let tmp = env::temp_dir().join(format!("period-publish-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).expect("should create temp dir");
        let src = tmp.join("greet.period");
        fs::write(&src,
            "export hi.\ndefine hi with x:\n    return x.\n",
        )
        .expect("should write source file");

        publish(PublishOptions {
            file: &src,
            name: None,
            version: Some("1.2.3"),
            registry_file: None,
            base_url: None,
        })
        .expect("publish should succeed");

        fs::remove_dir_all(&tmp).expect("should remove temp dir");
    }

    #[test]
    fn publish_creates_registry_file() {
        let tmp = env::temp_dir().join(format!("period-publish-reg-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).expect("should create temp dir");
        let src = tmp.join("greet.period");
        fs::write(&src, "export hi.").expect("should write source file");
        let reg = tmp.join("registry.json");

        publish(PublishOptions {
            file: &src,
            name: None,
            version: Some("1.2.3"),
            registry_file: Some(&reg),
            base_url: None,
        })
        .expect("publish should succeed");

        assert!(reg.exists());
        let index: RegistryIndex = serde_json::from_str(&fs::read_to_string(&reg).expect("registry file should read")).expect("registry should parse as JSON");
        assert_eq!(index.schema_version, "1");
        let versions = index.packages.get("greet").expect("greet package");
        assert!(versions.contains_key("1.2.3"));
        assert!(versions
            .get("1.2.3")
            .expect("1.2.3 version")
            .checksum
            .as_ref()
            .expect("1.2.3 checksum")
            .starts_with("sha256:"));

        fs::remove_dir_all(&tmp).expect("should remove temp dir");
    }

    #[test]
    fn publish_rejects_duplicate_version() {
        let tmp = env::temp_dir().join(format!("period-publish-dup-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).expect("should create temp dir");
        let src = tmp.join("greet.period");
        fs::write(&src, "export hi.").expect("should write source file");
        let reg = tmp.join("registry.json");

        publish(PublishOptions {
            file: &src,
            name: None,
            version: Some("1.0.0"),
            registry_file: Some(&reg),
            base_url: None,
        })
        .expect("first publish should succeed");
        let result = publish(PublishOptions {
            file: &src,
            name: None,
            version: Some("1.0.0"),
            registry_file: Some(&reg),
            base_url: None,
        });
        assert!(result.is_err());

        fs::remove_dir_all(&tmp).expect("should remove temp dir");
    }
}
