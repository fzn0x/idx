//! `idx watch`: a plain-text watchlist and the commands that read it.
//!
//! The list lives next to the config file (`watchlist.txt`, override with
//! `IDX_WATCHLIST`). `idx watch` quotes it; `idx watch returns` runs the
//! performance table over it; add/remove/list/clear edit it.

use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::error::{IdxError, Result};
use crate::model::{Symbol, Watchlist, IDX_EXCHANGE};
use crate::output::json::{SCHEMA_QUOTES, SCHEMA_RETURNS, SCHEMA_WATCHLIST};
use crate::output::{table, table_reports as reports};
use crate::providers::Sourced;
use crate::service;

use super::{emit, input, Context};

#[derive(Debug, Args)]
pub struct WatchArgs {
    #[command(subcommand)]
    pub cmd: Option<WatchCmd>,
}

#[derive(Debug, Subcommand)]
pub enum WatchCmd {
    /// Add tickers to the list (`-` reads them from stdin).
    Add {
        #[arg(required = true, value_name = "SYMBOLS")]
        symbols: Vec<String>,
    },
    /// Remove tickers from the list (`-` reads them from stdin).
    #[command(alias = "rm")]
    Remove {
        #[arg(required = true, value_name = "SYMBOLS")]
        symbols: Vec<String>,
    },
    /// Show the list and where it is stored.
    #[command(alias = "ls")]
    List,
    /// Empty the list.
    Clear,
    /// Price returns for every ticker on the list, with IHSG for comparison.
    Returns,
}

pub fn path(ctx: &Context) -> PathBuf {
    std::env::var_os("IDX_WATCHLIST")
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.settings.config_path.with_file_name("watchlist.txt"))
}

pub fn load(path: &std::path::Path) -> Result<Vec<Symbol>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => {
            return Err(IdxError::Other(format!(
                "watchlist {}: {e}",
                path.display()
            )))
        }
    };
    let mut out: Vec<Symbol> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        // Entries are stored canonically (BBCA, D05.SI), so a later
        // IDX_EXCHANGE never changes what a saved line means.
        let sym = Symbol::parse_on(line, IDX_EXCHANGE)
            .map_err(|e| IdxError::Config(format!("watchlist line {}: {e}", i + 1)))?;
        if !out.contains(&sym) {
            out.push(sym);
        }
    }
    Ok(out)
}

fn save(path: &std::path::Path, symbols: &[Symbol]) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = String::from("# idx watchlist: one ticker per line\n");
    for s in symbols {
        text.push_str(s.as_str());
        text.push('\n');
    }
    std::fs::write(path, text)?;
    Ok(())
}

fn report(path: &std::path::Path, symbols: Vec<Symbol>) -> Sourced<Watchlist> {
    Sourced::fresh(
        Watchlist {
            path: path.display().to_string(),
            symbols,
        },
        "local-file",
    )
}

fn list_table(w: &Watchlist) -> String {
    if w.symbols.is_empty() {
        return format!(
            "Watchlist is empty ({}). Add tickers with `idx watch add BBCA BBRI`.\n",
            w.path
        );
    }
    let names: Vec<&str> = w.symbols.iter().map(Symbol::as_str).collect();
    format!(
        "{} ({} tickers)\n{}\n",
        w.path,
        names.len(),
        names.join(" ")
    )
}

pub async fn run(ctx: &Context, args: WatchArgs) -> Result<()> {
    let file = path(ctx);
    let mut symbols = load(&file)?;
    match args.cmd {
        Some(WatchCmd::Add { symbols: raw }) => {
            for s in Symbol::parse_list_on(input::symbol_args(&raw)?, &ctx.settings.exchange)? {
                if !symbols.contains(&s) {
                    symbols.push(s);
                }
            }
            save(&file, &symbols)?;
            emit(ctx, SCHEMA_WATCHLIST, report(&file, symbols), list_table)
        }
        Some(WatchCmd::Remove { symbols: raw }) => {
            let drop = Symbol::parse_list_on(input::symbol_args(&raw)?, &ctx.settings.exchange)?;
            symbols.retain(|s| !drop.contains(s));
            save(&file, &symbols)?;
            emit(ctx, SCHEMA_WATCHLIST, report(&file, symbols), list_table)
        }
        Some(WatchCmd::Clear) => {
            save(&file, &[])?;
            emit(ctx, SCHEMA_WATCHLIST, report(&file, vec![]), list_table)
        }
        Some(WatchCmd::List) => emit(ctx, SCHEMA_WATCHLIST, report(&file, symbols), list_table),
        Some(WatchCmd::Returns) => {
            let symbols = non_empty(symbols, &file)?;
            let bench = Symbol::parse("^JKSE")?;
            let result = service::returns(ctx.provider.as_ref(), &symbols, Some(&bench)).await?;
            emit(ctx, SCHEMA_RETURNS, result, |d| {
                reports::returns(d, ctx.color)
            })
        }
        None => {
            let symbols = non_empty(symbols, &file)?;
            let result = ctx.provider.quotes(&symbols).await?;
            emit(ctx, SCHEMA_QUOTES, result, |d| table::quotes(d, ctx.color))
        }
    }
}

fn non_empty(symbols: Vec<Symbol>, file: &std::path::Path) -> Result<Vec<Symbol>> {
    if symbols.is_empty() {
        return Err(IdxError::Usage(format!(
            "watchlist is empty ({}); add tickers with `idx watch add BBCA BBRI`",
            file.display()
        )));
    }
    Ok(symbols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_parses_comments_and_dedupes() {
        let dir = std::env::temp_dir().join(format!("idx-watch-{}", std::process::id()));
        let file = dir.join("watchlist.txt");
        assert!(
            load(&file).unwrap().is_empty(),
            "missing file is an empty list"
        );
        let syms = vec![
            Symbol::parse("BBCA").unwrap(),
            Symbol::parse("TLKM").unwrap(),
        ];
        save(&file, &syms).unwrap();
        std::fs::write(
            &file,
            format!(
                "{}\nbbca   # again\n\n# comment\nBBRI.JK\n",
                std::fs::read_to_string(&file).unwrap()
            ),
        )
        .unwrap();
        let got: Vec<String> = load(&file)
            .unwrap()
            .iter()
            .map(|s| s.as_str().to_string())
            .collect();
        assert_eq!(got, ["BBCA", "TLKM", "BBRI"]);
        std::fs::write(&file, "BB CA\n").unwrap();
        assert!(load(&file).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn other_markets_are_stored_with_their_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("watchlist.txt");
        let syms = vec![
            Symbol::parse_on("D05", "SI").unwrap(),
            Symbol::parse_on("BBCA.JK", "SI").unwrap(),
        ];
        save(&file, &syms).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.ends_with("D05.SI\nBBCA\n"), "{text}");
        // Loaded the same way whatever IDX_EXCHANGE says later.
        assert_eq!(load(&file).unwrap(), syms);
    }
}
