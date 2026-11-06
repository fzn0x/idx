//! KSEI shareholder register ("Pemegang Saham di Atas 1%") as published
//! monthly by IDX: discovery and download (`discover`), parsing (`ingest`),
//! name normalisation (`normalize`) and the local SQLite store (`db`).
//!
//! Unlike the quote providers, this data is not fetched on demand: a monthly
//! file is imported once and every query runs locally, so the ownership
//! commands work offline after the first import.
//!
//! The query wrappers below (`holders`, `entity`, `changes`, ...) are what
//! the CLI, the MCP server and `stocks brief` call: each returns the report
//! with the database's notes about the releases it read (imported by an old
//! idx, rows dated otherwise) in `warnings`.
//!
//! `analytics` ranks a release's positions (search, cross-holders,
//! concentration, the network graph, spelling candidates); the `resolve_*`
//! wrappers write the user's spelling rules (`ownership resolve`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDate;
use sha2::{Digest, Sha256};

use crate::error::{IdxError, Result};
use crate::model::{
    ChangesReport, ConcentrationReport, ConcentrationSort, CrossHoldersReport, DiscoveredFile,
    EntityReport, ForeignFlowReport, ForeignReport, GraphReport, HoldersReport, ImportReport,
    Release, Resolution, ResolutionReport, ResolveCandidates, SearchReport, Symbol,
};
use crate::providers::Sourced;

pub mod analytics;
pub mod db;
pub mod discover;
pub mod ingest;
pub mod mirror;
pub mod normalize;

pub use db::{OwnershipDb, SCHEMA_VERSION};

/// The `provider` of every ownership query envelope.
pub const PROVIDER: &str = "local-db";

/// Default location of the SQLite file.
pub fn default_db_path() -> PathBuf {
    let base = if cfg!(windows) {
        dirs::data_local_dir()
    } else {
        dirs::data_dir()
    };
    base.unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("idx")
        .join("ownership.sqlite")
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Parses `bytes` (XLSX or PDF) and stores them. `name` is the file name or
/// URL used for provenance and format detection.
pub fn import_bytes(
    db: &mut OwnershipDb,
    name: &str,
    source: &str,
    bytes: &[u8],
) -> Result<(ImportReport, Vec<String>)> {
    let (parsed, format) = ingest::parse_any(name, bytes)?;
    let report = db.import(&parsed, format, source, &sha256_hex(bytes))?;
    let mut warnings = parsed.skipped.clone();
    warnings.extend(parsed.other_date_warnings());
    Ok((report, warnings))
}

fn sourced<T>(db: &OwnershipDb, data: T, dates: &[NaiveDate]) -> Result<Sourced<T>> {
    let mut s = Sourced::fresh(data, PROVIDER);
    s.warnings = db.release_notes(dates)?;
    Ok(s)
}

/// `ownership ticker`: holders above 1 % of one issuer.
pub fn holders(
    db: &OwnershipDb,
    symbol: &str,
    as_of: Option<NaiveDate>,
) -> Result<Sourced<HoldersReport>> {
    let r = db.holders(symbol, as_of)?;
    let date = r.as_of;
    sourced(db, r, &[date])
}

/// `ownership entity`: every position of the holders matching `name`
/// (with `exact`, of the one holder whose key it is).
pub fn entity(
    db: &OwnershipDb,
    name: &str,
    as_of: Option<NaiveDate>,
    exact: bool,
) -> Result<Sourced<EntityReport>> {
    let r = db.entity_matching(name, as_of, exact)?;
    let date = r.as_of;
    sourced(db, r, &[date])
}

/// `ownership changes` between two releases.
pub fn changes(
    db: &OwnershipDb,
    from: NaiveDate,
    to: NaiveDate,
    symbol: Option<&str>,
    min_pct_change: f64,
    limit: Option<usize>,
    offset: usize,
) -> Result<Sourced<ChangesReport>> {
    let r = db.changes(from, to, symbol, min_pct_change, limit, offset)?;
    sourced(db, r, &[from, to])
}

/// `ownership foreign SYMBOL`: one issuer across every release.
pub fn foreign_trend(db: &OwnershipDb, symbol: &str) -> Result<Sourced<ForeignReport>> {
    let r = db.foreign_trend(symbol)?;
    let dates: Vec<NaiveDate> = r.points.iter().map(|p| p.as_of).collect();
    sourced(db, r, &dates)
}

/// `ownership foreign`: every issuer between two releases.
pub fn foreign_flow(
    db: &OwnershipDb,
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
    limit: Option<usize>,
) -> Result<Sourced<ForeignFlowReport>> {
    let r = db.foreign_flow(from, to, limit)?;
    let dates = [r.from, r.to];
    sourced(db, r, &dates)
}

/// `ownership releases`.
pub fn releases(db: &OwnershipDb) -> Result<Sourced<Vec<Release>>> {
    let list = db.releases()?;
    let dates: Vec<NaiveDate> = list.iter().map(|r| r.as_of).collect();
    let mut s = sourced(db, list, &dates)?;
    if s.data.is_empty() {
        s.warnings
            .push("no ownership data imported yet; run `idx ownership import --latest`".into());
    }
    Ok(s)
}

fn with_notes<T>(
    db: &OwnershipDb,
    data: T,
    dates: &[NaiveDate],
    warnings: Vec<String>,
) -> Result<Sourced<T>> {
    let mut s = sourced(db, data, dates)?;
    s.warnings.extend(warnings);
    Ok(s)
}

/// `ownership search`: holders of every release matching `query`.
pub fn search(
    db: &OwnershipDb,
    query: &str,
    as_of: Option<NaiveDate>,
    limit: Option<usize>,
) -> Result<Sourced<SearchReport>> {
    let (r, w) = db.search(query, as_of, limit)?;
    let date = r.as_of;
    with_notes(db, r, &[date], w)
}

/// `ownership cross-holders`: holders ranked by issuers held.
pub fn cross_holders(
    db: &OwnershipDb,
    filter: &analytics::CrossFilter,
    limit: Option<usize>,
    as_of: Option<NaiveDate>,
) -> Result<Sourced<CrossHoldersReport>> {
    let r = db.cross_holders(filter, limit, as_of)?;
    let date = r.as_of;
    sourced(db, r, &[date])
}

/// `ownership concentration`: issuers ranked by HHI, top-1 or top-3 stake.
pub fn concentration(
    db: &OwnershipDb,
    symbols: Option<&[Symbol]>,
    by: ConcentrationSort,
    ascending: bool,
    limit: Option<usize>,
    as_of: Option<NaiveDate>,
) -> Result<Sourced<ConcentrationReport>> {
    let (r, w) = db.concentration(symbols, by, ascending, limit, as_of)?;
    let date = r.as_of;
    with_notes(db, r, &[date], w)
}

/// `ownership graph`: the network around a ticker or holder.
pub fn graph(
    db: &OwnershipDb,
    root: &str,
    force_holder: bool,
    depth: usize,
    min_pct: f64,
    max_nodes: usize,
    as_of: Option<NaiveDate>,
) -> Result<Sourced<GraphReport>> {
    let (r, w) = db.graph(root, force_holder, depth, min_pct, max_nodes, as_of)?;
    let date = r.as_of;
    with_notes(db, r, &[date], w)
}

/// `ownership resolve candidates`.
pub fn resolve_candidates(
    db: &OwnershipDb,
    limit: Option<usize>,
) -> Result<Sourced<ResolveCandidates>> {
    let r = db.resolve_candidates(limit)?;
    let dates: Vec<NaiveDate> = db.releases()?.iter().map(|r| r.as_of).collect();
    sourced(db, r, &dates)
}

/// `ownership resolve merge`.
pub fn resolve_merge(
    db: &mut OwnershipDb,
    keep: &str,
    merge: &[String],
    fold: bool,
    dry_run: bool,
) -> Result<Sourced<Vec<ResolutionReport>>> {
    let (r, w) = db.resolve_merge(keep, merge, fold, dry_run)?;
    let mut s = Sourced::fresh(r, PROVIDER);
    s.warnings = w;
    Ok(s)
}

/// `ownership resolve map`.
pub fn resolve_map(
    db: &mut OwnershipDb,
    spelling: &str,
    holder: &str,
    new: bool,
    fold: bool,
    dry_run: bool,
) -> Result<Sourced<Vec<ResolutionReport>>> {
    let (r, w) = db.resolve_map(spelling, holder, new, fold, dry_run)?;
    let mut s = Sourced::fresh(vec![r], PROVIDER);
    s.warnings = w;
    Ok(s)
}

/// `ownership resolve undo`.
pub fn resolve_undo(
    db: &mut OwnershipDb,
    id: i64,
    dry_run: bool,
) -> Result<Sourced<Vec<ResolutionReport>>> {
    let (r, w) = db.resolve_undo(id, dry_run)?;
    let mut s = Sourced::fresh(vec![r], PROVIDER);
    s.warnings = w;
    Ok(s)
}

/// `ownership resolve list`.
pub fn resolutions(db: &OwnershipDb) -> Result<Sourced<Vec<Resolution>>> {
    Ok(Sourced::fresh(db.resolutions()?, PROVIDER))
}

pub fn import_file(db: &mut OwnershipDb, path: &Path) -> Result<(ImportReport, Vec<String>)> {
    let bytes = std::fs::read(path)
        .map_err(|e| IdxError::Usage(format!("cannot read {}: {e}", path.display())))?;
    let name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    import_bytes(db, &name, &path.display().to_string(), &bytes)
}

pub async fn import_url(
    db: &mut OwnershipDb,
    url: &str,
    timeout: Duration,
) -> Result<(ImportReport, Vec<String>)> {
    let bytes = discover::fetch_browser(url, timeout).await?;
    import_bytes(db, &discover::basename(url), url, &bytes)
}

/// Downloads and imports every discovered file that is not in the database
/// yet (or only the newest when `latest_only`). Returns the reports in the
/// order imported plus any parser warnings.
pub async fn import_discovered(
    db: &mut OwnershipDb,
    files: &[DiscoveredFile],
    latest_only: bool,
    force: bool,
    timeout: Duration,
) -> Result<(Vec<ImportReport>, Vec<String>)> {
    let have: Vec<chrono::NaiveDate> = db.releases()?.iter().map(|r| r.as_of).collect();
    let mut reports = Vec::new();
    let mut warnings = Vec::new();
    for f in files.iter().filter(|f| force || !have.contains(&f.as_of)) {
        tracing::info!(as_of = %f.as_of, url = %f.url, origin = %f.origin, "importing ownership file");
        let (report, skipped) = if f.origin == mirror::ORIGIN {
            let bytes = mirror::download(&f.url, f.sha256.as_deref(), timeout).await?;
            import_bytes(db, &discover::basename(&f.url), &f.url, &bytes)?
        } else {
            import_url(db, &f.url, timeout).await?
        };
        warnings.extend(skipped.into_iter().map(|s| format!("{}: {s}", f.as_of)));
        reports.push(report);
        if latest_only {
            break;
        }
    }
    Ok((reports, warnings))
}
