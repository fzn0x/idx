//! Rendering: tables for people, JSON for programs.
//!
//! Tables go to stdout; diagnostics go to stderr. JSON output is a stable
//! envelope documented in `docs/json-output.md` and printable as JSON Schema
//! with `idx schema <name>`.

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{IdxError, Result};

pub mod csv;
pub mod format;
pub mod json;
pub mod jsonl;
pub mod table;
pub mod table_bandar;
pub mod table_broker;
pub mod table_composition;
pub mod table_exchange;
pub mod table_insights;
pub mod table_institutions;
pub mod table_intraday;
pub mod table_lenses;
pub mod table_news;
pub mod table_ownership;
pub mod table_ownership_analytics;
pub mod table_ownership_sync;
pub mod table_reports;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// Human-readable table.
    Table,
    /// Machine-readable JSON envelope.
    Json,
    /// One compact JSON record per line (JSON Lines); see [`jsonl`].
    Jsonl,
    /// The main list in the payload as comma-separated rows (spreadsheets, shell pipelines).
    Csv,
}

impl OutputFormat {
    /// `json` and `jsonl`: errors go to stderr as one JSON line.
    pub fn is_json(self) -> bool {
        matches!(self, OutputFormat::Json | OutputFormat::Jsonl)
    }
}

impl std::str::FromStr for OutputFormat {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <OutputFormat as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            IdxError::Config(format!(
                "unknown output format {s:?}; use table, json, jsonl or csv"
            ))
        })
    }
}

impl std::fmt::Display for OutputFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OutputFormat::Table => "table",
            OutputFormat::Json => "json",
            OutputFormat::Jsonl => "jsonl",
            OutputFormat::Csv => "csv",
        })
    }
}

/// What the user asked for with `-o`, `IDX_OUTPUT` or `output`: a format, or
/// `auto`, which [`OutputSetting::resolve`] turns into one for this run.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum OutputSetting {
    /// A table when stdout is a terminal, the JSON envelope when it is piped or captured.
    Auto,
    /// Human-readable table.
    Table,
    /// Machine-readable JSON envelope.
    Json,
    /// One compact JSON record per line (JSON Lines).
    Jsonl,
    /// The main list in the payload as comma-separated rows.
    Csv,
}

impl OutputSetting {
    /// The format for this run. `auto` is a table on a terminal and JSON
    /// otherwise; it never picks jsonl or csv.
    pub fn resolve(self, stdout_is_terminal: bool) -> OutputFormat {
        match self {
            OutputSetting::Auto if stdout_is_terminal => OutputFormat::Table,
            OutputSetting::Auto => OutputFormat::Json,
            OutputSetting::Table => OutputFormat::Table,
            OutputSetting::Json => OutputFormat::Json,
            OutputSetting::Jsonl => OutputFormat::Jsonl,
            OutputSetting::Csv => OutputFormat::Csv,
        }
    }
}

impl std::str::FromStr for OutputSetting {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <OutputSetting as clap::ValueEnum>::from_str(s.trim(), true).map_err(|_| {
            IdxError::Config(format!(
                "unknown output format {s:?}; use auto, table, json, jsonl or csv"
            ))
        })
    }
}

impl std::fmt::Display for OutputSetting {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OutputSetting::Auto => "auto",
            OutputSetting::Table => "table",
            OutputSetting::Json => "json",
            OutputSetting::Jsonl => "jsonl",
            OutputSetting::Csv => "csv",
        })
    }
}

/// Prints a value that has no envelope (`version`, `config get`, ...) in a
/// machine format: the pretty document for `json`, one line per array item
/// (or one line) for `jsonl`. Returns `false` for `table` and `csv`, where
/// the caller prints its own text.
pub fn print_value<T: Serialize>(format: OutputFormat, v: &T) -> Result<bool> {
    match format {
        OutputFormat::Json => print_stdout(&json::to_pretty(v)?).map(|_| true),
        OutputFormat::Jsonl => {
            print_stdout(&jsonl::render_value(&serde_json::to_value(v)?)?).map(|_| true)
        }
        OutputFormat::Table | OutputFormat::Csv => Ok(false),
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "lowercase")]
pub enum ColorMode {
    Auto,
    Always,
    Never,
}

impl ColorMode {
    /// Whether to emit ANSI colours, given whether stdout is a terminal.
    pub fn enabled(&self, stdout_is_tty: bool) -> bool {
        match self {
            ColorMode::Auto => stdout_is_tty,
            ColorMode::Always => true,
            ColorMode::Never => false,
        }
    }
}

impl std::str::FromStr for ColorMode {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <ColorMode as clap::ValueEnum>::from_str(s, true).map_err(|_| {
            IdxError::Config(format!(
                "unknown color mode {s:?}; use auto, always or never"
            ))
        })
    }
}

impl std::fmt::Display for ColorMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ColorMode::Auto => "auto",
            ColorMode::Always => "always",
            ColorMode::Never => "never",
        })
    }
}

/// Writes to stdout, treating a closed pipe (`idx ... | head`) as success.
pub fn print_stdout(text: &str) -> Result<()> {
    crate::progress::finish();
    let stdout = std::io::stdout();
    let mut lock = stdout.lock();
    match lock.write_all(text.as_bytes()).and_then(|_| lock.flush()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Errors, hints and instructions the user must act on: always printed.
/// Informational lines go through [`print_notice`].
pub fn print_stderr(text: &str) {
    crate::progress::suspend(|| {
        let _ = writeln!(std::io::stderr(), "{text}");
    });
}

static QUIET: AtomicBool = AtomicBool::new(false);

/// `--quiet` / `IDX_QUIET`: hide [`print_notice`] lines for this process.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Warnings, the stale notice and confirmations on stderr; `--quiet` hides
/// them. Never used for errors or for the payload.
pub fn print_notice(text: &str) {
    if !is_quiet() {
        print_stderr(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_resolves_against_the_terminal() {
        use OutputFormat as F;
        use OutputSetting as S;
        for (setting, tty, want) in [
            (S::Auto, true, F::Table),
            (S::Auto, false, F::Json),
            (S::Table, false, F::Table),
            (S::Json, true, F::Json),
            (S::Jsonl, true, F::Jsonl),
            (S::Jsonl, false, F::Jsonl),
            (S::Csv, true, F::Csv),
        ] {
            assert_eq!(setting.resolve(tty), want, "{setting} tty={tty}");
        }
    }

    #[test]
    fn settings_parse_case_insensitively_and_list_every_value() {
        assert_eq!(
            "AUTO".parse::<OutputSetting>().unwrap(),
            OutputSetting::Auto
        );
        assert_eq!(
            " JsonL ".parse::<OutputSetting>().unwrap(),
            OutputSetting::Jsonl
        );
        assert_eq!("csv".parse::<OutputFormat>().unwrap(), OutputFormat::Csv);
        let e = "yaml".parse::<OutputSetting>().unwrap_err();
        assert_eq!(e.exit_code(), 6);
        assert!(
            e.to_string().contains("auto, table, json, jsonl or csv"),
            "{e}"
        );
        for s in ["auto", "table", "json", "jsonl", "csv"] {
            assert_eq!(s.parse::<OutputSetting>().unwrap().to_string(), s);
        }
        assert!(OutputFormat::Jsonl.is_json() && OutputFormat::Json.is_json());
        assert!(!OutputFormat::Csv.is_json() && !OutputFormat::Table.is_json());
    }
}
