//! `idx market ...`: the whole board in one call.

use chrono::Datelike;
use clap::Subcommand;

use crate::error::Result;
use crate::model::{
    AstroPeriod, BrokerSort, BrokerSource, CalendarCategory, ForeignSort, NatalChoice, Symbol,
};
use crate::output::json::{
    SCHEMA_ASTRO, SCHEMA_CALENDAR, SCHEMA_HOLIDAYS, SCHEMA_INDICES, SCHEMA_MARKET,
    SCHEMA_MARKET_BROKERS, SCHEMA_MARKET_FOREIGN, SCHEMA_MEMBERS,
};
use crate::output::table_exchange as tx;
use crate::output::table_reports as reports;
use crate::service;
use crate::universe;

use super::{emit, Context};

#[derive(Debug, Subcommand)]
pub enum MarketCmd {
    /// IHSG, breadth, gainers, losers, most traded and heavyweights from one batch of quotes.
    #[command(
        after_help = "Examples:\n  idx market overview\n  idx market overview --top 15\n  idx -o json market overview | jq '.data | {index, advancing, declining}'\n\nWalks the bundled universe (~950 tickers) in batched quote calls; gainers and losers are\nlimited to companies above the liquidity floor (--min-mcap) with volume today."
    )]
    Overview {
        /// Rows per list.
        #[arg(long, short = 'n', default_value_t = 10)]
        top: usize,
        /// Market-cap floor for gainers/losers, in IDR (accepts K/M/B/T).
        #[arg(long, default_value = "1T", value_name = "IDR")]
        min_mcap: String,
        /// Index symbol shown at the top.
        #[arg(long, default_value = "^JKSE", value_name = "INDEX")]
        index: String,
        /// CSV of symbol,name to walk instead of the bundled universe.
        #[arg(long, value_name = "PATH")]
        universe: Option<std::path::PathBuf>,
    },
    /// Every IDX index for one trading day: close, change, range, constituents, value, market cap.
    #[command(
        after_help = "Examples:\n  idx market indices\n  idx market indices LQ45 IDX30 IDX80\n  idx market indices --date 2026-10-16\n  idx -o json market indices IHSG | jq '.data.indices[0].close'\n\nFrom idx.co.id's daily index summary (COMPOSITE is IHSG, ^JKSE on Yahoo). Without --date: the\nlatest session published; weekends and holidays are skipped."
    )]
    Indices {
        /// Index codes to keep (LQ45, IDX30, ...; IHSG means COMPOSITE). Default: all.
        #[arg(value_name = "CODE")]
        codes: Vec<String>,
        /// Trading day, YYYY-MM-DD. Default: the latest session.
        #[arg(long, short = 'd', value_name = "DATE")]
        date: Option<chrono::NaiveDate>,
    },
    /// Foreign net buying and selling across every stock for one session (top foreign / top net).
    #[command(
        after_help = "Examples:\n  idx market foreign\n  idx market foreign --top 20 --date 2026-10-16\n  idx market foreign --sort shares\n  idx -o json market foreign | jq '.data.net_buy[0]'\n\nFrom idx.co.id's daily stock summary, latest session by default (weekends and holidays\nskipped). IDX publishes foreign trading in shares; lists are ranked by net shares × close\n(--sort value, a rupiah scale that is not the traded value) or by net shares."
    )]
    Foreign {
        /// Trading day, YYYY-MM-DD. Default: the latest session.
        #[arg(long, short = 'd', value_name = "DATE")]
        date: Option<chrono::NaiveDate>,
        /// Rows per list; 0 for all.
        #[arg(long, short = 'n', default_value_t = 10)]
        top: usize,
        /// Ranking figure.
        #[arg(long, value_enum, default_value_t = ForeignSort::Value)]
        sort: ForeignSort,
    },
    /// Exchange members ranked by the day's value, volume or frequency (top brokers).
    #[command(
        after_help = "Examples:\n  idx market brokers\n  idx market brokers --top 20 --sort frequency\n  idx market brokers --date 2026-10-16 --top 0\n  idx market brokers --source ipot     # with each broker's class and net\n  idx -o json market brokers | jq '.data.brokers[:3]'\n\n--source auto (default): idx.co.id's daily broker summary, latest session by default. When idx.co.id refuses (Cloudflare), cannot be reached or fails, has no copy under --offline, or has not published the session yet, IPOT's ranking answers instead, with a warning, if IPOT holds that session (the --date, or the latest that has ended); otherwise idx.co.id's answer stands, saying why IPOT could not.\n\n--source idx: idx.co.id only, any session. A session it has not published yet (its summary is empty on the day) is named: with no --date the session before is shown with a warning, and a --date of it is not found.\n\n--source ipot: Indo Premier's ranking of its latest session only, with each broker's class and net in the table and its value bought and sold in -o json, csv and jsonl. A --date before IPOT's session, or after today, is a usage error; a later one up to today, or a day without a session, is not found. IPOT abbreviates figures of a million and more to two decimals (rounded: true). IPOT's ranking read on a trading day from 08:58 to 19:00 WIB cannot be dated (when its pages roll over to a new session is not measured yet), so in those hours it is refused with a provider error.\n\nEach member's buying and selling count together, so all members' value is twice the market's. Which stocks a broker traded: `idx bandar broker CODE`; one stock's brokers: `idx broker SYMBOL`."
    )]
    Brokers {
        /// Trading day, YYYY-MM-DD. Default: the latest session.
        #[arg(long, short = 'd', value_name = "DATE")]
        date: Option<chrono::NaiveDate>,
        /// Rows; 0 for every member.
        #[arg(long, short = 'n', default_value_t = 20)]
        top: usize,
        /// Ranking figure.
        #[arg(long, value_enum, default_value_t = BrokerSort::Value)]
        sort: BrokerSort,
        /// Where the ranking comes from.
        #[arg(long, value_enum, default_value_t = BrokerSource::Auto)]
        source: BrokerSource,
    },
    /// Exchange members (Anggota Bursa): broker codes, names and licences.
    #[command(
        after_help = "Examples:\n  idx market members\n  idx market members --search mandiri\n  idx -o json market members | jq '.data.members[] | select(.code==\"YP\")'\n\nFrom idx.co.id's member list; the bundled list (data/brokers.csv) stands in when it cannot\nbe reached. The codes are the ones `idx broker` and `idx bandar` report."
    )]
    Members {
        /// Code or name, case-insensitive substring.
        #[arg(long, short = 's', value_name = "TEXT")]
        search: Option<String>,
    },
    /// Exchange calendar: dividend and rights dates, RUPS, public exposes, listings (idx.co.id).
    #[command(
        after_help = "Examples:\n  idx market calendar                          # the next 14 days\n  idx market calendar --days 30 --type dividend,rights\n  idx market calendar --symbol BBCA --from 2026-01-01 --to 2026-03-31\n  idx market calendar --date 2026-10-19 --type all\n  idx -o json market calendar | jq '.data.events[] | select(.milestone==\"cum\")'\n\nTypes: dividend, rights, bonus, split, warrant, rups, public_expose (expose), buyback, listing,\nbond, other; `all` or `default` (everything but bond events and buyback reports). Type and\nstep (cum, ex, recording, payment, plan, notice, result, ...) are classified by idx from each\nentry's wording. Ranges cover at most 92 days."
    )]
    Calendar {
        /// One day, YYYY-MM-DD (instead of a range).
        #[arg(long, short = 'd', value_name = "DATE", conflicts_with_all = ["from", "to", "days"])]
        date: Option<chrono::NaiveDate>,
        /// Start of the range, YYYY-MM-DD. Default: today.
        #[arg(long, value_name = "DATE")]
        from: Option<chrono::NaiveDate>,
        /// End of the range, YYYY-MM-DD. Default: --from plus --days.
        #[arg(long, value_name = "DATE", conflicts_with = "days")]
        to: Option<chrono::NaiveDate>,
        /// Days to look ahead when --to is not given.
        #[arg(long, value_name = "N", default_value_t = service::CALENDAR_DEFAULT_DAYS)]
        days: u32,
        /// One issuer, e.g. BBCA.
        #[arg(long, short = 's', value_name = "SYMBOL")]
        symbol: Option<String>,
        /// Comma-separated types, `all` or `default`.
        #[arg(
            long = "type",
            short = 't',
            value_name = "TYPES",
            default_value = "default"
        )]
        types: String,
    },
    /// Trading holidays (Kalender Libur Bursa), whether today is a trading day, and the next one.
    #[command(
        after_help = "Examples:\n  idx market holidays\n  idx market holidays --year 2027\n  idx market holidays --upcoming\n  idx -o json market holidays | jq '.data.next_trading_day'\n\nBundled from the yearly calendar idx.co.id publishes (weekend holidays are not listed);\nIDX can add a holiday later. Override the list with IDX_HOLIDAYS=/path/to.csv (date,name,name_en)."
    )]
    Holidays {
        /// Calendar year; default: the current year.
        #[arg(long, value_name = "YEAR", conflicts_with = "all")]
        year: Option<i32>,
        /// Every year on the list.
        #[arg(long)]
        all: bool,
        /// Only holidays from today on.
        #[arg(long)]
        upcoming: bool,
    },
    /// Astrology for IHSG: planets, Moon phase, Mercury retrograde, weton and shio for a date, with an optional returns backtest.
    #[command(after_help = "Examples:
  idx market astro                         # today at the close: planets, Moon, Mercury, weton, shio, next 30 days
  idx market astro --date 2026-12-24 --days 7
  idx market astro --natal base            # transits to the base-date chart (10 Aug 1982 = 100)
  idx market astro --backtest              # IHSG daily returns by lunar phase, Mercury, pasaran and Sun sign since 1990
  idx market astro --backtest --period 10y
  idx -o json market astro | jq '.data.moon'
  idx -o csv market astro                  # the planet table

Positions are computed on this machine for 16:00 WIB (the IDX close) from JPL's Keplerian elements,
Schlyter's Jupiter-Saturn terms and Meeus's lunar theory, checked against JPL Horizons (Sun and Moon within
0.01°, planets within 0.04°, 1800-2050), so it works offline. IHSG's chart is cast for noon WIB on its launch
(1983-04-01) or base date (1982-08-10), both from secondary sources. --backtest fetches ^JKSE daily closes and
tests each group against all other days (Welch's t, Bonferroni). Traditional astrology, not a forecast or
investment advice.")]
    Astro {
        /// Date, YYYY-MM-DD (1800-01-01 to 2050-12-31). Default: today in Jakarta.
        #[arg(long, short = 'd', value_name = "DATE")]
        date: Option<chrono::NaiveDate>,
        /// Days of lunar phases, stations and sign ingresses to list from the date (1-366).
        #[arg(long, value_name = "N", default_value_t = service::ASTRO_DEFAULT_DAYS)]
        days: u32,
        /// IHSG chart the transits refer to.
        #[arg(long, value_enum, value_name = "CHART", default_value_t = NatalChoice::Launch)]
        natal: NatalChoice,
        /// Add the backtest of IHSG daily returns (fetches ^JKSE history).
        #[arg(long)]
        backtest: bool,
        /// Backtest look-back. Default: max, all of the history (^JKSE on Yahoo: from 1990-04-06).
        #[arg(long, value_enum, value_name = "PERIOD", requires = "backtest")]
        period: Option<AstroPeriod>,
    },
}

pub async fn run(ctx: &Context, cmd: MarketCmd) -> Result<()> {
    match cmd {
        MarketCmd::Overview {
            top,
            min_mcap,
            index,
            universe: path,
        } => {
            let floor = crate::analysis::screen::parse_number(&min_mcap).ok_or_else(|| {
                crate::error::IdxError::Usage(format!(
                    "--min-mcap {min_mcap:?} is not a number (try 1T, 500B)"
                ))
            })?;
            let index = Symbol::parse(&index)?;
            let mut list = universe::load(path.as_deref())?;
            universe::extend_from_ownership(&mut list, &ctx.settings.ownership_db);
            let result =
                service::market_overview(ctx.provider.as_ref(), &list, &index, top, floor).await?;
            emit(ctx, SCHEMA_MARKET, result, |d| {
                reports::market(d, ctx.color)
            })
        }
        MarketCmd::Indices { codes, date } => {
            let result = service::indices(&ctx.fetcher, date, &codes, service::today_wib()).await?;
            emit(ctx, SCHEMA_INDICES, result, |d| tx::indices(d, ctx.color))
        }
        MarketCmd::Foreign { date, top, sort } => {
            let result =
                service::market_foreign(&ctx.fetcher, date, top, sort, service::today_wib())
                    .await?;
            emit(ctx, SCHEMA_MARKET_FOREIGN, result, |d| {
                tx::market_foreign(d, ctx.color)
            })
        }
        MarketCmd::Brokers {
            date,
            top,
            sort,
            source,
        } => {
            let result = service::market_brokers_from(
                &ctx.fetcher,
                source,
                date,
                top,
                sort,
                chrono::Utc::now(),
            )
            .await?;
            let provider = result.provider;
            emit(ctx, SCHEMA_MARKET_BROKERS, result, |d| {
                tx::market_brokers(d, provider, ctx.color)
            })
        }
        MarketCmd::Members { search } => {
            let result = service::members(&ctx.fetcher, search.as_deref()).await?;
            emit(ctx, SCHEMA_MEMBERS, result, |d| tx::members(d, ctx.color))
        }
        MarketCmd::Calendar {
            date,
            from,
            to,
            days,
            symbol,
            types,
        } => {
            let q = service::CalendarQuery {
                date,
                from,
                to,
                days: Some(days),
                symbol,
                categories: CalendarCategory::parse_list(&types)?,
            };
            let result = service::calendar(&ctx.fetcher, &q, service::today_wib()).await?;
            emit(ctx, SCHEMA_CALENDAR, result, |d| tx::calendar(d, ctx.color))
        }
        MarketCmd::Holidays {
            year,
            all,
            upcoming,
        } => {
            let today = service::today_wib();
            let year = if all {
                None
            } else {
                Some(year.unwrap_or(today.year()))
            };
            let result = service::holidays(year, upcoming, today)?;
            emit(ctx, SCHEMA_HOLIDAYS, result, |d| tx::holidays(d, ctx.color))
        }
        MarketCmd::Astro {
            date,
            days,
            natal,
            backtest,
            period,
        } => {
            let q = service::AstroQuery {
                date: date.unwrap_or_else(service::today_wib),
                days,
                natal,
                backtest: backtest.then(|| period.unwrap_or(AstroPeriod::Max)),
            };
            let result = service::astro(ctx.provider.as_ref(), &q).await?;
            emit(ctx, SCHEMA_ASTRO, result, |d| {
                crate::output::table_astro::astro(d, ctx.color)
            })
        }
    }
}
