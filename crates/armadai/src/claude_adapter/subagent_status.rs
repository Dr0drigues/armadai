//! The rows of Claude Code's `subagentStatusLine` for a flow in progress.
//!
//! Native format, read 2026-11-05 at
//! https://code.claude.com/docs/en/statusline ("Subagent status lines"):
//! the command gets one JSON object on stdin (the base hook fields, `columns`
//! and `tasks[]`, each task with `id`, `agentType`, `status`, `description`,
//! `tokenCount`, `cwd`...) and writes one JSON line per row to override, as
//! `{"id": "<task id>", "content": "<row body>"}`; `content` is rendered as-is,
//! ANSI colours and OSC 8 links included; a task left out keeps its default row.

use armadai_model::Project;

use crate::flow_status::line::line_at;
use crate::flow_status::runs::{Mark, Run};

/// A subagent row Claude Code shows.
pub struct Task {
    pub id: String,
    pub agent_type: String,
}

/// The tasks of the JSON on stdin; an entry without a string `id` and
/// `agentType` is skipped.
pub fn tasks(json: &serde_json::Value) -> Vec<Task> {
    let text = |t: &serde_json::Value, k: &str| t.get(k)?.as_str().map(str::to_string);
    json.get("tasks")
        .and_then(|t| t.as_array())
        .into_iter()
        .flatten()
        .filter_map(|t| {
            Some(Task {
                id: text(t, "id")?,
                agent_type: text(t, "agentType")?,
            })
        })
        .collect()
}

/// One `(task id, content)` per task whose agent type holds a running step of
/// a run in progress; the content is the line of that run naming that step.
pub fn lines(project: &Project, runs: &[Run], tasks: &[Task]) -> Vec<(String, String)> {
    tasks
        .iter()
        .filter_map(|t| {
            let (run, step) = runs
                .iter()
                .rev()
                .filter(|r| !r.finished && !r.interrupted)
                .find_map(|r| {
                    let flow = project.flows.get(&r.flow)?;
                    let v = r.steps.iter().find(|v| {
                        v.mark == Mark::Running
                            && flow
                                .steps
                                .get(&v.step)
                                .is_some_and(|s| s.agent == t.agent_type)
                    })?;
                    Some((r, v))
                })?;
            let pieces = line_at(run, step);
            Some((t.id.clone(), crate::cli::flow::painted(&pieces)))
        })
        .collect()
}

/// The JSON lines to print, one per row of `lines`.
pub fn rows(project: &Project, runs: &[Run], tasks: &[Task]) -> Vec<String> {
    lines(project, runs, tasks)
        .into_iter()
        .map(|(id, content)| serde_json::json!({ "id": id, "content": content }).to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::style;
    use crate::flow_status::runs::StepView;

    fn project() -> Project {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join(".armadai");
        std::fs::create_dir_all(a.join("agents")).unwrap();
        std::fs::create_dir_all(a.join("flows")).unwrap();
        std::fs::write(a.join("project.yaml"), "schema_version: 1\n").unwrap();
        for agent in ["architect", "dev", "qa", "ops"] {
            std::fs::write(
                a.join(format!("agents/{agent}.md")),
                "---\ndescription: d\n---\nB\n",
            )
            .unwrap();
        }
        std::fs::write(
            a.join("flows/deliver.yaml"),
            "steps:\n  design: { agent: architect }\n  build: { agent: dev }\n  guard: { agent: qa }\n  validate: { agent: ops }\nlinks:\n  - { from: design, to: build }\n  - { from: build, to: guard }\n  - { from: guard, to: validate }\n",
        )
        .unwrap();
        armadai_model::load_checked(dir.path()).unwrap().0
    }

    fn view(step: &str, mark: Mark) -> StepView {
        StepView {
            step: step.into(),
            mark,
            state: None,
            note: None,
        }
    }

    fn task(id: &str, agent: &str) -> Task {
        Task {
            id: id.into(),
            agent_type: agent.into(),
        }
    }

    fn run(finished: bool) -> Run {
        Run {
            flow: "deliver".into(),
            steps: vec![
                view("design", Mark::Done),
                view("build", Mark::Running),
                view("guard", Mark::Running),
                view("validate", Mark::Pending),
            ],
            loops: vec![],
            transitions: vec![],
            finished,
            interrupted: false,
        }
    }

    #[test]
    fn a_row_names_the_step_its_agent_holds_and_a_task_without_a_step_gets_none() {
        let tasks = [
            task("t-qa", "qa"),
            task("t-explore", "Explore"),
            task("t-arch", "architect"), // holds `design`, which is done
            task("t-ops", "ops"),        // holds `validate`, which is pending
        ];
        let got = lines(&project(), &[run(false)], &tasks);
        let want = [crate::cli::flow::painted(&[
            (crate::flow_status::line::Piece::Flow, "deliver".into()),
            (
                crate::flow_status::line::Piece::Mark(Mark::Running),
                "●".into(),
            ),
            (
                crate::flow_status::line::Piece::Mark(Mark::Running),
                "guard".into(),
            ),
            (crate::flow_status::line::Piece::Progress, "1/4".into()),
        ])];
        assert_eq!(got, vec![("t-qa".to_string(), want[0].clone())]);
        assert_eq!(
            want[0],
            format!(
                "{}deliver{} {}●{} {}guard{} {}1/4{}",
                style::accent(),
                style::accent().render_reset(),
                style::running(),
                style::running().render_reset(),
                style::running(),
                style::running().render_reset(),
                style::muted(),
                style::muted().render_reset(),
            )
        );
    }

    #[test]
    fn a_finished_run_gives_no_row() {
        assert!(lines(&project(), &[run(true)], &[task("t", "qa")]).is_empty());
    }

    #[test]
    fn tasks_skip_what_is_not_a_task() {
        let json = serde_json::json!({"tasks": [
            {"id": "a", "agentType": "qa", "tokenCount": 3},
            {"id": "b"},
            {"agentType": "dev"},
            {"id": 4, "agentType": "dev"},
        ]});
        let got: Vec<_> = tasks(&json)
            .into_iter()
            .map(|t| (t.id, t.agent_type))
            .collect();
        assert_eq!(got, vec![("a".to_string(), "qa".to_string())]);
        assert!(tasks(&serde_json::json!({"tasks": "x"})).is_empty());
        assert!(tasks(&serde_json::json!({})).is_empty());
    }
}
