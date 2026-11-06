//! Entity resolution: rules that merge two holders or move one spelling to
//! another holder (`ownership resolve`), and the resolver every import uses
//! so the rules also hold for months imported later.
//!
//! Rules live in `resolutions` (schema version 3). A merge re-points every
//! spelling and register line of one holder to another; a map re-points one
//! spelling. Releases that have register lines are then rebuilt with
//! [`rebuild_positions`], so a rule and its undo are exact, also where both
//! holders were listed in one issuer (`--fold`): the lines keep their
//! spelling. Releases imported by idx 0.8.1 or older have no lines; their
//! positions are edited in place, and a fold there cannot be split again.

use std::collections::{BTreeSet, HashMap, HashSet};

use chrono::NaiveDate;
use rusqlite::{params, OptionalExtension, Statement, Transaction};

use super::{db_err, parse_date, rebuild_positions, round4, OwnershipDb};
use crate::error::{IdxError, Result};
use crate::model::{IssuerMonth, Resolution, ResolutionAction, ResolutionKind, ResolutionReport};
use crate::ownership::normalize;

/// Version 3: the user's resolution rules.
pub(super) const V3_TABLES: &str = "
CREATE TABLE IF NOT EXISTS resolutions (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,   -- never reused, so `undo N` stays unambiguous
  kind       TEXT NOT NULL CHECK (kind IN ('merge', 'map')),
  subject    TEXT NOT NULL,      -- merge: the entity_key folded away; map: the spelling as printed
  entity_id  INTEGER NOT NULL REFERENCES entities(id),   -- holder it belongs to now
  combined   TEXT NOT NULL DEFAULT '[]',                -- JSON [{as_of, symbol}] where both held a position
  created_at TEXT NOT NULL,
  UNIQUE (kind, subject)
);
";

/// Resolves each printed name of an import to an entity: the spelling's
/// alias first (a map, or any spelling seen before), else the merge rule of
/// its key, else the entity with that key, else a new entity. The spelling
/// is then recorded. Keys resolved without an alias are cached, so a file's
/// thousands of lines cost one lookup per new key.
pub(super) struct EntityResolver<'t> {
    find_alias: Statement<'t>,
    find_merge: Statement<'t>,
    find_entity: Statement<'t>,
    insert_entity: Statement<'t>,
    insert_alias: Statement<'t>,
    by_key: HashMap<String, i64>,
}

impl<'t> EntityResolver<'t> {
    pub(super) fn new(tx: &'t Transaction<'_>) -> Result<Self> {
        let prep = |sql: &str| tx.prepare(sql).map_err(db_err);
        Ok(EntityResolver {
            find_alias: prep("SELECT entity_id FROM aliases WHERE raw_name = ?1")?,
            find_merge: prep(
                "SELECT entity_id FROM resolutions WHERE kind = 'merge' AND subject = ?1",
            )?,
            find_entity: prep("SELECT id FROM entities WHERE entity_key = ?1")?,
            insert_entity: prep("INSERT INTO entities (entity_key, display_name) VALUES (?1, ?2)")?,
            insert_alias: prep(
                "INSERT OR IGNORE INTO aliases (raw_name, entity_id) VALUES (?1, ?2)",
            )?,
            by_key: HashMap::new(),
        })
    }

    pub(super) fn resolve(&mut self, raw: &str) -> Result<i64> {
        let by_alias: Option<i64> = self
            .find_alias
            .query_row([raw], |r| r.get(0))
            .optional()
            .map_err(db_err)?;
        let id = match by_alias {
            Some(id) => id,
            None => {
                let key = normalize::entity_key(raw);
                match self.by_key.get(&key) {
                    Some(id) => *id,
                    None => {
                        let merged: Option<i64> = self
                            .find_merge
                            .query_row([&key], |r| r.get(0))
                            .optional()
                            .map_err(db_err)?;
                        let id = match merged {
                            Some(id) => id,
                            None => {
                                let existing: Option<i64> = self
                                    .find_entity
                                    .query_row([&key], |r| r.get(0))
                                    .optional()
                                    .map_err(db_err)?;
                                match existing {
                                    Some(id) => id,
                                    None => self
                                        .insert_entity
                                        .insert(params![key, normalize::display_name(raw)])
                                        .map_err(db_err)?,
                                }
                            }
                        };
                        self.by_key.insert(key, id);
                        id
                    }
                }
            }
        };
        self.insert_alias
            .execute(params![raw, id])
            .map_err(db_err)?;
        Ok(id)
    }
}

/// Which printed names an undo moves back.
type Pick = Box<dyn Fn(&str) -> bool>;

#[derive(Debug, Clone)]
struct Holder {
    id: i64,
    key: String,
    name: String,
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn entity_by_key(tx: &Transaction, key: &str) -> Result<Option<Holder>> {
    tx.query_row(
        "SELECT id, entity_key, display_name FROM entities WHERE entity_key = ?1",
        [key],
        |r| {
            Ok(Holder {
                id: r.get(0)?,
                key: r.get(1)?,
                name: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(db_err)
}

fn entity_by_id(tx: &Transaction, id: i64) -> Result<Holder> {
    tx.query_row(
        "SELECT id, entity_key, display_name FROM entities WHERE id = ?1",
        [id],
        |r| {
            Ok(Holder {
                id: r.get(0)?,
                key: r.get(1)?,
                name: r.get(2)?,
            })
        },
    )
    .map_err(db_err)
}

/// A holder named on the command line by key or display name. Never a
/// substring match: writes must hit exactly one holder.
fn find_holder(tx: &Transaction, arg: &str) -> Result<Holder> {
    let key = normalize::entity_key(arg);
    if key.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        return Err(IdxError::Usage(format!(
            "{:?} is not a holder; name one by key or display name",
            arg.trim()
        )));
    }
    let merged: Option<(i64, String)> = tx
        .query_row(
            "SELECT r.id, e.display_name FROM resolutions r JOIN entities e ON e.id = r.entity_id
             WHERE r.kind = 'merge' AND r.subject = ?1",
            [&key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(db_err)?;
    if let Some((id, into)) = merged {
        return Err(IdxError::Usage(format!(
            "{key} was merged into {into} (#{id}); use {into}, or undo it with `idx ownership resolve undo {id}`"
        )));
    }
    entity_by_key(tx, &key)?.ok_or_else(|| {
        IdxError::NotFound(format!(
            "no holder with key {key:?}; find it with `idx ownership search`"
        ))
    })
}

/// A recorded spelling: exactly as given, else with spacing collapsed, else
/// the one spelling that matches ignoring case.
fn find_spelling(tx: &Transaction, arg: &str) -> Result<(String, i64)> {
    let mut st = tx
        .prepare("SELECT raw_name, entity_id FROM aliases ORDER BY raw_name")
        .map_err(db_err)?;
    let all = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    let collapse = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(hit) = all.iter().find(|(raw, _)| raw == arg) {
        return Ok(hit.clone());
    }
    let want = collapse(arg);
    let spaced: Vec<_> = all
        .iter()
        .filter(|(raw, _)| collapse(raw) == want)
        .collect();
    if spaced.len() == 1 {
        return Ok(spaced[0].clone());
    }
    let upper = want.to_uppercase();
    let cased: Vec<_> = all
        .iter()
        .filter(|(raw, _)| collapse(raw).to_uppercase() == upper)
        .collect();
    match cased.len() {
        1 => Ok(cased[0].clone()),
        0 => Err(IdxError::NotFound(format!(
            "no spelling {:?} recorded; `idx ownership search` lists each holder's spellings",
            arg.trim()
        ))),
        _ => Err(IdxError::Usage(format!(
            "{:?} matches several spellings ({}); give the one you mean exactly",
            arg.trim(),
            cased
                .iter()
                .map(|(r, _)| format!("{r:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

fn ids(tx: &Transaction, sql: &str, p: impl rusqlite::Params) -> Result<BTreeSet<i64>> {
    let mut st = tx.prepare(sql).map_err(db_err)?;
    let rows = st
        .query_map(p, |r| r.get::<_, i64>(0))
        .map_err(db_err)?
        .collect::<std::result::Result<BTreeSet<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

fn months(tx: &Transaction, sql: &str, p: impl rusqlite::Params) -> Result<Vec<IssuerMonth>> {
    let mut st = tx.prepare(sql).map_err(db_err)?;
    let rows = st
        .query_map(p, |r| {
            Ok(IssuerMonth {
                as_of: parse_date(&r.get::<_, String>(0)?),
                symbol: r.get(1)?,
            })
        })
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

/// Releases imported by idx 0.8.1 or older (no register lines).
fn legacy_releases(tx: &Transaction) -> Result<HashSet<i64>> {
    Ok(
        ids(tx, "SELECT id FROM releases WHERE positions IS NULL", [])?
            .into_iter()
            .collect(),
    )
}

fn dates_of(tx: &Transaction, releases: &BTreeSet<i64>) -> Result<Vec<NaiveDate>> {
    let mut out = Vec::new();
    for id in releases {
        let d: String = tx
            .query_row("SELECT as_of FROM releases WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .map_err(db_err)?;
        out.push(parse_date(&d));
    }
    out.sort();
    Ok(out)
}

/// Issuer-months where both holders have a position.
fn co_held(tx: &Transaction, a: i64, b: i64) -> Result<Vec<IssuerMonth>> {
    months(
        tx,
        "SELECT r.as_of, x.symbol FROM holdings x
         JOIN holdings y ON y.release_id = x.release_id AND y.symbol = x.symbol
         JOIN releases r ON r.id = x.release_id
         WHERE x.entity_id = ?1 AND y.entity_id = ?2
         ORDER BY r.as_of, x.symbol",
        params![a, b],
    )
}

fn fold_refused(co: &[IssuerMonth], moving: &str, into: &str) -> IdxError {
    let mut symbols: Vec<&str> = co.iter().map(|m| m.symbol.as_str()).collect();
    symbols.sort();
    symbols.dedup();
    let mut dates: Vec<String> = co.iter().map(|m| m.as_of.to_string()).collect();
    dates.sort();
    dates.dedup();
    IdxError::Usage(format!(
        "both hold {} in {}: they may be two holders (or one holder's two accounts); pass --fold to add {moving}'s stake to {into}",
        symbols.join(", "),
        dates.join(", ")
    ))
}

fn legacy_note(dates: &[NaiveDate]) -> String {
    let list: Vec<String> = dates.iter().map(|d| d.to_string()).collect();
    let one = list.len() == 1;
    format!(
        "release{} {} {} imported by idx 0.8.1 or older; the rule was applied to {} positions; re-import {} (`idx ownership import --all --force`) to apply it to every register line",
        if one { "" } else { "s" },
        list.join(", "),
        if one { "was" } else { "were" },
        if one { "its" } else { "their" },
        if one { "it" } else { "them" },
    )
}

/// Moves `from`'s positions in a legacy release to `to` where `pick`
/// accepts the printed name; where `to` already has a position in that
/// issuer the two are added up. Returns (moved, combined).
fn move_positions(
    tx: &Transaction,
    release_id: i64,
    from: i64,
    to: i64,
    pick: &dyn Fn(&str) -> bool,
) -> Result<(usize, usize)> {
    type Row = (String, String, Option<i64>, Option<i64>, i64, f64);
    let rows: Vec<Row> = {
        let mut st = tx
            .prepare(
                "SELECT symbol, holder_raw, shares_scripless, shares_scrip, shares_total, pct
                 FROM holdings WHERE release_id = ?1 AND entity_id = ?2 ORDER BY symbol",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map(params![release_id, from], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            })
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows
    };
    let add = |a: Option<i64>, b: Option<i64>| match (a, b) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
    };
    let (mut moved, mut combined) = (0, 0);
    for (symbol, raw, scripless, scrip, total, pct) in rows {
        if !pick(&raw) {
            continue;
        }
        let there: Option<(Option<i64>, Option<i64>, i64, f64)> = tx
            .query_row(
                "SELECT shares_scripless, shares_scrip, shares_total, pct FROM holdings
                 WHERE release_id = ?1 AND symbol = ?2 AND entity_id = ?3",
                params![release_id, symbol, to],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(db_err)?;
        match there {
            Some((s2, c2, t2, p2)) => {
                tx.execute(
                    "UPDATE holdings SET shares_scripless = ?4, shares_scrip = ?5, shares_total = ?6, pct = ?7
                     WHERE release_id = ?1 AND symbol = ?2 AND entity_id = ?3",
                    params![
                        release_id,
                        symbol,
                        to,
                        add(scripless, s2),
                        add(scrip, c2),
                        total + t2,
                        round4(pct + p2)
                    ],
                )
                .map_err(db_err)?;
                tx.execute(
                    "DELETE FROM holdings WHERE release_id = ?1 AND symbol = ?2 AND entity_id = ?3",
                    params![release_id, symbol, from],
                )
                .map_err(db_err)?;
                combined += 1;
            }
            None => {
                tx.execute(
                    "UPDATE holdings SET entity_id = ?3 WHERE release_id = ?1 AND symbol = ?2 AND entity_id = ?4",
                    params![release_id, symbol, to, from],
                )
                .map_err(db_err)?;
            }
        }
        moved += 1;
    }
    Ok((moved, combined))
}

fn insert_rule(
    tx: &Transaction,
    kind: ResolutionKind,
    subject: &str,
    entity_id: i64,
    combined: &[IssuerMonth],
) -> Result<(i64, String)> {
    let created_at = now();
    let kind_s = match kind {
        ResolutionKind::Merge => "merge",
        ResolutionKind::Map => "map",
    };
    tx.execute(
        "INSERT INTO resolutions (kind, subject, entity_id, combined, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(kind, subject) DO UPDATE SET entity_id = excluded.entity_id,
           combined = excluded.combined, created_at = excluded.created_at",
        params![
            kind_s,
            subject,
            entity_id,
            serde_json::to_string(combined)?,
            created_at
        ],
    )
    .map_err(db_err)?;
    let id: i64 = tx
        .query_row(
            "SELECT id FROM resolutions WHERE kind = ?1 AND subject = ?2",
            params![kind_s, subject],
            |r| r.get(0),
        )
        .map_err(db_err)?;
    Ok((id, created_at))
}

fn rebuild_all(tx: &Transaction, releases: &BTreeSet<i64>) -> Result<()> {
    for r in releases {
        rebuild_positions(tx, *r)?;
    }
    Ok(())
}

/// Register-line positions (release, symbol) where `entity` has lines
/// printed `raw` dated the release.
fn spelling_positions(tx: &Transaction, entity: i64, raw: &str) -> Result<usize> {
    tx.query_row(
        "SELECT COUNT(*) FROM (SELECT DISTINCT l.release_id, l.symbol FROM holding_lines l
           JOIN releases r ON r.id = l.release_id
           WHERE l.entity_id = ?1 AND l.holder_raw = ?2 AND l.report_date = r.as_of)",
        params![entity, raw],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n as usize)
    .map_err(db_err)
}

impl OwnershipDb {
    /// `resolve merge KEEP MERGE...`: every spelling and register line of
    /// each MERGE holder joins KEEP, in every imported release and every
    /// later import. Refused (usage) where both are listed in one issuer in
    /// one release unless `fold`. `dry_run` reports and rolls back.
    pub fn resolve_merge(
        &mut self,
        keep: &str,
        merge: &[String],
        fold: bool,
        dry_run: bool,
    ) -> Result<(Vec<ResolutionReport>, Vec<String>)> {
        if merge.is_empty() {
            return Err(IdxError::Usage("name at least one holder to merge".into()));
        }
        let tx = self.conn.transaction().map_err(db_err)?;
        let keeper = find_holder(&tx, keep)?;
        let legacy = legacy_releases(&tx)?;
        let mut reports = Vec::new();
        let mut legacy_touched: BTreeSet<i64> = BTreeSet::new();
        let mut seen: HashSet<i64> = HashSet::new();
        for m in merge {
            let gone = find_holder(&tx, m)?;
            if gone.id == keeper.id {
                return Err(IdxError::Usage(format!(
                    "{} and {} are the same holder",
                    keep.trim(),
                    m.trim()
                )));
            }
            if !seen.insert(gone.id) {
                continue;
            }
            let co = co_held(&tx, keeper.id, gone.id)?;
            if !co.is_empty() && !fold {
                return Err(fold_refused(&co, &gone.name, &keeper.name));
            }
            let positions_moved: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM holdings WHERE entity_id = ?1",
                    [gone.id],
                    |r| r.get(0),
                )
                .map_err(db_err)?;
            let line_releases = ids(
                &tx,
                "SELECT DISTINCT release_id FROM holding_lines WHERE entity_id = ?1",
                [gone.id],
            )?;
            let mut touched = ids(
                &tx,
                "SELECT DISTINCT release_id FROM holdings WHERE entity_id = ?1",
                [gone.id],
            )?;
            touched.extend(line_releases.iter().copied());
            let (id, created_at) =
                insert_rule(&tx, ResolutionKind::Merge, &gone.key, keeper.id, &co)?;
            tx.execute(
                "UPDATE resolutions SET entity_id = ?1 WHERE entity_id = ?2",
                params![keeper.id, gone.id],
            )
            .map_err(db_err)?;
            let spellings_moved = tx
                .execute(
                    "UPDATE aliases SET entity_id = ?1 WHERE entity_id = ?2",
                    params![keeper.id, gone.id],
                )
                .map_err(db_err)?;
            let lines_moved = tx
                .execute(
                    "UPDATE holding_lines SET entity_id = ?1 WHERE entity_id = ?2",
                    params![keeper.id, gone.id],
                )
                .map_err(db_err)?;
            rebuild_all(&tx, &line_releases)?;
            for r in touched.iter().filter(|r| legacy.contains(r)) {
                move_positions(&tx, *r, gone.id, keeper.id, &|_| true)?;
                legacy_touched.insert(*r);
            }
            reports.push(ResolutionReport {
                action: ResolutionAction::Merge,
                dry_run,
                resolution: Resolution {
                    id: (!dry_run).then_some(id),
                    kind: ResolutionKind::Merge,
                    subject: gone.key.clone(),
                    holder: keeper.name.clone(),
                    entity_key: keeper.key.clone(),
                    combined: co.clone(),
                    created_at,
                },
                spellings_moved,
                lines_moved,
                positions_moved: positions_moved as usize,
                positions_combined: co.len(),
                releases: dates_of(&tx, &touched)?,
            });
        }
        let mut warnings = Vec::new();
        if !legacy_touched.is_empty() {
            warnings.push(legacy_note(&dates_of(&tx, &legacy_touched)?));
        }
        if dry_run {
            tx.rollback().map_err(db_err)?;
        } else {
            tx.commit().map_err(db_err)?;
        }
        Ok((reports, warnings))
    }

    /// `resolve map SPELLING HOLDER`: one printed spelling joins HOLDER (an
    /// existing one, or with `new` a new holder named HOLDER).
    pub fn resolve_map(
        &mut self,
        spelling: &str,
        holder: &str,
        new: bool,
        fold: bool,
        dry_run: bool,
    ) -> Result<(ResolutionReport, Vec<String>)> {
        let tx = self.conn.transaction().map_err(db_err)?;
        let (raw, current) = find_spelling(&tx, spelling)?;
        let target = if new {
            let key = normalize::entity_key(holder);
            if key.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
                return Err(IdxError::Usage(format!(
                    "{:?} cannot name a holder",
                    holder.trim()
                )));
            }
            if entity_by_key(&tx, &key)?.is_some() {
                return Err(IdxError::Usage(format!(
                    "a holder with key {key:?} already exists; drop --new to map onto it"
                )));
            }
            let name = normalize::display_name(holder);
            tx.execute(
                "INSERT INTO entities (entity_key, display_name) VALUES (?1, ?2)",
                params![key, name],
            )
            .map_err(db_err)?;
            Holder {
                id: tx.last_insert_rowid(),
                key,
                name,
            }
        } else {
            find_holder(&tx, holder)?
        };
        if target.id == current {
            return Err(IdxError::Usage(format!(
                "{raw:?} already belongs to {}",
                target.name
            )));
        }
        let co = months(
            &tx,
            "SELECT DISTINCT r.as_of, l.symbol FROM holding_lines l
             JOIN releases r ON r.id = l.release_id
             JOIN holdings h ON h.release_id = l.release_id AND h.symbol = l.symbol AND h.entity_id = ?2
             WHERE l.entity_id = ?1 AND l.holder_raw = ?3 AND l.report_date = r.as_of
             UNION
             SELECT r.as_of, x.symbol FROM holdings x
             JOIN releases r ON r.id = x.release_id AND r.positions IS NULL
             JOIN holdings y ON y.release_id = x.release_id AND y.symbol = x.symbol AND y.entity_id = ?2
             WHERE x.entity_id = ?1 AND x.holder_raw = ?3
             ORDER BY 1, 2",
            params![current, target.id, raw],
        )?;
        if !co.is_empty() && !fold {
            return Err(fold_refused(&co, &raw, &target.name));
        }
        let legacy = legacy_releases(&tx)?;
        let line_releases = ids(
            &tx,
            "SELECT DISTINCT release_id FROM holding_lines WHERE entity_id = ?1 AND holder_raw = ?2",
            params![current, raw],
        )?;
        let legacy_rows = ids(
            &tx,
            "SELECT DISTINCT release_id FROM holdings WHERE entity_id = ?1 AND holder_raw = ?2",
            params![current, raw],
        )?;
        let legacy_rows: BTreeSet<i64> = legacy_rows
            .into_iter()
            .filter(|r| legacy.contains(r))
            .collect();
        let mut positions_moved = spelling_positions(&tx, current, &raw)?;
        let (id, created_at) = insert_rule(&tx, ResolutionKind::Map, &raw, target.id, &co)?;
        let spellings_moved = tx
            .execute(
                "UPDATE aliases SET entity_id = ?1 WHERE raw_name = ?2",
                params![target.id, raw],
            )
            .map_err(db_err)?;
        let lines_moved = tx
            .execute(
                "UPDATE holding_lines SET entity_id = ?1 WHERE entity_id = ?2 AND holder_raw = ?3",
                params![target.id, current, raw],
            )
            .map_err(db_err)?;
        rebuild_all(&tx, &line_releases)?;
        for r in &legacy_rows {
            let (moved, _) = move_positions(&tx, *r, current, target.id, &|h| h == raw)?;
            positions_moved += moved;
        }
        let mut touched = line_releases.clone();
        touched.extend(legacy_rows.iter().copied());
        let mut warnings = Vec::new();
        if !legacy_rows.is_empty() {
            warnings.push(legacy_note(&dates_of(&tx, &legacy_rows)?));
        }
        if lines_moved == 0 && legacy_rows.is_empty() {
            warnings.push(format!(
                "{raw:?} has no register line in the imported releases; the rule applies to later imports"
            ));
        }
        let report = ResolutionReport {
            action: ResolutionAction::Map,
            dry_run,
            resolution: Resolution {
                id: (!dry_run).then_some(id),
                kind: ResolutionKind::Map,
                subject: raw.clone(),
                holder: target.name.clone(),
                entity_key: target.key.clone(),
                combined: co.clone(),
                created_at,
            },
            spellings_moved,
            lines_moved,
            positions_moved,
            positions_combined: co.len(),
            releases: dates_of(&tx, &touched)?,
        };
        if dry_run {
            tx.rollback().map_err(db_err)?;
        } else {
            tx.commit().map_err(db_err)?;
        }
        Ok((report, warnings))
    }

    /// `resolve list`: every rule, oldest first.
    pub fn resolutions(&self) -> Result<Vec<Resolution>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT r.id, r.kind, r.subject, e.display_name, e.entity_key, r.combined, r.created_at
                 FROM resolutions r JOIN entities e ON e.id = r.entity_id ORDER BY r.id",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        rows.into_iter()
            .map(|(id, kind, subject, holder, key, combined, created_at)| {
                Ok(Resolution {
                    id: Some(id),
                    kind: if kind == "merge" {
                        ResolutionKind::Merge
                    } else {
                        ResolutionKind::Map
                    },
                    subject,
                    holder,
                    entity_key: key,
                    combined: serde_json::from_str(&combined).unwrap_or_default(),
                    created_at,
                })
            })
            .collect()
    }

    /// `resolve undo ID`: deletes the rule and moves what it moved back:
    /// a merge returns the spellings and lines whose key is the merged one
    /// (except spellings pinned by a map), a map returns its spelling to
    /// where its key leads. Releases with lines are rebuilt, so folded
    /// positions split again exactly.
    pub fn resolve_undo(
        &mut self,
        id: i64,
        dry_run: bool,
    ) -> Result<(ResolutionReport, Vec<String>)> {
        let rule = self
            .resolutions()?
            .into_iter()
            .find(|r| r.id == Some(id))
            .ok_or_else(|| {
                IdxError::NotFound(format!(
                    "no resolution #{id}; `idx ownership resolve list` shows them"
                ))
            })?;
        let tx = self.conn.transaction().map_err(db_err)?;
        let a = entity_by_key(&tx, &rule.entity_key)?
            .ok_or_else(|| IdxError::Other(format!("resolution #{id}: its holder is missing")))?;
        tx.execute("DELETE FROM resolutions WHERE id = ?1", [id])
            .map_err(db_err)?;
        let legacy = legacy_releases(&tx)?;
        let mut warnings = Vec::new();
        let (target, pick): (Holder, Pick) = match rule.kind {
            ResolutionKind::Merge => {
                let k = rule.subject.clone();
                let b = match entity_by_key(&tx, &k)? {
                    Some(b) => b,
                    None => {
                        let name = normalize::display_name(&k);
                        tx.execute(
                            "INSERT INTO entities (entity_key, display_name) VALUES (?1, ?2)",
                            params![k, name],
                        )
                        .map_err(db_err)?;
                        Holder {
                            id: tx.last_insert_rowid(),
                            key: k.clone(),
                            name,
                        }
                    }
                };
                let pinned: HashSet<String> = {
                    let mut st = tx
                        .prepare("SELECT subject FROM resolutions WHERE kind = 'map'")
                        .map_err(db_err)?;
                    let rows = st
                        .query_map([], |r| r.get::<_, String>(0))
                        .map_err(db_err)?
                        .collect::<std::result::Result<HashSet<_>, _>>()
                        .map_err(db_err)?;
                    rows
                };
                (
                    b,
                    Box::new(move |raw: &str| {
                        normalize::entity_key(raw) == k && !pinned.contains(raw)
                    }),
                )
            }
            ResolutionKind::Map => {
                let raw = rule.subject.clone();
                let key = normalize::entity_key(&raw);
                let merged: Option<i64> = tx
                    .query_row(
                        "SELECT entity_id FROM resolutions WHERE kind = 'merge' AND subject = ?1",
                        [&key],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(db_err)?;
                let t = match merged {
                    Some(id) => entity_by_id(&tx, id)?,
                    None => match entity_by_key(&tx, &key)? {
                        Some(h) => h,
                        None => {
                            let name = normalize::display_name(&raw);
                            tx.execute(
                                "INSERT INTO entities (entity_key, display_name) VALUES (?1, ?2)",
                                params![key, name],
                            )
                            .map_err(db_err)?;
                            Holder {
                                id: tx.last_insert_rowid(),
                                key: key.clone(),
                                name,
                            }
                        }
                    },
                };
                (t, Box::new(move |h: &str| h == raw))
            }
        };
        let (mut spellings_moved, mut lines_moved, mut positions_moved) = (0, 0, 0);
        let mut line_releases: BTreeSet<i64> = BTreeSet::new();
        let mut legacy_touched: BTreeSet<i64> = BTreeSet::new();
        if target.id != a.id {
            let spellings: Vec<String> = {
                let mut st = tx
                    .prepare("SELECT raw_name FROM aliases WHERE entity_id = ?1")
                    .map_err(db_err)?;
                let rows = st
                    .query_map([a.id], |r| r.get::<_, String>(0))
                    .map_err(db_err)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(db_err)?;
                rows
            };
            for raw in spellings.iter().filter(|r| pick(r)) {
                spellings_moved += tx
                    .execute(
                        "UPDATE aliases SET entity_id = ?1 WHERE raw_name = ?2",
                        params![target.id, raw],
                    )
                    .map_err(db_err)?;
            }
            let lines: Vec<(i64, i64, String, String, bool)> = {
                let mut st = tx
                    .prepare(
                        "SELECT l.release_id, l.line, l.holder_raw, l.symbol, l.report_date = r.as_of
                         FROM holding_lines l JOIN releases r ON r.id = l.release_id
                         WHERE l.entity_id = ?1",
                    )
                    .map_err(db_err)?;
                let rows = st
                    .query_map([a.id], |r| {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
                    })
                    .map_err(db_err)?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(db_err)?;
                rows
            };
            let mut positions: BTreeSet<(i64, String)> = BTreeSet::new();
            for (release, line, _, symbol, dated) in lines.iter().filter(|l| pick(&l.2)) {
                lines_moved += tx
                    .execute(
                        "UPDATE holding_lines SET entity_id = ?1 WHERE release_id = ?2 AND line = ?3",
                        params![target.id, release, line],
                    )
                    .map_err(db_err)?;
                line_releases.insert(*release);
                if *dated {
                    positions.insert((*release, symbol.clone()));
                }
            }
            positions_moved += positions.len();
            rebuild_all(&tx, &line_releases)?;
            let legacy_with_a = ids(
                &tx,
                "SELECT DISTINCT release_id FROM holdings WHERE entity_id = ?1",
                [a.id],
            )?;
            for r in legacy_with_a.iter().filter(|r| legacy.contains(r)) {
                let (moved, _) = move_positions(&tx, *r, a.id, target.id, &*pick)?;
                if moved > 0 {
                    positions_moved += moved;
                    legacy_touched.insert(*r);
                }
            }
        }
        let legacy_dates: HashSet<NaiveDate> = dates_of(&tx, &legacy.iter().copied().collect())?
            .into_iter()
            .collect();
        let (split, stuck): (Vec<&IssuerMonth>, Vec<&IssuerMonth>) = rule
            .combined
            .iter()
            .partition(|m| !legacy_dates.contains(&m.as_of));
        if !stuck.is_empty() {
            warnings.push(format!(
                "positions in {} were combined in a release imported by idx 0.8.1 or older and cannot be split there; re-import it (`idx ownership import --all --force`)",
                stuck
                    .iter()
                    .map(|m| format!("{} ({})", m.symbol, m.as_of))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !legacy_touched.is_empty() {
            warnings.push(legacy_note(&dates_of(&tx, &legacy_touched)?));
        }
        let mut touched = line_releases.clone();
        touched.extend(legacy_touched.iter().copied());
        let report = ResolutionReport {
            action: ResolutionAction::Undo,
            dry_run,
            resolution: rule.clone(),
            spellings_moved,
            lines_moved,
            positions_moved,
            positions_combined: split.len(),
            releases: dates_of(&tx, &touched)?,
        };
        if dry_run {
            tx.rollback().map_err(db_err)?;
        } else {
            tx.commit().map_err(db_err)?;
        }
        Ok((report, warnings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ownership::ingest::{parse_xlsx, Parsed, Row};

    const AUG: &[u8] =
        include_bytes!("../../../tests/fixtures/ownership/above1_20260831_analytics_excerpt.xlsx");
    const SEP: &[u8] =
        include_bytes!("../../../tests/fixtures/ownership/above1_20260930_analytics_excerpt.xlsx");

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn excerpts() -> OwnershipDb {
        let mut db = OwnershipDb::in_memory().unwrap();
        db.import(&parse_xlsx(AUG).unwrap(), "xlsx", "aug.xlsx", "a")
            .unwrap();
        db.import(&parse_xlsx(SEP).unwrap(), "xlsx", "sep.xlsx", "s")
            .unwrap();
        db
    }

    /// Change rows between the two excerpts whose holder key contains `word`.
    fn change_rows(db: &OwnershipDb, word: &str) -> usize {
        db.changes(date(2026, 8, 31), date(2026, 9, 30), None, 0.0, None, 0)
            .unwrap()
            .changes
            .iter()
            .filter(|c| c.entity_key.contains(word))
            .count()
    }

    fn count(db: &OwnershipDb, sql: &str) -> i64 {
        db.conn.query_row(sql, [], |r| r.get(0)).unwrap()
    }

    fn wgsh(db: &OwnershipDb, as_of: NaiveDate, key: &str) -> Option<(f64, i64, Option<u32>)> {
        db.holders("WGSH", Some(as_of))
            .unwrap()
            .holders
            .into_iter()
            .find(|h| h.entity_key == key)
            .map(|h| (h.pct, h.shares_total, h.accounts))
    }

    #[test]
    fn a_merge_moves_every_month_survives_a_reimport_and_undoes() {
        let mut db = excerpts();
        assert_eq!(change_rows(&db, "SURONO"), 6);
        let (dry, _) = db
            .resolve_merge(
                "DRS SURONO SUBEKTI",
                &["SURONO SUBEKTI".into()],
                false,
                true,
            )
            .unwrap();
        assert!(dry[0].dry_run);
        assert_eq!(dry[0].resolution.id, None);
        assert_eq!(change_rows(&db, "SURONO"), 6, "a dry run writes nothing");
        assert!(db.resolutions().unwrap().is_empty());

        let (done, warnings) = db
            .resolve_merge(
                "drs. surono subekti",
                &["Surono Subekti".into()],
                false,
                false,
            )
            .unwrap();
        assert!(warnings.is_empty());
        let r = &done[0];
        assert_eq!(r.resolution.id, Some(1));
        assert_eq!(r.resolution.subject, "SURONO SUBEKTI");
        assert_eq!(r.resolution.entity_key, "DRS SURONO SUBEKTI");
        assert_eq!(
            (
                r.spellings_moved,
                r.lines_moved,
                r.positions_moved,
                r.positions_combined
            ),
            (1, 3, 3, 0)
        );
        assert_eq!(r.releases, [date(2026, 8, 31)]);
        assert_eq!(
            change_rows(&db, "SURONO"),
            0,
            "identical shares both months"
        );
        let aug = db
            .entity_matching("DRS SURONO SUBEKTI", Some(date(2026, 8, 31)), true)
            .unwrap();
        assert_eq!(aug.holdings.len(), 3);
        let (found, _) = db.search("surono", None, None).unwrap();
        assert_eq!(
            found.matches.len(),
            1,
            "the merged holder has no positions left"
        );
        assert_eq!(found.matches[0].spellings.len(), 2);

        // Merged away: refused by name, still reserved.
        let err = db
            .resolve_merge(
                "SURONO SUBEKTI",
                &["DRS SURONO SUBEKTI".into()],
                false,
                false,
            )
            .unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(
            err.to_string()
                .contains("was merged into DRS SURONO SUBEKTI (#1)"),
            "{err}"
        );

        // Re-importing a month applies the rule again.
        db.import(&parse_xlsx(AUG).unwrap(), "xlsx", "aug.xlsx", "a2")
            .unwrap();
        assert_eq!(change_rows(&db, "SURONO"), 0);

        let (undone, _) = db.resolve_undo(1, false).unwrap();
        assert_eq!(undone.action, ResolutionAction::Undo);
        assert_eq!(
            (
                undone.spellings_moved,
                undone.lines_moved,
                undone.positions_moved
            ),
            (1, 3, 3)
        );
        assert_eq!(change_rows(&db, "SURONO"), 6);
        assert!(db.resolutions().unwrap().is_empty());
        assert_eq!(db.resolve_undo(1, false).unwrap_err().exit_code(), 4);
    }

    #[test]
    fn listed_together_needs_fold_and_undo_splits_exactly() {
        let mut db = excerpts();
        let keep = "WALDEN GLOBAL SERVICES";
        let gone = "WALDEN GLOBAL SERVICES PTE LTD";
        let err = db
            .resolve_merge(keep, &[gone.into()], false, false)
            .unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(
            err.to_string()
                .contains("both hold WGSH in 2026-08-31, 2026-09-30"),
            "{err}"
        );
        assert!(err.to_string().contains("--fold"));

        let (done, _) = db.resolve_merge(keep, &[gone.into()], true, false).unwrap();
        assert_eq!(done[0].positions_combined, 2);
        assert_eq!(done[0].resolution.combined.len(), 2);
        for d in [date(2026, 8, 31), date(2026, 9, 30)] {
            assert_eq!(
                wgsh(&db, d, keep),
                Some((35.0, 729_760_698, Some(2))),
                "{d}"
            );
            assert_eq!(wgsh(&db, d, gone), None);
        }
        let id = done[0].resolution.id.unwrap();
        let (undone, w) = db.resolve_undo(id, false).unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(undone.positions_combined, 2);
        let sep = date(2026, 9, 30);
        assert_eq!(wgsh(&db, sep, gone), Some((1.86, 38_800_000, Some(1))));
        assert_eq!(wgsh(&db, sep, keep), Some((33.14, 690_960_698, Some(1))));
    }

    #[test]
    fn a_map_moves_one_spelling_and_back() {
        let mut db = excerpts();
        assert_eq!(change_rows(&db, "PERMADI"), 6);
        let (r, w) = db
            .resolve_map(
                "IR. T. PERMADI RACHMAT",
                "T PERMADI RACHMAT",
                false,
                false,
                false,
            )
            .unwrap();
        assert!(w.is_empty());
        assert_eq!(r.resolution.kind, ResolutionKind::Map);
        assert_eq!(r.resolution.subject, "IR. T. PERMADI RACHMAT");
        assert_eq!(
            (r.spellings_moved, r.lines_moved, r.positions_moved),
            (1, 3, 3)
        );
        assert_eq!(change_rows(&db, "PERMADI"), 0);
        // Case and spacing do not matter when the spelling is unique.
        let again = db
            .resolve_map(
                "ir.  t. permadi rachmat",
                "T PERMADI RACHMAT",
                false,
                false,
                false,
            )
            .unwrap_err();
        assert!(again.to_string().contains("already belongs to"), "{again}");
        let (undone, _) = db.resolve_undo(r.resolution.id.unwrap(), false).unwrap();
        assert_eq!((undone.lines_moved, undone.positions_moved), (3, 3));
        assert_eq!(change_rows(&db, "PERMADI"), 6);
        let back = db
            .entity_matching("IR T PERMADI RACHMAT", Some(date(2026, 8, 31)), true)
            .unwrap();
        assert_eq!(back.holdings.len(), 3);

        assert_eq!(
            db.resolve_map("NO SUCH NAME", "T PERMADI RACHMAT", false, false, false)
                .unwrap_err()
                .exit_code(),
            4
        );
        assert_eq!(
            db.resolve_map(
                "IR. T. PERMADI RACHMAT",
                "T PERMADI RACHMAT",
                true,
                false,
                false
            )
            .unwrap_err()
            .exit_code(),
            2,
            "--new with an existing key"
        );
        assert_eq!(
            db.resolve_merge("NOBODY HERE", &["T PERMADI RACHMAT".into()], false, false)
                .unwrap_err()
                .exit_code(),
            4
        );
        assert_eq!(
            db.resolve_merge(
                "T PERMADI RACHMAT",
                &["t permadi rachmat".into()],
                false,
                false
            )
            .unwrap_err()
            .exit_code(),
            2
        );
    }

    fn row(line: u32, as_of: NaiveDate, symbol: &str, raw: &str, shares: i64, pct: f64) -> Row {
        Row {
            line,
            as_of,
            symbol: symbol.into(),
            issuer_name: Some(format!("{symbol} Tbk")),
            holder_raw: raw.into(),
            investor_type: Some("CORPORATE".into()),
            local_foreign: Some("L".into()),
            nationality: None,
            domicile: None,
            shares_scripless: Some(shares),
            shares_scrip: Some(0),
            shares_total: shares,
            pct,
            controller_or_affiliate: None,
        }
    }

    fn parsed(as_of: NaiveDate, rows: Vec<Row>) -> Parsed {
        Parsed {
            as_of,
            rows,
            skipped: vec![],
            controller_column: false,
        }
    }

    #[test]
    fn later_imports_follow_the_rules() {
        let mut db = excerpts();
        db.resolve_merge(
            "DRS SURONO SUBEKTI",
            &["SURONO SUBEKTI".into()],
            false,
            false,
        )
        .unwrap();
        let entities = count(&db, "SELECT COUNT(*) FROM entities");
        // A new spelling whose key was merged away joins the kept holder.
        let oct = date(2026, 10, 31);
        db.import(
            &parsed(
                oct,
                vec![row(7, oct, "OBAT", "Surono  Subekti,", 1_000, 14.57)],
            ),
            "xlsx",
            "oct.xlsx",
            "o",
        )
        .unwrap();
        assert_eq!(count(&db, "SELECT COUNT(*) FROM entities"), entities);
        let e = db
            .entity_matching("DRS SURONO SUBEKTI", Some(oct), true)
            .unwrap();
        assert_eq!(e.holdings[0].holder_raw, "Surono  Subekti,");

        // The spelling is looked up before the key, so a mapped spelling and
        // an unmapped one with the same key part ways.
        let nov = date(2026, 11, 30);
        db.import(
            &parsed(
                nov,
                vec![
                    row(7, nov, "AAAA", "PT XENON NUSA", 100, 5.0),
                    row(8, nov, "BBBB", "PT. XENON NUSA", 200, 6.0),
                ],
            ),
            "xlsx",
            "nov.xlsx",
            "n",
        )
        .unwrap();
        db.resolve_map("PT. XENON NUSA", "YOGA NUSA", true, false, false)
            .unwrap();
        let dec = date(2026, 12, 31);
        db.import(
            &parsed(
                dec,
                vec![
                    row(7, dec, "CCCC", "PT XENON NUSA", 100, 5.0),
                    row(8, dec, "DDDD", "PT. XENON NUSA", 200, 6.0),
                    row(9, dec, "EEEE", "XENON NUSA, PT", 300, 7.0),
                ],
            ),
            "xlsx",
            "dec.xlsx",
            "d",
        )
        .unwrap();
        let xenon = db.entity_matching("XENON NUSA", Some(dec), true).unwrap();
        let syms: Vec<&str> = xenon.holdings.iter().map(|h| h.symbol.as_str()).collect();
        assert_eq!(syms, ["EEEE", "CCCC"]);
        let yoga = db.entity_matching("YOGA NUSA", Some(dec), true).unwrap();
        assert_eq!(yoga.holdings.len(), 1);
        assert_eq!(yoga.holdings[0].symbol, "DDDD");
        // And the November release was re-pointed when the map was made.
        let yoga_nov = db.entity_matching("YOGA NUSA", Some(nov), true).unwrap();
        assert_eq!(yoga_nov.holdings[0].symbol, "BBBB");
    }

    #[test]
    fn legacy_releases_are_edited_in_place_with_a_warning() {
        let mut db = OwnershipDb::in_memory().unwrap();
        db.import(&parse_xlsx(SEP).unwrap(), "xlsx", "sep.xlsx", "s")
            .unwrap();
        let id = |key: &str| -> i64 {
            db.conn
                .query_row(
                    "SELECT id FROM entities WHERE entity_key = ?1",
                    [key],
                    |r| r.get(0),
                )
                .unwrap()
        };
        let (keep, gone) = (
            id("WALDEN GLOBAL SERVICES"),
            id("WALDEN GLOBAL SERVICES PTE LTD"),
        );
        // A July release the way idx 0.8.1 stored it: positions only, no lines.
        db.conn
            .execute_batch(&format!(
                "INSERT INTO releases (id, as_of, format, source, sha256, row_count, imported_at)
                   VALUES (90, '2026-07-31', 'xlsx', 'old.xlsx', 'x', 2, '2026-08-01T00:00:00Z');
                 INSERT INTO holdings (release_id, symbol, issuer_name, entity_id, holder_raw,
                   investor_type, local_foreign, shares_scripless, shares_scrip, shares_total, pct)
                 VALUES (90, 'WGSH', 'WIRA GLOBAL SOLUSI Tbk', {keep}, 'WALDEN GLOBAL SERVICES',
                   'CORPORATE', 'L', 690960698, 0, 690960698, 33.14),
                        (90, 'WGSH', 'WIRA GLOBAL SOLUSI Tbk', {gone}, 'WALDEN GLOBAL SERVICES PTE LTD',
                   'CORPORATE', 'A', 38800000, NULL, 38800000, 1.86);"
            ))
            .unwrap();
        let jul = date(2026, 7, 31);
        let (done, w) = db
            .resolve_merge(
                "WALDEN GLOBAL SERVICES",
                &["WALDEN GLOBAL SERVICES PTE LTD".into()],
                true,
                false,
            )
            .unwrap();
        assert_eq!(done[0].releases, [jul, date(2026, 9, 30)]);
        assert_eq!(done[0].positions_combined, 2);
        assert_eq!(w.len(), 1);
        assert!(
            w[0].contains("release 2026-07-31 was imported by idx 0.8.1 or older"),
            "{w:?}"
        );
        assert_eq!(
            wgsh(&db, jul, "WALDEN GLOBAL SERVICES"),
            Some((35.0, 729_760_698, None))
        );
        let row: (Option<i64>, Option<i64>) = db
            .conn
            .query_row(
                "SELECT shares_scripless, shares_scrip FROM holdings WHERE release_id = 90",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, (Some(729_760_698), Some(0)), "a null side counts as 0");

        let (_, w) = db
            .resolve_undo(done[0].resolution.id.unwrap(), false)
            .unwrap();
        assert!(
            w.iter()
                .any(|w| w.contains("WGSH (2026-07-31)") && w.contains("cannot be split")),
            "{w:?}"
        );
        // The register-line release splits again; the legacy one cannot.
        assert!(wgsh(&db, date(2026, 9, 30), "WALDEN GLOBAL SERVICES PTE LTD").is_some());
        assert!(wgsh(&db, jul, "WALDEN GLOBAL SERVICES PTE LTD").is_none());
    }
}
