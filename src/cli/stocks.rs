//! `idx stocks ...`

use clap::Subcommand;

use crate::analysis::screen::{parse_filters, Key, ScreenPreset};
use crate::error::IdxError;
use crate::error::Result;
use crate::model::{
    EarningsFilter, ForeignSource, Interval, Lens, Period, StatementPeriod, Symbol,
};
use crate::output::json::{
    SCHEMA_ACTIONS, SCHEMA_BRIEF, SCHEMA_COMPARE, SCHEMA_DIVIDENDS, SCHEMA_EARNINGS,
    SCHEMA_FINANCIALS, SCHEMA_FOREIGN, SCHEMA_FOREIGN_VALUE, SCHEMA_FUNDAMENTALS, SCHEMA_GROWTH,
    SCHEMA_HISTORY, SCHEMA_INSIGHTS, SCHEMA_INSTRUMENTS, SCHEMA_INTRADAY, SCHEMA_NEWS,
    SCHEMA_PROFILE, SCHEMA_QUOTES, SCHEMA_RETURNS, SCHEMA_RISK, SCHEMA_SCREEN, SCHEMA_SENTIMENT,
    SCHEMA_TECHNICAL, SCHEMA_VALUATION,
};
use crate::output::table;
use crate::output::table_exchange as tx;
use crate::output::table_insights as tins;
use crate::output::table_intraday as intraday_table;
use crate::output::table_lenses as lenses;
use crate::output::table_news as newstbl;
use crate::output::table_reports::{self as reports, StatementFlags};
use crate::service::{self, ScreenOptions};
use crate::universe;

use super::{emit, input, Context};

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
        after_help = "Examples:\n  idx stocks quote BBCA\n  idx stocks quote BBCA,BBRI,TLKM\n  idx -o json stocks quote bbca bbri\n  echo BBCA BBRI | idx stocks quote -          # tickers from stdin\n  idx stocks quote - < watchlist.txt\n\n`-` reads tickers from stdin: text (spaces, commas or lines; # starts a comment) or\nidx's own -o json / -o jsonl output, taking each row's symbol.\n\nOther Yahoo Finance markets: give the suffix (idx stocks quote D05.SI) or set the default\nfor bare tickers with IDX_EXCHANGE / `idx config set exchange SI`. Applies to quote, history,\nprofile, financials, earnings, dividends, returns, fundamental, technical and compare.\n\nRows carry behind_session: true when the last trade is dated before the latest IDX session that has ended (16:15 WIB; holidays skipped): suspended, not traded since, or the source lags. The table lists such rows under it."
    )]
    Quote {
        /// IDX tickers, e.g. BBCA or BBCA,BBRI. The .JK suffix is optional; another market's suffix works too (D05.SI); `-` reads them from stdin.
        #[arg(required = true, value_name = "SYMBOLS")]
        symbols: Vec<String>,
    },

    /// OHLCV price history.
    #[command(
        after_help = "Examples:\n  idx stocks history BBCA\n  idx stocks history TLKM --period 5y --interval 1mo\n  idx stocks history BBRI --period 5d --interval 1h --limit 0\n  idx stocks history ^JKSE --period 1d --interval 1m\n  idx stocks history D05.SI --period 5d             # another Yahoo Finance market, by its suffix\n\nFor the latest session by the minute with the quote and VWAP, see `idx stocks intraday`."
    )]
    History {
        /// Ticker: BBCA (IDX, .JK optional) or another Yahoo Finance market's with its suffix (D05.SI). A bare code takes IDX_EXCHANGE (default IDX).
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Look-back window.
        #[arg(long, short = 'p', value_enum, default_value = "1y")]
        period: Period,
        /// Bar size. 1m needs --period 1d or 5d, 2m at most 1mo; the other intraday sizes (5m..1h) only work with short periods.
        #[arg(long, short = 'i', value_enum, default_value = "1d")]
        interval: Interval,
        /// Rows shown in table mode (most recent first kept); 0 shows everything. JSON, JSONL and CSV are never limited.
        #[arg(long, short = 'n', default_value_t = 30)]
        limit: usize,
    },

    /// The latest session by the minute: OHLCV bars, the quote and a session summary with VWAP.
    #[command(
        after_help = "Examples:\n  idx stocks intraday BBCA\n  idx stocks intraday ^JKSE --interval 5m\n  idx stocks intraday BBRI --limit 0\n  idx -o json stocks intraday TLKM | jq '.data.summary'\n  idx -o csv stocks intraday BBCA > bbca-minutes.csv\n\nToday's session while the market trades, otherwise the last one (weekends, holidays, before\nthe open, and the first minutes after it, before Yahoo has bars for the day; a warning says so).\nFrom Yahoo's chart, no login, about 10 minutes behind the market. Minutes without a\ntrade have no bar; the opening auction's volume is in no bar, so the bars' volume is below the\nday's, and the closing auction lands in the 16:00 bar. VWAP is computed by idx from the bars'\ntypical prices, an approximation of IDX's average price. With --provider stockbit (after\n`idx stockbit login`) the bars come from your Stockbit session in real time."
    )]
    Intraday {
        /// IDX ticker or index, e.g. BBCA or ^JKSE.
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Bar size: 1m, 2m, 5m, 15m, 30m or 1h.
        #[arg(long, short = 'i', value_enum, default_value = "1m")]
        interval: Interval,
        /// Most recent bars shown in table mode; 0 shows all. JSON and CSV carry every bar.
        #[arg(long, short = 'n', default_value_t = 15)]
        limit: usize,
    },

    /// Company profile: sector, industry, description, size, management.
    #[command(
        after_help = "Examples:\n  idx stocks profile BBCA\n  idx -o json stocks profile TLKM | jq .data.sector"
    )]
    Profile {
        /// Ticker: BBCA (IDX, .JK optional) or another Yahoo Finance market's with its suffix (D05.SI). A bare code takes IDX_EXCHANGE (default IDX).
        #[arg(value_name = "SYMBOL")]
        symbol: String,
    },

    /// Income statement, balance sheet and cash flow (annual, quarterly or TTM).
    #[command(
        after_help = "Examples:\n  idx stocks financials BBCA\n  idx stocks financials TLKM --period quarterly --statement income\n  idx stocks financials TLKM -s income,cash\n  idx -o json stocks financials ASII --period ttm\n\n--statement (income, balance, cash or all; comma-separated or repeated) narrows the table.\nJSON and CSV always carry all three statements, which are columns of each period."
    )]
    Financials {
        /// Ticker: BBCA (IDX, .JK optional) or another Yahoo Finance market's with its suffix (D05.SI). A bare code takes IDX_EXCHANGE (default IDX).
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Reporting series.
        #[arg(long, short = 'p', value_enum, default_value = "annual")]
        period: StatementPeriod,
        /// Statements shown in table mode, comma-separated or repeated (JSON, JSONL and CSV always carry all three).
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
        /// Ticker: BBCA (IDX, .JK optional) or another Yahoo Finance market's with its suffix (D05.SI). A bare code takes IDX_EXCHANGE (default IDX).
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
        /// Ticker: BBCA (IDX, .JK optional) or another Yahoo Finance market's with its suffix (D05.SI). A bare code takes IDX_EXCHANGE (default IDX).
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Most recent payments listed in table mode (JSON, JSONL and CSV carry all).
        #[arg(long, short = 'n', default_value_t = 12)]
        limit: usize,
    },

    /// Price returns over 1w..5y, YTD, CAGR, volatility, drawdowns and 52-week position, with excess returns over IHSG.
    #[command(
        after_help = "Examples:\n  idx stocks returns BBCA\n  idx stocks returns BBCA BBRI BMRI BBNI          # side by side, plus IHSG\n  idx stocks returns TLKM --benchmark ^JKLQ45\n  idx stocks returns ASII --no-benchmark\n  idx -o json stocks returns BBCA | jq '.data[0] | {returns, benchmark}'\n  idx stocks returns - < tickers.txt              # tickers from stdin\n\nComputed by idx from five years of daily closes (total price return, dividends excluded).\nIndex codes keep their caret: ^JKSE is IHSG, ^JKLQ45 is LQ45."
    )]
    Returns {
        /// One or more tickers, comma or space separated: IDX codes, or another Yahoo Finance market's with its suffix (D05.SI); `-` reads them from stdin.
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
        after_help = "Examples:\n  idx stocks fundamental BBCA\n  idx -o json stocks fundamental TLKM | jq '.data.verdict'\n  idx -o json stocks fundamental ITMG | jq '.data | {currency, financial_currency}'\n  idx stocks fundamental TLKM --lens risk   # same as idx stocks risk TLKM\n\nThe verdict is a transparent rule score over public ratios (every rule that fires is listed). Negative shareholders' equity scores against the company and sets the P/B, ROE and debt/equity rules aside. When the statements are in another currency than the price (financial_currency USD, price in IDR), P/B, P/S and EV multiples mix the two, so P/B is not scored and the table marks them with *. With --lens growth, valuation or risk a focused reading replaces the report (idx.growth.v1, idx.valuation.v1, idx.risk.v1). It is not investment advice."
    )]
    Fundamental {
        /// Ticker: BBCA (IDX, .JK optional) or another Yahoo Finance market's with its suffix (D05.SI). A bare code takes IDX_EXCHANGE (default IDX).
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// A focused reading instead of the full report: the same as `stocks growth|valuation|risk`.
        #[arg(long, value_enum, ignore_case = true, value_name = "LENS")]
        lens: Option<Lens>,
    },

    /// Growth lens: latest quarter, forward EPS against trailing, and revenue and net income across the fiscal years.
    #[command(
        after_help = "Examples:\n  idx stocks growth BBCA\n  idx -o json stocks growth TLKM | jq '.data.reading'\n  idx stocks fundamental BBCA --lens growth   # the same\n\nLatest-quarter revenue and earnings growth, forward EPS against trailing, and revenue and net\nincome across the annual statements (about four fiscal years: CAGR, how often revenue grew,\na turn from profit to loss). Every rule that ran is listed with its points (+ favours growth),\nevery rule that could not run is noted. Label: growing (score 2 or more), shrinking (-2 or\nless), mixed (points both ways), stable, or insufficient_data (fewer than 3 rules).\nHeuristic, not investment advice."
    )]
    Growth {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
    },

    /// Valuation lens: P/E, PEG, P/B, P/S, EV/EBITDA, dividend yield and the analyst target, each scored.
    #[command(
        after_help = "Examples:\n  idx stocks valuation ITMG\n  idx -o json stocks valuation BBCA | jq '.data.reading.rules'\n  idx stocks fundamental ITMG --lens valuation   # the same\n\nP/E, PEG, P/B, P/S, EV/EBITDA, dividend yield and the analyst mean target (3 or more\nanalysts), each scored (+ favours cheapness). Book, sales and EV multiples are not scored for\ncompanies whose statements are in another currency than the price (USD reporters such as\nITMG), P/B not with negative equity, and P/S and EV/EBITDA not for banks and insurers. Label:\ncheap (score 3 or more), expensive (-3 or less), fair, or insufficient_data (fewer than 3\nrules). It can disagree with the verdict of `stocks fundamental`, which also weighs quality\nand growth. Heuristic, not investment advice."
    )]
    Valuation {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
    },

    /// Risk lens: negative equity, leverage, liquidity, losses, and beta, volatility and largest fall against IHSG.
    #[command(
        after_help = "Examples:\n  idx stocks risk GIAA\n  idx -o json stocks risk TLKM | jq '.data.price'\n  idx stocks fundamental TLKM --lens risk   # the same\n\nBalance sheet (negative equity, debt/equity, current ratio; banks and insurers skip the last\ntwo), losses over the trailing twelve months, and a year of daily closes against IHSG (^JKSE)\non the sessions both traded: beta (120 daily returns or more), volatility relative to IHSG's,\nand the largest fall relative to IHSG's. A close that did not move on more than half of those\nsessions (suspended or barely traded) scores an illiquidity point instead of those three.\nPoints add risk. Label: high (score 3 or more), low (-2 or less), moderate, or\ninsufficient_data (fewer than 3 rules). Beta is computed by idx; the source's own beta in\n`stocks fundamental` does not track IHSG. Heuristic, not investment advice."
    )]
    Risk {
        #[arg(value_name = "SYMBOL")]
        symbol: String,
    },

    /// RSI, MACD, moving averages, Bollinger bands, support/resistance, trend and a buy/sell/neutral signal.
    #[command(
        after_help = "Examples:\n  idx stocks technical BBRI\n  idx stocks technical TLKM --period 2y\n  idx -o json stocks technical BBCA | jq '.data.signal'\n\nIndicators are computed locally from daily candles; the signal is a rule score, not investment advice."
    )]
    Technical {
        /// Ticker: BBCA (IDX, .JK optional) or another Yahoo Finance market's with its suffix (D05.SI). A bare code takes IDX_EXCHANGE (default IDX).
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Daily candles to analyse (1y gives the 200-day average room to work).
        #[arg(long, short = 'p', value_enum, default_value = "1y")]
        period: Period,
    },

    /// Screen the IDX universe (~950 tickers) by valuation, size, dividend, returns and, optionally, profitability; or pick a preset.
    #[command(
        after_help = "Examples:\n  idx stocks screen -f \"pe<10,div>5,mcap>5T\"\n  idx stocks screen --preset top-performers -n 20            # best 1-year price return\n  idx stocks screen --preset top-performers --sort ret3m     # best 3-month return instead\n  idx stocks screen --preset low-pe -f \"mcap>10T\"\n  idx stocks screen --preset 52w-high\n  idx stocks screen -f \"ret1y>50,pe<15\" --sort ret1y\n  idx stocks screen -f \"roe>15,pe<15,sector=financial\" --sort roe\n  idx stocks screen -f \"mcap>50T\" --fundamentals --sort margin -n 10\n\nPresets: top-performers (sort ret1y desc), worst-performers (ret1y asc), high-dividend\n(div>0, div desc), low-pe (pe>0, pe asc), 52w-high (range>=95, range desc), 52w-low (range<=5,\nrange asc), high-volume (volume desc), large-cap (mcap desc).\nKeys: pe fpe pb ps peg mcap price div eps volume chg range (from one batch quote call);\n      ret1w ret1m ret3m ret6m retytd ret1y (price return %, as MSN reports it, from one request\n      for MSN's whole-market list, cached 15 minutes);\n      roe roa margin opmargin de growth epsgrowth sector industry (per-company fundamentals,\n      fetched for the largest --max-fundamentals companies that pass the quote filters).\nrange is the position in the 52-week range: 0 at the low, 100 at the high. volume is the\n3-month average.\nOperators: < <= > >= = !=. Numbers accept K M B T suffixes and a trailing %.\nUniverse: bundled list of IDX issuers plus any seen by `idx ownership import`; override with --universe or IDX_UNIVERSE."
    )]
    Screen {
        /// Ready-made screen; -f adds clauses, --sort/--asc/--desc override its order.
        #[arg(long, value_enum)]
        preset: Option<ScreenPreset>,
        /// Comma-separated clauses, e.g. "pe<15,roe>15,sector=Banking".
        #[arg(long, short = 'f', value_name = "SPEC")]
        filter: Option<String>,
        /// Sort key (any numeric filter key). Default: the preset's, else mcap.
        #[arg(long, value_name = "KEY")]
        sort: Option<String>,
        /// Sort ascending (low-pe, worst-performers and 52w-low do by default).
        #[arg(long, conflicts_with = "desc")]
        asc: bool,
        /// Sort descending.
        #[arg(long)]
        desc: bool,
        /// Rows shown; 0 for all.
        #[arg(long, short = 'n', default_value_t = 25)]
        limit: usize,
        /// Cap on per-company fundamentals calls.
        #[arg(long, default_value_t = 50, value_name = "N")]
        max_fundamentals: usize,
        /// Fetch fundamentals for the results even when no filter needs them.
        #[arg(long)]
        fundamentals: bool,
        /// Join MSN's return windows (ret1w..ret1y) even when no filter or the sort needs them.
        #[arg(long)]
        returns: bool,
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
        after_help = "Examples:\n  idx stocks sentiment BBCA\n  idx -o json stocks sentiment BBRI | jq '.data | {label, score, items}'\n\nCounts Indonesian and English finance terms per headline. Indicative only.\nMSN's crowd sentiment is not used: it has no data for IDX tickers (checked 2026-11-06)."
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

    /// MSN Money insights: about 30 rated statements on valuation, earnings, growth, performance and health against peers and the industry.
    #[command(
        after_help = "Examples:\n  idx stocks insights BBCA\n  idx stocks insights TLKM --all          # include the ones MSN could not rate\n  idx -o json stocks insights GOTO | jq '.data.insights[] | select(.status==\"bad\") | .statement'\n\nRule-based statements by MSN Money, each rated good, bad, neutral or not_rated (missing data)\nagainst the company's peers, its industry, its own history, the price or a fixed threshold\n(compared_with). value and benchmark_value are as MSN reports them; units differ by statement.\npeers lists the IDX tickers MSN compares against. Not investment advice."
    )]
    Insights {
        /// IDX ticker, e.g. BBCA.
        #[arg(value_name = "SYMBOL")]
        symbol: String,
        /// Also list the statements MSN could not rate (JSON always carries all).
        #[arg(long)]
        all: bool,
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

    /// Foreign buying and selling per session, with totals and the net-buy/net-sell streak: idx.co.id (shares) or IPOT (rupiah).
    #[command(
        after_help = "Examples:\n  idx stocks foreign BBCA\n  idx stocks foreign BBRI --days 60\n  idx stocks foreign BBCA --from 2026-10-01 --to 2026-10-31\n  idx stocks foreign TLKM --source ipot\n  idx stocks foreign BBCA --source ipot --from 2026-10-12 --to 2026-10-16\n  idx -o json stocks foreign TLKM --source ipot | jq '.data.totals'\n\n--source idx (default): idx.co.id's daily stock summary. Foreign buy and sell in shares on the\nregular board, as IDX publishes them; the net × close column multiplies the net shares by the\nclose for a rupiah scale, which is not the value foreigners traded at. Up to 250 sessions in\none request. When idx.co.id's bot protection refuses, a challenged request is asked again for\n30 s, then a cached copy stands in if there is one.\n--source ipot: Indo Premier's broker summary. Foreign buy, sell and net in rupiah across all\nboards (negotiated trades included) and foreign investors' share of the trading; two requests\nper session (cached), at most 60 sessions; schema idx.foreign.value.v1. The source rounds\nfigures of a million and more to one decimal.\nFor the whole market on one day see `idx market foreign`."
    )]
    Foreign {
        /// IDX ticker, e.g. BBCA.
        symbol: String,
        /// Latest sessions to show (default 20; idx 1-250, ipot 1-60). Not with --from/--to.
        #[arg(long, short = 'n', conflicts_with_all = ["from", "to"])]
        days: Option<usize>,
        /// First session of a range, YYYY-MM-DD (WIB).
        #[arg(long, value_name = "DATE")]
        from: Option<chrono::NaiveDate>,
        /// Last session of the range, YYYY-MM-DD. Default: today.
        #[arg(long, value_name = "DATE", requires = "from")]
        to: Option<chrono::NaiveDate>,
        /// Where the figures come from: idx (shares, regular board) or ipot (rupiah, all boards).
        #[arg(long, value_enum, default_value_t = ForeignSource::Idx)]
        source: ForeignSource,
    },

    /// Side-by-side fundamentals for two or more tickers.
    #[command(
        after_help = "Examples:\n  idx stocks compare BBCA BBRI BMRI BBNI\n  idx -o json stocks compare TLKM,ISAT,EXCL | jq '.data[].valuation.pe_trailing'\n  idx -o jsonl stocks screen -f \"pe<10,roe>15\" -n 5 | idx stocks compare -   # the screen's rows"
    )]
    Compare {
        /// Two or more tickers, comma or space separated: IDX codes, or another Yahoo Finance market's with its suffix (D05.SI); `-` reads them from stdin.
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
        StocksCmd::Intraday {
            symbol,
            interval,
            limit,
        } => {
            let sym = Symbol::parse(&symbol)?;
            let list = universe::load(None).unwrap_or_default();
            let result = service::intraday(ctx.provider.as_ref(), &list, &sym, interval).await?;
            let provider = result.provider;
            let limit = (limit > 0).then_some(limit);
            emit(ctx, SCHEMA_INTRADAY, result, |d| {
                intraday_table::intraday(d, limit, provider, ctx.color)
            })
        }
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
        StocksCmd::Fundamental {
            symbol,
            lens: Some(l),
        } => lens(ctx, &symbol, l).await,
        StocksCmd::Fundamental { symbol, lens: None } => fundamental(ctx, &symbol).await,
        StocksCmd::Growth { symbol } => lens(ctx, &symbol, Lens::Growth).await,
        StocksCmd::Valuation { symbol } => lens(ctx, &symbol, Lens::Valuation).await,
        StocksCmd::Risk { symbol } => lens(ctx, &symbol, Lens::Risk).await,
        StocksCmd::Technical { symbol, period } => technical(ctx, &symbol, period).await,
        StocksCmd::Compare { symbols } => compare(ctx, &symbols).await,
        StocksCmd::Foreign {
            symbol,
            days,
            from,
            to,
            source,
        } => {
            let sym = Symbol::parse(&symbol)?;
            let list = universe::load(None).unwrap_or_default();
            let today = service::today_wib();
            let range = service::ForeignRange::from_args(days, from, to, today)?;
            match source {
                ForeignSource::Idx => {
                    let result =
                        service::foreign_flow(&ctx.fetcher, &list, &sym, range, today).await?;
                    emit(ctx, SCHEMA_FOREIGN, result, |d| {
                        tx::foreign_flow(d, ctx.color)
                    })
                }
                ForeignSource::Ipot => {
                    let result =
                        service::foreign_flow_ipot(&ctx.fetcher, &list, &sym, range, today).await?;
                    emit(ctx, SCHEMA_FOREIGN_VALUE, result, |d| {
                        tx::foreign_value_flow(d, ctx.color)
                    })
                }
            }
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
            preset,
            filter,
            sort,
            asc,
            desc,
            limit,
            max_fundamentals,
            fundamentals,
            returns,
            universe,
        } => {
            let filters = parse_filters(filter.as_deref().unwrap_or(""))?;
            let sort = match sort.as_deref() {
                None => None,
                Some(s) => Some(Key::parse(s).ok_or_else(|| {
                    IdxError::Usage(format!("unknown sort key {s:?}; use one of {}", Key::ALL))
                })?),
            };
            let mut list = universe::load(universe.as_deref())?;
            universe::extend_from_ownership(&mut list, &ctx.settings.ownership_db);
            let opts = ScreenOptions {
                preset,
                filters,
                sort,
                descending: if asc {
                    Some(false)
                } else if desc {
                    Some(true)
                } else {
                    None
                },
                limit,
                max_fundamentals,
                with_fundamentals: fundamentals,
                with_returns: returns,
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
        StocksCmd::Insights { symbol, all } => {
            let sym = Symbol::parse(&symbol)?;
            let result = ctx.provider.insights(&sym).await?;
            emit(ctx, SCHEMA_INSIGHTS, result, |d| {
                tins::insights(d, all, ctx.color)
            })
        }
    }
}

async fn quote(ctx: &Context, raw: &[String]) -> Result<()> {
    let symbols = Symbol::parse_list_on(input::symbol_args(raw)?, &ctx.settings.exchange)?;
    let result = service::quotes(ctx.provider.as_ref(), &symbols).await?;
    emit(ctx, SCHEMA_QUOTES, result, |d| table::quotes(d, ctx.color))
}

async fn history(
    ctx: &Context,
    raw: &str,
    period: Period,
    interval: Interval,
    limit: usize,
) -> Result<()> {
    let symbol = Symbol::parse_on(raw, &ctx.settings.exchange)?;
    interval.check_period(period)?;
    let result = ctx.provider.history(&symbol, period, interval).await?;
    let limit = if limit == 0 { None } else { Some(limit) };
    emit(ctx, SCHEMA_HISTORY, result, |d| {
        table::history(d, limit, ctx.color)
    })
}

async fn profile(ctx: &Context, raw: &str) -> Result<()> {
    let symbol = Symbol::parse_on(raw, &ctx.settings.exchange)?;
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
    let symbol = Symbol::parse_on(raw, &ctx.settings.exchange)?;
    let result = ctx.provider.financials(&symbol, period).await?;
    let flags = Statement::flags(statement);
    emit(ctx, SCHEMA_FINANCIALS, result, |d| {
        reports::financials(d, &flags, ctx.color)
    })
}

async fn earnings(ctx: &Context, raw: &str, filter: EarningsFilter) -> Result<()> {
    let symbol = Symbol::parse_on(raw, &ctx.settings.exchange)?;
    let result = service::earnings(ctx.provider.as_ref(), &symbol, filter).await?;
    emit(ctx, SCHEMA_EARNINGS, result, |d| {
        reports::earnings(d, ctx.color)
    })
}

async fn dividends(ctx: &Context, raw: &str, limit: usize) -> Result<()> {
    let symbol = Symbol::parse_on(raw, &ctx.settings.exchange)?;
    let result = service::dividends(ctx.provider.as_ref(), &symbol).await?;
    emit(ctx, SCHEMA_DIVIDENDS, result, |d| {
        reports::dividends(d, limit, ctx.color)
    })
}

async fn returns(ctx: &Context, raw: &[String], benchmark: &str, none: bool) -> Result<()> {
    let symbols = Symbol::parse_list_on(input::symbol_args(raw)?, &ctx.settings.exchange)?;
    let bench = if none || benchmark.eq_ignore_ascii_case("none") {
        None
    } else {
        Some(Symbol::parse_on(benchmark, &ctx.settings.exchange)?)
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
    let symbol = Symbol::parse_on(raw, &ctx.settings.exchange)?;
    let result = service::fundamentals(ctx.provider.as_ref(), &symbol).await?;
    emit(ctx, SCHEMA_FUNDAMENTALS, result, |d| {
        reports::fundamentals(d, ctx.color)
    })
}

async fn lens(ctx: &Context, raw: &str, which: Lens) -> Result<()> {
    let symbol = Symbol::parse(raw)?;
    match service::lens(ctx.provider.as_ref(), &symbol, which).await? {
        service::LensResult::Growth(r) => {
            emit(ctx, SCHEMA_GROWTH, r, |d| lenses::growth(d, ctx.color))
        }
        service::LensResult::Valuation(r) => emit(ctx, SCHEMA_VALUATION, r, |d| {
            lenses::valuation(d, ctx.color)
        }),
        service::LensResult::Risk(r) => emit(ctx, SCHEMA_RISK, r, |d| lenses::risk(d, ctx.color)),
    }
}

async fn technical(ctx: &Context, raw: &str, period: Period) -> Result<()> {
    let symbol = Symbol::parse_on(raw, &ctx.settings.exchange)?;
    let result = service::technical(ctx.provider.as_ref(), &symbol, period).await?;
    emit(ctx, SCHEMA_TECHNICAL, result, |d| {
        reports::technical(d, ctx.color)
    })
}

async fn compare(ctx: &Context, raw: &[String]) -> Result<()> {
    let symbols = Symbol::parse_list_on(input::symbol_args(raw)?, &ctx.settings.exchange)?;
    let result = service::compare(ctx.provider.as_ref(), &symbols).await?;
    emit(ctx, SCHEMA_COMPARE, result, |d| {
        reports::compare(d, ctx.color)
    })
}
