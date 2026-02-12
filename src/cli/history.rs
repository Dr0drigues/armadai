pub async fn execute(agent: Option<String>, replay: Option<String>) -> anyhow::Result<()> {
    #[cfg(feature = "storage")]
    {
        use crate::storage::{init_db, queries};

        let db = init_db().await?;

        // Replay mode: re-run an agent with the same input
        if let Some(ref _id) = replay {
            // TODO: lookup run by ID and re-execute
            anyhow::bail!("Replay is not yet implemented");
        }

        let records = queries::get_history(&db, agent.as_deref(), 50).await?;

        if records.is_empty() {
            println!("No execution records found.");
            return Ok(());
        }

        println!(
            "{:<20} {:<15} {:<20} {:>6} {:>6} {:>10} {:>8}",
            "AGENT", "PROVIDER", "MODEL", "IN", "OUT", "COST", "MS"
        );
        println!("{}", "-".repeat(89));

        for r in &records {
            let model_short = if r.model.len() > 18 {
                format!("{}...", &r.model[..17])
            } else {
                r.model.clone()
            };
            println!(
                "{:<20} {:<15} {:<20} {:>6} {:>6} {:>10.6} {:>8}",
                r.agent, r.provider, model_short, r.tokens_in, r.tokens_out, r.cost, r.duration_ms
            );
        }

        Ok(())
    }

    #[cfg(not(feature = "storage"))]
    {
        let _ = (agent, replay);
        anyhow::bail!(
            "History requires the 'storage' feature. Build with: cargo build --features storage"
        )
    }
}
