//! `armadai herdr watch|open-team`: open ArmadAI panes in Herdr.app. Explicit commands only
//! (never `link`); the app's binaries come from `PATH`, one argument each, no shell.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, bail};
use clap::Subcommand;

use crate::herdr::{HC, HERDR_APP, find_bin, terminal_id};

#[derive(Subcommand)]
pub enum HerdrCommand {
    /// Open armadai flow status --watch in a new pane below
    Watch,
    /// Open a space with the coordinator, flow status and the sessions declared under herdr
    OpenTeam,
}

/// What a Herdr command needs, checked once: shared with `open-team`.
pub(crate) struct Ready {
    pub root: PathBuf,
    pub project: armadai_model::Project,
    pub herdr_app: PathBuf,
    pub hc: PathBuf,
}

/// find_project + load_checked + herdr_enabled + terminal_id + both binaries, in that order.
pub(crate) fn guard() -> anyhow::Result<Ready> {
    let root = crate::compile::find_project(&std::env::current_dir()?)?;
    let view = crate::pack::effective::open(&root, crate::pack::effective::Vars::Literal)?;
    let project = match view.load_checked() {
        Ok((p, _)) => p,
        Err(ds) => {
            for d in &ds {
                let s = crate::cli::style::warn();
                anstream::eprintln!("{s}{}{s:#}", d.render());
            }
            bail!("the project is invalid; run `armadai validate`")
        }
    };
    if !project.manifest.herdr_enabled() {
        bail!("herdr is not enabled for this project: add herdr: true to .armadai/project.yaml");
    }
    if terminal_id().is_none() {
        bail!(
            "not in a Herdr terminal (HC_TERMINAL_ID is not set): run this command in a terminal of Herdr.app"
        );
    }
    let bin = |name: &str| {
        find_bin(name).with_context(|| {
            format!(
                "{name} not found in PATH: Herdr.app installs hc and herdr-app in ~/.local/bin, put that folder in PATH"
            )
        })
    };
    let herdr_app = bin(HERDR_APP)?;
    let hc = bin(HC)?;
    Ok(Ready {
        root,
        project,
        herdr_app,
        hc,
    })
}

/// The text of a failure, in the order of the measured forms (plan M2, M3, M5): an `error`
/// string on stdout, `error.message` on stderr, the first line of stderr, the exit status.
fn reason(out: &std::process::Output) -> String {
    let err_of = |bytes: &[u8]| -> Option<String> {
        let v: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        let e = v.get("error")?;
        e.as_str()
            .or_else(|| e.get("message")?.as_str())
            .map(str::to_string)
    };
    let stderr = String::from_utf8_lossy(&out.stderr);
    err_of(&out.stdout)
        .or_else(|| err_of(&out.stderr))
        .or_else(|| {
            stderr
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| exit_reason(out.status))
}

/// `exit status {code}`, else `killed by signal {n}`, else `exit status unknown` (plan 3.3).
fn exit_reason(status: std::process::ExitStatus) -> String {
    if let Some(c) = status.code() {
        return format!("exit status {c}");
    }
    #[cfg(unix)]
    if let Some(n) = std::os::unix::process::ExitStatusExt::signal(&status) {
        return format!("killed by signal {n}");
    }
    "exit status unknown".to_string()
}

/// Run one call and return its stdout; a failure becomes `{what} failed: {reason}`.
/// No delay: the user ran the command, and `herdr-app do` hands control back.
pub(crate) fn call(bin: &Path, args: &[String], what: &str) -> anyhow::Result<String> {
    let out = Command::new(bin)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| anyhow::anyhow!("{what} failed: {e}"))?;
    if !out.status.success() {
        bail!("{what} failed: {}", reason(&out));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn execute(what: HerdrCommand) -> anyhow::Result<()> {
    let ready = guard()?;
    match what {
        HerdrCommand::Watch => {
            let argv = serde_json::to_string(&["armadai", "flow", "status", "--watch"])?;
            let args =
                ["do", "pane.split", "--direction", "down", "--argv", &argv].map(String::from);
            call(&ready.herdr_app, &args, "herdr-app do pane.split")?;
            println!("opened flow status --watch in a new Herdr pane");
            Ok(())
        }
        HerdrCommand::OpenTeam => open_team(&ready),
    }
}

/// The coordinator program: the first effective target that runs in Herdr (plan 2.2).
fn coordinator_kind(targets: &[String]) -> Option<&'static str> {
    targets
        .iter()
        .find_map(|t| ["claude", "codex", "opencode"].into_iter().find(|k| k == t))
}

/// A tab id of our own, unique per session in a fresh space (plan 2.4: `hc run` creates the tab).
fn tab_id(space: &str, name: &str) -> String {
    format!("tb{}-{name}", space.strip_prefix("sp").unwrap_or(space))
}

fn field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_string)
}

fn unexpected(what: &str) -> anyhow::Error {
    anyhow::anyhow!("{what} failed: unexpected output")
}

const TAB_RENAME_ATTEMPTS: usize = 20;
const TAB_RENAME_PAUSE: Duration = Duration::from_millis(100);

/// `hc run --bg` returns before the app has registered the tab: `tab.rename` is called again
/// while the app answers exactly `no tab "<tab>"`, at most `attempts` calls, `pause` between
/// two; any other error stops at once and the last error is returned as it is (plan 2.4).
fn rename_tab(
    app: &Path,
    tab: &str,
    name: &str,
    attempts: usize,
    pause: Duration,
) -> anyhow::Result<()> {
    let args = ["do", "tab.rename", "--id", tab, "--name", name].map(String::from);
    let what = format!("herdr-app do tab.rename {name}");
    let missing = format!("{what} failed: no tab \"{tab}\"");
    for i in 1..=attempts {
        match call(app, &args, &what) {
            Err(e) if i < attempts && e.to_string() == missing => std::thread::sleep(pause),
            r => return r.map(drop),
        }
    }
    Ok(())
}

fn open_team(ready: &Ready) -> anyhow::Result<()> {
    let project = &ready.project;
    let targets = super::link::targets_of(&[], project);
    let Some(kind) = coordinator_kind(&targets) else {
        bail!(
            "no target of this project runs in Herdr (claude, codex or opencode); targets: {}",
            targets.join(", ")
        );
    };
    let root = ready.root.to_string_lossy().into_owned();
    let space_name = project.manifest.name.clone().unwrap_or_else(|| {
        ready
            .root
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    });
    let app = |args: &[&str], what: &str| {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        call(&ready.herdr_app, &args, what)
    };

    // One JSON object, so that no value (the space name is unchecked) is read as an option.
    let new = serde_json::json!({ "name": space_name, "folder": root, "argv": [kind] });
    let what = "herdr-app do space.new";
    let out = app(&["do", "space.new", &new.to_string()], what)?;
    let made: serde_json::Value = serde_json::from_str(&out).map_err(|_| unexpected(what))?;
    let (Some(space), Some(_tab), Some(terminal)) = (
        field(&made, "space"),
        field(&made, "tab"),
        field(&made, "terminal"),
    ) else {
        return Err(unexpected(what));
    };

    // `--pane` wants a pane id, not the terminal id `space.new` returns
    let what = "herdr-app ls";
    let out = app(&["ls"], what)?;
    let listing: serde_json::Value = serde_json::from_str(&out).map_err(|_| unexpected(what))?;
    let pane = listing["spaces"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|s| s["tabs"].as_array().into_iter().flatten())
        .flat_map(|t| t["terminals"].as_array().into_iter().flatten())
        .find(|t| t["terminal"].as_str() == Some(terminal.as_str()))
        .and_then(|t| field(t, "pane"))
        .ok_or_else(|| unexpected(what))?;

    let argv = serde_json::to_string(&["armadai", "flow", "status", "--watch"])?;
    app(
        &[
            "do",
            "pane.split",
            "--direction",
            "down",
            "--pane",
            &pane,
            "--argv",
            &argv,
        ],
        "herdr-app do pane.split",
    )?;

    for s in project.manifest.herdr_sessions() {
        let tab = tab_id(&space, &s.name);
        let args = [
            "run",
            "--bg",
            "--name",
            &s.name,
            "--cwd",
            &root,
            "--no-parent",
            "--label",
            &format!("herdr.space={space}"),
            "--label",
            &format!("herdr.tab={tab}"),
            &s.kind,
        ]
        .map(String::from);
        call(&ready.hc, &args, &format!("hc run {}", s.name))?;
        rename_tab(
            &ready.herdr_app,
            &tab,
            &s.name,
            TAB_RENAME_ATTEMPTS,
            TAB_RENAME_PAUSE,
        )?;
    }

    let sessions = project.manifest.herdr_sessions();
    let width = sessions
        .iter()
        .map(|s| s.name.len())
        .fold("coordinator".len(), usize::max);
    println!("opened Herdr space `{space_name}`");
    println!("  {:<width$}  {kind}", "coordinator");
    println!("  {:<width$}  armadai flow status --watch", "flow status");
    for s in sessions {
        println!("  {:<width$}  {}", s.name, s.kind);
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake `herdr-app` logging each call (NUL-separated, one line per call); `tab.rename`
    /// answers `error` on stdout with exit 1 to the first `n` calls, then `{ "ok": true }`.
    fn fake(dir: &Path, n: usize, error: &str) -> (PathBuf, PathBuf) {
        let (bin, log) = (dir.join("herdr-app"), dir.join("log"));
        let script = format!(
            "#!/bin/sh\n: >> '{l}'\nk=$(wc -l < '{l}')\nprintf '%s\\0' \"$0\" \"$@\" >> '{l}'\nprintf '\\n' >> '{l}'\n\
             if [ \"$k\" -lt {n} ]; then echo '{{ \"error\": \"{error}\" }}'; exit 1; fi\necho '{{ \"ok\": true }}'\n",
            l = log.display()
        );
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        (bin, log)
    }

    fn calls(log: &Path) -> Vec<Vec<String>> {
        std::fs::read_to_string(log)
            .unwrap_or_default()
            .split("\0\n")
            .filter(|c| !c.is_empty())
            .map(|c| c.split('\0').map(str::to_string).collect())
            .collect()
    }

    const MS: Duration = Duration::from_millis(1);

    #[test]
    fn rename_tab_waits_for_the_tab() {
        let d = tempfile::tempdir().unwrap();
        let (bin, log) = fake(d.path(), 3, r#"no tab \"tb01-chantier\""#);
        rename_tab(&bin, "tb01-chantier", "chantier", 20, MS).unwrap();
        let one = vec![
            bin.display().to_string(),
            "do".into(),
            "tab.rename".into(),
            "--id".into(),
            "tb01-chantier".into(),
            "--name".into(),
            "chantier".into(),
        ];
        assert_eq!(calls(&log), vec![one; 4]);
    }

    #[test]
    fn rename_tab_gives_up_with_the_last_error() {
        let d = tempfile::tempdir().unwrap();
        let (bin, log) = fake(d.path(), 25, r#"no tab \"tb01-chantier\""#);
        let t = std::time::Instant::now();
        let e = rename_tab(&bin, "tb01-chantier", "chantier", 20, MS).unwrap_err();
        assert_eq!(
            e.to_string(),
            r#"herdr-app do tab.rename chantier failed: no tab "tb01-chantier""#
        );
        assert_eq!(calls(&log).len(), 20);
        assert!(t.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn rename_tab_stops_on_another_error() {
        let d = tempfile::tempdir().unwrap();
        let (bin, log) = fake(d.path(), 99, r#"no space \"sp01\""#);
        let e = rename_tab(&bin, "tb01-chantier", "chantier", 20, MS).unwrap_err();
        assert_eq!(
            e.to_string(),
            r#"herdr-app do tab.rename chantier failed: no space "sp01""#
        );
        assert_eq!(calls(&log).len(), 1);
        // a `no tab` about another tab does not retry either
        let d = tempfile::tempdir().unwrap();
        let (bin, log) = fake(d.path(), 99, r#"no tab \"tb99-autre\""#);
        let e = rename_tab(&bin, "tb01-chantier", "chantier", 20, MS).unwrap_err();
        assert_eq!(
            e.to_string(),
            r#"herdr-app do tab.rename chantier failed: no tab "tb99-autre""#
        );
        assert_eq!(calls(&log).len(), 1);
    }
}
