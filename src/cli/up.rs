use std::process::Command;

pub async fn start() -> anyhow::Result<()> {
    tracing::info!("Starting infrastructure services...");
    let status = Command::new("docker")
        .args(["compose", "up", "-d"])
        .status()?;

    if status.success() {
        println!("Infrastructure services started.");
    } else {
        anyhow::bail!("Failed to start infrastructure services");
    }
    Ok(())
}

pub async fn stop() -> anyhow::Result<()> {
    tracing::info!("Stopping infrastructure services...");
    let status = Command::new("docker")
        .args(["compose", "down"])
        .status()?;

    if status.success() {
        println!("Infrastructure services stopped.");
    } else {
        anyhow::bail!("Failed to stop infrastructure services");
    }
    Ok(())
}
