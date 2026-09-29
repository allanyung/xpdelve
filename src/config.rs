use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;

use crate::cli::Cli;

const DEFAULT_STDOUT_LIMIT: usize = 256 * 1024 * 1024;
const DEFAULT_STDERR_LIMIT: usize = 1024 * 1024;

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub read_only: bool,
    pub trace: TraceConfig,
    pub ui: UiConfig,
    pub skin: SkinConfig,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TraceConfig {
    pub program: String,
    pub args: Vec<String>,
    pub resource_args: Vec<String>,
    pub context_args: Vec<String>,
    pub namespace_args: Vec<String>,
    pub interval_seconds: u64,
    pub timeout_seconds: Option<u64>,
    pub stdout_limit_bytes: usize,
    pub stderr_limit_bytes: usize,
    pub retry_backoff_max_seconds: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiConfig {
    pub color: ColorMode,
    pub ascii: bool,
    pub horizontal_scroll: bool,
    pub short: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SkinConfig {
    pub name: Option<String>,
    pub colors: HashMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    #[default]
    Auto,
    Always,
    Never,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            read_only: false,
            trace: TraceConfig::default(),
            ui: UiConfig::default(),
            skin: SkinConfig::default(),
        }
    }
}

impl Default for TraceConfig {
    fn default() -> Self {
        Self {
            program: "crossplane".into(),
            args: vec![
                "resource".into(),
                "trace".into(),
                "-o".into(),
                "json".into(),
            ],
            resource_args: vec!["{resource}".into()],
            context_args: vec!["--context".into(), "{context}".into()],
            namespace_args: vec!["--namespace".into(), "{namespace}".into()],
            interval_seconds: 5,
            timeout_seconds: None,
            stdout_limit_bytes: DEFAULT_STDOUT_LIMIT,
            stderr_limit_bytes: DEFAULT_STDERR_LIMIT,
            retry_backoff_max_seconds: 60,
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            color: ColorMode::Auto,
            ascii: false,
            horizontal_scroll: false,
            short: false,
        }
    }
}

impl Config {
    pub fn load(cli: &Cli) -> Result<Self> {
        let path = cli.config.clone().unwrap_or_else(default_path);
        let mut config = if path.exists() {
            let source = fs::read_to_string(&path)
                .with_context(|| format!("failed to read configuration {}", path.display()))?;
            toml::from_str::<Self>(&source)
                .with_context(|| format!("invalid configuration {}", path.display()))?
        } else if cli.config.is_some() {
            bail!("configuration file does not exist: {}", path.display());
        } else {
            Self::default()
        };

        config.validate()?;
        config.apply_cli(cli)?;
        config.validate()?;
        Ok(config)
    }

    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.trace.interval_seconds)
    }

    pub fn timeout(&self) -> Option<Duration> {
        self.trace.timeout_seconds.map(Duration::from_secs)
    }

    fn apply_cli(&mut self, cli: &Cli) -> Result<()> {
        if let Some(command) = &cli.cmd {
            let mut words = shell_words::split(command).context("invalid --cmd shell words")?;
            if words.is_empty() {
                bail!("--cmd cannot be empty");
            }
            self.trace.program = words.remove(0);
            self.trace.args = words;
        }
        if cli.readonly {
            self.read_only = true;
        }
        if cli.short {
            self.ui.short = true;
        }
        if let Some(interval) = cli.watch_interval {
            self.trace.interval_seconds = interval.as_secs();
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.schema_version != 1 {
            bail!(
                "unsupported configuration schema version {}",
                self.schema_version
            );
        }
        if self.trace.program.trim().is_empty() {
            bail!("trace.program cannot be empty");
        }
        if self.trace.interval_seconds < 1 {
            bail!("trace.interval_seconds must be at least 1");
        }
        if self.trace.stdout_limit_bytes == 0 || self.trace.stderr_limit_bytes == 0 {
            bail!("trace output limits must be greater than zero");
        }
        if self.trace.retry_backoff_max_seconds == 0 {
            bail!("trace.retry_backoff_max_seconds must be at least 1");
        }
        validate_placeholder(&self.trace.resource_args, "{resource}", true)?;
        validate_placeholder(&self.trace.context_args, "{context}", false)?;
        validate_placeholder(&self.trace.namespace_args, "{namespace}", false)?;
        crate::theme::validate_skin(&self.skin)?;
        Ok(())
    }
}

fn validate_placeholder(args: &[String], placeholder: &str, required: bool) -> Result<()> {
    let count = args
        .iter()
        .filter(|arg| arg.as_str() == placeholder)
        .count();
    if required && count != 1 {
        bail!("trace.resource_args must contain {placeholder} exactly once");
    }
    if !required && !args.is_empty() && count != 1 {
        bail!("non-empty trace arguments must contain {placeholder} exactly once");
    }
    Ok(())
}

pub fn default_path() -> PathBuf {
    env::var_os("HOME")
        .map_or_else(|| Path::new(".").to_path_buf(), PathBuf::from)
        .join(".config/xpdelve/config.toml")
}

pub fn persist_skin(path: &Path, name: &str) -> Result<()> {
    crate::theme::validate_skin(&SkinConfig {
        name: Some(name.to_owned()),
        colors: HashMap::new(),
    })?;
    let source = if path.exists() {
        fs::read_to_string(path)
            .with_context(|| format!("failed to read configuration {}", path.display()))?
    } else {
        String::new()
    };
    let updated = update_skin_name(&source, name);
    toml::from_str::<Config>(&updated)
        .with_context(|| format!("updated configuration {} is invalid", path.display()))?;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create configuration directory {}",
                parent.display()
            )
        })?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let temporary = path.with_file_name(format!(".{file_name}.tmp-{}", std::process::id()));
    fs::write(&temporary, updated)
        .with_context(|| format!("failed to write configuration {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| {
        let _ = fs::remove_file(&temporary);
        format!("failed to replace configuration {}", path.display())
    })
}

fn update_skin_name(source: &str, name: &str) -> String {
    let quoted = toml::Value::String(name.to_owned()).to_string();
    let mut lines = source.lines().map(str::to_owned).collect::<Vec<_>>();
    let mut current_table = "";
    let mut skin_header = None;
    let mut skin_end = lines.len();
    let mut nested_skin_header = None;

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let declaration = trimmed.split('#').next().unwrap_or_default().trim();
        if declaration.starts_with('[') && declaration.ends_with(']') {
            if skin_header.is_some() && skin_end == lines.len() {
                skin_end = index;
            }
            current_table = declaration;
            if declaration == "[skin]" {
                skin_header = Some(index);
                skin_end = lines.len();
            } else if declaration.starts_with("[skin.") && nested_skin_header.is_none() {
                nested_skin_header = Some(index);
            }
            continue;
        }
        let Some((key, _)) = trimmed.split_once('=') else {
            continue;
        };
        if (current_table.is_empty() && key.trim() == "skin.name")
            || (current_table == "[skin]" && key.trim() == "name")
        {
            let indent = line.len() - line.trim_start().len();
            let comment = line.find('#').map_or("", |comment| &line[comment..]);
            lines[index] = format!(
                "{}{}name = {quoted}{}{}",
                " ".repeat(indent),
                if current_table.is_empty() {
                    "skin."
                } else {
                    ""
                },
                if comment.is_empty() { "" } else { " " },
                comment
            );
            return finish_config(lines, source);
        }
    }

    if let Some(header) = skin_header {
        lines.insert((header + 1).min(skin_end), format!("name = {quoted}"));
    } else {
        let insertion = nested_skin_header.unwrap_or(lines.len());
        let mut block = vec![
            "[skin]".to_owned(),
            format!("name = {quoted}"),
            String::new(),
        ];
        if insertion == lines.len() && lines.last().is_some_and(|line| !line.is_empty()) {
            block.insert(0, String::new());
        }
        lines.splice(insertion..insertion, block);
    }
    finish_config(lines, source)
}

fn finish_config(lines: Vec<String>, source: &str) -> String {
    let mut result = lines.join("\n");
    if source.ends_with('\n') || !result.is_empty() {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().unwrap();
    }

    #[test]
    fn rejects_unknown_fields() {
        let error = toml::from_str::<Config>("schema_version = 1\nunknown = true").unwrap_err();
        assert!(error.to_string().contains("unknown"));
    }

    #[test]
    fn accepts_valid_skin_and_rejects_invalid_skin() {
        let valid = toml::from_str::<Config>(
            "schema_version = 1\n[skin]\nname = 'gruvbox-dark'\n[skin.colors]\nred = '#fb4934'",
        )
        .unwrap();
        valid.validate().unwrap();

        let invalid =
            toml::from_str::<Config>("schema_version = 1\n[skin]\nname = 'missing'").unwrap();
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn updates_skin_without_discarding_other_configuration() {
        let source = "# keep me\nread_only = true\n\n[skin]\n# skin comment\nname = 'nord' # choice\n\n[skin.colors]\nred = '#ff0000'\n";
        let updated = update_skin_name(source, "dracula");
        assert!(updated.contains("# keep me"));
        assert!(updated.contains("name = \"dracula\" # choice"));
        assert!(updated.contains("red = '#ff0000'"));
        let config: Config = toml::from_str(&updated).unwrap();
        assert_eq!(config.skin.name.as_deref(), Some("dracula"));
    }

    #[test]
    fn adds_skin_before_an_existing_colors_table() {
        let source = "[skin.colors]\nred = '#ff0000'\n";
        let updated = update_skin_name(source, "nord");
        let config: Config = toml::from_str(&updated).unwrap();
        assert_eq!(config.skin.name.as_deref(), Some("nord"));
        assert_eq!(
            config.skin.colors.get("red").map(String::as_str),
            Some("#ff0000")
        );
    }
}
