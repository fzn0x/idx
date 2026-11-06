//! `idx mcp`: the same capabilities as the CLI, exposed as MCP tools over
//! stdio. Every tool returns the exact JSON envelope `idx -o json` prints
//! (see `docs/json-output.md`), so an agent that learned one learned both.
//! Failures come back as tool errors carrying the same `{error, exit_code}`
//! object the CLI writes to stderr.
//!
//! Nothing here may write to stdout except the protocol; logs go to stderr.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, ContentBlock, Implementation, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion, ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::RequestContext;
use rmcp::{
    schemars, tool, tool_handler, tool_router, ErrorData as McpError, RoleServer, ServerHandler,
};
use serde::Deserialize;

use crate::error::{IdxError, Result};
use crate::http::Fetcher;
use crate::model::{
    BandarPeriod, BrokerBoard, BrokerInvestor, Interval, Period, StatementPeriod, Symbol,
    IDX_EXCHANGE,
};
use crate::output::json::{
    self, Envelope, ErrorEnvelope, SCHEMA_BRIEF, SCHEMA_COMPARE, SCHEMA_DIVIDENDS, SCHEMA_EARNINGS,
    SCHEMA_FINANCIALS, SCHEMA_FUNDAMENTALS, SCHEMA_HISTORY, SCHEMA_MARKET,
    SCHEMA_OWNERSHIP_CHANGES, SCHEMA_OWNERSHIP_ENTITY, SCHEMA_OWNERSHIP_FOREIGN,
    SCHEMA_OWNERSHIP_FOREIGN_FLOW, SCHEMA_OWNERSHIP_HOLDERS, SCHEMA_OWNERSHIP_RELEASES,
    SCHEMA_PROFILE, SCHEMA_QUOTES, SCHEMA_RETURNS, SCHEMA_TECHNICAL,
};
use crate::output::json::{
    SCHEMA_ACTIONS, SCHEMA_CALENDAR, SCHEMA_FOREIGN, SCHEMA_INDICES, SCHEMA_INSTRUMENTS,
    SCHEMA_MARKET_BROKERS, SCHEMA_MARKET_FOREIGN, SCHEMA_MEMBERS,
};
use crate::output::json::{
    SCHEMA_BANDAR, SCHEMA_BANDAR_BROKER, SCHEMA_BANDAR_SCREEN, SCHEMA_BROKER, SCHEMA_HOLIDAYS,
    SCHEMA_NEWS, SCHEMA_ORDERBOOK, SCHEMA_SCREEN, SCHEMA_SENTIMENT,
};
use crate::ownership::OwnershipDb;
use crate::providers::stockbit::{login, SessionStore, Stockbit};
use crate::providers::{Provider, Sourced, StockbitOptions, Ttls};
use crate::service;
use crate::universe;

pub mod clients;
pub mod http;
pub mod schema;

const INSTRUCTIONS: &str = "\
idx: Indonesian stock market (IDX / IHSG) data and analysis from free public sources.
Tickers are IDX codes such as BBCA, BBRI, TLKM (no .JK suffix needed).
The quote, history, profile, financials, earnings, dividends, returns, fundamental,
technical and compare tools also take a ticker from another Yahoo Finance market
with its suffix, e.g. D05.SI; every other tool is IDX only.
Every tool returns the same JSON envelope as the `idx -o json` CLI:
{schema, provider, fetched_at, cached, stale, warnings?, data}. Fields the source did
not return are null; nothing is estimated. Prices are delayed as published.
Errors return isError=true with {error:{code,message}, exit_code}; code not_found
means the ticker is unknown, offline_miss means --offline with nothing cached.
idx_broker shows which brokers bought and sold a ticker (IPOT broker summary);
idx_bandar reads it as bandarmology (are the top brokers accumulating or distributing?)
and idx_bandar_screen finds the most traded stocks they accumulate or distribute.
Ownership tools read the local SQLite register of holders above 1% (KSEI via IDX,
monthly); if it is empty they say so and the user must run `idx ownership import --latest`.
Verdicts and signals are transparent rule scores, not investment advice.";

// Tickers for the Yahoo-backed tools (idx_quote, idx_compare).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolsArg {
    /// One or more tickers, comma or space separated, e.g. "BBCA,BBRI,TLKM". A bare code is IDX; another Yahoo Finance market's ticker takes its suffix, e.g. "D05.SI".
    pub symbols: String,
}

// One ticker for an IDX-only tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SymbolArg {
    /// One IDX ticker, e.g. "BBCA".
    pub symbol: String,
}

// One ticker for a Yahoo-backed tool (idx_profile, idx_earnings,
// idx_dividends, idx_fundamental).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct YahooSymbolArg {
    /// One ticker, e.g. "BBCA". A bare code is IDX; another Yahoo Finance market's ticker takes its suffix, e.g. "D05.SI".
    pub symbol: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StockbitLoginArgs {
    /// Seconds to wait for the sign-in (default 300, clamped to 30..900).
    #[serde(default)]
    pub timeout: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HistoryArgs {
    /// One ticker, e.g. "TLKM". A bare code is IDX; another Yahoo Finance market's ticker takes its suffix, e.g. "D05.SI".
    pub symbol: String,
    /// Look-back window: 1d, 5d, 1mo, 3mo, 6mo, 1y (default), 2y, 5y, 10y, ytd, max.
    #[serde(default)]
    pub period: Option<String>,
    /// Candle size: 5m, 15m, 30m, 1h, 1d (default), 1wk, 1mo. Intraday sizes need short periods.
    #[serde(default)]
    pub interval: Option<String>,
    /// Keep only the most recent N candles (default 60). 0 returns all of them.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FinancialsArgs {
    /// One ticker, e.g. "BBCA". A bare code is IDX; another Yahoo Finance market's ticker takes its suffix, e.g. "D05.SI".
    pub symbol: String,
    /// annual (default, ~4 fiscal years), quarterly (~5 quarters) or ttm (trailing twelve months).
    #[serde(default)]
    pub period: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TechnicalArgs {
    /// One ticker, e.g. "BBRI". A bare code is IDX; another Yahoo Finance market's ticker takes its suffix, e.g. "D05.SI".
    pub symbol: String,
    /// Daily candles to analyse: 6mo, 1y (default), 2y, 5y. Shorter windows leave the 200-day averages null.
    #[serde(default)]
    pub period: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OwnershipTickerArgs {
    /// One IDX ticker, e.g. "BBCA".
    pub symbol: String,
    /// Release date YYYY-MM-DD (default: newest imported). idx_ownership_releases lists them.
    #[serde(default)]
    pub as_of: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OwnershipEntityArgs {
    /// Part of a holder's name (person or institution), at least 3 characters, e.g. "Dwimuria".
    pub name: String,
    /// Release date YYYY-MM-DD (default: newest imported).
    #[serde(default)]
    pub as_of: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OwnershipChangesArgs {
    /// Earlier release date YYYY-MM-DD.
    pub from: String,
    /// Later release date YYYY-MM-DD.
    pub to: String,
    /// Restrict to one ticker.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Ignore moves smaller than this many percentage points (default 0).
    #[serde(default)]
    pub min_pct_change: Option<f64>,
    /// Rows per page, largest moves first (default 50; 0 = all).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Rows to skip before the page (default 0). total_matches and offset in the result tell you when to stop.
    #[serde(default)]
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OwnershipForeignArgs {
    /// One IDX ticker for its month-by-month foreign share; omit for the market-wide flow between two releases.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Earlier release YYYY-MM-DD for the market-wide flow (default: the release before `to`).
    #[serde(default)]
    pub from: Option<String>,
    /// Later release YYYY-MM-DD (default: newest imported).
    #[serde(default)]
    pub to: Option<String>,
    /// Rows in the market-wide flow (default 25; 0 = all).
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScreenArgs {
    /// Comma-separated clauses such as "pe<15,roe>15,sector=financial,mcap>10T". Keys: pe fpe pb ps peg mcap price div eps volume chg roe roa margin opmargin de growth epsgrowth sector industry. Operators < <= > >= = !=; numbers accept K/M/B/T and %.
    pub filter: String,
    /// Sort key (any filter key), default mcap, descending.
    #[serde(default)]
    pub sort: Option<String>,
    /// Rows returned (default 25, 0 = all).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Cap on per-company fundamentals fetches when a filter needs them (default 50).
    #[serde(default)]
    pub max_fundamentals: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NewsArgs {
    /// Optional IDX ticker; omit for general market headlines.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Publisher names: kontan, cnbcindonesia, bisnis, all (default: kontan + cnbcindonesia).
    #[serde(default)]
    pub sources: Option<Vec<String>>,
    /// Headlines returned (default 20, 0 = all).
    #[serde(default)]
    pub limit: Option<u32>,
    /// With a symbol: return only headlines that mention the company, no market context (default false).
    #[serde(default)]
    pub company_only: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MarketArgs {
    /// Rows per list (default 10).
    #[serde(default)]
    pub top: Option<u32>,
    /// Market-cap floor in IDR for gainers/losers (default 1e12 = 1T).
    #[serde(default)]
    pub min_market_cap: Option<f64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct InstrumentsArgs {
    /// Listing board, case-insensitive substring: utama, pengembangan, akselerasi, ekonomi baru, pemantauan khusus.
    #[serde(default)]
    pub board: Option<String>,
    /// Code or company name, case-insensitive substring, e.g. "bank".
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MembersArgs {
    /// Broker code or name, case-insensitive substring, e.g. "mandiri" or "YP".
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IndicesArgs {
    /// Index codes to keep, comma or space separated, e.g. "LQ45,IDX30" (IHSG means COMPOSITE). Default: every index.
    #[serde(default)]
    pub codes: Option<String>,
    /// Trading day, YYYY-MM-DD. Default: the latest session published.
    #[serde(default)]
    pub date: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ForeignArgs {
    /// One IDX ticker, e.g. "BBCA".
    pub symbol: String,
    /// Latest sessions to return, 1-250 (default 20).
    #[serde(default)]
    pub days: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MarketBrokersArgs {
    /// Trading day, YYYY-MM-DD. Default: the latest session published.
    #[serde(default)]
    pub date: Option<String>,
    /// Rows (default 20; 0 for every member).
    #[serde(default)]
    pub top: Option<u32>,
    /// Ranking figure: "value" (default), "volume" or "frequency".
    #[serde(default)]
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct MarketForeignArgs {
    /// Trading day, YYYY-MM-DD. Default: the latest session published.
    #[serde(default)]
    pub date: Option<String>,
    /// Rows per list (default 10; 0 for all).
    #[serde(default)]
    pub top: Option<u32>,
    /// Ranking figure: "value" (net shares times close, default) or "shares".
    #[serde(default)]
    pub sort: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ActionsArgs {
    /// One IDX ticker, e.g. "BBCA".
    pub symbol: String,
    /// Events from this date on, YYYY-MM-DD (upcoming ones included). Default: one year ago.
    #[serde(default)]
    pub since: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CalendarArgs {
    /// One day, YYYY-MM-DD. Not with from/to.
    #[serde(default)]
    pub date: Option<String>,
    /// Start of the range, YYYY-MM-DD. Default: today.
    #[serde(default)]
    pub from: Option<String>,
    /// End of the range, YYYY-MM-DD. Default: from plus days. At most 92 days in all.
    #[serde(default)]
    pub to: Option<String>,
    /// Days to look ahead when to is not given (default 14).
    #[serde(default)]
    pub days: Option<u32>,
    /// One issuer code, e.g. "BBCA".
    #[serde(default)]
    pub symbol: Option<String>,
    /// Comma-separated types: dividend, rights, bonus, split, warrant, rups, public_expose, buyback, listing, bond, other; or "all". Default: everything but bond and buyback.
    #[serde(default)]
    pub types: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HolidaysArgs {
    /// Calendar year, e.g. 2027. Default: the current year; 0 lists every year on the list.
    #[serde(default)]
    pub year: Option<u32>,
    /// Only holidays from today on (default false).
    #[serde(default)]
    pub upcoming: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReturnsArgs {
    /// One or more tickers, comma or space separated, e.g. "BBCA,BBRI". A bare code is IDX; another Yahoo Finance market's ticker takes its suffix, e.g. "D05.SI".
    pub symbols: String,
    /// Index for excess returns: "^JKSE" (IHSG, default), "^JKLQ45", or "none" to skip it.
    #[serde(default)]
    pub benchmark: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SentimentArgs {
    /// One IDX ticker.
    pub symbol: String,
    #[serde(default)]
    pub sources: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BrokerArgs {
    /// One or more IDX tickers (up to 10), comma or space separated, e.g. "BBCA,BBRI".
    pub symbols: String,
    /// One trading day, YYYY-MM-DD. Omit date, from and to for each ticker's latest session with trades.
    #[serde(default)]
    pub date: Option<String>,
    /// Start of a range, YYYY-MM-DD; the end defaults to today. Not with date.
    #[serde(default)]
    pub from: Option<String>,
    /// End of a range, YYYY-MM-DD. Not with date.
    #[serde(default)]
    pub to: Option<String>,
    /// Whose trades to count: all (default), foreign or domestic.
    #[serde(default)]
    pub investor: Option<String>,
    /// Market board: all (default), regular, cash or negotiated.
    #[serde(default)]
    pub board: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BandarArgs {
    /// One or more IDX tickers (up to 10; 3 with daily), comma or space separated, e.g. "BBCA,BBRI".
    pub symbols: String,
    /// Look-back ending at the latest session with trades: 1d, 1w (default), 1m or 3m. Ignored with date or from/to.
    #[serde(default)]
    pub period: Option<String>,
    /// One trading day, YYYY-MM-DD. Not with from/to.
    #[serde(default)]
    pub date: Option<String>,
    /// Start of an explicit range, YYYY-MM-DD; the end defaults to today.
    #[serde(default)]
    pub from: Option<String>,
    /// End of an explicit range, YYYY-MM-DD.
    #[serde(default)]
    pub to: Option<String>,
    /// Brokers per side behind the headline signal: 1, 3, 5 (default) or 10.
    #[serde(default)]
    pub top: Option<u32>,
    /// Whose trades to count: all (default), foreign or domestic.
    #[serde(default)]
    pub investor: Option<String>,
    /// Market board: all (default), regular, cash or negotiated.
    #[serde(default)]
    pub board: Option<String>,
    /// Add the session-by-session movement (default false; one request per session).
    #[serde(default)]
    pub daily: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BandarBrokerArgs {
    /// Exchange-member code, e.g. "YP" or "CC" (see idx_members).
    pub code: String,
    /// Look-back ending at the latest session: 1d, 1w (default), 1m or 3m.
    #[serde(default)]
    pub period: Option<String>,
    /// Rows per side (default 20; 0 for all).
    #[serde(default)]
    pub limit: Option<u32>,
    /// How many of the most traded stocks to scan (default 100, at most 300).
    #[serde(default)]
    pub scan: Option<u32>,
    /// Scan these tickers instead of the most traded, comma or space separated.
    #[serde(default)]
    pub symbols: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BandarScreenArgs {
    /// Ready-made screen: big-accumulation, accumulation, distribution, big-distribution, quiet-accumulation, quiet-distribution, foreign-accumulation or below-bandar.
    #[serde(default)]
    pub preset: Option<String>,
    /// Extra comma-separated clauses, combined with the preset, e.g. "acc>=5,fpct>0,gain<0". Keys: acc (top-N net, % of value), net (IDR), lots, fnet (foreign net, IDR), fpct (foreign net, % of value), gain (price vs top buyers' average, %), chg (price move, %), turnover, conc (top buyers' share of value, %), price, signal=..., phase=...
    #[serde(default)]
    pub filter: Option<String>,
    /// Look-back ending at the latest session: 1d, 1w (default), 1m or 3m.
    #[serde(default)]
    pub period: Option<String>,
    /// Brokers per side: 1, 3, 5 (default) or 10.
    #[serde(default)]
    pub top: Option<u32>,
    /// Sort key (any numeric filter key), default acc.
    #[serde(default)]
    pub sort: Option<String>,
    /// Sort ascending; the distribution presets default to it.
    #[serde(default)]
    pub ascending: Option<bool>,
    /// Rows returned (default 25, 0 = all).
    #[serde(default)]
    pub limit: Option<u32>,
    /// How many of the most traded stocks to scan (default 100, at most 300).
    #[serde(default)]
    pub scan: Option<u32>,
    /// Scan these tickers instead of the most traded, comma or space separated.
    #[serde(default)]
    pub symbols: Option<String>,
}

/// What the Stockbit tools need: the session file + env seeds, and the TTLs for
/// the Stockbit provider they build on demand.
#[derive(Clone, Default)]
pub struct StockbitConfig {
    pub options: StockbitOptions,
    pub ttls: Ttls,
}

#[derive(Clone)]
pub struct IdxServer {
    provider: Arc<dyn Provider>,
    fetcher: Arc<Fetcher>,
    ownership_db: PathBuf,
    stockbit: StockbitConfig,
    tool_router: ToolRouter<IdxServer>,
}

fn parse_date(v: &Option<String>) -> Result<Option<chrono::NaiveDate>> {
    match v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map(Some)
            .map_err(|_| IdxError::Usage(format!("invalid date {s:?}; use YYYY-MM-DD"))),
    }
}

fn ok<T: serde::Serialize>(schema: &str, s: Sourced<T>) -> Result<CallToolResult> {
    let env = Envelope::from_sourced(schema, s);
    Ok(CallToolResult::success(vec![ContentBlock::text(
        json::to_pretty(&env)?,
    )]))
}

/// Tool-level failure: the agent sees the same error object the CLI prints.
fn fail(e: IdxError) -> CallToolResult {
    let env = ErrorEnvelope::from(&e);
    let text = serde_json::to_string_pretty(&env).unwrap_or_else(|_| e.to_string());
    CallToolResult::error(vec![ContentBlock::text(text)])
}

fn finish(r: Result<CallToolResult>) -> std::result::Result<CallToolResult, McpError> {
    Ok(r.unwrap_or_else(fail))
}

fn parse_opt<T: std::str::FromStr<Err = IdxError>>(v: &Option<String>, default: T) -> Result<T> {
    match v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => s.parse(),
        None => Ok(default),
    }
}

#[tool_router]
impl IdxServer {
    pub fn new(
        provider: Arc<dyn Provider>,
        fetcher: Arc<Fetcher>,
        ownership_db: PathBuf,
        stockbit: StockbitConfig,
    ) -> Self {
        IdxServer {
            provider,
            fetcher,
            ownership_db,
            stockbit,
            tool_router: Self::tool_router(),
        }
    }

    fn db(&self) -> Result<OwnershipDb> {
        OwnershipDb::open(&self.ownership_db)
    }

    fn stockbit_store(&self) -> Arc<SessionStore> {
        self.stockbit.options.store()
    }

    fn stockbit_provider(&self) -> Stockbit {
        Stockbit::new(
            self.fetcher.clone(),
            self.stockbit.ttls,
            self.stockbit_store(),
        )
    }

    /// Names of the tools, for tests and docs.
    pub fn tool_names(&self) -> Vec<String> {
        self.tool_router
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect()
    }

    /// The tool list with input schemas rewritten into the subset every
    /// harness accepts (no `$schema`, no `format`, no nullable type arrays).
    pub fn tools(&self) -> Vec<Tool> {
        self.tool_router
            .list_all()
            .into_iter()
            .map(|mut t| {
                let mut v = serde_json::Value::Object((*t.input_schema).clone());
                schema::portable(&mut v);
                if let serde_json::Value::Object(m) = v {
                    t.input_schema = Arc::new(m);
                }
                t
            })
            .collect()
    }

    #[tool(
        name = "idx_quote",
        description = "Delayed quotes for one or more IDX tickers: price, change, previous close, OHLC, volume, market cap, P/E, dividend yield, 52-week range. Returns the idx.quotes.v1 envelope; unknown tickers are listed in warnings."
    )]
    async fn quote(
        &self,
        Parameters(a): Parameters<SymbolsArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbols = Symbol::parse_list_on([a.symbols.as_str()], IDX_EXCHANGE)?;
                ok(SCHEMA_QUOTES, self.provider.quotes(&symbols).await?)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_history",
        description = "OHLCV candles for one IDX ticker, oldest first, with a summary (first/last close, change %, high, low, average volume). Returns the idx.history.v1 envelope. Use limit to bound the payload; the summary always covers the whole period."
    )]
    async fn history(
        &self,
        Parameters(a): Parameters<HistoryArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse_on(&a.symbol, IDX_EXCHANGE)?;
                let period = parse_opt(&a.period, Period::Y1)?;
                let interval = parse_opt(&a.interval, Interval::Day1)?;
                let mut h = self.provider.history(&symbol, period, interval).await?;
                let limit = a.limit.unwrap_or(60) as usize;
                if limit > 0 && h.data.candles.len() > limit {
                    let skip = h.data.candles.len() - limit;
                    h.data.candles.drain(..skip);
                    h.warnings.push(format!(
                        "candles truncated to the last {limit}; summary covers the full period"
                    ));
                }
                ok(SCHEMA_HISTORY, h)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_profile",
        description = "Company profile for one IDX ticker: name, sector, industry, business description, employees, founding year, headquarters, website, market cap, shares outstanding, board and management. Returns the idx.profile.v1 envelope."
    )]
    async fn profile(
        &self,
        Parameters(a): Parameters<YahooSymbolArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse_on(&a.symbol, IDX_EXCHANGE)?;
                ok(SCHEMA_PROFILE, self.provider.profile(&symbol).await?)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_financials",
        description = "Income statement, balance sheet and cash flow for one IDX ticker, oldest period first, amounts in reporting currency (IDR). Returns the idx.financials.v1 envelope. Banks have no EBITDA/gross profit series (null)."
    )]
    async fn financials(
        &self,
        Parameters(a): Parameters<FinancialsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse_on(&a.symbol, IDX_EXCHANGE)?;
                let period = match a.period.as_deref().map(str::trim) {
                    None | Some("") | Some("annual") => StatementPeriod::Annual,
                    Some("quarterly") => StatementPeriod::Quarterly,
                    Some("ttm") => StatementPeriod::Ttm,
                    Some(other) => {
                        return Err(IdxError::Usage(format!(
                            "invalid period {other:?}; use annual, quarterly or ttm"
                        )))
                    }
                };
                ok(
                    SCHEMA_FINANCIALS,
                    self.provider.financials(&symbol, period).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_earnings",
        description = "Reported quarterly EPS versus analyst estimates with surprise %, fiscal-year and quarterly revenue/earnings, the next earnings date with consensus, and dividend dates for one IDX ticker. Returns the idx.earnings.v1 envelope."
    )]
    async fn earnings(
        &self,
        Parameters(a): Parameters<YahooSymbolArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse_on(&a.symbol, IDX_EXCHANGE)?;
                ok(SCHEMA_EARNINGS, self.provider.earnings(&symbol).await?)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_dividends",
        description = "Cash dividend and stock split history for one IDX ticker: every payment (ex-date, per-share amount as reported, split-adjusted), yearly totals with year-over-year change, trailing-twelve-month total and yield (TTM total / last price, computed by idx), the latest payment and the consecutive-years streak. Returns the idx.dividends.v1 envelope."
    )]
    async fn dividends(
        &self,
        Parameters(a): Parameters<YahooSymbolArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse_on(&a.symbol, IDX_EXCHANGE)?;
                ok(
                    SCHEMA_DIVIDENDS,
                    service::dividends(self.provider.as_ref(), &symbol).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_brief",
        description = "Start here for one ticker: a single call that returns the quote, fundamentals with the valuation verdict, one-year technicals with the signal, returns versus IHSG, dividend facts (TTM yield, streak), the local >1% holder register (top five, foreign share) and company headlines with sentiment, plus key_points (plain-language one-liners) and sections_failed for anything a source could not provide. Each section has the same shape as the dedicated tool. Returns the idx.brief.v1 envelope."
    )]
    async fn brief(
        &self,
        Parameters(a): Parameters<SymbolArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                let list = universe::load(None).unwrap_or_default();
                ok(
                    SCHEMA_BRIEF,
                    service::brief(
                        self.provider.as_ref(),
                        &self.fetcher,
                        &list,
                        &self.ownership_db,
                        &symbol,
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_market",
        description = "Whole-market overview from one pass over ~950 IDX tickers: IHSG (^JKSE) quote and market state, breadth (advancing/declining/unchanged), total turnover, and top lists of gainers, losers (both above a market-cap floor), most traded by value and heavyweights by market cap. Call this first for 'how is the market today'. Returns the idx.market.v1 envelope."
    )]
    async fn market(
        &self,
        Parameters(a): Parameters<MarketArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let mut list = universe::load(None)?;
                universe::extend_from_ownership(&mut list, &self.ownership_db);
                let index = Symbol::parse("^JKSE")?;
                ok(
                    SCHEMA_MARKET,
                    service::market_overview(
                        self.provider.as_ref(),
                        &list,
                        &index,
                        a.top.unwrap_or(10) as usize,
                        a.min_market_cap.unwrap_or(1e12),
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_instruments",
        description = "Listed stocks from idx.co.id: code, name (IDX cuts long names at 30 characters), listing board (Utama, Pengembangan, Akselerasi, Ekonomi Baru, Pemantauan Khusus), first listing date and listed shares, sorted by code. Filter by board or by a code/name substring. When idx.co.id cannot be reached and nothing is cached, the bundled universe stands in (codes and names only) and a warning says so. Returns the idx.instruments.v1 envelope."
    )]
    async fn instruments(
        &self,
        Parameters(a): Parameters<InstrumentsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                ok(
                    SCHEMA_INSTRUMENTS,
                    service::instruments(&self.fetcher, a.board.as_deref(), a.search.as_deref())
                        .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_members",
        description = "IDX exchange members (Anggota Bursa): the broker codes idx_broker and idx_bandar report (e.g. CC, YP, ZP) with the member name and licences (Perantara Pedagang Efek = broker-dealer, Penjamin Emisi Efek = underwriter). Filter by a code or name substring. From idx.co.id; the bundled member list stands in when it cannot be reached. Returns the idx.members.v1 envelope."
    )]
    async fn members(
        &self,
        Parameters(a): Parameters<MembersArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                ok(
                    SCHEMA_MEMBERS,
                    service::members(&self.fetcher, a.search.as_deref()).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_indices",
        description = "Every IDX index for one trading day from idx.co.id's index summary: COMPOSITE (IHSG), LQ45, IDX30, IDX80, sector and thematic indices, each with previous close, high, low, close, change and change percent, number of constituents, value traded and market capitalisation (IDR). Default: the latest session published (weekends and trading holidays skipped); pass codes to keep only some. Returns the idx.indices.v1 envelope."
    )]
    async fn indices(
        &self,
        Parameters(a): Parameters<IndicesArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let codes: Vec<String> = a.codes.into_iter().collect();
                ok(
                    SCHEMA_INDICES,
                    service::indices(
                        &self.fetcher,
                        parse_date(&a.date)?,
                        &codes,
                        service::today_wib(),
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_foreign",
        description = "Foreign investors' buying and selling of one IDX stock over its latest sessions (default 20, at most 250) from idx.co.id's daily stock summary, oldest first: close, change, volume, value, foreign buy and sell (shares), net shares and net shares times the close (a rupiah scale, not the traded value; IDX publishes foreign trading in shares only). Totals give the sums, how many sessions were net buying or net selling and the current streak (+3 = three net-buy sessions in a row). Returns the idx.foreign.v1 envelope."
    )]
    async fn foreign(
        &self,
        Parameters(a): Parameters<ForeignArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                let list = universe::load(None).unwrap_or_default();
                let days = a.days.unwrap_or(service::FOREIGN_DEFAULT_DAYS as u32) as usize;
                ok(
                    SCHEMA_FOREIGN,
                    service::foreign_flow(&self.fetcher, &list, &symbol, days).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_market_brokers",
        description = "Top brokers: every IDX exchange member's trading for one session from idx.co.id's daily broker summary (default: the latest published), ranked by value (default), volume or frequency, each with code, member name, value (IDR), volume (shares), frequency and its share of all members' value. A member's buying and selling count together, so all members' value is twice the market's. For which stocks one broker traded use idx_bandar_broker; for one stock's brokers idx_broker. Returns the idx.market.brokers.v1 envelope."
    )]
    async fn market_brokers(
        &self,
        Parameters(a): Parameters<MarketBrokersArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                ok(
                    SCHEMA_MARKET_BROKERS,
                    service::market_brokers(
                        &self.fetcher,
                        parse_date(&a.date)?,
                        a.top.unwrap_or(20) as usize,
                        parse_opt(&a.sort, crate::model::BrokerSort::Value)?,
                        service::today_wib(),
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_market_foreign",
        description = "Foreign net buying and selling across every IDX stock for one session from idx.co.id's daily stock summary (default: the latest published; weekends and holidays skipped): market totals of foreign buy, sell and net shares, then the top stocks foreigners net bought and net sold, each with close, change percent, foreign buy, sell, net shares and net shares times the close. Ranked by net shares times close (value, default; a rupiah scale, not the traded value) or by net shares. Returns the idx.market.foreign.v1 envelope."
    )]
    async fn market_foreign(
        &self,
        Parameters(a): Parameters<MarketForeignArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                ok(
                    SCHEMA_MARKET_FOREIGN,
                    service::market_foreign(
                        &self.fetcher,
                        parse_date(&a.date)?,
                        a.top.unwrap_or(10) as usize,
                        parse_opt(&a.sort, crate::model::ForeignSort::Value)?,
                        service::today_wib(),
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_corporate_actions",
        description = "One IDX stock's corporate actions from IPOT, upcoming ones included: cash and stock dividends, stock splits and reverse splits, bonus shares and rights issues, each with kind, the source's label, amount (IDR per share: the dividend, or a rights issue's exercise price), ratio old:new (1:5 split, 2:1 rights), cum, ex, recording and payment dates, status (Final, Tentative, ...) and the announcement text; plus shareholders' meetings (RUPS: date, time, venue, agenda) and listed warrants and rights (code, exercise price, issued, listing and maturity dates). Default: events from one year ago on. Returns the idx.actions.v1 envelope."
    )]
    async fn corporate_actions(
        &self,
        Parameters(a): Parameters<ActionsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                let list = universe::load(None).unwrap_or_default();
                ok(
                    SCHEMA_ACTIONS,
                    service::corporate_actions(
                        &self.fetcher,
                        &list,
                        &symbol,
                        parse_date(&a.since)?,
                        service::today_wib(),
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_calendar",
        description = "The IDX exchange calendar from idx.co.id over a day or a range (default: the next 14 days; at most 92): dividend dates (cum, ex, recording, payment), rights issue (HMETD) dates, bonus, split, warrant, shareholders' meetings (RUPS: plan, notice, result, with the meeting time and venue), public exposes, listings, and on request bond events and buyback reports. Each entry has the date, issuer code, category and milestone (classified by idx from the entry's wording), the text as published and the source's own type fields. Filter by symbol and types. Returns the idx.calendar.v1 envelope."
    )]
    async fn calendar(
        &self,
        Parameters(a): Parameters<CalendarArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let date = parse_date(&a.date)?;
                if date.is_some() && (a.from.is_some() || a.to.is_some()) {
                    return Err(IdxError::Usage("use date or from/to, not both".into()));
                }
                let q = service::CalendarQuery {
                    date,
                    from: parse_date(&a.from)?,
                    to: parse_date(&a.to)?,
                    days: a.days,
                    symbol: a.symbol,
                    categories: crate::model::CalendarCategory::parse_list(
                        a.types.as_deref().unwrap_or("default"),
                    )?,
                };
                ok(
                    SCHEMA_CALENDAR,
                    service::calendar(&self.fetcher, &q, service::today_wib()).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_holidays",
        description = "IDX trading holidays (Kalender Libur Bursa) for a year: each closed weekday with its Indonesian and English name, the year's trading-day count, whether today (Jakarta) is a trading day and the next trading day. Weekend holidays are not listed, as IDX publishes them; IDX can add a holiday later. Bundled from the yearly calendar idx.co.id publishes, so it works offline. Returns the idx.holidays.v1 envelope."
    )]
    async fn holidays(
        &self,
        Parameters(a): Parameters<HolidaysArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let today = service::today_wib();
                let year = match a.year {
                    Some(0) => None,
                    Some(y) => Some(
                        i32::try_from(y)
                            .map_err(|_| IdxError::Usage(format!("year {y} is out of range")))?,
                    ),
                    None => Some(chrono::Datelike::year(&today)),
                };
                ok(
                    SCHEMA_HOLIDAYS,
                    service::holidays(year, a.upcoming.unwrap_or(false), today)?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_returns",
        description = "Price performance for one or more IDX tickers computed from five years of daily closes: total returns over 1w, 1m, 3m, 6m, YTD, 1y, 3y, 5y; CAGR 3y/5y; annualised 1y volatility; max drawdown (1y and whole window, with peak/trough dates); best and worst day; 52-week range and distance from the high; plus the benchmark index's returns and the ticker's excess over it (IHSG ^JKSE by default). Dividends are excluded. Returns the idx.returns.v1 envelope (array, order as requested)."
    )]
    async fn returns(
        &self,
        Parameters(a): Parameters<ReturnsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbols = Symbol::parse_list_on([a.symbols.as_str()], IDX_EXCHANGE)?;
                let bench = match a.benchmark.as_deref().map(str::trim) {
                    None | Some("") => Some(Symbol::parse("^JKSE")?),
                    Some(s) if s.eq_ignore_ascii_case("none") => None,
                    Some(s) => Some(Symbol::parse_on(s, IDX_EXCHANGE)?),
                };
                ok(
                    SCHEMA_RETURNS,
                    service::returns(self.provider.as_ref(), &symbols, bench.as_ref()).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_fundamental",
        description = "Fundamentals report for one IDX ticker: valuation (P/E, forward P/E, PEG, P/B, P/S, EV/EBITDA), profitability (ROE, ROA, margins), balance sheet (debt/equity, current ratio, cash, debt), growth, dividend, per-share figures, analyst targets, and a plain-language verdict (undervalued | fair | overvalued | insufficient_data) with every rule that fired listed in reasons. Returns the idx.fundamentals.v1 envelope."
    )]
    async fn fundamental(
        &self,
        Parameters(a): Parameters<YahooSymbolArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse_on(&a.symbol, IDX_EXCHANGE)?;
                ok(
                    SCHEMA_FUNDAMENTALS,
                    service::fundamentals(self.provider.as_ref(), &symbol).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_technical",
        description = "Technical analysis of one IDX ticker from daily candles: RSI(14), MACD(12,26,9), SMA/EMA 20/50/200, Bollinger(20,2), ATR(14), volume vs 20-day average, support/resistance from swing points, 52-week range, trend label and a buy | sell | neutral signal with reasons. Returns the idx.technical.v1 envelope."
    )]
    async fn technical(
        &self,
        Parameters(a): Parameters<TechnicalArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse_on(&a.symbol, IDX_EXCHANGE)?;
                let period = parse_opt(&a.period, Period::Y1)?;
                ok(
                    SCHEMA_TECHNICAL,
                    service::technical(self.provider.as_ref(), &symbol, period).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_compare",
        description = "Side-by-side fundamentals (same fields as idx_fundamental, verdict included) for two or more IDX tickers. Returns the idx.compare.v1 envelope: data is an array in the order requested; tickers that failed are listed in warnings."
    )]
    async fn compare(
        &self,
        Parameters(a): Parameters<SymbolsArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbols = Symbol::parse_list_on([a.symbols.as_str()], IDX_EXCHANGE)?;
                ok(
                    SCHEMA_COMPARE,
                    service::compare(self.provider.as_ref(), &symbols).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_screen",
        description = "Screens the IDX universe (~950 tickers) with a filter string such as \"pe<15,roe>15,sector=financial,mcap>10T\". Quote-level keys (pe fpe pb mcap price div eps volume chg) come from one batch quote; fundamentals keys (roe roa margin opmargin de growth epsgrowth peg ps sector industry) are fetched per company for the largest max_fundamentals survivors. Returns the idx.screen.v1 envelope with counts explaining how many tickers each stage kept."
    )]
    async fn screen(
        &self,
        Parameters(a): Parameters<ScreenArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let filters = crate::analysis::screen::parse_filters(&a.filter)?;
                let sort = match a.sort.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    None => crate::analysis::screen::Key::Mcap,
                    Some(s) => crate::analysis::screen::Key::parse(s).ok_or_else(|| {
                        IdxError::Usage(format!(
                            "unknown sort key {s:?}; use one of {}",
                            crate::analysis::screen::Key::ALL
                        ))
                    })?,
                };
                let mut list = universe::load(None)?;
                universe::extend_from_ownership(&mut list, &self.ownership_db);
                let opts = service::ScreenOptions {
                    filters,
                    sort,
                    descending: true,
                    limit: a.limit.unwrap_or(25) as usize,
                    max_fundamentals: a.max_fundamentals.unwrap_or(50) as usize,
                    with_fundamentals: false,
                };
                ok(
                    SCHEMA_SCREEN,
                    service::screen(self.provider.as_ref(), &list, &opts).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_broker",
        description = "Broker summary for one or more IDX tickers (Indo Premier / IPOT): which brokers bought and sold the stock over one day or a date range. Top 10 buyers and top 10 sellers as the source ranks them (by lots), each with the broker code, name (IDX member list), class (local, foreign, state_owned = BUMN), lots, value in IDR and average price, plus total value, total lots, average price and foreign net value. Without dates each ticker gets its latest session with trades. The source rounds figures of a million and more to one decimal; such summaries carry rounded=true. Returns the idx.broker.v1 envelope; failed tickers are listed in warnings."
    )]
    async fn broker(
        &self,
        Parameters(a): Parameters<BrokerArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbols = Symbol::parse_list([a.symbols.as_str()])?;
                let date = parse_date(&a.date)?;
                let (from, to) = match date {
                    Some(d) if a.from.is_some() || a.to.is_some() => {
                        return Err(IdxError::Usage(format!(
                            "use date ({d}) or from/to, not both"
                        )))
                    }
                    Some(d) => (Some(d), Some(d)),
                    None => (parse_date(&a.from)?, parse_date(&a.to)?),
                };
                let query = service::BrokerQuery {
                    from,
                    to,
                    investor: parse_opt(&a.investor, BrokerInvestor::All)?,
                    board: parse_opt(&a.board, BrokerBoard::All)?,
                };
                let list = universe::load(None).unwrap_or_default();
                ok(
                    SCHEMA_BROKER,
                    service::broker(&self.fetcher, &list, &symbols, query, service::today_wib())
                        .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_bandar",
        description = "Bandarmology (bandar detector) for one or more IDX tickers, from the IPOT broker summary: are the top brokers accumulating or distributing? For N = 1, 3, 5 and 10 the top N buyers' value minus the top N sellers' value (bandar value; in lots bandar volume) as a percent of all value traded: +15% or more big_accumulation, +5% accumulation, -5% distribution, -15% or less big_distribution, otherwise neutral. The headline signal uses top (default 5). phase reads it against the price move over the range (accumulation with the price within +2% = quiet_accumulation, distribution with the price within -2% = quiet_distribution, else markup/markdown). bandar_average is the top buyers' average price and bandar_gain_pct the last price against it; foreign_net_pct the foreign flow. Default period 1w ending at the latest session; daily=true adds the session-by-session movement and streak. reasons explain the reading. Heuristic, not advice. Returns the idx.bandar.v1 envelope; failed tickers are listed in warnings."
    )]
    async fn bandar(
        &self,
        Parameters(a): Parameters<BandarArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbols = Symbol::parse_list([a.symbols.as_str()])?;
                let date = parse_date(&a.date)?;
                let (from, to) = match date {
                    Some(d) if a.from.is_some() || a.to.is_some() => {
                        return Err(IdxError::Usage(format!(
                            "use date ({d}) or from/to, not both"
                        )))
                    }
                    Some(d) => (Some(d), Some(d)),
                    None => (parse_date(&a.from)?, parse_date(&a.to)?),
                };
                let query = service::BandarQuery {
                    period: parse_opt(&a.period, BandarPeriod::Week1)?,
                    from,
                    to,
                    top: a.top.unwrap_or(crate::analysis::bandar::DEFAULT_TOP),
                    investor: parse_opt(&a.investor, BrokerInvestor::All)?,
                    board: parse_opt(&a.board, BrokerBoard::All)?,
                    daily: a.daily.unwrap_or(false),
                };
                let list = universe::load(None).unwrap_or_default();
                ok(
                    SCHEMA_BANDAR,
                    service::bandar(
                        self.provider.as_ref(),
                        &self.fetcher,
                        &list,
                        &symbols,
                        query,
                        service::today_wib(),
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_bandar_broker",
        description = "Broker activity: where one IDX exchange member (e.g. YP, CC, AK) sits among the top ten buyers or sellers of the most traded stocks (default 100, or the given symbols) over a period ending at the latest session, from the IPOT broker summary. For each stock: the broker's rank on that side, lots, value, average price, share of the stock's value traded, and its value on the other side and the net (buy minus sell) when it is also top ten there. Only the top ten per side are published, so stocks where the broker traded below tenth place do not show. Totals of the visible buying and selling. One broker-summary request per scanned stock, shared with idx_bandar_screen (cached). Returns the idx.bandar.broker.v1 envelope."
    )]
    async fn bandar_broker(
        &self,
        Parameters(a): Parameters<BandarBrokerArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let period = parse_opt(&a.period, BandarPeriod::Week1)?;
                let symbols = match a
                    .symbols
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    None => Vec::new(),
                    Some(s) => Symbol::parse_list([s])?,
                };
                let opts = service::BrokerActivityOptions {
                    code: a.code,
                    period,
                    scan: a.scan.unwrap_or(service::BANDAR_SCAN_DEFAULT as u32) as usize,
                    symbols,
                    limit: a.limit.unwrap_or(20) as usize,
                };
                let mut list = universe::load(None)?;
                universe::extend_from_ownership(&mut list, &self.ownership_db);
                ok(
                    SCHEMA_BANDAR_BROKER,
                    service::bandar_broker(
                        self.provider.as_ref(),
                        &self.fetcher,
                        &list,
                        &opts,
                        service::today_wib(),
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_bandar_screen",
        description = "Bandarmology screener: scans the most traded IDX stocks by value (default 100, or the given symbols) over a period ending at the latest session and returns the ones whose top brokers accumulate or distribute, as defined by idx_bandar. Presets: big-accumulation (acc>=15), accumulation (acc>=5), distribution (acc<=-5), big-distribution (acc<=-15), quiet-accumulation (top brokers buying while the price is flat or down), quiet-distribution, foreign-accumulation (acc>=5 and foreign net >=5% of value), below-bandar (accumulation with the price under the top buyers' average). filter adds clauses on acc, net, lots, fnet, fpct, gain, chg, turnover, conc, price, signal, phase. Rows carry signal, phase, net_pct, net_value, price move, foreign flow, bandar average and gain, and the top buyer codes. One broker-summary request per scanned stock (cached). Heuristic, not advice. Returns the idx.bandar.screen.v1 envelope."
    )]
    async fn bandar_screen(
        &self,
        Parameters(a): Parameters<BandarScreenArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let preset = match a.preset.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    Some(p) => Some(p.parse::<crate::analysis::bandar::Preset>()?),
                    None => None,
                };
                let symbols = match a
                    .symbols
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    Some(s) => Symbol::parse_list([s])?,
                    None => Vec::new(),
                };
                let opts = service::BandarScreenOptions {
                    period: parse_opt(&a.period, BandarPeriod::Week1)?,
                    top: a.top.unwrap_or(crate::analysis::bandar::DEFAULT_TOP),
                    preset,
                    filters: crate::analysis::bandar::parse_filters(
                        a.filter.as_deref().unwrap_or(""),
                    )?,
                    sort: crate::analysis::bandar::parse_sort(a.sort.as_deref())?,
                    descending: a.ascending.map(|asc| !asc),
                    limit: a.limit.unwrap_or(25) as usize,
                    scan: a
                        .scan
                        .map(|n| n as usize)
                        .unwrap_or(service::BANDAR_SCAN_DEFAULT),
                    symbols,
                };
                let mut list = universe::load(None)?;
                universe::extend_from_ownership(&mut list, &self.ownership_db);
                ok(
                    SCHEMA_BANDAR_SCREEN,
                    service::bandar_screen(
                        self.provider.as_ref(),
                        &self.fetcher,
                        &list,
                        &opts,
                        service::today_wib(),
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_news",
        description = "Latest Indonesian finance headlines from Kontan and CNBC Indonesia RSS feeds. With a symbol, headlines that mention the company (scope=company, matched by ticker or name) come first, followed by general market headlines (scope=market) up to limit; company_matches says how many company items exist. Each item carries a lexicon sentiment score and the matched terms. Returns the idx.news.v1 envelope."
    )]
    async fn news(
        &self,
        Parameters(a): Parameters<NewsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let sym = a
                    .symbol
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(Symbol::parse)
                    .transpose()?;
                let list = universe::load(None).unwrap_or_default();
                let sources = a.sources.unwrap_or_default();
                ok(
                    SCHEMA_NEWS,
                    service::news(
                        &self.fetcher,
                        &list,
                        sym.as_ref(),
                        &sources,
                        a.limit.unwrap_or(20) as usize,
                        a.company_only.unwrap_or(false),
                    )
                    .await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_sentiment",
        description = "Aggregate lexicon sentiment (positive | negative | neutral | no_coverage, score -1..+1) over current headlines mentioning one IDX ticker, with counts, top terms and the strongest headlines. Indicative only. Returns the idx.sentiment.v1 envelope."
    )]
    async fn sentiment(
        &self,
        Parameters(a): Parameters<SentimentArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let sym = Symbol::parse(&a.symbol)?;
                let list = universe::load(None).unwrap_or_default();
                let sources = a.sources.unwrap_or_default();
                ok(
                    SCHEMA_SENTIMENT,
                    service::sentiment(&self.fetcher, &list, &sym, &sources, 10).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_ticker",
        description = "Shareholders above 1% of one IDX ticker from KSEI's monthly register (local SQLite): holder name, investor type, local/foreign, shares, percent, sorted by percent. Returns the idx.ownership.holders.v1 envelope. Needs a prior `idx ownership import`."
    )]
    async fn ownership_ticker(
        &self,
        Parameters(a): Parameters<OwnershipTickerArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                let as_of = parse_date(&a.as_of)?;
                let report = self.db()?.holders(symbol.as_str(), as_of)?;
                ok(SCHEMA_OWNERSHIP_HOLDERS, Sourced::fresh(report, "local-db"))
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_entity",
        description = "Every IDX position above 1% held by a person or institution whose name contains the query, from KSEI's monthly register (local SQLite). Spelling variants (PT/Tbk placement, punctuation) are grouped. Returns the idx.ownership.entity.v1 envelope."
    )]
    async fn ownership_entity(
        &self,
        Parameters(a): Parameters<OwnershipEntityArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                if a.name.trim().len() < 3 {
                    return Err(IdxError::Usage("name needs at least 3 characters".into()));
                }
                let as_of = parse_date(&a.as_of)?;
                let report = self.db()?.entity(&a.name, as_of)?;
                ok(SCHEMA_OWNERSHIP_ENTITY, Sourced::fresh(report, "local-db"))
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_changes",
        description = "Month-over-month changes in holders above 1% between two imported release dates: new holders, exits and position changes with percentage-point and share deltas, largest moves first. The summary block (counts, net percentage points for foreign vs local holders and per investor type, per-issuer flow, top rises and falls) always covers every match; limit/offset page the rows. Optional ticker filter. Returns the idx.ownership.changes.v1 envelope."
    )]
    async fn ownership_changes(
        &self,
        Parameters(a): Parameters<OwnershipChangesArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let from = parse_date(&Some(a.from.clone()))?.expect("checked");
                let to = parse_date(&Some(a.to.clone()))?.expect("checked");
                let symbol = a.symbol.as_deref().map(Symbol::parse).transpose()?;
                let limit = match a.limit {
                    Some(0) => None,
                    Some(n) => Some(n as usize),
                    None => Some(50),
                };
                let report = self.db()?.changes(
                    from,
                    to,
                    symbol.as_ref().map(Symbol::as_str),
                    a.min_pct_change.unwrap_or(0.0),
                    limit,
                    a.offset.unwrap_or(0) as usize,
                )?;
                ok(SCHEMA_OWNERSHIP_CHANGES, Sourced::fresh(report, "local-db"))
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_foreign",
        description = "Disclosed foreign (KSEI flag A) versus local (L) stake in the >1% register. With symbol: one issuer's foreign percent, holder counts and month-over-month change for every imported release (idx.ownership.foreign.v1). Without symbol: every issuer's change in foreign percent between two releases, largest moves first, with inflow/outflow counts (idx.ownership.foreign-flow.v1). Sums cover holders above 1% only. Needs a prior `idx ownership import`."
    )]
    async fn ownership_foreign(
        &self,
        Parameters(a): Parameters<OwnershipForeignArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let db = self.db()?;
                match a.symbol.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    Some(s) => {
                        let sym = Symbol::parse(s)?;
                        let report = db.foreign_trend(sym.as_str())?;
                        ok(SCHEMA_OWNERSHIP_FOREIGN, Sourced::fresh(report, "local-db"))
                    }
                    None => {
                        let limit = match a.limit {
                            Some(0) => None,
                            Some(n) => Some(n as usize),
                            None => Some(25),
                        };
                        let report =
                            db.foreign_flow(parse_date(&a.from)?, parse_date(&a.to)?, limit)?;
                        ok(
                            SCHEMA_OWNERSHIP_FOREIGN_FLOW,
                            Sourced::fresh(report, "local-db"),
                        )
                    }
                }
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_releases",
        description = "Lists the monthly KSEI register releases imported into the local database (as-of date, row counts, source). Use it to pick dates for idx_ownership_changes. Returns the idx.ownership.releases.v1 envelope."
    )]
    async fn ownership_releases(&self) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let list = self.db()?.releases()?;
                let mut s = Sourced::fresh(list, "local-db");
                if s.data.is_empty() {
                    s.warnings.push(
                        "no ownership data imported yet; run `idx ownership import --latest`"
                            .into(),
                    );
                }
                ok(SCHEMA_OWNERSHIP_RELEASES, s)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_orderbook",
        description = "Realtime order book for one IDX ticker from the user's Stockbit session: the full depth ladder (bids and asks, sizes in shares), last/open/high/low, session volume and value, and foreign buy/sell/net. Returns the idx.orderbook.v1 envelope. Needs a Stockbit login (idx_stockbit_status says whether one exists); errors with 'no Stockbit session' otherwise."
    )]
    async fn orderbook(
        &self,
        Parameters(a): Parameters<SymbolArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                ok(
                    SCHEMA_ORDERBOOK,
                    self.stockbit_provider().orderbook(&symbol).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_stockbit_status",
        description = "Whether a Stockbit session is stored and when its access and refresh tokens expire (never the token values). Call this before idx_orderbook or before using the stockbit provider. If logged_in is false, tell the user to run `idx stockbit login` in a terminal."
    )]
    async fn stockbit_status(&self) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let status = self.stockbit_store().status(chrono::Utc::now().timestamp());
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    json::to_pretty(&status)?,
                )]))
            }
            .await,
        )
    }

    #[tool(
        name = "idx_stockbit_login",
        description = "Opens a browser ON THE MACHINE RUNNING THIS SERVER for the user to sign in to Stockbit, then captures and stores the session. Only useful for a local (stdio) server the user is sitting at; it cannot work for a remote HTTP server. Blocks until the user signs in or it times out. Prefer telling the user to run `idx stockbit login` themselves; use this only when they ask you to start it."
    )]
    async fn stockbit_login(
        &self,
        Parameters(a): Parameters<StockbitLoginArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let profile_dir = self
                    .stockbit
                    .options
                    .session_path
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("stockbit-chrome-profile");
                let secs = a.timeout.unwrap_or(300).clamp(30, 900);
                let opts = login::LoginOptions {
                    browser: None,
                    profile_dir,
                    timeout: std::time::Duration::from_secs(secs),
                };
                let (session, _how) = login::capture(&opts).await?;
                self.stockbit_store().save(&session)?;
                let status = self.stockbit_store().status(chrono::Utc::now().timestamp());
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    json::to_pretty(&status)?,
                )]))
            }
            .await,
        )
    }
}

#[tool_handler]
impl ServerHandler for IdxServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("idx", env!("CARGO_PKG_VERSION")))
            .with_protocol_version(ProtocolVersion::LATEST)
            .with_instructions(INSTRUCTIONS.to_string())
    }

    // Hand-written so the advertised schemas are the portable ones; the
    // `tool_handler` macro only fills in methods that are missing.
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self.tools(),
            ..Default::default()
        })
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools().into_iter().find(|t| t.name == name)
    }
}

/// Runs the server on stdin/stdout until the client disconnects.
pub async fn serve(
    provider: Arc<dyn Provider>,
    fetcher: Arc<Fetcher>,
    ownership_db: PathBuf,
    stockbit: StockbitConfig,
) -> Result<()> {
    use rmcp::{transport::stdio, ServiceExt};
    let server = IdxServer::new(provider, fetcher, ownership_db, stockbit);
    tracing::info!(tools = ?server.tool_names(), "idx mcp server starting on stdio");
    let running = server
        .serve(stdio())
        .await
        .map_err(|e| IdxError::Other(format!("mcp: {e}")))?;
    running
        .waiting()
        .await
        .map_err(|e| IdxError::Other(format!("mcp: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Fundamentals, History, Quote};
    use async_trait::async_trait;

    struct Fake;

    #[async_trait]
    impl Provider for Fake {
        fn name(&self) -> &'static str {
            "fake"
        }
        async fn quotes(&self, s: &[Symbol]) -> Result<Sourced<Vec<Quote>>> {
            if s[0].as_str() == "ZZZZ" {
                return Err(IdxError::NotFound("ZZZZ".into()));
            }
            let mut q = Quote::empty(s[0].clone(), "fake");
            q.price = Some(5175.0);
            Ok(Sourced::fresh(vec![q], "fake"))
        }
        async fn history(&self, _: &Symbol, _: Period, _: Interval) -> Result<Sourced<History>> {
            Err(IdxError::OfflineMiss("nothing cached".into()))
        }
        async fn fundamentals(&self, s: &Symbol) -> Result<Sourced<Fundamentals>> {
            Ok(Sourced::fresh(
                Fundamentals::empty(s.clone(), "fake"),
                "fake",
            ))
        }
    }

    fn server() -> IdxServer {
        let dir = std::env::temp_dir().join(format!("idx-mcp-test-{}", std::process::id()));
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(crate::http::FixtureBackend::new()),
            crate::cache::Cache::new(dir.join("cache")),
            crate::http::CacheMode::Offline,
        ));
        IdxServer::new(
            Arc::new(Fake),
            fetcher,
            dir.join("ownership.sqlite"),
            StockbitConfig::default(),
        )
    }

    fn text(r: &CallToolResult) -> String {
        r.content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect()
    }

    #[test]
    fn every_tool_is_listed_with_a_schema() {
        let s = server();
        let mut names = s.tool_names();
        names.sort();
        assert_eq!(
            names,
            [
                "idx_bandar",
                "idx_bandar_broker",
                "idx_bandar_screen",
                "idx_brief",
                "idx_broker",
                "idx_calendar",
                "idx_compare",
                "idx_corporate_actions",
                "idx_dividends",
                "idx_earnings",
                "idx_financials",
                "idx_foreign",
                "idx_fundamental",
                "idx_history",
                "idx_holidays",
                "idx_indices",
                "idx_instruments",
                "idx_market",
                "idx_market_brokers",
                "idx_market_foreign",
                "idx_members",
                "idx_news",
                "idx_orderbook",
                "idx_ownership_changes",
                "idx_ownership_entity",
                "idx_ownership_foreign",
                "idx_ownership_releases",
                "idx_ownership_ticker",
                "idx_profile",
                "idx_quote",
                "idx_returns",
                "idx_screen",
                "idx_sentiment",
                "idx_stockbit_login",
                "idx_stockbit_status",
                "idx_technical",
            ]
        );
        for t in s.tools() {
            let d = t.description.as_deref().unwrap_or("");
            assert!(d.len() > 60, "{} needs a real description", t.name);
            assert!(
                t.input_schema.get("properties").is_some(),
                "{} has no typed input",
                t.name
            );
            // Structural check: no null values, no default/format/$schema keys,
            // no type arrays anywhere in the schema (descriptions may say "null").
            fn walk(name: &str, v: &serde_json::Value) {
                match v {
                    serde_json::Value::Null => panic!("{name}: null value in schema"),
                    serde_json::Value::Object(m) => {
                        for (k, v) in m {
                            assert!(
                                !matches!(k.as_str(), "default" | "format" | "$schema"),
                                "{name}: {k} keyword breaks Gemini/OpenAI harnesses"
                            );
                            if k == "type" {
                                assert!(!v.is_array(), "{name}: type array");
                            }
                            walk(name, v);
                        }
                    }
                    serde_json::Value::Array(a) => a.iter().for_each(|v| walk(name, v)),
                    _ => {}
                }
            }
            walk(
                &t.name,
                &serde_json::Value::Object((*t.input_schema).clone()),
            );
        }
    }

    #[tokio::test]
    async fn quote_tool_returns_the_cli_envelope() {
        let s = server();
        let r = s
            .quote(Parameters(SymbolsArg {
                symbols: "bbca".into(),
            }))
            .await
            .unwrap();
        assert_ne!(r.is_error, Some(true));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["schema"], "idx.quotes.v1");
        assert_eq!(v["data"][0]["symbol"], "BBCA");
        assert_eq!(v["data"][0]["price"], 5175.0);
    }

    #[tokio::test]
    async fn failures_are_tool_errors_with_the_cli_error_shape() {
        let s = server();
        let r = s
            .quote(Parameters(SymbolsArg {
                symbols: "ZZZZ".into(),
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "not_found");
        assert_eq!(v["exit_code"], 4);

        let r = s
            .history(Parameters(HistoryArgs {
                symbol: "BBCA".into(),
                period: Some("7d".into()),
                interval: None,
                limit: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        assert!(text(&r).contains("usage"));

        let r = s
            .history(Parameters(HistoryArgs {
                symbol: "BBCA".into(),
                period: None,
                interval: None,
                limit: None,
            }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "offline_miss");

        let broker = |date: Option<&str>, from: Option<&str>, investor: Option<&str>| BrokerArgs {
            symbols: "BBCA".into(),
            date: date.map(Into::into),
            from: from.map(Into::into),
            to: None,
            investor: investor.map(Into::into),
            board: None,
        };
        for bad in [
            broker(Some("2026-10-16"), Some("2026-10-01"), None),
            broker(Some("16/10/2026"), None, None),
            broker(None, None, Some("retail")),
        ] {
            let r = s.broker(Parameters(bad)).await.unwrap();
            assert_eq!(r.is_error, Some(true));
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], "usage", "{v}");
        }
        let r = s
            .broker(Parameters(broker(
                Some("2026-10-16"),
                None,
                Some("foreign"),
            )))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "offline_miss");
    }

    #[tokio::test]
    async fn bandar_tools_check_their_arguments_before_fetching() {
        let s = server();
        let detect = |period: Option<&str>,
                      top: Option<u32>,
                      date: Option<&str>,
                      from: Option<&str>| BandarArgs {
            symbols: "BBCA".into(),
            period: period.map(Into::into),
            date: date.map(Into::into),
            from: from.map(Into::into),
            to: None,
            top,
            investor: None,
            board: None,
            daily: None,
        };
        for bad in [
            detect(Some("2w"), None, None, None),
            detect(None, Some(4), None, None),
            detect(None, None, Some("2026-10-16"), Some("2026-10-01")),
        ] {
            let r = s.bandar(Parameters(bad)).await.unwrap();
            assert_eq!(r.is_error, Some(true));
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], "usage", "{v}");
        }
        let r = s
            .bandar(Parameters(detect(Some("1m"), Some(3), None, None)))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "offline_miss", "{v}");

        let screen =
            |preset: Option<&str>, filter: Option<&str>, sort: Option<&str>| BandarScreenArgs {
                preset: preset.map(Into::into),
                filter: filter.map(Into::into),
                period: None,
                top: None,
                sort: sort.map(Into::into),
                ascending: None,
                limit: None,
                scan: None,
                symbols: Some("BBCA".into()),
            };
        for bad in [
            screen(Some("bandar-everything"), None, None),
            screen(None, Some("pe<10"), None),
            screen(None, None, Some("signal")),
        ] {
            let r = s.bandar_screen(Parameters(bad)).await.unwrap();
            assert_eq!(r.is_error, Some(true));
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], "usage", "{v}");
        }
    }

    #[tokio::test]
    async fn fundamental_tool_adds_a_verdict() {
        let s = server();
        let r = s
            .fundamental(Parameters(YahooSymbolArg {
                symbol: "BBCA".into(),
            }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["schema"], "idx.fundamentals.v1");
        assert_eq!(v["data"]["verdict"]["label"], "insufficient_data");
    }
}
