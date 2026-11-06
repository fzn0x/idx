//! `idx ownership ...`: KSEI's monthly ">1 % holders" register and KSEI's
//! monthly composition archive ("Kepemilikan Efek") in local SQLite.

use std::path::PathBuf;

use chrono::NaiveDate;
use clap::{Args, Subcommand};

use crate::error::{IdxError, Result};
use crate::model::Symbol;
use crate::output::json::{
    SCHEMA_OWNERSHIP_CHANGES, SCHEMA_OWNERSHIP_COMPOSITION, SCHEMA_OWNERSHIP_DISCOVER,
    SCHEMA_OWNERSHIP_ENTITY, SCHEMA_OWNERSHIP_FOREIGN, SCHEMA_OWNERSHIP_FOREIGN_FLOW,
    SCHEMA_OWNERSHIP_HOLDERS, SCHEMA_OWNERSHIP_IMPORT, SCHEMA_OWNERSHIP_RELEASES,
};
use crate::output::{self, table_composition, table_ownership as tbl};
use crate::ownership::discover::Family;
use crate::ownership::{self, composition, ksei, OwnershipDb};
use crate::providers::Sourced;

use super::{emit, Context};

#[derive(Debug, Subcommand)]
pub enum OwnershipCmd {
    /// Import a monthly "Pemegang Saham di Atas 1%" file (XLSX or PDF) or a KSEI composition archive (ZIP or TXT) into the local database.
    #[command(
        after_help = "Examples:\n  idx ownership import --latest                 # newest monthly file\n  idx ownership import --all                    # every month not imported yet\n  idx ownership import --file peng-2026-08-00017-satu-persen.xlsx\n  idx ownership import --url https://www.idx.co.id/Media/.../satu-persen.xlsx\n  idx ownership import --latest --family composition    # KSEI's newest Kepemilikan Efek archive\n  idx ownership import --file BalanceposEfek20261030.zip   # or the Balancepos20261030.txt inside\n  idx ownership import --url https://www.ksei.co.id/storage/Download/BalanceposEfek20261030.zip\n\nidx.co.id is behind Cloudflare. --latest/--all try idx.co.id (browser-like request, then\ncurl-impersonate if installed) and otherwise use the GitHub mirror that CI refreshes three\ntimes a day with the same IDX files, checking each file's SHA-256. IDX_OWNERSHIP_MIRROR=off\ndisables the mirror. A refusal by Cloudflare is the JSON error code `blocked` (exit 3).\n\n--file and --url tell the family from the content: a KSEI composition archive goes to its own\ntables (see `idx ownership composition`), never into the register of holders above 1%."
    )]
    Import(ImportArgs),

    /// Holders above 1 % of one ticker.
    #[command(
        after_help = "Examples:\n  idx ownership ticker BBCA\n  idx ownership ticker BBRI --as-of 2026-07-31\n  idx -o json ownership ticker KREN      # accounts, controller_or_affiliate per holder\n\nKSEI lists a holder once per SID and once per scrip account without one; those lines are\nsummed into one row (Accts column, `accounts` in JSON). From the 30 Oct 2026 register a\nControl column shows CONTROLLER / AFFILIATE OF THE CONTROLLER as the issuer reported it to IDX\nfor the previous month. Rows dated other than the release are kept but not listed."
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
        after_help = "Examples:\n  idx ownership changes --from 2026-07-31 --to 2026-08-31\n  idx ownership changes --from 2026-07-31 --to 2026-08-31 --symbol BBCA\n  idx ownership changes --from 2026-02-27 --to 2026-08-31 --min-change 1 --limit 100\n  idx ownership changes --from 2026-07-31 --to 2026-08-31 --limit 100 --offset 100   # page 2\n\nThe summary (counts, net percentage points by foreign/local and investor type, issuer flow,\nlargest rises and falls) always covers every match; --limit/--offset page the rows.\n\nA holder's register lines (one per account, e.g. scrip and scripless) are summed before\ncomparing; `accounts_from`/`accounts_to` show how many. With the same shares on both sides, a\nmove within the rounding of summed lines (0.005 pp per line) is not a change."
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
    #[command(
        after_help = "Examples:\n  idx ownership releases\n  idx -o json ownership releases\n\n`rows` counts register lines, `positions` the holders per ticker after summing each holder's\naccounts, `other_date_rows` the lines dated other than the release (kept, left out of its\nholdings), `controller_column` whether the file has CONTROLLER_OR_AFFILIATE. Releases imported\nby idx 0.8.1 or older show no positions and are named in a warning until\n`idx ownership import --all --force` re-imports them."
    )]
    Releases,

    /// List the files currently published on idx.co.id and ksei.co.id (needs network).
    #[command(
        after_help = "Examples:\n  idx ownership discover                                 # monthly >1% registers on idx.co.id\n  idx ownership discover --family all --limit 2           # every family, newest two each\n  idx ownership discover --family composition --limit 24  # KSEI Kepemilikan Efek archives (ksei.co.id)\n  idx -o json ownership discover --family above5          # family and importable on every entry\n\nFamilies: above1 (importable), above5, investor-type, classification (idx.co.id, listed only),\ncomposition (KSEI \"Kepemilikan Efek\" ZIP on ksei.co.id, importable). --limit counts per\nfamily; without it every idx.co.id file is listed and one ksei.co.id page (8 months); 0 lists\nall (the composition walks up to 10 pages). A family that fails is a warning while the others\nanswer; when every family fails the command fails, with JSON code `blocked` if Cloudflare\nrefused idx.co.id."
    )]
    Discover(DiscoverArgs),

    /// KSEI's monthly composition of one stock: local and foreign shares by investor type.
    #[command(
        after_help = "Examples:\n  idx ownership composition BBCA\n  idx ownership composition TLKM --as-of 2026-09-30\n  idx -o json ownership composition BBCA | jq '.data.foreign_pct'\n\nKSEI's monthly \"Kepemilikan Efek\" archive (ksei.co.id) splits every share held in KSEI\naccounts (scripless) between local and foreign holders in nine investor types: insurance,\ncorporate, pension fund, financial institution, individual, mutual fund, securities company,\nfoundation, others. Percentages are of the scripless shares; shares still held as\ncertificates (scrip) are outside the archive, so scripless_pct can be far below 100 (BBCA,\n30 Oct 2026: 42.55 %, its controlling holder's stake is scrip). It covers every holder, not\nonly those above 1 %, and names none.\n\nThe first call downloads the newest month (about 125 KB) into the local database; later\ncalls read it and ask ksei.co.id again only once a newer month can exist. --as-of imports a\nlisted month that is missing. --offline never downloads."
    )]
    Composition {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Month (the archive's date, e.g. 2026-10-30; default: newest).
        #[arg(long, value_name = "YYYY-MM-DD")]
        as_of: Option<NaiveDate>,
    },

    /// Print the database path.
    Path,
}

#[derive(Debug, Args)]
// The mode group names its four arguments: a struct-level `#[group]` would
// take in every field, `--force` and `--family` too, and refuse
// `--all --force` (idx 0.8.1 did).
#[command(group(
    clap::ArgGroup::new("mode")
        .required(true)
        .multiple(false)
        .args(["file", "url", "latest", "all"])
))]
pub struct ImportArgs {
    /// Local file: a register (XLSX or PDF) or a KSEI composition archive (ZIP or TXT).
    #[arg(long, value_name = "PATH")]
    pub file: Option<PathBuf>,
    /// Direct URL of a register (XLSX or PDF) or a KSEI composition archive (ZIP or TXT).
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    /// Newest listed file that is not imported yet.
    #[arg(long)]
    pub latest: bool,
    /// Every listed file that is not imported yet.
    #[arg(long)]
    pub all: bool,
    /// What --latest/--all import: the register of holders above 1 % (idx.co.id or the mirror) or KSEI's composition archive (ksei.co.id). --file and --url detect it from the content.
    #[arg(long, value_enum, value_name = "FAMILY", conflicts_with_all = ["file", "url"])]
    pub family: Option<ImportFamily>,
    /// Re-import even when the release date already exists (with --latest/--all).
    #[arg(long)]
    pub force: bool,
}

/// The families `import --latest/--all` can fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ImportFamily {
    /// The register of holders above 1 % (default).
    Above1,
    /// KSEI's composition archive.
    Composition,
}

#[derive(Debug, Args)]
pub struct DiscoverArgs {
    /// Which files to list.
    #[arg(long, value_enum, value_name = "FAMILY", default_value_t = Family::Above1)]
    pub family: Family,
    /// Entries per family, newest first; 0 for all. Default: every idx.co.id file, 8 composition months.
    #[arg(long, short = 'n', value_name = "N")]
    pub limit: Option<usize>,
}

/// The `provider` of a discovery envelope: where the files came from.
fn discovery_provider(files: &[crate::model::DiscoveredFile]) -> &'static str {
    let has = |o: &str| files.iter().any(|f| f.origin == o);
    match (
        has("idx.co.id"),
        has(ownership::mirror::ORIGIN),
        has(ksei::ORIGIN),
    ) {
        (false, true, false) => ownership::mirror::ORIGIN,
        (false, false, true) => ksei::PROVIDER,
        (true, false, true) => "idx+ksei",
        (false, true, true) => "github-mirror+ksei",
        _ => "idx",
    }
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
        OwnershipCmd::Discover(args) => {
            let offline = ctx.settings.cache_mode == crate::http::CacheMode::Offline;
            let (files, notes) = ownership::discover::discover_families(
                &ctx.fetcher,
                args.family,
                args.limit,
                offline,
                ctx.settings.timeout(),
            )
            .await?;
            let provider = discovery_provider(&files);
            let mut s = Sourced::fresh(files, provider);
            s.warnings.extend(notes);
            let db = open(ctx)?;
            let have: Vec<NaiveDate> = db.releases()?.iter().map(|r| r.as_of).collect();
            let have_composition = composition::months(&db)?;
            for (family, list) in [("above1", &have), ("composition", &have_composition)] {
                let missing: Vec<String> = s
                    .data
                    .iter()
                    .filter(|f| f.family == family && !list.contains(&f.as_of))
                    .map(|f| f.as_of.to_string())
                    .collect();
                if !missing.is_empty() {
                    s.warnings.push(format!(
                        "{}not imported yet: {}",
                        if family == "above1" {
                            ""
                        } else {
                            "composition "
                        },
                        missing.join(", ")
                    ));
                }
            }
            let show_family = args.family != Family::Above1;
            emit(ctx, SCHEMA_OWNERSHIP_DISCOVER, s, |r| {
                tbl::discovered(r, &have, &have_composition, show_family, ctx.color)
            })
        }
        OwnershipCmd::Composition { symbol, as_of } => {
            let sym = Symbol::parse(&symbol)?;
            let mut db = open(ctx)?;
            let s = composition::report(
                &mut db,
                &ctx.fetcher,
                sym.as_str(),
                as_of,
                download_timeout(ctx),
                crate::service::today_wib(),
            )
            .await?;
            emit(ctx, SCHEMA_OWNERSHIP_COMPOSITION, s, |r| {
                table_composition::composition(r, ctx.color)
            })
        }
        OwnershipCmd::Path => {
            output::print_stdout(&format!("{}\n", ctx.settings.ownership_db.display()))
        }
    }
}

/// Downloads get at least a minute whatever `--timeout` says.
fn download_timeout(ctx: &Context) -> std::time::Duration {
    ctx.settings
        .timeout()
        .max(std::time::Duration::from_secs(60))
}

async fn import(ctx: &Context, args: ImportArgs) -> Result<()> {
    let mut db = open(ctx)?;
    let timeout = download_timeout(ctx);
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
        let (files, notes) = match args.family.unwrap_or(ImportFamily::Above1) {
            ImportFamily::Above1 => ownership::discover::discover(timeout).await?,
            ImportFamily::Composition => {
                // --latest needs the first listing page only; --all walks them.
                let s =
                    ksei::list(&ctx.fetcher, if args.latest { ksei::PER_PAGE } else { 0 }).await?;
                (s.data, s.warnings)
            }
        };
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
