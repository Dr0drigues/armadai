//! Family `R` — rightsizing: the cost of the context loaded by default.
//!
//! Where the `A` rules judge one asset's own quality, `R` asks what every
//! invocation pays for. Same shape as every other family: each rule is a pure
//! function of [`AuditContext`], and the reverse pass is what read the disk.

use std::path::Path;

use super::{AuditContext, Finding, Severity};

/// R01 — a `SKILL.md` past the token threshold whose skill directory has no
/// `references/` at all.
///
/// Size alone is a bad signal, measured: across 460 real skills, 46% of those
/// above the p90 have a `references/` directory against 67% below it — large
/// skills are *more* often split. So both conditions are required, which is
/// also what keeps a correctly-structured 41795-word skill out of the report.
///
/// Counterpart of `A05` for skills (`A05` covers agents' `system_prompt`).
/// `A09` validates a skill's structure but never its size.
pub(super) fn r01_oversized_skill(ctx: &AuditContext) -> Vec<Finding> {
    ctx.config
        .skills
        .iter()
        // Anti-cascade, same as A05: a skill that failed to parse is A01/A09's
        // job — one root cause, one finding. No SKILL.md is A09's too, and
        // `source_path` then points at the directory, so a size claim about it
        // would be meaningless.
        .filter(|s| s.issues.is_empty() && s.has_skill_md)
        .filter(|s| s.body_tokens > ctx.settings.skill_token_threshold)
        .filter(|s| !has_references(&s.source_path))
        .map(|s| Finding {
            rule: "R01",
            severity: Severity::Warning,
            file: s.source_path.clone(),
            related: Vec::new(),
            message: format!(
                "skill '{}' is ~{} tokens (threshold {}) and has no references/, so all of it \
                 loads on every invocation",
                s.name, s.body_tokens, ctx.settings.skill_token_threshold
            ),
            suggestion: Some(
                "split the detail into references/ — the Agent Skills standard loads those on \
                 demand, and armadai already installs them (core/skill.rs)"
                    .to_string(),
            ),
        })
        .collect()
}

/// Whether the skill directory holding this `SKILL.md` has a non-empty
/// `references/` **directory**. An empty one is not progressive disclosure,
/// and neither is a plain file that happens to be named `references`:
/// `read_dir` fails on both, which is exactly the answer we want.
///
/// The only filesystem read any *rule* makes (the other one in this tree is
/// `AuditSettings::from_project` loading config), and unavoidable: "is this
/// skill split" is a question about the disk, and the reverse pass cannot
/// answer it without carrying a directory listing it has no other use for.
///
/// A relative `source_path` resolves against the process cwd — measured. That
/// is correct for `armadai audit <relative-path>`, since the reverse pass read
/// the very same relative path moments earlier from the same cwd, and nothing
/// between the two calls moves it. Tests must not rely on it: the cwd is
/// process-global and `IsolatedProjectDir` moves it mid-suite.
fn has_references(skill_md: &Path) -> bool {
    skill_md
        .parent()
        .map(|dir| dir.join("references"))
        .and_then(|refs| std::fs::read_dir(refs).ok())
        .is_some_and(|mut entries| entries.next().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::reverse::{ImportedConfig, ImportedSkill, ParseIssue};
    use crate::audit::rules::{AuditSettings, test_support::skill};

    fn ctx_for<'a>(config: &'a ImportedConfig, settings: &'a AuditSettings) -> AuditContext<'a> {
        AuditContext {
            config,
            settings,
            usage: None,
        }
    }

    /// A skill whose directory really exists under `root`, with no
    /// `references/` unless the caller adds one.
    ///
    /// Every R01 test goes through this rather than through `skill()`'s
    /// relative default path: `has_references` resolves a relative path
    /// against the process cwd, which is shared by the whole test binary and
    /// moved mid-suite by `IsolatedProjectDir`. A fixture on disk makes each
    /// assertion depend on the fixture alone.
    fn skill_on_disk(root: &Path, name: &str, body_tokens: usize) -> ImportedSkill {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = skill(name, body_tokens);
        s.source_path = dir.join("SKILL.md");
        s
    }

    fn config_of(skills: Vec<ImportedSkill>) -> ImportedConfig {
        ImportedConfig {
            skills,
            ..Default::default()
        }
    }

    #[test]
    fn r01_flags_a_big_skill_with_no_references() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_of(vec![skill_on_disk(dir.path(), "heavy", 5000)]);
        let settings = AuditSettings::default();
        let f = r01_oversized_skill(&ctx_for(&config, &settings));
        assert_eq!(f.len(), 1, "expected exactly one finding, got {f:?}");
        assert_eq!(f[0].rule, "R01");
        assert!(
            f[0].message.contains("5000"),
            "the message must carry the measured size: {}",
            f[0].message
        );
        assert!(
            f[0].message.contains("3000"),
            "the message must carry the threshold it was judged against: {}",
            f[0].message
        );
    }

    #[test]
    fn r01_leaves_a_small_skill_alone() {
        let dir = tempfile::tempdir().unwrap();
        // Below the default threshold, and no references/ either — so the size
        // condition is the only thing that can be keeping this one quiet.
        let config = config_of(vec![skill_on_disk(dir.path(), "light", 700)]);
        let settings = AuditSettings::default();
        assert!(r01_oversized_skill(&ctx_for(&config, &settings)).is_empty());
    }

    #[test]
    fn r01_honours_a_lowered_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_of(vec![skill_on_disk(dir.path(), "light", 700)]);
        let settings = AuditSettings {
            skill_token_threshold: 500,
            ..AuditSettings::default()
        };
        let f = r01_oversized_skill(&ctx_for(&config, &settings));
        assert_eq!(f.len(), 1, "expected exactly one finding, got {f:?}");
        assert!(
            f[0].message.contains("threshold 500"),
            "the configured threshold must be the one reported: {}",
            f[0].message
        );
    }

    #[test]
    fn r01_leaves_a_big_but_split_skill_alone() {
        // The `quality-playbook` case: 41795 words with 16 references. Big and
        // correctly structured must not be flagged.
        let dir = tempfile::tempdir().unwrap();
        let s = skill_on_disk(dir.path(), "split", 40_000);
        let refs = dir.path().join("split/references");
        std::fs::create_dir_all(&refs).unwrap();
        std::fs::write(refs.join("detail.md"), "detail").unwrap();

        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert!(
            r01_oversized_skill(&ctx_for(&config, &settings)).is_empty(),
            "a split skill must never be flagged, however big"
        );
    }

    #[test]
    fn r01_treats_an_empty_references_dir_as_no_disclosure() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill_on_disk(dir.path(), "hollow", 5000);
        std::fs::create_dir_all(dir.path().join("hollow/references")).unwrap();

        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert_eq!(
            r01_oversized_skill(&ctx_for(&config, &settings)).len(),
            1,
            "an empty references/ is not progressive disclosure"
        );
    }

    #[test]
    fn r01_treats_a_references_file_as_no_disclosure() {
        let dir = tempfile::tempdir().unwrap();
        let s = skill_on_disk(dir.path(), "decoy", 5000);
        // A regular file, not a directory: nothing here loads on demand.
        std::fs::write(dir.path().join("decoy/references"), "not a dir").unwrap();

        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert_eq!(
            r01_oversized_skill(&ctx_for(&config, &settings)).len(),
            1,
            "a file named references is not progressive disclosure"
        );
    }

    #[test]
    fn r01_does_not_stack_on_an_unparsable_skill() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = skill_on_disk(dir.path(), "broken", 9000);
        s.issues = vec![ParseIssue {
            file: s.source_path.clone(),
            message: "invalid yaml".into(),
        }];
        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert!(
            r01_oversized_skill(&ctx_for(&config, &settings)).is_empty(),
            "A01/A09 own parse failures — one root cause, one finding"
        );
    }

    #[test]
    fn r01_does_not_stack_on_a_skill_without_skill_md() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = skill_on_disk(dir.path(), "headless", 9000);
        s.has_skill_md = false;
        let config = config_of(vec![s]);
        let settings = AuditSettings::default();
        assert!(
            r01_oversized_skill(&ctx_for(&config, &settings)).is_empty(),
            "A09 owns a missing SKILL.md — there is no file to call oversized"
        );
    }

    /// The registry entry is a condition like any other: without this test,
    /// forgetting it leaves every unit test above green while `armadai audit`
    /// never runs R01.
    #[test]
    fn r01_is_wired_into_the_registry() {
        let dir = tempfile::tempdir().unwrap();
        let config = config_of(vec![skill_on_disk(dir.path(), "heavy", 5000)]);
        let settings = AuditSettings::default();
        let findings = crate::audit::rules::run_rules(&ctx_for(&config, &settings));
        let r01: Vec<_> = findings.iter().filter(|f| f.rule == "R01").collect();
        assert_eq!(
            r01.len(),
            1,
            "run_rules must emit R01; all findings: {findings:?}"
        );
    }
}
