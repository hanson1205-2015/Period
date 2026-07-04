use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::package_manager::downloader::sha256_hex;
use crate::package_manager::manifest::PeriodToml;
use crate::package_manager::registry::RegistryIndex;

/// Options for publishing a package.
pub struct PublishOptions<'a> {
    pub file: &'a Path,
    pub name: Option<&'a str>,
    pub version: Option<&'a str>,
    pub registry_dir: Option<&'a Path>,
    pub base_url: Option<&'a str>,
    pub push: bool,
    pub remote: Option<&'a str>,
    pub message: Option<&'a str>,
}

/// Publish a `.period` file to a local registry directory or upload it to a registry server.
///
/// If `version` is `None`, tries to read it from `period.toml` in the current
/// directory, falling back to `"1.0.0"`.
///
/// If `registry_dir` is `None`, defaults to a `registry/` directory next to
/// the current project's `period.toml`, or the current working directory.
pub fn publish(options: PublishOptions<'_>) -> Result<(), String> {
    let package_name = determine_package_name(options.file, options.name)?;
    let package_version = determine_version(options.version)?;

    let registry = options.registry_dir
        .map(PathBuf::from)
        .unwrap_or_else(default_registry_dir);

    let packages_dir = registry.join("packages");
    let index_dir = registry.join("index");
    fs::create_dir_all(&packages_dir)
        .map_err(|e| format!("cannot create {}: {}", packages_dir.display(), e))?;
    fs::create_dir_all(&index_dir)
        .map_err(|e| format!("cannot create {}: {}", index_dir.display(), e))?;

    let dest_filename = format!("{}-{}.period", package_name, package_version);
    let dest_path = packages_dir.join(&dest_filename);
    if dest_path.exists() {
        return Err(format!(
            "package '{} {}' already exists at {}",
            package_name,
            package_version,
            dest_path.display()
        ));
    }

    fs::copy(options.file, &dest_path)
        .map_err(|e| format!("cannot copy to {}: {}", dest_path.display(), e))?;

    let bytes = fs::read(&dest_path)
        .map_err(|e| format!("cannot read '{}': {}", dest_path.display(), e))?;
    let checksum = format!("sha256:{}", sha256_hex(&bytes));

    let registry_url = options.base_url
        .map(|u| format!("{}/packages/{}-{}.period", u.trim_end_matches('/'), package_name, package_version))
        .unwrap_or_else(|| default_registry_url_for(&package_name, &package_version));
    let index_path = index_dir.join(format!("{}.json", package_name));
    let mut index = load_or_create_index(&index_path, &package_name)?;

    index.versions.insert(
        package_version.clone(),
        crate::package_manager::registry::RegistryVersion {
            url: registry_url,
            checksum: Some(checksum),
            dependencies: BTreeMap::new(),
        },
    );

    let index_json = serde_json::to_string_pretty(&index)
        .map_err(|e| format!("cannot serialize index: {}", e))?;
    fs::write(&index_path, index_json)
        .map_err(|e| format!("cannot write {}: {}", index_path.display(), e))?;

    println!(
        "Published {} {} to registry\n  package: {}\n  index: {}",
        package_name,
        package_version,
        dest_path.display(),
        index_path.display()
    );

    if options.push {
        let remote_name = options.remote.unwrap_or("origin");
        git_push(&registry, remote_name, &package_name, &package_version, options.message)?;
        println!("Pushed registry changes to remote '{}'.", remote_name);
    } else {
        println!("\nNext steps:");
        println!("  1. Review the generated files.");
        println!("  2. git add registry/ && git commit -m 'publish {} {}'", package_name, package_version);
        println!("  3. git push origin main");
    }

    Ok(())
}

fn git_push(
    registry: &Path,
    remote: &str,
    name: &str,
    version: &str,
    message: Option<&str>,
) -> Result<(), String> {
    let default_msg = format!("publish {} {}", name, version);
    let msg = message.unwrap_or(&default_msg);

    // Find the repository root that contains the registry directory.
    let repo_root = find_git_root(registry)?;

    run_git(&repo_root, &["add", registry.to_string_lossy().as_ref()])?;
    run_git(&repo_root, &["commit", "-m", msg])?;
    run_git(&repo_root, &["push", remote])?;

    Ok(())
}

fn find_git_root(dir: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| format!("failed to run git: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "registry directory is not inside a git repository: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout)
        .map(|s| PathBuf::from(s.trim()))
        .map_err(|e| format!("invalid git output: {}", e))
}

fn run_git(repo: &Path, args: &[&str]) -> Result<(), String> {
    let output = match Command::new("git").current_dir(repo).args(args).output() {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(
                "git is not installed or not in PATH. \
                 Install Git from https://git-scm.com or use --server to upload to a registry server instead of --push."
                    .to_string(),
            );
        }
        Err(e) => return Err(format!("failed to run git {}: {}", args.join(" "), e)),
    };
    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
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

fn default_registry_dir() -> PathBuf {
    let manifest_path = PathBuf::from("period.toml");
    if let Some(parent) = manifest_path.parent() {
        return parent.join("registry");
    }
    PathBuf::from("registry")
}

fn default_registry_url_for(name: &str, version: &str) -> String {
    format!(
        "https://raw.githubusercontent.com/ExploreMaths/Period/main/registry/packages/{}-{}.period",
        name, version
    )
}

fn load_or_create_index(path: &Path, name: &str) -> Result<RegistryIndex, String> {
    if path.exists() {
        let text = fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
        serde_json::from_str(&text)
            .map_err(|e| format!("invalid index {}: {}", path.display(), e))
    } else {
        Ok(RegistryIndex {
            name: name.to_string(),
            versions: BTreeMap::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    #[test]
    fn publish_creates_index_and_package() {
        let tmp = env::temp_dir().join(format!("period-publish-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let src = tmp.join("greet.period");
        fs::write(&src, "export hi.\ndefine hi with x:\n    return x.\n").unwrap();

        let reg = tmp.join("registry");
        publish(PublishOptions {
            file: &src,
            name: None,
            version: Some("1.2.3"),
            registry_dir: Some(&reg),
            base_url: None,
            push: false,
            remote: None,
            message: None,
        }).unwrap();

        let pkg = reg.join("packages").join("greet-1.2.3.period");
        let idx = reg.join("index").join("greet.json");
        assert!(pkg.exists());
        assert!(idx.exists());

        let index: RegistryIndex = serde_json::from_str(&fs::read_to_string(&idx).unwrap()).unwrap();
        assert_eq!(index.name, "greet");
        assert!(index.versions.contains_key("1.2.3"));

        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn publish_rejects_duplicate_version() {
        let tmp = env::temp_dir().join(format!("period-publish-dup-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let src = tmp.join("greet.period");
        fs::write(&src, "export hi.").unwrap();

        let reg = tmp.join("registry");
        publish(PublishOptions {
            file: &src,
            name: None,
            version: Some("1.0.0"),
            registry_dir: Some(&reg),
            base_url: None,
            push: false,
            remote: None,
            message: None,
        }).unwrap();
        let result = publish(PublishOptions {
            file: &src,
            name: None,
            version: Some("1.0.0"),
            registry_dir: Some(&reg),
            base_url: None,
            push: false,
            remote: None,
            message: None,
        });
        assert!(result.is_err());

        fs::remove_dir_all(&tmp).unwrap();
    }
}
