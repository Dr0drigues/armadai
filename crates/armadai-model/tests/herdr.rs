use armadai_model::*;
use tempfile::TempDir;

fn project(manifest: &str, flow: Option<&str>) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join(".armadai/agents");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::write(a.join("dev.md"), "---\ndescription: dev\n---\nBody\n").unwrap();
    std::fs::write(
        dir.path().join(".armadai/project.yaml"),
        format!("schema_version: 1\n{manifest}"),
    )
    .unwrap();
    if let Some(f) = flow {
        let d = dir.path().join(".armadai/flows");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("lot.yaml"), f).unwrap();
    }
    dir
}

fn manifest(m: &str) -> Manifest {
    let dir = project(m, None);
    let l = load_project(dir.path());
    assert!(!has_errors(&l.diagnostics), "{:?}", l.diagnostics);
    l.project.unwrap().manifest
}

fn diags(m: &str) -> Vec<Diagnostic> {
    validate(project(m, None).path())
}

fn code<'a>(ds: &'a [Diagnostic], c: &str) -> Vec<&'a Diagnostic> {
    ds.iter().filter(|d| d.code == c).collect()
}

fn sess(name: &str, kind: &str) -> HerdrSession {
    HerdrSession {
        name: name.into(),
        kind: kind.into(),
    }
}

#[test]
fn herdr_true_is_enabled_without_sessions() {
    let m = manifest("herdr: true\n");
    assert!(m.herdr_enabled());
    assert_eq!(m.herdr_sessions(), &[]);
}

#[test]
fn herdr_false_and_absent_are_disabled() {
    assert!(!manifest("herdr: false\n").herdr_enabled());
    assert!(!manifest("").herdr_enabled());
}

#[test]
fn herdr_object_keeps_sessions_in_order() {
    let m = manifest(
        "herdr:\n  sessions:\n    - { name: chantier, kind: claude }\n    - { name: relecteur, kind: codex }\n",
    );
    assert!(m.herdr_enabled());
    assert_eq!(
        m.herdr_sessions(),
        &[sess("chantier", "claude"), sess("relecteur", "codex")]
    );
}

#[test]
fn herdr_unknown_key_is_refused_by_name() {
    let dir = project("herdr: { sesions: [] }\n", None);
    let l = load_project(dir.path());
    assert!(has_errors(&l.diagnostics));
    assert!(
        l.diagnostics.iter().any(|d| d.message.contains("sesions")),
        "{:?}",
        l.diagnostics
    );
}

#[test]
fn herdr_wrong_type_is_refused() {
    for v in ["3", "\"yes\""] {
        let dir = project(&format!("herdr: {v}\n"), None);
        assert!(
            has_errors(&load_project(dir.path()).diagnostics),
            "herdr: {v}"
        );
    }
}

#[test]
fn herdr_session_name_bounds() {
    let ok32 = "a".repeat(32);
    let bad = ["", &"a".repeat(33), "Chantier", "chan tier"];
    for n in ["a", ok32.as_str(), "x-y_9"] {
        let ds = diags(&format!(
            "herdr: {{ sessions: [{{ name: \"{n}\", kind: claude }}] }}\n"
        ));
        assert!(
            code(&ds, "invalid-herdr-session-name").is_empty(),
            "{n}: {ds:?}"
        );
    }
    for n in bad {
        let ds = diags(&format!(
            "herdr: {{ sessions: [{{ name: \"{n}\", kind: claude }}] }}\n"
        ));
        let h = code(&ds, "invalid-herdr-session-name");
        assert_eq!(h.len(), 1, "{n:?}: {ds:?}");
        assert_eq!(h[0].severity, Severity::Error);
        assert_eq!(h[0].path, "herdr.sessions[0].name");
        assert_eq!(
            h[0].message,
            format!("herdr session name `{n}` is not 1 to 32 of a-z 0-9 - _")
        );
    }
}

#[test]
fn herdr_duplicate_and_coordinator_names() {
    let ds = diags(
        "herdr:\n  sessions:\n    - { name: chantier, kind: claude }\n    - { name: chantier, kind: codex }\n",
    );
    let h = code(&ds, "duplicate-herdr-session");
    assert_eq!(h.len(), 1, "{ds:?}");
    assert_eq!(h[0].message, "herdr session `chantier` is declared twice");
    assert_eq!(h[0].path, "herdr.sessions[1].name");
    assert_eq!((h[0].line, h[0].column), (Some(5), Some(9)));

    let ds = diags("coordinator: dev\nherdr:\n  sessions:\n    - { name: dev, kind: claude }\n");
    let h = code(&ds, "duplicate-herdr-session");
    assert_eq!(h.len(), 1, "{ds:?}");
    assert_eq!(
        h[0].message,
        "herdr session `dev` has the id of the coordinator"
    );
}

#[test]
fn herdr_unknown_kind() {
    for k in ["claude", "codex", "opencode"] {
        let ds = diags(&format!(
            "herdr: {{ sessions: [{{ name: s, kind: {k} }}] }}\n"
        ));
        assert!(code(&ds, "unknown-herdr-kind").is_empty(), "{k}: {ds:?}");
    }
    let ds = diags("herdr: { sessions: [{ name: s, kind: bash }] }\n");
    let h = code(&ds, "unknown-herdr-kind");
    assert_eq!(h.len(), 1, "{ds:?}");
    assert_eq!(h[0].severity, Severity::Error);
    assert_eq!(h[0].path, "herdr.sessions[0].kind");
    assert_eq!(
        h[0].message,
        "herdr session `s` has kind `bash`: expected claude, codex or opencode"
    );
}

fn link_warnings(manifest: &str, to: &str) -> Vec<Diagnostic> {
    let flow = format!(
        "steps:\n  a: {{ agent: dev }}\nsession_link:\n  to: \"{to}\"\n  when: scope question\n"
    );
    let dir = project(manifest, Some(&flow));
    validate(dir.path())
        .into_iter()
        .filter(|d| d.code == "herdr-session-link-address")
        .collect()
}

#[test]
fn herdr_session_link_address_warns() {
    let w = link_warnings("herdr: true\n", "Chantier Un");
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(w[0].severity, Severity::Warning);
    assert_eq!(
        w[0].message,
        "session_link.to `Chantier Un` is not a Herdr address (a name of 1 to 32 a-z 0-9 - _, or tN): hc send cannot reach it"
    );
    assert_eq!((w[0].line, w[0].column), (Some(4), Some(3)));
    for ok in ["chantier", "t12"] {
        assert!(link_warnings("herdr: true\n", ok).is_empty(), "{ok}");
    }
    // `t` alone is a valid name (a-z), so no warning; the `tN` form only adds nothing new
    assert!(link_warnings("herdr: true\n", "t").is_empty());
    assert_eq!(link_warnings("herdr: true\n", "T12").len(), 1);
    assert!(link_warnings("", "Chantier Un").is_empty());
    assert!(link_warnings("herdr: false\n", "Chantier Un").is_empty());
}

#[test]
fn herdr_writer_round_trip() {
    for (text, want) in [
        ("herdr: true\n", "herdr: true\n"),
        (
            "herdr:\n  sessions:\n    - { name: chantier, kind: claude }\n",
            "herdr:\n  sessions:\n  - name: chantier\n    kind: claude\n",
        ),
    ] {
        let m = manifest(text);
        let out = render_manifest(&m);
        assert!(out.contains(want), "{out}");
        let dir = project(&out.replacen("schema_version: 1\n", "", 1), None);
        let back = load_project(dir.path()).project.unwrap().manifest;
        assert_eq!(back, m);
    }
}

#[test]
fn no_herdr_key_is_written_without_herdr() {
    assert!(!render_manifest(&manifest("")).contains("herdr"));
}
