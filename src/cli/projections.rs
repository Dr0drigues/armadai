//! Projection management commands — rebuild flat tables from the event log.
//!
//! The `projections rebuild` command allows re-deriving flat-table rows
//! (`runs`, `orchestration_runs`, `board_entries`, `ring_contributions`,
//! `ring_votes`, `delegation_events`) from the immutable `execution_events`
//! log. Useful after schema migrations, data corruption, or when debugging
//! the projector logic itself.
//!
//! The projector (`crate::cli::run_es_record::project_run`) is idempotent:
//! calling it multiple times produces the same result (no duplicate rows).

use clap::Subcommand;

#[derive(Subcommand)]
pub enum ProjectionsAction {
    /// Rebuild flat-table projections from the event log
    Rebuild {
        /// Rebuild projections for a specific run
        #[arg(long)]
        run: Option<String>,
        /// Rebuild projections for all runs (default)
        #[arg(long)]
        all: bool,
    },
}

#[cfg(feature = "storage")]
use crate::storage::Database;

/// Rebuild projections for a single run from its event log.
///
/// Calls the idempotent `project_run` projector on the given `run_id`.
/// If the run doesn't exist in the log, no error is raised (idempotent no-op).
#[cfg(feature = "storage")]
pub fn rebuild_run(db: &Database, run_id: &str) -> anyhow::Result<()> {
    crate::cli::run_es_record::project_run(db, run_id)
}

/// Rebuild projections for all runs present in the event log.
///
/// Iterates over all distinct `run_id` values in `execution_events` and
/// projects each one via `rebuild_run`. Returns the total count of runs
/// projected (including those that were already up-to-date, since the
/// projector is idempotent).
#[cfg(feature = "storage")]
pub fn rebuild_all(db: &Database) -> anyhow::Result<usize> {
    let run_ids = crate::storage::queries::all_event_log_run_ids(db)?;
    let count = run_ids.len();
    for run_id in run_ids {
        rebuild_run(db, &run_id)?;
    }
    Ok(count)
}

/// Execute the `armadai projections` command dispatcher.
pub async fn execute(action: ProjectionsAction) -> anyhow::Result<()> {
    match action {
        ProjectionsAction::Rebuild { run, all: _all } => {
            #[cfg(feature = "storage")]
            {
                let db = crate::storage::init_db()?;

                if let Some(id) = run {
                    rebuild_run(&db, &id)?;
                    println!("1 run reprojected");
                } else {
                    // Default to --all if no flag specified
                    let count = rebuild_all(&db)?;
                    let plural = if count == 1 { "run" } else { "runs" };
                    println!("{count} {plural} reprojected");
                }

                Ok(())
            }

            #[cfg(not(feature = "storage"))]
            {
                let _ = (run, _all);
                anyhow::bail!(
                    "Projections require the 'storage' feature. Build with: cargo build --features storage"
                )
            }
        }
    }
}

#[cfg(all(test, feature = "storage"))]
mod tests {
    use super::*;
    use crate::core::orchestration::es::event::ExecutionEvent;
    use crate::core::orchestration::es::log::EventLog;
    use crate::es_log::SqliteLog;
    use crate::storage::{init_embedded, queries};

    /// Helper: construct a minimal blackboard event log suitable for
    /// projection tests (copied from run_es_record.rs storage_tests).
    fn sample_blackboard_events(run_id: &str) -> Vec<ExecutionEvent> {
        vec![
            ExecutionEvent::RunStarted {
                run_id: run_id.to_string(),
                pattern: "blackboard".to_string(),
                agents: vec!["a".to_string(), "b".to_string()],
                input: "do research".to_string(),
                project: None,
            },
            ExecutionEvent::ConfigSnapshot {
                config_json:
                    r#"{"max_rounds":5,"convergence_threshold":0.8,"consecutive_rounds":2}"#
                        .to_string(),
            },
            ExecutionEvent::RoundStarted { round: 1 },
            ExecutionEvent::AgentInvoked {
                agent: "a".to_string(),
                input: "task input".to_string(),
            },
            ExecutionEvent::BoardEntryAdded {
                agent: "a".to_string(),
                round: 1,
                kind: "finding".to_string(),
                content: "first finding".to_string(),
                refs: vec![],
                confidence: 0.9,
                tokens_in: 50,
                tokens_out: 100,
                cost: 0.03,
            },
            ExecutionEvent::Completed {
                content: "final result".to_string(),
            },
        ]
    }

    #[test]
    fn rebuild_reprojects_a_run_from_the_log() {
        let db = init_embedded().unwrap();

        // Persist a log + project once.
        let run_id = "run-y";
        let mut log = SqliteLog::new(db.clone());
        for e in sample_blackboard_events(run_id) {
            log.append(run_id, &e).unwrap();
        }
        crate::cli::run_es_record::project_run(&db, run_id).unwrap();

        // Verify projection exists.
        assert!(
            queries::get_orchestration_run(&db, run_id)
                .unwrap()
                .is_some()
        );

        // Delete the projection.
        queries::delete_projection_for_run(&db, run_id).unwrap();
        assert!(
            queries::get_orchestration_run(&db, run_id)
                .unwrap()
                .is_none()
        );

        // Rebuild via our helper.
        rebuild_run(&db, run_id).unwrap();

        // Verify projection is back.
        let orch = queries::get_orchestration_run(&db, run_id)
            .unwrap()
            .expect("orchestration run should exist after rebuild");
        assert_eq!(orch.pattern, "blackboard");
        assert_eq!(orch.run_id, run_id);
    }

    #[test]
    fn rebuild_all_projects_all_runs_in_the_log() {
        let db = init_embedded().unwrap();

        // Persist two runs.
        let mut log = SqliteLog::new(db.clone());
        for e in sample_blackboard_events("run-a") {
            log.append("run-a", &e).unwrap();
        }
        for e in sample_blackboard_events("run-b") {
            log.append("run-b", &e).unwrap();
        }

        // Rebuild all.
        let count = rebuild_all(&db).unwrap();
        assert_eq!(count, 2);

        // Both runs should be projected.
        assert!(
            queries::get_orchestration_run(&db, "run-a")
                .unwrap()
                .is_some()
        );
        assert!(
            queries::get_orchestration_run(&db, "run-b")
                .unwrap()
                .is_some()
        );
    }
}
