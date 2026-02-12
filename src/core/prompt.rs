use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::parser::frontmatter::extract_frontmatter;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PromptFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub apply_to: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Prompt {
    pub name: String,
    pub description: Option<String>,
    pub apply_to: Vec<String>,
    pub body: String,
    pub source: PathBuf,
}

// ---------------------------------------------------------------------------
// Parsing & loading
// ---------------------------------------------------------------------------

impl Prompt {
    /// Parse a prompt from its raw content and source path.
    pub fn parse(content: &str, source: PathBuf) -> anyhow::Result<Self> {
        let (fm_str, body) = extract_frontmatter(content);

        let fm: PromptFrontmatter = match fm_str {
            Some(yaml) => serde_yml::from_str(yaml)?,
            None => PromptFrontmatter::default(),
        };

        let name = fm.name.unwrap_or_else(|| name_from_path(&source));

        Ok(Self {
            name,
            description: fm.description,
            apply_to: fm.apply_to,
            body: body.to_string(),
            source,
        })
    }

    /// Load a prompt from a file path.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Self::parse(&content, path.to_path_buf())
    }
}

/// Derive a prompt name from its file path (stem without extension).
fn name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string()
}

/// Load all `.md` prompts from a directory (non-recursive).
pub fn load_all_prompts(dir: &Path) -> Vec<Prompt> {
    let mut prompts = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return prompts,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            match Prompt::load(&path) {
                Ok(p) => prompts.push(p),
                Err(e) => eprintln!("  warn: failed to load prompt {}: {e}", path.display()),
            }
        }
    }
    prompts.sort_by(|a, b| a.name.cmp(&b.name));
    prompts
}

/// Filter prompts that match a given agent name via `apply_to`.
#[allow(dead_code)]
pub fn matching_prompts<'a>(prompts: &'a [Prompt], agent_name: &str) -> Vec<&'a Prompt> {
    prompts
        .iter()
        .filter(|p| p.apply_to.iter().any(|a| a == agent_name || a == "*"))
        .collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_with_frontmatter() {
        let content = "---\nname: rust-conventions\ndescription: Rust coding style\napply_to:\n  - code-reviewer\n  - test-writer\n---\nAlways use snake_case.";
        let prompt = Prompt::parse(content, PathBuf::from("prompts/rust-conventions.md")).unwrap();
        assert_eq!(prompt.name, "rust-conventions");
        assert_eq!(prompt.description.as_deref(), Some("Rust coding style"));
        assert_eq!(prompt.apply_to, vec!["code-reviewer", "test-writer"]);
        assert_eq!(prompt.body, "Always use snake_case.");
    }

    #[test]
    fn test_parse_without_frontmatter() {
        let content = "# Just some instructions\n\nDo the thing.";
        let prompt = Prompt::parse(content, PathBuf::from("prompts/simple.md")).unwrap();
        assert_eq!(prompt.name, "simple");
        assert!(prompt.description.is_none());
        assert!(prompt.apply_to.is_empty());
        assert_eq!(prompt.body, content);
    }

    #[test]
    fn test_parse_name_fallback_to_filename() {
        let content = "---\ndescription: no name field\n---\nBody.";
        let prompt =
            Prompt::parse(content, PathBuf::from("/home/user/prompts/my-style.md")).unwrap();
        assert_eq!(prompt.name, "my-style");
    }

    #[test]
    fn test_load_all_prompts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("alpha.md"),
            "---\nname: alpha\n---\nAlpha body.",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("beta.md"),
            "---\nname: beta\n---\nBeta body.",
        )
        .unwrap();
        // Non-md file should be ignored
        std::fs::write(dir.path().join("readme.txt"), "not a prompt").unwrap();

        let prompts = load_all_prompts(dir.path());
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].name, "alpha");
        assert_eq!(prompts[1].name, "beta");
    }

    #[test]
    fn test_load_all_prompts_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let prompts = load_all_prompts(dir.path());
        assert!(prompts.is_empty());
    }

    #[test]
    fn test_load_all_prompts_nonexistent_dir() {
        let prompts = load_all_prompts(Path::new("/nonexistent/dir"));
        assert!(prompts.is_empty());
    }

    #[test]
    fn test_matching_prompts() {
        let prompts = vec![
            Prompt {
                name: "rust-style".to_string(),
                description: None,
                apply_to: vec!["code-reviewer".to_string()],
                body: String::new(),
                source: PathBuf::new(),
            },
            Prompt {
                name: "global".to_string(),
                description: None,
                apply_to: vec!["*".to_string()],
                body: String::new(),
                source: PathBuf::new(),
            },
            Prompt {
                name: "test-style".to_string(),
                description: None,
                apply_to: vec!["test-writer".to_string()],
                body: String::new(),
                source: PathBuf::new(),
            },
        ];

        let matched = matching_prompts(&prompts, "code-reviewer");
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].name, "rust-style");
        assert_eq!(matched[1].name, "global");
    }

    #[test]
    fn test_matching_prompts_no_match() {
        let prompts = vec![Prompt {
            name: "test-only".to_string(),
            description: None,
            apply_to: vec!["test-writer".to_string()],
            body: String::new(),
            source: PathBuf::new(),
        }];

        let matched = matching_prompts(&prompts, "code-reviewer");
        assert!(matched.is_empty());
    }
}
