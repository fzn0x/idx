//! SQLite storage for the shareholder register.
//!
//! Schema (one file, WAL mode, `PRAGMA user_version` = [`SCHEMA_VERSION`]):
//! - `releases`       one row per imported monthly file, unique on `as_of`
//! - `entities`       one row per normalised holder key, with the display name
//! - `aliases`        every raw spelling seen, pointing at its entity
//! - `holding_lines`  every register line as printed, keyed by (release, sheet
//!   row), with the line's own `report_date` and controller flag: lossless
//! - `holdings`       positions: one row per (release, symbol, entity), the
//!   holder's lines dated the release's `as_of` summed (`accounts` counts
//!   them). [`rebuild_positions`] is the only writer for releases that have
//!   lines; every query reads positions.
//!
//! KSEI lists a holder once per SID and once per scrip account without an
//! SID, so one holder can have several lines in one issuer (typically a scrip
//! and a scripless line spelled differently). Releases imported by idx 0.8.1
//! or older kept one of them: they have no lines, `positions IS NULL`, and
//! [`OwnershipDb::release_notes`] names them until they are re-imported.
//!
//! Queries never touch the network. Importing the same `as_of` twice replaces
//! the earlier rows for that release.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use crate::error::{IdxError, Result};
use crate::model::{
    Change, ChangeBrief, ChangeKind, ChangesReport, ChangesSummary, EntityReport,
    ForeignFlowReport, ForeignPoint, ForeignReport, HoldersReport, Holding, ImportReport,
    IssuerFlow, IssuerForeign, Release, TypeFlow,
};

use super::ingest::{self, Parsed};
use super::normalize;

mod analytics;

/// `PRAGMA user_version` this idx writes. 0 is a file created by idx 0.8.1
/// or older (the pragma was never set) or a brand-new file; 2 adds register
/// lines and summed positions. A later schema change adds an `if v < 3`
/// step to `migrate` and bumps this.
pub const SCHEMA_VERSION: i64 = 2;

/// The idx 0.8.1 tables, frozen. Every file is created from them and then
/// migrated, so new and upgraded files end up identical.
const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS releases (
  id          INTEGER PRIMARY KEY,
  as_of       TEXT NOT NULL UNIQUE,
  format      TEXT NOT NULL,
  source      TEXT NOT NULL,
  sha256      TEXT NOT NULL,
  row_count   INTEGER NOT NULL,
  imported_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS entities (
  id           INTEGER PRIMARY KEY,
  entity_key   TEXT NOT NULL UNIQUE,
  display_name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS aliases (
  raw_name  TEXT PRIMARY KEY,
  entity_id INTEGER NOT NULL REFERENCES entities(id)
);
CREATE TABLE IF NOT EXISTS holdings (
  release_id       INTEGER NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
  symbol           TEXT NOT NULL,
  issuer_name      TEXT,
  entity_id        INTEGER NOT NULL REFERENCES entities(id),
  holder_raw       TEXT NOT NULL,
  investor_type    TEXT,
  local_foreign    TEXT,
  nationality      TEXT,
  domicile         TEXT,
  shares_scripless INTEGER,
  shares_scrip     INTEGER,
  shares_total     INTEGER NOT NULL,
  pct              REAL NOT NULL,
  PRIMARY KEY (release_id, symbol, entity_id)
);
CREATE INDEX IF NOT EXISTS holdings_symbol ON holdings(symbol, release_id);
CREATE INDEX IF NOT EXISTS holdings_entity ON holdings(entity_id, release_id);
";

/// Version 2: every register line, apart from the summed positions.
const V2_TABLES: &str = "
CREATE TABLE IF NOT EXISTS holding_lines (
  release_id       INTEGER NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
  line             INTEGER NOT NULL,   -- sheet row (XLSX, 1-based) or text line (PDF)
  report_date      TEXT NOT NULL,      -- the line's own DATE, YYYY-MM-DD
  symbol           TEXT NOT NULL,
  issuer_name      TEXT,
  entity_id        INTEGER NOT NULL REFERENCES entities(id),
  holder_raw       TEXT NOT NULL,
  investor_type    TEXT,
  controller_or_affiliate TEXT,
  local_foreign    TEXT,
  nationality      TEXT,
  domicile         TEXT,
  shares_scripless INTEGER,
  shares_scrip     INTEGER,
  shares_total     INTEGER NOT NULL,
  pct              REAL NOT NULL,
  PRIMARY KEY (release_id, line)
);
CREATE INDEX IF NOT EXISTS holding_lines_symbol ON holding_lines(release_id, symbol, entity_id);
CREATE INDEX IF NOT EXISTS holding_lines_entity ON holding_lines(entity_id, release_id);
";

/// Version 2 columns on the 0.8.1 tables, each added only when
/// `pragma_table_info` lacks it, so a half-migrated file finishes cleanly.
const V2_COLUMNS: &[(&str, &str, &str)] = &[
    // NULL = imported by idx <= 0.8.1 (one line kept per holder).
    ("holdings", "accounts", "INTEGER"),
    ("holdings", "controller_or_affiliate", "TEXT"),
    // NULL = a legacy release without lines.
    ("releases", "positions", "INTEGER"),
    ("releases", "other_date_rows", "INTEGER"),
    // 0/1, NULL = legacy.
    ("releases", "controller_column", "INTEGER"),
];

pub struct OwnershipDb {
    conn: Connection,
    path: PathBuf,
}

fn db_err(e: rusqlite::Error) -> IdxError {
    IdxError::Other(format!("ownership database: {e}"))
}

fn user_version(conn: &Connection) -> Result<i64> {
    conn.pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(db_err)
}

fn newer_schema(path: &Path, v: i64) -> IdxError {
    IdxError::Other(format!(
        "ownership database {} was written by a newer idx (schema {v}; this idx knows {SCHEMA_VERSION}); upgrade idx or point IDX_OWNERSHIP_DB at another file",
        path.display()
    ))
}

fn add_column(tx: &Transaction, table: &str, column: &str, ty: &str) -> Result<()> {
    let present: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
            [table, column],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if present == 0 {
        tx.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {ty};"))
            .map_err(db_err)?;
    }
    Ok(())
}

/// Brings the file to [`SCHEMA_VERSION`]. The common case (already current)
/// reads one pragma and takes no write lock; otherwise the steps run in one
/// `BEGIN IMMEDIATE` transaction that re-reads the version, so two processes
/// opening an old file at once migrate it once. A file from a newer idx is
/// refused rather than altered.
fn migrate(conn: &mut Connection, path: &Path) -> Result<()> {
    let v = user_version(conn)?;
    if v == SCHEMA_VERSION {
        return Ok(());
    }
    if v > SCHEMA_VERSION {
        return Err(newer_schema(path, v));
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    let v = user_version(&tx)?;
    if v > SCHEMA_VERSION {
        return Err(newer_schema(path, v));
    }
    if v < SCHEMA_VERSION {
        tx.execute_batch(SCHEMA_V1).map_err(db_err)?;
        if v < 2 {
            tx.execute_batch(V2_TABLES).map_err(db_err)?;
            for (table, column, ty) in V2_COLUMNS {
                add_column(&tx, table, column, ty)?;
            }
        }
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(db_err)?;
    }
    tx.commit().map_err(db_err)
}

/// One stored register line, as [`fold_accounts`] reads it.
#[derive(Debug, Clone, PartialEq)]
pub struct LineRow {
    pub line: u32,
    pub symbol: String,
    pub entity_id: i64,
    pub issuer_name: Option<String>,
    pub holder_raw: String,
    pub investor_type: Option<String>,
    pub controller_or_affiliate: Option<String>,
    pub local_foreign: Option<String>,
    pub nationality: Option<String>,
    pub domicile: Option<String>,
    pub shares_scripless: Option<i64>,
    pub shares_scrip: Option<i64>,
    pub shares_total: i64,
    pub pct: f64,
}

/// One holder's position in one issuer: its lines folded together.
#[derive(Debug, Clone, PartialEq)]
pub struct Position {
    pub symbol: String,
    pub entity_id: i64,
    pub issuer_name: Option<String>,
    pub holder_raw: String,
    pub investor_type: Option<String>,
    pub controller_or_affiliate: Option<String>,
    pub local_foreign: Option<String>,
    pub nationality: Option<String>,
    pub domicile: Option<String>,
    pub shares_scripless: Option<i64>,
    pub shares_scrip: Option<i64>,
    pub shares_total: i64,
    pub pct: f64,
    pub accounts: u32,
}

/// Folds one holder's lines in one issuer into a position (`None` for no
/// lines):
/// - `shares_total` and `pct` are summed (`pct` rounded to 4 decimals),
///   `shares_scripless` and `shares_scrip` sum the values present (`None`
///   when no line has one), `accounts` counts the lines;
/// - `holder_raw` is the spelling on the largest line (most shares, then
///   highest percent, then first in the file);
/// - issuer name, investor type, controller flag, local/foreign, nationality
///   and domicile come from the largest line that has a value. KSEI leaves
///   them blank on scrip lines it cannot classify; a blank there means "not
///   recorded for this account", not a different holder.
pub fn fold_accounts(lines: &[LineRow]) -> Option<Position> {
    let mut order: Vec<&LineRow> = lines.iter().collect();
    order.sort_by(|a, b| {
        b.shares_total
            .cmp(&a.shares_total)
            .then(
                b.pct
                    .partial_cmp(&a.pct)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
            .then(a.line.cmp(&b.line))
    });
    let largest = *order.first()?;
    let pick = |f: fn(&LineRow) -> &Option<String>| order.iter().find_map(|l| f(l).clone());
    let sum_some = |f: fn(&LineRow) -> Option<i64>| {
        lines
            .iter()
            .filter_map(f)
            .fold(None, |acc: Option<i64>, v| Some(acc.unwrap_or(0) + v))
    };
    Some(Position {
        symbol: largest.symbol.clone(),
        entity_id: largest.entity_id,
        issuer_name: pick(|l| &l.issuer_name),
        holder_raw: largest.holder_raw.clone(),
        investor_type: pick(|l| &l.investor_type),
        controller_or_affiliate: pick(|l| &l.controller_or_affiliate),
        local_foreign: pick(|l| &l.local_foreign),
        nationality: pick(|l| &l.nationality),
        domicile: pick(|l| &l.domicile),
        shares_scripless: sum_some(|l| l.shares_scripless),
        shares_scrip: sum_some(|l| l.shares_scrip),
        shares_total: lines.iter().map(|l| l.shares_total).sum(),
        pct: ingest::clean_pct(lines.iter().map(|l| l.pct).sum()),
        accounts: lines.len() as u32,
    })
}

/// What [`rebuild_positions`] wrote for one release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PositionStats {
    pub positions: u64,
    pub other_date_rows: u64,
    pub issuers: u64,
    pub entities: u64,
}

/// Rebuilds `holdings` for one release from its lines dated the release's
/// `as_of`: one row per (symbol, entity), lines summed by [`fold_accounts`],
/// and the release's `positions` / `other_date_rows` counts. Lines with
/// another date are never summed in. Returns `None`, touching nothing, for a
/// legacy release without lines (its holdings are all that is left of it).
///
/// This is the only writer of `holdings` for releases that have lines: an
/// import calls it, and so must anything that re-points
/// `holding_lines.entity_id` (an entity merge), for every affected release.
pub fn rebuild_positions(tx: &Transaction, release_id: i64) -> Result<Option<PositionStats>> {
    let lines: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM holding_lines WHERE release_id = ?1",
            [release_id],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    if lines == 0 {
        return Ok(None);
    }
    let as_of: String = tx
        .query_row(
            "SELECT as_of FROM releases WHERE id = ?1",
            [release_id],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    tx.execute("DELETE FROM holdings WHERE release_id = ?1", [release_id])
        .map_err(db_err)?;
    let rows: Vec<LineRow> = {
        let mut st = tx
            .prepare(
                "SELECT line, symbol, entity_id, issuer_name, holder_raw, investor_type,
                        controller_or_affiliate, local_foreign, nationality, domicile,
                        shares_scripless, shares_scrip, shares_total, pct
                 FROM holding_lines WHERE release_id = ?1 AND report_date = ?2
                 ORDER BY symbol, entity_id, line",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![release_id, as_of], |r| {
                Ok(LineRow {
                    line: r.get::<_, i64>(0)? as u32,
                    symbol: r.get(1)?,
                    entity_id: r.get(2)?,
                    issuer_name: r.get(3)?,
                    holder_raw: r.get(4)?,
                    investor_type: r.get(5)?,
                    controller_or_affiliate: r.get(6)?,
                    local_foreign: r.get(7)?,
                    nationality: r.get(8)?,
                    domicile: r.get(9)?,
                    shares_scripless: r.get(10)?,
                    shares_scrip: r.get(11)?,
                    shares_total: r.get(12)?,
                    pct: r.get(13)?,
                })
            })
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows
    };
    let mut issuers = BTreeSet::new();
    let mut entities = BTreeSet::new();
    let mut positions = 0u64;
    {
        let mut insert = tx
            .prepare(
                "INSERT INTO holdings (release_id, symbol, issuer_name, entity_id, holder_raw,
                 investor_type, local_foreign, nationality, domicile, shares_scripless, shares_scrip,
                 shares_total, pct, accounts, controller_or_affiliate)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
            )
            .map_err(db_err)?;
        for group in rows.chunk_by(|a, b| a.symbol == b.symbol && a.entity_id == b.entity_id) {
            let Some(p) = fold_accounts(group) else {
                continue;
            };
            insert
                .execute(params![
                    release_id,
                    p.symbol,
                    p.issuer_name,
                    p.entity_id,
                    p.holder_raw,
                    p.investor_type,
                    p.local_foreign,
                    p.nationality,
                    p.domicile,
                    p.shares_scripless,
                    p.shares_scrip,
                    p.shares_total,
                    p.pct,
                    p.accounts,
                    p.controller_or_affiliate,
                ])
                .map_err(db_err)?;
            issuers.insert(p.symbol);
            entities.insert(p.entity_id);
            positions += 1;
        }
    }
    let other_date_rows: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM holding_lines WHERE release_id = ?1 AND report_date <> ?2",
            params![release_id, as_of],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    tx.execute(
        "UPDATE releases SET positions = ?2, other_date_rows = ?3 WHERE id = ?1",
        params![release_id, positions as i64, other_date_rows],
    )
    .map_err(db_err)?;
    Ok(Some(PositionStats {
        positions,
        other_date_rows: other_date_rows as u64,
        issuers: issuers.len() as u64,
        entities: entities.len() as u64,
    }))
}

impl OwnershipDb {
    /// Opens (and creates) the database at `path`, migrating it to
    /// [`SCHEMA_VERSION`] when it is older.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn = Connection::open(path).map_err(db_err)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(db_err)?;
        migrate(&mut conn, path)?;
        Ok(OwnershipDb {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// In-memory database for tests.
    pub fn in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory().map_err(db_err)?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(db_err)?;
        let path = PathBuf::from(":memory:");
        migrate(&mut conn, &path)?;
        Ok(OwnershipDb { conn, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `PRAGMA user_version` of the open file ([`SCHEMA_VERSION`] once open).
    pub fn schema_version(&self) -> Result<i64> {
        user_version(&self.conn)
    }

    /// Stores a parsed file as the release for its `as_of`, replacing any
    /// earlier import of the same date: every line in `holding_lines`, then
    /// the summed positions in `holdings`.
    pub fn import(
        &mut self,
        parsed: &Parsed,
        format: &str,
        source: &str,
        sha256: &str,
    ) -> Result<ImportReport> {
        let as_of = parsed.as_of.to_string();
        let tx = self.conn.transaction().map_err(db_err)?;
        let existing: Option<i64> = tx
            .query_row("SELECT id FROM releases WHERE as_of = ?1", [&as_of], |r| {
                r.get(0)
            })
            .optional()
            .map_err(db_err)?;
        if let Some(id) = existing {
            tx.execute("DELETE FROM holdings WHERE release_id = ?1", [id])
                .map_err(db_err)?;
            tx.execute("DELETE FROM holding_lines WHERE release_id = ?1", [id])
                .map_err(db_err)?;
            tx.execute("DELETE FROM releases WHERE id = ?1", [id])
                .map_err(db_err)?;
        }
        let imported_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        tx.execute(
            "INSERT INTO releases (as_of, format, source, sha256, row_count, imported_at, controller_column)
             VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![
                as_of,
                format,
                source,
                sha256,
                parsed.rows.len() as i64,
                imported_at,
                parsed.controller_column
            ],
        )
        .map_err(db_err)?;
        let release_id = tx.last_insert_rowid();

        let mut entity_cache: HashMap<String, i64> = HashMap::new();
        {
            let mut find_alias = tx
                .prepare("SELECT entity_id FROM aliases WHERE raw_name = ?1")
                .map_err(db_err)?;
            let mut find_entity = tx
                .prepare("SELECT id FROM entities WHERE entity_key = ?1")
                .map_err(db_err)?;
            let mut insert_entity = tx
                .prepare("INSERT INTO entities (entity_key, display_name) VALUES (?1, ?2)")
                .map_err(db_err)?;
            let mut insert_alias = tx
                .prepare("INSERT OR IGNORE INTO aliases (raw_name, entity_id) VALUES (?1, ?2)")
                .map_err(db_err)?;
            let mut insert_line = tx
                .prepare(
                    "INSERT INTO holding_lines (release_id, line, report_date, symbol, issuer_name,
                     entity_id, holder_raw, investor_type, controller_or_affiliate, local_foreign,
                     nationality, domicile, shares_scripless, shares_scrip, shares_total, pct)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                )
                .map_err(db_err)?;

            for r in &parsed.rows {
                let key = normalize::entity_key(&r.holder_raw);
                let entity_id = match entity_cache.get(&key) {
                    Some(id) => *id,
                    None => {
                        let by_alias: Option<i64> = find_alias
                            .query_row([&r.holder_raw], |row| row.get(0))
                            .optional()
                            .map_err(db_err)?;
                        let id = match by_alias {
                            Some(id) => id,
                            None => {
                                let by_key: Option<i64> = find_entity
                                    .query_row([&key], |row| row.get(0))
                                    .optional()
                                    .map_err(db_err)?;
                                match by_key {
                                    Some(id) => id,
                                    None => {
                                        insert_entity
                                            .execute(params![
                                                key,
                                                normalize::display_name(&r.holder_raw)
                                            ])
                                            .map_err(db_err)?;
                                        tx.last_insert_rowid()
                                    }
                                }
                            }
                        };
                        entity_cache.insert(key.clone(), id);
                        id
                    }
                };
                insert_alias
                    .execute(params![r.holder_raw, entity_id])
                    .map_err(db_err)?;
                insert_line
                    .execute(params![
                        release_id,
                        r.line,
                        r.as_of.to_string(),
                        r.symbol,
                        r.issuer_name,
                        entity_id,
                        r.holder_raw,
                        r.investor_type,
                        r.controller_or_affiliate,
                        r.local_foreign,
                        r.nationality,
                        r.domicile,
                        r.shares_scripless,
                        r.shares_scrip,
                        r.shares_total,
                        r.pct,
                    ])
                    .map_err(|e| match e {
                        rusqlite::Error::SqliteFailure(f, _)
                            if f.code == rusqlite::ErrorCode::ConstraintViolation =>
                        {
                            IdxError::Other(format!(
                                "ownership import: line {} appears twice in the parsed file",
                                r.line
                            ))
                        }
                        e => db_err(e),
                    })?;
            }
        }
        let stats = rebuild_positions(&tx, release_id)?.ok_or_else(|| {
            IdxError::Other("ownership import: no register lines were stored".into())
        })?;
        tx.commit().map_err(db_err)?;
        Ok(ImportReport {
            release: Release {
                as_of: parsed.as_of,
                format: format.to_string(),
                source: source.to_string(),
                sha256: sha256.to_string(),
                rows: parsed.rows.len() as u64,
                issuers: stats.issuers,
                entities: stats.entities,
                imported_at,
                positions: Some(stats.positions),
                other_date_rows: Some(stats.other_date_rows),
                controller_column: Some(parsed.controller_column),
            },
            replaced: existing.is_some(),
            database: self.path.display().to_string(),
        })
    }

    /// Every issuer code seen in any release, with the latest name on file.
    pub fn issuers(&self) -> Result<Vec<(String, Option<String>)>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT symbol, MAX(issuer_name) FROM holdings GROUP BY symbol ORDER BY symbol",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    pub fn releases(&self) -> Result<Vec<Release>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT r.as_of, r.format, r.source, r.sha256, r.row_count, r.imported_at,
                        (SELECT COUNT(DISTINCT symbol) FROM holdings h WHERE h.release_id = r.id),
                        (SELECT COUNT(DISTINCT entity_id) FROM holdings h WHERE h.release_id = r.id),
                        r.positions, r.other_date_rows, r.controller_column
                 FROM releases r ORDER BY r.as_of",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([], |row| {
                Ok(Release {
                    as_of: parse_date(&row.get::<_, String>(0)?),
                    format: row.get(1)?,
                    source: row.get(2)?,
                    sha256: row.get(3)?,
                    rows: row.get::<_, i64>(4)? as u64,
                    imported_at: row.get(5)?,
                    issuers: row.get::<_, i64>(6)? as u64,
                    entities: row.get::<_, i64>(7)? as u64,
                    positions: row.get::<_, Option<i64>>(8)?.map(|v| v as u64),
                    other_date_rows: row.get::<_, Option<i64>>(9)?.map(|v| v as u64),
                    controller_column: row.get::<_, Option<i64>>(10)?.map(|v| v != 0),
                })
            })
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// Warnings about the releases on `dates` (unknown dates are ignored):
    /// - one line naming the releases imported by idx 0.8.1 or older, whose
    ///   holdings kept a single account per holder and ticker;
    /// - per release whose file also listed rows with another date, what was
    ///   left out of its holdings.
    ///
    /// Every query that reads those releases appends these to `warnings`.
    pub fn release_notes(&self, dates: &[NaiveDate]) -> Result<Vec<String>> {
        let mut wanted: Vec<NaiveDate> = dates.to_vec();
        wanted.sort();
        wanted.dedup();
        let mut legacy = Vec::new();
        let mut other = Vec::new();
        for d in wanted {
            let row: Option<(i64, Option<i64>, Option<i64>)> = self
                .conn
                .query_row(
                    "SELECT id, positions, other_date_rows FROM releases WHERE as_of = ?1",
                    [d.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(db_err)?;
            match row {
                Some((_, None, _)) => legacy.push(d.to_string()),
                Some((id, Some(_), Some(n))) if n > 0 => other.push((id, d)),
                _ => {}
            }
        }
        let mut notes = Vec::new();
        if !legacy.is_empty() {
            let one = legacy.len() == 1;
            notes.push(format!(
                "ownership release{} {} {} imported by idx 0.8.1 or older, which kept one account per holder and ticker; re-import {} with `idx ownership import --all --force` (or --file)",
                if one { "" } else { "s" },
                legacy.join(", "),
                if one { "was" } else { "were" },
                if one { "it" } else { "them" },
            ));
        }
        for (id, as_of) in other {
            let mut st = self
                .conn
                .prepare(
                    "SELECT report_date, COUNT(*), GROUP_CONCAT(DISTINCT symbol)
                     FROM holding_lines WHERE release_id = ?1 AND report_date <> ?2
                     GROUP BY report_date ORDER BY report_date",
                )
                .map_err(db_err)?;
            let groups = st
                .query_map(params![id, as_of.to_string()], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(db_err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_err)?;
            for (date, n, symbols) in groups {
                let mut symbols: Vec<String> = symbols.split(',').map(str::to_string).collect();
                symbols.sort();
                notes.push(ingest::other_date_note(
                    &format!("the {as_of} file"),
                    n as usize,
                    parse_date(&date),
                    &symbols,
                    as_of,
                ));
            }
        }
        Ok(notes)
    }

    /// Release id for `as_of` (exact), or the latest when `None`.
    fn release_for(&self, as_of: Option<NaiveDate>) -> Result<(i64, NaiveDate)> {
        let row: Option<(i64, String)> = match as_of {
            Some(d) => self
                .conn
                .query_row(
                    "SELECT id, as_of FROM releases WHERE as_of = ?1",
                    [d.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(db_err)?,
            None => self
                .conn
                .query_row(
                    "SELECT id, as_of FROM releases ORDER BY as_of DESC LIMIT 1",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(db_err)?,
        };
        match row {
            Some((id, d)) => Ok((id, parse_date(&d))),
            None => {
                let available: Vec<String> = self
                    .releases()?
                    .iter()
                    .map(|r| r.as_of.to_string())
                    .collect();
                Err(IdxError::NotFound(if available.is_empty() {
                    "no ownership data imported yet; run `idx ownership import --latest` or `--file <xlsx>`".into()
                } else {
                    format!(
                        "no ownership release for {}; available: {}",
                        as_of
                            .map(|d| d.to_string())
                            .unwrap_or_else(|| "latest".into()),
                        available.join(", ")
                    )
                }))
            }
        }
    }

    /// Whether the release's file had the CONTROLLER_OR_AFFILIATE column.
    fn controller_reported(&self, release_id: i64) -> Result<bool> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT controller_column FROM releases WHERE id = ?1",
                [release_id],
                |r| r.get(0),
            )
            .map_err(db_err)?;
        Ok(v == Some(1))
    }

    fn holdings_where(
        &self,
        release_id: i64,
        as_of: NaiveDate,
        clause: &str,
        arg: &str,
    ) -> Result<Vec<Holding>> {
        let sql = format!(
            "SELECT h.symbol, h.issuer_name, e.display_name, h.holder_raw, e.entity_key, h.investor_type,
                    h.local_foreign, h.nationality, h.domicile, h.shares_scripless, h.shares_scrip,
                    h.shares_total, h.pct, h.accounts, h.controller_or_affiliate
             FROM holdings h JOIN entities e ON e.id = h.entity_id
             WHERE h.release_id = ?1 AND {clause}
             ORDER BY h.pct DESC, h.symbol"
        );
        let mut st = self.conn.prepare(&sql).map_err(db_err)?;
        let rows = st
            .query_map(params![release_id, arg], |row| {
                Ok(Holding {
                    as_of,
                    symbol: row.get(0)?,
                    issuer_name: row.get(1)?,
                    holder: row.get(2)?,
                    holder_raw: row.get(3)?,
                    entity_key: row.get(4)?,
                    investor_type: row.get(5)?,
                    local_foreign: row.get(6)?,
                    nationality: row.get(7)?,
                    domicile: row.get(8)?,
                    shares_scripless: row.get(9)?,
                    shares_scrip: row.get(10)?,
                    shares_total: row.get(11)?,
                    pct: row.get(12)?,
                    accounts: row.get::<_, Option<i64>>(13)?.map(|v| v as u32),
                    controller_or_affiliate: row.get(14)?,
                })
            })
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// Holders above 1 % of `symbol` in the release for `as_of` (latest if `None`).
    pub fn holders(&self, symbol: &str, as_of: Option<NaiveDate>) -> Result<HoldersReport> {
        let (release_id, date) = self.release_for(as_of)?;
        let symbol = symbol.to_uppercase();
        let holders = self.holdings_where(release_id, date, "h.symbol = ?2", &symbol)?;
        if holders.is_empty() {
            let mut st = self
                .conn
                .prepare(
                    "SELECT DISTINCT report_date FROM holding_lines
                     WHERE release_id = ?1 AND symbol = ?2 AND report_date <> ?3
                     ORDER BY report_date",
                )
                .map_err(db_err)?;
            let elsewhere = st
                .query_map(params![release_id, symbol, date.to_string()], |r| {
                    r.get::<_, String>(0)
                })
                .map_err(db_err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(db_err)?;
            return Err(IdxError::NotFound(if elsewhere.is_empty() {
                format!("no holders above 1% recorded for {symbol} as of {date}")
            } else {
                format!(
                    "no holders above 1% recorded for {symbol} as of {date} (the file lists {symbol} only on rows dated {}, which are not part of that release)",
                    elsewhere.join(", ")
                )
            }));
        }
        Ok(HoldersReport {
            issuer_name: holders.iter().find_map(|h| h.issuer_name.clone()),
            as_of: date,
            listed_pct: holders.iter().map(|h| h.pct).sum(),
            controller_reported: self.controller_reported(release_id)?,
            symbol,
            holders,
        })
    }

    /// Every position of holders whose normalised name contains `query`.
    /// The query needs at least 3 letters or digits once normalised: `%%%` or
    /// `!!!` would otherwise normalise to an empty key and match everyone.
    pub fn entity(&self, query: &str, as_of: Option<NaiveDate>) -> Result<EntityReport> {
        self.entity_matching(query, as_of, false)
    }

    /// [`Self::entity`], or with `exact` only the holder whose key is the
    /// normalised query (a key from `ownership search`).
    pub fn entity_matching(
        &self,
        query: &str,
        as_of: Option<NaiveDate>,
        exact: bool,
    ) -> Result<EntityReport> {
        let key = normalize::entity_key(query);
        if key.chars().filter(|c| c.is_alphanumeric()).count() < 3 {
            return Err(IdxError::Usage(
                "name needs at least 3 letters or digits".into(),
            ));
        }
        let (release_id, date) = self.release_for(as_of)?;
        let holdings = if exact {
            self.holdings_where(release_id, date, "e.entity_key = ?2", &key)?
        } else {
            let pattern = format!("%{key}%");
            self.holdings_where(release_id, date, "e.entity_key LIKE ?2", &pattern)?
        };
        if holdings.is_empty() {
            return Err(IdxError::NotFound(format!(
                "no holder matching {query:?} as of {date}"
            )));
        }
        let mut entities: Vec<String> = holdings.iter().map(|h| h.holder.clone()).collect();
        entities.sort();
        entities.dedup();
        Ok(EntityReport {
            query: query.to_string(),
            as_of: date,
            entities,
            holdings,
            controller_reported: self.controller_reported(release_id)?,
        })
    }

    /// Movements between two releases, holder by holder (each side's
    /// accounts already summed): every match is summarised, then one page
    /// (`offset`, `limit`) of rows is returned, largest moves first.
    ///
    /// A holder with the same shares on both sides is skipped when the
    /// percentage did not move, or, when a side has several lines, moved by
    /// no more than the rounding of those lines (each line is printed to
    /// 0.01 pp, so a sum of `n` lines can drift by up to 0.005 * n: scrip
    /// converted to scripless). Single lines on both sides keep any move,
    /// since then the issued share count changed.
    pub fn changes(
        &self,
        from: NaiveDate,
        to: NaiveDate,
        symbol: Option<&str>,
        min_pct_change: f64,
        limit: Option<usize>,
        offset: usize,
    ) -> Result<ChangesReport> {
        if from >= to {
            return Err(IdxError::Usage("--from must be earlier than --to".into()));
        }
        let (from_id, _) = self.release_for(Some(from))?;
        let (to_id, _) = self.release_for(Some(to))?;
        let symbol = symbol.map(str::to_uppercase);

        let sql =
            "SELECT h.symbol, e.display_name, e.entity_key, h.investor_type, h.shares_total, h.pct,
                    h.local_foreign, h.accounts, h.controller_or_affiliate
             FROM holdings h JOIN entities e ON e.id = h.entity_id
             WHERE h.release_id = ?1 AND (?2 IS NULL OR h.symbol = ?2)";
        let load = |release_id: i64| -> Result<Positions> {
            let mut st = self.conn.prepare(sql).map_err(db_err)?;
            let rows = st
                .query_map(params![release_id, symbol], |row| {
                    Ok((
                        (row.get::<_, String>(0)?, row.get::<_, String>(2)?),
                        Side {
                            holder: row.get(1)?,
                            investor_type: row.get(3)?,
                            shares: row.get(4)?,
                            pct: row.get(5)?,
                            local_foreign: row.get(6)?,
                            accounts: row.get::<_, Option<i64>>(7)?.map(|v| v as u32),
                            controller_or_affiliate: row.get(8)?,
                        },
                    ))
                })
                .map_err(db_err)?
                .collect::<std::result::Result<BTreeMap<_, _>, _>>()
                .map_err(db_err)?;
            Ok(rows)
        };
        let before = load(from_id)?;
        let after = load(to_id)?;

        let mut changes: Vec<Change> = Vec::new();
        for ((sym, key), a) in &after {
            let row = |kind, b: Option<&Side>, pct_change: f64, shares_change: i64| Change {
                symbol: sym.clone(),
                holder: a.holder.clone(),
                entity_key: key.clone(),
                investor_type: a.investor_type.clone(),
                local_foreign: a.local_foreign.clone(),
                kind,
                pct_from: b.map(|b| b.pct),
                pct_to: Some(a.pct),
                pct_change: round4(pct_change),
                shares_from: b.map(|b| b.shares),
                shares_to: Some(a.shares),
                shares_change,
                accounts_from: b.and_then(|b| b.accounts),
                accounts_to: a.accounts,
                controller_or_affiliate: a.controller_or_affiliate.clone(),
            };
            match before.get(&(sym.clone(), key.clone())) {
                Some(b) => {
                    let pct_change = a.pct - b.pct;
                    if a.shares == b.shares {
                        let lines = b.accounts.unwrap_or(1) + a.accounts.unwrap_or(1);
                        if pct_change.abs() < 1e-9
                            || (lines > 2 && pct_change.abs() <= 0.005 * lines as f64 + 1e-9)
                        {
                            continue;
                        }
                    }
                    changes.push(row(
                        ChangeKind::Changed,
                        Some(b),
                        pct_change,
                        a.shares - b.shares,
                    ));
                }
                None => changes.push(row(ChangeKind::New, None, a.pct, a.shares)),
            }
        }
        for ((sym, key), b) in &before {
            if !after.contains_key(&(sym.clone(), key.clone())) {
                changes.push(Change {
                    symbol: sym.clone(),
                    holder: b.holder.clone(),
                    entity_key: key.clone(),
                    investor_type: b.investor_type.clone(),
                    local_foreign: b.local_foreign.clone(),
                    kind: ChangeKind::Exited,
                    pct_from: Some(b.pct),
                    pct_to: None,
                    pct_change: round4(-b.pct),
                    shares_from: Some(b.shares),
                    shares_to: None,
                    shares_change: -b.shares,
                    accounts_from: b.accounts,
                    accounts_to: None,
                    controller_or_affiliate: b.controller_or_affiliate.clone(),
                });
            }
        }
        changes.retain(|c| c.pct_change.abs() + 1e-12 >= min_pct_change);
        changes.sort_by(|a, b| {
            b.pct_change
                .abs()
                .partial_cmp(&a.pct_change.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.symbol.cmp(&b.symbol))
        });
        let total = changes.len();
        let summary = summarise(&changes);
        let offset = offset.min(total);
        changes.drain(..offset);
        if let Some(n) = limit {
            changes.truncate(n);
        }
        Ok(ChangesReport {
            from,
            to,
            symbol,
            min_pct_change,
            total_matches: total,
            offset,
            summary,
            changes,
        })
    }
}

impl OwnershipDb {
    /// Disclosed foreign/local split of one issuer across every imported release.
    pub fn foreign_trend(&self, symbol: &str) -> Result<ForeignReport> {
        let symbol = symbol.to_uppercase();
        let mut st = self
            .conn
            .prepare(
                "SELECT r.as_of, MAX(h.issuer_name),
                        COALESCE(SUM(CASE WHEN h.local_foreign = 'A' THEN h.pct END), 0),
                        COALESCE(SUM(CASE WHEN h.local_foreign = 'L' THEN h.pct END), 0),
                        COALESCE(SUM(CASE WHEN h.local_foreign IS NULL OR h.local_foreign NOT IN ('A','L') THEN h.pct END), 0),
                        SUM(CASE WHEN h.local_foreign = 'A' THEN 1 ELSE 0 END),
                        SUM(CASE WHEN h.local_foreign = 'L' THEN 1 ELSE 0 END),
                        SUM(h.pct)
                 FROM holdings h JOIN releases r ON r.id = h.release_id
                 WHERE h.symbol = ?1
                 GROUP BY r.as_of ORDER BY r.as_of",
            )
            .map_err(db_err)?;
        let mut issuer_name: Option<String> = None;
        let mut points: Vec<ForeignPoint> = st
            .query_map([&symbol], |row| {
                let name: Option<String> = row.get(1)?;
                Ok((
                    name,
                    ForeignPoint {
                        as_of: parse_date(&row.get::<_, String>(0)?),
                        foreign_pct: round4(row.get(2)?),
                        local_pct: round4(row.get(3)?),
                        unflagged_pct: round4(row.get(4)?),
                        foreign_holders: row.get::<_, i64>(5)? as usize,
                        local_holders: row.get::<_, i64>(6)? as usize,
                        listed_pct: round4(row.get(7)?),
                        foreign_change_pp: None,
                    },
                ))
            })
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?
            .into_iter()
            .map(|(name, p)| {
                if name.is_some() {
                    issuer_name = name;
                }
                p
            })
            .collect();
        if points.is_empty() {
            self.release_for(None)?; // "nothing imported" message when the db is empty
            return Err(IdxError::NotFound(format!(
                "no holders above 1% recorded for {symbol} in any imported release"
            )));
        }
        for i in 1..points.len() {
            points[i].foreign_change_pp =
                Some(round4(points[i].foreign_pct - points[i - 1].foreign_pct));
        }
        Ok(ForeignReport {
            symbol,
            issuer_name,
            points,
        })
    }

    fn foreign_by_issuer(&self, release_id: i64) -> Result<IssuerForeignAgg> {
        let mut st = self
            .conn
            .prepare(
                "SELECT symbol, MAX(issuer_name),
                        COALESCE(SUM(CASE WHEN local_foreign = 'A' THEN pct END), 0),
                        SUM(CASE WHEN local_foreign = 'A' THEN 1 ELSE 0 END)
                 FROM holdings WHERE release_id = ?1 GROUP BY symbol",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([release_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    (
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, i64>(3)? as usize,
                    ),
                ))
            })
            .map_err(db_err)?
            .collect::<std::result::Result<BTreeMap<_, _>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// Change in disclosed foreign stake per issuer between two releases
    /// (default: the two newest). Largest absolute moves first.
    pub fn foreign_flow(
        &self,
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
        limit: Option<usize>,
    ) -> Result<ForeignFlowReport> {
        let releases = self.releases()?;
        let (to_date, _) = match to {
            Some(d) => (d, ()),
            None => (
                releases
                    .last()
                    .ok_or_else(|| {
                        IdxError::NotFound(
                            "no ownership data imported yet; run `idx ownership import --latest`"
                                .into(),
                        )
                    })?
                    .as_of,
                (),
            ),
        };
        let from_date = match from {
            Some(d) => d,
            None => releases
                .iter()
                .rev()
                .map(|r| r.as_of)
                .find(|d| *d < to_date)
                .ok_or_else(|| {
                    IdxError::NotFound(format!(
                        "only one release ({to_date}) is imported; import an earlier month to compare"
                    ))
                })?,
        };
        if from_date >= to_date {
            return Err(IdxError::Usage("--from must be earlier than --to".into()));
        }
        let (from_id, _) = self.release_for(Some(from_date))?;
        let (to_id, _) = self.release_for(Some(to_date))?;
        let before = self.foreign_by_issuer(from_id)?;
        let after = self.foreign_by_issuer(to_id)?;
        let mut rows: Vec<IssuerForeign> = after
            .iter()
            .map(|(sym, (name, pct_to, holders))| {
                let pct_from = before.get(sym).map(|b| round4(b.1));
                IssuerForeign {
                    symbol: sym.clone(),
                    issuer_name: name.clone(),
                    foreign_pct_from: pct_from,
                    foreign_pct_to: round4(*pct_to),
                    change_pp: round4(pct_to - pct_from.unwrap_or(0.0)),
                    foreign_holders_to: *holders,
                }
            })
            .collect();
        rows.sort_by(|a, b| {
            b.change_pp
                .abs()
                .partial_cmp(&a.change_pp.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.symbol.cmp(&b.symbol))
        });
        let issuers = rows.len();
        let inflows = rows.iter().filter(|r| r.change_pp > 0.0).count();
        let outflows = rows.iter().filter(|r| r.change_pp < 0.0).count();
        let net_pp = round4(rows.iter().map(|r| r.change_pp).sum());
        if let Some(n) = limit {
            rows.truncate(n);
        }
        Ok(ForeignFlowReport {
            from: from_date,
            to: to_date,
            issuers,
            inflows,
            outflows,
            net_pp,
            rows,
        })
    }
}

/// symbol -> (issuer name, foreign pct, foreign holder count) for one release.
type IssuerForeignAgg = BTreeMap<String, (Option<String>, f64, usize)>;

/// One holder's summed position on one side of `changes`.
#[derive(Debug, Clone)]
struct Side {
    holder: String,
    investor_type: Option<String>,
    shares: i64,
    pct: f64,
    local_foreign: Option<String>,
    accounts: Option<u32>,
    controller_or_affiliate: Option<String>,
}

/// (symbol, entity_key) -> that holder's position in one release.
type Positions = BTreeMap<(String, String), Side>;

/// Aggregates every match into the `summary` block.
pub fn summarise(changes: &[Change]) -> ChangesSummary {
    use std::collections::BTreeSet;
    let brief = |c: &Change| ChangeBrief {
        symbol: c.symbol.clone(),
        holder: c.holder.clone(),
        kind: c.kind,
        pct_from: c.pct_from,
        pct_to: c.pct_to,
        pct_change: c.pct_change,
    };
    let is_foreign = |c: &Change| c.local_foreign.as_deref() == Some("A");
    let issuers: BTreeSet<&str> = changes.iter().map(|c| c.symbol.as_str()).collect();
    let holders: BTreeSet<&str> = changes.iter().map(|c| c.entity_key.as_str()).collect();

    let mut by_type: BTreeMap<String, (usize, f64)> = BTreeMap::new();
    let mut by_issuer: BTreeMap<&str, (usize, f64, f64)> = BTreeMap::new();
    for c in changes {
        let t = c
            .investor_type
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "UNKNOWN".into());
        let e = by_type.entry(t).or_default();
        e.0 += 1;
        e.1 += c.pct_change;
        let i = by_issuer.entry(c.symbol.as_str()).or_default();
        i.0 += 1;
        i.1 += c.pct_change;
        if is_foreign(c) {
            i.2 += c.pct_change;
        }
    }
    let mut by_investor_type: Vec<TypeFlow> = by_type
        .into_iter()
        .map(|(k, (rows, net))| TypeFlow {
            investor_type: k,
            rows,
            net_pp: round4(net),
        })
        .collect();
    by_investor_type.sort_by(|a, b| {
        b.net_pp
            .abs()
            .partial_cmp(&a.net_pp.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut issuer_flow: Vec<IssuerFlow> = by_issuer
        .into_iter()
        .map(|(s, (rows, net, foreign))| IssuerFlow {
            symbol: s.to_string(),
            rows,
            net_pp: round4(net),
            foreign_net_pp: round4(foreign),
        })
        .collect();
    issuer_flow.sort_by(|a, b| {
        b.net_pp
            .abs()
            .partial_cmp(&a.net_pp.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.symbol.cmp(&b.symbol))
    });
    issuer_flow.truncate(10);

    // `changes` arrives sorted by absolute move, so the first positive /
    // negative rows are the largest ones.
    let top_increases: Vec<ChangeBrief> = changes
        .iter()
        .filter(|c| c.pct_change > 0.0)
        .take(5)
        .map(brief)
        .collect();
    let top_decreases: Vec<ChangeBrief> = changes
        .iter()
        .filter(|c| c.pct_change < 0.0)
        .take(5)
        .map(brief)
        .collect();
    let net = |pred: &dyn Fn(&Change) -> bool| -> f64 {
        round4(
            changes
                .iter()
                .filter(|c| pred(c))
                .map(|c| c.pct_change)
                .sum(),
        )
    };
    ChangesSummary {
        issuers: issuers.len(),
        holders: holders.len(),
        new_holders: changes.iter().filter(|c| c.kind == ChangeKind::New).count(),
        exits: changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Exited)
            .count(),
        increased: changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Changed && c.pct_change > 0.0)
            .count(),
        decreased: changes
            .iter()
            .filter(|c| c.kind == ChangeKind::Changed && c.pct_change < 0.0)
            .count(),
        net_pp: net(&|_| true),
        foreign_net_pp: net(&is_foreign),
        local_net_pp: net(&|c| c.local_foreign.as_deref() == Some("L")),
        by_investor_type,
        issuer_flow,
        top_increases,
        top_decreases,
    }
}

fn round4(v: f64) -> f64 {
    let r = (v * 10_000.0).round() / 10_000.0;
    if r == 0.0 {
        0.0 // never -0.0 in JSON
    } else {
        r
    }
}

fn parse_date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ownership::ingest::parse_xlsx;

    const XLSX: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20260831_excerpt.xlsx");
    const XLSX_PREV: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20260731_synthetic.xlsx");

    fn loaded() -> OwnershipDb {
        let mut db = OwnershipDb::in_memory().unwrap();
        let prev = parse_xlsx(XLSX_PREV).unwrap();
        db.import(&prev, "xlsx", "prev.xlsx", "aaa").unwrap();
        let cur = parse_xlsx(XLSX).unwrap();
        db.import(&cur, "xlsx", "cur.xlsx", "bbb").unwrap();
        db
    }

    #[test]
    fn import_and_releases() {
        let db = loaded();
        let r = db.releases().unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[1].as_of, NaiveDate::from_ymd_opt(2026, 8, 31).unwrap());
        assert!(r[1].rows > 10);
        assert!(r[1].entities > 5);
    }

    #[test]
    fn reimport_replaces() {
        let mut db = loaded();
        let cur = parse_xlsx(XLSX).unwrap();
        let rep = db.import(&cur, "xlsx", "again.xlsx", "ccc").unwrap();
        assert!(rep.replaced);
        assert_eq!(db.releases().unwrap().len(), 2);
    }

    #[test]
    fn holders_latest_and_dated() {
        let db = loaded();
        let h = db.holders("aadi", None).unwrap();
        assert_eq!(h.symbol, "AADI");
        assert_eq!(h.as_of, NaiveDate::from_ymd_opt(2026, 8, 31).unwrap());
        assert!(h.holders[0].pct >= h.holders[1].pct, "sorted by pct");
        assert!(h.listed_pct > 50.0);
        assert_eq!(h.holders[0].holder, "ADARO STRATEGIC INVESTMENTS");
        let err = db.holders("ZZZZ", None).unwrap_err();
        assert_eq!(err.exit_code(), 4);
        let err = db
            .holders("AADI", NaiveDate::from_ymd_opt(2020, 1, 1))
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("available: 2026-07-31, 2026-08-31"),
            "{err}"
        );
    }

    #[test]
    fn entity_search_groups_spellings() {
        let db = loaded();
        let e = db.entity("alamtri resources", None).unwrap();
        assert!(!e.holdings.is_empty());
        assert!(e.entities.iter().all(|n| n.contains("ALAMTRI")));
        assert!(db.entity("no such holder", None).is_err());
    }

    #[test]
    fn entity_search_needs_three_letters_or_digits() {
        let empty = OwnershipDb::in_memory().unwrap();
        let db = loaded();
        for q in ["%%%", "!!!", "___", "  %_% ", "PT", "a.b"] {
            for d in [&empty, &db] {
                let err = d.entity(q, None).unwrap_err();
                assert_eq!(err.exit_code(), 2, "{q:?}: {err}");
                assert!(
                    err.to_string()
                        .contains("name needs at least 3 letters or digits"),
                    "{err}"
                );
            }
        }
        assert!(db.entity("Adaro", None).is_ok());
    }

    #[test]
    fn foreign_trend_and_flow() {
        let db = loaded();
        let t = db.foreign_trend("aadi").unwrap();
        assert_eq!(t.symbol, "AADI");
        assert_eq!(t.points.len(), 2);
        assert!(t.points[0].foreign_change_pp.is_none());
        assert!(t.points[1].foreign_change_pp.is_some());
        for p in &t.points {
            assert!(
                (p.foreign_pct + p.local_pct + p.unflagged_pct - p.listed_pct).abs() < 1e-6,
                "{p:?}"
            );
        }
        assert!(db.foreign_trend("ZZZZ").is_err());
        let f = db.foreign_flow(None, None, None).unwrap();
        assert_eq!(
            (f.from.to_string().as_str(), f.to.to_string().as_str()),
            ("2026-07-31", "2026-08-31")
        );
        assert!(f.issuers >= 1);
        assert_eq!(f.rows.len(), f.issuers);
        assert!(f
            .rows
            .windows(2)
            .all(|w| w[0].change_pp.abs() >= w[1].change_pp.abs()));
        let one = db.foreign_flow(None, None, Some(1)).unwrap();
        assert_eq!(one.rows.len(), 1);
        assert_eq!(one.issuers, f.issuers);
    }

    #[test]
    fn changes_between_months() {
        let db = loaded();
        let c = db
            .changes(
                NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
                NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
                None,
                0.0,
                None,
                0,
            )
            .unwrap();
        assert!(c.total_matches >= 3, "{c:?}");
        let s = &c.summary;
        assert_eq!(
            s.new_holders + s.exits + s.increased + s.decreased,
            c.total_matches,
            "{s:?}"
        );
        assert!(s.issuers >= 1 && s.holders >= 3);
        assert!(!s.top_increases.is_empty() && !s.top_decreases.is_empty());
        assert!(s.top_increases[0].pct_change > 0.0);
        assert!(s.top_decreases[0].pct_change < 0.0);
        assert!(
            (s.net_pp - (s.foreign_net_pp + s.local_net_pp)).abs() < 1e-6
                || s.by_investor_type
                    .iter()
                    .any(|t| t.investor_type == "UNKNOWN")
                || c.changes.iter().any(|x| x.local_foreign.is_none())
        );
        // Paging keeps the summary over every match.
        let page = db
            .changes(
                NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
                NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
                None,
                0.0,
                Some(1),
                1,
            )
            .unwrap();
        assert_eq!(page.offset, 1);
        assert_eq!(page.changes.len(), 1);
        assert_eq!(page.changes[0], c.changes[1]);
        assert_eq!(page.summary, c.summary);
        assert_eq!(page.total_matches, c.total_matches);
        let kinds: Vec<ChangeKind> = c.changes.iter().map(|x| x.kind).collect();
        assert!(kinds.contains(&ChangeKind::Changed));
        assert!(kinds.contains(&ChangeKind::New));
        assert!(kinds.contains(&ChangeKind::Exited));
        assert!(c.changes[0].pct_change.abs() >= c.changes[1].pct_change.abs());
        let only = db
            .changes(
                NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
                NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
                Some("aadi"),
                0.5,
                Some(1),
                0,
            )
            .unwrap();
        assert_eq!(only.changes.len(), 1);
        assert!(only.changes.iter().all(|x| x.symbol == "AADI"));
        assert!(db
            .changes(
                NaiveDate::from_ymd_opt(2026, 8, 31).unwrap(),
                NaiveDate::from_ymd_opt(2026, 7, 31).unwrap(),
                None,
                0.0,
                None,
                0
            )
            .is_err());
    }

    const AUG: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20260831_accounts_excerpt.xlsx");
    const SEP: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20260930_accounts_excerpt.xlsx");
    const MIXED: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_mixed_dates_synthetic.xlsx");
    const OCT: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20261030_excerpt.xlsx");

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn accounts_db() -> OwnershipDb {
        let mut db = OwnershipDb::in_memory().unwrap();
        db.import(&parse_xlsx(AUG).unwrap(), "xlsx", "aug.xlsx", "a")
            .unwrap();
        db.import(&parse_xlsx(SEP).unwrap(), "xlsx", "sep.xlsx", "s")
            .unwrap();
        db
    }

    fn holder<'a>(r: &'a HoldersReport, key: &str) -> &'a Holding {
        r.holders
            .iter()
            .find(|h| h.entity_key == key)
            .unwrap_or_else(|| panic!("{key} not in {}", r.symbol))
    }

    fn line(n: u32, raw: &str, shares: i64, pct: f64) -> LineRow {
        LineRow {
            line: n,
            symbol: "KREN".into(),
            entity_id: 7,
            issuer_name: None,
            holder_raw: raw.into(),
            investor_type: None,
            controller_or_affiliate: None,
            local_foreign: None,
            nationality: None,
            domicile: None,
            shares_scripless: None,
            shares_scrip: None,
            shares_total: shares,
            pct,
        }
    }

    #[test]
    fn fold_sums_accounts_and_takes_attributes_from_the_largest_line() {
        assert_eq!(fold_accounts(&[]), None);
        let scrip = LineRow {
            shares_scrip: Some(1_719_000_334),
            shares_scripless: Some(0),
            domicile: Some("INDONESIA".into()),
            ..line(3752, "PT KRESNA PRIMA INVEST", 1_719_000_334, 9.44)
        };
        let scripless = LineRow {
            shares_scripless: Some(729_400_000),
            investor_type: Some("CORPORATE".into()),
            local_foreign: Some("L".into()),
            controller_or_affiliate: Some("CONTROLLER".into()),
            ..line(3753, "PT. KRESNA PRIMA INVEST", 729_400_000, 4.01)
        };
        let p = fold_accounts(&[scripless.clone(), scrip.clone()]).unwrap();
        assert_eq!(p.accounts, 2);
        assert_eq!(p.shares_total, 2_448_400_334);
        assert_eq!(p.pct, 13.45, "summed and rounded to 4 decimals");
        assert_eq!(p.shares_scrip, Some(1_719_000_334));
        assert_eq!(p.shares_scripless, Some(729_400_000));
        assert_eq!(
            p.holder_raw, "PT KRESNA PRIMA INVEST",
            "largest line's spelling"
        );
        assert_eq!(p.domicile.as_deref(), Some("INDONESIA"));
        assert_eq!(
            p.investor_type.as_deref(),
            Some("CORPORATE"),
            "blank on the largest line, taken from the next"
        );
        assert_eq!(p.local_foreign.as_deref(), Some("L"));
        assert_eq!(p.controller_or_affiliate.as_deref(), Some("CONTROLLER"));
        // Order of the input does not matter; ties go to the first line.
        assert_eq!(fold_accounts(&[scrip.clone(), scripless]).unwrap(), p);
        let a = line(10, "FIRST", 100, 1.5);
        let b = line(11, "SECOND", 100, 1.5);
        let tie = fold_accounts(&[b, a]).unwrap();
        assert_eq!(tie.holder_raw, "FIRST");
        assert_eq!(tie.shares_scrip, None, "no line has a value");
        assert_eq!(tie.pct, 3.0);
        let single = fold_accounts(std::slice::from_ref(&scrip)).unwrap();
        assert_eq!(single.accounts, 1);
        assert_eq!(single.pct, 9.44);
    }

    #[test]
    fn a_holders_accounts_are_summed_into_one_position() {
        let db = accounts_db();
        let kren = db.holders("KREN", None).unwrap();
        assert_eq!(kren.as_of, date(2026, 9, 30));
        assert_eq!(kren.holders.len(), 12);
        assert_eq!(round4(kren.listed_pct), 54.41);
        assert!(!kren.controller_reported);
        let kresna = holder(&kren, "KRESNA PRIMA INVEST");
        assert_eq!(kresna.pct, 13.45);
        assert_eq!(kresna.shares_total, 2_448_400_334);
        assert_eq!(kresna.shares_scrip, Some(1_719_000_334));
        assert_eq!(kresna.shares_scripless, Some(729_400_000));
        assert_eq!(kresna.accounts, Some(2));
        assert_eq!(kresna.holder_raw, "PT KRESNA PRIMA INVEST");
        assert_eq!(kresna.controller_or_affiliate, None);
        // 13.45 % summed: second behind ASURANSI JIWA KRESNA (13.77 %), not fifth.
        assert_eq!(kren.holders[1].entity_key, "KRESNA PRIMA INVEST");

        let tguk = db.holders("TGUK", None).unwrap();
        let dinasti = holder(&tguk, "DINASTI KREATIF INDONESIA");
        assert_eq!(dinasti.pct, 69.33);
        assert_eq!(dinasti.shares_total, 2_476_250_000);
        assert_eq!(dinasti.investor_type.as_deref(), Some("CORPORATE"));
        assert_eq!(dinasti.local_foreign.as_deref(), Some("L"));
        assert_eq!(dinasti.holder_raw, "DINASTI KREATIF INDONESIA, PT");
        assert_eq!(dinasti.accounts, Some(2));

        let alka = db.holders("ALKA", None).unwrap();
        let gesit = holder(&alka, "GESIT PERKASA");
        assert_eq!(gesit.pct, 83.8);
        assert_eq!(gesit.domicile.as_deref(), Some("INDONESIA"));
        assert_eq!(gesit.accounts, Some(2));

        let pans = db.holders("PANS", Some(date(2026, 8, 31))).unwrap();
        let panin = holder(&pans, "BANK PAN INDONESIA");
        assert_eq!(panin.pct, 29.0);
        assert_eq!(panin.shares_total, 208_800_000);
        assert_eq!(
            panin.domicile.as_deref(),
            Some("INDONESIA"),
            "from the smaller line"
        );
        // Every other holder has one line.
        assert!(pans
            .holders
            .iter()
            .filter(|h| h.entity_key != "BANK PAN INDONESIA")
            .all(|h| h.accounts == Some(1)));

        let r = db.releases().unwrap();
        let counts: Vec<_> = r
            .iter()
            .map(|r| (r.rows, r.positions, r.other_date_rows, r.controller_column))
            .collect();
        assert_eq!(
            counts,
            vec![
                (45, Some(44), Some(0), Some(false)),
                (46, Some(43), Some(0), Some(false))
            ]
        );
        assert_eq!(r[1].issuers, 4);
        assert!(db
            .release_notes(&[date(2026, 8, 31), date(2026, 9, 30)])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn changes_compare_summed_positions() {
        let db = accounts_db();
        let c = db
            .changes(date(2026, 8, 31), date(2026, 9, 30), None, 0.0, None, 0)
            .unwrap();
        let rows: Vec<(&str, &str, ChangeKind, f64)> = c
            .changes
            .iter()
            .map(|x| {
                (
                    x.symbol.as_str(),
                    x.entity_key.as_str(),
                    x.kind,
                    x.pct_change,
                )
            })
            .collect();
        assert_eq!(c.total_matches, 7, "{rows:?}");
        let find = |sym: &str, key: &str| {
            c.changes
                .iter()
                .find(|x| x.symbol == sym && x.entity_key == key)
                .unwrap_or_else(|| panic!("{sym} {key} missing from {rows:?}"))
        };
        let kresna = find("KREN", "KRESNA PRIMA INVEST");
        assert_eq!(kresna.kind, ChangeKind::Changed);
        assert_eq!(kresna.pct_change, 4.01);
        assert_eq!(kresna.shares_change, 729_400_000);
        assert_eq!(
            (kresna.accounts_from, kresna.accounts_to),
            (Some(1), Some(2))
        );
        let karunia = find("KREN", "KARUNIA PERDANA ISTENIA");
        assert_eq!(
            (karunia.kind, karunia.pct_change),
            (ChangeKind::Exited, -5.13)
        );
        let bank_pan = find("PANS", "BANK PAN INDONESIA");
        assert_eq!(
            (bank_pan.kind, bank_pan.pct_change),
            (ChangeKind::Exited, -29.0)
        );
        assert_eq!(
            (bank_pan.accounts_from, bank_pan.accounts_to),
            (Some(2), None)
        );
        let panin = find("PANS", "PANIN BANK");
        assert_eq!((panin.kind, panin.pct_change), (ChangeKind::New, 29.0));
        assert_eq!(find("ALKA", "WENANTO GUNAWAN").pct_change, 0.03);
        assert_eq!(find("TGUK", "BUDIANTO M KURNIAWAN").pct_change, 0.01);
        let widayanti = find("PANS", "WIDAYANTI DARMAWAN");
        assert_eq!(widayanti.pct_change, 0.0, "same percent, more shares");
        assert_eq!(widayanti.shares_change, 20_500);
        // Scrip moved to scripless with the same total: no change.
        assert!(!rows.iter().any(|r| r.1 == "DINASTI KREATIF INDONESIA"));
        assert!(
            !rows.iter().any(|r| r.1 == "GESIT PERKASA"),
            "83.79 -> 83.80 is rounding"
        );
        let s = &c.summary;
        assert_eq!(
            (s.new_holders, s.exits, s.increased, s.decreased),
            (1, 2, 3, 0)
        );
        assert_eq!(s.net_pp, -1.08);
        let tguk = db
            .changes(
                date(2026, 8, 31),
                date(2026, 9, 30),
                Some("tguk"),
                0.0,
                None,
                0,
            )
            .unwrap();
        assert_eq!(tguk.total_matches, 1);
        assert_eq!(tguk.changes[0].entity_key, "BUDIANTO M KURNIAWAN");
    }

    #[test]
    fn rows_dated_otherwise_are_kept_out_of_the_release() {
        let mut db = OwnershipDb::in_memory().unwrap();
        let parsed = parse_xlsx(MIXED).unwrap();
        let rep = db.import(&parsed, "xlsx", "mixed.xlsx", "m").unwrap();
        assert_eq!(rep.release.as_of, date(2026, 2, 27));
        assert_eq!(rep.release.rows, 5);
        assert_eq!(rep.release.positions, Some(3));
        assert_eq!(rep.release.other_date_rows, Some(2));
        assert_eq!(rep.release.issuers, 2, "CCCC has no line dated 2026-02-27");

        let aaaa = db.holders("AAAA", None).unwrap();
        assert_eq!(aaaa.holders.len(), 2);
        assert_eq!(aaaa.listed_pct, 40.0);
        assert!(aaaa.holders.iter().all(|h| h.holder_raw != "PT EMPAT JAYA"));
        let err = db.holders("CCCC", None).unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(
            err.to_string().contains("only on rows dated 2026-01-30"),
            "{err}"
        );
        let bbbb = db.holders("BBBB", None).unwrap();
        assert_eq!(bbbb.holders[0].local_foreign.as_deref(), Some("A"));
        assert!(
            db.entity("empat jaya", None).is_err(),
            "not part of the release"
        );

        assert_eq!(
            db.release_notes(&[date(2026, 2, 27)]).unwrap(),
            vec!["the 2026-02-27 file also lists 2 rows dated 2026-01-30 (AAAA, CCCC); they are kept but are not part of the 2026-02-27 holdings".to_string()]
        );
        assert!(db.release_notes(&[date(2026, 1, 30)]).unwrap().is_empty());
        let r = &db.releases().unwrap()[0];
        assert_eq!(
            (r.rows, r.positions, r.other_date_rows),
            (5, Some(3), Some(2))
        );
        // The other-dated lines are stored, not dropped.
        let stored: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM holding_lines", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, 5);
    }

    #[test]
    fn controller_flag_is_stored_and_reported() {
        let mut db = loaded();
        db.import(&parse_xlsx(OCT).unwrap(), "xlsx", "oct.xlsx", "o")
            .unwrap();
        let bbca = db.holders("BBCA", None).unwrap();
        assert!(bbca.controller_reported);
        let flag = |key: &str| holder(&bbca, key).controller_or_affiliate.clone();
        assert_eq!(
            flag("DWIMURIA INVESTAMA ANDALAN").as_deref(),
            Some("CONTROLLER")
        );
        assert_eq!(
            flag("CATURGUWIRATNA SUMAPALA").as_deref(),
            Some("AFFILIATE OF THE CONTROLLER")
        );
        assert_eq!(flag("ANTHONI SALIM"), None);
        let zyrx = db.holders("ZYRX", None).unwrap();
        assert_eq!(zyrx.holders.len(), 7);
        assert_eq!(round4(zyrx.listed_pct), 91.16);
        assert_eq!(
            zyrx.holders[0].controller_or_affiliate.as_deref(),
            Some("CONTROLLER")
        );
        let e = db.entity("dwimuria", None).unwrap();
        assert!(e.controller_reported);
        // Earlier releases have no such column.
        let aug = db.holders("AADI", Some(date(2026, 8, 31))).unwrap();
        assert!(!aug.controller_reported);
        assert!(aug
            .holders
            .iter()
            .all(|h| h.controller_or_affiliate.is_none()));
        let r = db.releases().unwrap();
        assert_eq!(r.last().unwrap().controller_column, Some(true));
        assert_eq!(r[0].controller_column, Some(false));
    }

    #[test]
    fn reimport_replaces_lines_and_positions() {
        let mut db = accounts_db();
        let count = |db: &OwnershipDb, table: &str| -> i64 {
            db.conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        };
        let (lines, holdings) = (count(&db, "holding_lines"), count(&db, "holdings"));
        assert_eq!((lines, holdings), (45 + 46, 44 + 43));
        let rep = db
            .import(&parse_xlsx(SEP).unwrap(), "xlsx", "again.xlsx", "s2")
            .unwrap();
        assert!(rep.replaced);
        assert_eq!(rep.release.positions, Some(43));
        assert_eq!(db.releases().unwrap().len(), 2);
        assert_eq!(
            (count(&db, "holding_lines"), count(&db, "holdings")),
            (lines, holdings)
        );
    }

    /// Builds a file the way idx 0.8.1 left it: the 0.8.1 tables, no
    /// user_version, one release whose holdings kept one line per holder.
    fn legacy_file(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute_batch(
            "INSERT INTO releases (id, as_of, format, source, sha256, row_count, imported_at)
               VALUES (1, '2026-09-30', 'xlsx', 'old.xlsx', 'abc', 2, '2026-10-01T00:00:00Z');
             INSERT INTO entities (id, entity_key, display_name)
               VALUES (1, 'KRESNA PRIMA INVEST', 'PT. KRESNA PRIMA INVEST');
             INSERT INTO aliases (raw_name, entity_id) VALUES ('PT. KRESNA PRIMA INVEST', 1);
             INSERT INTO holdings (release_id, symbol, issuer_name, entity_id, holder_raw,
               investor_type, local_foreign, nationality, domicile, shares_scripless, shares_scrip,
               shares_total, pct)
               VALUES (1, 'KREN', 'QUANTUM CLOVERA INVESTAMA Tbk', 1, 'PT. KRESNA PRIMA INVEST',
               'CORPORATE', 'L', NULL, 'INDONESIA', 729400000, 0, 729400000, 4.01);",
        )
        .unwrap();
        assert_eq!(user_version(&conn).unwrap(), 0);
    }

    #[test]
    fn a_0_8_1_database_migrates_and_stays_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ownership.sqlite");
        legacy_file(&path);

        let db = OwnershipDb::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        let tables: i64 = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'holding_lines'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 1);
        let r = db.releases().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].rows, r[0].positions, r[0].controller_column),
            (2, None, None)
        );
        let kren = db.holders("KREN", None).unwrap();
        assert_eq!(kren.holders[0].pct, 4.01);
        assert_eq!(kren.holders[0].accounts, None);
        assert!(!kren.controller_reported);
        let notes = db.release_notes(&[date(2026, 9, 30)]).unwrap();
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].contains("2026-09-30 was imported by idx 0.8.1 or older"),
            "{notes:?}"
        );
        assert!(notes[0].contains("idx ownership import --all --force"));
        drop(db);

        // Opening again is a no-op; re-importing the month makes it current.
        let mut db = OwnershipDb::open(&path).unwrap();
        assert_eq!(db.releases().unwrap().len(), 1);
        db.import(&parse_xlsx(SEP).unwrap(), "xlsx", "sep.xlsx", "s")
            .unwrap();
        let kren = db.holders("KREN", None).unwrap();
        assert_eq!(holder(&kren, "KRESNA PRIMA INVEST").pct, 13.45);
        assert!(db.release_notes(&[date(2026, 9, 30)]).unwrap().is_empty());
        assert_eq!(db.releases().unwrap()[0].positions, Some(43));
    }

    #[test]
    fn a_file_from_a_newer_idx_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ownership.sqlite");
        {
            let db = OwnershipDb::open(&path).unwrap();
            db.conn.pragma_update(None, "user_version", 99).unwrap();
        }
        let err = OwnershipDb::open(&path).err().expect("refused");
        assert!(err.to_string().contains("newer idx"), "{err}");
        assert!(err.to_string().contains("schema 99"), "{err}");
    }

    #[test]
    fn fresh_files_get_the_current_schema() {
        let db = OwnershipDb::in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
        assert!(db.releases().unwrap().is_empty());
        assert!(db.release_notes(&[date(2026, 9, 30)]).unwrap().is_empty());
    }
}
