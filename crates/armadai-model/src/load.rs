//! Loading of a `.armadai/` directory, accumulating positioned diagnostics.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::SCHEMA_VERSION;
use crate::diag::{Diagnostic, Severity, locate, locate_key};
use crate::types::*;

#[derive(Debug, Clone, PartialEq)]
pub struct Agent {
    pub front: AgentFront,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Skill {
    pub front: SkillFront,
    pub body: String,
}

/// Canonical flow: `sequence` already expanded.
#[derive(Debug, Clone, PartialEq)]
pub struct Flow {
    pub description: Option<String>,
    /// The declared entry step (`start:`), when the flow says where it begins.
    pub start: Option<String>,
    pub steps: BTreeMap<String, Step>,
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub manifest: Manifest,
    pub agents: BTreeMap<String, Agent>,
    pub skills: BTreeMap<String, Skill>,
    pub flows: BTreeMap<String, Flow>,
    pub automations: BTreeMap<String, Automation>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Manifest,
    Agent,
    Skill,
    Flow,
    Automation,
}

/// Source path and text of every loaded file (the manifest is keyed by `"project"`).
#[derive(Debug, Clone, Default)]
pub struct Origins {
    pub files: BTreeMap<(Kind, String), (PathBuf, String)>,
}

#[derive(Debug)]
pub struct Loaded {
    pub project: Option<Project>,
    pub origins: Origins,
    pub diagnostics: Vec<Diagnostic>,
}

/// Split `---\n<yaml>\n---\n<body>`. Returns the yaml, the body and the 1-based
/// line of the first yaml character. Handles `\r\n`.
pub fn split_frontmatter(text: &str) -> Option<(&str, &str, usize)> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.split_inclusive('\n');
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    let start = text.find('\n')? + 1;
    let mut pos = start;
    for line in lines {
        if line.trim_end() == "---" {
            return Some((&text[start..pos], &text[pos + line.len()..], 2));
        }
        pos += line.len();
    }
    None
}

/// (step id, agent) of every sequence item; repeated agents get `-2`, `-3`, ...
fn sequence_ids(items: &[SeqItem]) -> Vec<(String, String)> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    items
        .iter()
        .map(|item| match item {
            SeqItem::Agent(a) => {
                let n = seen.entry(a.as_str()).or_insert(0);
                *n += 1;
                (
                    if *n == 1 {
                        a.clone()
                    } else {
                        format!("{a}-{n}")
                    },
                    a.clone(),
                )
            }
            SeqItem::Explicit(e) => (e.id.clone(), e.agent.clone()),
        })
        .collect()
}

/// Step ids that the sequence defines more than once (they would overwrite each other).
pub fn sequence_duplicates(items: &[SeqItem]) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut dups = Vec::new();
    for (id, _) in sequence_ids(items) {
        if !seen.insert(id.clone()) && !dups.contains(&id) {
            dups.push(id);
        }
    }
    dups
}

/// Expand the `sequence` shortcut into steps and consecutive links.
/// Duplicate ids overwrite: check [`sequence_duplicates`] first.
pub fn expand_sequence(items: &[SeqItem]) -> (BTreeMap<String, Step>, Vec<Link>) {
    let ids = sequence_ids(items);
    let steps = ids
        .iter()
        .map(|(id, agent)| {
            (
                id.clone(),
                Step {
                    agent: agent.clone(),
                    states: Vec::new(),
                    on_failure: None,
                },
            )
        })
        .collect();
    let links = ids
        .windows(2)
        .map(|w| Link {
            from: w[0].0.clone(),
            to: w[1].0.clone(),
            when: None,
        })
        .collect();
    (steps, links)
}

/// Remove every ` at line N, column M` suffix, wherever it sits.
fn strip_positions(msg: &str) -> String {
    let mut out = String::new();
    let mut rest = msg;
    while let Some(i) = rest.find(" at line ") {
        let after = &rest[i + " at line ".len()..];
        let digits = |s: &str| s.bytes().take_while(u8::is_ascii_digit).count();
        let n = digits(after);
        let tail = &after[n..];
        match tail.strip_prefix(", column ") {
            Some(t) if n > 0 && digits(t) > 0 => {
                out.push_str(&rest[..i]);
                rest = &t[digits(t)..];
            }
            _ => {
                out.push_str(&rest[..i + 1]);
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

struct Ctx {
    origins: Origins,
    diagnostics: Vec<Diagnostic>,
}

impl Ctx {
    fn diag(
        &mut self,
        severity: Severity,
        code: &'static str,
        file: &Path,
        pos: (Option<usize>, Option<usize>),
        message: String,
    ) {
        self.diagnostics.push(Diagnostic {
            severity,
            code,
            file: file.to_path_buf(),
            path: String::new(),
            line: pos.0,
            column: pos.1,
            message,
            hint: None,
        });
    }

    fn error(
        &mut self,
        code: &'static str,
        file: &Path,
        pos: (Option<usize>, Option<usize>),
        message: String,
    ) {
        self.diag(Severity::Error, code, file, pos, message);
    }

    /// True for `.yaml`; a `.yml` file is skipped with an `ignored-file` warning.
    fn is_yaml(&mut self, p: &Path) -> bool {
        match p.extension().and_then(|e| e.to_str()) {
            Some("yaml") => true,
            Some("yml") => {
                self.ignored(p, "`.yml` is not loaded, rename it to `.yaml`");
                false
            }
            _ => false,
        }
    }

    fn ignored(&mut self, file: &Path, message: &str) {
        self.diag(
            Severity::Warning,
            "ignored-file",
            file,
            (None, None),
            message.into(),
        );
    }

    /// Read a file, recording origins; I/O and UTF-8 failures become diagnostics.
    fn read(&mut self, kind: Kind, id: &str, path: &Path) -> Option<String> {
        match std::fs::read_to_string(path) {
            Ok(t) => {
                let t = t.trim_start_matches('\u{feff}').to_string();
                self.origins
                    .files
                    .insert((kind, id.to_string()), (path.to_path_buf(), t.clone()));
                Some(t)
            }
            Err(e) => {
                self.error(
                    "invalid-yaml",
                    path,
                    (None, None),
                    format!("cannot read file: {e}"),
                );
                None
            }
        }
    }

    /// Deserialize YAML; `line_offset` shifts positions to the real file line.
    fn parse<T: DeserializeOwned>(
        &mut self,
        yaml: &str,
        file: &Path,
        line_offset: usize,
    ) -> Option<T> {
        if yaml.trim().is_empty() {
            self.error(
                "invalid-yaml",
                file,
                (Some(line_offset + 1), Some(1)),
                "file is empty".into(),
            );
            return None;
        }
        // duplicate keys are rejected everywhere, free maps included (see `crate::yaml`)
        let err = match crate::yaml::from_str(yaml) {
            Ok(v) => return Some(v),
            Err(e) => e,
        };
        let loc = err.location();
        // the position lives in the line/column fields
        let message = strip_positions(&err.to_string());
        self.diagnostics.push(Diagnostic {
            severity: Severity::Error,
            code: "invalid-yaml",
            file: file.to_path_buf(),
            path: String::new(),
            line: loc.as_ref().map(|l| l.line() as usize + line_offset),
            column: loc.as_ref().map(|l| l.column() as usize),
            message,
            hint: None,
        });
        None
    }

    /// Frontmatter + body of a markdown file, with diagnostics on malformed input.
    fn frontmatter<'a>(&mut self, text: &'a str, file: &Path) -> Option<(&'a str, &'a str, usize)> {
        let parts = split_frontmatter(text);
        if parts.is_none() {
            if text
                .trim_start_matches('\u{feff}')
                .lines()
                .next()
                .is_some_and(|l| l.trim_end() == "---")
            {
                self.error(
                    "unclosed-frontmatter",
                    file,
                    (Some(1), Some(1)),
                    "frontmatter is not closed by a second `---`".into(),
                );
            } else {
                self.error(
                    "missing-frontmatter",
                    file,
                    (Some(1), Some(1)),
                    "file must start with a `---` frontmatter".into(),
                );
            }
        }
        parts
    }
}

/// Sorted entries of `dir` (empty when it does not exist).
fn sorted(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Load `<root>/.armadai/`. `project` is `None` when the manifest is absent or invalid.
pub fn load_project(root: &Path) -> Loaded {
    let base = root.join(".armadai");
    let mut cx = Ctx {
        origins: Origins::default(),
        diagnostics: Vec::new(),
    };

    let manifest_path = base.join("project.yaml");
    let manifest = if manifest_path.is_file() {
        cx.read(Kind::Manifest, "project", &manifest_path)
            .and_then(|text| {
                let m: Manifest = cx.parse(&text, &manifest_path, 0)?;
                if m.schema_version != SCHEMA_VERSION {
                    cx.error(
                        "unsupported-schema-version",
                        &manifest_path,
                        {
                            let (l, c) = locate_key(&text, "schema_version");
                            (l.or(Some(1)), c.or(Some(1)))
                        },
                        format!(
                            "schema_version {} is not supported (expected {SCHEMA_VERSION})",
                            m.schema_version
                        ),
                    );
                    return None;
                }
                Some(m)
            })
    } else {
        cx.error(
            "missing-manifest",
            &manifest_path,
            (None, None),
            "`.armadai/project.yaml` not found".into(),
        );
        None
    };

    let mut agents = BTreeMap::new();
    for p in sorted(&base.join("agents")) {
        if p.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let id = stem(&p);
        let Some(text) = cx.read(Kind::Agent, &id, &p) else {
            continue;
        };
        let Some((yaml, body, first)) = cx.frontmatter(&text, &p) else {
            continue;
        };
        if let Some(front) = cx.parse::<AgentFront>(yaml, &p, first - 1) {
            agents.insert(
                id,
                Agent {
                    front,
                    body: body.to_string(),
                },
            );
        }
    }

    let mut skills = BTreeMap::new();
    for dir in sorted(&base.join("skills")) {
        let p = dir.join("SKILL.md");
        if !p.is_file() {
            if dir.is_dir() {
                cx.ignored(&dir, "skill folder without a `SKILL.md`");
            }
            continue;
        }
        let id = dir
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(text) = cx.read(Kind::Skill, &id, &p) else {
            continue;
        };
        let Some((yaml, body, first)) = cx.frontmatter(&text, &p) else {
            continue;
        };
        let Some(front) = cx.parse::<SkillFront>(yaml, &p, first - 1) else {
            continue;
        };
        for k in front.extra.keys() {
            let (l, c) = locate_key(yaml, k);
            cx.diag(
                Severity::Warning,
                "unknown-skill-key",
                &p,
                (l.map(|l| l + first - 1), c),
                format!("unknown skill frontmatter key `{k}`"),
            );
        }
        skills.insert(
            id,
            Skill {
                front,
                body: body.to_string(),
            },
        );
    }

    let mut flows = BTreeMap::new();
    for p in sorted(&base.join("flows")) {
        if !cx.is_yaml(&p) {
            continue;
        }
        let id = stem(&p);
        let Some(text) = cx.read(Kind::Flow, &id, &p) else {
            continue;
        };
        let Some(raw) = cx.parse::<FlowFile>(&text, &p, 0) else {
            continue;
        };
        let has = |k: &str| {
            matches!(crate::yaml::from_str::<BTreeMap<String, serde::de::IgnoredAny>>(&text),
                Ok(m) if m.contains_key(k))
        };
        let (steps, links) = if !has("sequence") {
            (raw.steps, raw.links)
        } else if has("steps") || has("links") {
            cx.error(
                "sequence-and-steps",
                &p,
                locate_key(&text, "sequence"),
                "`sequence` is exclusive with `steps`/`links`".into(),
            );
            continue;
        } else if let Some(dup) = sequence_duplicates(&raw.sequence).first() {
            cx.error(
                "duplicate-step",
                &p,
                locate(&text, dup),
                format!("step id `{dup}` is defined more than once in `sequence`"),
            );
            continue;
        } else {
            expand_sequence(&raw.sequence)
        };
        flows.insert(
            id,
            Flow {
                description: raw.description,
                start: raw.start,
                steps,
                links,
            },
        );
    }

    let mut automations = BTreeMap::new();
    for p in sorted(&base.join("automations")) {
        if !cx.is_yaml(&p) {
            continue;
        }
        let id = stem(&p);
        let Some(text) = cx.read(Kind::Automation, &id, &p) else {
            continue;
        };
        if let Some(a) = cx.parse::<Automation>(&text, &p, 0) {
            automations.insert(id, a);
        }
    }

    Loaded {
        project: manifest.map(|manifest| Project {
            manifest,
            agents,
            skills,
            flows,
            automations,
        }),
        origins: cx.origins,
        diagnostics: cx.diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::strip_positions;

    #[test]
    fn strips_every_position_suffix_wherever_it_is() {
        assert_eq!(
            strip_positions("bad value at line 3, column 7"),
            "bad value"
        );
        assert_eq!(
            strip_positions("a at line 1, column 2, then b at line 4, column 5"),
            "a, then b"
        );
        assert_eq!(
            strip_positions("arrived at line speed"),
            "arrived at line speed"
        );
        assert_eq!(
            strip_positions("at line 3, column x"),
            "at line 3, column x"
        );
    }
}
