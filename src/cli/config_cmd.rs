//! `idx config ...`

use clap::Subcommand;

use crate::config::{self, FileConfig, KEYS, TEMPLATE};
use crate::error::{IdxError, Result};
use crate::output;

use super::GlobalOpts;

#[derive(Debug, Subcommand)]
pub enum ConfigCmd {
    /// Write a commented starter config file (refuses to overwrite without --force).
    Init {
        #[arg(long)]
        force: bool,
    },
    /// Set a key, e.g. `idx config set provider yahoo` or `idx config set cache.quote_ttl_secs 120`.
    Set { key: String, value: String },
    /// Show effective settings, or a single key from the config file.
    Get { key: Option<String> },
    /// Print the config file path in use.
    Path,
}

pub fn run(global: &GlobalOpts, cmd: ConfigCmd) -> Result<()> {
    let path = global
        .config
        .clone()
        .unwrap_or_else(config::default_config_path);
    let format = super::requested_format(global);
    match cmd {
        ConfigCmd::Init { force } => {
            if path.exists() && !force {
                return Err(IdxError::Config(format!(
                    "{} already exists (use --force to overwrite)",
                    path.display()
                )));
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, TEMPLATE)?;
            output::print_stderr(&format!("wrote {}", path.display()));
            Ok(())
        }
        ConfigCmd::Set { key, value } => {
            let mut cfg = FileConfig::load(&path)?;
            cfg.set(&key, &value)?;
            cfg.save(&path)?;
            output::print_stderr(&format!("{key} = {value} ({})", path.display()));
            Ok(())
        }
        ConfigCmd::Get { key: Some(key) } => {
            let cfg = FileConfig::load(&path)?;
            let v = cfg.get(&key)?;
            if output::print_value(format, &serde_json::json!({ key: v }))? {
                return Ok(());
            }
            output::print_stdout(&format!("{}\n", v.unwrap_or_else(|| "(unset)".into())))
        }
        ConfigCmd::Get { key: None } => {
            let settings = super::resolve_settings(global)?;
            if output::print_value(format, &settings)? {
                return Ok(());
            }
            let mut s = String::new();
            s.push_str(&format!(
                "config file : {}{}\n",
                settings.config_path.display(),
                if settings.config_path.exists() {
                    ""
                } else {
                    " (not present)"
                }
            ));
            s.push_str(&format!("provider    : {}\n", settings.provider));
            if settings.output_setting == crate::output::OutputSetting::Auto {
                s.push_str("output      : auto (table on a terminal, json when piped)\n");
            } else {
                s.push_str(&format!("output      : {}\n", settings.output));
            }
            s.push_str(&format!("color       : {}\n", settings.color));
            s.push_str(&format!("timeout     : {}s\n", settings.timeout_secs));
            s.push_str(&format!("cache dir   : {}\n", settings.cache_dir.display()));
            s.push_str(&format!(
                "cache ttl   : quote {}s · history {}s\n",
                settings.quote_ttl_secs, settings.history_ttl_secs
            ));
            s.push_str(&format!("keys        : {}\n", KEYS.join(", ")));
            output::print_stdout(&s)
        }
        ConfigCmd::Path => output::print_stdout(&format!("{}\n", path.display())),
    }
}
