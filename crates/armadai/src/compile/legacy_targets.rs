//! Codex, OpenCode and Copilot fed from the canonical model through their existing
//! linkers: same output as before step 2 (differences listed in the commit), and a
//! report that says what these targets do not compile yet (step 4).

use std::path::{Path, PathBuf};

use armadai_core::model_resolution::{ModelTier, fallback_model_for_tier};
use armadai_model::{Agent, Project, Tier};

use super::{Artifact, ArtifactKind, Compiler, Plan, Status, read_tree};
use crate::linker::manifest::ProducedBy;
use crate::linker::{LinkAgent, create_linker};

const STEP4: &str = "not supported by this target yet (step 4)";

fn tier(t: Tier) -> ModelTier {
    match t {
        Tier::Fast => ModelTier::Fast,
        Tier::Pro => ModelTier::Pro,
        Tier::Max => ModelTier::Max,
    }
}

/// `native.<target>.model`, else the tier through the static table (never the local
/// models cache: the output must not depend on the machine).
fn model_for(agent: &Agent, target: &str) -> Option<String> {
    let native = agent
        .front
        .native
        .get(target)
        .and_then(|n| n.get("model"))
        .and_then(|m| m.as_str());
    if let Some(m) = native {
        return Some(m.to_string());
    }
    match target {
        // codex always had a model: Pro unless the agent says otherwise
        "codex" => Some(
            fallback_model_for_tier("openai", tier(agent.front.tier.unwrap_or(Tier::Pro))).into(),
        ),
        // the provider legacy agents defaulted to
        "opencode" => agent
            .front
            .tier
            .map(|t| fallback_model_for_tier("anthropic", tier(t)).into()),
        _ => None,
    }
}

/// Every agent of `project` (coordinator included), as the legacy linkers take them.
pub fn link_agents(project: &Project, target: &str) -> Vec<LinkAgent> {
    project
        .agents
        .iter()
        .map(|(id, a)| LinkAgent {
            name: id.clone(),
            // the legacy parser trimmed sections; the linkers add their own newline
            system_prompt: a.body.trim_end().to_string(),
            instructions: None,
            output_format: None,
            context: None,
            description: Some(a.front.description.clone()),
            model: model_for(a, target),
            temperature: a.front.temperature.unwrap_or(0.7),
        })
        .collect()
}

pub struct LegacyCompiler {
    pub target: &'static str,
}

impl Compiler for LegacyCompiler {
    fn compile(&self, project: &Project, root: &Path) -> anyhow::Result<Plan> {
        let t = self.target;
        let linker = create_linker(t)?;
        let mut agents = link_agents(project, t);
        let coordinator = project.manifest.coordinator.as_deref().and_then(|c| {
            let i = agents.iter().position(|a| a.name == c)?;
            Some(agents.remove(i))
        });
        let mut plan = Plan::default();
        let files = linker.generate(&agents, coordinator.as_ref());
        let default_dir = Path::new(linker.default_output_dir());
        for (i, f) in files.into_iter().enumerate() {
            // linkers emit one file per agent, in order, then the coordinator's file
            let (kind, produced_by) = match agents.get(i) {
                Some(a) => (ArtifactKind::Agent, ProducedBy::agent(&a.name)),
                None => (
                    ArtifactKind::Instructions,
                    ProducedBy::coordinator(coordinator.as_ref().map_or(t, |c| c.name.as_str())),
                ),
            };
            plan.artifacts.push(Artifact {
                path: f
                    .path
                    .strip_prefix(default_dir)
                    .unwrap_or(&f.path)
                    .to_path_buf(),
                content: f.content.into_bytes(),
                kind,
                produced_by,
            });
        }
        for id in project.skills.keys() {
            for (rel, content) in read_tree(
                &root.join(".armadai/skills").join(id),
                &root.join(".armadai"),
            )? {
                plan.artifacts.push(Artifact {
                    path: PathBuf::from("skills").join(id).join(rel),
                    content,
                    kind: ArtifactKind::Skill,
                    produced_by: ProducedBy::skill(id),
                });
            }
            plan.report
                .push(t, format!("skill {id}"), Status::Compiled, None);
        }
        plan.artifacts.sort_by(|a, b| a.path.cmp(&b.path));

        let r = &mut plan.report;
        for (id, a) in &project.agents {
            let f = &a.front;
            if Some(id.as_str()) == project.manifest.coordinator.as_deref() {
                r.push(
                    t,
                    format!("agent {id}"),
                    Status::Compiled,
                    Some("coordinator, rendered into the instructions file"),
                );
            } else {
                r.push(t, format!("agent {id}"), Status::Compiled, None);
            }
            if f.capabilities != Default::default() {
                r.push(
                    t,
                    format!("agent {id} capabilities"),
                    Status::NotCompiled,
                    Some(STEP4),
                );
            }
            if !f.skills.is_empty() {
                r.push(
                    t,
                    format!("agent {id} skills"),
                    Status::NotCompiled,
                    Some(STEP4),
                );
            }
            if f.tier.is_some() && t == "copilot" {
                r.push(
                    t,
                    format!("agent {id} tier"),
                    Status::NotCompiled,
                    Some("Copilot agent files carry no model"),
                );
            }
            let native = f.native.get(t).and_then(|n| n.as_mapping());
            if native.is_some_and(|n| n.keys().any(|k| k.as_str() != Some("model"))) {
                r.push(
                    t,
                    format!("agent {id} native"),
                    Status::NotCompiled,
                    Some("only native model is compiled for this target (step 4)"),
                );
            }
        }
        for id in project.flows.keys() {
            r.push(t, format!("flow {id}"), Status::NotCompiled, Some(STEP4));
        }
        for id in project.automations.keys() {
            r.push(
                t,
                format!("automation {id}"),
                Status::NotCompiled,
                Some(STEP4),
            );
        }
        if project.manifest.policy.delegation == armadai_model::Delegation::Strict {
            r.push(
                t,
                "policy delegation=strict",
                Status::NotCompiled,
                Some("no delegation gate for this target (step 4)"),
            );
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGETS: [&str; 3] = ["codex", "copilot", "opencode"];

    fn copy_tree(from: &Path, to: &Path) {
        for e in std::fs::read_dir(from).unwrap() {
            let p = e.unwrap().path();
            let dest = to.join(p.file_name().unwrap());
            if p.is_dir() {
                std::fs::create_dir_all(&dest).unwrap();
                copy_tree(&p, &dest);
            } else {
                std::fs::copy(&p, &dest).unwrap();
            }
        }
    }

    /// The legacy fixture, migrated: the project its old-path goldens describe.
    fn migrated() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/link-targets");
        copy_tree(&fixture, dir.path());
        crate::migrate::run(dir.path(), false, false).unwrap();
        let (p, _) = armadai_model::load_checked(dir.path()).unwrap();
        (dir, p)
    }

    fn render(plan: &Plan) -> String {
        plan.artifacts
            .iter()
            .map(|a| {
                format!(
                    "=== {} ===\n{}\n",
                    a.path.display(),
                    String::from_utf8_lossy(&a.content)
                )
            })
            .collect()
    }

    /// Same files, byte for byte, as the old `link` on the legacy fixture
    /// (`tests/golden/legacy_targets/`, generated by the pre-step-2 binary with an
    /// empty models cache) — here with a models cache that would change the answer
    /// if it were read.
    #[test]
    fn the_legacy_targets_match_the_old_link_output_and_ignore_the_models_cache() {
        let iso = armadai_core::test_support::IsolatedConfigDir::enter();
        let doc = serde_json::json!({
            "fetched_at": "2026-10-12T00:00:00Z",
            "providers": {
                "openai": [{"id": "gpt-5", "name": "gpt-5",
                            "cost": {"input": 1.25, "output": 10.0},
                            "limit": {"context": 128000, "output": 4096}}],
                "anthropic": [{"id": "claude-sonnet-9", "name": "claude-sonnet-9",
                            "cost": {"input": 3.0, "output": 15.0},
                            "limit": {"context": 128000, "output": 4096}}]
            }
        });
        std::fs::write(
            iso.config_dir()
                .join(armadai_core::config::MODELS_CACHE_FILE),
            doc.to_string(),
        )
        .unwrap();
        assert_ne!(
            armadai_core::model_resolution::resolve_model_for_tier("openai", ModelTier::Pro),
            "gpt-4o",
            "the seeded cache must change what a cache reader answers"
        );
        let (dir, p) = migrated();
        for t in TARGETS {
            let plan = LegacyCompiler { target: t }
                .compile(&p, dir.path())
                .unwrap();
            let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("tests/golden/legacy_targets/{t}.txt"));
            // the one listed difference: `migrate` recomposed the coordinator's
            // sections, so its prompt carries the `## Instructions` heading the old
            // root file dropped
            let expected = std::fs::read_to_string(golden).unwrap().replace(
                "team.\n\nLEAD_INSTRUCTIONS_MARKER",
                "team.\n\n## Instructions\n\nLEAD_INSTRUCTIONS_MARKER",
            );
            assert_eq!(render(&plan), expected, "{t}");
        }
    }

    fn line<'a>(plan: &'a Plan, element: &str) -> Option<&'a crate::compile::report::ReportLine> {
        plan.report.lines.iter().find(|l| l.element == element)
    }

    #[test]
    fn what_the_legacy_targets_do_not_compile_is_reported() {
        let (dir, p) = migrated();
        std::fs::create_dir_all(dir.path().join(".armadai/automations")).unwrap();
        std::fs::write(
            dir.path().join(".armadai/automations/fmt.yaml"),
            "event: after-tool\nrun: cargo fmt\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".armadai/agents/qa.md"),
            "---\ndescription: d\ncapabilities:\n  shell: deny\n---\nB\n",
        )
        .unwrap();
        let (p2, _) = armadai_model::load_checked(dir.path()).unwrap();
        drop(p);
        for t in TARGETS {
            let plan = LegacyCompiler { target: t }
                .compile(&p2, dir.path())
                .unwrap();
            for e in [
                "flow orchestration",
                "automation fmt",
                "agent qa capabilities",
            ] {
                let l = line(&plan, e).unwrap_or_else(|| panic!("{t}: no `{e}`"));
                assert_eq!(l.status, Status::NotCompiled, "{t}: {e}");
            }
            assert_eq!(line(&plan, "agent dev").unwrap().status, Status::Compiled);
            assert_eq!(
                line(&plan, "skill review-checklist").unwrap().status,
                Status::Compiled
            );
        }
    }

    #[test]
    fn native_model_wins_and_codex_defaults_to_pro() {
        let (_dir, mut p) = migrated();
        let dev = p.agents.get_mut("dev").unwrap();
        dev.front.tier = None;
        dev.front.native.insert(
            "opencode".into(),
            serde_yaml_ng::from_str("model: openai/gpt-5").unwrap(),
        );
        let model = |t: &str| {
            link_agents(&p, t)
                .into_iter()
                .find(|a| a.name == "dev")
                .unwrap()
                .model
        };
        assert_eq!(model("opencode").as_deref(), Some("openai/gpt-5"));
        assert_eq!(model("codex").as_deref(), Some("gpt-4o"));
        assert_eq!(model("copilot"), None);
    }
}
