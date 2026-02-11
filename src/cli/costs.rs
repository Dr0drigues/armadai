pub async fn execute(agent: Option<String>, _from: Option<String>) -> anyhow::Result<()> {
    #[cfg(feature = "storage")]
    {
        use crate::storage::{init_db, queries};

        let db = init_db().await?;
        let summaries = queries::get_costs_summary(&db, agent.as_deref()).await?;

        if summaries.is_empty() {
            println!("No execution records found.");
            return Ok(());
        }

        // Header
        println!(
            "{:<20} {:>6} {:>12} {:>10} {:>10}",
            "AGENT", "RUNS", "COST (USD)", "TOKENS IN", "TOKENS OUT"
        );
        println!("{}", "-".repeat(62));

        let mut total_cost = 0.0;
        let mut total_runs = 0i64;

        for s in &summaries {
            println!(
                "{:<20} {:>6} {:>12.6} {:>10} {:>10}",
                s.agent, s.total_runs, s.total_cost, s.total_tokens_in, s.total_tokens_out
            );
            total_cost += s.total_cost;
            total_runs += s.total_runs;
        }

        if summaries.len() > 1 {
            println!("{}", "-".repeat(62));
            println!("{:<20} {:>6} {:>12.6}", "TOTAL", total_runs, total_cost);
        }

        Ok(())
    }

    #[cfg(not(feature = "storage"))]
    {
        let _ = (agent, _from);
        anyhow::bail!(
            "Cost tracking requires the 'storage' feature. Build with: cargo build --features storage"
        )
    }
}
