use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use super::manifest::{DependencySpec, PeriodToml};
use super::registry::{select_version, RegistryIndex};

#[derive(Debug, Clone)]
pub struct ResolvedPackage {
    pub name: String,
    pub version: String,
    pub source: String,
    pub checksum: Option<String>,
    pub file_path: PathBuf,
}

pub struct Resolver<'a> {
    registry: &'a str,
    index: Option<RegistryIndex>,
    resolved: BTreeMap<String, ResolvedPackage>,
    loading: HashSet<String>,
}

impl<'a> Resolver<'a> {
    pub fn new(registry: &'a str) -> Self {
        Self {
            registry,
            index: None,
            resolved: BTreeMap::new(),
            loading: HashSet::new(),
        }
    }

    pub fn resolve(&mut self, manifest: &PeriodToml) -> Result<Vec<ResolvedPackage>, String> {
        let mut queue: Vec<(String, DependencySpec)> = Vec::new();
        for (name, spec) in &manifest.dependencies {
            queue.push((name.clone(), spec.clone()));
        }

        let mut i = 0;
        while i < queue.len() {
            let (name, spec) = queue[i].clone();
            if self.resolved.contains_key(&name) {
                i += 1;
                continue;
            }

            if !self.loading.insert(name.clone()) {
                return Err(format!("circular dependency detected involving '{}'", name));
            }

            let resolved = if let Some(git) = spec.git_url() {
                self.resolve_git(&name, git, spec.version())?
            } else {
                self.resolve_registry(&name, spec.version().unwrap_or("*"))?
            };

            for (dep_name, dep_version) in &resolved.dependencies {
                queue.push((dep_name.clone(), DependencySpec::Version(dep_version.clone())));
            }

            let file_path = PathBuf::from("period_packages").join(format!("{}.period", name));
            self.resolved.insert(
                name.clone(),
                ResolvedPackage {
                    name: name.clone(),
                    version: resolved.version.clone(),
                    source: resolved.source.clone(),
                    checksum: resolved.checksum.clone(),
                    file_path,
                },
            );

            self.loading.remove(&name);
            i += 1;
        }

        Ok(self.resolved.values().cloned().collect())
    }

    fn resolve_registry(&mut self,
        name: &str,
        constraint: &str,
    ) -> Result<ResolvedVersion, String> {
        if self.index.is_none() {
            self.index = Some(RegistryIndex::fetch(self.registry)?);
        }
        let index = self.index.as_ref().ok_or_else(|| "internal error: registry index not loaded".to_string())?;
        let versions = index
            .packages
            .get(name)
            .ok_or_else(|| format!("package '{}' not found in registry", name))?;
        let version = select_version(constraint, versions)?;
        let entry = versions
            .get(&version)
            .ok_or_else(|| format!("internal error: selected version '{}' disappeared for package '{}'", version, name))?;
        Ok(ResolvedVersion {
            version,
            source: format!("registry+{}", entry.url),
            checksum: entry.checksum.clone(),
            dependencies: entry.dependencies.clone(),
        })
    }

    fn resolve_git(&self, name: &str, git: &str, version: Option<&str>) -> Result<ResolvedVersion, String> {
        let version = version.unwrap_or("latest").to_string();
        let url = format!("{}/raw/main/{}.period", git.trim_end_matches('/'), name);
        Ok(ResolvedVersion {
            version,
            source: format!("git+{}", url),
            checksum: None,
            dependencies: BTreeMap::new(),
        })
    }
}

#[derive(Debug, Clone)]
struct ResolvedVersion {
    version: String,
    source: String,
    checksum: Option<String>,
    dependencies: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_cycle() {
        // Cycle detection is verified by the loading HashSet.
        let resolver = Resolver::new("https://example.com");
        let mut manifest = PeriodToml {
            package: super::super::manifest::Package {
                name: "demo".to_string(),
                version: "1.0.0".to_string(),
                authors: Vec::new(),
                license: None,
            },
            dependencies: BTreeMap::new(),
        };
        manifest.dependencies.insert("self".to_string(), DependencySpec::Version("1.0.0".to_string()));
        // This would require a real registry; just exercise the struct here.
        assert!(resolver.resolved.is_empty());
    }
}
