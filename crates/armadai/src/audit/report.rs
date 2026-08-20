use std::fmt::Write as _;
use std::path::PathBuf;

use super::rules::{Finding, Severity};

/// Assembled result of one audit run.
pub struct AuditReport {
    pub root: PathBuf,
    pub detected: Vec<String>,
    pub agent_count: usize,
    pub skill_count: usize,
    pub findings: Vec<Finding>,
    /// Raw, unstructured deep-analysis text (set when the LLM's deep-audit
    /// response could not be parsed as structured D0x findings).
    pub deep_raw: Option<String>,
    /// Observed usage, when transcripts were found for this project.
    pub usage: Option<crate::audit::usage::UsageFacts>,
}

/// Sorted, truncated view data shared by `AuditReport::usage_markdown` and
/// `AuditReport::print_usage` — see `AuditReport::usage_view`.
struct UsageView<'a> {
    usage: &'a crate::audit::usage::UsageFacts,
    agents: Vec<(&'a String, &'a crate::audit::usage::facts::AgentUsage)>,
    skills: Vec<(&'a String, &'a u32)>,
}

impl UsageView<'_> {
    /// How many declared agents were truncated off the top-10 list — 0 when
    /// the list is already complete.
    fn agents_hidden(&self) -> usize {
        self.usage.agents.len().saturating_sub(self.agents.len())
    }

    /// Same as `agents_hidden`, for skills.
    fn skills_hidden(&self) -> usize {
        self.usage.skills.len().saturating_sub(self.skills.len())
    }
}

/// `12 invocation(s), 87 turn(s)` — or just the invocations when the agent's own
/// transcript was never found. Invocations count how often an agent was asked
/// for; turns count what it actually did. Measured 2026-08-20, they differ by
/// an order of magnitude.
fn agent_volume(stats: &crate::audit::usage::facts::AgentUsage) -> String {
    if stats.turns > 0 {
        format!(
            "{} invocation(s), {} turn(s)",
            stats.invocations, stats.turns
        )
    } else {
        format!("{} invocation(s)", stats.invocations)
    }
}

impl AuditReport {
    pub fn critical_count(&self) -> usize {
        self.count(Severity::Critical)
    }

    fn count(&self, s: Severity) -> usize {
        self.findings.iter().filter(|f| f.severity == s).count()
    }

    fn summary_line(&self) -> String {
        format!(
            "{} critical, {} warning(s), {} info",
            self.count(Severity::Critical),
            self.count(Severity::Warning),
            self.count(Severity::Info)
        )
    }

    /// Findings paths relative to the audited root (falls back to the raw path).
    fn rel(&self, p: &std::path::Path) -> String {
        p.strip_prefix(&self.root)
            .unwrap_or(p)
            .display()
            .to_string()
    }

    fn severity_title(s: Severity) -> &'static str {
        match s {
            Severity::Critical => "Critical",
            Severity::Warning => "Warning",
            Severity::Info => "Info",
        }
    }

    /// Accent style for a severity level (terminal output only).
    fn severity_style(s: Severity) -> anstyle::Style {
        match s {
            Severity::Critical => crate::cli::style::err(),
            Severity::Warning => crate::cli::style::warn(),
            Severity::Info => crate::cli::style::ok(),
        }
    }

    /// Per-rule counts, e.g. `A01×2  A06×1(+3)  A08×1(+23)`.
    fn breakdown_line(&self) -> String {
        let mut per_rule: std::collections::BTreeMap<&str, (usize, usize)> =
            std::collections::BTreeMap::new();
        for f in &self.findings {
            let entry = per_rule.entry(f.rule).or_insert((0, 0));
            entry.0 += 1;
            entry.1 += f.related.len();
        }
        per_rule
            .into_iter()
            .map(|(rule, (count, related))| {
                if related > 0 {
                    format!("{rule}×{count}(+{related})")
                } else {
                    format!("{rule}×{count}")
                }
            })
            .collect::<Vec<_>>()
            .join("  ")
    }

    /// Funnel block: what adopting ArmadAI would fix automatically.
    fn funnel_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        let remappable = self.findings.iter().filter(|f| f.rule == "A03").count();
        if remappable > 0 {
            lines.push(format!(
                "{remappable} deprecated model(s) remapped automatically (model aliases)"
            ));
        }
        let clusters = self.findings.iter().filter(|f| f.rule == "A06").count();
        if clusters > 0 {
            lines.push(format!(
                "{clusters} shared content cluster(s) factored into reusable prompt fragments"
            ));
        }
        let oversized = self.findings.iter().filter(|f| f.rule == "A05").count();
        if oversized > 0 {
            lines.push(format!(
                "{oversized} oversized prompt(s) split with composable fragments"
            ));
        }
        lines
    }

    /// Sorted, top-10-truncated view of `self.usage`, shared by
    /// `usage_markdown` and `print_usage` — `None` when nothing was observed
    /// (no transcripts found, or `usage` is `None`). Extracted so the
    /// sort/tie-break and truncation logic — the part most likely to drift
    /// under a future change — exists exactly once; each renderer still does
    /// its own, format-specific printing.
    fn usage_view(&self) -> Option<UsageView<'_>> {
        let usage = self.usage.as_ref().filter(|u| !u.is_empty())?;
        let mut agents: Vec<_> = usage.agents.iter().collect();
        agents.sort_by(|a, b| b.1.invocations.cmp(&a.1.invocations).then(a.0.cmp(b.0)));
        agents.truncate(10);
        let mut skills: Vec<_> = usage.skills.iter().collect();
        skills.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        skills.truncate(10);
        Some(UsageView {
            usage,
            agents,
            skills,
        })
    }

    /// Markdown block describing what was observed. Empty when nothing was.
    fn usage_markdown(&self) -> String {
        let Some(view) = self.usage_view() else {
            return String::new();
        };
        let mut out = String::new();
        let _ = writeln!(out, "\n## Observed usage\n");
        let _ = writeln!(out, "- Sessions scanned: {}", view.usage.sessions);
        if view.usage.observed_depth > 0 {
            let _ = writeln!(
                out,
                "- Observed delegation depth: {}",
                view.usage.observed_depth
            );
        }
        if let Some((from, to)) = &view.usage.window {
            let _ = writeln!(out, "- Window: {from} → {to}");
        }
        if !view.agents.is_empty() {
            let _ = writeln!(out, "\n### Agents by invocation\n");
            for (name, stats) in &view.agents {
                let _ = writeln!(out, "- `{}` — {}", name, agent_volume(stats));
            }
            let hidden = view.agents_hidden();
            if hidden > 0 {
                let _ = writeln!(out, "- *(+{hidden} others)*");
            }
        }
        if !view.skills.is_empty() {
            let _ = writeln!(out, "\n### Skills by attributed turns\n");
            for (name, turns) in &view.skills {
                let _ = writeln!(out, "- `{name}` — {turns} turn(s)");
            }
            let hidden = view.skills_hidden();
            if hidden > 0 {
                let _ = writeln!(out, "- *(+{hidden} others)*");
            }
        }
        out
    }

    /// Terminal rendering of the same content as `usage_markdown`, printed
    /// before the findings so the report states what was measured first.
    /// Silent when nothing was observed.
    fn print_usage(&self) {
        let Some(view) = self.usage_view() else {
            return;
        };
        let h = crate::cli::style::header();
        let m = crate::cli::style::muted();
        anstream::println!();
        anstream::println!("  {h}Observed usage{h:#}");
        anstream::println!("  {m}Sessions scanned:{m:#} {}", view.usage.sessions);
        if view.usage.observed_depth > 0 {
            anstream::println!(
                "  {m}Observed delegation depth:{m:#} {}",
                view.usage.observed_depth
            );
        }
        if let Some((from, to)) = &view.usage.window {
            anstream::println!("  {m}Window:{m:#} {from} → {to}");
        }
        if !view.agents.is_empty() {
            anstream::println!("  {m}Agents by invocation:{m:#}");
            for (name, stats) in &view.agents {
                anstream::println!("    {name} — {}", agent_volume(stats));
            }
            let hidden = view.agents_hidden();
            if hidden > 0 {
                anstream::println!("    {m}(+{hidden} others){m:#}");
            }
        }
        if !view.skills.is_empty() {
            anstream::println!("  {m}Skills by attributed turns:{m:#}");
            for (name, turns) in &view.skills {
                anstream::println!("    {name} — {turns} turn(s)");
            }
            let hidden = view.skills_hidden();
            if hidden > 0 {
                anstream::println!("    {m}(+{hidden} others){m:#}");
            }
        }
    }

    /// HTML rendering of the same content as `usage_markdown`/`print_usage`,
    /// escaping every value — window timestamps are the transcript's own
    /// text, and agent/skill names come from `record_delegation`/
    /// `record_skill_turn` (sanitized of control characters, but not of HTML
    /// metacharacters). Empty when nothing was observed.
    fn usage_html(&self) -> String {
        let Some(view) = self.usage_view() else {
            return String::new();
        };
        let mut out = String::new();
        out.push_str("<section class=\"usage\">\n<h2>Observed usage</h2>\n<ul>\n");
        let _ = writeln!(out, "<li>Sessions scanned: {}</li>", view.usage.sessions);
        if view.usage.observed_depth > 0 {
            let _ = writeln!(
                out,
                "<li>Observed delegation depth: {}</li>",
                view.usage.observed_depth
            );
        }
        if let Some((from, to)) = &view.usage.window {
            let _ = writeln!(
                out,
                "<li>Window: {} → {}</li>",
                html_escape(from),
                html_escape(to)
            );
        }
        out.push_str("</ul>\n");
        if !view.agents.is_empty() {
            out.push_str("<h3>Agents by invocation</h3>\n<ul>\n");
            for (name, stats) in &view.agents {
                let _ = writeln!(
                    out,
                    "<li><code>{}</code> — {}</li>",
                    html_escape(name),
                    html_escape(&agent_volume(stats))
                );
            }
            let hidden = view.agents_hidden();
            if hidden > 0 {
                let _ = writeln!(out, "<li class=\"more\">(+{hidden} others)</li>");
            }
            out.push_str("</ul>\n");
        }
        if !view.skills.is_empty() {
            out.push_str("<h3>Skills by attributed turns</h3>\n<ul>\n");
            for (name, turns) in &view.skills {
                let _ = writeln!(
                    out,
                    "<li><code>{}</code> — {turns} turn(s)</li>",
                    html_escape(name)
                );
            }
            let hidden = view.skills_hidden();
            if hidden > 0 {
                let _ = writeln!(out, "<li class=\"more\">(+{hidden} others)</li>");
            }
            out.push_str("</ul>\n");
        }
        out.push_str("</section>\n");
        out
    }

    /// Plain aligned output on stdout, findings ordered by severity
    /// (findings are already sorted by run_rules). The CLI's final
    /// `anyhow::bail!` on critical findings is what signals errors on
    /// stderr, so this only ever writes to stdout.
    pub fn print_terminal(&self, min_severity: Severity) {
        let h = crate::cli::style::header();
        anstream::println!("{h}armadai audit - {}{h:#}", self.root.display());
        let m = crate::cli::style::muted();
        let a = crate::cli::style::accent();
        anstream::println!(
            "  {m}Detected:{m:#} {a}{}{a:#} {m}({} agent(s), {} skill(s)){m:#}",
            self.detected.join(", "),
            self.agent_count,
            self.skill_count
        );
        self.print_usage();
        for severity in [Severity::Critical, Severity::Warning, Severity::Info] {
            if severity > min_severity {
                continue;
            }
            let group: Vec<&Finding> = self
                .findings
                .iter()
                .filter(|f| f.severity == severity)
                .collect();
            if group.is_empty() {
                continue;
            }
            anstream::println!();
            let s = Self::severity_style(severity);
            anstream::println!(
                "  {s}{} ({}){s:#}",
                Self::severity_title(severity),
                group.len()
            );
            for f in group {
                let related = if f.related.is_empty() {
                    String::new()
                } else {
                    format!(" (+{} others)", f.related.len())
                };
                let a = crate::cli::style::accent();
                let m = crate::cli::style::muted();
                anstream::println!(
                    "    {a}{:<4}{a:#} {m}{}{related}{m:#}  {}",
                    f.rule,
                    self.rel(&f.file),
                    f.message
                );
                if let Some(s) = &f.suggestion {
                    let m = crate::cli::style::muted();
                    anstream::println!("         {m}-> {s}{m:#}");
                }
            }
        }
        let hidden = self
            .findings
            .iter()
            .filter(|f| f.severity > min_severity)
            .count();
        anstream::println!();
        let h = crate::cli::style::header();
        anstream::println!("  {h}Summary:{h:#} {}", self.summary_line());
        if !self.findings.is_empty() {
            anstream::println!("  {h}Breakdown:{h:#} {}", self.breakdown_line());
        }
        if hidden > 0 {
            let m = crate::cli::style::muted();
            anstream::println!(
                "  {m}({hidden} finding(s) hidden below the severity threshold){m:#}"
            );
        }
        let funnel = self.funnel_lines();
        if !funnel.is_empty() {
            anstream::println!();
            let h = crate::cli::style::header();
            anstream::println!("  {h}What ArmadAI would give you:{h:#}");
            let m = crate::cli::style::muted();
            for l in funnel {
                anstream::println!("    {m}- {l}{m:#}");
            }
            let a = crate::cli::style::accent();
            anstream::println!("    {a}Run `armadai audit --propose` to generate the config.{a:#}");
        }
        if let Some(raw) = &self.deep_raw {
            anstream::println!();
            let h = crate::cli::style::header();
            anstream::println!("  {h}Deep analysis (unstructured):{h:#}");
            for line in raw.lines() {
                anstream::println!("    {line}");
            }
        }
    }

    pub fn to_markdown(&self) -> String {
        let mut md = String::new();
        let _ = writeln!(md, "# armadai audit — {}\n", self.root.display());
        let _ = writeln!(
            md,
            "Detected: {} ({} agents, {} skills)\n",
            self.detected.join(", "),
            self.agent_count,
            self.skill_count
        );
        let _ = writeln!(md, "**Summary: {}**\n", self.summary_line());
        md.push_str(&self.usage_markdown());
        if !self.findings.is_empty() {
            let _ = writeln!(md, "Breakdown: {}\n", self.breakdown_line());
        }
        for severity in [Severity::Critical, Severity::Warning, Severity::Info] {
            let group: Vec<&Finding> = self
                .findings
                .iter()
                .filter(|f| f.severity == severity)
                .collect();
            if group.is_empty() {
                continue;
            }
            let _ = writeln!(
                md,
                "## {} ({})\n",
                Self::severity_title(severity),
                group.len()
            );
            let _ = writeln!(md, "| Rule | File | Related | Message | Suggestion |");
            let _ = writeln!(md, "|---|---|---|---|---|");
            for f in &group {
                let related = if f.related.is_empty() {
                    "—".to_string()
                } else {
                    format!("+{}", f.related.len())
                };
                let _ = writeln!(
                    md,
                    "| {} | `{}` | {} | {} | {} |",
                    f.rule,
                    self.rel(&f.file),
                    related,
                    md_cell(&f.message),
                    md_cell(f.suggestion.as_deref().unwrap_or("—"))
                );
            }
            let _ = writeln!(md);
            for f in &group {
                if f.related.is_empty() {
                    continue;
                }
                let _ = writeln!(
                    md,
                    "<details><summary>{} — {} related file(s)</summary>\n",
                    f.rule,
                    f.related.len()
                );
                for p in &f.related {
                    let _ = writeln!(md, "- `{}`", self.rel(p));
                }
                let _ = writeln!(md, "\n</details>\n");
            }
        }
        let collisions: Vec<&Finding> = self
            .findings
            .iter()
            .filter(|f| f.rule.starts_with('C'))
            .collect();
        if !collisions.is_empty() {
            let _ = writeln!(md, "## Collision matrix\n");
            let _ = writeln!(md, "| Rule | Assets | Claim | Suggestion |");
            let _ = writeln!(md, "|---|---|---|---|");
            for f in &collisions {
                let mut assets = vec![self.rel(&f.file)];
                assets.extend(f.related.iter().map(|p| self.rel(p)));
                let _ = writeln!(
                    md,
                    "| {} | `{}` | {} | {} |",
                    f.rule,
                    assets.join("`, `"),
                    md_cell(&f.message),
                    md_cell(f.suggestion.as_deref().unwrap_or("—"))
                );
            }
            let _ = writeln!(md);
        }
        let funnel = self.funnel_lines();
        if !funnel.is_empty() {
            let _ = writeln!(md, "## What ArmadAI would give you\n");
            for l in funnel {
                let _ = writeln!(md, "- {l}");
            }
        }
        if let Some(raw) = &self.deep_raw {
            let _ = writeln!(md, "## Deep analysis (unstructured)\n");
            let fence = "`".repeat(fence_len_for(raw));
            let _ = writeln!(md, "{fence}\n{raw}\n{fence}\n");
        }
        md
    }

    /// Fully self-contained HTML report: no external resources, no JavaScript,
    /// inline CSS with light/dark support via `prefers-color-scheme`.
    pub fn to_html(&self) -> String {
        let root = html_escape(&self.root.display().to_string());
        let detected = html_escape(&self.detected.join(", "));
        let mut html = String::new();
        let _ = write!(
            html,
            r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>armadai audit — {root}</title>
<style>
{css}
</style>
</head>
<body>
<header>
<h1>armadai audit — <code>{root}</code></h1>
<p class="detected">Detected: {detected} ({agent_count} agent(s), {skill_count} skill(s))</p>
</header>
<p class="summary"><strong>Summary: {summary}</strong></p>
"#,
            css = HTML_CSS,
            agent_count = self.agent_count,
            skill_count = self.skill_count,
            summary = html_escape(&self.summary_line()),
        );

        html.push_str(&self.usage_html());

        if !self.findings.is_empty() {
            let _ = writeln!(
                html,
                r#"<p class="summary">Breakdown: {}</p>"#,
                html_escape(&self.breakdown_line())
            );
        }
        for severity in [Severity::Critical, Severity::Warning, Severity::Info] {
            let group: Vec<&Finding> = self
                .findings
                .iter()
                .filter(|f| f.severity == severity)
                .collect();
            if group.is_empty() {
                continue;
            }
            let badge_class = match severity {
                Severity::Critical => "badge-crit",
                Severity::Warning => "badge-warn",
                Severity::Info => "badge-info",
            };
            let _ = writeln!(
                html,
                r#"<h2><span class="badge {badge_class}">{}</span> {} ({})</h2>
<table>
<thead>
<tr><th>Rule</th><th>File</th><th>Message</th><th>Suggestion</th></tr>
</thead>
<tbody>"#,
                html_escape(severity.label()),
                html_escape(Self::severity_title(severity)),
                group.len()
            );
            for f in &group {
                let related_html = if f.related.is_empty() {
                    String::new()
                } else {
                    let mut d = format!(
                        "<details><summary>+{} related</summary><ul>",
                        f.related.len()
                    );
                    for p in &f.related {
                        let _ = write!(
                            d,
                            "<li><code class=\"path\">{}</code></li>",
                            html_escape(&self.rel(p))
                        );
                    }
                    d.push_str("</ul></details>");
                    d
                };
                let _ = writeln!(
                    html,
                    r#"<tr>
<td>{rule}</td>
<td><code class="path">{file}</code>{related_html}</td>
<td>{message}</td>
<td>{suggestion}</td>
</tr>"#,
                    rule = html_escape(f.rule),
                    file = html_escape(&self.rel(&f.file)),
                    message = html_escape(&f.message),
                    suggestion = f
                        .suggestion
                        .as_deref()
                        .map(html_escape)
                        .unwrap_or_else(|| "—".to_string()),
                );
            }
            html.push_str("</tbody>\n</table>\n");
        }

        let collisions: Vec<&Finding> = self
            .findings
            .iter()
            .filter(|f| f.rule.starts_with('C'))
            .collect();
        if !collisions.is_empty() {
            html.push_str(
                r#"<h2>Collision matrix</h2>
<table>
<thead>
<tr><th>Rule</th><th>Assets</th><th>Claim</th><th>Suggestion</th></tr>
</thead>
<tbody>
"#,
            );
            for f in &collisions {
                let mut assets = format!(
                    "<code class=\"path\">{}</code>",
                    html_escape(&self.rel(&f.file))
                );
                for p in &f.related {
                    let _ = write!(
                        assets,
                        ", <code class=\"path\">{}</code>",
                        html_escape(&self.rel(p))
                    );
                }
                let _ = writeln!(
                    html,
                    r#"<tr>
<td>{rule}</td>
<td>{assets}</td>
<td>{claim}</td>
<td>{suggestion}</td>
</tr>"#,
                    rule = html_escape(f.rule),
                    claim = html_escape(&f.message),
                    suggestion = f
                        .suggestion
                        .as_deref()
                        .map(html_escape)
                        .unwrap_or_else(|| "—".to_string()),
                );
            }
            html.push_str("</tbody>\n</table>\n");
        }

        let funnel = self.funnel_lines();
        if !funnel.is_empty() {
            html.push_str(
                "<section class=\"funnel\">\n<h2>What ArmadAI would give you</h2>\n<ul>\n",
            );
            for l in funnel {
                let _ = writeln!(html, "<li>{}</li>", html_escape(&l));
            }
            html.push_str("</ul>\n</section>\n");
        }

        if let Some(raw) = &self.deep_raw {
            let _ = writeln!(
                html,
                "<section class=\"deep-raw\">\n<h2>Deep analysis (unstructured)</h2>\n<pre>{}</pre>\n</section>",
                html_escape(raw)
            );
        }

        html.push_str("<footer><p>generated by armadai audit</p></footer>\n</body>\n</html>\n");
        html
    }
}

const HTML_CSS: &str = r#"
:root {
  color-scheme: light dark;
  --bg: #ffffff;
  --fg: #1a1a1a;
  --muted: #5a5a5a;
  --border: #d8d8d8;
  --row-alt: #f5f5f5;
  --crit-bg: #fde2e1;
  --crit-fg: #8b1a1a;
  --warn-bg: #fdf0d5;
  --warn-fg: #8a5a00;
  --info-bg: #dbeafe;
  --info-fg: #1e40af;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #1e1e1e;
    --fg: #e6e6e6;
    --muted: #a0a0a0;
    --border: #3a3a3a;
    --row-alt: #262626;
    --crit-bg: #4a1414;
    --crit-fg: #ff9a9a;
    --warn-bg: #4a3a10;
    --warn-fg: #ffcf7a;
    --info-bg: #12305a;
    --info-fg: #9ec5ff;
  }
}
* { box-sizing: border-box; }
body {
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif;
  background: var(--bg);
  color: var(--fg);
  margin: 0;
  padding: 1.5rem;
  line-height: 1.5;
}
header h1 { font-size: 1.3rem; margin-bottom: 0.25rem; }
header .detected, .summary { color: var(--muted); }
code { font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }
table {
  width: 100%;
  border-collapse: collapse;
  margin-top: 1rem;
}
th, td {
  text-align: left;
  padding: 0.5rem 0.6rem;
  border-bottom: 1px solid var(--border);
  vertical-align: top;
}
tbody tr:nth-child(even) { background: var(--row-alt); }
code.path {
  word-break: break-all;
  overflow-wrap: anywhere;
}
.badge {
  display: inline-block;
  padding: 0.15rem 0.5rem;
  border-radius: 0.75rem;
  font-size: 0.75rem;
  font-weight: 600;
  white-space: nowrap;
}
.badge-crit { background: var(--crit-bg); color: var(--crit-fg); }
.badge-warn { background: var(--warn-bg); color: var(--warn-fg); }
.badge-info { background: var(--info-bg); color: var(--info-fg); }
h2 { font-size: 1.05rem; margin-top: 1.5rem; }
details ul { margin: 0.25rem 0 0 1rem; padding: 0; }
.funnel { margin-top: 1.5rem; }
.funnel h2 { font-size: 1.05rem; }
.usage h2 { font-size: 1.05rem; }
.usage h3 { font-size: 0.95rem; margin-top: 1rem; }
.usage .more { color: var(--muted); }
footer {
  margin-top: 2rem;
  color: var(--muted);
  font-size: 0.85rem;
}
"#;

/// Escape a string for a GFM table cell: pipes and newlines break rows.
fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// Length of the code fence needed to safely wrap `text` in a Markdown code
/// block: at least 3 backticks, and longer than any run of consecutive
/// backticks already present in `text` (otherwise an inner run could close
/// the fence early and corrupt the rendered report).
fn fence_len_for(text: &str) -> usize {
    let mut longest_run = 0;
    let mut current_run = 0;
    for c in text.chars() {
        if c == '`' {
            current_run += 1;
            longest_run = longest_run.max(current_run);
        } else {
            current_run = 0;
        }
    }
    (longest_run + 1).max(3)
}

/// Escape the five HTML special characters so untrusted findings text
/// (file paths, messages, suggestions) can never break out of markup.
fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::rules::{Finding, Severity};

    fn report_with(findings: Vec<Finding>) -> AuditReport {
        AuditReport {
            root: ".".into(),
            detected: vec!["claude".into()],
            agent_count: 2,
            skill_count: 1,
            findings,
            deep_raw: None,
            usage: None,
        }
    }

    fn finding(rule: &'static str, severity: Severity) -> Finding {
        Finding {
            rule,
            severity,
            file: ".claude/agents/x.md".into(),
            related: Vec::new(),
            message: "msg".into(),
            suggestion: Some("fix".into()),
        }
    }

    fn finding_with_related(rule: &'static str, severity: Severity, related: usize) -> Finding {
        Finding {
            rule,
            severity,
            file: ".claude/agents/x.md".into(),
            related: (0..related)
                .map(|i| format!(".claude/agents/r{i}.md").into())
                .collect(),
            message: "msg".into(),
            suggestion: Some("fix".into()),
        }
    }

    #[test]
    fn markdown_groups_by_severity_with_counts() {
        let r = report_with(vec![
            finding("A01", Severity::Critical),
            finding_with_related("A08", Severity::Info, 3),
        ]);
        let md = r.to_markdown();
        assert!(md.contains("## Critical (1)"));
        assert!(md.contains("## Info (1)"));
        assert!(md.contains("Breakdown:"));
        assert!(md.contains("A08×1(+3)"));
    }

    #[test]
    fn html_lists_related_files_in_details() {
        let r = report_with(vec![finding_with_related("A06", Severity::Warning, 2)]);
        let html = r.to_html();
        assert!(html.contains("<details>"));
        assert!(html.contains("r0.md"));
        assert!(html.contains("+2"));
    }

    #[test]
    fn paths_are_relative_to_root() {
        let mut r = report_with(vec![finding("A01", Severity::Critical)]);
        r.root = PathBuf::from("/repo");
        r.findings[0].file = PathBuf::from("/repo/.claude/agents/x.md");
        let md = r.to_markdown();
        assert!(md.contains("`.claude/agents/x.md`"));
        assert!(!md.contains("`/repo/.claude"));
    }

    #[test]
    fn markdown_contains_breakdown_line() {
        let mut f = finding("A08", Severity::Info);
        f.related = vec!["r1.md".into(), "r2.md".into()];
        let r = report_with(vec![f]);
        assert!(r.to_markdown().contains("Breakdown: A08×1(+2)"));
    }

    #[test]
    fn critical_count_counts_only_critical() {
        let r = report_with(vec![
            finding("A03", Severity::Critical),
            finding("A05", Severity::Warning),
        ]);
        assert_eq!(r.critical_count(), 1);
    }

    #[test]
    fn markdown_contains_summary_and_funnel_block() {
        let r = report_with(vec![finding("A03", Severity::Critical)]);
        let md = r.to_markdown();
        assert!(md.contains("# armadai audit"));
        assert!(md.contains("| A03 |"));
        assert!(md.contains("1 critical"));
        assert!(md.contains("What ArmadAI would give you"));
        assert!(md.contains("1 deprecated model(s) remapped automatically"));
    }

    #[test]
    fn funnel_counts_clusters_not_pairs() {
        let r = report_with(vec![finding("A06", Severity::Warning)]);
        let md = r.to_markdown();
        assert!(md.contains("1 shared content cluster(s)"));
    }

    #[test]
    fn html_contains_document_structure_and_summary() {
        let r = report_with(vec![finding("A03", Severity::Critical)]);
        let html = r.to_html();
        assert!(html.starts_with("<!doctype html>"));
        assert!(html.contains("armadai audit"));
        assert!(html.contains("1 critical"));
        assert!(html.contains("CRIT"));
    }

    #[test]
    fn html_escapes_dynamic_content() {
        let mut f = finding("A03", Severity::Critical);
        f.message = "a <b> & \"c\" | d".into();
        let r = report_with(vec![f]);
        let html = r.to_html();
        assert!(html.contains("&lt;b&gt;"));
        assert!(html.contains("&amp;"));
        assert!(html.contains("&quot;"));
        assert!(!html.contains("<b>"));
    }

    #[test]
    fn markdown_escapes_pipes_in_cells() {
        let mut f = finding("A01", Severity::Critical);
        f.message = "value contains | a pipe".into();
        let r = report_with(vec![f]);
        let md = r.to_markdown();
        assert!(md.contains("value contains \\| a pipe"));
    }

    #[test]
    fn collision_matrix_lists_c_findings() {
        let mut c = finding("C02", Severity::Warning);
        c.related = vec![".claude/agents/b.md".into()];
        let a = finding("A01", Severity::Critical);
        let r = report_with(vec![a, c]);
        let md = r.to_markdown();
        assert!(md.contains("## Collision matrix"));
        assert!(md.contains("| C02 |"));
        let html = r.to_html();
        assert!(html.contains("Collision matrix"));
    }

    #[test]
    fn no_collision_matrix_without_c_findings() {
        let r = report_with(vec![finding("A01", Severity::Critical)]);
        assert!(!r.to_markdown().contains("Collision matrix"));
    }

    #[test]
    fn deep_raw_renders_section() {
        let mut r = report_with(vec![]);
        r.deep_raw = Some("Free-form deep notes.".into());
        let md = r.to_markdown();
        assert!(md.contains("Deep analysis"));
        assert!(md.contains("Free-form deep notes."));
        let html = r.to_html();
        assert!(html.contains("Deep analysis"));
    }

    #[test]
    fn deep_findings_appear_in_severity_groups() {
        let r = report_with(vec![finding("D01", Severity::Warning)]);
        let md = r.to_markdown();
        assert!(md.contains("| D01 |"));
    }

    #[test]
    fn markdown_reports_the_observed_window_and_top_agents() {
        let mut usage = crate::audit::usage::UsageFacts {
            sessions: 2,
            ..Default::default()
        };
        usage.observe_timestamp("2026-07-01T00:00:00Z");
        usage.observe_timestamp("2026-08-13T00:00:00Z");
        usage.record_delegation(crate::audit::usage::facts::ROOT_AGENT, "qa", "m");

        let report = AuditReport {
            root: std::path::PathBuf::from("/p"),
            detected: vec!["claude".to_string()],
            agent_count: 1,
            skill_count: 0,
            findings: vec![],
            deep_raw: None,
            usage: Some(usage),
        };
        let md = report.to_markdown();
        assert!(md.contains("Observed usage"), "{md}");
        assert!(md.contains("2026-07-01T00:00:00Z"), "window start: {md}");
        assert!(md.contains("qa"), "top agents: {md}");
    }

    #[test]
    fn html_reports_the_observed_usage_section_with_a_known_agent_name() {
        let mut usage = crate::audit::usage::UsageFacts {
            sessions: 2,
            ..Default::default()
        };
        usage.observe_timestamp("2026-07-01T00:00:00Z");
        usage.observe_timestamp("2026-08-13T00:00:00Z");
        usage.record_delegation(crate::audit::usage::facts::ROOT_AGENT, "qa", "m");

        let report = AuditReport {
            root: std::path::PathBuf::from("/p"),
            detected: vec!["claude".to_string()],
            agent_count: 1,
            skill_count: 0,
            findings: vec![],
            deep_raw: None,
            usage: Some(usage),
        };
        let html = report.to_html();
        assert!(html.contains("Observed usage"), "{html}");
        assert!(html.contains("qa"), "top agents: {html}");
        assert!(
            html.contains("2026-07-01T00:00:00Z"),
            "window start: {html}"
        );
    }

    #[test]
    fn usage_sections_indicate_truncation_beyond_the_top_ten() {
        let mut usage = crate::audit::usage::UsageFacts {
            sessions: 1,
            ..Default::default()
        };
        for i in 0..12 {
            usage.record_delegation(
                crate::audit::usage::facts::ROOT_AGENT,
                &format!("agent-{i:02}"),
                "m",
            );
        }
        let report = AuditReport {
            root: std::path::PathBuf::from("/p"),
            detected: vec!["claude".to_string()],
            agent_count: 12,
            skill_count: 0,
            findings: vec![],
            deep_raw: None,
            usage: Some(usage),
        };
        let md = report.to_markdown();
        assert!(
            md.contains("+2 others"),
            "12 agents truncated to 10 must say how many are hidden: {md}"
        );
        let html = report.to_html();
        assert!(
            html.contains("+2 others"),
            "the HTML section must carry the same indication: {html}"
        );
    }

    #[test]
    fn deep_raw_with_backtick_fence_uses_longer_outer_fence() {
        let mut r = report_with(vec![]);
        let raw = "before\n```\nnested code block\n```\nafter";
        r.deep_raw = Some(raw.to_string());
        let md = r.to_markdown();
        // The full raw text (including its own ``` fence) must survive intact.
        assert!(md.contains(raw));
        // The outer fence must be longer than the longest inner backtick run (3).
        assert!(md.contains("````\n"));
        assert!(!md.contains("`````"));
    }
}
