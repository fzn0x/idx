//! `idx ownership search | cross-holders | concentration | graph`:
//! analytics over the local KSEI register.
//! The variants live in `ownership::OwnershipCmd`; their arguments and
//! handlers live here.

use chrono::NaiveDate;
use clap::Args;

use crate::error::Result;
use crate::model::{ConcentrationSort, HolderOrigin, Symbol};
use crate::output::json::{
    SCHEMA_OWNERSHIP_CONCENTRATION, SCHEMA_OWNERSHIP_CROSS_HOLDERS, SCHEMA_OWNERSHIP_GRAPH,
    SCHEMA_OWNERSHIP_SEARCH,
};
use crate::output::{self, table_ownership_analytics as tbl};
use crate::ownership::{self, analytics};

use super::ownership::open;
use super::{emit, Context};

/// 0 means "all", like every other idx `--limit`.
fn limit(n: usize) -> Option<usize> {
    (n > 0).then_some(n)
}

#[derive(Debug, Args)]
pub struct SearchArgs {
    /// Part of a holder's name (person or institution); words in any order, quotes optional.
    #[arg(value_name = "QUERY", required = true, num_args = 1..)]
    pub query: Vec<String>,
    /// Matches shown; 0 for all.
    #[arg(long, short = 'n', default_value_t = 20)]
    pub limit: usize,
    /// Release in which issuers are counted (default: newest imported).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_of: Option<NaiveDate>,
}

#[derive(Debug, Args)]
pub struct CrossHoldersArgs {
    /// Minimum number of issuers held above 1 %.
    #[arg(long, alias = "min-tickers", default_value_t = 2, value_name = "N")]
    pub min_issuers: usize,
    /// KSEI investor type, case-insensitive substring: individual, corporate, "mutual fund", ...
    #[arg(long = "type", value_name = "TYPE")]
    pub investor_type: Option<String>,
    /// KSEI local/foreign flag: foreign (A) or local (L).
    #[arg(long, value_enum)]
    pub origin: Option<HolderOrigin>,
    /// Holders shown, most issuers first; 0 for all.
    #[arg(long, short = 'n', default_value_t = 25)]
    pub limit: usize,
    /// Release date (default: newest imported).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_of: Option<NaiveDate>,
}

#[derive(Debug, Args)]
pub struct ConcentrationArgs {
    /// Only these issuers, in this order (comma or space separated); default: rank every issuer.
    #[arg(value_name = "SYMBOLS")]
    pub symbols: Vec<String>,
    /// Ranking: hhi (Herfindahl-Hirschman index), top1 (largest stake) or top3.
    #[arg(long, value_enum, default_value_t = ConcentrationSort::Hhi)]
    pub by: ConcentrationSort,
    /// Least concentrated (most widely held) first.
    #[arg(long)]
    pub ascending: bool,
    /// Issuers shown; 0 for all. Ignored with SYMBOLS.
    #[arg(long, short = 'n', default_value_t = 25)]
    pub limit: usize,
    /// Release date (default: newest imported).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_of: Option<NaiveDate>,
}

#[derive(Debug, Args)]
pub struct GraphArgs {
    /// A ticker, or a holder's name or key (quotes optional).
    #[arg(value_name = "ROOT", required = true, num_args = 1..)]
    pub root: Vec<String>,
    /// Hops to walk, 1 to 6: from a ticker, 1 = its holders, 2 = their other holdings.
    #[arg(long, default_value_t = 2)]
    pub depth: usize,
    /// Ignore holdings below this percent (5 keeps major stakes only).
    #[arg(long, default_value_t = 0.0, value_name = "PCT")]
    pub min_pct: f64,
    /// Stop after this many nodes (1 to 1000); the output then says truncated.
    #[arg(long, default_value_t = 100, value_name = "N")]
    pub max_nodes: usize,
    /// Treat ROOT as a holder's name even when it is also a ticker.
    #[arg(long)]
    pub holder: bool,
    /// Print Graphviz DOT instead of a table (wins over -o and the configured output).
    #[arg(long)]
    pub dot: bool,
    /// Release date (default: newest imported).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub as_of: Option<NaiveDate>,
}

pub async fn search(ctx: &Context, a: SearchArgs) -> Result<()> {
    let query = a.query.join(" ");
    let db = open(ctx)?;
    let s = ownership::search(&db, &query, a.as_of, limit(a.limit))?;
    emit(ctx, SCHEMA_OWNERSHIP_SEARCH, s, |r| {
        tbl::search(r, ctx.color)
    })
}

pub async fn cross_holders(ctx: &Context, a: CrossHoldersArgs) -> Result<()> {
    let filter = analytics::CrossFilter {
        min_issuers: a.min_issuers,
        investor_type: a.investor_type,
        origin: a.origin,
    };
    let db = open(ctx)?;
    let s = ownership::cross_holders(&db, &filter, limit(a.limit), a.as_of)?;
    emit(ctx, SCHEMA_OWNERSHIP_CROSS_HOLDERS, s, |r| {
        tbl::cross_holders(r, ctx.color)
    })
}

pub async fn concentration(ctx: &Context, a: ConcentrationArgs) -> Result<()> {
    let symbols = if a.symbols.is_empty() {
        None
    } else {
        Some(Symbol::parse_list(&a.symbols)?)
    };
    let db = open(ctx)?;
    let s = ownership::concentration(
        &db,
        symbols.as_deref(),
        a.by,
        a.ascending,
        limit(a.limit),
        a.as_of,
    )?;
    emit(ctx, SCHEMA_OWNERSHIP_CONCENTRATION, s, |r| {
        tbl::concentration(r, ctx.color)
    })
}

pub async fn graph(ctx: &Context, a: GraphArgs) -> Result<()> {
    let root = a.root.join(" ");
    let db = open(ctx)?;
    let s = ownership::graph(
        &db,
        &root,
        a.holder,
        a.depth,
        a.min_pct,
        a.max_nodes,
        a.as_of,
    )?;
    if a.dot {
        for w in &s.warnings {
            output::print_stderr(&format!("warning: {w}"));
        }
        return output::print_stdout(&analytics::to_dot(&s.data));
    }
    emit(ctx, SCHEMA_OWNERSHIP_GRAPH, s, |r| tbl::graph(r, ctx.color))
}
