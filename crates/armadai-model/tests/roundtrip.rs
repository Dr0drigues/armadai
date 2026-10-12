use std::path::{Path, PathBuf};

use armadai_model::*;

fn fixtures() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/roundtrip");
    let mut v: Vec<_> = std::fs::read_dir(root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    v.sort();
    v
}

fn fixture(name: &str) -> PathBuf {
    fixtures().into_iter().find(|p| p.ends_with(name)).unwrap()
}

fn load_ok(root: &Path) -> Project {
    let l = load_project(root);
    assert!(!has_errors(&l.diagnostics), "{:?}", l.diagnostics);
    l.project.unwrap()
}

/// The writer ends every non-empty body with a newline.
fn normalized(mut p: Project) -> Project {
    let fix = |b: &mut String| {
        if !b.is_empty() && !b.ends_with('\n') {
            b.push('\n');
        }
    };
    p.agents.values_mut().for_each(|a| fix(&mut a.body));
    p.skills.values_mut().for_each(|s| fix(&mut s.body));
    p
}

/// Every file under `<root>/.armadai`, as (relative path, bytes).
fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&root.join(".armadai"), &mut files);
    files.sort();
    files
        .into_iter()
        .map(|p| {
            (
                p.strip_prefix(root).unwrap().to_path_buf(),
                std::fs::read(&p).unwrap(),
            )
        })
        .collect()
}

#[test]
fn roundtrip_is_semantically_stable() {
    let all = fixtures();
    assert!(!all.is_empty());
    for fx in all {
        let a = normalized(load_ok(&fx));
        let dir = tempfile::tempdir().unwrap();
        write_project(&a, dir.path()).unwrap();
        let b = load_ok(dir.path());
        assert_eq!(a, b, "{}", fx.display());
        let ds = validate(dir.path());
        assert!(!has_errors(&ds), "{ds:?}");
    }
}

#[test]
fn write_is_idempotent_byte_for_byte() {
    for fx in fixtures() {
        let a = load_ok(&fx);
        let d1 = tempfile::tempdir().unwrap();
        let written = write_project(&a, d1.path()).unwrap();
        let d2 = tempfile::tempdir().unwrap();
        write_project(&load_ok(d1.path()), d2.path()).unwrap();
        assert_eq!(snapshot(d1.path()), snapshot(d2.path()), "{}", fx.display());
        let mut sorted = written.clone();
        sorted.sort();
        assert_eq!(written, sorted);
        assert!(
            written
                .iter()
                .all(|p| p.starts_with(d1.path().join(".armadai")))
        );
        assert_eq!(written.len(), snapshot(d1.path()).len());
    }
}

#[test]
fn body_without_trailing_newline_gets_one() {
    let agent = Agent {
        front: serde_yaml_ng::from_str("description: d\n").unwrap(),
        body: "no newline".into(),
    };
    assert_eq!(
        render_agent("a", &agent),
        "---\ndescription: d\n---\nno newline\n"
    );
    let agent = Agent {
        body: "kept\n".into(),
        ..agent
    };
    assert_eq!(
        render_agent("a", &agent),
        "---\ndescription: d\n---\nkept\n"
    );
}

#[test]
fn sequence_flow_is_written_expanded() {
    let dir = tempfile::tempdir().unwrap();
    write_project(&load_ok(&fixture("full")), dir.path()).unwrap();
    let text = std::fs::read_to_string(dir.path().join(".armadai/flows/linear.yaml")).unwrap();
    assert!(!text.contains("sequence"), "{text}");
    assert!(text.contains("steps:") && text.contains("links:"), "{text}");
    assert!(text.contains("second-look"), "{text}");
}

#[test]
fn agent_key_order_follows_the_struct() {
    let a = load_ok(&fixture("full"));
    let out = render_agent("developpeur", &a.agents["developpeur"]);
    let pos = |k: &str| {
        out.find(&format!("\n{k}:"))
            .unwrap_or_else(|| panic!("{k} in {out}"))
    };
    assert!(out.starts_with("---\ndescription:"));
    assert!(pos("title") < pos("tier") && pos("tier") < pos("temperature"));
    assert!(pos("temperature") < pos("tags") && pos("skills") < pos("capabilities"));
    assert!(pos("capabilities") < pos("native"));
}

#[test]
fn unknown_skill_keys_survive_the_round_trip() {
    let edge = fixture("edge");
    let a = load_ok(&edge);
    let out = render_skill(&a.skills["tooling"]);
    assert!(out.contains("allowed-tools: Bash"), "{out}");
    assert!(out.contains("x-custom:"), "{out}");
    assert!(out.contains("compatibility:"), "{out}");
}

#[test]
fn frontmatter_with_fence_lookalikes_is_not_cut_short() {
    let edge = fixture("edge");
    let a = load_ok(&edge);
    assert_eq!(
        a.agents["edge"].front.description,
        "Line one\n---\nLine three\n"
    );
    assert_eq!(
        a.agents["edge"].body,
        "Intro\n---\nA rule above and below\n---\nEnd\n"
    );
}

#[test]
fn write_refuses_ids_that_are_not_safe_file_names() {
    let mut p = load_ok(&fixture("full"));
    let agent = p.agents["lead"].clone();
    for bad in ["../evil", "a/b", "..", "Dev", ""] {
        let mut q = p.clone();
        q.agents.insert(bad.to_string(), agent.clone());
        let dir = tempfile::tempdir().unwrap();
        let e = write_project(&q, dir.path()).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput, "{bad}");
        assert!(
            !dir.path().join(".armadai").exists(),
            "{bad}: wrote before refusing"
        );
    }
    p.agents.clear();
    p.flows.clear();
    let dir = tempfile::tempdir().unwrap();
    write_project(&p, dir.path()).unwrap();
}
