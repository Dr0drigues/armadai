//! Report aggregation for the e2e harness: turns a `Vec<CaseOutcome>` (produced by
//! `runner::evaluate`/`harness::run_case`) into a weighted score plus two artifacts —
//! `e2e-report.json` (machine-readable, for CI gating/history) and `e2e-report.html`
//! (self-contained, human-readable, for local/CI-artifact viewing).

use std::fmt::Write as _;
use std::path::Path;

use anyhow::Context;
use serde::Serialize;

use super::runner::CaseOutcome;

/// Write `e2e-report.json` and `e2e-report.html` into `out_dir`.
///
/// `weighted_score = Σ(weight of passed cases) / Σ(weight of all cases)`, as an
/// `f64` in `[0.0, 1.0]`. When the total weight is `0` (no cases, or all cases
/// weighted `0`), the score is defined as `1.0` (vacuously "fully passing" — there's
/// nothing to fail).
pub fn write_reports(outcomes: &[CaseOutcome], out_dir: &Path) -> anyhow::Result<()> {
    let summary = Summary::from_outcomes(outcomes);

    let json = serde_json::to_string_pretty(&summary).context("serializing report to JSON")?;
    std::fs::write(out_dir.join("e2e-report.json"), json).context("writing e2e-report.json")?;

    let html = render_html(&summary, outcomes);
    std::fs::write(out_dir.join("e2e-report.html"), html).context("writing e2e-report.html")?;

    Ok(())
}

/// The JSON-serializable shape of the report: aggregate score/counts plus one
/// [`CaseSummary`] per outcome. Mirrors the schema from the task brief — note
/// `expected`/`observed` are deliberately **not** part of the JSON shape (large
/// free-form debug dumps); the HTML report renders those straight from the
/// `outcomes` slice instead (see `render_html`).
#[derive(Debug, Serialize)]
struct Summary {
    weighted_score: f64,
    total: usize,
    passed: usize,
    failed: usize,
    cases: Vec<CaseSummary>,
}

#[derive(Debug, Serialize)]
struct CaseSummary {
    name: String,
    weight: u32,
    passed: bool,
    allow_fail: bool,
    diffs: Vec<String>,
}

impl Summary {
    fn from_outcomes(outcomes: &[CaseOutcome]) -> Self {
        let total_weight: u64 = outcomes.iter().map(|o| u64::from(o.weight)).sum();
        let passed_weight: u64 = outcomes
            .iter()
            .filter(|o| o.passed)
            .map(|o| u64::from(o.weight))
            .sum();

        let weighted_score = if total_weight == 0 {
            1.0
        } else {
            passed_weight as f64 / total_weight as f64
        };

        let passed = outcomes.iter().filter(|o| o.passed).count();
        let total = outcomes.len();

        Summary {
            weighted_score,
            total,
            passed,
            failed: total - passed,
            cases: outcomes
                .iter()
                .map(|o| CaseSummary {
                    name: o.name.clone(),
                    weight: o.weight,
                    passed: o.passed,
                    allow_fail: o.allow_fail,
                    diffs: o.diffs.clone(),
                })
                .collect(),
        }
    }
}

/// Escape the five HTML-significant characters. All case-controlled strings
/// (`name`, `expected`, `observed`, `diffs`) are run through this before being
/// concatenated into the report — cases are meant to be hand-writable/agent-
/// generatable (see `case.rs` module doc), so their content is untrusted input as
/// far as the HTML renderer is concerned.
fn escape_html(s: &str) -> String {
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

/// Render the self-contained `e2e-report.html` artifact: design-token `<style>`,
/// a weighted-score gauge + chip row, a scenario table, and an expected-vs-observed
/// block per failing case. No external assets (fonts/CDN/JS libraries) — this is
/// meant to be opened locally or archived as a CI artifact without network access.
///
/// Takes both `summary` (aggregate counts, already computed once by
/// [`write_reports`]) and the raw `outcomes` (for `expected`/`observed`, which the
/// JSON `Summary` intentionally drops — see its doc comment).
fn render_html(summary: &Summary, outcomes: &[CaseOutcome]) -> String {
    let score_pct = (summary.weighted_score * 100.0).round() as i64;

    let mut rows = String::new();
    for case in &summary.cases {
        let badge = if case.passed {
            r#"<span class="badge badge-ok">PASS</span>"#
        } else if case.allow_fail {
            r#"<span class="badge badge-warn">FAIL (allowed)</span>"#
        } else {
            r#"<span class="badge badge-fail">FAIL</span>"#
        };
        let _ = write!(
            rows,
            r#"<tr><td>{}</td><td class="mono">{}</td><td>{}</td></tr>"#,
            escape_html(&case.name),
            case.weight,
            badge,
        );
    }

    let mut failures = String::new();
    for outcome in outcomes.iter().filter(|o| !o.passed) {
        let diffs_html: String = outcome
            .diffs
            .iter()
            .map(|d| format!("<li>{}</li>", escape_html(d)))
            .collect();
        let _ = write!(
            failures,
            r#"<section class="failure">
  <h3>{name}</h3>
  <ul class="diffs">{diffs}</ul>
  <div class="expected-observed">
    <div><h4>Expected</h4><pre class="mono">{expected}</pre></div>
    <div><h4>Observed</h4><pre class="mono">{observed}</pre></div>
  </div>
</section>
"#,
            name = escape_html(&outcome.name),
            diffs = diffs_html,
            expected = escape_html(&outcome.expected),
            observed = escape_html(&outcome.observed),
        );
    }

    format!(
        r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>ArmadAI e2e report</title>
<style>
{tokens}
* {{ box-sizing: border-box; }}
body {{
  margin: 0; padding: 2rem; background: var(--bg-base); color: var(--text-primary);
  font-family: system-ui, sans-serif;
}}
h1, h2, h3, h4 {{ color: var(--text-primary); }}
.mono {{ font-family: ui-monospace, "IBM Plex Mono", monospace; font-variant-numeric: tabular-nums; }}
header {{ display: flex; align-items: center; justify-content: space-between; margin-bottom: 1.5rem; }}
.theme-toggle {{
  background: var(--surface-2); color: var(--text-primary); border: 1px solid var(--border);
  border-radius: 0.4rem; padding: 0.4rem 0.8rem; cursor: pointer; font-family: system-ui, sans-serif;
}}
.gauge {{
  display: flex; align-items: baseline; gap: 0.75rem; margin-bottom: 1rem;
}}
.gauge-value {{ font-size: 3rem; font-weight: 700; color: var(--brass); }}
.gauge-track {{
  width: 100%; height: 0.6rem; border-radius: 999px; background: var(--surface-2);
  border: 1px solid var(--border-faint); overflow: hidden; margin-bottom: 1.5rem;
}}
.gauge-fill {{ height: 100%; background: var(--brass); }}
.chips {{ display: flex; gap: 0.75rem; margin-bottom: 2rem; flex-wrap: wrap; }}
.chip {{
  background: var(--surface-1); border: 1px solid var(--border); border-radius: 0.5rem;
  padding: 0.5rem 1rem; color: var(--text-secondary);
}}
.chip .mono {{ color: var(--text-primary); font-weight: 600; }}
table {{
  width: 100%; border-collapse: collapse; background: var(--surface-1);
  border: 1px solid var(--border); border-radius: 0.5rem; overflow: hidden; margin-bottom: 2rem;
}}
th, td {{ text-align: left; padding: 0.6rem 1rem; border-bottom: 1px solid var(--border-faint); }}
th {{ color: var(--text-muted); font-weight: 600; font-size: 0.85rem; text-transform: uppercase; }}
tr:last-child td {{ border-bottom: none; }}
.badge {{
  display: inline-block; padding: 0.15rem 0.6rem; border-radius: 999px; font-size: 0.8rem; font-weight: 600;
}}
.badge-ok {{ background: var(--signal-ok-bg); color: var(--signal-ok-fg); }}
.badge-fail {{ background: var(--signal-critical-bg); color: var(--signal-critical-fg); }}
.badge-warn {{ background: var(--signal-warning-bg); color: var(--text-primary); }}
.failure {{
  background: var(--surface-1); border: 1px solid var(--signal-critical); border-radius: 0.5rem;
  padding: 1rem 1.25rem; margin-bottom: 1rem;
}}
.failure h3 {{ margin-top: 0; color: var(--signal-critical-fg); }}
.diffs {{ color: var(--text-secondary); }}
.expected-observed {{ display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }}
.expected-observed pre {{
  background: var(--surface-2); border: 1px solid var(--border-faint); border-radius: 0.4rem;
  padding: 0.75rem; overflow-x: auto; white-space: pre-wrap; word-break: break-word; color: var(--text-secondary);
  max-height: 20rem; overflow-y: auto;
}}
@media (max-width: 700px) {{ .expected-observed {{ grid-template-columns: 1fr; }} }}
</style>
</head>
<body>
<header>
  <h1>ArmadAI — e2e command bridge report</h1>
  <button class="theme-toggle" onclick="const r=document.documentElement;r.setAttribute('data-theme', r.getAttribute('data-theme')==='light'?'dark':'light');">Toggle theme</button>
</header>
<div class="gauge">
  <span class="gauge-value mono">{score_pct}%</span>
  <span>weighted score</span>
</div>
<div class="gauge-track"><div class="gauge-fill" style="width:{score_pct}%"></div></div>
<div class="chips">
  <div class="chip">total <span class="mono">{total}</span></div>
  <div class="chip">passed <span class="mono">{passed}</span></div>
  <div class="chip">failed <span class="mono">{failed}</span></div>
</div>
<table>
<thead><tr><th>Scenario</th><th>Weight</th><th>Signal</th></tr></thead>
<tbody>
{rows}
</tbody>
</table>
{failures}
</body>
</html>
"##,
        tokens = DESIGN_TOKENS,
        score_pct = score_pct,
        total = summary.total,
        passed = summary.passed,
        failed = summary.failed,
        rows = rows,
        failures = failures,
    )
}

const DESIGN_TOKENS: &str = r#":root, [data-theme="dark"] { color-scheme: dark;
  --bg-base: oklch(0.188 0.030 248); --surface-1: oklch(0.223 0.031 247); --surface-2: oklch(0.258 0.032 246);
  --border: oklch(0.360 0.030 244); --border-faint: oklch(0.280 0.028 246);
  --text-primary: oklch(0.955 0.008 240); --text-secondary: oklch(0.790 0.015 240); --text-muted: oklch(0.635 0.020 242); --text-faint: oklch(0.505 0.020 244); --text-on-accent: oklch(0.180 0.030 248);
  --brass: oklch(0.790 0.100 84); --brass-strong: oklch(0.855 0.110 86); --brass-bg: oklch(0.300 0.045 82);
  --signal-ok: oklch(0.760 0.150 152); --signal-ok-bg: oklch(0.320 0.070 152); --signal-ok-fg: oklch(0.860 0.160 152);
  --signal-critical: oklch(0.660 0.190 25); --signal-critical-bg: oklch(0.320 0.085 25); --signal-critical-fg: oklch(0.800 0.170 25);
  --signal-warning: oklch(0.820 0.155 70); --signal-warning-bg: oklch(0.340 0.070 70); }
[data-theme="light"] { color-scheme: light;
  --bg-base: oklch(0.966 0.008 236); --surface-1: oklch(0.996 0.003 236); --surface-2: oklch(0.980 0.006 237);
  --border: oklch(0.868 0.013 240); --border-faint: oklch(0.910 0.010 239);
  --text-primary: oklch(0.245 0.036 248); --text-secondary: oklch(0.400 0.030 246); --text-muted: oklch(0.520 0.026 245); --text-faint: oklch(0.630 0.022 244); --text-on-accent: oklch(0.985 0.004 236);
  --brass: oklch(0.560 0.110 78); --brass-strong: oklch(0.475 0.108 76); --brass-bg: oklch(0.930 0.045 84);
  --signal-ok: oklch(0.520 0.150 152); --signal-ok-bg: oklch(0.945 0.050 152); --signal-ok-fg: oklch(0.440 0.140 152);
  --signal-critical: oklch(0.535 0.195 27); --signal-critical-bg: oklch(0.948 0.050 27); --signal-critical-fg: oklch(0.470 0.185 27);
  --signal-warning: oklch(0.560 0.140 66); --signal-warning-bg: oklch(0.950 0.055 70); }
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_json_has_weighted_score_and_per_case() {
        let outs = vec![
            CaseOutcome {
                name: "a".into(),
                weight: 3,
                passed: true,
                ..Default::default()
            },
            CaseOutcome {
                name: "b".into(),
                weight: 1,
                passed: false,
                ..Default::default()
            },
        ];
        let dir = tempfile::tempdir().unwrap();
        write_reports(&outs, dir.path()).unwrap();
        let json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("e2e-report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(json["weighted_score"].as_f64().unwrap(), 0.75); // 3/(3+1)
        assert_eq!(json["cases"].as_array().unwrap().len(), 2);
        assert!(dir.path().join("e2e-report.html").exists());
    }

    #[test]
    fn zero_total_weight_scores_1_0() {
        let outs = vec![CaseOutcome {
            name: "z".into(),
            weight: 0,
            passed: false,
            ..Default::default()
        }];
        let dir = tempfile::tempdir().unwrap();
        write_reports(&outs, dir.path()).unwrap();
        let json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("e2e-report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(json["weighted_score"].as_f64().unwrap(), 1.0);
    }

    #[test]
    fn empty_outcomes_scores_1_0_and_writes_empty_tables() {
        let dir = tempfile::tempdir().unwrap();
        write_reports(&[], dir.path()).unwrap();
        let json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("e2e-report.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(json["weighted_score"].as_f64().unwrap(), 1.0);
        assert_eq!(json["total"].as_u64().unwrap(), 0);
        assert!(json["cases"].as_array().unwrap().is_empty());
    }

    #[test]
    fn json_omits_expected_and_observed_per_case() {
        let outs = vec![CaseOutcome {
            name: "a".into(),
            weight: 1,
            passed: true,
            expected: "should not leak into json".into(),
            observed: "should not leak into json".into(),
            ..Default::default()
        }];
        let dir = tempfile::tempdir().unwrap();
        write_reports(&outs, dir.path()).unwrap();
        let json: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("e2e-report.json")).unwrap(),
        )
        .unwrap();
        let case = &json["cases"][0];
        assert!(case.get("expected").is_none());
        assert!(case.get("observed").is_none());
    }

    #[test]
    fn html_escapes_case_name_and_diffs() {
        let outs = vec![CaseOutcome {
            name: "<script>alert(1)</script>".into(),
            weight: 1,
            passed: false,
            diffs: vec!["<img src=x onerror=alert(1)>".into()],
            expected: "<b>x</b>".into(),
            observed: "<i>y</i>".into(),
            ..Default::default()
        }];
        let dir = tempfile::tempdir().unwrap();
        write_reports(&outs, dir.path()).unwrap();
        let html = std::fs::read_to_string(dir.path().join("e2e-report.html")).unwrap();
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;img"));
    }

    #[test]
    fn html_contains_design_tokens_and_pass_fail_badges() {
        let outs = vec![
            CaseOutcome {
                name: "ok-case".into(),
                weight: 1,
                passed: true,
                ..Default::default()
            },
            CaseOutcome {
                name: "bad-case".into(),
                weight: 1,
                passed: false,
                diffs: vec!["boom".into()],
                ..Default::default()
            },
        ];
        let dir = tempfile::tempdir().unwrap();
        write_reports(&outs, dir.path()).unwrap();
        let html = std::fs::read_to_string(dir.path().join("e2e-report.html")).unwrap();
        assert!(html.contains("--brass"));
        assert!(html.contains("--signal-ok"));
        assert!(html.contains("--signal-critical"));
        assert!(html.contains("data-theme"));
        assert!(html.contains("badge-ok"));
        assert!(html.contains("badge-fail"));
        assert!(html.contains("ok-case"));
        assert!(html.contains("bad-case"));
    }

    #[test]
    fn allow_fail_case_gets_warn_badge_not_fail_badge() {
        let outs = vec![CaseOutcome {
            name: "flaky".into(),
            weight: 1,
            passed: false,
            allow_fail: true,
            ..Default::default()
        }];
        let dir = tempfile::tempdir().unwrap();
        write_reports(&outs, dir.path()).unwrap();
        let html = std::fs::read_to_string(dir.path().join("e2e-report.html")).unwrap();
        assert!(html.contains("badge-warn"));
    }
}
