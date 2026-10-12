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

fn load_ok(root: &Path) -> Project {
    let l = load_project(root);
    assert!(l.diagnostics.is_empty(), "{:?}", l.diagnostics);
    l.project.unwrap()
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
        let a = load_ok(&fx);
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
    write_project(&load_ok(&fixtures()[0]), dir.path()).unwrap();
    let text = std::fs::read_to_string(dir.path().join(".armadai/flows/linear.yaml")).unwrap();
    assert!(!text.contains("sequence"), "{text}");
    assert!(text.contains("steps:") && text.contains("links:"), "{text}");
    assert!(text.contains("second-look"), "{text}");
}

#[test]
fn agent_key_order_follows_the_struct() {
    let a = load_ok(&fixtures()[0]);
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
