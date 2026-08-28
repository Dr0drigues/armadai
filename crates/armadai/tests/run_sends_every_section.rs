//! Black-box regressions for #395: an agent declares up to four sections
//! (`## System Prompt`, `## Instructions`, `## Output Format`, `## Context`)
//! and `armadai link` writes all four into every native config it generates,
//! while `armadai run` used to hand the provider `agent.system_prompt` and
//! nothing else. The same agent therefore behaved differently depending on
//! which path executed it, and nothing said so.
//!
//! These are wiring tests on purpose. A unit test on the composition function
//! proves the string that function builds; it cannot prove that string is what
//! a provider receives. Each test below runs the real binary against a
//! `provider: cli` agent whose command appends the exact argument it was
//! handed to a capture log — hermetic, no key, no network — so the log *is*
//! the prompt that was sent, whatever the engine later does with the reply.
//! (Reading it back off stdout would not do: the orchestrated engines parse
//! the reply and only surface the fragment they extracted from it.)
//!
//! Measured on `master` before the fix, on every path below: only
//! `SYSPROMPT_MARKER_*` ever reached the provider; the three other sections
//! never did.

#[cfg(test)]
mod tests {
    use assert_cmd::Command;
    use std::path::{Path, PathBuf};

    /// Separates two captured invocations in the log. A token nothing the
    /// product emits could produce.
    const CAPTURE_SEP: &str = "\n<<<ARMADAI_PROMPT_CAPTURE>>>\n";

    /// Isolate `~/.config/armadai` per project: `link`/`run` register the
    /// project and scan the global agent library, which on a developer machine
    /// holds real agents that would shadow the fixture.
    fn isolated_config(dir: &Path) -> PathBuf {
        let config = dir.join("config");
        std::fs::create_dir_all(&config).unwrap();
        config
    }

    /// A four-section agent whose markers are tokens nothing else in the repo
    /// produces, so an assertion on one cannot be satisfied by boilerplate a
    /// linker or an engine emits anyway.
    ///
    /// `sh <script> <input>`: the CLI provider appends the composed input as
    /// the last argument, so `$1` inside the script is verbatim what the
    /// provider sent. Invoked via `sh <path>` rather than as an executable so
    /// no chmod is needed.
    fn four_section_agent(name: &str, script: &Path) -> String {
        let up = marker_suffix(name);
        format!(
            "# {name}\n\n\
             ## Metadata\n\
             - provider: cli\n\
             - command: sh\n\
             - args: [{script}]\n\n\
             ## System Prompt\n\n\
             SYSPROMPT_MARKER_{up} is the system prompt body.\n\n\
             ## Instructions\n\n\
             INSTRUCTIONS_MARKER_{up} is the instructions body.\n\n\
             ## Output Format\n\n\
             OUTPUTFORMAT_MARKER_{up} is the output format body.\n\n\
             ## Context\n\n\
             CONTEXT_MARKER_{up} is the context body.\n",
            script = script.display()
        )
    }

    fn marker_suffix(name: &str) -> String {
        name.to_uppercase().replace('-', "_")
    }

    /// Every marker of `name`'s four sections, in declaration order.
    fn markers(name: &str) -> [String; 4] {
        let up = marker_suffix(name);
        [
            format!("SYSPROMPT_MARKER_{up}"),
            format!("INSTRUCTIONS_MARKER_{up}"),
            format!("OUTPUTFORMAT_MARKER_{up}"),
            format!("CONTEXT_MARKER_{up}"),
        ]
    }

    /// A temp project holding `names`, each a four-section agent whose
    /// provider command records what it was sent into `capture_log()`.
    struct Fixture {
        dir: tempfile::TempDir,
    }

    impl Fixture {
        /// `fail_first`: the probe exits non-zero on its very first invocation
        /// and behaves normally afterwards — the only way to leave a run in
        /// the `Running` state `--resume` requires.
        fn new(names: &[&str], fail_first: bool) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("project");
            std::fs::create_dir_all(root.join("agents")).unwrap();
            std::fs::create_dir_all(root.join(".armadai")).unwrap();

            let log = dir.path().join("sent.log");
            let armed = dir.path().join("armed");
            // Records what it was sent, then answers a fixed harmless string.
            // It must NOT echo the prompt back: the hierarchical coordinator's
            // enriched prompt contains the literal `@agent-name:` delegation
            // syntax, and an echo would be parsed as a delegation to an agent
            // by that name, failing the run before the test can measure it.
            let capture = format!(
                "printf '%s{sep}' \"$1\" >> '{log}'\nprintf 'PROBE_REPLY'\n",
                sep = CAPTURE_SEP.escape_default(),
                log = log.display(),
            );
            let script_body = if fail_first {
                format!(
                    "if [ -f '{armed}' ]; then\n{capture}else\n  : > '{armed}'\n  exit 1\nfi\n",
                    armed = armed.display(),
                )
            } else {
                capture
            };
            let script = root.join("probe.sh");
            std::fs::write(&script, script_body).unwrap();

            let mut config = String::from("agents:\n");
            for name in names {
                std::fs::write(
                    root.join(format!("agents/{name}.md")),
                    four_section_agent(name, &script),
                )
                .unwrap();
                config.push_str(&format!("  - path: agents/{name}.md\n"));
            }
            std::fs::write(root.join(".armadai/config.yaml"), config).unwrap();
            Self { dir }
        }

        fn root(&self) -> PathBuf {
            self.dir.path().join("project")
        }

        /// Append `yaml` to the project config (an `orchestration:` block, to
        /// reach the patterns `--orchestrate` cannot name).
        fn append_config(&self, yaml: &str) {
            let path = self.root().join(".armadai/config.yaml");
            let mut content = std::fs::read_to_string(&path).unwrap();
            content.push_str(yaml);
            std::fs::write(&path, content).unwrap();
        }

        /// `armadai <args>` in the project, isolated from the developer's
        /// config AND data dirs.
        ///
        /// `run` records into SQLite, and `db.rs`'s `#[cfg(test)]` guard does
        /// not protect a *spawned* binary: without `XDG_DATA_HOME` the test
        /// writes into the real `~/.local/share/armadai/armadai.sqlite`,
        /// prompt in clear (measured during #392 — the `runs` table grew by
        /// one row).
        fn armadai(&self, args: &[&str]) -> std::process::Output {
            let mut cmd = Command::cargo_bin("armadai").unwrap();
            cmd.current_dir(self.root())
                .env("ARMADAI_CONFIG_DIR", isolated_config(self.dir.path()))
                .env("XDG_DATA_HOME", self.dir.path().join("data"))
                .args(args);
            cmd.output().unwrap()
        }

        /// Every prompt the CLI provider was handed, in call order.
        ///
        /// `CliProvider::compose_input` wraps the system prompt as
        /// `<system>\n{prompt}\n</system>\n\n{message}`, so what sits between
        /// the tags is byte-for-byte what the run handed the provider.
        fn sent_prompts(&self) -> Vec<String> {
            let log = std::fs::read_to_string(self.dir.path().join("sent.log")).unwrap_or_default();
            log.split(CAPTURE_SEP)
                .filter(|r| !r.trim().is_empty())
                .filter_map(|record| {
                    let after = record.strip_prefix("<system>\n")?;
                    let end = after.find("\n</system>")?;
                    Some(after[..end].to_string())
                })
                .collect()
        }

        /// Everything the provider was sent, concatenated — for "did this
        /// marker ever reach a model" assertions.
        fn all_sent(&self) -> String {
            self.sent_prompts().join("\n")
        }
    }

    fn assert_success(output: &std::process::Output, what: &str) {
        assert!(
            output.status.success(),
            "{what} failed: {}\n--- stdout ---\n{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }

    /// Assert that EVERY prompt the provider received carries all four
    /// sections of whichever agent it belongs to, and that each agent in
    /// `agents` was invoked at least once.
    ///
    /// Per call, not over the concatenation of all calls: measured, an
    /// assertion on the concatenation stays green when only one of an
    /// engine's two `CompletionRequest` sites is fixed (reverting the ring's
    /// circulate site alone, its vote site alone kept the markers present).
    /// That is the exact failure mode #376 cost a whole review.
    fn assert_every_call_carries_every_section(fx: &Fixture, agents: &[&str], what: &str) {
        let sent = fx.sent_prompts();
        assert!(
            !sent.is_empty(),
            "{what}: the probe recorded no provider call at all — the test's own \
             mechanism is broken, not the product."
        );
        let mut called: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        for (i, prompt) in sent.iter().enumerate() {
            let owner = agents
                .iter()
                .copied()
                .find(|a| prompt.contains(markers(a)[0].as_str()))
                .unwrap_or_else(|| {
                    panic!(
                        "{what}: call #{i} carries none of {agents:?}'s system prompts — \
                         the fixture and the assertion disagree.\n--- sent ---\n{prompt}"
                    )
                });
            called.insert(owner);
            for marker in markers(owner) {
                let hits = prompt.matches(marker.as_str()).count();
                assert_eq!(
                    hits, 1,
                    "{what}: call #{i} (agent {owner}) carries {marker} {hits}x, expected \
                     exactly once — 0 means the section was never sent, >1 means it was \
                     composed twice.\n--- sent ---\n{prompt}"
                );
            }
            for heading in ["## Instructions", "## Output Format", "## Context"] {
                assert!(
                    prompt.contains(heading),
                    "{what}: call #{i} (agent {owner}) lost {heading:?} — the bodies \
                     alone, concatenated, lose the structure they rely on.\n\
                     --- sent ---\n{prompt}"
                );
            }
        }
        for agent in agents {
            assert!(
                called.contains(agent),
                "{what}: {agent} was never invoked, so nothing was measured about it \
                 ({} call(s) recorded)",
                sent.len()
            );
        }
    }

    /// The plain single-agent path: `armadai run <agent> <input>`.
    #[test]
    fn run_sends_every_section_to_the_provider() {
        let fx = Fixture::new(&["solo"], false);
        assert_success(&fx.armadai(&["run", "solo", "TASK", "--headless"]), "run");
        assert_every_call_carries_every_section(&fx, &["solo"], "run");
    }

    /// `run` and `link` must compose the sections *identically* — same order,
    /// same separators — otherwise one inconsistency is traded for another,
    /// harder to see. Compared byte for byte on the same agent: the prompt
    /// `run` sent, against the body `link --target claude` wrote.
    #[test]
    fn run_and_link_compose_the_sections_identically() {
        let fx = Fixture::new(&["twin"], false);
        assert_success(&fx.armadai(&["run", "twin", "TASK", "--headless"]), "run");
        let sent = fx.sent_prompts();
        assert_eq!(sent.len(), 1, "expected exactly one provider call");

        assert_success(
            &fx.armadai(&["link", "--target", "claude", "--force"]),
            "link",
        );
        let linked = std::fs::read_to_string(fx.root().join(".claude/agents/twin.md")).unwrap();
        // Drop the leading YAML frontmatter; what follows is the composed body.
        let body = linked
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---\n\n"))
            .map(|(_, body)| body.to_string())
            .unwrap_or_else(|| panic!("no YAML frontmatter in the linked file:\n{linked}"));

        assert_eq!(
            sent[0].trim_end(),
            body.trim_end(),
            "`run` and `link` disagree on how an agent's sections compose.\n\
             --- run sent ---\n{}\n--- link wrote ---\n{body}",
            sent[0]
        );
    }

    /// Guided mode appends a clarifying-questions instruction to the prompt.
    /// It must land *after* every section, on both paths that implement it
    /// (the single-agent ES dispatch and the legacy `--pipe` loop), and it
    /// must not cause a section to be composed twice.
    #[test]
    fn guided_mode_appends_its_instruction_after_every_section() {
        for (extra, invoked) in [
            (Vec::new(), vec!["guided"]),
            (vec!["--pipe", "tail"], vec!["guided", "tail"]),
        ] {
            let fx = Fixture::new(&["guided", "tail"], false);
            let path = fx.root().join("agents/guided.md");
            let agent = std::fs::read_to_string(&path)
                .unwrap()
                .replace("- provider: cli", "- provider: cli\n- mode: guided");
            std::fs::write(&path, agent).unwrap();

            let mut args = vec!["run", "guided", "TASK"];
            args.extend_from_slice(&extra);
            args.push("--headless");
            let what = format!("guided run {extra:?}");
            assert_success(&fx.armadai(&args), &what);
            assert_every_call_carries_every_section(&fx, &invoked, &what);

            let sent = fx.sent_prompts();
            let guided = sent
                .iter()
                .find(|p| p.contains("CONTEXT_MARKER_GUIDED"))
                .unwrap_or_else(|| panic!("{what}: the guided agent was never invoked"));
            let needle = "Before providing your full response";
            let instruction = guided
                .find(needle)
                .unwrap_or_else(|| panic!("{what}: guided instruction missing:\n{guided}"));
            let context = guided.find("CONTEXT_MARKER_GUIDED").unwrap();
            assert!(
                instruction > context,
                "{what}: the guided instruction must come after the last section, \
                 not be buried mid-prompt.\n--- sent ---\n{guided}"
            );
        }
    }

    /// `--pipe` runs on the legacy sequential loop — a different function from
    /// the single-agent path, which builds its own `CompletionRequest`.
    #[test]
    fn pipe_sends_every_section_for_every_link() {
        let fx = Fixture::new(&["first", "second"], false);
        assert_success(
            &fx.armadai(&["run", "first", "TASK", "--pipe", "second", "--headless"]),
            "run --pipe",
        );
        assert_every_call_carries_every_section(&fx, &["first", "second"], "run --pipe");
    }

    /// `--orchestrate` dispatches to the blackboard and ring engines, each of
    /// which builds its own `CompletionRequest` (ring builds two).
    #[test]
    fn orchestrate_sends_every_section_to_the_provider() {
        for pattern in ["blackboard", "ring"] {
            let fx = Fixture::new(&["alpha", "beta"], false);
            assert_success(
                &fx.armadai(&[
                    "run",
                    "alpha",
                    "TASK",
                    "--pipe",
                    "beta",
                    "--orchestrate",
                    pattern,
                    "--headless",
                    "--no-tui",
                ]),
                &format!("run --orchestrate {pattern}"),
            );
            assert_every_call_carries_every_section(
                &fx,
                &["alpha", "beta"],
                &format!("--orchestrate {pattern}"),
            );
        }
    }

    /// The hierarchical engine is reachable only through the project's
    /// `orchestration:` block (`--orchestrate` accepts blackboard/ring only),
    /// and it is the one engine that *enriches* the system prompt with an
    /// orchestration protocol — the four sections must survive that too.
    #[test]
    fn hierarchical_sends_every_section_to_the_coordinator() {
        let fx = Fixture::new(&["lead", "worker"], false);
        fx.append_config(
            "orchestration:\n  \
               enabled: true\n  \
               pattern: hierarchical\n  \
               coordinator: lead\n  \
               teams:\n    \
                 - name: t1\n      \
                   agents: [worker]\n",
        );
        assert_success(
            &fx.armadai(&["run", "lead", "TASK", "--headless", "--no-tui"]),
            "hierarchical run",
        );
        // Only the coordinator is invoked: the probe answers a fixed string
        // with no `@`, so nothing is delegated to `worker`.
        assert_every_call_carries_every_section(&fx, &["lead"], "hierarchical");
        let sent = fx.all_sent();
        // The enrichment must still be there — the fix must add sections, not
        // replace the orchestration protocol with them.
        assert!(
            sent.contains("## Orchestration Protocol"),
            "hierarchical: the orchestration protocol enrichment was lost.\n\
             --- sent ---\n{sent}"
        );
        // …and after them: the protocol describes how to answer, so it must be
        // the last word, not buried between two of the agent's own sections.
        assert!(
            sent.find("## Orchestration Protocol") > sent.find("CONTEXT_MARKER_LEAD"),
            "hierarchical: the orchestration protocol must come after the agent's \
             own sections.\n--- sent ---\n{sent}"
        );
    }

    /// `--resume` reloads the roster from disk and re-enters the same effect
    /// runners. It is a distinct entry point (`execute_resume`/`resume_run`),
    /// so it is measured, not assumed.
    #[cfg(feature = "storage")]
    #[test]
    fn resume_sends_every_section_to_the_provider() {
        let fx = Fixture::new(&["solo"], true);

        // First run: the probe exits 1, leaving the run `Running` — the only
        // state `--resume` accepts.
        let first = fx.armadai(&["run", "solo", "TASK", "--headless", "--json"]);
        assert!(
            !first.status.success(),
            "the fail-first probe was expected to fail the first run"
        );
        let stdout = String::from_utf8_lossy(&first.stdout);
        let run_id = stdout
            .lines()
            .filter(|l| l.contains("\"t\":\"run_start\""))
            .find_map(|l| {
                let after = l.split_once("\"run_id\":\"")?.1;
                Some(after.split_once('"')?.0.to_string())
            })
            .unwrap_or_else(|| panic!("no run_start event in:\n{stdout}"));
        assert!(
            fx.sent_prompts().is_empty(),
            "the failed first call must not have recorded a prompt"
        );

        assert_success(
            &fx.armadai(&["run", "--resume", &run_id, "--headless", "--json"]),
            "run --resume",
        );
        assert_every_call_carries_every_section(&fx, &["solo"], "run --resume");
    }
}
