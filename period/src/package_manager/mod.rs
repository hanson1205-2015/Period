pub mod downloader;
pub mod lockfile;
pub mod manifest;
pub mod publisher;
pub mod registry;
pub mod resolver;

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::package_manager::downloader::{download, sha256_hex, verify_checksum};
use crate::package_manager::lockfile::{LockedPackage, PeriodLock};
use crate::package_manager::manifest::{default_manifest, DependencySpec, PeriodToml};
pub use crate::package_manager::publisher::{publish, PublishOptions};
use crate::package_manager::registry::default_registry;
use crate::package_manager::resolver::Resolver;

pub const MANIFEST_FILE: &str = "period.toml";
pub const LOCKFILE_FILE: &str = "period.lock";
pub const PACKAGES_DIR: &str = "period_packages";

/// Initialise a new Period project in the given directory.
pub fn init_project_at(dir: &Path, name: Option<&str>) -> Result<(), String> {
    let project_name = name
        .map(|s| s.to_string())
        .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "project".to_string()));

    let manifest_path = dir.join(MANIFEST_FILE);
    if manifest_path.exists() {
        return Err(format!("{} already exists", MANIFEST_FILE));
    }

    let manifest = default_manifest(&project_name);
    manifest.save(&manifest_path)?;
    println!("Created {} for project '{}'", MANIFEST_FILE, project_name);
    Ok(())
}

/// Initialise a new Period project in the current working directory.
pub fn init_project(name: Option<&str>) -> Result<(), String> {
    let cwd = env::current_dir().map_err(|e| format!("cannot get current directory: {}", e))?;
    init_project_at(&cwd, name)
}

/// Install dependencies declared in `period.toml`.
pub fn install() -> Result<(), String> {
    let manifest = load_manifest()?;
    resolve_and_download(&manifest, false)
}

/// Install a single package and add it to `period.toml`.
///
/// If `name_or_url` is a URL (http/https/file), the file is downloaded directly
/// without changing the manifest. Otherwise it is treated as a package name or
/// `name@version` spec, added to `[dependencies]`, and resolved.
pub fn install_package(name_or_url: &str) -> Result<(), String> {
    if is_url(name_or_url) {
        // Direct URL install: keep legacy behaviour.
        let filename = name_or_url.rsplit('/').next().unwrap_or("package.period");
        let filename = if filename.is_empty() { "package.period" } else { filename };
        let dest = PathBuf::from(PACKAGES_DIR).join(filename);
        download_url(name_or_url, &dest)?;
        println!("Installed {} -> {}", name_or_url, dest.display());
        return Ok(());
    }

    // Local file path: copy directly without touching the manifest.
    let as_path = PathBuf::from(name_or_url);
    if as_path.is_file() {
        let filename = as_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "package.period".to_string());
        fs::create_dir_all(PACKAGES_DIR)
            .map_err(|e| format!("cannot create {}: {}", PACKAGES_DIR, e))?;
        let dest = PathBuf::from(PACKAGES_DIR).join(&filename);
        fs::copy(&as_path, &dest)
            .map_err(|e| format!("cannot copy {} to {}: {}", as_path.display(), dest.display(), e))?;
        println!("Installed {} -> {}", as_path.display(), dest.display());
        return Ok(());
    }

    let mut manifest = load_manifest()?;
    let (name, version) = parse_package_spec(name_or_url);
    manifest.dependencies.insert(name.clone(), DependencySpec::Version(version.to_string()));
    manifest.save(&PathBuf::from(MANIFEST_FILE))?;
    println!("Added {} = \"{}\" to {}", name, version, MANIFEST_FILE);

    resolve_and_download(&manifest, false)
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://") || s.starts_with("file://")
}

fn download_url(url: &str, dest: &Path) -> Result<(), String> {
    if let Some(mut path) = url.strip_prefix("file://") {
        // file:///d:/path -> d:/path; file://server/share -> //server/share
        if path.len() >= 2 && path.as_bytes()[0] == b'/' && path.as_bytes()[2] == b':' {
            path = &path[1..];
        }
        let path = path.replace('/', "\\");
        let bytes = fs::read(&path)
            .map_err(|e| format!("cannot read local file '{}': {}", path, e))?;
        fs::create_dir_all(dest.parent().unwrap_or(dest))
            .map_err(|e| format!("cannot create directory: {}", e))?;
        fs::write(dest, bytes)
            .map_err(|e| format!("cannot write {}: {}", dest.display(), e))?;
        Ok(())
    } else {
        download(url, dest)?;
        Ok(())
    }
}

/// Re-resolve all dependencies and rewrite `period.lock`.
pub fn update() -> Result<(), String> {
    let manifest = load_manifest()?;
    resolve_and_download(&manifest, true)
}

fn load_manifest() -> Result<PeriodToml, String> {
    PeriodToml::load(&PathBuf::from(MANIFEST_FILE))
        .map_err(|e| format!("{} not found or invalid: {}", MANIFEST_FILE, e))
}

fn parse_package_spec(spec: &str) -> (String, String) {
    if let Some((name, version)) = spec.split_once('@') {
        (name.to_string(), version.to_string())
    } else {
        (spec.to_string(), "*".to_string())
    }
}

fn resolve_and_download(manifest: &PeriodToml, _force: bool) -> Result<(), String> {
    let registry = default_registry();
    let mut resolver = Resolver::new(&registry);
    let packages = resolver.resolve(manifest)?;

    fs::create_dir_all(PACKAGES_DIR)
        .map_err(|e| format!("cannot create {}: {}", PACKAGES_DIR, e))?;

    let mut lock = PeriodLock::default();
    for pkg in &packages {
        let url = pkg.source.strip_prefix("registry+").or_else(|| pkg.source.strip_prefix("git+")).unwrap_or(&pkg.source);
        let bytes = download(url, &pkg.file_path)?;
        if let Some(expected) = &pkg.checksum {
            verify_checksum(&bytes, expected)?;
        }
        let checksum = format!("sha256:{}", sha256_hex(&bytes));
        lock.packages.push(LockedPackage {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            source: pkg.source.clone(),
            checksum,
        });
    }

    lock.save(&PathBuf::from(LOCKFILE_FILE))?;
    println!("Wrote {} with {} package(s)", LOCKFILE_FILE, lock.packages.len());
    Ok(())
}

/// Load the lockfile from the given project root, if present.
pub fn load_lockfile_from(root: &Path) -> Option<PeriodLock> {
    PeriodLock::load(&root.join(LOCKFILE_FILE)).ok()
}

/// Return the file path for an installed package by name, relative to a project root.
///
/// If a `period.lock` exists in `root`, only packages listed there are considered
/// installed. Otherwise fall back to a loose file in `root/period_packages/` for
/// backwards compatibility with direct URL installs.
pub fn package_path_in(name: &str, root: &Path) -> Option<PathBuf> {
    if let Some(lock) = load_lockfile_from(root) {
        return lock.packages.iter().find(|p| p.name == name).map(|_| {
            PathBuf::from(PACKAGES_DIR).join(format!("{}.period", name))
        });
    }
    let loose = PathBuf::from(PACKAGES_DIR).join(format!("{}.period", name));
    if root.join(&loose).is_file() { Some(loose) } else { None }
}

/// Return the file path for an installed package by name, using the current directory
/// as the project root.
pub fn package_path(name: &str) -> Option<PathBuf> {
    let root = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    package_path_in(name, &root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_creates_manifest() {
        let tmp = std::env::temp_dir().join(format!("period-init-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        init_project_at(&tmp, Some("demo")).unwrap();
        let manifest = PeriodToml::load(&tmp.join(MANIFEST_FILE)).unwrap();
        assert_eq!(manifest.package.name, "demo");
        assert_eq!(manifest.package.version, "1.0.0");
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn package_path_reads_lockfile() {
        let tmp = std::env::temp_dir().join(format!("period-lock-test-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let lock = PeriodLock {
            packages: vec![LockedPackage {
                name: "foo".to_string(),
                version: "1.0.0".to_string(),
                source: "registry+https://example.com/foo.period".to_string(),
                checksum: "sha256:abcd".to_string(),
            }],
        };
        lock.save(&tmp.join(LOCKFILE_FILE)).unwrap();

        let path = package_path_in("foo", &tmp);

        assert_eq!(path, Some(PathBuf::from("period_packages/foo.period")));
        fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn package_path_falls_back_to_loose_file() {
        let tmp = std::env::temp_dir().join(format!("period-loose-test-{}", std::process::id()));
        fs::create_dir_all(&tmp.join(PACKAGES_DIR)).unwrap();
        fs::write(&tmp.join(PACKAGES_DIR).join("bar.period"), "export x.").unwrap();

        let path = package_path_in("bar", &tmp);

        assert_eq!(path, Some(PathBuf::from("period_packages/bar.period")));
        fs::remove_dir_all(&tmp).unwrap();
    }
}
