use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod commands;

#[derive(Parser)]
#[command(name = "passhound", about = "Personal password vault and recovery", version)]
struct Cli {
    /// Path to the vault file. Defaults to OS user data dir.
    #[arg(long, global = true)]
    vault: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new vault.
    Init,
    /// Add a new password to the vault.
    Add(commands::add::AddArgs),
    /// List all accounts in the vault.
    List,
    /// Reveal the current password for an account.
    Get(commands::get::GetArgs),
    /// Import passwords from a file or pasted text.
    Import(commands::import::ImportArgs),
    /// Generate ranked recovery candidates from your own history.
    Recover(commands::recover::RecoverArgs),
    /// Tokenize password history and populate base_words.
    Analyze(commands::analyze::AnalyzeArgs),
    /// Manage base words extracted by `analyze`.
    BaseWord(commands::base_word::BaseWordArgs),
}

fn default_vault_path() -> PathBuf {
    directories::ProjectDirs::from("com", "passhound", "passhound")
        .map(|d| d.data_dir().join("vault.db"))
        .unwrap_or_else(|| PathBuf::from("./passhound.db"))
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let vault_path = cli.vault.unwrap_or_else(default_vault_path);
    match cli.command {
        Command::Init => commands::init::run(&vault_path),
        Command::Add(args) => commands::add::run(&vault_path, args),
        Command::List => commands::list::run(&vault_path),
        Command::Get(args) => commands::get::run(&vault_path, args),
        Command::Import(args) => commands::import::run(&vault_path, args),
        Command::Recover(args) => commands::recover::run(&vault_path, args),
        Command::Analyze(args) => commands::analyze::run(&vault_path, args),
        Command::BaseWord(args) => commands::base_word::run(&vault_path, args),
    }
}
