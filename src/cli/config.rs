use clap::Subcommand;

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Manage provider configurations
    Providers,
    /// Initialize or manage secrets (SOPS + age)
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
}

#[derive(Subcommand)]
pub enum SecretsAction {
    /// Initialize SOPS + age encryption for this project
    Init,
    /// Rotate the age encryption key
    Rotate,
}

pub async fn execute(action: ConfigAction) -> anyhow::Result<()> {
    match action {
        ConfigAction::Providers => {
            todo!("config providers command")
        }
        ConfigAction::Secrets { action } => match action {
            SecretsAction::Init => {
                todo!("secrets init command")
            }
            SecretsAction::Rotate => {
                todo!("secrets rotate command")
            }
        },
    }
}
