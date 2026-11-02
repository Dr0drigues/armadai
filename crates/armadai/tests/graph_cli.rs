//! `armadai graph`: the Mermaid of one flow or of every flow, on stdout only.

use std::path::{Path, PathBuf};

use assert_cmd::Command;

const PROJECT: &[(&str, &str)] = &[
    (".armadai/project.yaml", "schema_version: 1\n"),
    (".armadai/agents/dev.md", "---\ndescription: d\n---\nB\n"),
    (".armadai/agents/rev.md", "---\ndescription: r\n---\nB\n"),
    (".armadai/flows/ship.yaml", "sequence: [dev, rev]\n"),
    (
        ".armadai/flows/review.yaml",
        "steps:\n  write: { agent: dev }\n  check: { agent: rev, states: [approved, rejected], on_failure: write }\nlinks:\n  - { from: write, to: check }\n  - { from: check, to: write, when: rejected }\n",
    ),
];

const SHIP: &str = "%% flow: ship
flowchart TD
  n_dev([\"dev\"])
  n_rev[[\"rev\"]]
  n_dev --> n_rev
  classDef startStep stroke-width:3px
  classDef endStep stroke-width:3px,stroke-dasharray:4 2
  class n_dev startStep
  class n_rev endStep
";

const REVIEW: &str = "%% flow: review
flowchart TD
  n_write([\"write (dev)\"])
  n_check[\"check (rev)\"]
  n_write --> n_check
  n_check -->|\"rejected\"| n_write
  n_check -.->|\"failed\"| n_write
  classDef startStep stroke-width:3px
  class n_write startStep
";

struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new(files: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in files {
            let path = dir.path().join("project").join(p);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, c).unwrap();
        }
        std::fs::create_dir_all(dir.path().join("project/.git")).unwrap();
        std::fs::create_dir_all(dir.path().join("config")).unwrap();
        Env { dir }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("project")
    }

    /// `(exit code, stdout, stderr)`
    fn run(&self, cwd: &Path, args: &[&str]) -> (Option<i32>, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(cwd)
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("NO_COLOR", "1")
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

#[test]
fn one_flow_prints_only_its_mermaid() {
    let env = Env::new(PROJECT);
    let (code, out, _) = env.run(&env.root(), &["graph", "review"]);
    assert_eq!(code, Some(0));
    assert_eq!(out, REVIEW);
    let (_, out, _) = env.run(&env.root(), &["graph", "ship", "--format", "mermaid"]);
    assert_eq!(out, SHIP, "a sequence is drawn expanded");
}

#[test]
fn every_flow_is_one_block_each_in_flow_order() {
    let env = Env::new(PROJECT);
    let sub = env.root().join("src");
    std::fs::create_dir_all(&sub).unwrap();
    let (code, out, _) = env.run(&sub, &["graph"]);
    assert_eq!(code, Some(0));
    assert_eq!(out, format!("{REVIEW}\n{SHIP}"));
}

#[test]
fn an_unknown_flow_lists_the_flows() {
    let env = Env::new(PROJECT);
    let (code, out, err) = env.run(&env.root(), &["graph", "shipp"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(
        err.contains("unknown flow `shipp`") && err.contains("review, ship"),
        "{err}"
    );
}

#[test]
fn a_project_without_flows_says_so() {
    let env = Env::new(&PROJECT[..3]);
    let (code, out, err) = env.run(&env.root(), &["graph"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("no flow"), "{err}");
}

#[test]
fn outside_a_project_is_a_clear_error() {
    let env = Env::new(&[]);
    let (code, out, err) = env.run(&env.root(), &["graph"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("armadai init --project"), "{err}");
}

#[test]
fn a_legacy_project_points_to_migrate() {
    let env = Env::new(&[(".armadai/config.yaml", "agents:\n  - name: dev\n")]);
    let (code, out, err) = env.run(&env.root(), &["graph"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("run `armadai migrate`"), "{err}");
}

#[test]
fn an_invalid_project_prints_its_diagnostics_and_fails() {
    let mut files = PROJECT.to_vec();
    files.push((".armadai/flows/bad.yaml", "steps:\n  x: { agent: ghost }\n"));
    let env = Env::new(&files);
    let (code, out, err) = env.run(&env.root(), &["graph", "ship"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert!(err.contains("unknown agent `ghost`"), "{err}");
}

// ---- formats, views and combinations -------------------------------------

/// `zed` sorts after `alpha`: the order is the flow's, not the alphabet's.
const ORDERED: &[(&str, &str)] = &[
    (
        ".armadai/project.yaml",
        "schema_version: 1\ncoordinator: lead\npolicy:\n  delegation: strict\n  free_agents: [Explore]\n",
    ),
    (".armadai/agents/lead.md", "---\ndescription: l\n---\nB\n"),
    (".armadai/agents/dev.md", "---\ndescription: d\n---\nB\n"),
    (".armadai/agents/rev.md", "---\ndescription: r\n---\nB\n"),
    (".armadai/agents/doc.md", "---\ndescription: o\n---\nB\n"),
    (
        ".armadai/flows/ship.yaml",
        "steps:\n  zed: { agent: dev }\n  alpha: { agent: rev }\nlinks:\n  - { from: zed, to: alpha }\n",
    ),
];

const ORDERED_DOT: &str = "// flow: ship
digraph \"ship\" {
  \"zed\" [label=\"zed (dev)\", shape=box, style=\"rounded,bold\"];
  \"alpha\" [label=\"alpha (rev)\", shape=box, peripheries=2];
  \"zed\" -> \"alpha\";
}
";

#[test]
fn one_flow_as_dot() {
    let env = Env::new(ORDERED);
    let (code, out, _) = env.run(&env.root(), &["graph", "ship", "--format", "dot"]);
    assert_eq!(code, Some(0));
    assert_eq!(out, ORDERED_DOT);
}

#[test]
fn one_flow_as_json() {
    let env = Env::new(PROJECT);
    let (code, out, _) = env.run(&env.root(), &["graph", "review", "--format", "json"]);
    assert_eq!(code, Some(0));
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    let want = serde_json::json!({"flows": [{
        "id": "review",
        "nodes": [
            {"id": "write", "agent": "dev", "states": [], "start": true, "end": false},
            {"id": "check", "agent": "rev", "states": ["approved", "rejected"], "start": false, "end": false},
        ],
        "edges": [
            {"from": "write", "to": "check", "when": null, "kind": "link"},
            {"from": "check", "to": "write", "when": "rejected", "kind": "link"},
            {"from": "check", "to": "write", "when": null, "kind": "on_failure"},
        ],
    }]});
    assert_eq!(got, want);
}

#[test]
fn the_flow_order_reaches_the_cli() {
    let env = Env::new(ORDERED);
    let (_, out, _) = env.run(&env.root(), &["graph", "ship"]);
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(lines[2], "  n_zed([\"zed (dev)\"])");
    assert_eq!(lines[3], "  n_alpha[[\"alpha (rev)\"]]");
    assert_eq!(lines[4], "  n_zed --> n_alpha");
}

#[test]
fn every_format_is_byte_stable() {
    let env = Env::new(ORDERED);
    for view in ["flows", "team"] {
        for format in ["mermaid", "dot", "json", "ascii"] {
            let args = ["graph", "--view", view, "--format", format];
            let (c1, a, _) = env.run(&env.root(), &args);
            let (_, b, _) = env.run(&env.root(), &args);
            assert_eq!(c1, Some(0), "{view} {format}");
            assert!(!a.is_empty(), "{view} {format}");
            assert_eq!(a, b, "{view} {format}");
        }
    }
}

#[test]
fn the_team_view() {
    let env = Env::new(ORDERED);
    let (code, out, _) = env.run(&env.root(), &["graph", "--view", "team"]);
    assert_eq!(code, Some(0));
    assert_eq!(
        out,
        "%% team (delegation: strict, enforced by the gate)
flowchart TD
  main([\"main thread (lead)\"])
  n_dev[\"dev · ship\"]
  n_doc[\"doc\"]
  n_rev[\"rev · ship\"]
  subgraph free[\"free agents, callable from anywhere\"]
    n_Explore[\"Explore\"]
  end
  main --> n_dev
  main --> n_rev
  n_dev --> n_rev
"
    );
}

#[test]
fn the_team_view_in_dot_and_json() {
    let env = Env::new(ORDERED);
    let (_, out, _) = env.run(&env.root(), &["graph", "--view", "team", "--format", "dot"]);
    assert_eq!(
        out.lines().next(),
        Some("// team (delegation: strict, enforced by the gate)")
    );
    let (_, out, _) = env.run(
        &env.root(),
        &["graph", "--view", "team", "--format", "json"],
    );
    let got: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(got["coordinator"], "lead");
    assert_eq!(
        got["free_agents"],
        serde_json::json!([{"id": "Explore", "flows": []}])
    );
}

#[test]
fn the_team_view_agrees_with_the_gate() {
    let env = Env::new(ORDERED);
    let (_, out, _) = env.run(
        &env.root(),
        &["graph", "--view", "team", "--format", "json"],
    );
    let g: serde_json::Value = serde_json::from_str(&out).unwrap();
    let gate = |caller: Option<&str>, target: &str| -> String {
        let payload = serde_json::json!({
            "cwd": env.root().to_string_lossy(),
            "tool_name": "Agent",
            "agent_type": caller.unwrap_or(""),
            "tool_input": {"subagent_type": target},
        })
        .to_string();
        let o = Command::cargo_bin("armadai")
            .unwrap()
            .arg("__claude-policy-gate")
            .env("ARMADAI_CONFIG_DIR", env.dir.path().join("config"))
            .env("HOME", env.dir.path())
            .write_stdin(payload)
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    let edges: Vec<(Option<String>, String)> = g["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["from"].as_str().map(String::from),
                e["to"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(!edges.is_empty());
    // every drawn edge is allowed
    for (from, to) in &edges {
        assert_eq!(
            gate(from.as_deref(), to),
            "",
            "drawn {from:?} -> {to} but the gate denies"
        );
    }
    // every undrawn pair is denied, but towards a free agent (allowed from anywhere)
    let free: Vec<&str> = g["free_agents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    let callers: Vec<Option<&str>> = std::iter::once(None)
        .chain(["dev", "rev", "doc"].map(Some))
        .collect();
    for caller in callers {
        for target in ["lead", "dev", "rev", "doc", "Explore"] {
            let drawn = edges
                .iter()
                .any(|(f, t)| f.as_deref() == caller && t == target);
            let allowed = gate(caller, target).is_empty();
            assert_eq!(
                allowed,
                drawn || free.contains(&target),
                "{caller:?} -> {target}"
            );
        }
    }
}

#[test]
fn a_flow_id_with_the_team_view_is_refused() {
    let env = Env::new(PROJECT);
    for format in ["mermaid", "dot", "json", "ascii"] {
        let (code, out, err) = env.run(
            &env.root(),
            &["graph", "review", "--view", "team", "--format", format],
        );
        assert_eq!(code, Some(1), "{format}");
        assert!(out.is_empty(), "{out}");
        assert_eq!(
            err.trim_end(),
            "Error: --view team draws the whole team: drop the flow id review"
        );
    }
}

#[test]
fn a_cycle_is_drawn_as_ascii_with_its_return_links_listed() {
    let env = Env::new(PROJECT);
    let (code, out, err) = env.run(&env.root(), &["graph", "review", "--format", "ascii"]);
    assert_eq!(code, Some(0), "{err}");
    assert!(!out.contains('\x1b'), "{out:?}");
    assert_eq!(
        out,
        "flow: review\n  [▶ write (dev)]\n         │\n         ↓\n   [check (rev)]\n\nlinks not drawn:\n  check → write (rejected)\n  check → write (failed)\n"
    );
    let (code, out, _) = env.run(
        &env.root(),
        &["graph", "--view", "team", "--format", "ascii"],
    );
    assert_eq!(code, Some(0));
    assert_eq!(
        out.lines().next(),
        Some("team (delegation: off, not enforced)")
    );
}

/// The validated `long_about` of `graph`, which opens `graph --help`.
const GRAPH_HELP: &str = "Draw how the project works, on stdout and nothing else: redirect it to a file or pipe it into a renderer.

What to draw (--view):
  flows  each flow's steps and links (default). Name a flow, or omit it to draw every flow.
  team   who may delegate to whom, as the delegation gate reads it. It draws the whole team, so it takes no flow id.

Output format (--format):
  mermaid  flowchart TD (default). Paste it into a Markdown ```mermaid block or a .mmd file.
  dot      Graphviz: armadai graph --format dot | dot -Tsvg > graph.svg
  json     nodes and edges. With --view flows it is always {\"flows\": [...]}, even for one flow; with --view team it is the team object.
  ascii    boxes for the terminal, without colour. A link the drawing cannot show is listed under `links not drawn:`: a link back to an earlier step, a second link between the same two steps, a link to an unknown step, and in the team view a delegation to an agent already drawn.

Several flows are several drawings, each opened by a `%% flow: <id>` comment (`// flow: <id>` in DOT, `flow: <id>` in ASCII) and separated by a blank line: Mermaid and Graphviz render one drawing per document, so take them one at a time. Conditional links are labelled with their state; failure paths (`when: failed`, `on_failure`) are dotted in Mermaid and dashed in DOT.
";

#[test]
fn the_help_is_the_validated_text() {
    let env = Env::new(&[]);
    let (code, out, _) = env.run(&env.root(), &["graph", "--help"]);
    assert_eq!(code, Some(0));
    let head: Vec<&str> = out.lines().take(GRAPH_HELP.lines().count()).collect();
    assert_eq!(head.join("\n") + "\n", GRAPH_HELP);
    assert!(
        out.lines()
            .any(|l| l == "  armadai graph review --format ascii"),
        "{out}"
    );
    let (_, short, _) = env.run(&env.root(), &["graph", "-h"]);
    assert_eq!(
        short.lines().next(),
        Some("Draw a flow, every flow, or the team (Mermaid, DOT, JSON or ASCII)")
    );
}

// ---- --open ----------------------------------------------------------------

impl Env {
    /// `run` with `PATH` set to `path_dir` alone (a fake opener, or nothing).
    fn run_with_path(&self, path_dir: &Path, args: &[&str]) -> (Option<i32>, String, String) {
        let out = Command::cargo_bin("armadai")
            .unwrap()
            .current_dir(self.root())
            .env("ARMADAI_CONFIG_DIR", self.dir.path().join("config"))
            .env("HOME", self.dir.path())
            .env("PATH", path_dir)
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// A directory holding an opener that logs its first argument to `opened.url`.
    #[cfg(unix)]
    fn fake_opener(&self, exit: i32) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let bin = self.dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let log = self.dir.path().join("opened.url");
        let name = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let script = bin.join(name);
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh
printf '%s' \"$1\" > '{}'
exit {exit}
",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        (bin, log)
    }
}

/// The Mermaid carried by a mermaid.live link.
fn decode_url(url: &str) -> String {
    use base64::Engine as _;
    let payload = url
        .strip_prefix("https://mermaid.live/edit#pako:")
        .expect(url);
    let zipped = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .unwrap();
    let json = miniz_oxide::inflate::decompress_to_vec_zlib(&zipped).unwrap();
    let state: serde_json::Value = serde_json::from_slice(&json).unwrap();
    state["code"].as_str().unwrap().to_string()
}

#[cfg(unix)]
#[test]
fn open_hands_the_mermaid_of_the_flow_to_the_opener_and_says_nothing() {
    let env = Env::new(PROJECT);
    let (bin, log) = env.fake_opener(0);
    let (code, out, err) = env.run_with_path(&bin, &["graph", "review", "--open"]);
    assert_eq!((code, out.as_str(), err.as_str()), (Some(0), "", ""));
    assert_eq!(decode_url(&std::fs::read_to_string(log).unwrap()), REVIEW);
}

#[cfg(unix)]
#[test]
fn open_accepts_the_team_view() {
    let env = Env::new(ORDERED);
    let (bin, log) = env.fake_opener(0);
    let (code, out, _) = env.run_with_path(&bin, &["graph", "--view", "team", "--open"]);
    assert_eq!((code, out.as_str()), (Some(0), ""));
    let (_, team, _) = env.run(&env.root(), &["graph", "--view", "team"]);
    assert_eq!(decode_url(&std::fs::read_to_string(log).unwrap()), team);
}

#[cfg(unix)]
#[test]
fn open_without_an_opener_prints_the_url_and_fails() {
    let env = Env::new(PROJECT);
    let empty = env.dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let (code, out, err) = env.run_with_path(&empty, &["graph", "review", "--open"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    let lines: Vec<&str> = err.lines().collect();
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    assert!(
        lines[0].starts_with(&format!("Error: could not open the browser ({cmd}: "))
            && lines[0].ends_with("); open this URL yourself:"),
        "{err}"
    );
    assert_eq!(decode_url(lines[1]), REVIEW);
}

#[test]
fn open_refuses_a_format_other_than_mermaid() {
    let env = Env::new(PROJECT);
    // an empty PATH: a regression must fail here, never reach the real browser
    let empty = env.dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    for format in ["dot", "json", "ascii"] {
        let (code, out, err) =
            env.run_with_path(&empty, &["graph", "review", "--open", "--format", format]);
        assert_eq!(code, Some(1), "{format}");
        assert!(out.is_empty(), "{out}");
        assert_eq!(
            err.trim_end(),
            format!("Error: --open draws Mermaid only; drop --format {format}")
        );
    }
}

#[cfg(unix)]
#[test]
fn open_without_a_name_needs_a_single_flow() {
    let env = Env::new(PROJECT);
    let (bin, log) = env.fake_opener(0);
    let (code, out, err) = env.run_with_path(&bin, &["graph", "--open"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    assert_eq!(
        err.trim_end(),
        "Error: --open draws one diagram: name a flow (flows: review, ship)"
    );
    assert!(!log.exists(), "the opener must not run");
    // a project with one flow: accepted
    let one = Env::new(&PROJECT[..4]);
    let (bin, log) = one.fake_opener(0);
    let (code, _, _) = one.run_with_path(&bin, &["graph", "--open"]);
    assert_eq!(code, Some(0));
    assert_eq!(decode_url(&std::fs::read_to_string(log).unwrap()), SHIP);
}

#[cfg(unix)]
#[test]
fn an_opener_that_fails_gives_the_url_and_exit_1() {
    let env = Env::new(PROJECT);
    let (bin, _) = env.fake_opener(1);
    let (code, out, err) = env.run_with_path(&bin, &["graph", "review", "--open"]);
    assert_eq!(code, Some(1));
    assert!(out.is_empty(), "{out}");
    let lines: Vec<&str> = err.lines().collect();
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    assert_eq!(
        lines[0],
        format!(
            "Error: could not open the browser ({cmd}: exit status: 1); open this URL yourself:"
        )
    );
    assert_eq!(decode_url(lines[1]), REVIEW);
}
