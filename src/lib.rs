//! `idx`: Indonesian stock market (IDX / IHSG) analysis for humans and AI agents.
//!
//! The binary in `src/main.rs` is a thin wrapper over [`cli::run`]. Everything
//! else is a library so the MCP server and tests can reuse the same code paths.
//!
//! Module map (see `AGENTS.md` for the long version):
//! - [`cli`]       clap subcommands and handlers
//! - [`providers`] the `Provider` trait and one module per data source
//! - [`service`]   provider call + local analysis, shared by CLI and MCP
//! - [`mcp`]       the same capabilities as MCP tools over stdio
//! - [`ownership`] KSEI monthly holder register: download, parse, SQLite, queries
//! - [`brokers`]   IDX exchange-member directory (broker code to name)
//! - [`holidays`]  IDX trading holidays (bundled) and trading-day arithmetic
//! - [`astro`]     offline ephemeris, weton and shio for `market astro`
//! - [`model`]     provider-agnostic domain types (the JSON contract)
//! - [`output`]    table and JSON renderers
//! - [`http`]      fetcher that puts the cache in front of the network
//! - [`progress`]  the CLI's loading line on stderr while requests are in flight
//! - [`cache`]     content-addressed file cache
//! - [`config`]    flags > env > file > defaults

pub mod analysis;
pub mod astro;
pub mod brokers;
pub mod cache;
pub mod cli;
pub mod config;
pub mod error;
pub mod holidays;
pub mod http;
pub mod mcp;
pub mod model;
pub mod output;
pub mod ownership;
pub mod progress;
pub mod providers;
pub mod service;
pub mod universe;

pub use error::{IdxError, Result};
