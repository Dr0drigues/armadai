//! Diagnostics of a standards pack (`standards:` of `project.yaml`): a pack that holds
//! skills only. Emitted here, not by the pure validator, because they point at files of
//! a pack snapshot and at its `pack.yaml`.

use std::path::{Path, PathBuf};

use armadai_model::{Diagnostic, Project, Severity};

pub const FOREIGN: &str = "standards-foreign-file";
pub const NO_SLOT: &str = "standards-skill-no-slot";
pub const DUPLICATE: &str = "duplicate-standards-skill";
pub const WRONG_KIND: &str = "standards-wrong-kind";

fn at(
    severity: Severity,
    code: &'static str,
    file: PathBuf,
    message: String,
    hint: &str,
) -> Diagnostic {
    Diagnostic {
        severity,
        code,
        file,
        path: String::new(),
        line: None,
        column: None,
        message,
        hint: Some(hint.into()),
    }
}

/// One error per file of a standards pack that is not under `skills/` (`rels` relative to
/// the pack).
pub fn foreign(pack_dir: &Path, rels: &[PathBuf]) -> Vec<Diagnostic> {
    rels.iter()
        .map(|rel| {
            at(
                Severity::Error,
                FOREIGN,
                pack_dir.join(rel),
                format!(
                    "a standards pack holds only skills: `{}` is not under skills/",
                    rel.display()
                ),
                "move it to a team pack, or remove it",
            )
        })
        .collect()
}

/// One warning per skill of `project` that fills no slot.
pub fn no_slot(pack_dir: &Path, project: &Project) -> Vec<Diagnostic> {
    (project.skills.iter())
        .filter(|(_, s)| armadai_model::filled_by(s).is_empty())
        .map(|(id, _)| {
            at(
                Severity::Warning,
                NO_SLOT,
                pack_dir.join("skills").join(id).join("SKILL.md"),
                format!("skill {id} fills no slot: no agent will receive it from this pack"),
                "add metadata: { armadai-slot: <slot> } to its frontmatter",
            )
        })
        .collect()
}

/// `a` and `b` read `pack <name>` or `standards <name>`, in the order of the layers.
pub fn duplicate(skill: &str, a: &str, b: &str, file: PathBuf) -> Diagnostic {
    at(
        Severity::Error,
        DUPLICATE,
        file,
        format!("skill {skill} comes from both {a} and {b}"),
        "remove one of the two packs, or put your own skills/<id>/ in .armadai/ to choose",
    )
}

pub fn wrong_kind(name: &str, pack_dir: &Path) -> Diagnostic {
    at(
        Severity::Error,
        WRONG_KIND,
        pack_dir.join("pack.yaml"),
        format!(
            "standards {name} is a team pack (its pack.yaml has no kind: standards): \
             reference it with pack:, not standards:"
        ),
        "if it holds only skills, add kind: standards to its pack.yaml",
    )
}
