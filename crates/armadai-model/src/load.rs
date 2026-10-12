//! Loading of a `.armadai/` directory, accumulating positioned diagnostics.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

use crate::SCHEMA_VERSION;
use crate::diag::{Diagnostic, Severity};
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

/// Expand the `sequence` shortcut into steps and consecutive links.
pub fn expand_sequence(items: &[SeqItem]) -> (BTreeMap<String, Step>, Vec<Link>) {
    let mut steps = BTreeMap::new();
    let mut order = Vec::new();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    for item in items {
        let (id, agent) = match item {
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
            SeqItem::Explicit { id, agent } => (id.clone(), agent.clone()),
        };
        steps.insert(
            id.clone(),
            Step {
                agent,
                states: Vec::new(),
                on_failure: None,
            },
        );
        order.push(id);
    }
    let links = order
        .windows(2)
        .map(|w| Link {
            from: w[0].clone(),
            to: w[1].clone(),
            when: None,
        })
        .collect();
    (steps, links)
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
        line: Option<usize>,
        message: String,
    ) {
        self.diagnostics.push(Diagnostic {
            severity,
            code,
            file: file.to_path_buf(),
            path: String::new(),
            line,
            column: None,
            message,
            hint: None,
        });
    }

    fn error(&mut self, code: &'static str, file: &Path, line: Option<usize>, message: String) {
        self.diag(Severity::Error, code, file, line, message);
    }

    /// Read a file, recording origins; I/O and UTF-8 failures become diagnostics.
    fn read(&mut self, kind: Kind, id: &str, path: &Path) -> Option<String> {
        match std::fs::read_to_string(path) {
            Ok(t) => {
                self.origins
                    .files
                    .insert((kind, id.to_string()), (path.to_path_buf(), t.clone()));
                Some(t)
            }
            Err(e) => {
                self.error("invalid-yaml", path, None, format!("cannot read file: {e}"));
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
            self.error("invalid-yaml", file, None, "file is empty".into());
            return None;
        }
        match serde_yaml_ng::from_str(yaml) {
            Ok(v) => Some(v),
            Err(e) => {
                let loc = e.location();
                self.diagnostics.push(Diagnostic {
                    severity: Severity::Error,
                    code: "invalid-yaml",
                    file: file.to_path_buf(),
                    path: String::new(),
                    line: loc.as_ref().map(|l| l.line() + line_offset),
                    column: loc.as_ref().map(|l| l.column()),
                    message: e.to_string(),
                    hint: None,
                });
                None
            }
        }
    }

    /// Frontmatter + body of a markdown file, with diagnostics on malformed input.
    fn frontmatter<'a>(&mut self, text: &'a str, file: &Path) -> Option<(&'a str, &'a str, usize)> {
        let parts = split_frontmatter(text);
        if parts.is_none() {
            if text.starts_with("---") {
                self.error(
                    "unclosed-frontmatter",
                    file,
                    Some(1),
                    "frontmatter is not closed by a second `---`".into(),
                );
            } else {
                self.error(
                    "missing-frontmatter",
                    file,
                    Some(1),
                    "file must start with a `---` frontmatter".into(),
                );
            }
        }
        parts
    }
}

/// 1-based line of the first line starting with `key:`.
fn key_line(text: &str, key: &str) -> Option<usize> {
    text.lines()
        .position(|l| l.strip_prefix(key).is_some_and(|r| r.starts_with(':')))
        .map(|i| i + 1)
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
                        key_line(&text, "schema_version"),
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
            None,
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
        if let Ok(serde_yaml_ng::Value::Mapping(m)) = serde_yaml_ng::from_str(yaml) {
            const KNOWN: [&str; 5] = [
                "name",
                "description",
                "license",
                "compatibility",
                "metadata",
            ];
            for k in m.keys().filter_map(|k| k.as_str()) {
                if !KNOWN.contains(&k) {
                    cx.diag(
                        Severity::Warning,
                        "unknown-skill-key",
                        &p,
                        key_line(yaml, k).map(|l| l + first - 1),
                        format!("unknown skill frontmatter key `{k}`"),
                    );
                }
            }
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
        if p.extension().is_none_or(|e| e != "yaml") {
            continue;
        }
        let id = stem(&p);
        let Some(text) = cx.read(Kind::Flow, &id, &p) else {
            continue;
        };
        let Some(raw) = cx.parse::<FlowFile>(&text, &p, 0) else {
            continue;
        };
        let (steps, links) = if raw.sequence.is_empty() {
            (raw.steps, raw.links)
        } else if !raw.steps.is_empty() || !raw.links.is_empty() {
            cx.error(
                "sequence-and-steps",
                &p,
                key_line(&text, "sequence"),
                "`sequence` is exclusive with `steps`/`links`".into(),
            );
            continue;
        } else {
            expand_sequence(&raw.sequence)
        };
        flows.insert(
            id,
            Flow {
                description: raw.description,
                steps,
                links,
            },
        );
    }

    let mut automations = BTreeMap::new();
    for p in sorted(&base.join("automations")) {
        if p.extension().is_none_or(|e| e != "yaml") {
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
