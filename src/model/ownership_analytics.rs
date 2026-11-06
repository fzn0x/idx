//! JSON contract of the ownership analytics over the local KSEI register:
//! holder search, holders across issuers, ownership concentration and the
//! holder-issuer network.
//!
//! Every figure is computed from the positions of one monthly release (a
//! holder's register lines in one issuer summed), on idx's 0-100 percent
//! scale, rounded to 4 decimals at the last step. Holders at or below 1 %
//! are not published, so every sum covers the listed holders only.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{IdxError, Result};

// ---- search -------------------------------------------------------------------

/// How a holder matched a search, strongest first.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SearchMatchKind {
    /// The query normalises to the holder's `entity_key`.
    Exact,
    /// The query is one of the spellings KSEI printed (case and spacing ignored).
    Spelling,
    /// The holder's key starts with the normalised query.
    Prefix,
    /// The normalised query appears inside the key or a normalised spelling.
    Contains,
    /// Every word of the query starts a word of the key or of a spelling, in any order.
    Words,
}

/// One holder found by `ownership search`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SearchMatch {
    /// Stable key of the holder: pass it to `ownership entity --exact` or `ownership graph`.
    pub entity_key: String,
    /// Display name of the holder.
    pub holder: String,
    pub matched: SearchMatchKind,
    /// Every spelling recorded for the holder in any imported release, sorted.
    pub spellings: Vec<String>,
    /// Issuers the holder owns more than 1 % of in `as_of` (0 when absent that month).
    pub issuers: usize,
    /// Sum of the holder's percentages over those issuers: a breadth measure,
    /// not a share of anything.
    pub total_pct: f64,
    /// From the holder's largest position in `as_of`; `null` when absent that month.
    pub investor_type: Option<String>,
    /// `L` (local) or `A` (foreign) from the largest position in `as_of`.
    pub local_foreign: Option<String>,
    /// Newest imported release that lists the holder.
    pub last_seen: NaiveDate,
}

/// `ownership search`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SearchReport {
    pub query: String,
    /// Release in which `issuers` and `total_pct` are counted.
    pub as_of: NaiveDate,
    /// Matches before `--limit`.
    pub total_matches: usize,
    /// Exact and spelling matches first, then holders present in `as_of`,
    /// then prefix, contains and word matches, then by issuers held.
    pub matches: Vec<SearchMatch>,
}

// ---- cross-holders ------------------------------------------------------------

/// KSEI's local/foreign flag as a filter.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum HolderOrigin {
    /// Flag `A` (asing).
    Foreign,
    /// Flag `L` (lokal).
    Local,
}

impl HolderOrigin {
    /// The KSEI flag this origin matches.
    pub fn flag(&self) -> &'static str {
        match self {
            HolderOrigin::Foreign => "A",
            HolderOrigin::Local => "L",
        }
    }
}

impl std::str::FromStr for HolderOrigin {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <HolderOrigin as clap::ValueEnum>::from_str(s.trim(), true)
            .map_err(|_| IdxError::Usage(format!("invalid origin {s:?}; use foreign or local")))
    }
}

/// One position of a cross-holder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CrossPosition {
    pub symbol: String,
    pub issuer_name: Option<String>,
    pub shares_total: i64,
    pub pct: f64,
}

/// A holder ranked by the number of issuers it owns more than 1 % of.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CrossHolder {
    pub holder: String,
    pub entity_key: String,
    /// From the holder's largest position.
    pub investor_type: Option<String>,
    /// `L` or `A` from the holder's largest position.
    pub local_foreign: Option<String>,
    /// Distinct issuers held above 1 %.
    pub issuers: usize,
    /// Sum of `pct` over those issuers: a breadth measure, not a share of anything.
    pub total_pct: f64,
    /// Largest single stake.
    pub max_pct: f64,
    /// Every position, largest first.
    pub positions: Vec<CrossPosition>,
}

/// `ownership cross-holders`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CrossHoldersReport {
    pub as_of: NaiveDate,
    pub min_issuers: usize,
    /// Investor-type filter applied (case-insensitive substring), if any.
    pub investor_type: Option<String>,
    /// Local/foreign filter applied, if any.
    pub origin: Option<HolderOrigin>,
    /// Holders that pass the filters, before `--limit`.
    pub total_matches: usize,
    /// Most issuers first, then the largest `total_pct`.
    pub holders: Vec<CrossHolder>,
}

// ---- concentration ------------------------------------------------------------

/// Ranking key of `ownership concentration`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
pub enum ConcentrationSort {
    /// Herfindahl-Hirschman index over the listed holders.
    Hhi,
    /// Largest single stake.
    Top1,
    /// Sum of the three largest stakes.
    Top3,
}

impl std::str::FromStr for ConcentrationSort {
    type Err = IdxError;
    fn from_str(s: &str) -> Result<Self> {
        <ConcentrationSort as clap::ValueEnum>::from_str(s.trim(), true)
            .map_err(|_| IdxError::Usage(format!("invalid ranking {s:?}; use hhi, top1 or top3")))
    }
}

/// Disclosed ownership concentration of one issuer in one release.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Concentration {
    pub symbol: String,
    pub issuer_name: Option<String>,
    /// Holders listed above 1 %, each holder's accounts summed.
    pub holders_listed: usize,
    /// Display name of the largest holder (ties: name ascending).
    pub top_holder: String,
    /// Largest single stake, percent.
    pub top1_pct: f64,
    /// Sum of the three largest stakes (fewer when fewer holders are listed).
    pub top3_pct: f64,
    /// Sum of the listed stakes.
    pub listed_pct: f64,
    /// `max(0, 100 - listed_pct)`: what the register does not show (holders
    /// at or below 1 %). Not IDX's regulatory free float.
    pub undisclosed_pct: f64,
    /// Herfindahl-Hirschman index: the sum of the squared stakes of the
    /// listed holders, 0 to 10,000 (10,000 = one holder owns everything).
    /// The undisclosed part is left out, so this is a lower bound.
    pub hhi: f64,
    /// Sum of the stakes flagged CONTROLLER or AFFILIATE OF THE CONTROLLER
    /// (registers from 30 Oct 2026); `null` when the release has no such
    /// column (see `controller_reported`).
    pub controller_pct: Option<f64>,
}

/// `ownership concentration`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConcentrationReport {
    pub as_of: NaiveDate,
    pub sort: ConcentrationSort,
    /// `true` when the least concentrated come first.
    pub ascending: bool,
    /// Issuers ranked (or found, with symbols), before `--limit`.
    pub total_matches: usize,
    /// Whether the release carries CONTROLLER_OR_AFFILIATE; when false every
    /// `controller_pct` is null for lack of data.
    pub controller_reported: bool,
    pub rows: Vec<Concentration>,
}

// ---- graph --------------------------------------------------------------------

/// An issuer in the ownership network.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphIssuer {
    /// `ticker:<SYMBOL>`.
    pub id: String,
    pub symbol: String,
    pub issuer_name: Option<String>,
    /// Hops from the root (0 for the root).
    pub hops: usize,
}

/// A holder in the ownership network.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphHolder {
    /// `holder:<entity_key>`, the same in every database.
    pub id: String,
    pub entity_key: String,
    pub holder: String,
    /// From the holder's largest position in the release.
    pub investor_type: Option<String>,
    pub local_foreign: Option<String>,
    pub hops: usize,
}

/// A holding above 1 %: an edge from a holder to an issuer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphLink {
    /// Holder node id.
    pub from: String,
    /// Issuer node id.
    pub to: String,
    pub holder: String,
    pub symbol: String,
    pub shares_total: i64,
    pub pct: f64,
    /// The larger hop count of its two ends.
    pub hops: usize,
}

/// `ownership graph`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GraphReport {
    pub as_of: NaiveDate,
    /// Node id the walk started from.
    pub root: String,
    pub depth: usize,
    /// Holdings below this percent were ignored.
    pub min_pct: f64,
    pub max_nodes: usize,
    /// `true` when `max_nodes` stopped the walk before `depth`.
    pub truncated: bool,
    /// In the order the walk reached them (strongest links first per hop).
    pub issuers: Vec<GraphIssuer>,
    pub holders: Vec<GraphHolder>,
    /// Every holding between two nodes of the walk, by hops, symbol, percent.
    pub links: Vec<GraphLink>,
}
