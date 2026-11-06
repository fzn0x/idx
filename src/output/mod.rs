//! Rendering: tables for people, JSON for programs.
//!
//! Tables go to stdout; diagnostics go to stderr. JSON output is a stable
//! envelope documented in `docs/json-output.md` and printable as JSON Schema
//! with `idx schema <name>`.

use std::io::Write;

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
pub mod table_exchange;
pub mod table_news;
pub mod table_ownership;
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

pub fn print_stderr(text: &str) {
    crate::progress::suspend(|| {
        let _ = writeln!(std::io::stderr(), "{text}");
    });
}
