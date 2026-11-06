//! SQLite storage for the shareholder register.
//!
//! Schema (one file, WAL mode):
//! - `releases`  one row per imported monthly file, unique on `as_of`
//! - `entities`  one row per normalised holder key, with the display name
//! - `aliases`   every raw spelling seen, pointing at its entity
//! - `holdings`  one row per (release, symbol, entity)
//!
//! Queries never touch the network. Importing the same `as_of` twice replaces
//! the earlier rows for that release.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{IdxError, Result};
use crate::model::{
    Change, ChangeBrief, ChangeKind, ChangesReport, ChangesSummary, EntityReport,
    ForeignFlowReport, ForeignPoint, ForeignReport, HoldersReport, Holding, ImportReport,
    IssuerFlow, IssuerForeign, Release, TypeFlow,
};

use super::ingest::Parsed;
use super::normalize;

const SCHEMA: &str = "
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

pub struct OwnershipDb {
    conn: Connection,
    path: PathBuf,
}

fn db_err(e: rusqlite::Error) -> IdxError {
    IdxError::Other(format!("ownership database: {e}"))
}

impl OwnershipDb {
    /// Opens (and creates) the database at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path).map_err(db_err)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        Ok(OwnershipDb {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// In-memory database for tests.
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(db_err)?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(db_err)?;
        conn.execute_batch(SCHEMA).map_err(db_err)?;
        Ok(OwnershipDb {
            conn,
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Stores a parsed file as the release for its `as_of`, replacing any
    /// earlier import of the same date.
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
            tx.execute("DELETE FROM releases WHERE id = ?1", [id])
                .map_err(db_err)?;
        }
        let imported_at = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
        tx.execute(
            "INSERT INTO releases (as_of, format, source, sha256, row_count, imported_at) VALUES (?1,?2,?3,?4,?5,?6)",
            params![as_of, format, source, sha256, parsed.rows.len() as i64, imported_at],
        )
        .map_err(db_err)?;
        let release_id = tx.last_insert_rowid();

        let mut entity_cache: HashMap<String, i64> = HashMap::new();
        let mut entities_touched = std::collections::HashSet::new();
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
            let mut insert_holding = tx
                .prepare(
                    "INSERT OR REPLACE INTO holdings (release_id, symbol, issuer_name, entity_id, holder_raw,
                     investor_type, local_foreign, nationality, domicile, shares_scripless, shares_scrip,
                     shares_total, pct) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
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
                entities_touched.insert(entity_id);
                insert_holding
                    .execute(params![
                        release_id,
                        r.symbol,
                        r.issuer_name,
                        entity_id,
                        r.holder_raw,
                        r.investor_type,
                        r.local_foreign,
                        r.nationality,
                        r.domicile,
                        r.shares_scripless,
                        r.shares_scrip,
                        r.shares_total,
                        r.pct,
                    ])
                    .map_err(db_err)?;
            }
        }
        tx.commit().map_err(db_err)?;
        Ok(ImportReport {
            release: Release {
                as_of: parsed.as_of,
                format: format.to_string(),
                source: source.to_string(),
                sha256: sha256.to_string(),
                rows: parsed.rows.len() as u64,
                issuers: parsed.issuers() as u64,
                entities: entities_touched.len() as u64,
                imported_at,
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
                        (SELECT COUNT(DISTINCT entity_id) FROM holdings h WHERE h.release_id = r.id)
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
                })
            })
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
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
                    h.shares_total, h.pct
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
        let holders =
            self.holdings_where(release_id, date, "h.symbol = ?2", &symbol.to_uppercase())?;
        if holders.is_empty() {
            return Err(IdxError::NotFound(format!(
                "no holders above 1% recorded for {symbol} as of {date}"
            )));
        }
        Ok(HoldersReport {
            symbol: symbol.to_uppercase(),
            issuer_name: holders.iter().find_map(|h| h.issuer_name.clone()),
            as_of: date,
            listed_pct: holders.iter().map(|h| h.pct).sum(),
            holders,
        })
    }

    /// Every position of holders whose normalised name contains `query`.
    /// The query needs at least 3 letters or digits once normalised: `%%%` or
    /// `!!!` would otherwise normalise to an empty key and match everyone.
    pub fn entity(&self, query: &str, as_of: Option<NaiveDate>) -> Result<EntityReport> {
        let key = normalize::entity_key(query);
        if key.chars().filter(|c| c.is_alphanumeric()).count() < 3 {
            return Err(IdxError::Usage(
                "name needs at least 3 letters or digits".into(),
            ));
        }
        let (release_id, date) = self.release_for(as_of)?;
        let pattern = format!("%{key}%");
        let holdings = self.holdings_where(release_id, date, "e.entity_key LIKE ?2", &pattern)?;
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
        })
    }

    /// Movements between two releases: every match is summarised, then one
    /// page (`offset`, `limit`) of rows is returned, largest moves first.
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
                          h.local_foreign
                   FROM holdings h JOIN entities e ON e.id = h.entity_id
                   WHERE h.release_id = ?1 AND (?2 IS NULL OR h.symbol = ?2)";
        let load = |release_id: i64| -> Result<Positions> {
            let mut st = self.conn.prepare(sql).map_err(db_err)?;
            let rows = st
                .query_map(params![release_id, symbol], |row| {
                    Ok((
                        (row.get::<_, String>(0)?, row.get::<_, String>(2)?),
                        (
                            row.get::<_, String>(1)?,
                            row.get::<_, Option<String>>(3)?,
                            row.get::<_, i64>(4)?,
                            row.get::<_, f64>(5)?,
                            row.get::<_, Option<String>>(6)?,
                        ),
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
        for ((sym, key), (name, itype, shares_to, pct_to, lf)) in &after {
            match before.get(&(sym.clone(), key.clone())) {
                Some((_, _, shares_from, pct_from, _)) => {
                    let pct_change = pct_to - pct_from;
                    if (pct_change).abs() < 1e-9 && shares_to == shares_from {
                        continue;
                    }
                    changes.push(Change {
                        symbol: sym.clone(),
                        holder: name.clone(),
                        entity_key: key.clone(),
                        investor_type: itype.clone(),
                        local_foreign: lf.clone(),
                        kind: ChangeKind::Changed,
                        pct_from: Some(*pct_from),
                        pct_to: Some(*pct_to),
                        pct_change: round4(pct_change),
                        shares_from: Some(*shares_from),
                        shares_to: Some(*shares_to),
                        shares_change: shares_to - shares_from,
                    });
                }
                None => changes.push(Change {
                    symbol: sym.clone(),
                    holder: name.clone(),
                    entity_key: key.clone(),
                    investor_type: itype.clone(),
                    local_foreign: lf.clone(),
                    kind: ChangeKind::New,
                    pct_from: None,
                    pct_to: Some(*pct_to),
                    pct_change: round4(*pct_to),
                    shares_from: None,
                    shares_to: Some(*shares_to),
                    shares_change: *shares_to,
                }),
            }
        }
        for ((sym, key), (name, itype, shares_from, pct_from, lf)) in &before {
            if !after.contains_key(&(sym.clone(), key.clone())) {
                changes.push(Change {
                    symbol: sym.clone(),
                    holder: name.clone(),
                    entity_key: key.clone(),
                    investor_type: itype.clone(),
                    local_foreign: lf.clone(),
                    kind: ChangeKind::Exited,
                    pct_from: Some(*pct_from),
                    pct_to: None,
                    pct_change: round4(-pct_from),
                    shares_from: Some(*shares_from),
                    shares_to: None,
                    shares_change: -shares_from,
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

/// (symbol, entity_key) -> (display name, investor type, shares, pct, local/foreign).
type Positions = BTreeMap<(String, String), (String, Option<String>, i64, f64, Option<String>)>;

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
}
