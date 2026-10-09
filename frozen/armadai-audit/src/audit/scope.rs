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
//! One location needs a different reader rather than a different root:
//! `~/.config/armadai/agents` holds ArmadAI-format agents (`# H1` +
//! `## Metadata` + `## System Prompt`), which carry no YAML frontmatter at all.
//! [`armadai::parse_agents`] reads those through the product's own
//! `parse_agent_file`, and the assembly shape is what makes adding it a
//! one-line change here.
//!
//! [`ReverseLinker`]: crate::audit::reverse::ReverseLinker

use std::path::{Path, PathBuf};

use super::reverse::{
    ImportedConfig, armadai,
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
    /// Resolve from the environment, or `None` when `$HOME` is unset.
    ///
    /// Refusing is the only honest answer: a global audit *is* "what is under
    /// `~`", so with no `~` there is nothing to audit. Defaulting to `.` was
    /// measured to report the current repository's `.claude/` as the user's
    /// library, labelled `~/.claude` — indistinguishable, on stdout, from a
    /// real global run. `usage::discovery::projects_root` takes the same
    /// refusal (`.ok()?`) for the same reason, and has its own `NoHomeGuard`
    /// test for it.
    ///
    /// The ArmadAI root goes through `config_dir()` so `$ARMADAI_CONFIG_DIR`
    /// and `$XDG_CONFIG_HOME` keep working.
    pub fn from_env() -> Option<Self> {
        let home = PathBuf::from(std::env::var("HOME").ok()?);
        Some(Self {
            claude_home: home.join(".claude"),
            armadai_config: armadai_core::config::config_dir(),
            home,
        })
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
/// Two roots:
/// - `~/.claude` — `agents/`, `skills/` and `CLAUDE.md`, the same three
///   surfaces a repository exposes, at user level;
/// - `~/.config/armadai` — installed `skills/` and the user's own `agents/`.
///   The skills follow the Agent Skills standard (`SKILL.md` + frontmatter),
///   the same format `~/.claude/skills` uses, so the same parser reads them;
///   the agents are ArmadAI-format Markdown and go through
///   [`armadai::parse_agents`] instead (issue #391 — measured on one real
///   library, that directory was the largest pile of agentic assets on the
///   machine and the only one no pass could see).
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
        config
            .agents
            .extend(parse_agents(&claude_agents, &layout.claude_home));
        config
            .skills
            .extend(parse_skills(&claude_skills, &layout.claude_home));
        config.instructions = parse_instructions(&claude_md);
    }

    let armadai_skills = layout.armadai_config.join("skills");
    let armadai_agents = layout.armadai_config.join("agents");
    if armadai_skills.is_dir() || armadai_agents.is_dir() {
        detected.push(format!(
            "armadai ({})",
            tildify(layout, &layout.armadai_config)
        ));
        config
            .skills
            .extend(parse_skills(&armadai_skills, &layout.armadai_config));
        config.agents.extend(armadai::parse_agents(
            &armadai_agents,
            &layout.armadai_config,
        ));
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
/// The exclusions themselves are defensible; being silent about them is not.
/// A user reading "48 skills, 49967 tokens" reads it as the whole bill.
/// Measured on one real machine, `~/.claude/plugins/cache` alone holds 17
/// further `SKILL.md` worth ~39177 tokens, so the unqualified total was 44%
/// short of what is actually installed — and two of those skills cross `R01`.
///
/// Under `~/.claude`:
///
/// - **`plugins/cache/`** holds the plugins that are *installed*: their skills
///   are live in the session, exactly like `~/.claude/skills`. This pass does
///   not read them because knowing which of them are actually enabled means
///   reading Claude Code's own `installed_plugins.json` and the per-plugin
///   manifests — a plugin-aware importer, and a separate piece of work. Until
///   it exists the note carries the count, so the total is visibly a floor.
/// - **`plugins/marketplaces/`** holds the *catalogue* each plugin is
///   installed from — every plugin on offer, not the ones in use. Same
///   category as `registry/` below, and kept a separate line from
///   `plugins/cache/` precisely because the installed/catalogue distinction is
///   the one this pass makes everywhere else.
///
/// `plugins/data/` gets no line: it is Claude Code's per-plugin *writable
/// state*, one empty directory per installed plugin (measured: 5 directories,
/// 0 files, 0 `SKILL.md`). Nothing there is an agentic asset, so a note about
/// it would be the lecture this doc comment opens by ruling out.
///
/// Under `~/.config/armadai`:
///
/// - **`registry/`** is a synced catalogue of *other people's* assets. It is
///   also what skewed `R01`'s original calibration: of the 461 `SKILL.md`
///   files the threshold was derived from, 407 (88%) came from here. Auditing
///   them would be noise, and having measured them was a mistake.
/// - **`starters/`** holds starter packs — assets not installed, so not in
///   anyone's context. Same category as the catalogue.
///
/// `agents/` used to have a line here, and no longer does: since #391 it is
/// *read*, through the product's own parser rather than the Claude Code one
/// (see [`import_global_surfaces`]). A `Not read:` note is a promise about
/// what the report leaves out, so it has to disappear the moment the surface
/// stops being left out — a stale one is worse than none.
fn skipped_locations(layout: &GlobalLayout) -> Vec<String> {
    let mut out = Vec::new();

    let plugin_cache = layout.claude_home.join("plugins/cache");
    if plugin_cache.is_dir() {
        out.push(format!(
            "{} ({} skill(s)) — installed plugin skills, in your session context; reading them \
             needs a plugin-aware importer, so the counts above are a floor",
            tildify(layout, &plugin_cache),
            skill_md_count(&plugin_cache)
        ));
    }
    let marketplaces = layout.claude_home.join("plugins/marketplaces");
    if marketplaces.is_dir() {
        out.push(format!(
            "{} — catalogue of plugin marketplaces, not installed assets",
            tildify(layout, &marketplaces)
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

/// `SKILL.md` files anywhere under `dir`, to a bounded depth — how many skills
/// the plugin-cache note is about.
///
/// Recursive, because plugin skills sit five levels
/// down (`<marketplace>/<plugin>/<version>/skills/<name>/SKILL.md`) and the
/// layout is Claude Code's, not ours, so hard-coding that shape would go stale
/// silently. Three bounds keep the walk from becoming a liability on a tree we
/// do not own:
///
/// - [`MAX_SKILL_SCAN_DEPTH`] caps the descent;
/// - entries whose name starts with `.` are skipped, so a plugin checkout that
///   carries a `.git` never costs a walk through its object store (measured:
///   `plugins/marketplaces` has one, which is why *that* note carries no
///   count at all);
/// - the recursion tests [`std::fs::DirEntry::file_type`], which does not
///   follow symlinks, so a link pointing back up the tree cannot loop.
///
/// `0` when the directory is unreadable — the same answer the caller would
/// want to print.
fn skill_md_count(dir: &Path) -> usize {
    fn walk(dir: &Path, depth: u32) -> usize {
        if depth > MAX_SKILL_SCAN_DEPTH {
            return 0;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return 0;
        };
        let mut count = 0;
        for entry in entries.flatten() {
            let name = entry.file_name();
            if name.to_string_lossy().starts_with('.') {
                continue;
            }
            match entry.file_type() {
                Ok(t) if t.is_dir() => count += walk(&entry.path(), depth + 1),
                Ok(t) if t.is_file() && name == "SKILL.md" => count += 1,
                _ => {}
            }
        }
        count
    }
    walk(dir, 0)
}

/// Depth cap for [`skill_md_count`]: deeper than the five levels a plugin skill
/// really sits at, shallow enough that a pathological tree cannot stall the
/// audit. Same reasoning as `rules::rightsizing::MAX_INDEX_DEPTH`.
const MAX_SKILL_SCAN_DEPTH: u32 = 8;

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

    /// The 77 ArmadAI-format agents of one real library were the largest pile
    /// of agentic assets on the machine and the only one no pass could see
    /// (#391). They are read now — through the product's own parser, not the
    /// Claude Code one, which is what stops each of them from yielding an
    /// `A01` "missing YAML frontmatter".
    #[test]
    fn armadai_format_agents_are_read_through_their_own_parser() {
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

        let names: Vec<&str> = imported
            .config
            .agents
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, vec!["agent-builder", "dev-lead"]);
        assert!(
            imported.config.agents.iter().all(|a| a.issues.is_empty()),
            "read through the Claude Code parser every one of these yields an \
             A01 critical; through their own, none do: {:?}",
            imported
                .config
                .agents
                .iter()
                .flat_map(|a| a.issues.iter())
                .collect::<Vec<_>>()
        );
        assert!(
            !imported
                .skipped
                .iter()
                .any(|l| l.contains("armadai/agents")),
            "a `Not read:` note is a promise about what is left out, and this \
             surface is no longer left out: {:?}",
            imported.skipped
        );
    }

    /// The `armadai` root must be detected on its `agents/` alone. A library
    /// with agents and no installed skills is an ordinary shape (measured: the
    /// same machine had 77 agents and would still have them with zero skills),
    /// and gating detection on `skills/` would read them into a report that
    /// never names where they came from.
    #[test]
    fn an_armadai_root_holding_only_agents_is_still_detected() {
        let fake = Fake::new();
        fake.write(
            ".config/armadai/agents/capitaine.md",
            "# Capitaine\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nHi.",
        );

        let imported = import_global_surfaces(&fake.layout());

        assert_eq!(
            imported.detected,
            vec!["armadai (~/.config/armadai)".to_string()]
        );
        assert_eq!(imported.config.agents.len(), 1);
    }

    /// `~/.claude/agents` and `~/.config/armadai/agents` are two formats in one
    /// report, and each needs its own reader: the native file must keep its
    /// declared `tools`, the ArmadAI one must not be judged on a field it
    /// cannot carry.
    #[test]
    fn the_two_agent_roots_keep_their_own_formats() {
        let fake = Fake::new();
        fake.write(
            ".claude/agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews\ntools: Read\n---\nYou review.",
        );
        fake.write(
            ".config/armadai/agents/capitaine.md",
            "# Capitaine\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\n\
             You coordinate the fleet.",
        );

        let imported = import_global_surfaces(&fake.layout());

        let native = imported
            .config
            .agents
            .iter()
            .find(|a| a.name == "reviewer")
            .expect("the native agent must still be read");
        assert!(native.format.declares_tools());
        assert_eq!(
            native.metadata.tools.as_deref(),
            Some(&["Read".to_string()][..])
        );
        let mine = imported
            .config
            .agents
            .iter()
            .find(|a| a.name == "capitaine")
            .expect("the ArmadAI agent must be read");
        assert!(!mine.format.declares_tools());
        assert_eq!(
            mine.metadata.description.as_deref(),
            Some("You coordinate the fleet."),
            "the description ArmadAI publishes for it, as `link` writes it"
        );
    }

    /// Plugin skills are *installed* and live in the session, exactly like
    /// `~/.claude/skills`. This pass cannot read them yet (which plugins are
    /// enabled lives in Claude Code's own manifests), so the one thing it must
    /// not do is stay quiet: measured on one real machine, `plugins/cache`
    /// holds 17 further `SKILL.md` worth ~39177 tokens against a reported
    /// total of 49967, and two of them cross `R01`.
    #[test]
    fn installed_plugin_skills_are_named_with_their_count() {
        let fake = Fake::new();
        fake.write(".claude/skills/mine/SKILL.md", &skill_md("mine"));
        for name in ["brainstorming", "writing-skills"] {
            fake.write(
                &format!(".claude/plugins/cache/official/superpowers/6.3.0/skills/{name}/SKILL.md"),
                &skill_md(name),
            );
        }

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
            "plugin skills are not read by this pass — only named"
        );
        let note = imported
            .skipped
            .iter()
            .find(|l| l.contains("~/.claude/plugins/cache"))
            .unwrap_or_else(|| {
                panic!(
                    "installed plugin skills must be named, not silent: {:?}",
                    imported.skipped
                )
            });
        assert!(
            note.contains("2 skill(s)"),
            "the note must carry how many were left out, got: {note}"
        );
        assert!(
            note.contains("installed"),
            "the note must say these are installed, not catalogue, got: {note}"
        );
    }

    /// The catalogue is a different claim from the installed set, and gets its
    /// own line rather than being folded into one `~/.claude/plugins` note:
    /// merging them would erase the very installed/catalogue distinction this
    /// pass makes for `~/.config/armadai` (`skills` read, `registry` excluded).
    /// It carries no count — it holds a `.git` (measured), so counting it means
    /// walking an object store.
    #[test]
    fn the_plugin_marketplace_catalogue_is_a_separate_uncounted_line() {
        let fake = Fake::new();
        fake.write(".claude/skills/mine/SKILL.md", &skill_md("mine"));
        fake.write(
            ".claude/plugins/marketplaces/official/plugins/receipts/skills/receipts/SKILL.md",
            &skill_md("receipts"),
        );
        fake.write(
            ".claude/plugins/cache/official/superpowers/6.3.0/skills/x/SKILL.md",
            &skill_md("x"),
        );

        let imported = import_global_surfaces(&fake.layout());

        let catalogue: Vec<&String> = imported
            .skipped
            .iter()
            .filter(|l| l.contains("~/.claude/plugins/marketplaces"))
            .collect();
        assert_eq!(
            catalogue.len(),
            1,
            "the catalogue must have its own line: {:?}",
            imported.skipped
        );
        assert!(
            catalogue[0].contains("catalogue"),
            "the reason must be the catalogue one, got: {}",
            catalogue[0]
        );
        assert!(
            !catalogue[0].contains("skill(s)"),
            "the catalogue line must carry no count (it holds a .git), got: {}",
            catalogue[0]
        );
        assert!(
            imported
                .skipped
                .iter()
                .any(|l| l.contains("~/.claude/plugins/cache")),
            "and the installed set keeps its own, distinct line: {:?}",
            imported.skipped
        );
    }

    /// `plugins/data` is Claude Code's per-plugin *writable state* — measured
    /// on one real machine: 5 directories, 0 files. Nothing there is an
    /// agentic asset, and a `Not read:` line about it would be the lecture the
    /// "only when it exists" rule is meant to avoid.
    #[test]
    fn the_plugin_data_directory_gets_no_note() {
        let fake = Fake::new();
        fake.write(".claude/skills/mine/SKILL.md", &skill_md("mine"));
        std::fs::create_dir_all(fake.dir.path().join(".claude/plugins/data/superpowers")).unwrap();

        let imported = import_global_surfaces(&fake.layout());

        assert!(
            !imported.skipped.iter().any(|l| l.contains("plugins/data")),
            "per-plugin writable state is not an asset pile: {:?}",
            imported.skipped
        );
    }

    /// The plugin-cache count walks a tree we do not own, so it skips
    /// dot-directories. Without that, a plugin checkout carrying a `.git`
    /// costs a walk through its object store — and any `SKILL.md`-named blob
    /// in there would be counted as a skill.
    #[test]
    fn the_plugin_skill_count_ignores_dot_directories() {
        let fake = Fake::new();
        fake.write(".claude/skills/mine/SKILL.md", &skill_md("mine"));
        fake.write(
            ".claude/plugins/cache/official/plug/1.0.0/skills/real/SKILL.md",
            &skill_md("real"),
        );
        fake.write(
            ".claude/plugins/cache/official/plug/.git/objects/skills/x/SKILL.md",
            "not a skill",
        );

        let imported = import_global_surfaces(&fake.layout());

        let note = imported
            .skipped
            .iter()
            .find(|l| l.contains("~/.claude/plugins/cache"))
            .unwrap();
        assert!(
            note.contains("1 skill(s)"),
            "only the real skill counts, got: {note}"
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
