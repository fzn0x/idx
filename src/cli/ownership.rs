//! `idx ownership ...`: KSEI's monthly ">1 % holders" register in local SQLite.

use std::path::PathBuf;

use chrono::NaiveDate;
use clap::{Args, Subcommand};

use crate::error::{IdxError, Result};
use crate::model::Symbol;
use crate::output::json::{
    SCHEMA_OWNERSHIP_CHANGES, SCHEMA_OWNERSHIP_DISCOVER, SCHEMA_OWNERSHIP_ENTITY,
    SCHEMA_OWNERSHIP_FOREIGN, SCHEMA_OWNERSHIP_FOREIGN_FLOW, SCHEMA_OWNERSHIP_HOLDERS,
    SCHEMA_OWNERSHIP_IMPORT, SCHEMA_OWNERSHIP_RELEASES,
};
use crate::output::{self, table_ownership as tbl};
use crate::ownership::{self, OwnershipDb};
use crate::providers::Sourced;

use super::{emit, Context};

#[derive(Debug, Subcommand)]
pub enum OwnershipCmd {
    /// Import a monthly "Pemegang Saham di Atas 1%" file (XLSX or PDF) into the local database.
    #[command(
        after_help = "Examples:\n  idx ownership import --latest                 # newest monthly file\n  idx ownership import --all                    # every month not imported yet\n  idx ownership import --file peng-2026-08-00017-satu-persen.xlsx\n  idx ownership import --url https://www.idx.co.id/Media/.../satu-persen.xlsx\n\nidx.co.id is behind Cloudflare. --latest/--all try idx.co.id (browser-like request, then\ncurl-impersonate if installed) and otherwise use the GitHub mirror that CI refreshes three\ntimes a day with the same IDX files, checking each file's SHA-256. IDX_OWNERSHIP_MIRROR=off\ndisables the mirror."
    )]
    Import(ImportArgs),

    /// Holders above 1 % of one ticker.
    #[command(
        after_help = "Examples:\n  idx ownership ticker BBCA\n  idx ownership ticker BBRI --as-of 2026-07-31"
    )]
    Ticker {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Release date (default: newest imported). See `idx ownership releases`.
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_of: Option<NaiveDate>,
    },

    /// Every position of a person or institution (substring match on the normalised name).
    #[command(
        after_help = "Examples:\n  idx ownership entity \"Dwimuria Investama\"\n  idx ownership entity \"statens pensjonsfond\" --as-of 2026-08-31"
    )]
    Entity {
        #[arg(value_name = "NAME")]
        name: String,
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_of: Option<NaiveDate>,
    },

    /// Month-over-month changes: new holders, exits and position changes, with a summary.
    #[command(
        after_help = "Examples:\n  idx ownership changes --from 2026-07-31 --to 2026-08-31\n  idx ownership changes --from 2026-07-31 --to 2026-08-31 --symbol BBCA\n  idx ownership changes --from 2026-02-27 --to 2026-08-31 --min-change 1 --limit 100\n  idx ownership changes --from 2026-07-31 --to 2026-08-31 --limit 100 --offset 100   # page 2\n\nThe summary (counts, net percentage points by foreign/local and investor type, issuer flow,\nlargest rises and falls) always covers every match; --limit/--offset page the rows."
    )]
    Changes {
        #[arg(long, value_name = "YYYY-MM-DD")]
        from: NaiveDate,
        #[arg(long, value_name = "YYYY-MM-DD")]
        to: NaiveDate,
        /// Restrict to one ticker.
        #[arg(long, short = 's', value_name = "SYMBOL")]
        symbol: Option<String>,
        /// Ignore moves smaller than this many percentage points.
        #[arg(long, default_value_t = 0.0, value_name = "PP")]
        min_change: f64,
        /// Rows per page (largest moves first); 0 for all.
        #[arg(long, short = 'n', default_value_t = 50)]
        limit: usize,
        /// Rows to skip before the page.
        #[arg(long, default_value_t = 0, value_name = "N")]
        offset: usize,
    },

    /// Disclosed foreign vs local stake: one issuer's trend across releases, or market-wide flow between two releases.
    #[command(
        after_help = "Examples:\n  idx ownership foreign BBCA                      # foreign share of the >1% register, month by month\n  idx ownership foreign                           # where foreign holders moved between the two newest releases\n  idx ownership foreign --from 2026-02-27 --to 2026-08-31 --limit 30\n\nKSEI flags each listed holder L (local) or A (asing / foreign). Sums cover holders above 1% only."
    )]
    Foreign {
        #[arg(value_name = "SYMBOL")]
        symbol: Option<String>,
        /// Earlier release for the market-wide view (default: the one before --to).
        #[arg(long, value_name = "YYYY-MM-DD", conflicts_with = "symbol")]
        from: Option<NaiveDate>,
        /// Later release for the market-wide view (default: newest).
        #[arg(long, value_name = "YYYY-MM-DD", conflicts_with = "symbol")]
        to: Option<NaiveDate>,
        /// Rows in the market-wide view; 0 for all.
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
    },

    /// Imported releases in the local database.
    Releases,

    /// List the monthly files currently published on idx.co.id (needs network).
    Discover,

    /// Print the database path.
    Path,
}

#[derive(Debug, Args)]
#[group(id = "mode", required = true, multiple = false)]
pub struct ImportArgs {
    /// Local XLSX or PDF file.
    #[arg(long, value_name = "PATH", group = "mode")]
    pub file: Option<PathBuf>,
    /// Direct URL of an XLSX or PDF file.
    #[arg(long, value_name = "URL", group = "mode")]
    pub url: Option<String>,
    /// Newest file on the IDX page that is not imported yet.
    #[arg(long, group = "mode")]
    pub latest: bool,
    /// Every file on the IDX page that is not imported yet.
    #[arg(long, group = "mode")]
    pub all: bool,
    /// Re-import even when the release date already exists (with --latest/--all).
    #[arg(long)]
    pub force: bool,
}

fn open(ctx: &Context) -> Result<OwnershipDb> {
    OwnershipDb::open(&ctx.settings.ownership_db)
}

fn local<T>(data: T) -> Sourced<T> {
    Sourced::fresh(data, ownership::PROVIDER)
}

pub async fn run(ctx: &Context, cmd: OwnershipCmd) -> Result<()> {
    match cmd {
        OwnershipCmd::Import(args) => import(ctx, args).await,
        OwnershipCmd::Ticker { symbol, as_of } => {
            let sym = Symbol::parse(&symbol)?;
            let db = open(ctx)?;
            let report = ownership::holders(&db, sym.as_str(), as_of)?;
            emit(ctx, SCHEMA_OWNERSHIP_HOLDERS, report, |r| {
                tbl::holders(r, ctx.color)
            })
        }
        OwnershipCmd::Entity { name, as_of } => {
            if name.trim().len() < 3 {
                return Err(IdxError::Usage(
                    "entity name needs at least 3 characters".into(),
                ));
            }
            let db = open(ctx)?;
            let report = ownership::entity(&db, &name, as_of)?;
            emit(ctx, SCHEMA_OWNERSHIP_ENTITY, report, |r| {
                tbl::entity(r, ctx.color)
            })
        }
        OwnershipCmd::Changes {
            from,
            to,
            symbol,
            min_change,
            limit,
            offset,
        } => {
            let sym = symbol.map(|s| Symbol::parse(&s)).transpose()?;
            let db = open(ctx)?;
            let report = ownership::changes(
                &db,
                from,
                to,
                sym.as_ref().map(Symbol::as_str),
                min_change,
                if limit == 0 { None } else { Some(limit) },
                offset,
            )?;
            emit(ctx, SCHEMA_OWNERSHIP_CHANGES, report, |r| {
                tbl::changes(r, ctx.color)
            })
        }
        OwnershipCmd::Foreign {
            symbol,
            from,
            to,
            limit,
        } => {
            let db = open(ctx)?;
            match symbol {
                Some(s) => {
                    let sym = Symbol::parse(&s)?;
                    let report = ownership::foreign_trend(&db, sym.as_str())?;
                    emit(ctx, SCHEMA_OWNERSHIP_FOREIGN, report, |r| {
                        tbl::foreign(r, ctx.color)
                    })
                }
                None => {
                    let report = ownership::foreign_flow(
                        &db,
                        from,
                        to,
                        if limit == 0 { None } else { Some(limit) },
                    )?;
                    emit(ctx, SCHEMA_OWNERSHIP_FOREIGN_FLOW, report, |r| {
                        tbl::foreign_flow(r, ctx.color)
                    })
                }
            }
        }
        OwnershipCmd::Releases => {
            let db = open(ctx)?;
            let s = ownership::releases(&db)?;
            emit(ctx, SCHEMA_OWNERSHIP_RELEASES, s, |r| {
                tbl::releases(r, &ctx.settings.ownership_db, ctx.color)
            })
        }
        OwnershipCmd::Discover => {
            if ctx.settings.cache_mode == crate::http::CacheMode::Offline {
                return Err(IdxError::OfflineMiss("discover needs the network".into()));
            }
            let (files, notes) = ownership::discover::discover(ctx.settings.timeout()).await?;
            let via_mirror = files
                .first()
                .is_some_and(|f| f.origin == ownership::mirror::ORIGIN);
            let mut s = Sourced::fresh(
                files,
                if via_mirror {
                    ownership::mirror::ORIGIN
                } else {
                    "idx"
                },
            );
            s.warnings.extend(notes);
            let have: Vec<NaiveDate> = open(ctx)?.releases()?.iter().map(|r| r.as_of).collect();
            let missing: Vec<String> = s
                .data
                .iter()
                .filter(|f| !have.contains(&f.as_of))
                .map(|f| f.as_of.to_string())
                .collect();
            if !missing.is_empty() {
                s.warnings
                    .push(format!("not imported yet: {}", missing.join(", ")));
            }
            emit(ctx, SCHEMA_OWNERSHIP_DISCOVER, s, |r| {
                tbl::discovered(r, &have, ctx.color)
            })
        }
        OwnershipCmd::Path => {
            output::print_stdout(&format!("{}\n", ctx.settings.ownership_db.display()))
        }
    }
}

async fn import(ctx: &Context, args: ImportArgs) -> Result<()> {
    let mut db = open(ctx)?;
    let timeout = ctx
        .settings
        .timeout()
        .max(std::time::Duration::from_secs(60));
    let (reports, warnings) = if let Some(path) = &args.file {
        let (r, w) = ownership::import_file(&mut db, path)?;
        (vec![r], w)
    } else if let Some(url) = &args.url {
        if ctx.settings.cache_mode == crate::http::CacheMode::Offline {
            return Err(IdxError::OfflineMiss("--url needs the network".into()));
        }
        let (r, w) = ownership::import_url(&mut db, url, timeout).await?;
        (vec![r], w)
    } else {
        if ctx.settings.cache_mode == crate::http::CacheMode::Offline {
            return Err(IdxError::OfflineMiss(
                "--latest/--all need the network".into(),
            ));
        }
        let (files, notes) = ownership::discover::discover(timeout).await?;
        let (reports, mut warnings) =
            ownership::import_discovered(&mut db, &files, args.latest, args.force, timeout).await?;
        warnings.splice(0..0, notes.clone());
        if reports.is_empty() {
            let mut s = local(Vec::<crate::model::ImportReport>::new());
            s.warnings.extend(notes);
            s.warnings.push(format!(
                "nothing new: newest published file ({}) is already imported",
                files
                    .first()
                    .map(|f| f.as_of.to_string())
                    .unwrap_or_default()
            ));
            return emit(ctx, SCHEMA_OWNERSHIP_IMPORT, s, |r| tbl::imports(r));
        }
        (reports, warnings)
    };
    let mut s = local(reports);
    s.warnings = warnings;
    emit(ctx, SCHEMA_OWNERSHIP_IMPORT, s, |r| tbl::imports(r))
}
