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
// `project` and `hc` are read by `open-team`.
#[allow(dead_code)]
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
        HerdrCommand::OpenTeam => bail!("armadai herdr open-team is not available yet"),
    }
}
