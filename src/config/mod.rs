//! Configuration with the precedence: CLI flags > environment > config file > defaults.
//!
//! File location (override with `IDX_CONFIG` or `--config`):
//! - Linux / macOS: `$XDG_CONFIG_HOME/idx/config.toml`, else `~/.config/idx/config.toml`
//! - Windows: `%APPDATA%\idx\config.toml`
//!
//! Cache location (override with `IDX_CACHE_DIR` or `cache.dir`):
//! - Linux / macOS: `$XDG_CACHE_HOME/idx`, else `~/.cache/idx`
//! - Windows: `%LOCALAPPDATA%\idx\cache`

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{IdxError, Result};
use crate::http::CacheMode;
use crate::output::{ColorMode, OutputFormat};
use crate::providers::ProviderKind;

pub const ENV_PREFIX: &str = "IDX_";

/// What the TOML file may contain. Every field is optional.
#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FileConfig {
    /// `auto` | `yahoo` | `msn`
    pub provider: Option<String>,
    /// `table` | `json`
    pub output: Option<String>,
    /// `auto` | `always` | `never`
    pub color: Option<String>,
    /// Per-request timeout.
    pub timeout_secs: Option<u64>,
    pub cache: CacheSection,
    pub msn: MsnSection,
    pub ownership: OwnershipSection,
    pub stockbit: StockbitSection,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct StockbitSection {
    /// Session file holding the (rotating) Stockbit tokens.
    pub session: Option<PathBuf>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct OwnershipSection {
    /// SQLite file holding the imported shareholder registers.
    pub db: Option<PathBuf>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct MsnSection {
    /// Overrides MSN's built-in public web key.
    pub api_key: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct CacheSection {
    pub dir: Option<PathBuf>,
    pub quote_ttl_secs: Option<u64>,
    pub history_ttl_secs: Option<u64>,
    pub fundamentals_ttl_secs: Option<u64>,
    pub financials_ttl_secs: Option<u64>,
}

impl FileConfig {
    pub fn load(path: &Path) -> Result<FileConfig> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text)
                .map_err(|e| IdxError::Config(format!("{}: {e}", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FileConfig::default()),
            Err(e) => Err(IdxError::Config(format!("{}: {e}", path.display()))),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| IdxError::Config(format!("serialise config: {e}")))?;
        std::fs::write(path, body)?;
        Ok(())
    }

    /// Sets a dotted key such as `provider` or `cache.quote_ttl_secs`.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let bad = || IdxError::Config(format!("{key}: invalid value {value:?}"));
        match key {
            "provider" => {
                value.parse::<ProviderKind>()?;
                self.provider = Some(value.to_string());
            }
            "output" => {
                value.parse::<OutputFormat>()?;
                self.output = Some(value.to_string());
            }
            "color" => {
                value.parse::<ColorMode>()?;
                self.color = Some(value.to_string());
            }
            "timeout_secs" => self.timeout_secs = Some(value.parse().map_err(|_| bad())?),
            "cache.dir" => self.cache.dir = Some(PathBuf::from(value)),
            "cache.quote_ttl_secs" => {
                self.cache.quote_ttl_secs = Some(value.parse().map_err(|_| bad())?)
            }
            "cache.history_ttl_secs" => {
                self.cache.history_ttl_secs = Some(value.parse().map_err(|_| bad())?)
            }
            "cache.fundamentals_ttl_secs" => {
                self.cache.fundamentals_ttl_secs = Some(value.parse().map_err(|_| bad())?)
            }
            "cache.financials_ttl_secs" => {
                self.cache.financials_ttl_secs = Some(value.parse().map_err(|_| bad())?)
            }
            "msn.api_key" => self.msn.api_key = Some(value.to_string()),
            "ownership.db" => self.ownership.db = Some(PathBuf::from(value)),
            "stockbit.session" => self.stockbit.session = Some(PathBuf::from(value)),
            _ => {
                return Err(IdxError::Config(format!(
                    "unknown key {key:?}; valid keys: {}",
                    KEYS.join(", ")
                )))
            }
        }
        Ok(())
    }

    /// Reads a dotted key back as a string, `None` if unset.
    pub fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(match key {
            "provider" => self.provider.clone(),
            "output" => self.output.clone(),
            "color" => self.color.clone(),
            "timeout_secs" => self.timeout_secs.map(|v| v.to_string()),
            "cache.dir" => self.cache.dir.as_ref().map(|p| p.display().to_string()),
            "cache.quote_ttl_secs" => self.cache.quote_ttl_secs.map(|v| v.to_string()),
            "cache.history_ttl_secs" => self.cache.history_ttl_secs.map(|v| v.to_string()),
            "cache.fundamentals_ttl_secs" => {
                self.cache.fundamentals_ttl_secs.map(|v| v.to_string())
            }
            "cache.financials_ttl_secs" => self.cache.financials_ttl_secs.map(|v| v.to_string()),
            "msn.api_key" => self.msn.api_key.clone(),
            "ownership.db" => self.ownership.db.as_ref().map(|p| p.display().to_string()),
            "stockbit.session" => self
                .stockbit
                .session
                .as_ref()
                .map(|p| p.display().to_string()),
            _ => {
                return Err(IdxError::Config(format!(
                    "unknown key {key:?}; valid keys: {}",
                    KEYS.join(", ")
                )))
            }
        })
    }
}

pub const KEYS: &[&str] = &[
    "provider",
    "output",
    "color",
    "timeout_secs",
    "cache.dir",
    "cache.quote_ttl_secs",
    "cache.history_ttl_secs",
    "cache.fundamentals_ttl_secs",
    "cache.financials_ttl_secs",
    "msn.api_key",
    "ownership.db",
    "stockbit.session",
];

/// Template written by `idx config init`.
pub const TEMPLATE: &str = r#"# idx configuration. Every key is optional.
# Precedence: CLI flags > IDX_* environment variables > this file > defaults.

# Data source: "auto" tries providers in order and falls back when one fails.
# provider = "auto"          # auto | yahoo | msn      (env: IDX_PROVIDER)

# Default output format.
# output = "table"           # table | json | csv      (env: IDX_OUTPUT)

# ANSI colours in tables.
# color = "auto"             # auto | always | never   (env: IDX_COLOR, NO_COLOR)

# Per-request network timeout in seconds.
# timeout_secs = 15          # (env: IDX_TIMEOUT_SECS)

[cache]
# dir = "/path/to/cache"     # (env: IDX_CACHE_DIR)
# quote_ttl_secs = 60        # (env: IDX_QUOTE_TTL_SECS)
# history_ttl_secs = 3600    # (env: IDX_HISTORY_TTL_SECS)
# fundamentals_ttl_secs = 21600  # profile, ratios, earnings, MSN insights (env: IDX_FUNDAMENTALS_TTL_SECS)
# financials_ttl_secs = 86400    # statements (env: IDX_FINANCIALS_TTL_SECS)

[msn]
# MSN Money needs its own public web-client key, which idx ships with.
# Only set this if MSN rotates the key.
# api_key = ""               # (env: IDX_MSN_API_KEY)

[ownership]
# SQLite file for the imported KSEI shareholder registers.
# db = "/path/to/ownership.sqlite"   # (env: IDX_OWNERSHIP_DB)

[stockbit]
# Opt-in realtime provider (--provider stockbit). Run `idx stockbit login` once;
# the session file below holds your rotating tokens. Seed tokens through the
# environment (IDX_STOCKBIT_REFRESH_TOKEN / IDX_STOCKBIT_ACCESS_TOKEN) for an
# unattended deployment, never in this file.
# session = "/path/to/stockbit-session.json"   # (env: IDX_STOCKBIT_SESSION)
"#;

/// Values coming from the command line. `None`/`false` means "not given".
#[derive(Debug, Default, Clone)]
pub struct CliOverrides {
    pub provider: Option<ProviderKind>,
    pub output: Option<OutputFormat>,
    pub no_color: bool,
    pub offline: bool,
    pub no_cache: bool,
    pub timeout_secs: Option<u64>,
    pub cache_dir: Option<PathBuf>,
}

/// Fully resolved, ready-to-use settings.
#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    pub provider: ProviderKind,
    pub output: OutputFormat,
    pub color: ColorMode,
    pub timeout_secs: u64,
    pub cache_dir: PathBuf,
    #[serde(serialize_with = "ser_cache_mode")]
    pub cache_mode: CacheMode,
    pub quote_ttl_secs: u64,
    pub history_ttl_secs: u64,
    pub fundamentals_ttl_secs: u64,
    pub financials_ttl_secs: u64,
    /// Not serialised; `config get` reports whether it is set.
    #[serde(skip)]
    pub msn_api_key: Option<String>,
    pub ownership_db: PathBuf,
    /// Stockbit session file (`idx stockbit login` writes it).
    pub stockbit_session: PathBuf,
    /// Env-seed tokens for the Stockbit session; not serialised.
    #[serde(skip)]
    pub stockbit_access_seed: Option<String>,
    #[serde(skip)]
    pub stockbit_refresh_seed: Option<String>,
    pub config_path: PathBuf,
}

fn ser_cache_mode<S: serde::Serializer>(
    m: &CacheMode,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    s.serialize_str(match m {
        CacheMode::Normal => "normal",
        CacheMode::Offline => "offline",
        CacheMode::Bypass => "bypass",
    })
}

impl Settings {
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }
    pub fn quote_ttl(&self) -> Duration {
        Duration::from_secs(self.quote_ttl_secs)
    }
    pub fn history_ttl(&self) -> Duration {
        Duration::from_secs(self.history_ttl_secs)
    }
    pub fn fundamentals_ttl(&self) -> Duration {
        Duration::from_secs(self.fundamentals_ttl_secs)
    }
    pub fn financials_ttl(&self) -> Duration {
        Duration::from_secs(self.financials_ttl_secs)
    }
}

pub const DEFAULT_TIMEOUT_SECS: u64 = 15;
pub const DEFAULT_QUOTE_TTL_SECS: u64 = 60;
pub const DEFAULT_HISTORY_TTL_SECS: u64 = 3600;
pub const DEFAULT_FUNDAMENTALS_TTL_SECS: u64 = 6 * 3600;
pub const DEFAULT_FINANCIALS_TTL_SECS: u64 = 24 * 3600;

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn default_config_path() -> PathBuf {
    if cfg!(windows) {
        dirs::config_dir()
            .unwrap_or_else(home)
            .join("idx")
            .join("config.toml")
    } else if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(xdg).join("idx").join("config.toml")
    } else {
        home().join(".config").join("idx").join("config.toml")
    }
}

/// Default Stockbit session file, beside the config file.
pub fn default_stockbit_session() -> PathBuf {
    default_config_path()
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(home)
        .join("stockbit-session.json")
}

pub fn default_cache_dir() -> PathBuf {
    if cfg!(windows) {
        dirs::cache_dir()
            .unwrap_or_else(home)
            .join("idx")
            .join("cache")
    } else if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").filter(|v| !v.is_empty()) {
        PathBuf::from(xdg).join("idx")
    } else {
        home().join(".cache").join("idx")
    }
}

fn truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Merge the three layers. `env` is injectable so precedence is unit-testable.
pub fn resolve(
    cli: &CliOverrides,
    file: &FileConfig,
    config_path: PathBuf,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Settings> {
    let env_s = |k: &str| env(&format!("{ENV_PREFIX}{k}")).filter(|v| !v.trim().is_empty());

    let provider = match (&cli.provider, env_s("PROVIDER"), &file.provider) {
        (Some(p), _, _) => *p,
        (None, Some(e), _) => e.parse()?,
        (None, None, Some(f)) => f.parse()?,
        _ => ProviderKind::Auto,
    };

    let output = match (&cli.output, env_s("OUTPUT"), &file.output) {
        (Some(o), _, _) => *o,
        (None, Some(e), _) => e.parse()?,
        (None, None, Some(f)) => f.parse()?,
        _ => OutputFormat::Table,
    };

    let color = if cli.no_color || env("NO_COLOR").is_some_and(|v| !v.is_empty()) {
        ColorMode::Never
    } else {
        match (env_s("COLOR"), &file.color) {
            (Some(e), _) => e.parse()?,
            (None, Some(f)) => f.parse()?,
            _ => ColorMode::Auto,
        }
    };

    let parse_u64 = |k: &str, v: String| -> Result<u64> {
        v.trim().parse().map_err(|_| {
            IdxError::Config(format!("{ENV_PREFIX}{k}: expected an integer, got {v:?}"))
        })
    };

    let timeout_secs = match (cli.timeout_secs, env_s("TIMEOUT_SECS"), file.timeout_secs) {
        (Some(t), _, _) => t,
        (None, Some(e), _) => parse_u64("TIMEOUT_SECS", e)?,
        (None, None, Some(f)) => f,
        _ => DEFAULT_TIMEOUT_SECS,
    };
    if timeout_secs == 0 {
        return Err(IdxError::Config("timeout_secs must be > 0".into()));
    }

    let cache_dir = cli
        .cache_dir
        .clone()
        .or_else(|| env_s("CACHE_DIR").map(PathBuf::from))
        .or_else(|| file.cache.dir.clone())
        .unwrap_or_else(default_cache_dir);

    let offline = cli.offline || env_s("OFFLINE").is_some_and(|v| truthy(&v));
    let no_cache = cli.no_cache || env_s("NO_CACHE").is_some_and(|v| truthy(&v));
    if offline && no_cache {
        return Err(IdxError::Usage(
            "--offline and --no-cache cannot be combined".into(),
        ));
    }
    let cache_mode = if offline {
        CacheMode::Offline
    } else if no_cache {
        CacheMode::Bypass
    } else {
        CacheMode::Normal
    };

    let quote_ttl_secs = match (env_s("QUOTE_TTL_SECS"), file.cache.quote_ttl_secs) {
        (Some(e), _) => parse_u64("QUOTE_TTL_SECS", e)?,
        (None, Some(f)) => f,
        _ => DEFAULT_QUOTE_TTL_SECS,
    };
    let history_ttl_secs = match (env_s("HISTORY_TTL_SECS"), file.cache.history_ttl_secs) {
        (Some(e), _) => parse_u64("HISTORY_TTL_SECS", e)?,
        (None, Some(f)) => f,
        _ => DEFAULT_HISTORY_TTL_SECS,
    };
    let fundamentals_ttl_secs = match (
        env_s("FUNDAMENTALS_TTL_SECS"),
        file.cache.fundamentals_ttl_secs,
    ) {
        (Some(e), _) => parse_u64("FUNDAMENTALS_TTL_SECS", e)?,
        (None, Some(f)) => f,
        _ => DEFAULT_FUNDAMENTALS_TTL_SECS,
    };
    let financials_ttl_secs = match (env_s("FINANCIALS_TTL_SECS"), file.cache.financials_ttl_secs) {
        (Some(e), _) => parse_u64("FINANCIALS_TTL_SECS", e)?,
        (None, Some(f)) => f,
        _ => DEFAULT_FINANCIALS_TTL_SECS,
    };
    let msn_api_key = env_s("MSN_API_KEY").or_else(|| file.msn.api_key.clone());
    let ownership_db = env_s("OWNERSHIP_DB")
        .map(PathBuf::from)
        .or_else(|| file.ownership.db.clone())
        .unwrap_or_else(crate::ownership::default_db_path);
    let stockbit_session = env_s("STOCKBIT_SESSION")
        .map(PathBuf::from)
        .or_else(|| file.stockbit.session.clone())
        .unwrap_or_else(default_stockbit_session);
    // Seed tokens live only in the environment (never the config file), for an
    // unattended deployment such as the web worker.
    let stockbit_access_seed = env_s("STOCKBIT_ACCESS_TOKEN");
    let stockbit_refresh_seed = env_s("STOCKBIT_REFRESH_TOKEN");

    Ok(Settings {
        provider,
        output,
        color,
        timeout_secs,
        cache_dir,
        cache_mode,
        quote_ttl_secs,
        history_ttl_secs,
        fundamentals_ttl_secs,
        financials_ttl_secs,
        msn_api_key,
        ownership_db,
        stockbit_session,
        stockbit_access_seed,
        stockbit_refresh_seed,
        config_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    #[test]
    fn defaults_when_nothing_is_set() {
        let s = resolve(
            &CliOverrides::default(),
            &FileConfig::default(),
            "x".into(),
            &|_| None,
        )
        .unwrap();
        assert_eq!(s.provider, ProviderKind::Auto);
        assert_eq!(s.output, OutputFormat::Table);
        assert_eq!(s.color, ColorMode::Auto);
        assert_eq!(s.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert_eq!(s.cache_mode, CacheMode::Normal);
        assert_eq!(s.quote_ttl_secs, 60);
    }

    #[test]
    fn precedence_cli_over_env_over_file() {
        let file = FileConfig {
            provider: Some("yahoo".into()),
            output: Some("json".into()),
            timeout_secs: Some(5),
            ..Default::default()
        };
        // file only
        let s = resolve(&CliOverrides::default(), &file, "x".into(), &|_| None).unwrap();
        assert_eq!(s.provider, ProviderKind::Yahoo);
        assert_eq!(s.output, OutputFormat::Json);
        assert_eq!(s.timeout_secs, 5);
        // env beats file
        let env = env_from(&[("IDX_PROVIDER", "auto"), ("IDX_TIMEOUT_SECS", "9")]);
        let s = resolve(&CliOverrides::default(), &file, "x".into(), &env).unwrap();
        assert_eq!(s.provider, ProviderKind::Auto);
        assert_eq!(s.timeout_secs, 9);
        assert_eq!(s.output, OutputFormat::Json);
        // cli beats env
        let cli = CliOverrides {
            provider: Some(ProviderKind::Yahoo),
            output: Some(OutputFormat::Table),
            timeout_secs: Some(1),
            ..Default::default()
        };
        let s = resolve(&cli, &file, "x".into(), &env).unwrap();
        assert_eq!(s.provider, ProviderKind::Yahoo);
        assert_eq!(s.output, OutputFormat::Table);
        assert_eq!(s.timeout_secs, 1);
    }

    #[test]
    fn offline_and_color_flags() {
        let env = env_from(&[("IDX_OFFLINE", "1"), ("NO_COLOR", "1")]);
        let s = resolve(
            &CliOverrides::default(),
            &FileConfig::default(),
            "x".into(),
            &env,
        )
        .unwrap();
        assert_eq!(s.cache_mode, CacheMode::Offline);
        assert_eq!(s.color, ColorMode::Never);
        let cli = CliOverrides {
            offline: true,
            no_cache: true,
            ..Default::default()
        };
        assert!(resolve(&cli, &FileConfig::default(), "x".into(), &|_| None).is_err());
    }

    #[test]
    fn bad_env_values_are_config_errors() {
        let env = env_from(&[("IDX_TIMEOUT_SECS", "soon")]);
        let e = resolve(
            &CliOverrides::default(),
            &FileConfig::default(),
            "x".into(),
            &env,
        )
        .unwrap_err();
        assert_eq!(e.exit_code(), 6);
        let env = env_from(&[("IDX_PROVIDER", "bloomberg")]);
        assert!(resolve(
            &CliOverrides::default(),
            &FileConfig::default(),
            "x".into(),
            &env
        )
        .is_err());
    }

    #[test]
    fn file_roundtrip_and_set_get() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        assert_eq!(FileConfig::load(&path).unwrap(), FileConfig::default());
        let mut cfg = FileConfig::default();
        cfg.set("provider", "yahoo").unwrap();
        cfg.set("cache.quote_ttl_secs", "120").unwrap();
        assert!(cfg.set("provider", "nope").is_err());
        assert!(cfg.set("nope", "1").is_err());
        cfg.save(&path).unwrap();
        let back = FileConfig::load(&path).unwrap();
        assert_eq!(back.get("provider").unwrap().as_deref(), Some("yahoo"));
        assert_eq!(
            back.get("cache.quote_ttl_secs").unwrap().as_deref(),
            Some("120")
        );
        assert_eq!(back.get("output").unwrap(), None);
        std::fs::write(&path, "provider = 3").unwrap();
        assert_eq!(FileConfig::load(&path).unwrap_err().exit_code(), 6);
        // The template must itself be valid TOML.
        let t: FileConfig = toml::from_str(TEMPLATE).unwrap();
        assert_eq!(t, FileConfig::default());
    }
}
