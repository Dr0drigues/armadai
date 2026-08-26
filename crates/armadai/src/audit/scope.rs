//! The two scopes an audit can run over, and the assembly of the global one.
//!
//! Project scope reads one repository through a [`ReverseLinker`]: everything
//! it needs sits under a single root, so `detect(root)` / `parse(root)` maps
//! onto it exactly. The global scope does not fit that shape — the user's
//! assets live in two unrelated trees (`~/.claude` and `~/.config/armadai`),
//! neither of which is under the other, and the global instructions file is
//! `~/.claude/CLAUDE.md` rather than `~/CLAUDE.md`. A `ReverseLinker` for it
//! would have to take a `root` it then ignores.
//!
//! So the global pass is an *assembly over known locations* instead: it calls
//! the very same three Claude Code parsers ([`parse_agents`], [`parse_skills`],
//! [`parse_instructions`]), pointed at explicit directories. Same reader, same
//! findings, no fake root.
//!
//! [`ReverseLinker`]: crate::audit::reverse::ReverseLinker

use std::path::{Path, PathBuf};

use super::reverse::{
    ImportedConfig,
    claude::{parse_agents, parse_instructions, parse_skills},
};

/// Which surface an audit run reads. Passed to the rule registry, never to a
/// rule: a rule that reads only `ctx.config` cannot tell which scope filled
/// it, so scope must not be something a rule can branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditScope {
    /// One repository: `.claude/agents/`, `.claude/skills/`, `CLAUDE.md`.
    Project,
    /// What this user carries into every session, wherever they work.
    Global,
}

/// The locations the global pass reads, resolved once.
///
/// Taken explicitly rather than read from the environment inside the pass, so
/// unit tests can point it at a tempdir without touching the process
/// environment (`env_lock()` is not reentrant, and the audit already reads
/// `$HOME` from two other places).
#[derive(Debug, Clone)]
pub struct GlobalLayout {
    /// `$HOME` — the anchor findings paths are displayed relative to.
    pub home: PathBuf,
    /// Claude Code's own user-level root, `~/.claude`.
    pub claude_home: PathBuf,
    /// ArmadAI's user-level root, `~/.config/armadai` (or wherever
    /// `$ARMADAI_CONFIG_DIR` / `$XDG_CONFIG_HOME` puts it).
    pub armadai_config: PathBuf,
}

impl GlobalLayout {
    /// Resolve from the environment. `$HOME` is read directly, exactly as
    /// `usage::discovery::projects_root` does for `~/.claude/projects`; the
    /// ArmadAI root goes through `config_dir()` so `$ARMADAI_CONFIG_DIR` and
    /// `$XDG_CONFIG_HOME` keep working.
    pub fn from_env() -> Self {
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));
        Self {
            claude_home: home.join(".claude"),
            armadai_config: armadai_core::config::config_dir(),
            home,
        }
    }
}

/// What the global pass read, and what it deliberately did not.
#[derive(Debug, Clone)]
pub struct GlobalImport {
    /// Labels of the roots that held at least one readable surface. Empty
    /// means "nothing to audit", which the CLI reports as such.
    pub detected: Vec<String>,
    pub config: ImportedConfig,
    /// Locations left unread, each with its reason. Stated rather than
    /// silent: a user with 77 agents in a directory the audit skips must be
    /// told, or the report reads as "you have no agents".
    pub skipped: Vec<String>,
}

/// Read every native Claude Code surface the user carries globally.
///
/// Two roots, one reader:
/// - `~/.claude` — `agents/`, `skills/` and `CLAUDE.md`, the same three
///   surfaces a repository exposes, at user level;
/// - `~/.config/armadai/skills` — installed skills. These follow the Agent
///   Skills standard (`SKILL.md` + frontmatter), the same format
///   `~/.claude/skills` uses, so the same parser reads them.
///
/// What it does not read, and why, is in [`skipped_locations`].
pub fn import_global_surfaces(layout: &GlobalLayout) -> GlobalImport {
    let mut config = ImportedConfig::default();
    let mut detected = Vec::new();

    let claude_agents = layout.claude_home.join("agents");
    let claude_skills = layout.claude_home.join("skills");
    let claude_md = layout.claude_home.join("CLAUDE.md");
    if claude_agents.is_dir() || claude_skills.is_dir() || claude_md.is_file() {
        detected.push(format!("claude ({})", tildify(layout, &layout.claude_home)));
        config.agents.extend(parse_agents(&claude_agents));
        config.skills.extend(parse_skills(&claude_skills));
        config.instructions = parse_instructions(&claude_md);
    }

    let armadai_skills = layout.armadai_config.join("skills");
    if armadai_skills.is_dir() {
        detected.push(format!(
            "armadai ({})",
            tildify(layout, &layout.armadai_config)
        ));
        config.skills.extend(parse_skills(&armadai_skills));
    }

    config.agents.sort_by(|a, b| a.name.cmp(&b.name));
    config.skills.sort_by(|a, b| a.name.cmp(&b.name));

    GlobalImport {
        detected,
        config,
        skipped: skipped_locations(layout),
    }
}

/// Global locations the audit deliberately leaves out, described only when
/// they actually exist on this machine — a note about a directory nobody has
/// is a lecture, not a fact.
///
/// Three exclusions, three different reasons:
///
/// - **`registry/`** is a synced catalogue of *other people's* assets. It is
///   also what skewed `R01`'s original calibration: of the 461 `SKILL.md`
///   files the threshold was derived from, 407 (88%) came from here. Auditing
///   them would be noise, and having measured them was a mistake.
/// - **`starters/`** holds starter packs — assets not installed, so not in
///   anyone's context. Same category as the catalogue.
/// - **`agents/`** holds ArmadAI-format agents (H1 + `## Metadata` +
///   `## System Prompt`), not native Claude Code frontmatter. Measured on the
///   77 agents of one real library: read through this reverse pass, every one
///   of them yields an `A01` critical ("missing YAML frontmatter") and the
///   command exits non-zero on a healthy library. Reading them needs an
///   ArmadAI-format reverse importer, which is a separate piece of work.
fn skipped_locations(layout: &GlobalLayout) -> Vec<String> {
    let mut out = Vec::new();
    let agents = layout.armadai_config.join("agents");
    if agents.is_dir() {
        out.push(format!(
            "{} ({} file(s)) — ArmadAI-format agents; this pass reads native Claude Code \
             frontmatter only",
            tildify(layout, &agents),
            md_file_count(&agents)
        ));
    }
    for (dir, why) in [
        ("registry", "synced catalogue of other people's assets"),
        ("starters", "starter packs, not installed assets"),
        (
            "skills-registry",
            "synced catalogue of other people's assets",
        ),
    ] {
        let path = layout.armadai_config.join(dir);
        if path.is_dir() {
            out.push(format!("{} — {why}", tildify(layout, &path)));
        }
    }
    out
}

/// Top-level `*.md` files in `dir` — how many assets the skipped note is
/// about. `0` when the directory is unreadable, which is also what the
/// caller would want to print.
fn md_file_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_file())
                .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                .count()
        })
        .unwrap_or(0)
}

/// `~/.config/armadai` rather than `/Users/someone/.config/armadai`, when the
/// path sits under `$HOME`. Absolute otherwise (a redirected
/// `$ARMADAI_CONFIG_DIR` need not be under the home directory at all).
fn tildify(layout: &GlobalLayout, p: &Path) -> String {
    match p.strip_prefix(&layout.home) {
        Ok(rel) => format!("~/{}", rel.display()),
        Err(_) => p.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A layout over a tempdir. No environment variable is touched: the whole
    /// point of taking `GlobalLayout` explicitly is that the unit tests can
    /// stay off `env_lock()` (which is not reentrant) while the black-box
    /// suite proves `from_env` separately.
    struct Fake {
        dir: tempfile::TempDir,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn layout(&self) -> GlobalLayout {
            let home = self.dir.path().to_path_buf();
            GlobalLayout {
                claude_home: home.join(".claude"),
                armadai_config: home.join(".config/armadai"),
                home,
            }
        }

        /// Writes `rel` under the fake home, parents included.
        fn write(&self, rel: &str, content: &str) {
            let p = self.dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }
    }

    fn skill_md(name: &str) -> String {
        format!("---\nname: {name}\ndescription: {name} does things\n---\nBody.")
    }

    #[test]
    fn both_global_homes_are_read_through_the_same_parsers() {
        let fake = Fake::new();
        fake.write(
            ".claude/agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews\n---\nYou review.",
        );
        fake.write(".claude/skills/native/SKILL.md", &skill_md("native"));
        fake.write(".claude/CLAUDE.md", "# Global\nUse @reviewer.");
        fake.write(
            ".config/armadai/skills/installed/SKILL.md",
            &skill_md("installed"),
        );

        let imported = import_global_surfaces(&fake.layout());

        let names: Vec<&str> = imported
            .config
            .skills
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["installed", "native"],
            "both skill roots must be read, sorted by name"
        );
        assert_eq!(
            imported.config.agents.len(),
            1,
            "~/.claude/agents must be read: {:?}",
            imported.config.agents
        );
        let instructions = imported
            .config
            .instructions
            .as_ref()
            .expect("~/.claude/CLAUDE.md is the global instructions file, not ~/CLAUDE.md");
        assert!(instructions.content.contains("@reviewer"));
        assert_eq!(
            imported.detected,
            vec![
                "claude (~/.claude)".to_string(),
                "armadai (~/.config/armadai)".to_string()
            ]
        );
    }

    /// `~/CLAUDE.md` is *not* the global instructions file — the project pass
    /// looks for `<root>/CLAUDE.md`, and reusing that mapping for the home
    /// directory would read the wrong file (or, here, nothing at all).
    #[test]
    fn the_home_directory_root_is_not_mistaken_for_the_instructions_file() {
        let fake = Fake::new();
        fake.write("CLAUDE.md", "# Wrong place");
        fake.write(".claude/skills/x/SKILL.md", &skill_md("x"));

        let imported = import_global_surfaces(&fake.layout());

        assert!(
            imported.config.instructions.is_none(),
            "~/CLAUDE.md must not be read as the global instructions file"
        );
    }

    /// The synced catalogue is what skewed `R01`'s calibration (407 of 461
    /// files). Neither scope reads it, and this is the guard that says so:
    /// the two decoys sit exactly where the two plausible mistakes would find
    /// them — as a direct child of `registry/` (the shape `parse_skills`
    /// reads) and at the real catalogue's own depth.
    #[test]
    fn the_synced_catalogue_is_never_read() {
        let fake = Fake::new();
        fake.write(".config/armadai/skills/mine/SKILL.md", &skill_md("mine"));
        fake.write(
            ".config/armadai/registry/borrowed/SKILL.md",
            &skill_md("borrowed"),
        );
        fake.write(
            ".config/armadai/registry/repo/skills/deep-borrowed/SKILL.md",
            &skill_md("deep-borrowed"),
        );

        let imported = import_global_surfaces(&fake.layout());

        let names: Vec<&str> = imported
            .config
            .skills
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(
            names,
            vec!["mine"],
            "only installed skills are audited; the catalogue is other people's"
        );
        assert!(
            imported
                .skipped
                .iter()
                .any(|l| l.contains("~/.config/armadai/registry")),
            "the exclusion must be stated, not silent: {:?}",
            imported.skipped
        );
    }

    /// Reading the 77 ArmadAI-format agents of a real library through this
    /// (Claude Code frontmatter) reverse pass produced 77 `A01` criticals and
    /// a non-zero exit on a healthy library. They are skipped — and named,
    /// with their count, so the report never reads as "you have no agents".
    #[test]
    fn armadai_format_agents_are_skipped_and_counted() {
        let fake = Fake::new();
        fake.write(
            ".config/armadai/agents/agent-builder.md",
            "# Agent Builder\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nHi.",
        );
        fake.write(
            ".config/armadai/agents/dev-lead.md",
            "# Dev Lead\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nHi.",
        );
        fake.write(".config/armadai/skills/mine/SKILL.md", &skill_md("mine"));

        let imported = import_global_surfaces(&fake.layout());

        assert!(
            imported.config.agents.is_empty(),
            "ArmadAI-format agents are not native frontmatter and must not be \
             parsed as such: {:?}",
            imported.config.agents
        );
        let note = imported
            .skipped
            .iter()
            .find(|l| l.contains("~/.config/armadai/agents"))
            .unwrap_or_else(|| panic!("the skipped pile must be named: {:?}", imported.skipped));
        assert!(
            note.contains("2 file(s)"),
            "the note must carry how much was skipped, got: {note}"
        );
    }

    #[test]
    fn nothing_installed_means_nothing_detected() {
        let fake = Fake::new();
        let imported = import_global_surfaces(&fake.layout());
        assert!(
            imported.detected.is_empty(),
            "an empty library must report nothing detected, so the CLI can say so"
        );
        assert!(imported.skipped.is_empty(), "{:?}", imported.skipped);
    }

    /// A config root outside `$HOME` (a redirected `$ARMADAI_CONFIG_DIR`) has
    /// no `~/` form, and printing one would be a lie about where the file is.
    #[test]
    fn a_config_root_outside_home_is_shown_absolute() {
        let home = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(elsewhere.path().join("skills/x")).unwrap();
        std::fs::write(elsewhere.path().join("skills/x/SKILL.md"), skill_md("x")).unwrap();
        let layout = GlobalLayout {
            home: home.path().to_path_buf(),
            claude_home: home.path().join(".claude"),
            armadai_config: elsewhere.path().to_path_buf(),
        };

        let imported = import_global_surfaces(&layout);

        assert_eq!(
            imported.detected,
            vec![format!("armadai ({})", elsewhere.path().display())]
        );
    }
}
