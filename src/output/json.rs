//! The JSON envelope every command emits with `-o json`.

use chrono::{DateTime, SubsecRound, Utc};
use schemars::JsonSchema;
use serde::Serialize;

use crate::error::{IdxError, Result};
use crate::model::{
    BandarReport, BandarScreen, Brief, BrokerSummary, Calendar, ChangesReport, CompositionReport,
    CorporateActions, DiscoveredFile, DividendHistory, Earnings, EntityReport, Financials,
    ForeignFlowReport, ForeignReport, Fundamentals, History, HoldersReport, Holidays, ImportReport,
    IndexBoard, InstrumentList, MarketOverview, MemberList, NewsReport, OrderBook, Performance,
    Profile, Quote, Release, ScreenReport, SentimentReport, SnapshotManifest, StaleReason,
    StatusReport, SyncReport, Technical, Watchlist,
};
use crate::model::{BrokerActivity, BrokerRanking, ForeignFlow, MarketForeign};
use crate::model::{
    ConcentrationReport, CrossHoldersReport, GraphReport, Resolution, ResolutionReport,
    ResolveCandidates, SearchReport,
};
use crate::providers::Sourced;

/// Stable wrapper. `schema` names the payload shape and is versioned
/// independently of the binary (`idx.quotes.v1`, `idx.history.v1`, ...).
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct Envelope<T> {
    /// Payload identifier, e.g. `idx.quotes.v1`.
    pub schema: String,
    /// Provider that produced the data (`yahoo`, `msn`, ...).
    pub provider: String,
    /// When the underlying response was fetched from the network (UTC).
    pub fetched_at: DateTime<Utc>,
    /// `true` when served from the local cache.
    pub cached: bool,
    /// `true` when the cached copy is older than its TTL (offline or network failure).
    pub stale: bool,
    /// Why `stale` is true: `offline` (`--offline`, the network was not
    /// tried) or `fetch_failed` (the live request failed; a warning names
    /// the failure). Omitted when the data is not stale.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub stale_reason: Option<StaleReason>,
    /// Non-fatal notes, e.g. symbols the source did not know.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub warnings: Vec<String>,
    pub data: T,
}

impl<T> Envelope<T> {
    pub fn from_sourced(schema: &str, s: Sourced<T>) -> Self {
        Envelope {
            schema: schema.to_string(),
            provider: s.provider.to_string(),
            fetched_at: s.fetched_at.trunc_subsecs(0),
            cached: s.cached,
            stale: s.stale,
            stale_reason: s.stale_reason,
            warnings: s.warnings,
            data: s.data,
        }
    }
}

pub const SCHEMA_QUOTES: &str = "idx.quotes.v1";
pub const SCHEMA_HISTORY: &str = "idx.history.v1";
pub const SCHEMA_PROFILE: &str = "idx.profile.v1";
pub const SCHEMA_FUNDAMENTALS: &str = "idx.fundamentals.v1";
pub const SCHEMA_COMPARE: &str = "idx.compare.v1";
pub const SCHEMA_FINANCIALS: &str = "idx.financials.v1";
pub const SCHEMA_EARNINGS: &str = "idx.earnings.v1";
pub const SCHEMA_DIVIDENDS: &str = "idx.dividends.v1";
pub const SCHEMA_RETURNS: &str = "idx.returns.v1";
pub const SCHEMA_MARKET: &str = "idx.market.v1";
pub const SCHEMA_BRIEF: &str = "idx.brief.v1";
pub const SCHEMA_BROKER: &str = "idx.broker.v1";
pub const SCHEMA_BANDAR: &str = "idx.bandar.v1";
pub const SCHEMA_BANDAR_SCREEN: &str = "idx.bandar.screen.v1";
pub const SCHEMA_BANDAR_BROKER: &str = "idx.bandar.broker.v1";
pub const SCHEMA_WATCHLIST: &str = "idx.watchlist.v1";
pub const SCHEMA_HOLIDAYS: &str = "idx.holidays.v1";
pub const SCHEMA_INSTRUMENTS: &str = "idx.instruments.v1";
pub const SCHEMA_MEMBERS: &str = "idx.members.v1";
pub const SCHEMA_INDICES: &str = "idx.indices.v1";
pub const SCHEMA_CALENDAR: &str = "idx.calendar.v1";
pub const SCHEMA_ACTIONS: &str = "idx.actions.v1";
pub const SCHEMA_FOREIGN: &str = "idx.foreign.v1";
pub const SCHEMA_MARKET_FOREIGN: &str = "idx.market.foreign.v1";
pub const SCHEMA_MARKET_BROKERS: &str = "idx.market.brokers.v1";
pub const SCHEMA_TECHNICAL: &str = "idx.technical.v1";
pub const SCHEMA_ORDERBOOK: &str = "idx.orderbook.v1";
pub const SCHEMA_SCREEN: &str = "idx.screen.v1";
pub const SCHEMA_NEWS: &str = "idx.news.v1";
pub const SCHEMA_SENTIMENT: &str = "idx.sentiment.v1";
pub const SCHEMA_OWNERSHIP_HOLDERS: &str = "idx.ownership.holders.v1";
pub const SCHEMA_OWNERSHIP_ENTITY: &str = "idx.ownership.entity.v1";
pub const SCHEMA_OWNERSHIP_CHANGES: &str = "idx.ownership.changes.v1";
pub const SCHEMA_OWNERSHIP_RELEASES: &str = "idx.ownership.releases.v1";
pub const SCHEMA_OWNERSHIP_FOREIGN: &str = "idx.ownership.foreign.v1";
pub const SCHEMA_OWNERSHIP_FOREIGN_FLOW: &str = "idx.ownership.foreign-flow.v1";
pub const SCHEMA_OWNERSHIP_IMPORT: &str = "idx.ownership.import.v1";
pub const SCHEMA_OWNERSHIP_DISCOVER: &str = "idx.ownership.discover.v1";
pub const SCHEMA_OWNERSHIP_SEARCH: &str = "idx.ownership.search.v1";
pub const SCHEMA_OWNERSHIP_CROSS_HOLDERS: &str = "idx.ownership.cross-holders.v1";
pub const SCHEMA_OWNERSHIP_CONCENTRATION: &str = "idx.ownership.concentration.v1";
pub const SCHEMA_OWNERSHIP_GRAPH: &str = "idx.ownership.graph.v1";
pub const SCHEMA_OWNERSHIP_RESOLVE_CANDIDATES: &str = "idx.ownership.resolve-candidates.v1";
pub const SCHEMA_OWNERSHIP_RESOLUTION: &str = "idx.ownership.resolution.v1";
pub const SCHEMA_OWNERSHIP_RESOLUTIONS: &str = "idx.ownership.resolutions.v1";
pub const SCHEMA_OWNERSHIP_COMPOSITION: &str = "idx.ownership.composition.v1";
/// `ownership sync`. Not `idx.ownership.sync.v1`: the web terminal's
/// `/api/ownership/status` already answers with that name (its D1 loader).
pub const SCHEMA_OWNERSHIP_SNAPSHOT_SYNC: &str = "idx.ownership.snapshot-sync.v1";
pub const SCHEMA_OWNERSHIP_STATUS: &str = "idx.ownership.status.v1";
/// The snapshot manifest (`snapshot-v{V}.json` on the mirror, and the data
/// of `ownership build-snapshot -o json`).
pub const SCHEMA_OWNERSHIP_SNAPSHOT: &str = "idx.ownership.snapshot.v1";

pub type QuotesEnvelope = Envelope<Vec<Quote>>;
pub type HistoryEnvelope = Envelope<History>;
pub type ProfileEnvelope = Envelope<Profile>;
pub type FundamentalsEnvelope = Envelope<Fundamentals>;
pub type CompareEnvelope = Envelope<Vec<Fundamentals>>;
pub type FinancialsEnvelope = Envelope<Financials>;
pub type EarningsEnvelope = Envelope<Earnings>;
pub type DividendsEnvelope = Envelope<DividendHistory>;
pub type ReturnsEnvelope = Envelope<Vec<Performance>>;
pub type MarketEnvelope = Envelope<MarketOverview>;
pub type BriefEnvelope = Envelope<Brief>;
pub type BrokerEnvelope = Envelope<Vec<BrokerSummary>>;
pub type BandarEnvelope = Envelope<Vec<BandarReport>>;
pub type BandarScreenEnvelope = Envelope<BandarScreen>;
pub type BandarBrokerEnvelope = Envelope<BrokerActivity>;
pub type WatchlistEnvelope = Envelope<Watchlist>;
pub type HolidaysEnvelope = Envelope<Holidays>;
pub type InstrumentsEnvelope = Envelope<InstrumentList>;
pub type MembersEnvelope = Envelope<MemberList>;
pub type IndicesEnvelope = Envelope<IndexBoard>;
pub type CalendarEnvelope = Envelope<Calendar>;
pub type ActionsEnvelope = Envelope<CorporateActions>;
pub type ForeignEnvelope = Envelope<ForeignFlow>;
pub type MarketForeignEnvelope = Envelope<MarketForeign>;
pub type MarketBrokersEnvelope = Envelope<BrokerRanking>;
pub type TechnicalEnvelope = Envelope<Technical>;
pub type OrderBookEnvelope = Envelope<OrderBook>;
pub type ScreenEnvelope = Envelope<ScreenReport>;
pub type NewsEnvelope = Envelope<NewsReport>;
pub type SentimentEnvelope = Envelope<SentimentReport>;
pub type OwnershipHoldersEnvelope = Envelope<HoldersReport>;
pub type OwnershipEntityEnvelope = Envelope<EntityReport>;
pub type OwnershipChangesEnvelope = Envelope<ChangesReport>;
pub type OwnershipReleasesEnvelope = Envelope<Vec<Release>>;
pub type OwnershipForeignEnvelope = Envelope<ForeignReport>;
pub type OwnershipForeignFlowEnvelope = Envelope<ForeignFlowReport>;
pub type OwnershipImportEnvelope = Envelope<Vec<ImportReport>>;
pub type OwnershipDiscoverEnvelope = Envelope<Vec<DiscoveredFile>>;
pub type OwnershipSearchEnvelope = Envelope<SearchReport>;
pub type OwnershipCrossHoldersEnvelope = Envelope<CrossHoldersReport>;
pub type OwnershipConcentrationEnvelope = Envelope<ConcentrationReport>;
pub type OwnershipGraphEnvelope = Envelope<GraphReport>;
pub type OwnershipResolveCandidatesEnvelope = Envelope<ResolveCandidates>;
pub type OwnershipResolutionEnvelope = Envelope<Vec<ResolutionReport>>;
pub type OwnershipResolutionsEnvelope = Envelope<Vec<Resolution>>;
pub type OwnershipCompositionEnvelope = Envelope<CompositionReport>;
pub type OwnershipSnapshotSyncEnvelope = Envelope<SyncReport>;
pub type OwnershipStatusEnvelope = Envelope<StatusReport>;
pub type OwnershipSnapshotEnvelope = Envelope<SnapshotManifest>;

/// Error shape written to stderr in JSON mode.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ErrorBody {
    /// `usage` | `config` | `network` | `provider` | `blocked` | `not_found` | `offline_miss` | `unsupported` | `io` | `internal`.
    /// `blocked` (exit 3): the source's bot protection (Cloudflare) refused this network.
    pub code: String,
    pub message: String,
}

impl From<&IdxError> for ErrorEnvelope {
    fn from(e: &IdxError) -> Self {
        ErrorEnvelope {
            error: ErrorBody {
                code: e.code().to_string(),
                message: e.to_string(),
            },
            exit_code: e.exit_code(),
        }
    }
}

pub fn to_pretty<T: Serialize>(v: &T) -> Result<String> {
    let mut s = serde_json::to_string_pretty(v)?;
    s.push('\n');
    Ok(s)
}

/// Names accepted by `idx schema <name>` and their JSON Schema documents.
pub fn schema_names() -> &'static [&'static str] {
    &[
        "quotes",
        "history",
        "profile",
        "fundamentals",
        "compare",
        "financials",
        "earnings",
        "dividends",
        "returns",
        "market",
        "brief",
        "broker",
        "bandar",
        "bandar-screen",
        "bandar-broker",
        "watchlist",
        "holidays",
        "instruments",
        "members",
        "indices",
        "calendar",
        "actions",
        "foreign",
        "market-foreign",
        "market-brokers",
        "technical",
        "orderbook",
        "screen",
        "news",
        "sentiment",
        "ownership-holders",
        "ownership-entity",
        "ownership-changes",
        "ownership-releases",
        "ownership-foreign",
        "ownership-foreign-flow",
        "ownership-import",
        "ownership-discover",
        "ownership-search",
        "ownership-cross-holders",
        "ownership-concentration",
        "ownership-graph",
        "ownership-resolve-candidates",
        "ownership-resolution",
        "ownership-resolutions",
        "ownership-composition",
        "ownership-snapshot-sync",
        "ownership-status",
        "ownership-snapshot",
        "error",
        "quote",
        "candle",
    ]
}

pub fn schema_for(name: &str) -> Result<serde_json::Value> {
    let root = match name {
        "quotes" => schemars::schema_for!(QuotesEnvelope),
        "history" => schemars::schema_for!(HistoryEnvelope),
        "profile" => schemars::schema_for!(ProfileEnvelope),
        "fundamentals" => schemars::schema_for!(FundamentalsEnvelope),
        "compare" => schemars::schema_for!(CompareEnvelope),
        "financials" => schemars::schema_for!(FinancialsEnvelope),
        "earnings" => schemars::schema_for!(EarningsEnvelope),
        "dividends" => schemars::schema_for!(DividendsEnvelope),
        "returns" => schemars::schema_for!(ReturnsEnvelope),
        "market" => schemars::schema_for!(MarketEnvelope),
        "brief" => schemars::schema_for!(BriefEnvelope),
        "broker" => schemars::schema_for!(BrokerEnvelope),
        "bandar" => schemars::schema_for!(BandarEnvelope),
        "bandar-screen" => schemars::schema_for!(BandarScreenEnvelope),
        "bandar-broker" => schemars::schema_for!(BandarBrokerEnvelope),
        "watchlist" => schemars::schema_for!(WatchlistEnvelope),
        "holidays" => schemars::schema_for!(HolidaysEnvelope),
        "instruments" => schemars::schema_for!(InstrumentsEnvelope),
        "members" => schemars::schema_for!(MembersEnvelope),
        "indices" => schemars::schema_for!(IndicesEnvelope),
        "calendar" => schemars::schema_for!(CalendarEnvelope),
        "actions" => schemars::schema_for!(ActionsEnvelope),
        "foreign" => schemars::schema_for!(ForeignEnvelope),
        "market-foreign" => schemars::schema_for!(MarketForeignEnvelope),
        "market-brokers" => schemars::schema_for!(MarketBrokersEnvelope),
        "technical" => schemars::schema_for!(TechnicalEnvelope),
        "orderbook" => schemars::schema_for!(OrderBookEnvelope),
        "screen" => schemars::schema_for!(ScreenEnvelope),
        "news" => schemars::schema_for!(NewsEnvelope),
        "sentiment" => schemars::schema_for!(SentimentEnvelope),
        "ownership-holders" => schemars::schema_for!(OwnershipHoldersEnvelope),
        "ownership-entity" => schemars::schema_for!(OwnershipEntityEnvelope),
        "ownership-changes" => schemars::schema_for!(OwnershipChangesEnvelope),
        "ownership-releases" => schemars::schema_for!(OwnershipReleasesEnvelope),
        "ownership-foreign" => schemars::schema_for!(OwnershipForeignEnvelope),
        "ownership-foreign-flow" => schemars::schema_for!(OwnershipForeignFlowEnvelope),
        "ownership-import" => schemars::schema_for!(OwnershipImportEnvelope),
        "ownership-discover" => schemars::schema_for!(OwnershipDiscoverEnvelope),
        "ownership-search" => schemars::schema_for!(OwnershipSearchEnvelope),
        "ownership-cross-holders" => schemars::schema_for!(OwnershipCrossHoldersEnvelope),
        "ownership-concentration" => schemars::schema_for!(OwnershipConcentrationEnvelope),
        "ownership-graph" => schemars::schema_for!(OwnershipGraphEnvelope),
        "ownership-resolve-candidates" => {
            schemars::schema_for!(OwnershipResolveCandidatesEnvelope)
        }
        "ownership-resolution" => schemars::schema_for!(OwnershipResolutionEnvelope),
        "ownership-resolutions" => schemars::schema_for!(OwnershipResolutionsEnvelope),
        "ownership-composition" => schemars::schema_for!(OwnershipCompositionEnvelope),
        "ownership-snapshot-sync" => schemars::schema_for!(OwnershipSnapshotSyncEnvelope),
        "ownership-status" => schemars::schema_for!(OwnershipStatusEnvelope),
        "ownership-snapshot" => schemars::schema_for!(OwnershipSnapshotEnvelope),
        "error" => schemars::schema_for!(ErrorEnvelope),
        "quote" => schemars::schema_for!(Quote),
        "candle" => schemars::schema_for!(crate::model::Candle),
        _ => {
            return Err(IdxError::Usage(format!(
                "unknown schema {name:?}; available: {}",
                schema_names().join(", ")
            )))
        }
    };
    Ok(serde_json::to_value(root)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Symbol;

    #[test]
    fn envelope_shape() {
        let s = Sourced::fresh(
            vec![Quote::empty(Symbol::parse("BBCA").unwrap(), "t")],
            "yahoo",
        );
        let env = Envelope::from_sourced(SCHEMA_QUOTES, s);
        let v: serde_json::Value = serde_json::from_str(&to_pretty(&env).unwrap()).unwrap();
        assert_eq!(v["schema"], "idx.quotes.v1");
        assert_eq!(v["provider"], "yahoo");
        assert_eq!(v["cached"], false);
        assert!(v.get("warnings").is_none(), "empty warnings are omitted");
        assert!(v.get("stale_reason").is_none(), "fresh data has no reason");
        assert_eq!(v["data"][0]["symbol"], "BBCA");
        assert!(v["data"][0]["price"].is_null());
    }

    #[test]
    fn stale_envelope_says_why() {
        let mut s = Sourced::fresh(Vec::<Quote>::new(), "yahoo");
        s.cached = true;
        s.stale = true;
        s.stale_reason = Some(StaleReason::FetchFailed);
        s.warnings
            .push(crate::providers::fetch_failed_warning("connection refused"));
        let text = to_pretty(&Envelope::from_sourced(SCHEMA_QUOTES, s)).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["stale_reason"], "fetch_failed");
        assert_eq!(
            v["warnings"][0],
            "live fetch failed (connection refused); showing cached copy"
        );
        let stale = text.find("\"stale\"").unwrap();
        let reason = text.find("\"stale_reason\"").unwrap();
        assert!(stale < reason && reason < text.find("\"warnings\"").unwrap());
        let schema = schema_for("quotes").unwrap().to_string();
        assert!(schema.contains("stale_reason") && schema.contains("fetch_failed"));
    }

    #[test]
    fn schemas_exist_for_every_name() {
        for n in schema_names() {
            let s = schema_for(n).unwrap();
            assert!(s.get("$schema").is_some(), "{n}");
        }
        assert!(schema_for("nope").is_err());
    }

    #[test]
    fn error_envelope() {
        let e = IdxError::NotFound("ZZZZ".into());
        let env = ErrorEnvelope::from(&e);
        assert_eq!(env.exit_code, 4);
        assert_eq!(env.error.code, "not_found");
        let e = IdxError::blocked(
            "idx",
            crate::http::cloudflare::Refusal::Block,
            "www.idx.co.id answered with a Cloudflare block page (HTTP 403)",
        );
        let env = ErrorEnvelope::from(&e);
        assert_eq!((env.error.code.as_str(), env.exit_code), ("blocked", 3));
    }
}
