//! Black-box coverage for the `R` (rightsizing) rules, on the real binary.
//!
//! The 36 unit tests in `audit/rules/rightsizing.rs` call the rule functions
//! directly, on a hand-built `AuditContext`. They cannot see the chain that
//! actually delivers a finding to a user: `cli::audit::execute` → reverse pass
//! → `AuditSettings::from_project` → `run_rules` → `print_terminal`. A rule can
//! be flawless and never run — the dead-call defect #374 shipped. So every case
//! below asserts on the **spawned binary's stdout**, and each one is anchored on
//! a number that only the real chain can produce:
//!
//! - `R01` on a token count computed by the reverse pass from a file on disk
//!   (not a fixture literal), against the default threshold;
//! - `R01`'s `references/` discrimination under a **relative** root, the one
//!   configuration `has_references`'s doc comment claims is correct and which
//!   no unit test is allowed to rely on (the process cwd is shared by the whole
//!   test binary and moved mid-suite by `IsolatedProjectDir`);
//! - `R02` resolving a cited path against the **audited root** while a decoy of
//!   the same relative path sits in the cwd — the trap tracked at
//!   `config.rs:303`, and a false *negative*, so no unit test on a tempdir can
//!   expose it;
//! - `R04`'s total as exact arithmetic over three real files, which also
//!   re-proves that a skill's size counts the whole `SKILL.md` (frontmatter
//!   included) rather than its body alone;
//! - the `audit.skill_token_threshold` project setting reaching `R01`, proven by
//!   two runs of the same fixture that differ only by `armadai.yaml`.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use std::path::{Path, PathBuf};

    /// A sandbox holding the audited project plus the three directories the
    /// binary must be pointed at instead of the developer's real ones.
    struct Sandbox {
        dir: tempfile::TempDir,
    }

    impl Sandbox {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            for sub in ["config", "data", "transcripts", "project"] {
                std::fs::create_dir_all(dir.path().join(sub)).unwrap();
            }
            Self { dir }
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn project(&self) -> PathBuf {
            self.dir.path().join("project")
        }

        /// Writes `rel` under the audited project, parents included.
        fn write(&self, rel: &str, content: &str) {
            let path = self.project().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }

        /// Writes `rel` under the sandbox but **outside** the audited project —
        /// used to plant a decoy in the cwd.
        fn write_outside(&self, rel: &str, content: &str) {
            let path = self.dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }

        /// Spawns `armadai audit <target>` from `cwd`, fully isolated.
        ///
        /// Three redirections, each for a directory the audit would otherwise
        /// touch on this machine:
        /// - `ARMADAI_CONFIG_DIR` and `XDG_DATA_HOME`: the `#[cfg(test)]` guards
        ///   inside the crate do not apply to a *spawned* binary, so without
        ///   these a run could reach `~/.config/armadai` and
        ///   `~/.local/share/armadai`.
        /// - `ARMADAI_CLAUDE_PROJECTS_DIR`: `execute()` scans this project's
        ///   Claude Code transcripts unconditionally. Left alone it reads the
        ///   developer's real `~/.claude/projects` — read-only, but
        ///   machine-dependent and potentially hundreds of megabytes, and any
        ///   `U0x` finding it produced would land in the very stdout these
        ///   tests assert on. Pointed at an empty directory, the usage pass
        ///   finds nothing and stays silent.
        fn audit(&self, cwd: &Path, target: &str) -> Output {
            let out = Command::cargo_bin("armadai")
                .unwrap()
                .current_dir(cwd)
                .args(["audit", target])
                .env("NO_COLOR", "1")
                .env("ARMADAI_CONFIG_DIR", self.root().join("config"))
                .env("XDG_DATA_HOME", self.root().join("data"))
                .env(
                    "ARMADAI_CLAUDE_PROJECTS_DIR",
                    self.root().join("transcripts"),
                )
                .output()
                .unwrap();
            Output {
                success: out.status.success(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }
        }

        /// The common case: audit the project by absolute path, from the
        /// sandbox root.
        fn audit_project(&self) -> Output {
            let project = self.project();
            self.audit(self.root(), project.to_str().unwrap())
        }
    }

    struct Output {
        success: bool,
        stdout: String,
        stderr: String,
    }

    impl Output {
        /// Asserts the audit actually produced a report, so that every
        /// *absence* assertion below means "the rule stayed silent" rather than
        /// "the command printed nothing".
        fn ran(&self) -> &Self {
            assert!(
                self.stdout.contains("armadai audit -"),
                "the audit must have produced a report (success={}):\n{}\n{}",
                self.success,
                self.stdout,
                self.stderr
            );
            self
        }

        /// The single finding line carrying `rule` and every fragment in
        /// `needles`. Same-line, never `contains` over the whole output: the
        /// report names the same file on several lines (`A09`, `A12`, `R04`…),
        /// so a whole-output `contains(rule) && contains(name)` can pass with
        /// the rule silent — measured in `audit_usage.rs` for `U01`.
        fn line_with(&self, rule: &str, needles: &[&str]) -> &str {
            let hits: Vec<&str> = self
                .stdout
                .lines()
                .filter(|l| l.contains(rule) && needles.iter().all(|n| l.contains(n)))
                .collect();
            assert_eq!(
                hits.len(),
                1,
                "expected exactly one {rule} line carrying {needles:?}, got {}:\n{}",
                hits.len(),
                self.stdout
            );
            hits[0]
        }

        fn has_no_line_with(&self, rule: &str, needles: &[&str]) {
            let hits: Vec<&str> = self
                .stdout
                .lines()
                .filter(|l| l.contains(rule) && needles.iter().all(|n| l.contains(n)))
                .collect();
            assert!(
                hits.is_empty(),
                "expected no {rule} line carrying {needles:?}, got {hits:?}:\n{}",
                self.stdout
            );
        }
    }

    /// A well-formed `SKILL.md` whose whole file is exactly `total_chars` long,
    /// so the token count the reverse pass will compute is arithmetic, not an
    /// approximation. Panics rather than silently padding short, because a
    /// fixture that quietly missed its target size would weaken every
    /// assertion built on it.
    fn skill_md(name: &str, total_chars: usize) -> String {
        let head = format!("---\nname: {name}\ndescription: d\n---\n");
        let pad = total_chars
            .checked_sub(head.chars().count())
            .expect("total_chars must leave room for the frontmatter");
        format!("{head}{}", "x".repeat(pad))
    }

    // -- R01 ----------------------------------------------------------------

    /// The token count in the report is computed by the reverse pass from the
    /// bytes on disk: 16 040 chars / 4 = 4010. No fixture supplies it, and no
    /// unit test can — they all hand `body_tokens` to the rule directly.
    #[test]
    fn r01_sizes_an_oversized_skill_from_the_file_on_disk() {
        let sb = Sandbox::new();
        sb.write(".claude/skills/heavy/SKILL.md", &skill_md("heavy", 16_040));

        let out = sb.audit_project();
        out.ran();
        // 4010 > the default 3000, and both numbers must appear: the count
        // proves the file was measured, the threshold proves which setting
        // judged it.
        out.line_with("R01", &["heavy", "~4010 tokens", "(threshold 3000)"]);
    }

    /// Two oversized skills, one split and one not, audited through a
    /// **relative** root. `has_references` resolves `source_path.parent()`,
    /// which the reverse pass builds as `root.join(..)` — so under a relative
    /// root it is itself relative and resolves against the cwd. That is the
    /// configuration its doc comment declares correct and that unit tests are
    /// forbidden from depending on; this is the only place it is exercised.
    #[test]
    fn r01_spares_the_split_skill_under_a_relative_root() {
        let sb = Sandbox::new();
        sb.write(".claude/skills/heavy/SKILL.md", &skill_md("heavy", 16_040));
        sb.write(".claude/skills/split/SKILL.md", &skill_md("split", 16_040));
        sb.write(".claude/skills/split/references/detail.md", "detail");

        // cwd = sandbox root, target = "project": every path the reverse pass
        // builds is relative from here.
        let out = sb.audit(sb.root(), "project");
        out.ran();
        out.line_with("R01", &["heavy"]);
        out.has_no_line_with("R01", &["split"]);
    }

    // -- R02 ----------------------------------------------------------------

    /// A citation is resolved against the audited root, never against the cwd.
    ///
    /// The decoy makes this a real discrimination: `src/vanished/engine.rs`
    /// exists in the cwd and not under the project, so a rule that resolved
    /// against the process cwd would report **nothing** — a false negative,
    /// which no unit test on a tempdir can produce. The second citation, which
    /// does exist under the root, is the positive control that keeps the first
    /// assertion from passing on a rule that simply flags everything.
    #[test]
    fn r02_resolves_citations_against_the_audited_root_not_the_cwd() {
        let sb = Sandbox::new();
        sb.write(
            "CLAUDE.md",
            "The engine lives in `src/vanished/engine.rs`.\n\
             The parser is in `src/present/here.rs`.\n",
        );
        sb.write("src/present/here.rs", "// real\n");
        // Same relative path, in the cwd the binary is launched from.
        sb.write_outside("src/vanished/engine.rs", "// decoy\n");

        let out = sb.audit(sb.root(), "project");
        out.ran();
        out.line_with("R02", &["src/vanished/engine.rs"]);
        out.has_no_line_with("R02", &["src/present/here.rs"]);
    }

    // -- R04 ----------------------------------------------------------------

    /// The total is exact arithmetic over three files nobody handed the rule:
    /// `CLAUDE.md` at 400 chars (100 tokens) plus two `SKILL.md` at 400 chars
    /// each (100 tokens each) = 300.
    ///
    /// It also re-proves, through the CLI, that a skill's size counts the whole
    /// file: counting the body alone would give 92 tokens per skill (368 chars
    /// after a 32-char frontmatter) and a total of 284, not 300.
    #[test]
    fn r04_totals_the_front_loaded_context_from_the_real_files() {
        let sb = Sandbox::new();
        sb.write("CLAUDE.md", &"x".repeat(400));
        sb.write(".claude/skills/aa/SKILL.md", &skill_md("aa", 400));
        sb.write(".claude/skills/bb/SKILL.md", &skill_md("bb", 400));

        let out = sb.audit_project();
        out.ran();
        out.line_with(
            "R04",
            &[
                "~300 tokens",
                "100 from the instructions file",
                "200 across 2 skill(s)",
            ],
        );
        // Neither skill is anywhere near the default threshold, so the Info
        // line above is not a repackaged R01.
        out.has_no_line_with("R01", &[]);
    }

    // -- settings plumbing --------------------------------------------------

    /// `audit.skill_token_threshold` in the project config reaches `R01`
    /// through the real command.
    ///
    /// Two runs on the same fixture, differing only by `armadai.yaml`. The
    /// first is the control: at 200 tokens the skill is far below the 3000
    /// default, so the finding in the second run can only come from the config
    /// being read, parsed, and carried into the rule.
    #[test]
    fn the_project_config_threshold_reaches_r01_through_the_cli() {
        let sb = Sandbox::new();
        sb.write(".claude/skills/mid/SKILL.md", &skill_md("mid", 800));

        let before = sb.audit_project();
        before.ran();
        before.has_no_line_with("R01", &["mid"]);

        sb.write("armadai.yaml", "audit:\n  skill_token_threshold: 100\n");

        let after = sb.audit_project();
        after.ran();
        after.line_with("R01", &["mid", "~200 tokens", "(threshold 100)"]);
    }
}
