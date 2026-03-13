use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::config::config_dir;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub path: String,
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProjectRegistry {
    pub projects: Vec<ProjectEntry>,
}

// ---------------------------------------------------------------------------
// Persistence
// ---------------------------------------------------------------------------

pub fn registry_path() -> PathBuf {
    config_dir().join("projects.json")
}

pub fn load() -> ProjectRegistry {
    let path = registry_path();
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => ProjectRegistry::default(),
    }
}

pub fn save(registry: &ProjectRegistry) -> anyhow::Result<()> {
    let path = registry_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(registry)?;
    std::fs::write(&path, json)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Register (upsert) a project in the registry by its root path.
pub fn register_project(root: &Path) -> anyhow::Result<()> {
    let canonical = root
        .canonicalize()
        .unwrap_or_else(|_| root.to_path_buf())
        .to_string_lossy()
        .to_string();

    let now = chrono_now();

    let mut registry = load();

    if let Some(entry) = registry.projects.iter_mut().find(|e| e.path == canonical) {
        entry.last_seen = now;
    } else {
        registry.projects.push(ProjectEntry {
            path: canonical,
            last_seen: now,
        });
    }

    save(&registry)
}

/// Remove projects whose config file no longer exists on disk.
/// Returns the list of pruned paths.
pub fn prune_stale(registry: &mut ProjectRegistry) -> Vec<String> {
    let mut pruned = Vec::new();
    registry.projects.retain(|entry| {
        let root = Path::new(&entry.path);
        let has_config = root.join(".armadai").join("config.yaml").is_file()
            || root.join("armadai.yaml").is_file()
            || root.join("armadai.yml").is_file();
        if !has_config {
            pruned.push(entry.path.clone());
        }
        has_config
    });
    pruned
}

/// ISO 8601 timestamp without external chrono dependency.
fn chrono_now() -> String {
    // Use std::time for a simple UTC-ish timestamp
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();

    // Convert to date/time components
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Days since epoch to year/month/day (simplified Gregorian)
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    days += 719468;
    let era = days / 146097;
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: save a registry to a specific file path and load it back.
    fn save_to(path: &Path, registry: &ProjectRegistry) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let json = serde_json::to_string_pretty(registry).unwrap();
        std::fs::write(path, json).unwrap();
    }

    fn load_from(path: &Path) -> ProjectRegistry {
        match std::fs::read_to_string(path) {
            Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
            Err(_) => ProjectRegistry::default(),
        }
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("projects.json");

        let registry = ProjectRegistry {
            projects: vec![ProjectEntry {
                path: "/home/user/my-project".to_string(),
                last_seen: "2025-06-01T12:00:00Z".to_string(),
            }],
        };

        save_to(&path, &registry);
        let loaded = load_from(&path);
        assert_eq!(loaded.projects.len(), 1);
        assert_eq!(loaded.projects[0].path, "/home/user/my-project");
    }

    #[test]
    fn test_upsert_logic() {
        // Test the upsert logic directly without env var dependency
        let mut registry = ProjectRegistry {
            projects: vec![ProjectEntry {
                path: "/home/user/project".to_string(),
                last_seen: "2025-01-01T00:00:00Z".to_string(),
            }],
        };

        // Upsert same path: should update timestamp
        let canonical = "/home/user/project".to_string();
        let new_ts = "2025-06-01T12:00:00Z".to_string();
        if let Some(entry) = registry.projects.iter_mut().find(|e| e.path == canonical) {
            entry.last_seen = new_ts.clone();
        }

        assert_eq!(registry.projects.len(), 1);
        assert_eq!(registry.projects[0].last_seen, "2025-06-01T12:00:00Z");

        // Upsert new path: should add
        let new_path = "/home/user/other-project".to_string();
        if registry
            .projects
            .iter_mut()
            .find(|e| e.path == new_path)
            .is_none()
        {
            registry.projects.push(ProjectEntry {
                path: new_path,
                last_seen: new_ts,
            });
        }
        assert_eq!(registry.projects.len(), 2);
    }

    #[test]
    fn test_prune_stale() {
        let mut registry = ProjectRegistry {
            projects: vec![
                ProjectEntry {
                    path: "/nonexistent/path1".to_string(),
                    last_seen: "2025-01-01T00:00:00Z".to_string(),
                },
                ProjectEntry {
                    path: "/nonexistent/path2".to_string(),
                    last_seen: "2025-01-02T00:00:00Z".to_string(),
                },
            ],
        };

        let pruned = prune_stale(&mut registry);
        assert_eq!(pruned.len(), 2);
        assert!(registry.projects.is_empty());
    }

    #[test]
    fn test_prune_keeps_valid() {
        let dir = tempfile::tempdir().unwrap();
        let project_dir = dir.path().join("valid-project");
        std::fs::create_dir_all(project_dir.join(".armadai")).unwrap();
        std::fs::write(
            project_dir.join(".armadai").join("config.yaml"),
            "agents: []\n",
        )
        .unwrap();

        let mut registry = ProjectRegistry {
            projects: vec![
                ProjectEntry {
                    path: project_dir.to_string_lossy().to_string(),
                    last_seen: "2025-01-01T00:00:00Z".to_string(),
                },
                ProjectEntry {
                    path: "/nonexistent/stale".to_string(),
                    last_seen: "2025-01-01T00:00:00Z".to_string(),
                },
            ],
        };

        let pruned = prune_stale(&mut registry);
        assert_eq!(pruned.len(), 1);
        assert_eq!(pruned[0], "/nonexistent/stale");
        assert_eq!(registry.projects.len(), 1);
    }

    #[test]
    fn test_empty_load_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.json");
        let registry = load_from(&path);
        assert!(registry.projects.is_empty());
    }

    #[test]
    fn test_chrono_now_format() {
        let ts = chrono_now();
        // Should match YYYY-MM-DDTHH:MM:SSZ
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }

    #[test]
    fn test_default_registry_empty() {
        let registry = ProjectRegistry::default();
        assert!(registry.projects.is_empty());
    }
}
