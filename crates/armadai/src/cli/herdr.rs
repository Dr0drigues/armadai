//! `armadai herdr watch|open-team`: open ArmadAI panes in Herdr.app. Explicit commands only
//! (never `link`); the app's binaries come from `PATH`, one argument each, no shell.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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
        app(
            &["do", "tab.rename", "--id", &tab, "--name", &s.name],
            &format!("herdr-app do tab.rename {}", s.name),
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
