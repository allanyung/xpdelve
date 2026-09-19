use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "xpdelve",
    version,
    about = "A responsive Crossplane resource trace explorer",
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Configuration file. Defaults to ~/.config/xpdelve/config.toml.
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    /// Kubernetes context used by Crossplane and native operations.
    #[arg(long, global = true)]
    pub context: Option<String>,

    /// Namespace passed to the Crossplane trace command.
    #[arg(short = 'n', long, alias = "ns", global = true)]
    pub namespace: Option<String>,

    /// Kubeconfig used by Crossplane and native operations.
    #[arg(long, global = true)]
    pub kubeconfig: Option<PathBuf>,

    /// Legacy trace command prefix, parsed as shell words without a shell.
    #[arg(long, global = true)]
    pub cmd: Option<String>,

    /// Disable all mutating actions.
    #[arg(long, global = true)]
    pub readonly: bool,

    /// Use the compact column set.
    #[arg(long, global = true)]
    pub short: bool,

    /// Disable automatic trace refresh.
    #[arg(long, alias = "nw", global = true)]
    pub no_watch: bool,

    /// Automatic refresh interval.
    #[arg(long, alias = "wi", value_parser = parse_duration, global = true)]
    pub watch_interval: Option<Duration>,

    #[command(subcommand)]
    pub command: Option<Command>,

    /// Root resource as Kind/name or as separate Kind name arguments.
    #[arg(value_name = "RESOURCE", num_args = 0..=2)]
    pub resource: Vec<String>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Print version information.
    Version,
    /// Print build and runtime diagnostics.
    Info,
    /// Generate shell completion.
    Completion {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Shell {
    Bash,
    Elvish,
    Fish,
    Powershell,
    Zsh,
}

impl From<Shell> for clap_complete::Shell {
    fn from(value: Shell) -> Self {
        match value {
            Shell::Bash => Self::Bash,
            Shell::Elvish => Self::Elvish,
            Shell::Fish => Self::Fish,
            Shell::Powershell => Self::PowerShell,
            Shell::Zsh => Self::Zsh,
        }
    }
}

impl Cli {
    pub fn root_resource(&self) -> Result<Option<String>> {
        if self.command.is_some() {
            if !self.resource.is_empty() {
                bail!("a resource cannot be used with a utility command");
            }
            return Ok(None);
        }

        match self.resource.as_slice() {
            [] => Ok(None),
            [qualified] if valid_qualified_resource(qualified) => Ok(Some(qualified.clone())),
            [kind, name] if valid_part(kind) && valid_part(name) => {
                Ok(Some(format!("{kind}/{name}")))
            }
            [_] => bail!("resource must use Kind/name syntax"),
            _ => bail!("resource must use Kind/name or Kind name syntax"),
        }
    }
}

fn valid_qualified_resource(value: &str) -> bool {
    let mut parts = value.split('/');
    matches!((parts.next(), parts.next(), parts.next()), (Some(kind), Some(name), None) if valid_part(kind) && valid_part(name))
}

fn valid_part(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains('/') && !value.chars().any(char::is_whitespace)
}

fn parse_duration(value: &str) -> Result<Duration, String> {
    let value = value.trim();
    if let Some(seconds) = value.strip_suffix('s') {
        return seconds
            .parse::<u64>()
            .map(Duration::from_secs)
            .map_err(|error| error.to_string());
    }
    value
        .parse::<u64>()
        .map(Duration::from_secs)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_both_resource_forms() {
        let slash = Cli::try_parse_from(["xpdelve", "Bucket/example"]).unwrap();
        assert_eq!(
            slash.root_resource().unwrap().as_deref(),
            Some("Bucket/example")
        );

        let separate = Cli::try_parse_from(["xpdelve", "Bucket", "example"]).unwrap();
        assert_eq!(
            separate.root_resource().unwrap().as_deref(),
            Some("Bucket/example")
        );
    }

    #[test]
    fn rejects_incomplete_resource() {
        let cli = Cli::try_parse_from(["xpdelve", "Bucket"]).unwrap();
        assert!(cli.root_resource().is_err());
    }
}
