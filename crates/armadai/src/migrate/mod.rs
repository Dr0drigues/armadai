//! `armadai migrate`: a legacy project converted to the canonical `.armadai/` model.

pub mod convert;
pub mod read;
pub mod report;

#[cfg(test)]
mod tests {
    use super::*;

    pub const CASES: [&str; 6] = [
        "armadai-dev",
        "code-analysis-rust",
        "declarative",
        "demo-rust-team",
        "fullstack",
        "rust-dev",
    ];

    fn fixture(case: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/migrate")
            .join(case)
    }

    fn converted(case: &str) -> (armadai_model::Project, report::Report) {
        let _iso = armadai_core::test_support::IsolatedConfigDir::enter();
        let old = read::read_old(&fixture(case)).unwrap();
        let (p, _, r) = convert::convert(&old);
        (p, r)
    }

    #[test]
    fn every_fixture_converts_to_a_valid_project() {
        for case in CASES {
            let (p, r) = converted(case);
            assert!(!p.agents.is_empty(), "{case}");
            assert!(
                !r.warnings.iter().any(|w| w.contains("not migrated")),
                "{case}: {:?}",
                r.warnings
            );
            for (id, a) in &p.agents {
                assert!(!a.front.description.is_empty(), "{case}/{id}");
                assert!(!a.body.trim().is_empty(), "{case}/{id}");
            }
            let dir = tempfile::tempdir().unwrap();
            armadai_model::write_project(&p, dir.path()).unwrap();
            let ds = armadai_model::validate(dir.path());
            assert!(
                !armadai_model::has_errors(&ds),
                "{case}: {}",
                ds.iter().map(|d| d.render()).collect::<Vec<_>>().join("\n")
            );
        }
    }

    #[test]
    fn fullstack_orchestration_omits_the_unknown_agent() {
        let (p, r) = converted("fullstack");
        let f = &p.flows["orchestration"];
        assert!(!f.steps.contains_key("ghost-agent"));
        assert!(f.steps.contains_key("doc-generator"));
        assert!(r.warnings.iter().any(|w| w.contains("`ghost-agent`")));
        assert_eq!(p.manifest.coordinator.as_deref(), Some("code-reviewer"));
        assert_eq!(p.manifest.policy.free_agents, ["doc-generator"]);
    }

    #[test]
    fn declarative_agents_keep_their_composed_prompt_and_description() {
        let (p, _) = converted("declarative");
        let core = &p.agents["core-specialist"];
        assert_eq!(core.front.description, "Owns the domain layer");
        assert_eq!(
            core.body,
            "You are core-specialist, a specialist of this Rust workspace.\n\n\
             You only change files of the `core` module."
        );
        assert_eq!(core.front.tier, Some(armadai_model::Tier::Pro));
        assert_eq!(core.front.tags, ["team"]);
        assert_eq!(p.agents["cli-specialist"].front.temperature, Some(0.3));
        assert!(p.skills.contains_key("review-checklist"));
    }
}
