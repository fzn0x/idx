//! `idx stocks ...`

use clap::Subcommand;

use crate::analysis::screen::{parse_filters, Key};
use crate::error::IdxError;
use crate::error::Result;
use crate::model::{EarningsFilter, Interval, Period, StatementPeriod, Symbol};
use crate::output::json::{
    SCHEMA_ACTIONS, SCHEMA_BRIEF, SCHEMA_COMPARE, SCHEMA_DIVIDENDS, SCHEMA_EARNINGS,
    SCHEMA_FINANCIALS, SCHEMA_FOREIGN, SCHEMA_FUNDAMENTALS, SCHEMA_HISTORY, SCHEMA_INSTRUMENTS,
    SCHEMA_NEWS, SCHEMA_PROFILE, SCHEMA_QUOTES, SCHEMA_RETURNS, SCHEMA_SCREEN, SCHEMA_SENTIMENT,
    SCHEMA_TECHNICAL,
};
use crate::output::table;
use crate::output::table_exchange as tx;
use crate::output::table_news as newstbl;
use crate::output::table_reports::{self as reports, StatementFlags};
use crate::service::{self, ScreenOptions};
use crate::universe;

use super::{emit, Context};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Statement {
    All,
    Income,
    Balance,
    #[value(alias = "cashflow", alias = "cash-flow")]
    Cash,
}

impl Statement {
    /// The table sections a `--statement` list asks for (`all` anywhere means all three).
    pub fn flags(list: &[Statement]) -> StatementFlags {
        let all = list.is_empty() || list.contains(&Statement::All);
        StatementFlags {
            income: all || list.contains(&Statement::Income),
            balance: all || list.contains(&Statement::Balance),
            cash_flow: all || list.contains(&Statement::Cash),
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum StocksCmd {
    /// Delayed quote for one or more tickers (comma or space separated).
    #[command(
        after_help = "Examples:\n  idx stocks quote BBCA\n  idx stocks quote BBCA,BBRI,TLKM\n  idx -o json stocks quote bbca bbri"
    )]
    Quote {
        /// IDX tickers, e.g. BBCA or BBCA,BBRI. The .JK suffix is optional.
        #[arg(required = true, value_name = "SYMBOLS")]
        symbols: Vec<String>,
    },

    /// OHLCV price history.
    #[command(
        after_help = "Examples:\n  idx stocks history BBCA\n  idx stocks history TLKM --period 5y --interval 1mo\n  idx stocks history BBRI --period 5d --interval 1h --limit 0"
    )]
    History {
        /// IDX ticker, e.g. BBCA.
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Look-back window.
        #[arg(long, short = 'p', value_enum, default_value = "1y")]
        period: Period,
        /// Bar size. Intraday sizes (5m..1h) only work with short periods.
        #[arg(long, short = 'i', value_enum, default_value = "1d")]
        interval: Interval,
        /// Rows shown in table mode (most recent first kept); 0 shows everything. JSON is never limited.
        #[arg(long, short = 'n', default_value_t = 30)]
        limit: usize,
    },

    /// Company profile: sector, industry, description, size, management.
    #[command(
        after_help = "Examples:\n  idx stocks profile BBCA\n  idx -o json stocks profile TLKM | jq .data.sector"
    )]
    Profile {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
    },

    /// Income statement, balance sheet and cash flow (annual, quarterly or TTM).
    #[command(
        after_help = "Examples:\n  idx stocks financials BBCA\n  idx stocks financials TLKM --period quarterly --statement income\n  idx stocks financials TLKM -s income,cash\n  idx -o json stocks financials ASII --period ttm\n\n--statement (income, balance, cash or all; comma-separated or repeated) narrows the table.\nJSON and CSV always carry all three statements, which are columns of each period."
    )]
    Financials {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Reporting series.
        #[arg(long, short = 'p', value_enum, default_value = "annual")]
        period: StatementPeriod,
        /// Statements shown in table mode, comma-separated or repeated (JSON and CSV always carry all three).
        #[arg(
            long,
            short = 's',
            value_enum,
            value_delimiter = ',',
            default_value = "all",
            value_name = "STATEMENT"
        )]
        statement: Vec<Statement>,
    },

    /// Reported quarterly EPS vs estimates, yearly and quarterly revenue and earnings, analyst consensus, next earnings date.
    #[command(
        after_help = "Examples:\n  idx stocks earnings BBCA\n  idx stocks earnings TLKM --annual --estimates\n  idx stocks earnings BBRI --quarterly --reported\n  idx -o json stocks earnings BBRI | jq .data.next\n\nWith neither --annual nor --quarterly every period is shown; with neither --reported nor\n--estimates both are (--history and --forecast are aliases). The filters apply to JSON and\nCSV too, and data.scope records what was kept."
    )]
    Earnings {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Only fiscal-year rows: yearly revenue and earnings, this and next year's consensus.
        #[arg(long)]
        annual: bool,
        /// Only quarterly rows: reported quarters, quarterly revenue and earnings, the quarter consensus and the next report date.
        #[arg(long)]
        quarterly: bool,
        /// Only reported figures (no consensus).
        #[arg(long, visible_alias = "history")]
        reported: bool,
        /// Only analyst consensus: the estimates and the next earnings date.
        #[arg(long, visible_alias = "forecast")]
        estimates: bool,
    },

    /// Cash dividend and split history with yearly totals, trailing-twelve-month yield and the payout streak.
    #[command(
        after_help = "Examples:\n  idx stocks dividends BBCA\n  idx stocks dividends TLKM --limit 20\n  idx -o json stocks dividends BBRI | jq '.data | {ttm_total, ttm_yield_pct, consecutive_years}'\n\nAmounts are per share as reported by the source (split-adjusted on Yahoo), keyed by ex-date.\nThe yearly totals, TTM yield (TTM total / last price) and streak are computed by idx."
    )]
    Dividends {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Most recent payments listed in table mode (JSON carries all).
        #[arg(long, short = 'n', default_value_t = 12)]
        limit: usize,
    },

    /// Price returns over 1w..5y, YTD, CAGR, volatility, drawdowns and 52-week position, with excess returns over IHSG.
    #[command(
        after_help = "Examples:\n  idx stocks returns BBCA\n  idx stocks returns BBCA BBRI BMRI BBNI          # side by side, plus IHSG\n  idx stocks returns TLKM --benchmark ^JKLQ45\n  idx stocks returns ASII --no-benchmark\n  idx -o json stocks returns BBCA | jq '.data[0] | {returns, benchmark}'\n\nComputed by idx from five years of daily closes (total price return, dividends excluded).\nIndex codes keep their caret: ^JKSE is IHSG, ^JKLQ45 is LQ45."
    )]
    Returns {
        /// One or more IDX tickers, comma or space separated.
        #[arg(required = true, value_name = "SYMBOLS")]
        symbols: Vec<String>,
        /// Index to measure excess returns against.
        #[arg(long, default_value = "^JKSE", value_name = "INDEX")]
        benchmark: String,
        /// Skip the benchmark fetch.
        #[arg(long)]
        no_benchmark: bool,
    },

    /// One-page research brief: quote, verdict, signal, returns vs IHSG, dividends, holders and headlines in one call.
    #[command(
        after_help = "Examples:\n  idx stocks brief BBCA\n  idx -o json stocks brief TLKM | jq '.data.key_points'\n\nRuns quote, fundamental, technical, returns, dividends, ownership ticker and news\nconcurrently and merges them. A section whose source fails is left out with a warning."
    )]
    Brief {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
    },

    /// Valuation, profitability, balance-sheet, growth and dividend ratios with a plain-language verdict.
    #[command(
        after_help = "Examples:\n  idx stocks fundamental BBCA\n  idx -o json stocks fundamental TLKM | jq '.data.verdict'\n\nThe verdict is a transparent rule score over public ratios (every rule that fires is listed). It is not investment advice."
    )]
    Fundamental {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
    },

    /// RSI, MACD, moving averages, Bollinger bands, support/resistance, trend and a buy/sell/neutral signal.
    #[command(
        after_help = "Examples:\n  idx stocks technical BBRI\n  idx stocks technical TLKM --period 2y\n  idx -o json stocks technical BBCA | jq '.data.signal'\n\nIndicators are computed locally from daily candles; the signal is a rule score, not investment advice."
    )]
    Technical {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Daily candles to analyse (1y gives the 200-day average room to work).
        #[arg(long, short = 'p', value_enum, default_value = "1y")]
        period: Period,
    },

    /// Screen the IDX universe (~950 tickers) by valuation, size, dividend and, optionally, profitability.
    #[command(
        after_help = "Examples:\n  idx stocks screen -f \"pe<10,div>5,mcap>5T\"\n  idx stocks screen -f \"roe>15,pe<15,sector=financial\" --sort roe\n  idx stocks screen -f \"mcap>50T\" --fundamentals --sort margin -n 10\n\nKeys: pe fpe pb ps peg mcap price div eps volume chg (from one batch quote call);\n      roe roa margin opmargin de growth epsgrowth sector industry (per-company fundamentals,\n      fetched for the largest --max-fundamentals companies that pass the quote filters).\nOperators: < <= > >= = !=. Numbers accept K M B T suffixes and a trailing %.\nUniverse: bundled list of IDX issuers plus any seen by `idx ownership import`; override with --universe or IDX_UNIVERSE."
    )]
    Screen {
        /// Comma-separated clauses, e.g. "pe<15,roe>15,sector=Banking".
        #[arg(long, short = 'f', value_name = "SPEC", required = true)]
        filter: String,
        /// Sort key (any filter key); descending unless --asc.
        #[arg(long, default_value = "mcap", value_name = "KEY")]
        sort: String,
        #[arg(long)]
        asc: bool,
        /// Rows shown; 0 for all.
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
        /// Cap on per-company fundamentals calls.
        #[arg(long, default_value_t = 50, value_name = "N")]
        max_fundamentals: usize,
        /// Fetch fundamentals for the results even when no filter needs them.
        #[arg(long)]
        fundamentals: bool,
        /// CSV of symbol,name to screen instead of the bundled universe.
        #[arg(long, value_name = "PATH")]
        universe: Option<std::path::PathBuf>,
    },

    /// Latest headlines from Kontan and CNBC Indonesia; with a ticker, the company's headlines first, then market context.
    #[command(
        after_help = "Examples:\n  idx stocks news                # market headlines\n  idx stocks news BBCA           # BBCA / Bank Central Asia first, then market headlines\n  idx stocks news BBCA --company-only\n  idx stocks news TLKM --source kontan --limit 40\n\nSources: kontan, cnbcindonesia (defaults), bisnis (blocked for non-browser clients; tried on request), all."
    )]
    News {
        #[arg(value_name = "SYMBOL")]
        symbol: Option<String>,
        /// Publisher names; repeatable.
        #[arg(long, short = 's', value_name = "NAME")]
        source: Vec<String>,
        /// Headlines shown; 0 for all.
        #[arg(long, short = 'n', default_value_t = 20)]
        limit: usize,
        /// Only headlines that mention the ticker; no market context.
        #[arg(long, requires = "symbol")]
        company_only: bool,
    },

    /// Lexicon sentiment over the headlines that mention one ticker.
    #[command(
        after_help = "Examples:\n  idx stocks sentiment BBCA\n  idx -o json stocks sentiment BBRI | jq '.data | {label, score, items}'\n\nCounts Indonesian and English finance terms per headline. Indicative only."
    )]
    Sentiment {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        #[arg(long, short = 's', value_name = "NAME")]
        source: Vec<String>,
        /// Strongest headlines shown.
        #[arg(long, short = 'n', default_value_t = 10)]
        limit: usize,
    },

    /// Listed stocks from idx.co.id: code, name, listing board, listing date and shares.
    #[command(
        after_help = "Examples:\n  idx stocks list\n  idx stocks list --search bank\n  idx stocks list --board akselerasi\n  idx -o csv stocks list > stocks.csv\n\nBoards: Utama, Pengembangan, Akselerasi, Ekonomi Baru, Pemantauan Khusus. Without idx.co.id\n(and nothing cached) the bundled universe is listed instead, codes and names only."
    )]
    List {
        /// Listing board, case-insensitive substring (e.g. utama, pengembangan).
        #[arg(long, value_name = "BOARD")]
        board: Option<String>,
        /// Code or name, case-insensitive substring.
        #[arg(long, short = 's', value_name = "TEXT")]
        search: Option<String>,
    },

    /// Corporate actions (dividends, splits, bonus shares, rights), shareholders' meetings (RUPS), warrants and rights.
    #[command(
        after_help = "Examples:\n  idx stocks actions BBCA\n  idx stocks actions ENRG --since 2025-01-01\n  idx -o json stocks actions BBRI | jq '.data.actions[] | {label, amount, cum_date, ex_date}'\n\nFrom IPOT's corporate-action page: each action with its amount (IDR per share: the dividend,\nor a rights issue's exercise price), ratio (old:new), cum, ex, recording and payment dates and\nstatus; each RUPS with date, time and venue (JSON carries the agenda); listed warrants and rights.\nUpcoming events are included. For the market-wide schedule see `idx market calendar`."
    )]
    Actions {
        /// IDX ticker, e.g. BBCA.
        symbol: String,
        /// Events from this date on, YYYY-MM-DD. Default: one year ago.
        #[arg(long, value_name = "DATE")]
        since: Option<chrono::NaiveDate>,
    },

    /// Foreign buying and selling per session (idx.co.id), with totals and the net-buy/net-sell streak.
    #[command(
        after_help = "Examples:\n  idx stocks foreign BBCA\n  idx stocks foreign BBRI --days 60\n  idx -o json stocks foreign TLKM | jq '.data.totals'\n\nFrom idx.co.id's daily stock summary. IDX publishes foreign trading in shares only; the\nnet × close column multiplies the net shares by the close for a rupiah scale, which is not\nthe value foreigners traded at. For the whole market on one day see `idx market foreign`."
    )]
    Foreign {
        /// IDX ticker, e.g. BBCA.
        symbol: String,
        /// Latest sessions to show (1-250).
        #[arg(long, short = 'n', default_value_t = service::FOREIGN_DEFAULT_DAYS)]
        days: usize,
    },

    /// Side-by-side fundamentals for two or more tickers.
    #[command(
        after_help = "Examples:\n  idx stocks compare BBCA BBRI BMRI BBNI\n  idx -o json stocks compare TLKM,ISAT,EXCL | jq '.data[].valuation.pe_trailing'"
    )]
    Compare {
        /// Two or more IDX tickers, comma or space separated.
        #[arg(required = true, value_name = "SYMBOLS")]
        symbols: Vec<String>,
    },
}

pub async fn run(ctx: &Context, cmd: StocksCmd) -> Result<()> {
    match cmd {
        StocksCmd::Quote { symbols } => quote(ctx, &symbols).await,
        StocksCmd::History {
            symbol,
            period,
            interval,
            limit,
        } => history(ctx, &symbol, period, interval, limit).await,
        StocksCmd::Profile { symbol } => profile(ctx, &symbol).await,
        StocksCmd::Financials {
            symbol,
            period,
            statement,
        } => financials(ctx, &symbol, period, &statement).await,
        StocksCmd::Earnings {
            symbol,
            annual,
            quarterly,
            reported,
            estimates,
        } => {
            let filter = EarningsFilter {
                annual,
                quarterly,
                reported,
                estimates,
            };
            earnings(ctx, &symbol, filter).await
        }
        StocksCmd::Dividends { symbol, limit } => dividends(ctx, &symbol, limit).await,
        StocksCmd::Brief { symbol } => brief(ctx, &symbol).await,
        StocksCmd::Returns {
            symbols,
            benchmark,
            no_benchmark,
        } => returns(ctx, &symbols, &benchmark, no_benchmark).await,
        StocksCmd::Fundamental { symbol } => fundamental(ctx, &symbol).await,
        StocksCmd::Technical { symbol, period } => technical(ctx, &symbol, period).await,
        StocksCmd::Compare { symbols } => compare(ctx, &symbols).await,
        StocksCmd::Foreign { symbol, days } => {
            let sym = Symbol::parse(&symbol)?;
            let list = universe::load(None).unwrap_or_default();
            let result = service::foreign_flow(&ctx.fetcher, &list, &sym, days).await?;
            emit(ctx, SCHEMA_FOREIGN, result, |d| {
                tx::foreign_flow(d, ctx.color)
            })
        }
        StocksCmd::Actions { symbol, since } => {
            let sym = Symbol::parse(&symbol)?;
            let list = universe::load(None).unwrap_or_default();
            let result =
                service::corporate_actions(&ctx.fetcher, &list, &sym, since, service::today_wib())
                    .await?;
            emit(ctx, SCHEMA_ACTIONS, result, |d| {
                tx::corporate_actions(d, ctx.color)
            })
        }
        StocksCmd::List { board, search } => {
            let result =
                service::instruments(&ctx.fetcher, board.as_deref(), search.as_deref()).await?;
            emit(ctx, SCHEMA_INSTRUMENTS, result, |d| {
                tx::instruments(d, ctx.color)
            })
        }
        StocksCmd::Screen {
            filter,
            sort,
            asc,
            limit,
            max_fundamentals,
            fundamentals,
            universe,
        } => {
            let filters = parse_filters(&filter)?;
            let Some(sort_key) = Key::parse(&sort) else {
                return Err(IdxError::Usage(format!(
                    "unknown sort key {sort:?}; use one of {}",
                    Key::ALL
                )));
            };
            let mut list = universe::load(universe.as_deref())?;
            universe::extend_from_ownership(&mut list, &ctx.settings.ownership_db);
            let opts = ScreenOptions {
                filters,
                sort: sort_key,
                descending: !asc,
                limit,
                max_fundamentals,
                with_fundamentals: fundamentals,
            };
            let result = service::screen(ctx.provider.as_ref(), &list, &opts).await?;
            emit(ctx, SCHEMA_SCREEN, result, |d| {
                newstbl::screen(d, ctx.color)
            })
        }
        StocksCmd::News {
            symbol,
            source,
            limit,
            company_only,
        } => {
            let sym = symbol.map(|s| Symbol::parse(&s)).transpose()?;
            let list = universe::load(None).unwrap_or_default();
            let result = service::news(
                &ctx.fetcher,
                &list,
                sym.as_ref(),
                &source,
                limit,
                company_only,
            )
            .await?;
            emit(ctx, SCHEMA_NEWS, result, |d| newstbl::news(d, ctx.color))
        }
        StocksCmd::Sentiment {
            symbol,
            source,
            limit,
        } => {
            let sym = Symbol::parse(&symbol)?;
            let list = universe::load(None).unwrap_or_default();
            let result = service::sentiment(&ctx.fetcher, &list, &sym, &source, limit).await?;
            emit(ctx, SCHEMA_SENTIMENT, result, |d| {
                newstbl::sentiment(d, ctx.color)
            })
        }
    }
}

async fn quote(ctx: &Context, raw: &[String]) -> Result<()> {
    let symbols = Symbol::parse_list(raw)?;
    let result = ctx.provider.quotes(&symbols).await?;
    emit(ctx, SCHEMA_QUOTES, result, |d| table::quotes(d, ctx.color))
}

async fn history(
    ctx: &Context,
    raw: &str,
    period: Period,
    interval: Interval,
    limit: usize,
) -> Result<()> {
    let symbol = Symbol::parse(raw)?;
    let result = ctx.provider.history(&symbol, period, interval).await?;
    let limit = if limit == 0 { None } else { Some(limit) };
    emit(ctx, SCHEMA_HISTORY, result, |d| {
        table::history(d, limit, ctx.color)
    })
}

async fn profile(ctx: &Context, raw: &str) -> Result<()> {
    let symbol = Symbol::parse(raw)?;
    let result = ctx.provider.profile(&symbol).await?;
    emit(ctx, SCHEMA_PROFILE, result, |d| {
        reports::profile(d, ctx.color)
    })
}

async fn financials(
    ctx: &Context,
    raw: &str,
    period: StatementPeriod,
    statement: &[Statement],
) -> Result<()> {
    let symbol = Symbol::parse(raw)?;
    let result = ctx.provider.financials(&symbol, period).await?;
    let flags = Statement::flags(statement);
    emit(ctx, SCHEMA_FINANCIALS, result, |d| {
        reports::financials(d, &flags, ctx.color)
    })
}

async fn earnings(ctx: &Context, raw: &str, filter: EarningsFilter) -> Result<()> {
    let symbol = Symbol::parse(raw)?;
    let result = service::earnings(ctx.provider.as_ref(), &symbol, filter).await?;
    emit(ctx, SCHEMA_EARNINGS, result, |d| {
        reports::earnings(d, ctx.color)
    })
}

async fn dividends(ctx: &Context, raw: &str, limit: usize) -> Result<()> {
    let symbol = Symbol::parse(raw)?;
    let result = service::dividends(ctx.provider.as_ref(), &symbol).await?;
    emit(ctx, SCHEMA_DIVIDENDS, result, |d| {
        reports::dividends(d, limit, ctx.color)
    })
}

async fn returns(ctx: &Context, raw: &[String], benchmark: &str, none: bool) -> Result<()> {
    let symbols = Symbol::parse_list(raw)?;
    let bench = if none || benchmark.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(Symbol::parse(benchmark)?)
    };
    let result = service::returns(ctx.provider.as_ref(), &symbols, bench.as_ref()).await?;
    emit(ctx, SCHEMA_RETURNS, result, |d| {
        reports::returns(d, ctx.color)
    })
}

async fn brief(ctx: &Context, raw: &str) -> Result<()> {
    let symbol = Symbol::parse(raw)?;
    let list = universe::load(None).unwrap_or_default();
    let result = service::brief(
        ctx.provider.as_ref(),
        &ctx.fetcher,
        &list,
        &ctx.settings.ownership_db,
        &symbol,
    )
    .await?;
    emit(ctx, SCHEMA_BRIEF, result, |d| reports::brief(d, ctx.color))
}

async fn fundamental(ctx: &Context, raw: &str) -> Result<()> {
    let symbol = Symbol::parse(raw)?;
    let result = service::fundamentals(ctx.provider.as_ref(), &symbol).await?;
    emit(ctx, SCHEMA_FUNDAMENTALS, result, |d| {
        reports::fundamentals(d, ctx.color)
    })
}

async fn technical(ctx: &Context, raw: &str, period: Period) -> Result<()> {
    let symbol = Symbol::parse(raw)?;
    let result = service::technical(ctx.provider.as_ref(), &symbol, period).await?;
    emit(ctx, SCHEMA_TECHNICAL, result, |d| {
        reports::technical(d, ctx.color)
    })
}

async fn compare(ctx: &Context, raw: &[String]) -> Result<()> {
    let symbols = Symbol::parse_list(raw)?;
    let result = service::compare(ctx.provider.as_ref(), &symbols).await?;
    emit(ctx, SCHEMA_COMPARE, result, |d| {
        reports::compare(d, ctx.color)
    })
}
