use std::io;

use anyhow::{Context, Result};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use xpdelve::app;
use xpdelve::cli::{Cli, Command};
use xpdelve::config::{self, Config};
use xpdelve::theme::Theme;
use xpdelve::trace;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Version) => {
            println!("app: xpdelve");
            println!("version: {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(Command::Info) => {
            print_info(&cli)?;
            return Ok(());
        }
        Some(Command::Completion { shell }) => {
            let mut command = Cli::command();
            let name = command.get_name().to_owned();
            let shell: clap_complete::Shell = shell.into();
            generate(shell, &mut command, name, &mut io::stdout());
            return Ok(());
        }
        None => {}
    }

    let Some(resource) = cli.root_resource()? else {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };
    let config = Config::load(&cli)?;
    app::run(&cli, resource, config).await
}

fn print_info(cli: &Cli) -> Result<()> {
    let path = cli.config.clone().unwrap_or_else(config::default_path);
    let config = Config::load(cli).context("configuration check failed")?;
    println!("app: xpdelve");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    println!("config: {}", path.display());
    println!("trace program: {}", config.trace.program);
    println!(
        "trace program available: {}",
        trace::executable_available(&config.trace.program).unwrap_or(false)
    );
    println!("read only: {}", config.read_only);
    println!(
        "configured skin: {}",
        config.skin.name.as_deref().unwrap_or("auto")
    );
    let theme = Theme::resolve(&config.skin, config.ui.color)?;
    println!("resolved skin: {}", theme.resolved_name);
    println!("colors enabled: {}", theme.colors_enabled);
    println!("platform: {}", std::env::consts::OS);
    println!("architecture: {}", std::env::consts::ARCH);
    Ok(())
}
