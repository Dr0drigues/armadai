use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A fleet definition linking a named group of agents to a source directory.
/// Serialized as `armadai.yaml` in project directories.
///
/// **Deprecated**: The fleet format is superseded by `ProjectConfig` (the modern
/// `armadai.yaml` format with `agents:`, `prompts:`, `skills:`, etc.).
/// Fleet files are still loaded for backward compatibility but will be removed
/// in a future release. Migrate by running `armadai init --project`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FleetDefinition {
    /// Fleet name
    pub fleet: String,
    /// Agent names (file stems without .md)
    pub agents: Vec<String>,
    /// Absolute path to the ArmadAI source directory
    pub source: PathBuf,
}

impl FleetDefinition {
    /// Load a fleet definition from a YAML file.
    ///
    /// **Deprecated**: Use `ProjectConfig::load()` instead.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let def: FleetDefinition = serde_yaml_ng::from_str(&content)?;
        Ok(def)
    }

    /// Save the fleet definition to a YAML file.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let content = serde_yaml_ng::to_string(self)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Check if the fleet contains a given agent name.
    pub fn contains_agent(&self, name: &str) -> bool {
        self.agents.iter().any(|a| a == name)
    }

    /// Return the agents directory for this fleet's source.
    pub fn agents_dir(&self) -> PathBuf {
        self.source.join("agents")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fleet_serialize_deserialize() {
        let fleet = FleetDefinition {
            fleet: "test-fleet".to_string(),
            agents: vec!["code-reviewer".to_string(), "test-writer".to_string()],
            source: PathBuf::from("/home/user/armadai"),
        };

        let yaml = serde_yaml_ng::to_string(&fleet).unwrap();
        let parsed: FleetDefinition = serde_yaml_ng::from_str(&yaml).unwrap();

        assert_eq!(parsed.fleet, "test-fleet");
        assert_eq!(parsed.agents.len(), 2);
        assert_eq!(parsed.source, PathBuf::from("/home/user/armadai"));
    }

    #[test]
    fn test_fleet_contains_agent() {
        let fleet = FleetDefinition {
            fleet: "test".to_string(),
            agents: vec!["a".to_string(), "b".to_string()],
            source: PathBuf::from("/tmp"),
        };

        assert!(fleet.contains_agent("a"));
        assert!(fleet.contains_agent("b"));
        assert!(!fleet.contains_agent("c"));
    }

    #[test]
    fn test_fleet_load_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("armadai.yaml");

        let fleet = FleetDefinition {
            fleet: "my-fleet".to_string(),
            agents: vec!["agent-a".to_string(), "agent-b".to_string()],
            source: PathBuf::from("/opt/armadai"),
        };

        fleet.save(&path).unwrap();
        let loaded = FleetDefinition::load(&path).unwrap();

        assert_eq!(loaded.fleet, "my-fleet");
        assert_eq!(loaded.agents, vec!["agent-a", "agent-b"]);
        assert_eq!(loaded.source, PathBuf::from("/opt/armadai"));
    }
}
