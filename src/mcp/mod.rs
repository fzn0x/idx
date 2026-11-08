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
    AstroPeriod, BandarPeriod, BrokerBoard, BrokerInvestor, ConcentrationSort, CorporateEventKind,
    EarningsFilter, ForeignSource, HolderOrigin, Interval, Lens, MovementFilter, MovementSource,
    NatalChoice, Period, StatementPeriod, Symbol, IDX_EXCHANGE,
};
use crate::output::json::{
    self, Envelope, ErrorEnvelope, SCHEMA_BRIEF, SCHEMA_COMPARE, SCHEMA_DIVIDENDS, SCHEMA_EARNINGS,
    SCHEMA_FINANCIALS, SCHEMA_FUNDAMENTALS, SCHEMA_GROWTH, SCHEMA_HISTORY, SCHEMA_INTRADAY,
    SCHEMA_MARKET, SCHEMA_OWNERSHIP_CHANGES, SCHEMA_OWNERSHIP_CONCENTRATION,
    SCHEMA_OWNERSHIP_CROSS_HOLDERS, SCHEMA_OWNERSHIP_ENTITY, SCHEMA_OWNERSHIP_FOREIGN,
    SCHEMA_OWNERSHIP_FOREIGN_FLOW, SCHEMA_OWNERSHIP_GRAPH, SCHEMA_OWNERSHIP_HOLDERS,
    SCHEMA_OWNERSHIP_INSTITUTIONS, SCHEMA_OWNERSHIP_RELEASES, SCHEMA_OWNERSHIP_SEARCH,
    SCHEMA_OWNERSHIP_SNAPSHOT_SYNC, SCHEMA_OWNERSHIP_STATUS, SCHEMA_PROFILE, SCHEMA_QUOTES,
    SCHEMA_RETURNS, SCHEMA_RISK, SCHEMA_TECHNICAL, SCHEMA_VALUATION,
};
use crate::output::json::{
    SCHEMA_ACTIONS, SCHEMA_CALENDAR, SCHEMA_FOREIGN, SCHEMA_FOREIGN_VALUE, SCHEMA_INDICES,
    SCHEMA_INSTRUMENTS, SCHEMA_MARKET_BROKERS, SCHEMA_MARKET_FOREIGN, SCHEMA_MEMBERS,
};
use crate::output::json::{
    SCHEMA_ASTRO, SCHEMA_BANDAR, SCHEMA_BANDAR_BROKER, SCHEMA_BANDAR_SCREEN, SCHEMA_BROKER,
    SCHEMA_HOLIDAYS, SCHEMA_INSIGHTS, SCHEMA_NEWS, SCHEMA_ORDERBOOK, SCHEMA_OWNERSHIP_COMPOSITION,
    SCHEMA_SCREEN, SCHEMA_SENTIMENT, SCHEMA_STOCKBIT_ACTIONS, SCHEMA_STOCKBIT_MOVEMENTS,
};
use crate::ownership::{self, snapshot, OwnershipDb};
use crate::providers::stockbit::auth::{SignInProgress, Status};
use crate::providers::stockbit::movement::MovementQuery;
use crate::providers::stockbit::{login, page, SessionStore, Stockbit};
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
{schema, provider, fetched_at, cached, stale, stale_reason?, warnings?, data}. Fields the
source did not return are null; nothing is estimated. Prices are delayed as published.
stale=true means a cached copy older than its TTL was served: stale_reason is offline
(the server runs with --offline) or fetch_failed (the source failed or refused the
request; a warning names the failure).
Errors return isError=true with {error:{code,message}, exit_code}; code not_found
means the ticker is unknown, offline_miss means --offline with nothing cached, blocked
means the source's bot protection (Cloudflare, idx.co.id) refused the request; for
idx.co.id's data a challenged request is already retried for 30 s and a cached copy,
when there is one, is served instead. It usually passes within a minute: call again
then. If it keeps refusing, the server's network is likely refused (data centres, VPNs
and CI runners are refused more often): tell the user. Or take the alternative the
message names; for the ownership register, the GitHub mirror.
idx_broker shows which brokers bought and sold a ticker (IPOT broker summary);
idx_bandar reads it as bandarmology (are the top brokers accumulating or distributing?)
and idx_bandar_screen finds the most traded stocks they accumulate or distribute.
Ownership tools read the local SQLite register of holders above 1% (KSEI via IDX,
monthly); if it is empty they say so: call idx_ownership_snapshot with action \"sync\"
(or the user runs `idx ownership sync`).
idx_ownership_search finds a holder's entity_key by name; idx_ownership_rankings ranks
holders by issuers held or issuers by concentration; idx_ownership_graph walks the network.
idx_ownership_composition reads KSEI's monthly composition archive (local vs foreign by
investor type, every holder, none named) and downloads the newest month on first use.
idx_ownership_institutions is the exception: it asks MSN live and needs no import.
Stockbit tools (idx_orderbook, idx_stockbit_*) use the user's own Stockbit login: check
idx_stockbit_status first, and never ask for passwords, cookies or tokens in the chat.
idx_astro is traditional astrology computed offline; never present it as a forecast;
its backtest says whether IHSG's history supports it.
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
pub struct FundamentalArgs {
    /// One ticker, e.g. "BBCA". A bare code is IDX; without `lens`, another Yahoo Finance market's ticker takes its suffix, e.g. "D05.SI" (the lenses are IDX only).
    pub symbol: String,
    /// Optional focused reading instead of the full report: "growth" (idx.growth.v1), "valuation"
    /// (idx.valuation.v1) or "risk" (idx.risk.v1; adds a year of daily candles for the stock and IHSG).
    #[serde(default)]
    pub lens: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StockbitLoginArgs {
    /// Seconds to wait for the sign-in (default 300, clamped to 30..900).
    #[serde(default)]
    pub timeout: Option<u64>,
    /// "page": return at once with a URL on 127.0.0.1 (and an ssh -L command when this server is reached over SSH) for the user's own browser. "browser": open a browser on this server's machine and wait. Default "auto": browser where this machine has a display and a Chromium-family browser and is not reached over SSH, else page.
    #[serde(default)]
    pub method: Option<String>,
    /// Port for the page (default 7878, or a free one when it is taken; 0 for any).
    #[serde(default)]
    pub port: Option<u32>,
    /// Sign in even when the stored session still works.
    #[serde(default)]
    pub force: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StockbitActionsArgs {
    /// One IDX ticker, e.g. "BBCA": that stock's whole history, newest first. Omit it for the market calendar: every stock's actions in a date range, oldest first.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Kinds, comma separated: dividend, stock_dividend, bonus, rights, warrant, split, reverse_split, rups, public_expose, ipo, tender_offer, or "all". Default: every kind but ipo for a stock, every kind but warrant for the calendar. Each kind is one request to Stockbit.
    #[serde(default)]
    pub kind: Option<String>,
    /// First day, YYYY-MM-DD. A stock: only actions dated from it. The calendar: default today.
    #[serde(default)]
    pub from: Option<String>,
    /// Last day, YYYY-MM-DD. A stock: only actions dated up to it. The calendar: default from plus days.
    #[serde(default)]
    pub to: Option<String>,
    /// Calendar only: days after from when to is not given (default 14; the range covers at most 366 days).
    #[serde(default)]
    pub days: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StockbitMovementsArgs {
    /// One IDX ticker, e.g. "BBCA"; omit it for the whole market.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Only changes from this day, YYYY-MM-DD.
    #[serde(default)]
    pub from: Option<String>,
    /// Only changes up to this day, YYYY-MM-DD.
    #[serde(default)]
    pub to: Option<String>,
    /// One holder: the holder_id of a row (digits).
    #[serde(default)]
    pub holder: Option<String>,
    /// buy, sell, transfer, cross, or corporate_action (rights issues, private placements, tender offers and the like).
    #[serde(default)]
    pub action: Option<String>,
    /// idx (insider disclosures, any size, with the trade price) or ksei (holders at 5% and above, no price filed).
    #[serde(default)]
    pub source: Option<String>,
    /// Pages of 50 rows to read, 1 to 10 (default 1); each page is one request to Stockbit.
    #[serde(default)]
    pub pages: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HistoryArgs {
    /// One ticker, e.g. "TLKM". A bare code is IDX; another Yahoo Finance market's ticker takes its suffix, e.g. "D05.SI".
    pub symbol: String,
    /// Look-back window: 1d, 5d, 1mo, 3mo, 6mo, 1y (default), 2y, 5y, 10y, ytd, max.
    #[serde(default)]
    pub period: Option<String>,
    /// Candle size: 1m, 2m, 5m, 15m, 30m, 1h, 1d (default), 1wk, 1mo. 1m needs period 1d or 5d, 2m at most 1mo; the other intraday sizes need short periods.
    #[serde(default)]
    pub interval: Option<String>,
    /// Keep only the most recent N candles (default 60). 0 returns all of them.
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IntradayArgs {
    /// One IDX ticker or index, e.g. "BBCA" or "^JKSE".
    pub symbol: String,
    /// Bar size: 1m (default), 2m, 5m, 15m, 30m or 1h.
    #[serde(default)]
    pub interval: Option<String>,
    /// Keep only the most recent N bars (default 120). 0 returns all of them. The summary covers the whole session.
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
pub struct EarningsArgs {
    /// One ticker, e.g. "TLKM". A bare code is IDX; another Yahoo Finance market's ticker takes its suffix, e.g. "D05.SI".
    pub symbol: String,
    /// Keep only annual (fiscal-year) or quarterly rows. Leave it out (or "all") for both.
    #[serde(default)]
    pub period: Option<String>,
    /// Keep only reported figures or analyst estimates (consensus and the next date). Leave it out (or "all") for both.
    #[serde(default)]
    pub kind: Option<String>,
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
pub struct OwnershipCompositionArgs {
    /// One IDX ticker, e.g. "BBCA".
    pub symbol: String,
    /// Month YYYY-MM-DD, the archive's date (a month's last trading day, e.g. 2026-10-30); default: the newest. A listed month that is not imported yet is downloaded.
    #[serde(default)]
    pub as_of: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OwnershipEntityArgs {
    /// Part of a holder's name (person or institution), at least 3 letters or digits, e.g. "Dwimuria".
    pub name: String,
    /// Release date YYYY-MM-DD (default: newest imported).
    #[serde(default)]
    pub as_of: Option<String>,
    /// Match the holder's key exactly, e.g. an entity_key from idx_ownership_search (default false: substring).
    #[serde(default)]
    pub exact: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OwnershipSearchArgs {
    /// Part of a holder's name (person or institution), at least 2 letters or digits; words in any order, e.g. "salim" or "lo kheng hong".
    pub query: String,
    /// Matches returned (default 20; 0 = all, at most 100; total_matches counts every match).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Release date YYYY-MM-DD in which issuers are counted (default: newest imported).
    #[serde(default)]
    pub as_of: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OwnershipRankingsArgs {
    /// "holders" (default): holders ranked by the number of issuers they own more than 1% of. "issuers": issuers ranked by ownership concentration.
    #[serde(default)]
    pub view: Option<String>,
    /// view holders: minimum number of issuers held above 1% (default 2).
    #[serde(default)]
    pub min_issuers: Option<u32>,
    /// view holders: KSEI investor type, case-insensitive substring, e.g. "individual", "mutual fund", "pension".
    #[serde(default)]
    pub investor_type: Option<String>,
    /// view holders: "foreign" (KSEI flag A) or "local" (L); default both.
    #[serde(default)]
    pub origin: Option<String>,
    /// view issuers: IDX tickers, comma or space separated; omit to rank every issuer.
    #[serde(default)]
    pub symbols: Option<String>,
    /// view issuers: "hhi" (default), "top1" or "top3".
    #[serde(default)]
    pub by: Option<String>,
    /// view issuers: least concentrated (most widely held) first (default false).
    #[serde(default)]
    pub ascending: Option<bool>,
    /// Rows returned (default 25; 0 = all). Ignored with symbols.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Release date YYYY-MM-DD (default: newest imported).
    #[serde(default)]
    pub as_of: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OwnershipGraphArgs {
    /// Ticker (e.g. "BBCA") or a holder's name or entity_key (e.g. "Anthoni Salim") to start from.
    pub root: String,
    /// Hops to walk, 1-6 (default 2): from a ticker, 1 = its holders, 2 = their other holdings.
    #[serde(default)]
    pub depth: Option<u32>,
    /// Ignore holdings below this percent (default 0; 5 keeps major stakes only).
    #[serde(default)]
    pub min_pct: Option<f64>,
    /// Stop after this many nodes (default 100, at most 1000); truncated=true when hit.
    #[serde(default)]
    pub max_nodes: Option<u32>,
    /// Treat root as a holder name even when it is also a ticker (default false).
    #[serde(default)]
    pub holder: Option<bool>,
    /// Release date YYYY-MM-DD (default: newest imported).
    #[serde(default)]
    pub as_of: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct OwnershipSnapshotArgs {
    /// "status" (read-only: the local database, the mirror and what sync would do) or "sync" (install the prebuilt snapshot, or import the missing months).
    pub action: String,
    /// With action "sync": replace a local database that holds months or data the snapshot lacks (a .bak copy is kept). Default false.
    #[serde(default)]
    pub force: Option<bool>,
}

/// Network timeout of idx_ownership_snapshot (the server has no settings).
const SNAPSHOT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

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
    /// Ready-made screen: top-performers (sort ret1y desc), worst-performers (ret1y asc), high-dividend (div>0, div desc), low-pe (pe>0, pe asc), 52w-high (range>=95, range desc), 52w-low (range<=5, range asc), high-volume (volume desc) or large-cap (mcap desc).
    #[serde(default)]
    pub preset: Option<String>,
    /// Comma-separated clauses such as "pe<15,roe>15,sector=financial,mcap>10T", combined with the preset. Keys: pe fpe pb ps peg mcap price div eps volume chg range (0 at the 52-week low, 100 at the high) ret1w ret1m ret3m ret6m retytd ret1y (price return %, as MSN reports it) roe roa margin opmargin de growth epsgrowth sector industry. Operators < <= > >= = !=; numbers accept K/M/B/T and %.
    #[serde(default)]
    pub filter: Option<String>,
    /// Sort key (any numeric filter key). Default: the preset's, else mcap.
    #[serde(default)]
    pub sort: Option<String>,
    /// Sort ascending. Default: descending, except for the low-pe, worst-performers and 52w-low presets.
    #[serde(default)]
    pub ascending: Option<bool>,
    /// Rows returned (default 25, 0 = all).
    #[serde(default)]
    pub limit: Option<u32>,
    /// Cap on per-company fundamentals fetches when a filter needs them (default 50).
    #[serde(default)]
    pub max_fundamentals: Option<u32>,
    /// Join the return windows (return_1w_pct..return_1y_pct) even when no filter or the sort needs them (one extra request).
    #[serde(default)]
    pub returns: Option<bool>,
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
    /// Latest sessions to return (default 20; source idx at most 250, ipot at most 60). Not with from/to.
    #[serde(default)]
    pub days: Option<u32>,
    /// First session of a date range, YYYY-MM-DD.
    #[serde(default)]
    pub from: Option<String>,
    /// Last session of the range, YYYY-MM-DD. Default: today; not in the future.
    #[serde(default)]
    pub to: Option<String>,
    /// "idx" (default): idx.co.id, shares on the regular board, returns idx.foreign.v1. "ipot": Indo Premier, rupiah across all boards, returns idx.foreign.value.v1.
    #[serde(default)]
    pub source: Option<String>,
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
    /// "auto" (default): idx.co.id, and IPOT when idx.co.id refuses, cannot be reached, fails or has not published the session, if IPOT holds it. "idx": idx.co.id only, any session. "ipot": Indo Premier, its latest session only, with each broker's value bought, sold and net and its class.
    #[serde(default)]
    pub source: Option<String>,
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
pub struct AstroArgs {
    /// Date YYYY-MM-DD, 1800-01-01 to 2050-12-31 (default today in Jakarta). Positions are for 16:00 WIB, the IDX close.
    #[serde(default)]
    pub date: Option<String>,
    /// Days of lunar phases, stations and sign ingresses to list from the date (default 30, at most 366).
    #[serde(default)]
    pub days: Option<u32>,
    /// IHSG chart for the transits: "launch" (1983-04-01, default), "base" (1982-08-10, value 100) or "none".
    #[serde(default)]
    pub natal: Option<String>,
    /// Add the backtest of IHSG daily returns by lunar phase, Mercury retrograde, pasaran and Sun sign (default false; fetches ^JKSE history).
    #[serde(default)]
    pub backtest: Option<bool>,
    /// Backtest look-back, only with backtest=true: 1y, 2y, 5y, 10y or max (default max, since 1990).
    #[serde(default)]
    pub period: Option<String>,
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
    /// The process's session store for `options`, taken once and shared by
    /// every clone (the server of each MCP HTTP session), so its pacer and
    /// the refresh token it saw refused last as long as the server, not one
    /// tool call. Filled by [`StockbitConfig::resolved`].
    store: Option<Arc<SessionStore>>,
}

impl StockbitConfig {
    pub fn new(options: StockbitOptions, ttls: Ttls) -> Self {
        StockbitConfig {
            options,
            ttls,
            store: None,
        }
        .resolved()
    }

    /// The same config holding the process's store for its options.
    pub fn resolved(mut self) -> Self {
        if self.store.is_none() {
            self.store = Some(self.options.store());
        }
        self
    }
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

/// Advice that messages word for the CLI, naming flags an agent cannot pass;
/// every error and warning a tool returns carries the MCP wording instead.
const MCP_ADVICE: &[crate::error::Advice] = &[
    crate::providers::idx::NO_CACHE,
    service::IPOT_SOURCE_ADVICE,
    service::BANDAR_FNET_ADVICE,
    service::BROKERS_IPOT_ADVICE,
];

/// `text` with its CLI advice in MCP wording.
fn for_mcp(text: &str) -> String {
    MCP_ADVICE
        .iter()
        .fold(text.to_string(), |t, a| t.replace(a.cli, a.mcp))
}

fn ok<T: serde::Serialize>(schema: &str, s: Sourced<T>) -> Result<CallToolResult> {
    let mut env = Envelope::from_sourced(schema, s);
    for w in &mut env.warnings {
        *w = for_mcp(w);
    }
    Ok(CallToolResult::success(vec![ContentBlock::text(
        json::to_pretty(&env)?,
    )]))
}

/// Tool-level failure: the agent sees the same error object the CLI prints,
/// its advice worded for a tool call.
fn fail(e: IdxError) -> CallToolResult {
    let mut env = ErrorEnvelope::from(&e);
    env.error.message = for_mcp(&env.error.message);
    let text = serde_json::to_string_pretty(&env).unwrap_or_else(|_| for_mcp(&e.to_string()));
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
            stockbit: stockbit.resolved(),
            tool_router: Self::tool_router(),
        }
    }

    fn db(&self) -> Result<OwnershipDb> {
        OwnershipDb::open(&self.ownership_db)
    }

    /// The store [`IdxServer::new`] took once: every tool call paces and
    /// refreshes on it.
    fn stockbit_store(&self) -> Arc<SessionStore> {
        match &self.stockbit.store {
            Some(s) => s.clone(),
            None => self.stockbit.options.store(),
        }
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
        description = "Delayed quotes for one or more IDX tickers: price, change, previous close, OHLC, volume, market cap, P/E, dividend yield, 52-week range and market_time (last trade, WIB). behind_session is true when that last trade is dated before the latest IDX session that has ended (IDX holidays are not counted): the stock is suspended or did not trade, or the source lags the close; null when the time is unknown or the symbol does not trade on IDX. Returns the idx.quotes.v1 envelope; unknown tickers are listed in warnings."
    )]
    async fn quote(
        &self,
        Parameters(a): Parameters<SymbolsArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbols = Symbol::parse_list_on([a.symbols.as_str()], IDX_EXCHANGE)?;
                ok(
                    SCHEMA_QUOTES,
                    service::quotes(self.provider.as_ref(), &symbols).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_history",
        description = "OHLCV candles for one IDX ticker, oldest first, with a summary (first/last close, change %, high, low, average volume). Returns the idx.history.v1 envelope. Use limit to bound the payload; the summary always covers the whole period. The envelope's provider names the source the candles came from (the server's history_provider setting)."
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
                interval.check_period(period)?;
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
        name = "idx_intraday",
        description = "The latest session of one IDX stock or index by the minute (Yahoo's chart, no login, about 10 minutes behind the market): OHLCV bars oldest first, the quote (previous close, last price and change, the day's open, high, low and volume, market state, last trade time) and a session summary (first and last bar, open, high, low, close, the bars' volume, VWAP computed by idx from the bars' typical prices). Today's session while the market trades, otherwise the last one (also in the first minutes after the open, before Yahoo has bars for the day; a warning says so). Minutes without a trade have no bar, and the opening auction's volume is in no bar. With the server on --provider stockbit the bars are realtime from the user's session. Returns the idx.intraday.v1 envelope."
    )]
    async fn intraday(
        &self,
        Parameters(a): Parameters<IntradayArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                let interval = parse_opt(&a.interval, Interval::Min1)?;
                let list = universe::load(None).unwrap_or_default();
                let mut r =
                    service::intraday(self.provider.as_ref(), &list, &symbol, interval).await?;
                let limit = a.limit.unwrap_or(120) as usize;
                if limit > 0 && r.data.bars.len() > limit {
                    let skip = r.data.bars.len() - limit;
                    r.data.bars.drain(..skip);
                    r.warnings.push(format!(
                        "bars truncated to the last {limit}; the summary covers the whole session"
                    ));
                }
                ok(SCHEMA_INTRADAY, r)
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
        description = "Income statement, balance sheet and cash flow for one IDX ticker, oldest period first, amounts in reporting currency (IDR). Returns the idx.financials.v1 envelope. Every period carries all three statements (the CLI's --statement only narrows its table). Banks have no EBITDA/gross profit series (null)."
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
        description = "Reported quarterly EPS versus analyst estimates with surprise %, fiscal-year and quarterly revenue and earnings, analyst consensus for this and next quarter and fiscal year, the next earnings date, and dividend dates for one IDX ticker. period (annual | quarterly) and kind (reported | estimates) keep only those rows; data.scope says what was kept. Returns the idx.earnings.v1 envelope."
    )]
    async fn earnings(
        &self,
        Parameters(a): Parameters<EarningsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse_on(&a.symbol, IDX_EXCHANGE)?;
                let filter = EarningsFilter::from_names(a.period.as_deref(), a.kind.as_deref())?;
                ok(
                    SCHEMA_EARNINGS,
                    service::earnings(self.provider.as_ref(), &symbol, filter).await?,
                )
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
        description = "Foreign investors' buying and selling of one IDX stock over its latest sessions (default 20, at most 250) or a from/to date range, oldest first. Default source idx: idx.co.id's daily stock summary with close, change, volume, value, foreign buy and sell (shares), net shares and net shares times the close (a rupiah scale, not the traded value; IDX publishes foreign trading in shares only); returns the idx.foreign.v1 envelope. With source=ipot the flow comes from Indo Premier's broker summary instead: foreign buy, sell and net value in rupiah across all boards, the value traded, average price and foreign investors' share of the trading, two requests per session (cached), at most 60 sessions, figures of a million and more rounded by the source (rounded=true); returns the idx.foreign.value.v1 envelope, and works where idx.co.id refuses the connection. Totals give the sums, how many sessions were net buying or net selling and the current streak (+3 = three net-buy sessions in a row)."
    )]
    async fn foreign(
        &self,
        Parameters(a): Parameters<ForeignArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                let source = parse_opt(&a.source, ForeignSource::Idx)?;
                let list = universe::load(None).unwrap_or_default();
                let today = service::today_wib();
                let range = service::ForeignRange::from_args(
                    a.days.map(|d| d as usize),
                    parse_date(&a.from)?,
                    parse_date(&a.to)?,
                    today,
                )?;
                match source {
                    ForeignSource::Idx => ok(
                        SCHEMA_FOREIGN,
                        service::foreign_flow(&self.fetcher, &list, &symbol, range, today).await?,
                    ),
                    ForeignSource::Ipot => ok(
                        SCHEMA_FOREIGN_VALUE,
                        service::foreign_flow_ipot(&self.fetcher, &list, &symbol, range, today)
                            .await?,
                    ),
                }
            }
            .await,
        )
    }

    #[tool(
        name = "idx_market_brokers",
        description = "Top brokers: every IDX exchange member's trading for one session (default: the latest published), ranked by value (default), volume or frequency, each with code, member name, value (IDR), volume (shares), frequency and its share of all members' value. A member's buying and selling count together, so all members' value is twice the market's. By default (source auto) the ranking comes from idx.co.id's daily broker summary, and when idx.co.id refuses (Cloudflare), cannot be reached, fails or has not published the session yet, from IPOT's ranking if IPOT holds that session (the date, or with none the latest session that has ended), with a warning; the envelope's provider says which answered. source=idx asks idx.co.id only, any session; when it refuses, the error names source=ipot if IPOT could answer (no date or the latest session that has ended, outside a trading day's 08:58 to 19:00 WIB). A session idx.co.id has not published yet is named: with no date the session before comes with a warning, and that date is not_found. source=ipot asks IPOT directly: its latest session only (a date before that session, or after today, is a usage error; a later one up to today, or a day without a session, is not_found), with each broker's value bought, sold and net and its class (local, foreign, state_owned), figures of a million and more abbreviated to two decimals (rounded=true). From idx.co.id buy_value, sell_value, net_value and kind are null and rounded is false. IPOT's ranking read on a trading day from 08:58 to 19:00 WIB cannot be dated and is refused (a provider error, or offline_miss or network for a copy read then and served offline or after a failed read; under auto, idx.co.id's error then stands). For which stocks one broker traded use idx_bandar_broker; for one stock's brokers idx_broker. Returns the idx.market.brokers.v1 envelope."
    )]
    async fn market_brokers(
        &self,
        Parameters(a): Parameters<MarketBrokersArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                ok(
                    SCHEMA_MARKET_BROKERS,
                    service::market_brokers_from(
                        &self.fetcher,
                        parse_opt(&a.source, crate::model::BrokerSource::Auto)?,
                        parse_date(&a.date)?,
                        a.top.unwrap_or(20) as usize,
                        parse_opt(&a.sort, crate::model::BrokerSort::Value)?,
                        chrono::Utc::now(),
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
        name = "idx_astro",
        description = "Astrology for IHSG, computed offline: apparent positions of the Sun, Moon and planets (sign, degree, speed, retrograde) at 16:00 WIB on a date; the Moon's phase, illumination and next new and full moon; Mercury's retrograde period; the Javanese weton (day, pasaran, neptu) and Chinese shio; lunar phases, stations and sign ingresses ahead; aspects to IHSG's own chart (launched 1983-04-01, a date from secondary sources). backtest=true adds IHSG's daily returns since 1990 by lunar phase, Mercury retrograde, pasaran and Sun sign, each tested against all other days (Welch t, Bonferroni). Traditional astrology, not a forecast: present the backtest when asked whether it works. Returns the idx.astro.v1 envelope."
    )]
    async fn astro(
        &self,
        Parameters(a): Parameters<AstroArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                // As on the CLI (--period requires --backtest) and the web.
                let backtest = a.backtest.unwrap_or(false);
                if !backtest && a.period.as_deref().is_some_and(|s| !s.trim().is_empty()) {
                    return Err(IdxError::Usage(
                        "period is the backtest's look-back; pass it with backtest=true".into(),
                    ));
                }
                let period = parse_opt(&a.period, AstroPeriod::Max)?;
                let q = service::AstroQuery {
                    date: parse_date(&a.date)?.unwrap_or_else(service::today_wib),
                    days: a.days.unwrap_or(service::ASTRO_DEFAULT_DAYS),
                    natal: parse_opt(&a.natal, NatalChoice::Launch)?,
                    backtest: backtest.then_some(period),
                };
                ok(
                    SCHEMA_ASTRO,
                    service::astro(self.provider.as_ref(), &q).await?,
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
        description = "Fundamentals report for one IDX ticker: valuation (P/E, forward P/E, PEG, P/B, P/S, EV/EBITDA), profitability (ROE, ROA, margins), balance sheet (debt/equity, current ratio, cash, debt), growth, dividend, per-share figures, analyst targets, and a plain-language verdict (undervalued | fair | overvalued | insufficient_data) with every rule that fired listed in reasons. When financial_currency differs from currency (USD statements, IDR price), P/B, P/S and the EV multiples divide the IDR price by USD figures and are not comparable, book value, revenue per share, total debt and total cash are in financial_currency, and the verdict does not score P/B. Returns the idx.fundamentals.v1 envelope. With lens, a focused reading instead: growth (latest quarter, forward EPS, fiscal-year revenue and net income; growing | stable | mixed | shrinking; idx.growth.v1), valuation (multiples, yield, analyst target; cheap | fair | expensive; idx.valuation.v1) or risk (negative equity, leverage, liquidity, losses, and beta, volatility and largest fall against IHSG over a year of daily closes; low | moderate | high; idx.risk.v1). A lens lists every rule that ran with its points and notes every rule it skipped."
    )]
    async fn fundamental(
        &self,
        Parameters(a): Parameters<FundamentalArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let lens = match a.lens.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    Some(l) => Some(l.parse::<Lens>()?),
                    None => None,
                };
                let Some(lens) = lens else {
                    // The full report takes another market's ticker too; the
                    // lenses, like `stocks growth|valuation|risk`, IDX only.
                    let symbol = Symbol::parse_on(&a.symbol, IDX_EXCHANGE)?;
                    return ok(
                        SCHEMA_FUNDAMENTALS,
                        service::fundamentals(self.provider.as_ref(), &symbol).await?,
                    );
                };
                let symbol = Symbol::parse(&a.symbol)?;
                match service::lens(self.provider.as_ref(), &symbol, lens).await? {
                    service::LensResult::Growth(r) => ok(SCHEMA_GROWTH, r),
                    service::LensResult::Valuation(r) => ok(SCHEMA_VALUATION, r),
                    service::LensResult::Risk(r) => ok(SCHEMA_RISK, r),
                }
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
        description = "Screens the IDX universe (~950 tickers) with a filter string such as \"pe<15,roe>15,sector=financial,mcap>10T\" and/or a preset (top-performers, worst-performers, high-dividend, low-pe, 52w-high, 52w-low, high-volume, large-cap). Quote-level keys (pe fpe pb mcap price div eps volume chg range) come from one batch quote; ret1w ret1m ret3m ret6m retytd ret1y (price return %, as MSN reports it) from one request for MSN's whole-market list; fundamentals keys (roe roa margin opmargin de growth epsgrowth peg ps sector industry) are fetched per company for the largest max_fundamentals survivors. When a row's financial_currency differs from its currency (USD statements, IDR price), its price_to_book and price_to_sales divide the IDR price by USD figures: they are kept as reported, but no pb or ps clause matches them, and a pb or ps sort puts them last. Returns the idx.screen.v1 envelope with the preset, every clause applied and counts explaining how many tickers each stage kept."
    )]
    async fn screen(
        &self,
        Parameters(a): Parameters<ScreenArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                use crate::analysis::screen::{parse_filters, Key, ScreenPreset};
                let preset = match a.preset.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    Some(p) => Some(p.parse::<ScreenPreset>()?),
                    None => None,
                };
                let filters = parse_filters(a.filter.as_deref().unwrap_or(""))?;
                let sort = match a.sort.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    None => None,
                    Some(s) => Some(Key::parse(s).ok_or_else(|| {
                        IdxError::Usage(format!("unknown sort key {s:?}; use one of {}", Key::ALL))
                    })?),
                };
                let mut list = universe::load(None)?;
                universe::extend_from_ownership(&mut list, &self.ownership_db);
                let opts = service::ScreenOptions {
                    preset,
                    filters,
                    sort,
                    descending: a.ascending.map(|asc| !asc),
                    limit: a.limit.unwrap_or(25) as usize,
                    max_fundamentals: a.max_fundamentals.unwrap_or(50) as usize,
                    with_fundamentals: false,
                    with_returns: a.returns.unwrap_or(false),
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
        description = "Broker summary for one or more IDX tickers (Indo Premier / IPOT): which brokers bought and sold the stock over one day or a date range. Top 10 buyers and top 10 sellers as the source ranks them (by lots), each with the broker code, name (IDX member list), class (local, foreign, state_owned = BUMN), lots, value in IDR and average price, plus total value, total lots, average price, foreign net value, buy_value and sell_value. With investor foreign or domestic the source's totals are that side's buying, so total_value, total_lots and average_price are null and buy_value/sell_value say what the side bought and sold; with a board filter foreign_net_value is null (the source's net covers every board). Over a range the foreign net is rebuilt from single sessions (the source counts its latest session twice there). Without dates each ticker gets its latest session with trades. The source rounds figures of a million and more to one decimal; such summaries carry rounded=true. Returns the idx.broker.v1 envelope; failed tickers are listed in warnings."
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
        description = "Bandarmology (bandar detector) for one or more IDX tickers, from the IPOT broker summary: are the top brokers accumulating or distributing? For N = 1, 3, 5 and 10 the top N buyers' value minus the top N sellers' value (bandar value; in lots bandar volume) as a percent of all value traded: +15% or more big_accumulation, +5% accumulation, -5% distribution, -15% or less big_distribution, otherwise neutral. The headline signal uses top (default 5). phase reads it against the price move over the range (accumulation with the price within +2% = quiet_accumulation, distribution with the price within -2% = quiet_distribution, else markup/markdown). bandar_average is the top buyers' average price and bandar_gain_pct the last price against it; foreign_net_pct the foreign flow. With investor foreign or domestic the brokers are that side's and every percentage stays a share of the stock's value traded. Default period 1w ending at the latest session; daily=true adds the session-by-session movement and streak. reasons explain the reading. Heuristic, not advice. Returns the idx.bandar.v1 envelope; failed tickers are listed in warnings."
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
        description = "Bandarmology screener: scans the most traded IDX stocks by value (default 100, or the given symbols) over a period ending at the latest session and returns the ones whose top brokers accumulate or distribute, as defined by idx_bandar. Presets: big-accumulation (acc>=15), accumulation (acc>=5), distribution (acc<=-5), big-distribution (acc<=-15), quiet-accumulation (top brokers buying while the price is flat or down), quiet-distribution, foreign-accumulation (acc>=5 and foreign net >=5% of value), below-bandar (accumulation with the price under the top buyers' average). filter adds clauses on acc, net, lots, fnet, fpct, gain, chg, turnover, conc, price, signal, phase. Rows carry signal, phase, net_pct, net_value, price move, foreign flow, bandar average and gain, and the top buyer codes. One broker-summary request per scanned stock, and on 1w/1m/3m usually three more for its foreign net (up to six when the latest sessions had no trades), which is rebuilt from single sessions because the source counts its latest session twice over a range (cached). Heuristic, not advice. Returns the idx.bandar.screen.v1 envelope."
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
        name = "idx_insights",
        description = "MSN Money's rule-based insights for one IDX ticker: about 30 statements in five categories (Valuation, Earnings, Growth, Performance, Health), each rated good, bad, neutral or not_rated against the company's peers, its industry, its own history, the price or a fixed threshold (compared_with), with the value and benchmark_value as MSN reports them (units differ by statement), the counts per rating, when MSN last updated them and the peer tickers MSN compares against. Returns the idx.insights.v1 envelope."
    )]
    async fn insights(
        &self,
        Parameters(a): Parameters<SymbolArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                ok(SCHEMA_INSIGHTS, self.provider.insights(&symbol).await?)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_ticker",
        description = "Shareholders above 1% of one IDX ticker from KSEI's monthly register (local SQLite): holder name, investor type, local/foreign, shares, percent, sorted by percent. A holder listed on several register lines (scrip and scripless accounts) is summed into one row with `accounts`; from the 30 Oct 2026 register each row carries `controller_or_affiliate` (`controller_reported` says whether the release has it). Returns the idx.ownership.holders.v1 envelope. Needs ownership data: idx_ownership_snapshot with action \"sync\" fills an empty database."
    )]
    async fn ownership_ticker(
        &self,
        Parameters(a): Parameters<OwnershipTickerArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                let as_of = parse_date(&a.as_of)?;
                let report = ownership::holders(&self.db()?, symbol.as_str(), as_of)?;
                ok(SCHEMA_OWNERSHIP_HOLDERS, report)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_entity",
        description = "Every IDX position above 1% held by a person or institution whose name contains the query, from KSEI's monthly register (local SQLite). Spelling variants (PT/Tbk placement, punctuation) are grouped. With exact=true only the holder whose entity_key is the query (use a key from idx_ownership_search). Returns the idx.ownership.entity.v1 envelope."
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
                let report =
                    ownership::entity(&self.db()?, &a.name, as_of, a.exact.unwrap_or(false))?;
                ok(SCHEMA_OWNERSHIP_ENTITY, report)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_changes",
        description = "Month-over-month changes in holders above 1% between two imported release dates: new holders, exits and position changes with percentage-point and share deltas, largest moves first. The summary block (counts, net percentage points for foreign vs local holders and per investor type, per-issuer flow, top rises and falls) always covers every match; limit/offset page the rows. Optional ticker filter. Holders are compared by their summed position; `accounts_from`/`accounts_to` show the number of register lines. Returns the idx.ownership.changes.v1 envelope."
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
                let report = ownership::changes(
                    &self.db()?,
                    from,
                    to,
                    symbol.as_ref().map(Symbol::as_str),
                    a.min_pct_change.unwrap_or(0.0),
                    limit,
                    a.offset.unwrap_or(0) as usize,
                )?;
                ok(SCHEMA_OWNERSHIP_CHANGES, report)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_foreign",
        description = "Disclosed foreign (KSEI flag A) versus local (L) stake in the >1% register. With symbol: one issuer's foreign percent, holder counts and month-over-month change for every imported release (idx.ownership.foreign.v1). Without symbol: every issuer's change in foreign percent between two releases, largest moves first, with inflow/outflow counts (idx.ownership.foreign-flow.v1). Sums cover holders above 1% only. Needs ownership data: idx_ownership_snapshot with action \"sync\" fills an empty database."
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
                        let report = ownership::foreign_trend(&db, sym.as_str())?;
                        ok(SCHEMA_OWNERSHIP_FOREIGN, report)
                    }
                    None => {
                        let limit = match a.limit {
                            Some(0) => None,
                            Some(n) => Some(n as usize),
                            None => Some(25),
                        };
                        let report = ownership::foreign_flow(
                            &db,
                            parse_date(&a.from)?,
                            parse_date(&a.to)?,
                            limit,
                        )?;
                        ok(SCHEMA_OWNERSHIP_FOREIGN_FLOW, report)
                    }
                }
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_releases",
        description = "Lists the monthly KSEI register releases imported into the local database (as-of date, source, `rows` = register lines, `positions` after summing each holder's accounts, `other_date_rows`, `controller_column`). Releases imported by older idx versions are named in `warnings`. Use it to pick dates for idx_ownership_changes. Returns the idx.ownership.releases.v1 envelope."
    )]
    async fn ownership_releases(&self) -> std::result::Result<CallToolResult, McpError> {
        finish(async { ok(SCHEMA_OWNERSHIP_RELEASES, ownership::releases(&self.db()?)?) }.await)
    }

    #[tool(
        name = "idx_ownership_search",
        description = "Finds holders in KSEI's monthly register of shareholders above 1% (local SQLite) by name: case-insensitive, words in any order, every spelling KSEI printed in any imported month included. Each match has the entity_key (pass it to idx_ownership_entity with exact=true, or to idx_ownership_graph), the display name, how it matched (exact, spelling, prefix, contains, words), its spellings, the issuers it holds above 1% in the release with their summed percent, investor type, local/foreign flag and the last release it appears in. No match is an empty list with a warning. Returns the idx.ownership.search.v1 envelope."
    )]
    async fn ownership_search(
        &self,
        Parameters(a): Parameters<OwnershipSearchArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                ownership::analytics::search_key(&a.query)?;
                let as_of = parse_date(&a.as_of)?;
                // 0 means all, as for every idx limit, within the cap of 100.
                let limit = match a.limit {
                    Some(0) => 100,
                    Some(n) => n.min(100),
                    None => 20,
                } as usize;
                let report = ownership::search(&self.db()?, &a.query, as_of, Some(limit))?;
                ok(SCHEMA_OWNERSHIP_SEARCH, report)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_rankings",
        description = "Rankings over KSEI's monthly register of holders above 1% (local SQLite), accounts summed per holder. view=holders (default): holders that own more than 1% of the most IDX issuers, with issuer count, summed percent (a breadth measure, not a share of anything), largest stake and every position, optionally filtered by KSEI investor type or foreign/local; custodian and nominee accounts rank high because they hold for clients (idx.ownership.cross-holders.v1 envelope). view=issuers: ownership concentration per issuer, the Herfindahl-Hirschman index (sum of squared stakes of the listed holders, 0-10,000), largest and top-3 stakes, listed and undisclosed percent, holder count and, from the 30 Oct 2026 register, the controller group's percent; holders at or below 1% are not published, so HHI is a lower bound. Ranks every issuer by hhi, top1 or top3 (ascending for the most widely held) or reports the given symbols (idx.ownership.concentration.v1 envelope)."
    )]
    async fn ownership_rankings(
        &self,
        Parameters(a): Parameters<OwnershipRankingsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let view = a
                    .view
                    .as_deref()
                    .map(|v| v.trim().to_ascii_lowercase())
                    .filter(|v| !v.is_empty())
                    .unwrap_or_else(|| "holders".into());
                let as_of = parse_date(&a.as_of)?;
                let limit = match a.limit {
                    Some(0) => None,
                    Some(n) => Some(n as usize),
                    None => Some(25),
                };
                let given = |v: &Option<String>| v.as_deref().is_some_and(|s| !s.trim().is_empty());
                match view.as_str() {
                    // The other view's arguments are refused unless they hold
                    // their default, which changes nothing.
                    "holders" => {
                        let by = parse_opt(&a.by, ConcentrationSort::Hhi)?;
                        if given(&a.symbols)
                            || by != ConcentrationSort::Hhi
                            || a.ascending == Some(true)
                        {
                            return Err(IdxError::Usage(
                                "symbols, by and ascending apply to view \"issuers\"".into(),
                            ));
                        }
                        let origin =
                            match a.origin.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                                Some(o) => Some(o.parse::<HolderOrigin>()?),
                                None => None,
                            };
                        let filter = ownership::analytics::CrossFilter {
                            min_issuers: a.min_issuers.unwrap_or(2) as usize,
                            investor_type: a.investor_type.clone(),
                            origin,
                        };
                        let report = ownership::cross_holders(&self.db()?, &filter, limit, as_of)?;
                        ok(SCHEMA_OWNERSHIP_CROSS_HOLDERS, report)
                    }
                    "issuers" => {
                        if a.min_issuers.is_some_and(|n| n != 2)
                            || given(&a.investor_type)
                            || given(&a.origin)
                        {
                            return Err(IdxError::Usage(
                                "min_issuers, investor_type and origin apply to view \"holders\""
                                    .into(),
                            ));
                        }
                        let by = parse_opt(&a.by, ConcentrationSort::Hhi)?;
                        let symbols = match a
                            .symbols
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                        {
                            Some(s) => Some(Symbol::parse_list([s])?),
                            None => None,
                        };
                        let report = ownership::concentration(
                            &self.db()?,
                            symbols.as_deref(),
                            by,
                            a.ascending.unwrap_or(false),
                            limit,
                            as_of,
                        )?;
                        ok(SCHEMA_OWNERSHIP_CONCENTRATION, report)
                    }
                    other => Err(IdxError::Usage(format!(
                        "invalid view {other:?}; use holders or issuers"
                    ))),
                }
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_snapshot",
        description = "The local KSEI ownership database (holders above 1%) and the prebuilt copy on the project's GitHub mirror. action \"status\": the local SQLite file (path, schema version, imported months, the snapshot it was installed from) and, unless offline, what the mirror and its snapshot hold, the missing months and what sync would do (idx.ownership.status.v1); it writes nothing. action \"sync\": brings the database up to date by installing the prebuilt snapshot (a few MB, SHA-256 checked) when the database is empty, older or a subset of it, otherwise by importing only the missing monthly files; months only the local database has are never dropped unless force=true, and a .bak copy is kept (idx.ownership.snapshot-sync.v1). Call status first; call sync when the ownership tools report an empty database."
    )]
    async fn ownership_snapshot(
        &self,
        Parameters(a): Parameters<OwnershipSnapshotArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let offline = self.fetcher.is_offline();
                match a.action.trim().to_ascii_lowercase().as_str() {
                    "sync" => {
                        let base = snapshot::mirror_base(offline)?;
                        let s = snapshot::sync(
                            &self.ownership_db,
                            &base,
                            a.force.unwrap_or(false),
                            SNAPSHOT_TIMEOUT,
                        )
                        .await?;
                        ok(SCHEMA_OWNERSHIP_SNAPSHOT_SYNC, s)
                    }
                    "status" => {
                        if a.force == Some(true) {
                            return Err(IdxError::Usage(
                                "force applies to action \"sync\" only".into(),
                            ));
                        }
                        let (base, note) = snapshot::status_mirror(offline);
                        let mut s =
                            snapshot::status(&self.ownership_db, base.as_deref(), SNAPSHOT_TIMEOUT)
                                .await?;
                        s.warnings.splice(0..0, note);
                        ok(SCHEMA_OWNERSHIP_STATUS, s)
                    }
                    other => Err(IdxError::Usage(format!(
                        "action must be \"sync\" or \"status\", not {other:?}"
                    ))),
                }
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_graph",
        description = "Ownership network around a ticker or a holder from KSEI's monthly register of holders above 1% (local SQLite): walks holder-issuer links for 1-6 hops (default 2: a ticker's holders and their other holdings, or a holder's issuers and their co-holders), strongest links first. Returns issuers and holders with their hop count (node ids ticker:SYMBOL and holder:entity_key) and every link between them (holder, issuer, percent, shares) in the idx.ownership.graph.v1 envelope; min_pct keeps larger stakes, max_nodes caps the size (truncated=true when hit). An ambiguous holder name starts at the best match and says so in warnings."
    )]
    async fn ownership_graph(
        &self,
        Parameters(a): Parameters<OwnershipGraphArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let as_of = parse_date(&a.as_of)?;
                let report = ownership::graph(
                    &self.db()?,
                    &a.root,
                    a.holder.unwrap_or(false),
                    a.depth.unwrap_or(2) as usize,
                    a.min_pct.unwrap_or(0.0),
                    a.max_nodes.unwrap_or(100) as usize,
                    as_of,
                )?;
                ok(SCHEMA_OWNERSHIP_GRAPH, report)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_composition",
        description = "KSEI's monthly composition of one IDX stock (ksei.co.id \"Kepemilikan Efek\" archive): local versus foreign shares in nine investor types (insurance, corporate, pension fund, financial institution, individual, mutual fund, securities company, foundation, others) as a percent of the shares held in KSEI accounts (scripless), the scripless share of all listed shares (`scripless_pct`; shares held as certificates are outside the archive), and the foreign share for every imported month with the change in percentage points. Covers every holder, not only those above 1%, and names none. Downloads the newest month (about 125 KB) into the local database on first use and when a newer month can exist; `as_of` downloads a listed month that is missing. Returns the idx.ownership.composition.v1 envelope."
    )]
    async fn ownership_composition(
        &self,
        Parameters(a): Parameters<OwnershipCompositionArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                let as_of = parse_date(&a.as_of)?;
                let mut db = self.db()?;
                let report = ownership::composition::report(
                    &mut db,
                    &self.fetcher,
                    symbol.as_str(),
                    as_of,
                    std::time::Duration::from_secs(60),
                    service::today_wib(),
                )
                .await?;
                ok(SCHEMA_OWNERSHIP_COMPOSITION, report)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_ownership_institutions",
        description = "Institutional and fund holders of one IDX ticker from MSN Money (live, not the KSEI register; no import needed): up to 15 institutions and 20 funds with shares, percent of shares outstanding, the change since each holder's previous report (each holder reports on its own date, see report_date) and a signal computed by idx (new, buyer, seller, exited, unchanged), plus institutional, top-10 institutional, fund and free-float percentages and signal counts per list. Funds overlap their managers: never add the two lists. A stock MSN lists no holders for returns an empty report with a warning. Returns the idx.ownership.institutions.v1 envelope."
    )]
    async fn ownership_institutions(
        &self,
        Parameters(a): Parameters<SymbolArg>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let symbol = Symbol::parse(&a.symbol)?;
                ok(
                    SCHEMA_OWNERSHIP_INSTITUTIONS,
                    self.provider.institutions(&symbol).await?,
                )
            }
            .await,
        )
    }

    #[tool(
        name = "idx_orderbook",
        description = "Realtime order book for one IDX ticker from the user's Stockbit session: the full depth ladder (bids and asks, sizes in shares), last/open/high/low, session volume and value, and foreign buy/sell/net (null, with a warning, while Stockbit marks them as not current with has_foreign_bs not true; a book recorded during trading carried the previous session's figures that way). Returns the idx.orderbook.v1 envelope. Needs a Stockbit login (idx_stockbit_status says whether one exists); errors with 'no Stockbit session' otherwise."
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
        description = "Whether the stored Stockbit session can work now, and when its access and refresh tokens expire (never the token values). Call this before idx_orderbook, idx_stockbit_actions, idx_stockbit_movements or the stockbit provider. logged_in is false when no session is stored or it cannot work (expired is true when its tokens have expired); renewable is false when idx cannot renew it, so a logged-in session stops working when its access token expires; reason says why and what to do. If logged_in is false, tell the user to run `idx stockbit login` in a terminal (or start idx_stockbit_login when they ask); when only renewable is false, suggest the same before the access token expires. While a page sign-in started by idx_stockbit_login runs, sign_in shows its state (waiting, signed_in, failed), url, ssh_forward, expires_in_secs and the latest warning."
    )]
    async fn stockbit_status(&self) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let mut status = self.stockbit_store().status(chrono::Utc::now().timestamp());
                status.sign_in = page::progress();
                Ok(CallToolResult::success(vec![ContentBlock::text(
                    json::to_pretty(&status)?,
                )]))
            }
            .await,
        )
    }

    #[tool(
        name = "idx_stockbit_login",
        description = "Starts a Stockbit sign-in for the machine running this server; use it only when the user asks (otherwise tell them to run `idx stockbit login` in a terminal). method 'page' returns at once with sign_in.url, a page on 127.0.0.1 (plus sign_in.ssh_forward, an ssh -L command, when this server is reached over SSH): give the user the URL; they sign in to stockbit.com in their own browser, ideally a private window, and send the session with the page's bookmark or by pasting the cookie there; then poll idx_stockbit_status until sign_in.state is signed_in or failed. method 'browser' opens Chrome, Edge or Brave on Stockbit's sign-in page on this machine and waits until the user signs in or the timeout; it suits a local stdio server the user sits at. Default 'auto' picks page over SSH, without a display or without a browser. Either way idx checks the session with Stockbit before storing it, and a session that already works and that idx can renew is reported (sign_in.state already_signed_in) without opening anything unless force is true. Never ask the user for their Stockbit password, and never ask them to paste the cookie or a token into this conversation."
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
                let timeout = std::time::Duration::from_secs(secs);
                let sb = Arc::new(self.stockbit_provider());
                let now = chrono::Utc::now().timestamp();
                let env = |k: &str| std::env::var(k).ok();
                let method = a
                    .method
                    .as_deref()
                    .map(|m| m.trim().to_ascii_lowercase())
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "auto".into());
                let use_page = match method.as_str() {
                    "page" => true,
                    "browser" => false,
                    "auto" => {
                        let set = |k: &str| env(k).is_some_and(|v| !v.trim().is_empty());
                        page::over_ssh(&env)
                            || (cfg!(all(unix, not(target_os = "macos")))
                                && !set("DISPLAY")
                                && !set("WAYLAND_DISPLAY"))
                            || login::find_browser().is_none()
                    }
                    other => {
                        return Err(IdxError::Usage(format!(
                            "unknown method {other:?}; use page, browser or auto"
                        )))
                    }
                };
                let port = a
                    .port
                    .map(u16::try_from)
                    .transpose()
                    .map_err(|_| IdxError::Usage("port must be 0 to 65535".into()))?;
                let reply = |status: &Status| -> Result<CallToolResult> {
                    Ok(CallToolResult::success(vec![ContentBlock::text(
                        json::to_pretty(status)?,
                    )]))
                };
                // A session that works and that idx can renew is reported, not replaced.
                if !a.force.unwrap_or(false) && sb.already_signed_in().await {
                    let mut status = sb.store().status(now);
                    status.sign_in = Some(SignInProgress {
                        state: "already_signed_in".into(),
                        method: "none".into(),
                        ..Default::default()
                    });
                    return reply(&status);
                }
                if use_page {
                    let progress = page::start_in_background(
                        sb.clone(),
                        page::PageOptions { port, timeout },
                        &env,
                    )
                    .await?;
                    let mut status = sb.store().status(now);
                    status.sign_in = Some(progress);
                    return reply(&status);
                }
                let opts = login::LoginOptions {
                    browser: None,
                    profile_dir,
                    timeout,
                };
                // Progress goes to the server's log (stderr), never stdout.
                let say = |s: &str| tracing::info!("{s}");
                let acc = login::capture(
                    &opts,
                    |v: String| {
                        let sb = sb.clone();
                        async move { sb.accept_session(&v).await }
                    },
                    &say,
                )
                .await?;
                let mut status = acc.status;
                status.sign_in = Some(SignInProgress {
                    state: "signed_in".into(),
                    method: "browser".into(),
                    warning: acc.warning,
                    ..Default::default()
                });
                reply(&status)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_stockbit_actions",
        description = "Corporate actions from the user's own Stockbit session, one structured row each. With symbol: that stock's whole history, newest first, as far back as Stockbit keeps it (BBCA's cash dividends go back to 2009). Without symbol: the market calendar, oldest first, dated from `from` to `to` (default today and the next 14 days, at most 366), plus rights trading, tender offers and IPO offers still open in the range. kind filters: dividend, stock_dividend, bonus, rights, warrant, split, reverse_split, rups, public_expose, ipo, tender_offer, or all (default: all but ipo for a stock, all but warrant for the calendar, as structured warrants list dozens a month). Each row: kind, date (cum date for entitlements, else the event's first day), cum/ex/recording/distribution dates, dividend_per_share and currency, ratio_old:ratio_new, factor, price (rights or warrant exercise, tender or IPO price, IDR), trading/exercise/offer windows, listing_date, shares and shares_pct, meeting time and venue; null where Stockbit has nothing. One request per kind, a second apart, cached 6 h; a failing kind leaves a warning. Needs a Stockbit login (idx_stockbit_status); IPOT's free view with announcement texts is idx_corporate_actions, the exchange's calendar idx_calendar. Returns the idx.stockbit.actions.v1 envelope."
    )]
    async fn stockbit_actions(
        &self,
        Parameters(a): Parameters<StockbitActionsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let from = parse_date(&a.from)?;
                let to = parse_date(&a.to)?;
                let symbol = match a.symbol.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                    Some(s) => Some(Symbol::parse(s)?),
                    None => None,
                };
                let sb = self.stockbit_provider();
                let s = match symbol {
                    Some(sym) => {
                        if a.days.is_some() {
                            return Err(IdxError::Usage(
                                "days is for the market calendar (no symbol); give from/to to narrow a stock's history".into(),
                            ));
                        }
                        let kinds = CorporateEventKind::parse_list(
                            a.kind.as_deref(),
                            &CorporateEventKind::STOCK_DEFAULT,
                        )?;
                        let list = universe::load(None).unwrap_or_default();
                        service::stockbit_actions(&sb, &list, &sym, &kinds, from, to).await?
                    }
                    None => {
                        let q = service::StockbitCalendarQuery {
                            from,
                            to,
                            days: a.days,
                            kinds: CorporateEventKind::parse_list(
                                a.kind.as_deref(),
                                &CorporateEventKind::CALENDAR_DEFAULT,
                            )?,
                        };
                        service::stockbit_calendar(&sb, &q, service::today_wib()).await?
                    }
                };
                ok(SCHEMA_STOCKBIT_ACTIONS, s)
            }
            .await,
        )
    }

    #[tool(
        name = "idx_stockbit_movements",
        description = "Reported changes in insiders' and major holders' stakes in IDX stocks, from the user's own Stockbit session, newest first: IDX insider disclosures (directors, commissioners and other insiders, any size, with the trade price) and KSEI's daily changes of holders at 5% and above (no price filed: Stockbit's estimate is in estimated_price and estimated_value, never in price or value). One stock's with symbol, the whole market's without; filter by from/to, holder (a row's holder_id, digits), action (buy, sell, transfer, cross, corporate_action) and source (idx, ksei). shares_from/to/change are exact and signed; pct_from/to/change are percent of the company as Stockbit rounds them; roles say director or commissioner. 50 rows a page, pages 1 to 10 (default 1; one request each, a second apart, cached 1 h); more says rows remain. Filings, not signals: many insiders buying at one price on one day is usually a share plan, and rows appear days after the change. Needs a Stockbit login (idx_stockbit_status). Returns the idx.stockbit.movements.v1 envelope."
    )]
    async fn stockbit_movements(
        &self,
        Parameters(a): Parameters<StockbitMovementsArgs>,
    ) -> std::result::Result<CallToolResult, McpError> {
        finish(
            async {
                let trimmed = |v: &Option<String>| -> Option<String> {
                    v.as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                };
                let action = match trimmed(&a.action) {
                    Some(s) => Some(
                        <MovementFilter as clap::ValueEnum>::from_str(&s, true).map_err(|_| {
                            IdxError::Usage(format!(
                                "unknown action {s:?}; use buy, sell, transfer, cross or corporate_action"
                            ))
                        })?,
                    ),
                    None => None,
                };
                let source = match trimmed(&a.source) {
                    Some(s) => Some(
                        <MovementSource as clap::ValueEnum>::from_str(&s, true).map_err(|_| {
                            IdxError::Usage(format!("unknown source {s:?}; use idx or ksei"))
                        })?,
                    ),
                    None => None,
                };
                let q = MovementQuery {
                    symbol: trimmed(&a.symbol)
                        .map(|s| Symbol::parse(&s))
                        .transpose()?,
                    from: parse_date(&a.from)?,
                    to: parse_date(&a.to)?,
                    holder_id: trimmed(&a.holder),
                    action,
                    source,
                };
                q.validate()?;
                let pages = a.pages.unwrap_or(1);
                ok(
                    SCHEMA_STOCKBIT_MOVEMENTS,
                    service::stockbit_movements(&self.stockbit_provider(), &q, pages).await?,
                )
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

    /// The Stockbit session store lives as long as the server: its pacer and
    /// the refresh token it saw refused carry from one tool call to the next,
    /// and every clone (one per MCP HTTP session) shares it.
    #[test]
    fn the_stockbit_store_lives_as_long_as_the_server() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(crate::http::FixtureBackend::new()),
            crate::cache::Cache::new(dir.path().join("cache")),
            crate::http::CacheMode::Offline,
        ));
        let cfg = StockbitConfig {
            options: StockbitOptions {
                session_path: dir.path().join("stockbit-session.json"),
                ..Default::default()
            },
            ..Default::default()
        };
        let s = IdxServer::new(
            Arc::new(Fake),
            fetcher.clone(),
            dir.path().join("ownership.sqlite"),
            cfg.clone(),
        );
        let first = s.stockbit_store();
        let held = Arc::downgrade(&first);
        drop(first);
        let second = s.stockbit_store();
        let first = held.upgrade().expect("the server still holds the store");
        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::ptr_eq(&s.clone().stockbit_store(), &second));
        drop((first, second));
        drop(s);
        // Servers built from one config (one per HTTP session) share it too.
        let cfg = cfg.resolved();
        let a = IdxServer::new(
            Arc::new(Fake),
            fetcher.clone(),
            dir.path().join("ownership.sqlite"),
            cfg.clone(),
        );
        let held = Arc::downgrade(&a.stockbit_store());
        drop(a);
        let b = IdxServer::new(
            Arc::new(Fake),
            fetcher,
            dir.path().join("ownership.sqlite"),
            cfg,
        );
        assert!(Arc::ptr_eq(&held.upgrade().unwrap(), &b.stockbit_store()));
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
        // The count `idx mcp setup` prints (clients::tool_count reads the router).
        assert_eq!(names.len(), 47);
        assert_eq!(
            names,
            [
                "idx_astro",
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
                "idx_insights",
                "idx_instruments",
                "idx_intraday",
                "idx_market",
                "idx_market_brokers",
                "idx_market_foreign",
                "idx_members",
                "idx_news",
                "idx_orderbook",
                "idx_ownership_changes",
                "idx_ownership_composition",
                "idx_ownership_entity",
                "idx_ownership_foreign",
                "idx_ownership_graph",
                "idx_ownership_institutions",
                "idx_ownership_rankings",
                "idx_ownership_releases",
                "idx_ownership_search",
                "idx_ownership_snapshot",
                "idx_ownership_ticker",
                "idx_profile",
                "idx_quote",
                "idx_returns",
                "idx_screen",
                "idx_sentiment",
                "idx_stockbit_actions",
                "idx_stockbit_login",
                "idx_stockbit_movements",
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

        // One-minute bars over a month: refused before any fetch.
        let r = s
            .history(Parameters(HistoryArgs {
                symbol: "BBCA".into(),
                period: Some("1mo".into()),
                interval: Some("1m".into()),
                limit: None,
            }))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "usage", "{v}");
        assert!(v["error"]["message"].as_str().unwrap().contains("1d or 5d"));
        for (interval, code) in [("1d", "usage"), ("7m", "usage"), ("1m", "offline_miss")] {
            let r = s
                .intraday(Parameters(IntradayArgs {
                    symbol: "BBCA".into(),
                    interval: Some(interval.into()),
                    limit: None,
                }))
                .await
                .unwrap();
            assert_eq!(r.is_error, Some(true));
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], code, "{interval}: {v}");
        }

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

    fn server_at(db: &std::path::Path) -> IdxServer {
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(crate::http::FixtureBackend::new()),
            crate::cache::Cache::new(db.with_extension("cache")),
            crate::http::CacheMode::Offline,
        ));
        IdxServer::new(
            Arc::new(Fake),
            fetcher,
            db.to_path_buf(),
            StockbitConfig::default(),
        )
    }

    fn json_of(r: &CallToolResult) -> serde_json::Value {
        serde_json::from_str(&text(r)).unwrap()
    }

    #[tokio::test]
    async fn astro_tool_computes_offline_and_checks_its_arguments() {
        let s = server();
        let args = |date: Option<&str>,
                    natal: Option<&str>,
                    backtest: Option<bool>,
                    period: Option<&str>| {
            Parameters(AstroArgs {
                date: date.map(Into::into),
                days: Some(7),
                natal: natal.map(Into::into),
                backtest,
                period: period.map(Into::into),
            })
        };
        let r = s
            .astro(args(Some("2026-11-06"), None, None, None))
            .await
            .unwrap();
        assert_ne!(r.is_error, Some(true), "{}", text(&r));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["schema"], "idx.astro.v1");
        assert_eq!(v["provider"], "computed");
        assert_eq!(v["data"]["weton"]["pasaran"], "Pahing");
        assert_eq!(v["data"]["natal"]["chart"], "launch");
        assert!(v["data"]["backtest"].is_null());
        for bad in [
            args(Some("2026-11-06"), Some("birth"), None, None),
            args(Some("2026-11-06"), None, Some(true), Some("6mo")),
            args(Some("2026-11-06"), None, None, Some("2y")),
            args(Some("2026-11-06"), None, Some(false), Some("6mo")),
            args(Some("1700-01-01"), None, None, None),
            args(Some("06/11/2026"), None, None, None),
        ] {
            let r = s.astro(bad).await.unwrap();
            assert_eq!(r.is_error, Some(true));
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], "usage", "{v}");
        }
        // The fake provider has no history cached: the backtest is an offline miss.
        let r = s
            .astro(args(
                Some("2026-11-06"),
                Some("none"),
                Some(true),
                Some("2y"),
            ))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["exit_code"], 5, "{v}");
    }

    #[tokio::test]
    async fn earnings_tool_checks_its_filters_before_fetching() {
        let s = server();
        let args = |period: Option<&str>, kind: Option<&str>| EarningsArgs {
            symbol: "TLKM".into(),
            period: period.map(Into::into),
            kind: kind.map(Into::into),
        };
        for bad in [args(Some("weekly"), None), args(None, Some("forecasts"))] {
            let r = s.earnings(Parameters(bad)).await.unwrap();
            assert_eq!(r.is_error, Some(true));
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], "usage", "{v}");
        }
        // Valid filters reach the provider (the fake has no earnings).
        for good in [
            args(Some("annual"), Some("estimates")),
            args(Some("all"), None),
            args(None, Some("reported")),
        ] {
            let r = s.earnings(Parameters(good)).await.unwrap();
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], "unsupported", "{v}");
        }
    }

    #[tokio::test]
    async fn ownership_analytics_tools_check_their_arguments_and_answer() {
        // Its own database: tests run in parallel and `server()` shares one.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("own.sqlite");
        let s = server_at(&path);
        let search = |q: &str| OwnershipSearchArgs {
            query: q.into(),
            limit: None,
            as_of: None,
        };
        let graph =
            |root: &str, depth: Option<u32>, min_pct: Option<f64>, max_nodes: Option<u32>| {
                OwnershipGraphArgs {
                    root: root.into(),
                    depth,
                    min_pct,
                    max_nodes,
                    holder: None,
                    as_of: None,
                }
            };
        let rankings =
            |view: Option<&str>, by: Option<&str>, origin: Option<&str>, symbols: Option<&str>| {
                OwnershipRankingsArgs {
                    view: view.map(Into::into),
                    min_issuers: None,
                    investor_type: None,
                    origin: origin.map(Into::into),
                    symbols: symbols.map(Into::into),
                    by: by.map(Into::into),
                    ascending: None,
                    limit: None,
                    as_of: None,
                }
            };
        let code = |r: &CallToolResult| {
            json_of(r)["error"]["code"]
                .as_str()
                .unwrap_or("")
                .to_string()
        };

        // Empty database: not found, as for the other ownership tools.
        let r = s
            .ownership_search(Parameters(search("salim")))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        assert_eq!(code(&r), "not_found");
        let r = s
            .ownership_rankings(Parameters(rankings(None, None, None, None)))
            .await
            .unwrap();
        assert_eq!(code(&r), "not_found");

        // Bad arguments are usage errors whatever the database holds.
        let r = s.ownership_search(Parameters(search("x"))).await.unwrap();
        assert_eq!(code(&r), "usage");
        for bad in [
            graph("AADI", Some(0), None, None),
            graph("AADI", Some(7), None, None),
            graph("AADI", None, Some(150.0), None),
            graph("AADI", None, None, Some(2000)),
        ] {
            let r = s.ownership_graph(Parameters(bad)).await.unwrap();
            assert_eq!(code(&r), "usage");
        }
        for bad in [
            rankings(Some("nope"), None, None, None),
            rankings(Some("issuers"), Some("nope"), None, None),
            rankings(Some("holders"), None, Some("mars"), None),
            rankings(Some("holders"), Some("top1"), None, None),
            rankings(Some("holders"), None, None, Some("BBCA")),
            rankings(Some("issuers"), None, Some("foreign"), None),
        ] {
            let r = s.ownership_rankings(Parameters(bad)).await.unwrap();
            assert_eq!(code(&r), "usage", "{}", text(&r));
        }

        {
            let mut db = OwnershipDb::open(&path).unwrap();
            for (bytes, name) in [
                (
                    &include_bytes!(
                        "../../tests/fixtures/ownership/above1_20260831_analytics_excerpt.xlsx"
                    )[..],
                    "aug.xlsx",
                ),
                (
                    &include_bytes!(
                        "../../tests/fixtures/ownership/above1_20260930_analytics_excerpt.xlsx"
                    )[..],
                    "sep.xlsx",
                ),
            ] {
                let parsed = crate::ownership::ingest::parse_xlsx(bytes).unwrap();
                db.import(&parsed, "xlsx", name, name).unwrap();
            }
        }

        let v = json_of(
            &s.ownership_search(Parameters(search("permadi")))
                .await
                .unwrap(),
        );
        assert_eq!(v["schema"], "idx.ownership.search.v1");
        assert_eq!(v["provider"], "local-db");
        assert_eq!(v["data"]["matches"][0]["entity_key"], "T PERMADI RACHMAT");
        assert_eq!(v["data"]["matches"][1]["last_seen"], "2026-08-31");
        // limit 0 is every match (within the cap), as for the other limits.
        for (limit, shown) in [(Some(1), 1), (Some(0), 2), (None, 2)] {
            let mut args = search("permadi");
            args.limit = limit;
            let v = json_of(&s.ownership_search(Parameters(args)).await.unwrap());
            assert_eq!(v["data"]["total_matches"], 2);
            assert_eq!(
                v["data"]["matches"].as_array().unwrap().len(),
                shown,
                "{limit:?}"
            );
        }

        let v = json_of(
            &s.ownership_rankings(Parameters(rankings(None, None, None, None)))
                .await
                .unwrap(),
        );
        assert_eq!(v["schema"], "idx.ownership.cross-holders.v1");
        assert_eq!(v["data"]["holders"][0]["entity_key"], "DRS SURONO SUBEKTI");
        assert_eq!(v["data"]["total_matches"], 8);
        // The other view's arguments at their defaults are fine; other values are not.
        let mut holders = rankings(None, Some(" HHI "), None, None);
        holders.ascending = Some(false);
        let r = s.ownership_rankings(Parameters(holders)).await.unwrap();
        assert_ne!(r.is_error, Some(true), "{}", text(&r));
        let mut holders = rankings(None, None, None, None);
        holders.ascending = Some(true);
        let r = s.ownership_rankings(Parameters(holders)).await.unwrap();
        assert_eq!(code(&r), "usage");
        for (min_issuers, ok) in [(Some(2), true), (Some(3), false), (Some(1), false)] {
            let mut issuers = rankings(Some("issuers"), None, None, None);
            issuers.min_issuers = min_issuers;
            let r = s.ownership_rankings(Parameters(issuers)).await.unwrap();
            assert_eq!(
                r.is_error != Some(true),
                ok,
                "{min_issuers:?}: {}",
                text(&r)
            );
        }
        let v = json_of(
            &s.ownership_rankings(Parameters(rankings(None, None, Some("foreign"), None)))
                .await
                .unwrap(),
        );
        assert_eq!(v["data"]["total_matches"], 2);

        let v = json_of(
            &s.ownership_rankings(Parameters(rankings(
                Some("issuers"),
                Some("top1"),
                None,
                Some("bbca, tcid"),
            )))
            .await
            .unwrap(),
        );
        assert_eq!(v["schema"], "idx.ownership.concentration.v1");
        assert_eq!(v["data"]["sort"], "top1");
        assert_eq!(v["data"]["rows"][0]["symbol"], "BBCA");
        assert_eq!(v["data"]["rows"][0]["hhi"], 3021.9114);
        assert_eq!(v["data"]["rows"][1]["symbol"], "TCID");
        assert!(v["data"]["rows"][0]["controller_pct"].is_null());

        let v = json_of(
            &s.ownership_graph(Parameters(graph("AADI", None, None, None)))
                .await
                .unwrap(),
        );
        assert_eq!(v["schema"], "idx.ownership.graph.v1");
        assert_eq!(v["data"]["root"], "ticker:AADI");
        assert_eq!(v["data"]["links"].as_array().unwrap().len(), 13);
        let r = s
            .ownership_graph(Parameters(graph("ZZZZ", None, None, None)))
            .await
            .unwrap();
        assert_eq!(code(&r), "not_found");

        let exact = |name: &str, exact: Option<bool>| OwnershipEntityArgs {
            name: name.into(),
            as_of: Some("2026-08-31".into()),
            exact,
        };
        let v = json_of(
            &s.ownership_entity(Parameters(exact("T PERMADI RACHMAT", None)))
                .await
                .unwrap(),
        );
        assert_eq!(
            v["data"]["holdings"].as_array().unwrap().len(),
            3,
            "substring"
        );
        let r = s
            .ownership_entity(Parameters(exact("T PERMADI RACHMAT", Some(true))))
            .await
            .unwrap();
        assert_eq!(code(&r), "not_found");
    }

    #[tokio::test]
    async fn a_cloudflare_refusal_is_a_blocked_tool_error() {
        let dir = tempfile::tempdir().unwrap();
        let challenge =
            include_str!("../../tests/fixtures/idx/cloudflare_challenge_synthetic.html");
        let backend = crate::http::FixtureBackend::new()
            .route_with_headers(
                "GetTradingInfoSS",
                403,
                challenge,
                &[("cf-mitigated", "challenge"), ("server", "cloudflare")],
            )
            .route("GetStockSummary", 503, "maintenance");
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(backend),
            crate::cache::Cache::new(dir.path().join("cache")),
            crate::http::CacheMode::Normal,
        ));
        let s = IdxServer::new(
            Arc::new(Fake),
            fetcher,
            dir.path().join("ownership.sqlite"),
            StockbitConfig::default(),
        );
        let r = s
            .foreign(Parameters(ForeignArgs {
                symbol: "BBCA".into(),
                days: None,
                from: None,
                to: None,
                source: None,
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "blocked", "{v}");
        assert_eq!(v["exit_code"], 3);
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(msg.contains("Cloudflare challenge"), "{msg}");
        // Advice an agent can act on: a tool argument, not the CLI's flag.
        assert!(
            msg.ends_with(
                "; or call the tool again with source \"ipot\" for Indo Premier's figures in rupiah (at most 60 sessions)"
            ),
            "{msg}"
        );
        assert!(!msg.contains("--"), "{msg}");
    }

    #[tokio::test]
    async fn blocked_advice_names_tool_arguments_not_flags() {
        let dir = tempfile::tempdir().unwrap();
        let server = |mode| {
            let backend = crate::http::FixtureBackend::new()
                .route("GetStockSummary", 403, "")
                .route("GetSecuritiesStock", 403, "");
            let fetcher = Arc::new(Fetcher::new(
                Arc::new(backend),
                crate::cache::Cache::new(dir.path().join("cache")),
                mode,
            ));
            IdxServer::new(
                Arc::new(Fake),
                fetcher,
                dir.path().join("ownership.sqlite"),
                StockbitConfig::default(),
            )
        };
        let s = server(crate::http::CacheMode::Normal);
        let v = json_of(
            &s.market_foreign(Parameters(MarketForeignArgs {
                date: None,
                top: None,
                sort: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(v["error"]["code"], "blocked", "{v}");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(
            msg.ends_with("with idx_bandar_screen, period \"1d\" and sort \"fnet\""),
            "{msg}"
        );
        assert!(!msg.contains("--"), "{msg}");

        // Warnings too: a server run with --no-cache says so, and leaves
        // out the CLI's --offline.
        let s = server(crate::http::CacheMode::Bypass);
        let v = json_of(
            &s.instruments(Parameters(InstrumentsArgs {
                board: None,
                search: Some("BBCA".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(v["provider"], "bundled", "{v}");
        let w = v["warnings"][0].as_str().unwrap();
        assert!(
            w.contains(", and the server runs with --no-cache, which skips the cache. "),
            "{w}"
        );
        assert!(!w.contains("--offline"), "{w}");
    }

    #[tokio::test]
    async fn composition_tool_never_downloads_offline() {
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(crate::http::FixtureBackend::new()),
            crate::cache::Cache::new(dir.path().join("cache")),
            crate::http::CacheMode::Offline,
        ));
        let s = IdxServer::new(
            Arc::new(Fake),
            fetcher,
            dir.path().join("ownership.sqlite"),
            StockbitConfig::default(),
        );
        let call = |symbol: &str, as_of: Option<&str>| OwnershipCompositionArgs {
            symbol: symbol.into(),
            as_of: as_of.map(Into::into),
        };
        let r = s
            .ownership_composition(Parameters(call("BBCA", None)))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "offline_miss", "{v}");
        let r = s
            .ownership_composition(Parameters(call("BBCA", Some("30/10/2026"))))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "usage", "{v}");

        let mut db = OwnershipDb::open(&dir.path().join("ownership.sqlite")).unwrap();
        ownership::import_file(
            &mut db,
            std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/tests/fixtures/ownership/ksei_balancepos_20261030_excerpt_constructed.zip"
            )),
        )
        .unwrap();
        let r = s
            .ownership_composition(Parameters(call("bbca", None)))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["schema"], "idx.ownership.composition.v1", "{v}");
        assert_eq!(v["provider"], "local-db");
        assert_eq!(v["data"]["foreign_pct"], 67.2201);
        assert_eq!(v["data"]["by_type"].as_array().unwrap().len(), 9);
    }

    #[tokio::test]
    async fn ownership_snapshot_tool_reports_offline_and_checks_its_action() {
        let s = server();
        let call = |action: &str, force: Option<bool>| OwnershipSnapshotArgs {
            action: action.into(),
            force,
        };
        let r = s
            .ownership_snapshot(Parameters(call("status", None)))
            .await
            .unwrap();
        assert_ne!(r.is_error, Some(true), "{}", text(&r));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["schema"], "idx.ownership.status.v1");
        assert_eq!(v["provider"], "local-db");
        assert!(v["data"]["remote"].is_null());
        assert!(v["data"]["sync"].is_null());
        assert_eq!(
            v["data"]["supported_schema_version"],
            ownership::SCHEMA_VERSION
        );
        assert!(v["warnings"][0].as_str().unwrap().contains("offline"));

        let r = s
            .ownership_snapshot(Parameters(call("SYNC", Some(true))))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "offline_miss", "{v}");
        assert_eq!(v["exit_code"], 5);

        for bad in [call("install", None), call("status", Some(true))] {
            let r = s.ownership_snapshot(Parameters(bad)).await.unwrap();
            assert_eq!(r.is_error, Some(true));
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], "usage", "{v}");
        }
    }

    #[tokio::test]
    async fn market_brokers_tool_takes_a_source_and_stands_in_for_idx_co_id() {
        let dir = tempfile::tempdir().unwrap();
        let backend = crate::http::FixtureBackend::new().route_with_headers(
            "GetBrokerSummary",
            403,
            include_str!("../../tests/fixtures/idx/cloudflare_challenge_synthetic.html"),
            &[("cf-mitigated", "challenge"), ("server", "cloudflare")],
        );
        // IPOT's pages as read on Saturday, served because IPOT has no route
        // here: a ranking read now could fall in trading hours, which IPOT's
        // ranking cannot be dated in.
        let cache = crate::cache::Cache::new(dir.path().join("cache"));
        let saturday = chrono::DateTime::parse_from_rfc3339("2026-11-07T10:00:00+07:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        for (url, page) in [
            (
                "https://www.indopremier.com/module/saham/include/ma-topbroker.php",
                include_str!("../../tests/fixtures/ipot/topbroker_20261106.html"),
            ),
            (
                "https://www.indopremier.com/module/saham/include/statistic.php?code=BBCA",
                include_str!("../../tests/fixtures/ipot/statistic_bbca_20261106.html"),
            ),
        ] {
            cache.put_fetched_at(url, 200, page, saturday).unwrap();
        }
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(backend),
            cache,
            crate::http::CacheMode::Normal,
        ));
        let s = IdxServer::new(
            Arc::new(Fake),
            fetcher,
            dir.path().join("ownership.sqlite"),
            StockbitConfig::default(),
        );
        let tool = s
            .tools()
            .into_iter()
            .find(|t| t.name == "idx_market_brokers")
            .unwrap();
        let source = &tool.input_schema["properties"]["source"];
        assert_eq!(source["type"], "string", "{source}");
        let call = |source: Option<&str>, date: Option<&str>| MarketBrokersArgs {
            date: date.map(Into::into),
            top: Some(3),
            sort: None,
            source: source.map(Into::into),
        };

        // auto, the default: idx.co.id is challenged, IPOT answers for the
        // session its copy holds. The date keeps the test off the clock:
        // with none, auto asks for the latest session that has ended.
        let v = json_of(
            &s.market_brokers(Parameters(call(None, Some("2026-11-06"))))
                .await
                .unwrap(),
        );
        assert_eq!(v["schema"], "idx.market.brokers.v1");
        assert_eq!(v["provider"], "ipot", "{v}");
        assert_eq!(v["data"]["date"], "2026-11-06");
        assert_eq!(v["data"]["brokers"].as_array().unwrap().len(), 3);
        assert_eq!(v["data"]["brokers"][0]["kind"], "local");
        assert!(v["warnings"][0]
            .as_str()
            .unwrap()
            .starts_with("idx.co.id answered with a Cloudflare challenge"));

        for (source, date, code) in [
            (Some("idx"), None, "blocked"),
            (Some("ipot"), Some("2026-10-16"), "usage"),
            (Some("rti"), None, "usage"),
        ] {
            let r = s
                .market_brokers(Parameters(call(source, date)))
                .await
                .unwrap();
            assert_eq!(r.is_error, Some(true), "{source:?}");
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], code, "{v}");
        }
    }

    #[tokio::test]
    async fn market_brokers_advice_names_the_source_argument() {
        // The tool reads the real clock, and in a trading day's hours IPOT
        // could not answer, so the service is called with Sunday's.
        let dir = tempfile::tempdir().unwrap();
        let backend = crate::http::FixtureBackend::new().route(
            "GetBrokerSummary",
            403,
            include_str!("../../tests/fixtures/idx/cloudflare_blocked.html"),
        );
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(backend),
            crate::cache::Cache::new(dir.path().join("cache")),
            crate::http::CacheMode::Normal,
        ));
        let sunday = chrono::DateTime::parse_from_rfc3339("2026-11-08T10:00:00+07:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let err = service::market_brokers_from(
            &fetcher,
            crate::model::BrokerSource::Idx,
            None,
            5,
            crate::model::BrokerSort::Value,
            sunday,
        )
        .await
        .unwrap_err();
        let v = json_of(&fail(err));
        assert_eq!(v["error"]["code"], "blocked", "{v}");
        let msg = v["error"]["message"].as_str().unwrap();
        assert!(
            msg.ends_with(
                "; or call the tool again with source \"ipot\" (or \"auto\", the default) for IPOT's (Indo Premier) ranking of its latest session"
            ),
            "{msg}"
        );
        assert!(!msg.contains("--"), "{msg}");
    }

    #[tokio::test]
    async fn screen_tool_checks_preset_and_keys_before_fetching() {
        let s = server();
        let args = |preset: Option<&str>, filter: Option<&str>, sort: Option<&str>| ScreenArgs {
            preset: preset.map(Into::into),
            filter: filter.map(Into::into),
            sort: sort.map(Into::into),
            ascending: None,
            limit: None,
            max_fundamentals: None,
            returns: None,
        };
        for bad in [
            args(Some("best-ever"), None, None),
            args(None, Some("ret2y>5"), None),
            args(Some("low-pe"), None, Some("nope")),
        ] {
            let r = s.screen(Parameters(bad)).await.unwrap();
            assert_eq!(r.is_error, Some(true));
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["error"]["code"], "usage", "{v}");
        }
        // The fake provider has no market-wide return windows: unsupported.
        let r = s
            .screen(Parameters(args(Some("top-performers"), None, None)))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["exit_code"], 6, "{v}");
        // The fake answers quotes; a preset without return keys works.
        let r = s
            .screen(Parameters(args(Some("large-cap"), None, None)))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["schema"], "idx.screen.v1", "{v}");
        assert_eq!(v["data"]["preset"], "large-cap");
        assert_eq!(v["data"]["sort"], "mcap");
    }

    #[tokio::test]
    async fn insights_tool_is_unsupported_without_msn() {
        let s = server();
        let r = s
            .insights(Parameters(SymbolArg {
                symbol: "BBCA".into(),
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["exit_code"], 6, "{v}");
    }

    fn fundamental_args(lens: Option<&str>) -> FundamentalArgs {
        FundamentalArgs {
            symbol: "BBCA".into(),
            lens: lens.map(Into::into),
        }
    }

    #[tokio::test]
    async fn fundamental_tool_adds_a_verdict() {
        let s = server();
        for lens in [None, Some(""), Some("  ")] {
            let r = s
                .fundamental(Parameters(fundamental_args(lens)))
                .await
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["schema"], "idx.fundamentals.v1");
            assert_eq!(v["data"]["verdict"]["label"], "insufficient_data");
        }
    }

    #[tokio::test]
    async fn fundamental_tool_returns_a_lens() {
        let s = server();
        for (lens, schema) in [
            ("growth", "idx.growth.v1"),
            ("Valuation", "idx.valuation.v1"),
            (" risk ", "idx.risk.v1"),
        ] {
            let r = s
                .fundamental(Parameters(fundamental_args(Some(lens))))
                .await
                .unwrap();
            assert_ne!(r.is_error, Some(true), "{lens}: {}", text(&r));
            let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
            assert_eq!(v["schema"], schema);
            assert_eq!(v["data"]["reading"]["label"], "insufficient_data");
            assert_eq!(v["data"]["symbol"], "BBCA");
        }
        // The fake has no statements and no candles: the lens still answers,
        // with warnings that say why.
        let r = s
            .fundamental(Parameters(fundamental_args(Some("risk"))))
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert!(v["data"]["price"].is_null());
        assert!(v["warnings"].to_string().contains("price history: "), "{v}");

        let r = s
            .fundamental(Parameters(fundamental_args(Some("bogus"))))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "usage");
        assert!(v["error"]["message"]
            .as_str()
            .unwrap()
            .contains("growth, valuation or risk"));
    }

    #[tokio::test]
    async fn institutions_tool_reports_an_unsupported_provider() {
        let s = server();
        let r = s
            .ownership_institutions(Parameters(SymbolArg {
                symbol: "BBCA".into(),
            }))
            .await
            .unwrap();
        assert_eq!(r.is_error, Some(true));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["error"]["code"], "unsupported", "{v}");
        assert_eq!(v["exit_code"], 6);
    }

    #[tokio::test]
    async fn institutions_tool_returns_the_msn_envelope() {
        use crate::http::{CacheMode, FixtureBackend};
        use crate::providers::msn::Msn;
        let be = FixtureBackend::new()
            .route(
                "csautosuggest/api/v1/Query?query=JTPE",
                200,
                include_str!("../../tests/fixtures/msn/autosuggest_jtpe.json"),
            )
            .route(
                "csautosuggest/api/v1/Query?query=ATAP",
                200,
                include_str!("../../tests/fixtures/msn/autosuggest_atap.json"),
            )
            .route(
                "Finance/Equities?ids=bkgwu2",
                200,
                include_str!("../../tests/fixtures/msn/equities_jtpe_all_signals.json"),
            )
            .route(
                "Finance/Equities?ids=bwzup2",
                200,
                include_str!("../../tests/fixtures/msn/equities_atap_empty_holders.json"),
            );
        let dir = tempfile::tempdir().unwrap();
        let fetcher = Arc::new(Fetcher::new(
            Arc::new(be),
            crate::cache::Cache::new(dir.path().join("cache")),
            CacheMode::Normal,
        ));
        let s = IdxServer::new(
            Arc::new(Msn::new(fetcher.clone(), Ttls::default(), None)),
            fetcher,
            dir.path().join("ownership.sqlite"),
            StockbitConfig::default(),
        );
        let call = |symbol: &str| {
            s.ownership_institutions(Parameters(SymbolArg {
                symbol: symbol.into(),
            }))
        };

        let r = call("jtpe").await.unwrap();
        assert_ne!(r.is_error, Some(true), "{}", text(&r));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["schema"], "idx.ownership.institutions.v1");
        assert_eq!(v["provider"], "msn");
        assert_eq!(v["data"]["symbol"], "JTPE");
        assert_eq!(v["data"]["holders"].as_array().unwrap().len(), 9);
        assert!(v.get("warnings").is_none(), "{v}");

        let r = call("ATAP").await.unwrap();
        assert_ne!(r.is_error, Some(true), "{}", text(&r));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(v["schema"], "idx.ownership.institutions.v1");
        assert_eq!(v["data"]["holders"].as_array().unwrap().len(), 0);
        assert_eq!(
            v["warnings"][0],
            "MSN lists no institutional or fund holders for ATAP"
        );
    }
}
