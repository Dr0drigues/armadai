//! `armadai grant` and `armadai revoke`: switch the permission levels of
//! `policy.grants` on for a while, and off. The state is
//! `.armadai/state/grants.yaml` (see `claude_adapter::grant_hook`, which reads it at
//! every permission request).
//!
//! The pure parts (`plan_grant`, `render_list`, `plan_revoke`, the duration and time
//! parsing) take the clock and what they read from the settings as arguments, so a test
//! fixes both; `execute_*` only wires them to the project and the disk.
//!
//! A grant never lifts a `deny` (Claude Code evaluates `deny` first, and the hook only
//! answers for a request that would otherwise ask), so a level whose rule is a `deny`
//! rule of the user's settings is refused here instead of being a dead letter.
//! These commands are for the human at the terminal: `link` compiles `deny` rules that
//! stop an agent from running them.

use std::fmt::Display;
use std::path::Path;

use anyhow::{Context, bail};
use armadai_model::Project;
use chrono::{DateTime, Duration, NaiveTime, TimeZone, Utc};
use serde_json::Value;

use crate::claude_adapter::grant_hook::{
    ActiveGrant, GrantState, STATE_FILE, marked_since_in_logs, read_state,
};

/// What ends a level, as the user typed it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Bounds {
    pub until: Option<String>,
    pub span: Option<Duration>,
    pub at: Option<NaiveTime>,
}

/// What `plan_grant` reads from the machine.
pub struct Env<'a, Tz: TimeZone> {
    pub now: DateTime<Tz>,
    /// The flow recording hook that writes the delegation markers is installed.
    pub markers: bool,
    /// `(rule, file)` of every `deny` rule of the user's settings.
    pub denies: &'a [(String, String)],
    /// The command runs in a shell of a Claude Code session (`CLAUDECODE` is set).
    pub claude_shell: bool,
}

/// `30m`, `2h`, `1h30m`, `45s`: hours, minutes and seconds in that order, each once.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let bad = || format!("invalid duration {text}; write 30m, 2h or 1h30m");
    let mut rest = text;
    let mut seconds = 0i64;
    let mut last = 0;
    while !rest.is_empty() {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        let (n, tail) = rest.split_at(digits);
        let unit = tail.chars().next().ok_or_else(bad)?;
        let (rank, mult) = match unit {
            'h' => (1, 3600),
            'm' => (2, 60),
            's' => (3, 1),
            _ => return Err(bad()),
        };
        if n.is_empty() || rank <= last {
            return Err(bad());
        }
        seconds += n.parse::<i64>().map_err(|_| bad())? * mult;
        last = rank;
        rest = &tail[1..];
    }
    if seconds == 0 {
        return Err(bad());
    }
    Ok(Duration::seconds(seconds))
}

/// `HH:MM`.
pub fn parse_time(text: &str) -> Result<NaiveTime, String> {
    NaiveTime::parse_from_str(text, "%H:%M")
        .map_err(|_| format!("invalid time {text}; write it as HH:MM (24 hours, your local time)"))
}

/// `<flow>/<step>` for a step named as `<flow>/<step>` or as a bare id of exactly one flow.
fn qualify(project: &Project, raw: &str) -> Result<String, String> {
    if let Some((flow, step)) = raw.split_once('/') {
        let known = (project.flows.get(flow)).is_some_and(|f| f.steps.contains_key(step));
        return if known {
            Ok(raw.to_string())
        } else {
            Err(format!("{raw} names no step of the flows of the project"))
        };
    }
    let owners: Vec<String> = (project.flows.iter())
        .filter(|(_, f)| f.steps.contains_key(raw))
        .map(|(id, _)| format!("{id}/{raw}"))
        .collect();
    match owners.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(format!("{raw} names no step of the flows of the project")),
        many => Err(format!(
            "{raw} is a step of several flows ({}); write <flow>/<step>",
            many.join(", ")
        )),
    }
}

fn hhmm<Tz: TimeZone>(at: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: Display,
{
    at.with_timezone(tz).format("%H:%M").to_string()
}

/// The next time `time` is on the clock of `now`'s zone: today when it is still ahead,
/// else tomorrow. A time the zone skips (the spring change) is refused, never moved to
/// another day; a time it has twice (the autumn change) is the first one.
fn next_local<Tz: TimeZone>(now: &DateTime<Tz>, time: NaiveTime) -> Result<DateTime<Utc>, String> {
    let tz = now.timezone();
    let missing = || format!("{} does not exist in your time zone", time.format("%H:%M"));
    let today = now.date_naive().and_time(time);
    let at = first(tz.from_local_datetime(&today)).ok_or_else(missing)?;
    if at > *now {
        return Ok(at.with_timezone(&Utc));
    }
    let tomorrow =
        first(tz.from_local_datetime(&(today + Duration::days(1)))).ok_or_else(missing)?;
    Ok(tomorrow.with_timezone(&Utc))
}

/// The earlier instant of a local time: a time the zone has twice comes in an order that
/// depends on the platform, so it is compared, not trusted.
fn first<Tz: TimeZone>(local: chrono::LocalResult<DateTime<Tz>>) -> Option<DateTime<Tz>> {
    match local {
        chrono::LocalResult::Single(t) => Some(t),
        chrono::LocalResult::Ambiguous(a, b) => Some(a.min(b)),
        chrono::LocalResult::None => None,
    }
}

fn rules(n: usize) -> String {
    format!("{n} rule{}", if n == 1 { "" } else { "s" })
}

/// The grant to record and the line to print, or the refusal.
pub fn plan_grant<Tz: TimeZone>(
    project: &Project,
    level: &str,
    bounds: &Bounds,
    env: &Env<'_, Tz>,
) -> Result<(ActiveGrant, String), String>
where
    Tz::Offset: Display,
{
    // `link` denies `Bash(armadai grant *)`, which an agent gets around with a full path,
    // `sh -c` or `env`; Claude Code puts `CLAUDECODE` in every shell it starts (measured
    // 2026-11-10, claude -p, Bash tool). Not a boundary (`env -u CLAUDECODE` lifts it), a
    // guard against the plain forms the deny rule misses.
    if env.claude_shell {
        return Err("grant is for the person at a terminal, and this shell belongs to a Claude Code session (CLAUDECODE is set); run armadai grant in your own terminal".into());
    }
    let levels = &project.manifest.policy.grants;
    let Some(grant) = levels.get(level) else {
        let names: Vec<&str> = levels.keys().map(String::as_str).collect();
        return Err(if names.is_empty() {
            format!(
                "unknown grant level {level}; no level is declared (policy.grants in .armadai/project.yaml)"
            )
        } else {
            format!("unknown grant level {level}; levels: {}", names.join(", "))
        });
    };
    // the bound of the command, else the `until` of the level
    let typed = bounds.until.is_some() || bounds.span.is_some() || bounds.at.is_some();
    let until = if typed {
        bounds.until.as_deref()
    } else {
        grant.until.as_deref()
    };
    if until.is_none() && bounds.span.is_none() && bounds.at.is_none() {
        return Err(format!(
            "grant {level} needs a bound: --until <step>, --for <duration> or --until-time <HH:MM>, or until: in policy.grants"
        ));
    }
    let until_step = until.map(|u| qualify(project, u)).transpose()?;
    if let Some(raw) = until
        && !env.markers
    {
        return Err(format!(
            "--until {raw} needs the delegation markers that the flow recording hook writes; run armadai link --target claude first"
        ));
    }
    for rule in &grant.allow {
        if let Some((_, file)) = env.denies.iter().find(|(r, _)| r == rule) {
            return Err(format!(
                "grant {level} refused: its rule {rule} is a deny rule of {file}, and a grant never lifts a deny"
            ));
        }
    }
    let tz = env.now.timezone();
    let expires_at = if let Some(span) = bounds.span {
        Some(env.now.with_timezone(&Utc) + span)
    } else if let Some(time) = bounds.at {
        Some(next_local(&env.now, time)?)
    } else {
        None
    };
    let n = rules(grant.allow.len());
    let line = match (&expires_at, &until_step) {
        (Some(end), None) => format!(
            "granted {level} until {} ({n}); armadai revoke {level} ends it",
            hhmm(*end, &tz)
        ),
        (None, Some(step)) => {
            format!("granted {level} until a delegation to {step} is recorded ({n})")
        }
        (Some(end), Some(step)) => format!(
            "granted {level} until {} or a delegation to {step} is recorded ({n}); armadai revoke {level} ends it",
            hhmm(*end, &tz)
        ),
        (None, None) => unreachable!("a bound was checked above"),
    };
    let active = ActiveGrant {
        since: env.now.with_timezone(&Utc),
        expires_at,
        until_step,
    };
    Ok((active, line))
}

/// `armadai grant` with no argument.
pub fn render_list<Tz: TimeZone>(
    project: &Project,
    state: &GrantState,
    now: &DateTime<Tz>,
    ended: &dyn Fn(&str, DateTime<Utc>) -> bool,
) -> String
where
    Tz::Offset: Display,
{
    let levels = &project.manifest.policy.grants;
    if levels.is_empty() {
        return "no grant level declared (policy.grants in .armadai/project.yaml)\n".into();
    }
    let tz = now.timezone();
    let width = levels.keys().map(String::len).max().unwrap_or(0);
    let mut out = String::from("grant levels (policy.grants)\n");
    for (name, grant) in levels {
        let status = match state.active.get(name) {
            Some(a)
                if a.expires_at
                    .is_some_and(|end| end <= now.with_timezone(&Utc)) =>
            {
                format!("expired at {}", hhmm(a.expires_at.unwrap_or(a.since), &tz))
            }
            Some(a) if (a.until_step.as_deref()).is_some_and(|s| ended(s, a.since)) => {
                format!(
                    "ended ({} started)",
                    a.until_step.as_deref().unwrap_or_default()
                )
            }
            Some(a) => {
                let ends: Vec<String> = (a.until_step.iter())
                    .map(|s| format!("{s} starts"))
                    .chain(a.expires_at.map(|e| hhmm(e, &tz)))
                    .collect();
                format!(
                    "active until {}, since {}",
                    ends.join(" or "),
                    hhmm(a.since, &tz)
                )
            }
            None => match &grant.description {
                Some(d) => format!("inactive  {d}"),
                None => "inactive".to_string(),
            },
        };
        out += format!("  {name:<width$}  {status}\n").trim_end();
        out.push('\n');
    }
    out
}

/// The state after `revoke` and the line to print. `level` `None` is `--all`.
pub fn plan_revoke(
    state: &GrantState,
    level: Option<&str>,
) -> Result<(GrantState, String), String> {
    let mut next = state.clone();
    let names: Vec<String> = match level {
        Some(l) if state.active.contains_key(l) => vec![l.to_string()],
        Some(l) => return Err(format!("grant {l} is not active")),
        None => state.active.keys().cloned().collect(),
    };
    for n in &names {
        next.active.remove(n);
    }
    let line = match names.as_slice() {
        [] => "no grant is active".to_string(),
        [one] => format!("revoked {one}"),
        many => format!("revoked {} levels: {}", many.len(), many.join(", ")),
    };
    Ok((next, line))
}

/// `(rule, file)` of every `permissions.deny` rule of the settings of the user in this
/// project (`.claude/settings.json`, `.claude/settings.local.json`). A file that is
/// missing or does not parse holds none.
pub fn deny_rules(root: &Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for file in [".claude/settings.json", ".claude/settings.local.json"] {
        let Ok(text) = std::fs::read_to_string(root.join(file)) else {
            continue;
        };
        let Ok(doc) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let deny = doc.pointer("/permissions/deny").and_then(Value::as_array);
        for rule in deny.into_iter().flatten().filter_map(Value::as_str) {
            out.push((rule.to_string(), file.to_string()));
        }
    }
    out
}

/// The flow recording hook of the delegation markers (`PreToolUse` on `Agent|Task`) is
/// in `.claude/settings.json`.
pub fn markers_installed(root: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(root.join(".claude/settings.json")) else {
        return false;
    };
    let Ok(doc) = serde_json::from_str::<Value>(&text) else {
        return false;
    };
    let entries = doc.pointer("/hooks/PreToolUse").and_then(Value::as_array);
    entries.into_iter().flatten().any(|e| {
        e.get("matcher").and_then(Value::as_str) == Some("Agent|Task")
            && (e
                .get("hooks")
                .and_then(Value::as_array)
                .into_iter()
                .flatten())
            .filter_map(|h| h.get("command").and_then(Value::as_str))
            .any(|c| c.contains("__record-flow-event"))
    })
}

/// Write the state atomically: a temporary file beside it, then a rename.
pub fn write_state(root: &Path, state: &GrantState) -> anyhow::Result<()> {
    let path = root.join(STATE_FILE);
    let dir = path.parent().expect("the state file has a folder");
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let text = armadai_model::yaml::to_string(state)?;
    let tmp = dir.join(format!("grants.yaml.tmp-{}", std::process::id()));
    std::fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// The state on disk: empty when there is no file. A file that does not read (damaged, or
/// of a later version) is an error, never silently replaced.
fn load_state(root: &Path) -> anyhow::Result<GrantState> {
    match read_state(root) {
        Some(s) => Ok(s),
        None if root.join(STATE_FILE).exists() => bail!(
            "{STATE_FILE} does not read (damaged, or written by a later version); delete it and run the command again"
        ),
        None => Ok(GrantState::default()),
    }
}

fn open_project() -> anyhow::Result<(std::path::PathBuf, Project)> {
    let cwd = std::env::current_dir()?;
    let Some(root) = crate::claude_adapter::policy_gate::resolve_project(&cwd) else {
        crate::compile::find_project(&cwd)?;
        bail!("no ArmadAI project found (.armadai/project.yaml)");
    };
    let view = crate::pack::effective::open(&root, crate::pack::effective::Vars::Literal)?;
    match view.load_checked() {
        Ok((p, _)) => Ok((root, p)),
        Err(ds) => {
            for d in &ds {
                let s = crate::cli::style::warn();
                anstream::eprintln!("{s}{}{s:#}", d.render());
            }
            bail!("the project is invalid; run `armadai validate`")
        }
    }
}

pub fn execute_grant(
    level: Option<String>,
    until: Option<String>,
    span: Option<String>,
    at: Option<String>,
) -> anyhow::Result<()> {
    let (root, project) = open_project()?;
    let mut state = load_state(&root)?;
    let now = chrono::Local::now();
    let Some(level) = level else {
        let ended = |step: &str, since| marked_since_in_logs(&root, step, since);
        anstream::print!("{}", render_list(&project, &state, &now, &ended));
        return Ok(());
    };
    let bounds = Bounds {
        until,
        span: span
            .as_deref()
            .map(parse_duration)
            .transpose()
            .map_err(anyhow::Error::msg)?,
        at: at
            .as_deref()
            .map(parse_time)
            .transpose()
            .map_err(anyhow::Error::msg)?,
    };
    let denies = deny_rules(&root);
    let env = Env {
        now,
        markers: markers_installed(&root),
        denies: &denies,
        claude_shell: std::env::var_os("CLAUDECODE").is_some_and(|v| !v.is_empty()),
    };
    let (active, line) = plan_grant(&project, &level, &bounds, &env).map_err(anyhow::Error::msg)?;
    state.active.insert(level, active);
    write_state(&root, &state)?;
    anstream::println!("{line}");
    Ok(())
}

pub fn execute_revoke(level: Option<String>) -> anyhow::Result<()> {
    let (root, _) = open_project()?;
    let state = load_state(&root)?;
    let (next, line) = plan_revoke(&state, level.as_deref()).map_err(anyhow::Error::msg)?;
    if next != state {
        write_state(&root, &next)?;
    }
    anstream::println!("{line}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    const MANIFEST: &str = "schema_version: 1\npolicy:\n  grants:\n    auto-until-mr:\n      allow: [\"Bash(git push origin feature/*)\", \"Bash(git commit *)\"]\n      until: publish\n    read-logs:\n      description: Lire les journaux de recette\n      allow: [\"Bash(tail *)\"]\n";

    fn project(manifest: &str) -> Project {
        project_with(manifest, &["evolution"])
    }

    fn project_with(manifest: &str, flows: &[&str]) -> Project {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join(".armadai");
        std::fs::create_dir_all(a.join("agents")).unwrap();
        std::fs::create_dir_all(a.join("flows")).unwrap();
        std::fs::write(a.join("project.yaml"), manifest).unwrap();
        for agent in ["dev", "lead"] {
            std::fs::write(
                a.join(format!("agents/{agent}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
        let flow = "steps:\n  build: { agent: dev }\n  publish: { agent: lead }\nlinks:\n  - { from: build, to: publish }\n";
        for id in flows {
            std::fs::write(a.join(format!("flows/{id}.yaml")), flow).unwrap();
        }
        armadai_model::load_checked(dir.path()).unwrap().0
    }

    /// 2026-11-10 10:00 at UTC+1 (09:00 UTC)
    fn now() -> DateTime<FixedOffset> {
        "2026-11-10T10:00:00+01:00".parse().unwrap()
    }

    fn env(markers: bool, denies: &[(String, String)]) -> Env<'_, FixedOffset> {
        Env {
            now: now(),
            markers,
            denies,
            claude_shell: false,
        }
    }

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn durations_read_hours_minutes_and_seconds() {
        assert_eq!(parse_duration("30m"), Ok(Duration::minutes(30)));
        assert_eq!(parse_duration("2h"), Ok(Duration::hours(2)));
        assert_eq!(parse_duration("1h30m"), Ok(Duration::minutes(90)));
        assert_eq!(parse_duration("45s"), Ok(Duration::seconds(45)));
        for bad in ["", "0m", "30", "m", "1m1h", "1h1h", "2d", "-5m", "1h 30m"] {
            assert_eq!(
                parse_duration(bad),
                Err(format!("invalid duration {bad}; write 30m, 2h or 1h30m")),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_time_of_day_reads_hh_mm() {
        assert_eq!(
            parse_time("18:00"),
            Ok(NaiveTime::from_hms_opt(18, 0, 0).unwrap())
        );
        assert!(parse_time("25:00").is_err());
        assert!(parse_time("6pm").is_err());
    }

    /// T9: the line of each kind of grant, to the letter.
    #[test]
    fn a_grant_says_what_it_did() {
        let p = project(MANIFEST);
        let by_time = Bounds {
            span: Some(Duration::hours(2)),
            ..Bounds::default()
        };
        let (a, line) = plan_grant(&p, "read-logs", &by_time, &env(false, &[])).unwrap();
        assert_eq!(
            line,
            "granted read-logs until 12:00 (1 rule); armadai revoke read-logs ends it"
        );
        assert_eq!(
            a,
            ActiveGrant {
                since: utc("2026-11-10T09:00:00Z"),
                expires_at: Some(utc("2026-11-10T11:00:00Z")),
                until_step: None,
            }
        );
        let (a, line) =
            plan_grant(&p, "auto-until-mr", &Bounds::default(), &env(true, &[])).unwrap();
        assert_eq!(
            line,
            "granted auto-until-mr until a delegation to evolution/publish is recorded (2 rules)"
        );
        assert_eq!(a.until_step.as_deref(), Some("evolution/publish"));
        assert_eq!(a.expires_at, None);
    }

    /// The command's own bound replaces the `until` of the level.
    #[test]
    fn a_bound_typed_replaces_the_level_bound() {
        let p = project(MANIFEST);
        let b = Bounds {
            span: Some(Duration::minutes(30)),
            ..Bounds::default()
        };
        let (a, _) = plan_grant(&p, "auto-until-mr", &b, &env(false, &[])).unwrap();
        assert_eq!(a.until_step, None);
        assert_eq!(a.expires_at, Some(utc("2026-11-10T09:30:00Z")));
    }

    #[test]
    fn until_time_is_today_before_the_hour_and_tomorrow_after() {
        let p = project(MANIFEST);
        let at = |h, m| Bounds {
            at: Some(NaiveTime::from_hms_opt(h, m, 0).unwrap()),
            ..Bounds::default()
        };
        let end = |b: Bounds| {
            plan_grant(&p, "read-logs", &b, &env(false, &[]))
                .unwrap()
                .0
                .expires_at
        };
        // 18:00 at UTC+1 is 17:00 UTC today
        assert_eq!(end(at(18, 0)), Some(utc("2026-11-10T17:00:00Z")));
        // 09:00 has passed (it is 10:00): tomorrow
        assert_eq!(end(at(9, 0)), Some(utc("2026-11-11T08:00:00Z")));
        // the minute itself has passed too
        assert_eq!(end(at(10, 0)), Some(utc("2026-11-11T09:00:00Z")));
        let (_, line) = plan_grant(&p, "read-logs", &at(9, 0), &env(false, &[])).unwrap();
        assert_eq!(
            line,
            "granted read-logs until 09:00 (1 rule); armadai revoke read-logs ends it"
        );
    }

    #[test]
    fn a_grant_is_refused_without_a_bound_for_an_unknown_level_without_markers_or_against_a_deny() {
        let p = project(MANIFEST);
        let err = |level: &str, b: Bounds, markers: bool, denies: &[(String, String)]| {
            plan_grant(&p, level, &b, &env(markers, denies)).unwrap_err()
        };
        assert_eq!(
            err("read-logs", Bounds::default(), true, &[]),
            "grant read-logs needs a bound: --until <step>, --for <duration> or --until-time <HH:MM>, or until: in policy.grants"
        );
        assert_eq!(
            err("x", Bounds::default(), true, &[]),
            "unknown grant level x; levels: auto-until-mr, read-logs"
        );
        assert_eq!(
            err("auto-until-mr", Bounds::default(), false, &[]),
            "--until publish needs the delegation markers that the flow recording hook writes; run armadai link --target claude first"
        );
        let deny = [(
            "Bash(tail *)".to_string(),
            ".claude/settings.local.json".to_string(),
        )];
        let span = Bounds {
            span: Some(Duration::hours(1)),
            ..Bounds::default()
        };
        assert_eq!(
            err("read-logs", span.clone(), true, &deny),
            "grant read-logs refused: its rule Bash(tail *) is a deny rule of .claude/settings.local.json, and a grant never lifts a deny"
        );
        // a deny of another rule does not matter
        let other = [(
            "Bash(rm *)".to_string(),
            ".claude/settings.json".to_string(),
        )];
        assert!(plan_grant(&p, "read-logs", &span, &env(true, &other)).is_ok());
    }

    #[test]
    fn a_step_is_named_in_full_or_by_its_single_owner() {
        let p = project_with(
            &MANIFEST.replace("until: publish", "until: hotfix/publish"),
            &["evolution", "hotfix"],
        );
        let by_step = |s: &str| Bounds {
            until: Some(s.into()),
            ..Bounds::default()
        };
        let got = |s: &str| plan_grant(&p, "read-logs", &by_step(s), &env(true, &[]));
        assert_eq!(
            got("hotfix/publish").unwrap().0.until_step.as_deref(),
            Some("hotfix/publish")
        );
        assert_eq!(
            got("publish").unwrap_err(),
            "publish is a step of several flows (evolution/publish, hotfix/publish); write <flow>/<step>"
        );
        assert_eq!(
            got("nope").unwrap_err(),
            "nope names no step of the flows of the project"
        );
        assert_eq!(
            got("evolution/nope").unwrap_err(),
            "evolution/nope names no step of the flows of the project"
        );
    }

    /// T8
    #[test]
    fn the_list_shows_each_level_and_its_state() {
        let p = project(MANIFEST);
        let mut state = GrantState::default();
        assert_eq!(
            render_list(&p, &state, &now(), &|_, _| false),
            "grant levels (policy.grants)\n  auto-until-mr  inactive\n  read-logs      inactive  Lire les journaux de recette\n"
        );
        state.active.insert(
            "auto-until-mr".into(),
            ActiveGrant {
                since: utc("2026-11-10T08:12:00Z"),
                expires_at: Some(utc("2026-11-10T17:00:00Z")),
                until_step: Some("evolution/publish".into()),
            },
        );
        state.active.insert(
            "read-logs".into(),
            ActiveGrant {
                since: utc("2026-11-10T07:00:00Z"),
                expires_at: Some(utc("2026-11-10T08:30:00Z")),
                until_step: None,
            },
        );
        assert_eq!(
            render_list(&p, &state, &now(), &|_, _| false),
            "grant levels (policy.grants)\n  auto-until-mr  active until evolution/publish starts or 18:00, since 09:12\n  read-logs      expired at 09:30\n"
        );
        state.active.get_mut("read-logs").unwrap().expires_at = Some(utc("2026-11-10T11:00:00Z"));
        state.active.get_mut("read-logs").unwrap().until_step = None;
        assert_eq!(
            render_list(&p, &state, &now(), &|_, _| false)
                .lines()
                .nth(2),
            Some("  read-logs      active until 12:00, since 08:00")
        );
        let none = project("schema_version: 1\n");
        assert_eq!(
            render_list(&none, &GrantState::default(), &now(), &|_, _| false),
            "no grant level declared (policy.grants in .armadai/project.yaml)\n"
        );
    }

    /// T10
    #[test]
    fn revoke_says_what_it_switched_off() {
        let on = |names: &[&str]| GrantState {
            version: 1,
            active: names
                .iter()
                .map(|n| {
                    let a = ActiveGrant {
                        since: utc("2026-11-10T09:00:00Z"),
                        expires_at: Some(utc("2026-11-10T11:00:00Z")),
                        until_step: None,
                    };
                    (n.to_string(), a)
                })
                .collect(),
        };
        let two = on(&["auto-until-mr", "read-logs"]);
        let (next, line) = plan_revoke(&two, Some("read-logs")).unwrap();
        assert_eq!(
            (line.as_str(), next),
            ("revoked read-logs", on(&["auto-until-mr"]))
        );
        let (next, line) = plan_revoke(&two, None).unwrap();
        assert_eq!(
            (line.as_str(), next),
            ("revoked 2 levels: auto-until-mr, read-logs", on(&[]))
        );
        assert_eq!(plan_revoke(&on(&[]), None).unwrap().1, "no grant is active");
        assert_eq!(
            plan_revoke(&two, Some("x")).unwrap_err(),
            "grant x is not active"
        );
    }

    #[test]
    fn the_state_is_written_whole_and_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut state = GrantState::default();
        state.active.insert(
            "read-logs".into(),
            ActiveGrant {
                since: utc("2026-11-10T09:00:00Z"),
                expires_at: Some(utc("2026-11-10T11:00:00Z")),
                until_step: Some("evolution/publish".into()),
            },
        );
        write_state(dir.path(), &state).unwrap();
        assert_eq!(read_state(dir.path()), Some(state));
        let text = std::fs::read_to_string(dir.path().join(STATE_FILE)).unwrap();
        assert!(text.starts_with("version: 1\n"), "{text}");
        // nothing is left beside it
        let left: Vec<_> = std::fs::read_dir(dir.path().join(".armadai/state"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["grants.yaml".to_string()]);
    }

    #[test]
    fn a_state_that_does_not_read_is_refused_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_state(dir.path()).unwrap(), GrantState::default());
        let path = dir.path().join(STATE_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "version: 2\nactive: {}\n").unwrap();
        assert!(load_state(dir.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "version: 2\nactive: {}\n"
        );
    }

    #[test]
    fn the_deny_rules_of_both_settings_files_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let c = dir.path().join(".claude");
        std::fs::create_dir_all(&c).unwrap();
        std::fs::write(
            c.join("settings.json"),
            r#"{"permissions":{"deny":["Bash(a)"],"allow":["Bash(z)"]}}"#,
        )
        .unwrap();
        std::fs::write(
            c.join("settings.local.json"),
            r#"{"permissions":{"deny":["Bash(b)","Edit(/x)"]}}"#,
        )
        .unwrap();
        assert_eq!(
            deny_rules(dir.path()),
            vec![
                ("Bash(a)".to_string(), ".claude/settings.json".to_string()),
                (
                    "Bash(b)".to_string(),
                    ".claude/settings.local.json".to_string()
                ),
                (
                    "Edit(/x)".to_string(),
                    ".claude/settings.local.json".to_string()
                ),
            ]
        );
        assert_eq!(deny_rules(&dir.path().join("nowhere")), vec![]);
    }

    #[test]
    fn the_marker_hook_is_the_agent_task_entry_of_the_recorder() {
        let dir = tempfile::tempdir().unwrap();
        let c = dir.path().join(".claude");
        std::fs::create_dir_all(&c).unwrap();
        let settings = |matcher: &str, command: &str| {
            serde_json::json!({"hooks": {"PreToolUse": [{"matcher": matcher, "hooks": [{"type": "command", "command": command}]}]}}).to_string()
        };
        let installed = |text: String| {
            std::fs::write(c.join("settings.json"), text).unwrap();
            markers_installed(dir.path())
        };
        assert!(installed(settings(
            "Agent|Task",
            "armadai __record-flow-event"
        )));
        // the gate sits on the same matcher, and is not the recorder
        assert!(!installed(settings(
            "Agent|Task",
            "armadai __claude-policy-gate"
        )));
        // the handback recorder is not the marker hook
        assert!(!installed(settings(
            "SubagentHandback",
            "armadai __record-flow-event"
        )));
        assert!(!installed("{}".into()));
        assert!(!markers_installed(&dir.path().join("nowhere")));
    }

    /// Paris in 2027: UTC+1, UTC+2 from 2027-03-28T01:00Z to 2027-10-31T01:00Z. A test
    /// zone, since `chrono-tz` is not in the lock.
    #[derive(Clone, Copy, Debug)]
    struct Paris;

    fn summer(utc: &chrono::NaiveDateTime) -> bool {
        let at = |s: &str| s.parse::<DateTime<Utc>>().unwrap().naive_utc();
        *utc >= at("2027-03-28T01:00:00Z") && *utc < at("2027-10-31T01:00:00Z")
    }

    impl TimeZone for Paris {
        type Offset = FixedOffset;
        fn from_offset(_: &FixedOffset) -> Paris {
            Paris
        }
        fn offset_from_local_date(
            &self,
            d: &chrono::NaiveDate,
        ) -> chrono::LocalResult<FixedOffset> {
            self.offset_from_local_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_local_datetime(
            &self,
            l: &chrono::NaiveDateTime,
        ) -> chrono::LocalResult<FixedOffset> {
            let (cet, cest) = (
                FixedOffset::east_opt(3600).unwrap(),
                FixedOffset::east_opt(7200).unwrap(),
            );
            // an offset is valid when the instant it gives falls in its own season
            let valid = |off: FixedOffset, is_summer: bool| {
                summer(&(*l - Duration::seconds(off.local_minus_utc().into()))) == is_summer
            };
            match (valid(cest, true), valid(cet, false)) {
                // the order of the real `Local` (winter time first): the code must not rely on it
                (true, true) => chrono::LocalResult::Ambiguous(cet, cest),
                (true, false) => chrono::LocalResult::Single(cest),
                (false, true) => chrono::LocalResult::Single(cet),
                (false, false) => chrono::LocalResult::None,
            }
        }
        fn offset_from_utc_date(&self, d: &chrono::NaiveDate) -> FixedOffset {
            self.offset_from_utc_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
        }
        fn offset_from_utc_datetime(&self, u: &chrono::NaiveDateTime) -> FixedOffset {
            FixedOffset::east_opt(if summer(u) { 7200 } else { 3600 }).unwrap()
        }
    }

    fn paris(utc: &str) -> DateTime<Paris> {
        Paris.from_utc_datetime(&utc.parse::<DateTime<Utc>>().unwrap().naive_utc())
    }

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn the_test_zone_is_paris() {
        assert_eq!(
            paris("2027-03-28T00:59:00Z").to_rfc3339(),
            "2027-03-28T01:59:00+01:00"
        );
        assert_eq!(
            paris("2027-03-28T01:00:00Z").to_rfc3339(),
            "2027-03-28T03:00:00+02:00"
        );
        assert_eq!(
            paris("2027-10-31T01:00:00Z").to_rfc3339(),
            "2027-10-31T02:00:00+01:00"
        );
    }

    /// The spring change: 02:30 does not exist on 2027-03-28 in Paris. It is refused, today
    /// as tomorrow, and never moved to the next day.
    #[test]
    fn a_time_the_zone_skips_is_refused() {
        let skipped = "02:30 does not exist in your time zone".to_string();
        // 01:00 CET on the day of the change: 02:30 is skipped today
        assert_eq!(
            next_local(&paris("2027-03-28T00:00:00Z"), t(2, 30)),
            Err(skipped.clone())
        );
        // 03:00 on the day before (UTC+1): 02:30 has passed, and tomorrow's does not exist
        assert_eq!(
            next_local(&paris("2027-03-27T02:00:00Z"), t(2, 30)),
            Err(skipped)
        );
        // the same hour on an ordinary day is fine
        assert_eq!(
            next_local(&paris("2027-03-26T00:00:00Z"), t(2, 30)),
            Ok(utc("2027-03-26T01:30:00Z"))
        );
    }

    /// The autumn change: 02:30 happens twice on 2027-10-31, the first one (CEST) is kept.
    #[test]
    fn a_time_the_zone_has_twice_is_the_first_one() {
        // 00:30 CEST (22:30Z the day before): 02:30 CEST is 00:30Z
        assert_eq!(
            next_local(&paris("2027-10-30T22:30:00Z"), t(2, 30)),
            Ok(utc("2027-10-31T00:30:00Z"))
        );
        // 02:45 CET (the second 02:xx): 02:30 has passed, tomorrow's is 01:30Z the day after
        assert_eq!(
            next_local(&paris("2027-10-31T01:45:00Z"), t(2, 30)),
            Ok(utc("2027-11-01T01:30:00Z"))
        );
    }

    #[test]
    fn a_grant_in_a_claude_code_shell_is_refused() {
        let p = project(MANIFEST);
        let span = Bounds {
            span: Some(Duration::hours(1)),
            ..Bounds::default()
        };
        let e = Env {
            claude_shell: true,
            ..env(true, &[])
        };
        assert_eq!(
            plan_grant(&p, "read-logs", &span, &e).unwrap_err(),
            "grant is for the person at a terminal, and this shell belongs to a Claude Code session (CLAUDECODE is set); run armadai grant in your own terminal"
        );
        assert!(plan_grant(&p, "read-logs", &span, &env(true, &[])).is_ok());
    }

    /// A level whose step was delegated since it began has ended, whatever the clock says.
    #[test]
    fn a_level_whose_step_started_is_shown_ended() {
        let p = project(MANIFEST);
        let mut state = GrantState::default();
        state.active.insert(
            "auto-until-mr".into(),
            ActiveGrant {
                since: utc("2026-11-10T08:12:00Z"),
                expires_at: None,
                until_step: Some("evolution/publish".into()),
            },
        );
        let asked = std::cell::RefCell::new(Vec::new());
        let ended = |step: &str, since: DateTime<Utc>| {
            asked.borrow_mut().push((step.to_string(), since));
            true
        };
        let list = render_list(&p, &state, &now(), &ended);
        assert_eq!(
            list.lines().nth(1),
            Some("  auto-until-mr  ended (evolution/publish started)")
        );
        assert_eq!(
            asked.into_inner(),
            vec![("evolution/publish".to_string(), utc("2026-11-10T08:12:00Z"))]
        );
        // not ended: the line is the active one again
        let list = render_list(&p, &state, &now(), &|_, _| false);
        assert_eq!(
            list.lines().nth(1),
            Some("  auto-until-mr  active until evolution/publish starts, since 09:12")
        );
    }

    /// Run inside a child process under `TZ=Europe/Paris` by the test below: the real
    /// `Local`, whose order for an ambiguous time is the platform's.
    #[test]
    #[ignore = "run by the_local_zone_is_read_under_tz_europe_paris"]
    fn local_zone_probe() {
        let now = chrono::Local.from_utc_datetime(&utc("2027-10-30T22:30:00Z").naive_utc());
        assert_eq!(
            now.offset().local_minus_utc(),
            7200,
            "TZ=Europe/Paris is not known here"
        );
        assert_eq!(
            next_local(&now, t(2, 30)),
            Ok(utc("2027-10-31T00:30:00Z")),
            "02:30 happens twice that night, the first one is kept"
        );
        let spring = chrono::Local.from_utc_datetime(&utc("2027-03-28T00:00:00Z").naive_utc());
        assert_eq!(
            next_local(&spring, t(2, 30)),
            Err("02:30 does not exist in your time zone".to_string())
        );
    }

    /// The real `Local` under a zone with a change of hour. The zone is set in a child
    /// process, so this process's environment is not touched.
    #[test]
    fn the_local_zone_is_read_under_tz_europe_paris() {
        let exe = std::env::current_exe().unwrap();
        let out = std::process::Command::new(exe)
            .env("TZ", "Europe/Paris")
            .args([
                "--exact",
                "cli::grant::tests::local_zone_probe",
                "--ignored",
            ])
            .output()
            .unwrap();
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{text}");
        assert!(text.contains("1 passed"), "the probe did not run: {text}");
    }
}
