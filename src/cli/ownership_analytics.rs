//! `idx ownership search | cross-holders | concentration | graph | resolve`:
//! analytics over the local KSEI register and the user's spelling rules.
//! The variants live in `ownership::OwnershipCmd`; their arguments and
//! handlers live here.

use chrono::NaiveDate;
use clap::{Args, Subcommand};

use crate::error::{IdxError, Result};
use crate::model::{ConcentrationSort, HolderOrigin, Symbol};
use crate::output::json::{
    SCHEMA_OWNERSHIP_CONCENTRATION, SCHEMA_OWNERSHIP_CROSS_HOLDERS, SCHEMA_OWNERSHIP_GRAPH,
    SCHEMA_OWNERSHIP_RESOLUTION, SCHEMA_OWNERSHIP_RESOLUTIONS, SCHEMA_OWNERSHIP_RESOLVE_CANDIDATES,
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

fn rule_id(s: &str) -> std::result::Result<i64, String> {
    s.trim()
        .trim_start_matches('#')
        .parse::<i64>()
        .map_err(|_| format!("{s:?} is not a rule number (see `idx ownership resolve list`)"))
}

#[derive(Debug, Subcommand)]
pub enum ResolveCmd {
    /// Pairs of holders that may be one, with the evidence (read-only).
    #[command(
        alias = "unresolved",
        after_help = "Examples:\n  idx ownership resolve candidates\n  idx -o json ownership resolve candidates --limit 0\n\nrenamed: the same share count left one name and reached the other in the same issuer between\ntwo consecutive months. similar name: the same words once legal forms (PT, TBK, LTD, PTE,\nLIMITED, ...) and titles (DRS, DR, IR, H, ...) are dropped, in any order. Listed together: both\nnames appear in one issuer in one month, which often means two different people."
    )]
    Candidates {
        /// Pairs shown; 0 for all.
        #[arg(long, short = 'n', default_value_t = 50)]
        limit: usize,
    },
    /// Merge holders: every spelling of each MERGE holder joins KEEP, in every month.
    #[command(
        after_help = "Examples:\n  idx ownership resolve merge \"DRS SURONO SUBEKTI\" \"SURONO SUBEKTI\" --dry-run\n  idx ownership resolve merge \"UOB KAY HIAN PRIVATE LIMITED\" \"UOB KAY HIAN PTE LTD\"\n\nHolders are named by key or display name, exactly (see idx ownership search)."
    )]
    Merge {
        /// The holder that stays.
        #[arg(value_name = "KEEP")]
        keep: String,
        /// Holders folded into KEEP.
        #[arg(value_name = "MERGE", required = true, num_args = 1..)]
        merge: Vec<String>,
        /// Also merge where both are listed in one issuer in one month (their stakes are added up).
        #[arg(long)]
        fold: bool,
        /// Show what would change and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Move one spelling (exactly as KSEI printed it) to another holder.
    #[command(
        after_help = "Examples:\n  idx ownership resolve map \"IR. T. PERMADI RACHMAT\" \"T PERMADI RACHMAT\"\n  idx ownership resolve map \"PT. X\" \"X HOLDINGS\" --new\n\n`idx ownership search` lists each holder's spellings."
    )]
    Map {
        /// A spelling as printed in the register.
        #[arg(value_name = "SPELLING")]
        spelling: String,
        /// The holder it belongs to (key or display name), or with --new the name of a new holder.
        #[arg(value_name = "HOLDER")]
        holder: String,
        /// Create HOLDER as a new holder.
        #[arg(long)]
        new: bool,
        /// Also move it where HOLDER is listed in the same issuer and month (stakes added up).
        #[arg(long)]
        fold: bool,
        /// Show what would change and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Every rule stored in the database.
    List,
    /// Remove a rule and move back what it moved.
    Undo {
        /// Rule number from `resolve list` (1 or #1).
        #[arg(value_name = "ID", value_parser = rule_id)]
        id: i64,
        /// Show what would change and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
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
            output::print_notice(&format!("warning: {w}"));
        }
        return output::print_stdout(&analytics::to_dot(&s.data));
    }
    emit(ctx, SCHEMA_OWNERSHIP_GRAPH, s, |r| tbl::graph(r, ctx.color))
}

pub async fn resolve(ctx: &Context, cmd: ResolveCmd) -> Result<()> {
    match cmd {
        ResolveCmd::Candidates { limit: n } => {
            let db = open(ctx)?;
            let s = ownership::resolve_candidates(&db, limit(n))?;
            emit(ctx, SCHEMA_OWNERSHIP_RESOLVE_CANDIDATES, s, |r| {
                tbl::resolve_candidates(r, ctx.color)
            })
        }
        ResolveCmd::Merge {
            keep,
            merge,
            fold,
            dry_run,
        } => {
            let mut db = open(ctx)?;
            let s = ownership::resolve_merge(&mut db, &keep, &merge, fold, dry_run)?;
            emit(ctx, SCHEMA_OWNERSHIP_RESOLUTION, s, |r| {
                tbl::resolution_reports(r)
            })
        }
        ResolveCmd::Map {
            spelling,
            holder,
            new,
            fold,
            dry_run,
        } => {
            let mut db = open(ctx)?;
            let s = ownership::resolve_map(&mut db, &spelling, &holder, new, fold, dry_run)?;
            emit(ctx, SCHEMA_OWNERSHIP_RESOLUTION, s, |r| {
                tbl::resolution_reports(r)
            })
        }
        ResolveCmd::List => {
            let db = open(ctx)?;
            let s = ownership::resolutions(&db)?;
            emit(ctx, SCHEMA_OWNERSHIP_RESOLUTIONS, s, |r| {
                tbl::resolutions(r, ctx.color)
            })
        }
        ResolveCmd::Undo { id, dry_run } => {
            if id <= 0 {
                return Err(IdxError::Usage(format!("no resolution #{id}")));
            }
            let mut db = open(ctx)?;
            let s = ownership::resolve_undo(&mut db, id, dry_run)?;
            emit(ctx, SCHEMA_OWNERSHIP_RESOLUTION, s, |r| {
                tbl::resolution_reports(r)
            })
        }
    }
}
