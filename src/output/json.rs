//! The JSON envelope every command emits with `-o json`.

use chrono::{DateTime, SubsecRound, Utc};
use schemars::JsonSchema;
use serde::Serialize;

use crate::error::{IdxError, Result};
use crate::model::{
    BandarReport, BandarScreen, Brief, BrokerSummary, Calendar, ChangesReport, CorporateActions,
    DiscoveredFile, DividendHistory, Earnings, EntityReport, Financials, ForeignFlowReport,
    ForeignReport, Fundamentals, History, HoldersReport, Holidays, ImportReport, IndexBoard,
    InstrumentList, Intraday, MarketOverview, MemberList, NewsReport, OrderBook, Performance,
    Profile, Quote, Release, ScreenReport, SentimentReport, Technical, Watchlist,
};
use crate::model::{BrokerActivity, BrokerRanking, ForeignFlow, MarketForeign};
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
            warnings: s.warnings,
            data: s.data,
        }
    }
}

pub const SCHEMA_QUOTES: &str = "idx.quotes.v1";
pub const SCHEMA_HISTORY: &str = "idx.history.v1";
pub const SCHEMA_INTRADAY: &str = "idx.intraday.v1";
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

pub type QuotesEnvelope = Envelope<Vec<Quote>>;
pub type HistoryEnvelope = Envelope<History>;
pub type IntradayEnvelope = Envelope<Intraday>;
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

/// Error shape written to stderr in JSON mode.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
    pub exit_code: i32,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ErrorBody {
    /// `usage` | `config` | `network` | `provider` | `not_found` | `offline_miss` | `io` | `internal`
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
        "intraday",
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
        "error",
        "quote",
        "candle",
    ]
}

pub fn schema_for(name: &str) -> Result<serde_json::Value> {
    let root = match name {
        "quotes" => schemars::schema_for!(QuotesEnvelope),
        "history" => schemars::schema_for!(HistoryEnvelope),
        "intraday" => schemars::schema_for!(IntradayEnvelope),
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
        assert_eq!(v["data"][0]["symbol"], "BBCA");
        assert!(v["data"][0]["price"].is_null());
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
    }
}
