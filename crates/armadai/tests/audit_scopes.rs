//! Black-box coverage for `armadai audit --global`, on the real binary.
//!
//! The unit tests in `audit/scope.rs` and `audit/rules/mod.rs` prove the
//! assembly and the registry in isolation. They cannot prove the thing the
//! feature *is*: that a flag on the command line changes which files get
//! audited. Two measured defects on this repository say why that gap matters —
//! a rule can be perfect and never registered (#374), and a whole output loop
//! can be cut while 734 unit tests stay green. So every case below asserts on
//! the **spawned binary's stdout**, and each one is a *differential*: the same
//! fixture audited in both scopes, where the two must disagree.
//!
//! Nothing here reads the developer's real configuration. Five directories are
//! redirected (see `Sandbox::run`), `HOME` among them, because the global
//! scope resolves `~/.claude` from it — a test that skipped that redirection
//! would pass or fail according to whose machine ran it.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use std::path::{Path, PathBuf};

    /// A `SKILL.md` body big enough to cross `R01`'s 4000-token default
    /// (`chars / 4`), so the finding is arithmetic rather than a guess.
    fn heavy_skill(name: &str) -> String {
        let fm = format!("---\nname: {name}\ndescription: {name} does things\n---\n");
        format!("{fm}{}", "x".repeat(20_000))
    }

    /// A sandbox holding a fake `$HOME`, a fake ArmadAI config root under it,
    /// and a working project — every location either scope can reach.
    struct Sandbox {
        dir: tempfile::TempDir,
    }

    impl Sandbox {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            for sub in [
                "home",
                "home/.config/armadai",
                "data",
                "transcripts",
                "work",
            ] {
                std::fs::create_dir_all(dir.path().join(sub)).unwrap();
            }
            Self { dir }
        }

        fn home(&self) -> PathBuf {
            self.dir.path().join("home")
        }

        fn work(&self) -> PathBuf {
            self.dir.path().join("work")
        }

        /// Writes `rel` under the sandbox root, parents included.
        fn write(&self, rel: &str, content: &str) {
            let path = self.dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }

        /// Spawns `armadai audit <args>` from `work/`, fully isolated.
        ///
        /// Five redirections, each for a directory one of the two scopes would
        /// otherwise touch on this machine:
        /// - `HOME`: the global scope resolves `~/.claude/{agents,skills}` and
        ///   `~/.claude/CLAUDE.md` from it.
        /// - `ARMADAI_CONFIG_DIR` **and** `XDG_CONFIG_HOME`: the global scope
        ///   reads `<config>/skills`, and the second is set as well so the
        ///   first failing to apply cannot silently fall through to the real
        ///   `~/.config/armadai`.
        /// - `XDG_DATA_HOME`: the `#[cfg(test)]` guards inside the crate do
        ///   not apply to a *spawned* binary, so without it a run could reach
        ///   `~/.local/share/armadai`.
        /// - `ARMADAI_CLAUDE_PROJECTS_DIR`: project scope scans transcripts
        ///   unconditionally; left alone it reads the developer's real
        ///   `~/.claude/projects`, whose `U0x` findings would land in the very
        ///   stdout these tests assert on.
        fn run(&self, args: &[&str]) -> Output {
            let out = Command::cargo_bin("armadai")
                .unwrap()
                .current_dir(self.work())
                .arg("audit")
                .args(args)
                .env("NO_COLOR", "1")
                .env("HOME", self.home())
                .env("ARMADAI_CONFIG_DIR", self.home().join(".config/armadai"))
                .env("XDG_CONFIG_HOME", self.home().join(".config"))
                .env("XDG_DATA_HOME", self.dir.path().join("data"))
                .env(
                    "ARMADAI_CLAUDE_PROJECTS_DIR",
                    self.dir.path().join("transcripts"),
                )
                .output()
                .unwrap();
            Output {
                success: out.status.success(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }
        }

        /// Same run with `HOME` *removed* from the child's environment.
        ///
        /// A spawned binary is the only honest way to test this: `$HOME` is
        /// process-global, and unsetting it in-process means holding
        /// `env_lock()`, which is not reentrant and which several tests in
        /// this crate already take.
        fn run_without_home(&self, args: &[&str]) -> Output {
            let out = Command::cargo_bin("armadai")
                .unwrap()
                .current_dir(self.work())
                .arg("audit")
                .args(args)
                .env("NO_COLOR", "1")
                .env_remove("HOME")
                .env("ARMADAI_CONFIG_DIR", self.home().join(".config/armadai"))
                .env("XDG_CONFIG_HOME", self.home().join(".config"))
                .env("XDG_DATA_HOME", self.dir.path().join("data"))
                .env(
                    "ARMADAI_CLAUDE_PROJECTS_DIR",
                    self.dir.path().join("transcripts"),
                )
                .output()
                .unwrap();
            Output {
                success: out.status.success(),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            }
        }
    }

    struct Output {
        success: bool,
        stdout: String,
        stderr: String,
    }

    impl Output {
        /// Asserts the audit actually produced a report, so that every
        /// *absence* assertion below means "the rule stayed silent" rather
        /// than "the command printed nothing" — the failure mode that makes a
        /// negative assertion worthless.
        fn ran(&self) -> &Self {
            assert!(
                self.stdout.contains("armadai audit"),
                "the audit must have produced a report (success={}):\n{}\n{}",
                self.success,
                self.stdout,
                self.stderr
            );
            self
        }

        /// The single line carrying `rule` and every fragment in `needles`.
        /// Same-line, never `contains` over the whole output: the report names
        /// the same file on several lines (`A09`, `A12`, `R04`…), so a
        /// whole-output `contains(rule) && contains(name)` can pass with the
        /// rule silent.
        fn line_with(&self, rule: &str, needles: &[&str]) -> &str {
            let hit: Vec<&str> = self
                .stdout
                .lines()
                .filter(|l| l.contains(rule) && needles.iter().all(|n| l.contains(n)))
                .collect();
            assert_eq!(
                hit.len(),
                1,
                "expected exactly one {rule} line carrying {needles:?}, got {}:\n{}",
                hit.len(),
                self.stdout
            );
            hit[0]
        }

        fn has_no_line_with(&self, rule: &str, needles: &[&str]) -> &Self {
            let hit: Vec<&str> = self
                .stdout
                .lines()
                .filter(|l| l.contains(rule) && needles.iter().all(|n| l.contains(n)))
                .collect();
            assert!(
                hit.is_empty(),
                "expected no {rule} line carrying {needles:?}, got {hit:?}:\n{}",
                self.stdout
            );
            self
        }
    }

    /// The whole point of the feature: skills live in the user's library, not
    /// in repositories. Measured on three real projects — zero local skills
    /// against 48 installed globally — which is why `R01` never fired before
    /// this flag existed.
    #[test]
    fn global_finds_the_installed_skills_a_project_scope_cannot_see() {
        let s = Sandbox::new();
        s.write(
            "home/.claude/skills/native-heavy/SKILL.md",
            &heavy_skill("native-heavy"),
        );
        s.write(
            "home/.config/armadai/skills/installed-heavy/SKILL.md",
            &heavy_skill("installed-heavy"),
        );
        // A project that is detectable, so its own run produces a report.
        s.write(
            "work/.claude/agents/dev.md",
            "---\nname: dev\ndescription: Develops\ntools: Read\n---\nShort.",
        );

        let global = s.run(&["--global"]);
        global.ran();
        global.line_with("R01", &["native-heavy"]);
        global.line_with("R01", &["installed-heavy"]);

        let project = s.run(&[]);
        project.ran();
        project
            .has_no_line_with("R01", &["native-heavy"])
            .has_no_line_with("R01", &["installed-heavy"]);
    }

    /// The separation has to hold in both directions: a repository's own
    /// skills are the project's business and must not leak into the global
    /// report, or `--global` would just be "audit everything".
    #[test]
    fn a_project_skill_stays_out_of_the_global_report() {
        let s = Sandbox::new();
        s.write(
            "work/.claude/skills/repo-heavy/SKILL.md",
            &heavy_skill("repo-heavy"),
        );
        // A global skill, so `--global` has something to report and the
        // absence below is silence from the rule, not from the command.
        s.write(
            "home/.claude/skills/mine-heavy/SKILL.md",
            &heavy_skill("mine-heavy"),
        );

        let project = s.run(&[]);
        project.ran().line_with("R01", &["repo-heavy"]);

        let global = s.run(&["--global"]);
        global.ran().line_with("R01", &["mine-heavy"]);
        global.has_no_line_with("R01", &["repo-heavy"]);
    }

    /// `U01`-`U04` correlate declarations against *one project's* transcripts,
    /// so they are the single family the global registry drops. Proven with
    /// transcripts that demonstrably fire them in project scope from the same
    /// working directory — otherwise this would only prove that no transcript
    /// was found.
    #[test]
    fn global_never_measures_observed_usage() {
        let s = Sandbox::new();
        s.write(
            "work/.claude/agents/ghost.md",
            "---\nname: ghost\ndescription: never invoked\n---\nBody",
        );
        s.write(
            "home/.claude/skills/mine-heavy/SKILL.md",
            &heavy_skill("mine-heavy"),
        );
        // The binary runs with `current_dir(work)` and resolves its root from
        // `current_dir()`, which on macOS returns the `/private/var/...` form
        // of a tempdir's `/var/...` path. The transcript's `cwd` is what the
        // discovery pass matches on, so it has to be the same form.
        let cwd = std::fs::canonicalize(s.work())
            .unwrap()
            .to_string_lossy()
            .to_string();
        s.write(
            "transcripts/session/s1.jsonl",
            &format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"2026-08-01T00:00:00Z\",\
                 \"isSidechain\":false,\"uuid\":\"u1\",\"cwd\":\"{cwd}\",\"message\":{{\
                 \"model\":\"m\",\"content\":[{{\"type\":\"tool_use\",\"id\":\"t1\",\
                 \"name\":\"Agent\",\"input\":{{\"subagent_type\":\"general-purpose\",\
                 \"description\":\"work\"}}}}],\"usage\":{{\"input_tokens\":1,\
                 \"output_tokens\":1}}}}}}\n"
            ),
        );

        let project = s.run(&[]);
        project.ran();
        assert!(
            project.stdout.contains("Observed usage"),
            "the fixture must fire the usage pass in project scope:\n{}",
            project.stdout
        );
        project.line_with("U01", &["ghost"]);
        project.line_with("U02", &["general-purpose"]);

        let global = s.run(&["--global"]);
        global.ran().line_with("R01", &["mine-heavy"]);
        assert!(
            !global.stdout.contains("Observed usage"),
            "the global library belongs to no project: no transcript correlation:\n{}",
            global.stdout
        );
        global
            .has_no_line_with("U01", &["ghost"])
            .has_no_line_with("U02", &["general-purpose"]);
    }

    /// The synced catalogue is other people's assets, and it is what skewed
    /// `R01`'s original calibration (407 of 461 files). Excluding it is a
    /// correctness fix, so it gets a black-box guard: a decoy heavy skill in
    /// the catalogue against a control heavy skill in the installed set.
    #[test]
    fn global_never_reads_the_synced_catalogue() {
        let s = Sandbox::new();
        s.write(
            "home/.config/armadai/skills/mine-heavy/SKILL.md",
            &heavy_skill("mine-heavy"),
        );
        s.write(
            "home/.config/armadai/registry/borrowed-heavy/SKILL.md",
            &heavy_skill("borrowed-heavy"),
        );
        s.write(
            "home/.config/armadai/registry/repo/skills/deep-borrowed/SKILL.md",
            &heavy_skill("deep-borrowed"),
        );
        s.write(
            "home/.config/armadai/starters/pack/skills/starter-heavy/SKILL.md",
            &heavy_skill("starter-heavy"),
        );

        let out = s.run(&["--global"]);
        out.ran().line_with("R01", &["mine-heavy"]);
        out.has_no_line_with("R01", &["borrowed-heavy"])
            .has_no_line_with("R01", &["deep-borrowed"])
            .has_no_line_with("R01", &["starter-heavy"]);
        assert!(
            out.stdout.contains("Not read:") && out.stdout.contains("armadai/registry"),
            "the exclusion must be stated in the report:\n{}",
            out.stdout
        );
    }

    /// `~/.config/armadai/agents` holds ArmadAI-format agents, not native
    /// Claude Code frontmatter. Measured on a real 77-agent library, reading
    /// them through this reverse pass yields 77 `A01` criticals and a non-zero
    /// exit on a healthy library. They are skipped — and *named*, so the
    /// report never reads as "you have no agents".
    #[test]
    fn global_names_the_armadai_agent_library_it_skips() {
        let s = Sandbox::new();
        for name in ["agent-builder", "dev-lead", "qa"] {
            s.write(
                &format!("home/.config/armadai/agents/{name}.md"),
                "# Name\n\n## Metadata\n- provider: claude\n\n## System Prompt\n\nHi.",
            );
        }
        s.write(
            "home/.claude/skills/mine/SKILL.md",
            "---\nname: mine\ndescription: d\n---\nB.",
        );

        let out = s.run(&["--global"]);
        out.ran();
        assert!(
            out.success,
            "an ArmadAI-format library is not a critical finding:\n{}\n{}",
            out.stdout, out.stderr
        );
        let line = out.line_with("Not read:", &["armadai/agents", "3 file(s)"]);
        assert!(
            line.contains("native Claude Code"),
            "the note must say why, got: {line}"
        );
        out.has_no_line_with("A01", &["agent-builder"]);
    }

    /// The plugin trees under `~/.claude` are the largest thing the pass does
    /// not read, and the one it was silent about. Measured on one real machine:
    /// the report announced "48 skills, ~49967 tokens" while
    /// `~/.claude/plugins/cache` held 17 further installed `SKILL.md` worth
    /// ~39177 tokens — the stated total 44% short — two of which cross `R01`.
    ///
    /// Two lines, not one, and the distinction is the point: `cache/` is what
    /// is *installed* and live in the session, `marketplaces/` is the
    /// catalogue it was installed from. Folding them together would erase the
    /// installed-vs-catalogue distinction this pass makes everywhere else
    /// (`~/.config/armadai/skills` read, `registry/` excluded).
    #[test]
    fn global_names_the_plugin_skills_it_does_not_read() {
        let s = Sandbox::new();
        s.write(
            "home/.claude/skills/mine/SKILL.md",
            "---\nname: mine\ndescription: d\n---\nB.",
        );
        for name in ["writing-skills", "brainstorming"] {
            s.write(
                &format!(
                    "home/.claude/plugins/cache/official/superpowers/6.3.0/skills/{name}/SKILL.md"
                ),
                &heavy_skill(name),
            );
        }
        s.write(
            "home/.claude/plugins/marketplaces/official/plugins/receipts/skills/receipts/SKILL.md",
            &heavy_skill("receipts"),
        );

        let out = s.run(&["--global"]);
        out.ran();

        let installed = out.line_with("Not read:", &["plugins/cache", "2 skill(s)"]);
        assert!(
            installed.contains("installed"),
            "the installed set must be named as installed, got: {installed}"
        );
        let catalogue = out.line_with("Not read:", &["plugins/marketplaces"]);
        assert!(
            catalogue.contains("catalogue"),
            "the catalogue must be named as a catalogue, got: {catalogue}"
        );
        assert_ne!(
            installed, catalogue,
            "the two must be separate lines, not one note about ~/.claude/plugins"
        );
        // Named, not read: the heavy plugin skills must not become findings.
        out.has_no_line_with("R01", &["writing-skills"])
            .has_no_line_with("R01", &["receipts"]);
    }

    /// A report file that does not say which scope it covers is a trap: the
    /// two carry identical rule codes over different assets.
    #[test]
    fn a_global_report_file_says_so() {
        let s = Sandbox::new();
        s.write(
            "home/.claude/skills/mine/SKILL.md",
            "---\nname: mine\ndescription: d\n---\nB.",
        );

        let out = s.run(&["--global", "--report", "g.md"]);
        out.ran();
        let md = std::fs::read_to_string(s.work().join("g.md")).unwrap();
        assert!(
            md.starts_with("# armadai audit (global)"),
            "the report must name its scope:\n{md}"
        );
    }

    /// `--global` and a path are contradictory: one says "this repository",
    /// the other "no repository at all". Refused by the parser rather than
    /// silently resolved in favour of one of them.
    #[test]
    fn global_and_an_explicit_path_are_mutually_exclusive() {
        let s = Sandbox::new();
        let out = s.run(&["--global", "/tmp"]);
        assert!(!out.success, "expected a parse error:\n{}", out.stdout);
        assert!(
            out.stderr.contains("cannot be used with"),
            "expected clap's conflict message, got:\n{}",
            out.stderr
        );
    }

    /// The exit code must still mean what it means: a broken global asset is
    /// a critical finding, and the command fails on it.
    #[test]
    fn a_critical_finding_in_the_global_library_still_fails_the_command() {
        let s = Sandbox::new();
        s.write(
            "home/.claude/skills/broken/SKILL.md",
            "---\nname: broken\ndescription: has a bad : unquoted value\n---\nBody",
        );

        let out = s.run(&["--global"]);
        out.ran().line_with("A01", &["broken"]);
        assert!(
            !out.success,
            "a critical finding must still exit non-zero:\n{}",
            out.stdout
        );
    }

    /// `--propose` in global scope has no project root to write into, so it
    /// writes into the **current directory, whatever that is** — the same place
    /// project scope would.
    ///
    /// Not "never into `$HOME`": if the user stands *in* `$HOME`, that is where
    /// the pack lands, and correctly so. The invariant is the cwd, and the
    /// sandbox below stands in `work/`, which is why nothing appears under the
    /// fake home.
    #[test]
    fn global_propose_writes_into_the_current_directory() {
        let s = Sandbox::new();
        s.write(
            "home/.claude/agents/reviewer.md",
            "---\nname: reviewer\ndescription: Reviews code\ntools: Read\n---\nYou review code.",
        );
        s.write(
            "home/.claude/skills/mine/SKILL.md",
            "---\nname: mine\ndescription: d\n---\nB.",
        );

        let out = s.run(&["--global", "--propose"]);
        out.ran();
        assert!(
            s.work().join(".armadai-proposal/pack.yaml").is_file(),
            "the proposal belongs in the working directory:\n{}\n{}",
            out.stdout,
            out.stderr
        );
        assert!(
            !s.home().join(".armadai-proposal").exists(),
            "the pack follows the cwd, and the cwd here is work/, not the home"
        );
    }

    /// An empty library is not an error, and must not be reported as a
    /// project path the user never named.
    #[test]
    fn an_empty_global_library_is_reported_as_such() {
        let s = Sandbox::new();
        let out = s.run(&["--global"]);
        assert!(out.success, "{}\n{}", out.stdout, out.stderr);
        assert!(
            out.stdout
                .contains("No native agentic configuration detected in your global library"),
            "got:\n{}",
            out.stdout
        );
    }

    /// With no `$HOME` there is no `~`, so there is nothing global to audit.
    ///
    /// The fallback that used to stand there (`unwrap_or_else(|_| ".")`) made
    /// `--global` report the *current repository's* `.claude/` as the user's
    /// library, labelled `~/.claude` — a wrong answer indistinguishable from a
    /// right one on stdout. `usage::discovery::projects_root` refuses in the
    /// same situation; this is the guard that keeps the two consistent.
    #[test]
    fn global_refuses_to_run_without_a_home_rather_than_auditing_the_repository() {
        let s = Sandbox::new();
        // A project that would be picked up by the `.` fallback, and a global
        // library that is *not* it, so the two cannot be confused.
        s.write(
            "work/.claude/skills/repo-heavy/SKILL.md",
            &heavy_skill("repo-heavy"),
        );
        s.write(
            "home/.claude/skills/mine-heavy/SKILL.md",
            &heavy_skill("mine-heavy"),
        );

        let out = s.run_without_home(&["--global"]);

        assert!(
            !out.success,
            "a global audit with no $HOME must refuse, not improvise:\n{}\n{}",
            out.stdout, out.stderr
        );
        assert!(
            out.stderr.contains("$HOME"),
            "the refusal must name the cause, got:\n{}",
            out.stderr
        );
        assert!(
            !out.stdout.contains("repo-heavy"),
            "the working repository is not the user's global library:\n{}",
            out.stdout
        );
        assert!(
            !out.stdout.contains("~/.claude"),
            "and nothing may be labelled ~/.claude when there is no ~:\n{}",
            out.stdout
        );
        // The control: the very same fixture audits fine with a $HOME, so the
        // refusal above is about $HOME and not about the fixture.
        s.run(&["--global"]).ran().line_with("R01", &["mine-heavy"]);
    }

    /// A global audit reads one fixed set of assets, so it must reach one
    /// fixed verdict. Its thresholds come from the *global* config
    /// (`~/.config/armadai/config.yaml`), not from `<cwd>/armadai.yaml`:
    /// measured before this fix, the same library reported 2 `R01` warnings
    /// from a directory carrying `skill_token_threshold: 5` and 0 from a
    /// neutral one.
    #[test]
    fn global_thresholds_come_from_the_global_config_not_the_working_directory() {
        let s = Sandbox::new();
        // Small enough that the 4000 default never flags it.
        s.write(
            "home/.claude/skills/small/SKILL.md",
            "---\nname: small\ndescription: d\n---\nB.",
        );
        s.write(
            "home/.config/armadai/config.yaml",
            "audit:\n  skill_token_threshold: 1\n",
        );
        // The cwd says the opposite, loudly. It must not be consulted.
        s.write(
            "work/armadai.yaml",
            "audit:\n  skill_token_threshold: 100000\n",
        );

        let out = s.run(&["--global"]);
        out.ran().line_with("R01", &["small"]);
    }

    /// The mirror: a project audit keeps reading the project's own config, and
    /// the global one must not override it. Without this, "settings follow the
    /// audited surface" could be satisfied by making *both* scopes read the
    /// global file.
    #[test]
    fn a_project_audit_still_reads_the_project_config_not_the_global_one() {
        let s = Sandbox::new();
        s.write(
            "work/.claude/skills/small/SKILL.md",
            "---\nname: small\ndescription: d\n---\nB.",
        );
        s.write(
            "work/armadai.yaml",
            "audit:\n  skill_token_threshold: 1\n  usage: false\n",
        );
        // A global config that would silence the finding if it were read.
        s.write(
            "home/.config/armadai/config.yaml",
            "audit:\n  skill_token_threshold: 100000\n",
        );

        let out = s.run(&[]);
        out.ran().line_with("R01", &["small"]);
    }

    /// Findings in a global report are shown relative to `$HOME`, which is what
    /// makes them readable as `.claude/skills/x/SKILL.md` rather than as a
    /// twelve-segment absolute path. The anchor is not free-standing: it is
    /// `AuditScope::Global`'s other half, and swapping it for any other
    /// directory turns every path in the report absolute.
    #[test]
    fn global_findings_are_anchored_on_the_home_directory() {
        let s = Sandbox::new();
        s.write(
            "home/.claude/skills/mine-heavy/SKILL.md",
            &heavy_skill("mine-heavy"),
        );

        let out = s.run(&["--global"]);
        let line = out.ran().line_with("R01", &["mine-heavy"]);
        assert!(
            line.contains(".claude/skills/mine-heavy/SKILL.md"),
            "expected a home-relative path, got: {line}"
        );
        let home = s.home().display().to_string();
        assert!(
            !line.contains(&home),
            "the path must be relative to $HOME ({home}), not absolute: {line}"
        );
    }

    /// Guard on the isolation itself: with `HOME` pointed at an empty
    /// sandbox, the run must find nothing. A green suite whose sandbox leaked
    /// into the developer's real `~/.claude` would prove nothing at all — and
    /// this is the assertion that fails if `HOME` ever stops being honoured.
    #[test]
    fn the_sandbox_home_is_what_the_global_scope_reads() {
        let s = Sandbox::new();
        let out = s.run(&["--global"]);
        assert!(
            !out.stdout.contains("R01") && !out.stdout.contains("R04"),
            "the real home library must be unreachable from a sandboxed run:\n{}",
            out.stdout
        );
        assert!(!Path::new(&s.home().join(".claude")).exists());
    }
}
