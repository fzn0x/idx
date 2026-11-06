//! Shareholder-register types for `idx ownership ...`. The data is KSEI's
//! monthly "Pemegang Saham di Atas 1%" list as published by IDX, imported into
//! a local SQLite file. These structs are the JSON contract.

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One holder's position in one issuer on one date: the holder's register
/// lines (one per account) summed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Holding {
    pub as_of: NaiveDate,
    pub symbol: String,
    pub issuer_name: Option<String>,
    /// Normalised display name of the holder (see `entity_key`).
    pub holder: String,
    /// Name exactly as printed in the source file.
    pub holder_raw: String,
    /// Stable key that groups spelling variants of the same holder.
    pub entity_key: String,
    /// KSEI investor type, upper-case (`CORPORATE`, `INDIVIDUAL`, `MUTUAL FUND`, ...).
    pub investor_type: Option<String>,
    /// `L` (local) or `A` (foreign) as reported.
    pub local_foreign: Option<String>,
    pub nationality: Option<String>,
    pub domicile: Option<String>,
    pub shares_scripless: Option<i64>,
    pub shares_scrip: Option<i64>,
    pub shares_total: i64,
    /// Percentage of issued shares, e.g. `54.94`.
    pub pct: f64,
    /// Register lines summed into this position: KSEI lists a holder once
    /// per SID and once per scrip account without an SID, so one holder can
    /// have several lines in one issuer. `null` for releases imported by idx
    /// 0.8.1 or older, which kept only one of them.
    pub accounts: Option<u32>,
    /// `CONTROLLER` or `AFFILIATE OF THE CONTROLLER` as the issuer reported
    /// it to IDX (Form E009, previous month); `null` when blank or when the
    /// release has no such column (see `controller_reported`).
    pub controller_or_affiliate: Option<String>,
}

/// `ownership ticker`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HoldersReport {
    pub symbol: String,
    pub issuer_name: Option<String>,
    pub as_of: NaiveDate,
    /// Sum of `pct` over the listed holders (the rest is below 1 % each).
    pub listed_pct: f64,
    pub holders: Vec<Holding>,
    /// Whether this release carries CONTROLLER_OR_AFFILIATE (registers from
    /// 30 Oct 2026 imported by idx 0.8.2 or later). When false every
    /// `controller_or_affiliate` is null for lack of data.
    #[serde(default)]
    pub controller_reported: bool,
}

/// `ownership entity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EntityReport {
    pub query: String,
    pub as_of: NaiveDate,
    /// Distinct entity keys that matched the query.
    pub entities: Vec<String>,
    pub holdings: Vec<Holding>,
    /// Whether this release carries CONTROLLER_OR_AFFILIATE; see
    /// `HoldersReport::controller_reported`.
    #[serde(default)]
    pub controller_reported: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Present in both releases with a different position.
    Changed,
    /// Absent from the earlier release (crossed above 1 % or newly listed).
    New,
    /// Absent from the later release (dropped below 1 % or sold out).
    Exited,
}

/// One holder's movement between two releases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Change {
    pub symbol: String,
    pub holder: String,
    pub entity_key: String,
    pub investor_type: Option<String>,
    /// `L` (local) or `A` (foreign) in the later release (earlier one for exits).
    pub local_foreign: Option<String>,
    pub kind: ChangeKind,
    pub pct_from: Option<f64>,
    pub pct_to: Option<f64>,
    /// `pct_to - pct_from` in percentage points (missing side counts as 0).
    pub pct_change: f64,
    pub shares_from: Option<i64>,
    pub shares_to: Option<i64>,
    pub shares_change: i64,
    /// Register lines summed on each side (`null` when the holder is absent
    /// there, or the release was imported by idx 0.8.1 or older).
    pub accounts_from: Option<u32>,
    pub accounts_to: Option<u32>,
    /// From the later release (the earlier one for exits), like
    /// `local_foreign`.
    pub controller_or_affiliate: Option<String>,
}

/// A change reduced to what a summary needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChangeBrief {
    pub symbol: String,
    pub holder: String,
    pub kind: ChangeKind,
    pub pct_from: Option<f64>,
    pub pct_to: Option<f64>,
    pub pct_change: f64,
}

/// Net movement of one KSEI investor type.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TypeFlow {
    /// `CORPORATE`, `INDIVIDUAL`, `MUTUAL FUND`, ... or `UNKNOWN`.
    pub investor_type: String,
    /// Rows of this type among the matches.
    pub rows: usize,
    /// Sum of `pct_change` over those rows, percentage points.
    pub net_pp: f64,
}

/// Net movement within one issuer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IssuerFlow {
    pub symbol: String,
    pub rows: usize,
    /// Sum of `pct_change` over the issuer's matching rows.
    pub net_pp: f64,
    /// Part of `net_pp` from holders flagged foreign (`A`).
    pub foreign_net_pp: f64,
}

/// Aggregates over every match (before paging), so an agent can describe a
/// month without reading the rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChangesSummary {
    /// Distinct issuers with at least one match.
    pub issuers: usize,
    /// Distinct holders (entity keys) with at least one match.
    pub holders: usize,
    pub new_holders: usize,
    pub exits: usize,
    /// Existing holders whose stake rose / fell.
    pub increased: usize,
    pub decreased: usize,
    /// Sum of `pct_change` over all matches, percentage points. Across many
    /// issuers this is a register-wide flow indicator, not a share of any one
    /// company; for a single symbol it is exact.
    pub net_pp: f64,
    pub foreign_net_pp: f64,
    pub local_net_pp: f64,
    /// Sorted by absolute `net_pp`.
    pub by_investor_type: Vec<TypeFlow>,
    /// Up to 10 issuers with the largest absolute `net_pp`.
    pub issuer_flow: Vec<IssuerFlow>,
    /// Up to 5 largest rises / falls.
    pub top_increases: Vec<ChangeBrief>,
    pub top_decreases: Vec<ChangeBrief>,
}

/// `ownership changes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ChangesReport {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub symbol: Option<String>,
    /// Minimum absolute change in percentage points that was applied.
    pub min_pct_change: f64,
    /// Matches after `min_pct_change`, before paging.
    pub total_matches: usize,
    /// Rows skipped before `changes` (`--offset`); `changes.len()` is the page size.
    pub offset: usize,
    /// Computed over every match, not only the page.
    pub summary: ChangesSummary,
    /// Largest absolute moves first; one page of `total_matches`.
    pub changes: Vec<Change>,
}

/// Foreign versus local share of the >1 % register for one issuer on one date.
/// Percentages are sums over the listed holders only (holders below 1 % are
/// not published), so this is the disclosed foreign stake, not total foreign
/// ownership.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignPoint {
    pub as_of: NaiveDate,
    /// Sum of `pct` over holders flagged `A`.
    pub foreign_pct: f64,
    /// Sum over holders flagged `L`.
    pub local_pct: f64,
    /// Sum over holders with no flag in the source.
    pub unflagged_pct: f64,
    pub foreign_holders: usize,
    pub local_holders: usize,
    /// Sum over every listed holder.
    pub listed_pct: f64,
    /// `foreign_pct` minus the previous point's, percentage points; `null` for the first.
    pub foreign_change_pp: Option<f64>,
}

/// `ownership foreign SYMBOL`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignReport {
    pub symbol: String,
    pub issuer_name: Option<String>,
    /// Oldest first, one per imported release that lists the issuer.
    pub points: Vec<ForeignPoint>,
}

/// One issuer's disclosed foreign stake between two releases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IssuerForeign {
    pub symbol: String,
    pub issuer_name: Option<String>,
    /// `null` when the issuer was not in the earlier release.
    pub foreign_pct_from: Option<f64>,
    pub foreign_pct_to: f64,
    /// `foreign_pct_to - foreign_pct_from` (missing side counts as 0).
    pub change_pp: f64,
    pub foreign_holders_to: usize,
}

/// `ownership foreign` without a symbol: where disclosed foreign holdings
/// moved between two releases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ForeignFlowReport {
    pub from: NaiveDate,
    pub to: NaiveDate,
    /// Issuers present in the later release.
    pub issuers: usize,
    pub inflows: usize,
    pub outflows: usize,
    /// Sum of `change_pp` over all issuers (register-wide indicator).
    pub net_pp: f64,
    /// Largest absolute moves first; may be truncated by `--limit`.
    pub rows: Vec<IssuerForeign>,
}

/// One imported monthly file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Release {
    pub as_of: NaiveDate,
    /// `xlsx` or `pdf`.
    pub format: String,
    /// File path or URL the rows came from.
    pub source: String,
    pub sha256: String,
    /// Register lines stored (every line of the file that parsed).
    pub rows: u64,
    pub issuers: u64,
    pub entities: u64,
    pub imported_at: String,
    /// Holder positions after summing each holder's accounts; `null` for
    /// releases imported by idx 0.8.1 or older (one line kept per holder).
    pub positions: Option<u64>,
    /// Lines whose DATE differs from `as_of`: stored, but not part of this
    /// release's holdings. `null` for releases imported by idx 0.8.1 or older.
    pub other_date_rows: Option<u64>,
    /// Whether the file has the CONTROLLER_OR_AFFILIATE column; `null` for
    /// releases imported by idx 0.8.1 or older.
    pub controller_column: Option<bool>,
}

/// `ownership import`.
///
/// For a KSEI composition archive (`family` = `composition`) `release`
/// describes the month stored in the composition tables: `rows` and
/// `issuers` count the stocks (EQUITY rows), `entities` is 0 (the archive
/// names no holders) and `positions`, `other_date_rows` and
/// `controller_column` are `null` (they describe the register only).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImportReport {
    pub release: Release,
    /// `true` when this `as_of` already existed and was replaced.
    pub replaced: bool,
    pub database: String,
    /// `above1` (the register of holders above 1 %) or `composition` (KSEI's
    /// "Kepemilikan Efek" archive), detected from the file's content.
    #[serde(default = "default_family")]
    pub family: String,
    /// Composition archives only: rows of other security types (bonds,
    /// warrants, rights, funds, ...) left out; `null` for the register.
    #[serde(default)]
    pub other_securities: Option<u64>,
}

/// A downloadable file found on IDX's "Data Kepemilikan Saham" page, the
/// GitHub mirror or KSEI's "Kepemilikan Efek" page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiscoveredFile {
    pub as_of: NaiveDate,
    pub title: String,
    /// Where the file is downloaded from (idx.co.id, the GitHub mirror or
    /// ksei.co.id).
    pub url: String,
    pub format: String,
    /// `idx.co.id`, `github-mirror` or `ksei.co.id`.
    #[serde(default = "default_origin")]
    pub origin: String,
    /// SHA-256 of the file when the listing provides it (mirror index).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// The file's original address on idx.co.id when `url` is a mirror copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    /// `above1` (holders above 1 %), `above5` (holders above 5 %, a daily
    /// register), `investor-type` and `classification` (IDX's summaries per
    /// investor type and classification), all on idx.co.id, or
    /// `composition` (KSEI's monthly "Kepemilikan Efek" archive).
    #[serde(default = "default_family")]
    pub family: String,
    /// Whether `idx ownership import` can read this file (`above1` and
    /// `composition`); the other families are listed only.
    #[serde(default = "default_importable")]
    pub importable: bool,
}

fn default_origin() -> String {
    "idx.co.id".to_string()
}

fn default_family() -> String {
    "above1".to_string()
}

fn default_importable() -> bool {
    true
}
