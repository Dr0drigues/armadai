//! `armadai journal add|show`: the lot journal (see `crate::journal`).

use clap::Subcommand;

use crate::journal::{self, Entry, Kind};

#[derive(Subcommand)]
pub enum JournalCommand {
    /// Append one line to the journal of a lot (or of every lot)
    #[command(after_help = "Examples:\n  \
        armadai journal add --lot menu --kind decision --by lead \"keep pack 1.3.0\"\n  \
        armadai journal add --kind state \"pack 1.4.0 published\"")]
    Add {
        /// Lot name; without it the entry holds for every lot
        #[arg(long)]
        lot: Option<String>,
        /// What the entry is
        #[arg(long, value_enum, default_value_t = Kind::Note)]
        kind: Kind,
        /// Who writes it (a name you choose; not checked)
        #[arg(long)]
        by: Option<String>,
        /// One line, at most 2000 characters
        text: String,
    },
    /// Show the journal of a lot, or of every lot, oldest first
    #[command(after_help = "Examples:\n  \
        armadai journal show --lot menu --last 20\n  \
        armadai journal show --kind decision\n  \
        armadai journal show --json")]
    Show {
        /// Only this lot (exact case), plus the entries that hold for every lot
        #[arg(long)]
        lot: Option<String>,
        /// Only this kind
        #[arg(long, value_enum)]
        kind: Option<Kind>,
        /// How many of the latest entries to show
        #[arg(long, default_value_t = 20)]
        last: usize,
        /// Machine-readable output
        #[arg(long)]
        json: bool,
    },
}

#[derive(serde::Serialize)]
struct Shown<'a> {
    schema: &'static str,
    entries: &'a [Entry],
}

fn local(ts: u64) -> String {
    use chrono::TimeZone;
    i64::try_from(ts)
        .ok()
        .and_then(|t| chrono::Local.timestamp_opt(t, 0).single())
        .map_or_else(
            || ts.to_string(),
            |d| d.format("%Y-%m-%d %H:%M").to_string(),
        )
}

/// One line per entry; the lot and author columns are as wide as their longest value.
fn render(entries: &[Entry]) -> String {
    let lot = |e: &Entry| e.lot.clone().unwrap_or_else(|| "(all)".into());
    let by = |e: &Entry| e.by.clone().unwrap_or_else(|| "-".into());
    let wl = entries.iter().map(|e| lot(e).len()).max().unwrap_or(0);
    let wb = entries.iter().map(|e| by(e).len()).max().unwrap_or(0);
    entries
        .iter()
        .map(|e| {
            format!(
                "{}  {:<wl$}  {:<8}  {:<wb$}  {}\n",
                local(e.ts),
                lot(e),
                e.kind,
                by(e),
                e.text
            )
        })
        .collect()
}

pub fn execute(what: JournalCommand) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let Some(root) = crate::claude_adapter::policy_gate::resolve_project(&cwd) else {
        crate::compile::find_project(&cwd)?;
        anyhow::bail!("no ArmadAI project found (.armadai/project.yaml)");
    };
    match what {
        JournalCommand::Add {
            lot,
            kind,
            by,
            text,
        } => {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_secs();
            journal::add(&root, lot.as_deref(), kind, by.as_deref(), &text, now)?;
            match lot {
                Some(l) => println!("added to the journal of lot {l}"),
                None => println!("added to the journal of every lot"),
            }
        }
        JournalCommand::Show {
            lot,
            kind,
            last,
            json,
        } => {
            let got = journal::show(&root, lot.as_deref(), kind.map(Kind::as_str), last)?;
            if json {
                let shown = Shown {
                    schema: journal::SCHEMA,
                    entries: &got,
                };
                println!("{}", serde_json::to_string(&shown)?);
            } else {
                print!("{}", render(&got));
            }
        }
    }
    Ok(())
}
