//! KSEI's monthly "Kepemilikan Efek" archive: the composition of every
//! security's holders by investor type, local and foreign.
//!
//! KSEI publishes `BalanceposEfek<YYYYMMDD>.zip` on ksei.co.id (see
//! [`super::ksei`]) a few days after each month's last trading day. The ZIP
//! holds one pipe-separated TXT, `Balancepos<YYYYMMDD>.txt` (CRLF), with this
//! header (`Total` appears twice: the local total, then the foreign total):
//!
//! ```text
//! Date|Code|Type|Sec. Num|Price|Local IS|Local CP|Local PF|Local IB|Local ID|Local MF|Local SC|Local FD|Local OT|Total|Foreign IS|...|Foreign OT|Total
//! 30-OCT-2026|BBCA|EQUITY|123275050000|4980|2310613490|...|17194269088|666444425|...|35259422032
//! ```
//!
//! One row per security; idx keeps the `EQUITY` rows and counts the others
//! (bonds, sukuk, warrants, rights, funds, ...). The buckets cover the
//! shares held in KSEI accounts (scripless) only: shares still held as
//! certificates are outside the archive, so percentages are taken over the
//! scripless total and the scripless share of the listed shares is reported
//! next to them (BBCA, 30 Oct 2026: 42.55 %; its controlling holder's stake
//! is scrip).
//!
//! The archive names no holders, so it lives in its own tables
//! (`composition_releases`, `composition`) and never adds rows to the
//! register of holders above 1 %: a composition import leaves `ticker`,
//! `entity`, `changes`, `foreign` and `releases` exactly as they were.
//!
//! Archives are read in memory, never unpacked to disk. A ZIP is refused when
//! an entry's path leaves the archive (absolute or `..`), when it holds more
//! than [`MAX_ENTRIES`] entries, or when the TXT unpacks to more than
//! [`MAX_TXT_BYTES`].

use std::collections::{BTreeMap, HashSet};
use std::io::{Cursor, Read};
use std::sync::Arc;
use std::time::Duration;

use chrono::{Datelike, NaiveDate};
use rusqlite::{params, OptionalExtension, TransactionBehavior};

use crate::error::{IdxError, Result};
use crate::http::Fetcher;
use crate::model::{
    CompositionPoint, CompositionReport, CompositionType, DiscoveredFile, ImportReport, Release,
};
use crate::providers::Sourced;

use super::db::{db_err, parse_date, round4};
use super::{ksei, normalize, OwnershipDb};

/// The 25 columns of `Balancepos<YYYYMMDD>.txt`, in order.
pub const HEADER: [&str; 25] = [
    "Date",
    "Code",
    "Type",
    "Sec. Num",
    "Price",
    "Local IS",
    "Local CP",
    "Local PF",
    "Local IB",
    "Local ID",
    "Local MF",
    "Local SC",
    "Local FD",
    "Local OT",
    "Total",
    "Foreign IS",
    "Foreign CP",
    "Foreign PF",
    "Foreign IB",
    "Foreign ID",
    "Foreign MF",
    "Foreign SC",
    "Foreign FD",
    "Foreign OT",
    "Total",
];

/// KSEI's investor-type codes in the archive's column order.
pub const TYPE_CODES: [&str; 9] = ["IS", "CP", "PF", "IB", "ID", "MF", "SC", "FD", "OT"];

/// Entries a composition ZIP may hold (KSEI's hold one).
pub const MAX_ENTRIES: usize = 16;
/// The TXT may unpack to at most this many bytes (KSEI's are about 0.5 MiB).
pub const MAX_TXT_BYTES: u64 = 64 * 1024 * 1024;

/// Created by the version 3 step of `db::migrate`.
pub const TABLES: &str = "
CREATE TABLE IF NOT EXISTS composition_releases (
  id          INTEGER PRIMARY KEY,
  as_of       TEXT NOT NULL UNIQUE,
  format      TEXT NOT NULL,      -- zip | txt
  source      TEXT NOT NULL,
  sha256      TEXT NOT NULL,      -- of the bytes imported
  row_count   INTEGER NOT NULL,   -- EQUITY rows stored
  other_types INTEGER NOT NULL,   -- rows of other security types left out
  imported_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS composition (
  release_id    INTEGER NOT NULL REFERENCES composition_releases(id) ON DELETE CASCADE,
  symbol        TEXT NOT NULL,
  shares_listed INTEGER NOT NULL,
  price         REAL,
  local_is INTEGER NOT NULL, local_cp INTEGER NOT NULL, local_pf INTEGER NOT NULL,
  local_ib INTEGER NOT NULL, local_id INTEGER NOT NULL, local_mf INTEGER NOT NULL,
  local_sc INTEGER NOT NULL, local_fd INTEGER NOT NULL, local_ot INTEGER NOT NULL,
  local_total INTEGER NOT NULL,
  foreign_is INTEGER NOT NULL, foreign_cp INTEGER NOT NULL, foreign_pf INTEGER NOT NULL,
  foreign_ib INTEGER NOT NULL, foreign_id INTEGER NOT NULL, foreign_mf INTEGER NOT NULL,
  foreign_sc INTEGER NOT NULL, foreign_fd INTEGER NOT NULL, foreign_ot INTEGER NOT NULL,
  foreign_total INTEGER NOT NULL,
  PRIMARY KEY (release_id, symbol)
);
CREATE INDEX IF NOT EXISTS composition_symbol ON composition(symbol, release_id);
";

const COLUMNS: &str = "shares_listed, price,
  local_is, local_cp, local_pf, local_ib, local_id, local_mf, local_sc, local_fd, local_ot, local_total,
  foreign_is, foreign_cp, foreign_pf, foreign_ib, foreign_id, foreign_mf, foreign_sc, foreign_fd, foreign_ot, foreign_total";

/// One EQUITY row of the archive.
#[derive(Debug, Clone, PartialEq)]
pub struct CompositionRow {
    pub symbol: String,
    /// `Sec. Num`.
    pub shares_listed: i64,
    pub price: Option<f64>,
    /// By [`TYPE_CODES`].
    pub local: [i64; 9],
    pub local_total: i64,
    pub foreign: [i64; 9],
    pub foreign_total: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedComposition {
    /// The file's date (the most common row date).
    pub as_of: NaiveDate,
    pub rows: Vec<CompositionRow>,
    /// Rows left out and why (surfaced as warnings).
    pub skipped: Vec<String>,
    /// Rows of other security types, left out without a warning each.
    pub other_types: usize,
}

/// `30-OCT-2026` (any case; English or Indonesian month abbreviations).
pub fn parse_date_cell(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    if !s.is_ascii() {
        return None;
    }
    let mut parts = s.split('-');
    let (d, m, y) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let month = match m.to_ascii_uppercase().as_str() {
        "JAN" => 1,
        "FEB" | "PEB" => 2,
        "MAR" => 3,
        "APR" => 4,
        "MAY" | "MEI" => 5,
        "JUN" => 6,
        "JUL" => 7,
        "AUG" | "AGU" | "AGT" | "AGS" => 8,
        "SEP" => 9,
        "OCT" | "OKT" => 10,
        "NOV" | "NOP" => 11,
        "DEC" | "DES" => 12,
        _ => return None,
    };
    NaiveDate::from_ymd_opt(y.parse().ok()?, month, d.parse().ok()?)
}

fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes)
}

fn file_name(name: &str) -> String {
    super::discover::basename(name).to_ascii_lowercase()
}

/// Whether `bytes` (named `name`, a path or URL) is a composition archive
/// rather than a register workbook or PDF: a `.txt` or `.zip` name, the TXT
/// header, or a ZIP that carries a `.txt` and no workbook parts. An XLSX is
/// a ZIP too, so the entries decide.
pub fn looks_like(name: &str, bytes: &[u8]) -> bool {
    let name = file_name(name);
    if name.ends_with(".txt") || strip_bom(bytes).starts_with(b"Date|Code|Type|") {
        return true;
    }
    if name.ends_with(".xlsx") || name.ends_with(".pdf") || !bytes.starts_with(b"PK\x03\x04") {
        return false;
    }
    match zip::ZipArchive::new(Cursor::new(bytes)) {
        Ok(mut z) => {
            let mut txt = false;
            for i in 0..z.len().min(MAX_ENTRIES + 1) {
                let Ok(f) = z.by_index_raw(i) else { continue };
                let n = f.name().to_ascii_lowercase();
                if n.starts_with("xl/") || n == "[content_types].xml" {
                    return false;
                }
                txt |= n.ends_with(".txt");
            }
            txt || name.ends_with(".zip")
        }
        Err(_) => name.ends_with(".zip"),
    }
}

fn cell_int(v: &str) -> Option<i64> {
    let v = v.trim();
    if v.is_empty() || !v.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    v.parse().ok()
}

/// Parses the TXT. The header must match [`HEADER`] exactly (no guessing at
/// another layout). Rows with a bad cell are skipped with a reason; only the
/// `EQUITY` rows are kept.
pub fn parse_txt(text: &str) -> Result<ParsedComposition> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let (_, header) = lines
        .next()
        .ok_or_else(|| IdxError::Usage("empty KSEI composition file".into()))?;
    let cols: Vec<&str> = header.split('|').map(str::trim).collect();
    if cols != HEADER {
        let shown: String = header.chars().take(120).collect();
        return Err(IdxError::Usage(format!(
            "not a KSEI \"Kepemilikan Efek\" file: expected the header {}, got {shown:?}",
            HEADER.join("|")
        )));
    }
    let mut dated: Vec<(NaiveDate, CompositionRow)> = Vec::new();
    let mut skipped = Vec::new();
    let mut other_types = 0usize;
    let mut seen: HashSet<String> = HashSet::new();
    for (i, line) in lines {
        let n = i + 1;
        let c: Vec<&str> = line.split('|').map(str::trim).collect();
        if c.len() != HEADER.len() {
            skipped.push(format!(
                "line {n}: {} columns instead of {}",
                c.len(),
                HEADER.len()
            ));
            continue;
        }
        if !c[2].eq_ignore_ascii_case("EQUITY") {
            other_types += 1;
            continue;
        }
        let symbol = c[1].to_ascii_uppercase();
        if symbol.is_empty() || !symbol.bytes().all(|b| b.is_ascii_alphanumeric()) {
            skipped.push(format!("line {n}: invalid code {:?}", c[1]));
            continue;
        }
        let Some(date) = parse_date_cell(c[0]) else {
            skipped.push(format!("line {n} ({symbol}): invalid date {:?}", c[0]));
            continue;
        };
        let Some(shares_listed) = cell_int(c[3]).filter(|v| *v > 0) else {
            skipped.push(format!(
                "line {n} ({symbol}): Sec. Num {:?} is not a positive share count",
                c[3]
            ));
            continue;
        };
        let price = if c[4].is_empty() {
            None
        } else {
            match c[4].parse::<f64>() {
                Ok(p) if p.is_finite() && p >= 0.0 => Some(p),
                _ => {
                    skipped.push(format!("line {n} ({symbol}): invalid price {:?}", c[4]));
                    continue;
                }
            }
        };
        let nums: Option<Vec<i64>> = c[5..25].iter().map(|v| cell_int(v)).collect();
        let Some(nums) = nums else {
            skipped.push(format!(
                "line {n} ({symbol}): a share count is not a whole number"
            ));
            continue;
        };
        if !seen.insert(symbol.clone()) {
            skipped.push(format!(
                "line {n}: {symbol} appears twice; the first row is kept"
            ));
            continue;
        }
        let mut local = [0i64; 9];
        let mut foreign = [0i64; 9];
        local.copy_from_slice(&nums[0..9]);
        foreign.copy_from_slice(&nums[10..19]);
        let (local_total, foreign_total) = (nums[9], nums[19]);
        for (side, parts, total) in [
            ("local", &local, local_total),
            ("foreign", &foreign, foreign_total),
        ] {
            let sum: i64 = parts.iter().sum();
            if sum != total {
                skipped.push(format!(
                    "line {n} ({symbol}): the {side} types add up to {sum}, KSEI's total is {total}; the total is kept"
                ));
            }
        }
        dated.push((
            date,
            CompositionRow {
                symbol,
                shares_listed,
                price,
                local,
                local_total,
                foreign,
                foreign_total,
            },
        ));
    }
    let mut counts: BTreeMap<NaiveDate, usize> = BTreeMap::new();
    for (d, _) in &dated {
        *counts.entry(*d).or_default() += 1;
    }
    // Most rows wins; on a tie the later date.
    let Some(as_of) = counts
        .iter()
        .max_by_key(|(d, n)| (**n, **d))
        .map(|(d, _)| *d)
    else {
        return Err(IdxError::Usage(format!(
            "no EQUITY rows in the KSEI composition file{}",
            if skipped.is_empty() {
                String::new()
            } else {
                format!(" ({} skipped: {})", skipped.len(), skipped[0])
            }
        )));
    };
    let mut rows = Vec::with_capacity(dated.len());
    for (d, r) in dated {
        if d == as_of {
            rows.push(r);
        } else {
            skipped.push(format!(
                "{} is dated {d}, not {as_of}; left out of the {as_of} month",
                r.symbol
            ));
        }
    }
    Ok(ParsedComposition {
        as_of,
        rows,
        skipped,
        other_types,
    })
}

/// Whether a ZIP entry's name would leave the folder it is unpacked into:
/// absolute (`/x`, `\x`, `C:x`), a `..` component, or a NUL byte. The
/// archive is never unpacked to disk, but such a name has no business in
/// KSEI's archive. (`ZipFile::enclosed_name` alone accepts a leading `/`.)
fn escapes(name: &str) -> bool {
    name.starts_with(['/', '\\'])
        || name.contains('\0')
        || name.as_bytes().get(1) == Some(&b':')
        || name.split(['/', '\\']).any(|c| c == "..")
}

/// Reads the TXT inside a KSEI ZIP (see the module docs for the limits).
pub fn parse_zip(bytes: &[u8]) -> Result<ParsedComposition> {
    parse_zip_limited(bytes, MAX_TXT_BYTES)
}

fn parse_zip_limited(bytes: &[u8], max_txt: u64) -> Result<ParsedComposition> {
    let bad = |e: zip::result::ZipError| {
        IdxError::Usage(format!("not a readable KSEI composition ZIP: {e}"))
    };
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(bad)?;
    if z.len() > MAX_ENTRIES {
        return Err(IdxError::Usage(format!(
            "the ZIP holds {} entries; a KSEI composition archive holds one TXT file",
            z.len()
        )));
    }
    let mut pick: Option<(usize, String, u64)> = None;
    let mut txts = 0usize;
    for i in 0..z.len() {
        let f = z.by_index_raw(i).map_err(bad)?;
        let name = f.name().to_string();
        if escapes(&name) || f.enclosed_name().is_none() {
            return Err(IdxError::Usage(format!(
                "refusing the ZIP: entry {name:?} points outside the archive (absolute path or ..)"
            )));
        }
        if f.is_dir() || !name.to_ascii_lowercase().ends_with(".txt") {
            continue;
        }
        txts += 1;
        if pick.is_none() {
            pick = Some((i, name, f.size()));
        }
    }
    let (i, name, size) = pick.ok_or_else(|| {
        IdxError::Usage(
            "the ZIP holds no .txt file; a KSEI composition archive holds Balancepos<YYYYMMDD>.txt"
                .into(),
        )
    })?;
    let over = |n: u64| {
        IdxError::Usage(format!(
            "{name} unpacks to {n} bytes or more, over the {} limit for a composition file",
            ksei::size_limit(max_txt)
        ))
    };
    if size > max_txt {
        return Err(over(size));
    }
    let f = z.by_index(i).map_err(bad)?;
    let mut buf = Vec::with_capacity(size.min(max_txt) as usize);
    // The declared size can lie: never read past the limit.
    f.take(max_txt + 1)
        .read_to_end(&mut buf)
        .map_err(|e| IdxError::Usage(format!("cannot unpack {name}: {e}")))?;
    if buf.len() as u64 > max_txt {
        return Err(over(buf.len() as u64));
    }
    let mut parsed = parse_txt(&String::from_utf8_lossy(&buf))?;
    if txts > 1 {
        parsed.skipped.push(format!(
            "the ZIP holds {txts} .txt files; only {name} was read"
        ));
    }
    Ok(parsed)
}

/// Parses a ZIP or TXT archive; the second value is the format (`zip`|`txt`).
pub fn parse_any(name: &str, bytes: &[u8]) -> Result<(ParsedComposition, &'static str)> {
    if bytes.starts_with(b"PK\x03\x04") || file_name(name).ends_with(".zip") {
        return parse_zip(bytes).map(|p| (p, "zip"));
    }
    let text = std::str::from_utf8(strip_bom(bytes))
        .map(str::to_string)
        .unwrap_or_else(|_| String::from_utf8_lossy(bytes).into_owned());
    parse_txt(&text).map(|p| (p, "txt"))
}

fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// Stores a parsed archive as the month for its `as_of`, replacing an earlier
/// import of the same month. The write lock is taken up front (`BEGIN
/// IMMEDIATE`): a deferred read-then-write transaction fails at once with
/// "database is locked" when another connection commits in between, while
/// this one waits for the other import to finish.
pub fn import(
    db: &mut OwnershipDb,
    parsed: &ParsedComposition,
    format: &str,
    source: &str,
    sha256: &str,
) -> Result<ImportReport> {
    let database = db.path().display().to_string();
    let as_of = parsed.as_of.to_string();
    let imported_at = now();
    let tx = db
        .conn_mut()
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(db_err)?;
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM composition_releases WHERE as_of = ?1",
            [&as_of],
            |r| r.get(0),
        )
        .optional()
        .map_err(db_err)?;
    if let Some(id) = existing {
        tx.execute("DELETE FROM composition WHERE release_id = ?1", [id])
            .map_err(db_err)?;
        tx.execute("DELETE FROM composition_releases WHERE id = ?1", [id])
            .map_err(db_err)?;
    }
    tx.execute(
        "INSERT INTO composition_releases (as_of, format, source, sha256, row_count, other_types, imported_at)
         VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![
            as_of,
            format,
            source,
            sha256,
            parsed.rows.len() as i64,
            parsed.other_types as i64,
            imported_at
        ],
    )
    .map_err(db_err)?;
    let release_id = tx.last_insert_rowid();
    {
        let mut insert = tx
            .prepare(&format!(
                "INSERT INTO composition (release_id, symbol, {COLUMNS})
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24)"
            ))
            .map_err(db_err)?;
        for r in &parsed.rows {
            let mut values: Vec<rusqlite::types::Value> = vec![
                release_id.into(),
                r.symbol.clone().into(),
                r.shares_listed.into(),
                r.price.into(),
            ];
            values.extend(r.local.iter().map(|v| (*v).into()));
            values.push(r.local_total.into());
            values.extend(r.foreign.iter().map(|v| (*v).into()));
            values.push(r.foreign_total.into());
            insert
                .execute(rusqlite::params_from_iter(values))
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
            issuers: parsed.rows.len() as u64,
            entities: 0,
            imported_at,
            positions: None,
            other_date_rows: None,
            controller_column: None,
        },
        replaced: existing.is_some(),
        database,
        family: "composition".into(),
        other_securities: Some(parsed.other_types as u64),
    })
}

/// Imported months, oldest first.
pub fn months(db: &OwnershipDb) -> Result<Vec<NaiveDate>> {
    let mut st = db
        .conn()
        .prepare("SELECT as_of FROM composition_releases ORDER BY as_of")
        .map_err(db_err)?;
    let rows = st
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows.iter().map(|s| parse_date(s)).collect())
}

struct Stored {
    as_of: NaiveDate,
    source: String,
    shares_listed: i64,
    price: Option<f64>,
    local: [i64; 9],
    local_total: i64,
    foreign: [i64; 9],
    foreign_total: i64,
}

fn pct(part: i64, whole: i64) -> Option<f64> {
    (whole > 0).then(|| round4(part as f64 * 100.0 / whole as f64))
}

impl Stored {
    fn scripless(&self) -> i64 {
        self.local_total + self.foreign_total
    }
}

fn stored(db: &OwnershipDb, symbol: &str) -> Result<Vec<Stored>> {
    let mut st = db
        .conn()
        .prepare(&format!(
            "SELECT r.as_of, r.source, {COLUMNS}
             FROM composition c JOIN composition_releases r ON r.id = c.release_id
             WHERE c.symbol = ?1 ORDER BY r.as_of"
        ))
        .map_err(db_err)?;
    let rows = st
        .query_map([symbol], |row| {
            let mut local = [0i64; 9];
            let mut foreign = [0i64; 9];
            for (k, v) in local.iter_mut().enumerate() {
                *v = row.get(4 + k)?;
            }
            for (k, v) in foreign.iter_mut().enumerate() {
                *v = row.get(14 + k)?;
            }
            Ok(Stored {
                as_of: parse_date(&row.get::<_, String>(0)?),
                source: row.get(1)?,
                shares_listed: row.get(2)?,
                price: row.get(3)?,
                local,
                local_total: row.get(13)?,
                foreign,
                foreign_total: row.get(23)?,
            })
        })
        .map_err(db_err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(db_err)?;
    Ok(rows)
}

fn import_hint() -> &'static str {
    "run `idx ownership import --latest --family composition` (or --file BalanceposEfek<YYYYMMDD>.zip)"
}

/// One stock in one imported month (`None`: the newest) plus every imported
/// month as history. Reads the database only.
pub fn read(db: &OwnershipDb, symbol: &str, as_of: Option<NaiveDate>) -> Result<CompositionReport> {
    let have = months(db)?;
    let Some(newest) = have.last().copied() else {
        return Err(IdxError::NotFound(format!(
            "no KSEI composition imported yet; {}",
            import_hint()
        )));
    };
    let month = as_of.unwrap_or(newest);
    if !have.contains(&month) {
        return Err(IdxError::NotFound(format!(
            "KSEI composition {month} is not imported (imported: {}); {}",
            have.iter()
                .map(NaiveDate::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            import_hint()
        )));
    }
    let list = stored(db, symbol)?;
    let Some(cur) = list.iter().find(|s| s.as_of == month) else {
        return Err(IdxError::NotFound(format!(
            "{symbol} is not in KSEI's composition of {month} (it lists stocks, the EQUITY rows)"
        )));
    };
    let scripless = cur.scripless();
    let by_type = TYPE_CODES
        .iter()
        .enumerate()
        .map(|(k, code)| CompositionType {
            code: code.to_string(),
            investor_type: normalize::investor_type(code).unwrap_or_else(|| code.to_string()),
            local_shares: cur.local[k],
            foreign_shares: cur.foreign[k],
            local_pct: pct(cur.local[k], scripless),
            foreign_pct: pct(cur.foreign[k], scripless),
        })
        .collect();
    let mut history: Vec<CompositionPoint> = Vec::with_capacity(list.len());
    for s in &list {
        let foreign_pct = pct(s.foreign_total, s.scripless());
        let prev = history.last().and_then(|p| p.foreign_pct);
        history.push(CompositionPoint {
            as_of: s.as_of,
            local_pct: pct(s.local_total, s.scripless()),
            foreign_pct,
            scripless_pct: pct(s.scripless(), s.shares_listed),
            foreign_change_pp: match (prev, foreign_pct) {
                (Some(a), Some(b)) if !history.is_empty() => Some(round4(b - a)),
                _ => None,
            },
        });
    }
    Ok(CompositionReport {
        symbol: symbol.to_string(),
        as_of: month,
        price: cur.price,
        shares_listed: cur.shares_listed,
        shares_scripless: scripless,
        scripless_pct: pct(scripless, cur.shares_listed),
        local_shares: cur.local_total,
        foreign_shares: cur.foreign_total,
        local_pct: pct(cur.local_total, scripless),
        foreign_pct: pct(cur.foreign_total, scripless),
        by_type,
        history,
        source: cur.source.clone(),
    })
}

/// The last calendar day of the month after `d`: KSEI publishes a month's
/// archive a few days after it ends, so no newer archive than `d` can exist
/// before then.
fn next_month_end(d: NaiveDate) -> NaiveDate {
    let (y, m) = if d.month() == 12 {
        (d.year() + 1, 1)
    } else {
        (d.year(), d.month() + 1)
    };
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1)
        .and_then(|f| f.pred_opt())
        .unwrap_or(d)
}

/// One automatic import at a time in this process.
static AUTO_IMPORT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Downloads one listed archive and imports it, unless the month is in the
/// database by the time it is this call's turn: `None` then. Parallel calls
/// on a fresh database (an agent asking for several stocks at once) take
/// turns, so the archive is downloaded once; another process importing the
/// same month is waited for by [`import`].
pub async fn import_listed(
    db: &mut OwnershipDb,
    file: &DiscoveredFile,
    timeout: Duration,
) -> Result<Option<(ImportReport, Vec<String>)>> {
    let _turn = AUTO_IMPORT.lock().await;
    if months(db)?.contains(&file.as_of) {
        return Ok(None);
    }
    super::import_url(db, &file.url, timeout).await.map(Some)
}

/// `ownership composition`: [`read`], importing from ksei.co.id first when
/// the month asked for is missing, or (without `as_of`) when a newer month
/// than the newest imported can exist and is listed. `--offline` never
/// downloads. The provider is `ksei` when this call imported, otherwise
/// `local-db`.
pub async fn report(
    db: &mut OwnershipDb,
    fetcher: &Arc<Fetcher>,
    symbol: &str,
    as_of: Option<NaiveDate>,
    timeout: Duration,
    today: NaiveDate,
) -> Result<Sourced<CompositionReport>> {
    let have = months(db)?;
    let offline = fetcher.is_offline();
    let mut warnings = Vec::new();
    let mut imported = false;
    match as_of {
        Some(d) if have.contains(&d) => {}
        Some(d) => {
            if offline {
                return Err(IdxError::OfflineMiss(format!(
                    "KSEI composition {d} is not imported and --offline does not download it; {}",
                    import_hint()
                )));
            }
            let listing =
                ksei::list_until(fetcher, |files| files.iter().any(|f| f.as_of <= d)).await?;
            warnings.extend(listing.warnings);
            let file = listing.data.iter().find(|f| f.as_of == d).ok_or_else(|| {
                IdxError::NotFound(format!(
                    "ksei.co.id lists no composition archive for {d} (newest listed: {})",
                    listing
                        .data
                        .first()
                        .map(|f| f.as_of.to_string())
                        .unwrap_or_else(|| "none".into())
                ))
            })?;
            if let Some((r, w)) = import_listed(db, file, timeout).await? {
                warnings.extend(w);
                warnings.push(imported_note(&r));
                imported = true;
            }
        }
        None => {
            let newest = have.last().copied();
            let due = newest.is_none_or(|n| today > next_month_end(n));
            if newest.is_none() && offline {
                return Err(IdxError::OfflineMiss(format!(
                    "no KSEI composition imported yet and --offline does not download one; {}",
                    import_hint()
                )));
            }
            if due && !offline {
                match ksei::list(fetcher, ksei::PER_PAGE).await {
                    Ok(listing) => {
                        let newer = listing
                            .data
                            .first()
                            .filter(|f| newest.is_none_or(|n| f.as_of > n))
                            .cloned();
                        warnings.extend(listing.warnings);
                        match (newer, newest) {
                            (Some(f), _) => match import_listed(db, &f, timeout).await {
                                Ok(Some((r, w))) => {
                                    warnings.extend(w);
                                    warnings.push(imported_note(&r));
                                    imported = true;
                                }
                                Ok(None) => {}
                                Err(e) if newest.is_some() => warnings.push(format!(
                                    "could not import the newer KSEI composition {} ({e}); showing {}",
                                    f.as_of,
                                    newest.map(|n| n.to_string()).unwrap_or_default()
                                )),
                                Err(e) => return Err(e),
                            },
                            (None, Some(_)) => {}
                            (None, None) => {
                                return Err(IdxError::provider(
                                    ksei::PROVIDER,
                                    "the Kepemilikan Efek page lists no archive; the page layout may have changed",
                                ))
                            }
                        }
                    }
                    Err(e) => match newest {
                        Some(n) => warnings.push(format!(
                            "could not check ksei.co.id for a month newer than {n} ({e})"
                        )),
                        None => return Err(e),
                    },
                }
            }
        }
    }
    let r = read(db, symbol, as_of)?;
    let mut s = Sourced::fresh(
        r,
        if imported {
            ksei::PROVIDER
        } else {
            super::PROVIDER
        },
    );
    s.warnings = warnings;
    Ok(s)
}

fn imported_note(r: &ImportReport) -> String {
    format!(
        "imported KSEI composition {} from {} ({} stocks)",
        r.release.as_of,
        super::discover::basename(&r.release.source),
        r.release.rows
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const OCT: &str =
        include_str!("../../tests/fixtures/ownership/ksei_balancepos_20261030_excerpt.txt");
    const SEP: &str =
        include_str!("../../tests/fixtures/ownership/ksei_balancepos_20260930_excerpt.txt");
    const OCT_ZIP: &[u8] = include_bytes!(
        "../../tests/fixtures/ownership/ksei_balancepos_20261030_excerpt_constructed.zip"
    );
    const XLSX: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20260831_excerpt.xlsx");
    const XLSX_OCT: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20261030_excerpt.xlsx");

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, body) in entries {
            w.start_file(*name, opts).unwrap();
            w.write_all(body).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn row<'a>(p: &'a ParsedComposition, sym: &str) -> &'a CompositionRow {
        p.rows.iter().find(|r| r.symbol == sym).unwrap()
    }

    #[test]
    fn the_txt_keeps_the_stocks_and_counts_the_rest() {
        assert!(OCT.contains("\r\n"), "the fixture keeps KSEI's CRLF");
        let p = parse_txt(OCT).unwrap();
        assert_eq!(p.as_of, d(2026, 10, 30));
        let syms: Vec<&str> = p.rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(syms, ["AADI", "ABDA", "BBCA", "GOTO", "KREN", "SING"]);
        assert_eq!(p.other_types, 7);
        assert!(p.skipped.is_empty(), "{:?}", p.skipped);
        let bbca = row(&p, "BBCA");
        assert_eq!(bbca.shares_listed, 123_275_050_000);
        assert_eq!(bbca.price, Some(4980.0));
        assert_eq!(bbca.local_total, 17_194_269_088);
        assert_eq!(bbca.foreign_total, 35_259_422_032);
        assert_eq!(bbca.local[4], 12_030_567_470); // ID
        assert_eq!(bbca.foreign[5], 20_393_323_810); // MF
                                                     // LF line endings parse the same.
        assert_eq!(parse_txt(&OCT.replace("\r\n", "\n")).unwrap(), p);
    }

    #[test]
    fn the_zip_parses_like_its_txt() {
        let (z, fmt) = parse_any("BalanceposEfek20261030.zip", OCT_ZIP).unwrap();
        assert_eq!(fmt, "zip");
        assert_eq!(z, parse_txt(OCT).unwrap());
        let (t, fmt) = parse_any("Balancepos20261030.txt", OCT.as_bytes()).unwrap();
        assert_eq!(fmt, "txt");
        assert_eq!(t, z);
    }

    #[test]
    fn workbooks_are_not_archives() {
        assert!(looks_like("BalanceposEfek20261030.zip", OCT_ZIP));
        assert!(looks_like("download", OCT_ZIP), "detected from the entries");
        assert!(looks_like("Balancepos20261030.txt", OCT.as_bytes()));
        assert!(
            looks_like("stdin", OCT.as_bytes()),
            "detected from the header"
        );
        // An XLSX is a ZIP too: its parts decide, whatever the name says.
        assert!(!looks_like("peng-2026-08-00017-satu-persen.xlsx", XLSX));
        assert!(!looks_like("download", XLSX));
        assert!(!looks_like("download", XLSX_OCT));
        assert!(!looks_like("x.pdf", b"%PDF-1.7"));
    }

    #[test]
    fn a_different_layout_is_refused() {
        let err = parse_txt("Date|Code|Type\n30-OCT-2026|BBCA|EQUITY\n").unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("expected the header"), "{err}");
        assert_eq!(parse_txt("").unwrap_err().exit_code(), 2);
        let header = HEADER.join("|");
        let only_bonds = format!(
            "{header}\n30-OCT-2026|ADCP02B|CORPORATE BOND|1|1|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0\n"
        );
        let err = parse_txt(&only_bonds).unwrap_err();
        assert!(err.to_string().contains("no EQUITY rows"), "{err}");
    }

    #[test]
    fn bad_cells_skip_their_row_and_dates_never_panic() {
        let header = HEADER.join("|");
        let ok = "30-OCT-2026|AAAA|EQUITY|100|5|1|0|0|0|0|0|0|0|0|1|0|0|0|0|0|0|0|0|2|2";
        let text = [
            header.as_str(),
            ok,
            "3Ø-OCT-2026|BBBB|EQUITY|100|5|1|0|0|0|0|0|0|0|0|1|0|0|0|0|0|0|0|0|0|0",
            "30-XYZ-2026|CCCC|EQUITY|100|5|1|0|0|0|0|0|0|0|0|1|0|0|0|0|0|0|0|0|0|0",
            "30-OCT-2026|DDDD|EQUITY|0|5|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0|0",
            "30-OCT-2026|EEEE|EQUITY|100|5|1,5|0|0|0|0|0|0|0|0|1|0|0|0|0|0|0|0|0|0|0",
            "30-OCT-2026|FFFF|EQUITY|100|5",
            "30-OCT-2026|AAAA|EQUITY|100|5|1|0|0|0|0|0|0|0|0|1|0|0|0|0|0|0|0|0|0|0",
            "30-OCT-2026|GGGG|EQUITY|100||1|0|0|0|0|0|0|0|0|9|0|0|0|0|0|0|0|0|0|0",
            "29-OCT-2026|HHHH|EQUITY|100|5|1|0|0|0|0|0|0|0|0|1|0|0|0|0|0|0|0|0|0|0",
        ]
        .join("\r\n");
        let p = parse_txt(&text).unwrap();
        assert_eq!(p.as_of, d(2026, 10, 30));
        let syms: Vec<&str> = p.rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(syms, ["AAAA", "GGGG"]);
        assert_eq!(row(&p, "GGGG").price, None);
        // GGGG's local types sum to 1 against a total of 9: kept, warned.
        assert_eq!(row(&p, "GGGG").local_total, 9);
        let all = p.skipped.join("\n");
        for needle in [
            "BBBB): invalid date",
            "CCCC): invalid date",
            "DDDD): Sec. Num",
            "EEEE): a share count",
            "line 7: 5 columns",
            "AAAA appears twice",
            "GGGG): the local types add up to 1",
            "HHHH is dated 2026-10-29",
        ] {
            assert!(all.contains(needle), "{needle} in {all}");
        }
        assert_eq!(parse_date_cell("27-feb-2026"), Some(d(2026, 2, 27)));
        assert_eq!(parse_date_cell("31-DES-2025"), Some(d(2025, 12, 31)));
        assert_eq!(parse_date_cell("31-FEB-2026"), None);
        assert_eq!(parse_date_cell("ÅÅ-OCT-2026"), None);
    }

    #[test]
    fn zip_slip_entries_are_refused_and_nothing_is_written() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("evil.txt");
        let rel = format!("../{}", target.file_name().unwrap().to_string_lossy());
        for name in [
            rel.as_str(),
            "../../etc/Balancepos20261030.txt",
            "/abs/Balancepos.txt",
            "\\server\\Balancepos.txt",
            "C:/Balancepos.txt",
            "data/../../Balancepos.txt",
        ] {
            let bytes = zip_of(&[(name, OCT.as_bytes())]);
            let err = parse_zip(&bytes).unwrap_err();
            assert_eq!(err.exit_code(), 2, "{name}");
            assert!(
                err.to_string().contains("outside the archive"),
                "{name}: {err}"
            );
        }
        assert!(!target.exists());
        // A safe entry in a subfolder is fine.
        let ok = zip_of(&[("data/Balancepos20261030.txt", OCT.as_bytes())]);
        assert_eq!(parse_zip(&ok).unwrap().rows.len(), 6);
    }

    #[test]
    fn size_and_entry_limits() {
        let bytes = zip_of(&[("Balancepos20261030.txt", OCT.as_bytes())]);
        let err = parse_zip_limited(&bytes, 1024).unwrap_err();
        assert!(
            err.to_string().contains("over the 1024 bytes limit"),
            "{err}"
        );
        assert_eq!(err.exit_code(), 2);
        assert!(parse_zip_limited(&bytes, OCT.len() as u64).is_ok());
        // A highly compressible bomb is stopped at the limit, not unpacked.
        let bomb = vec![b'0'; 4 * 1024 * 1024];
        let bytes = zip_of(&[("Balancepos.txt", bomb.as_slice())]);
        assert!(bytes.len() < 64 * 1024, "the bomb compresses well");
        let err = parse_zip_limited(&bytes, 1024 * 1024).unwrap_err();
        assert!(err.to_string().contains("limit"), "{err}");
        let many: Vec<(String, &[u8])> = (0..MAX_ENTRIES + 1)
            .map(|i| (format!("f{i}.txt"), b"x".as_slice()))
            .collect();
        let refs: Vec<(&str, &[u8])> = many.iter().map(|(n, b)| (n.as_str(), *b)).collect();
        let err = parse_zip(&zip_of(&refs)).unwrap_err();
        assert!(err.to_string().contains("entries"), "{err}");
        let err = parse_zip(&zip_of(&[("readme.md", b"hi".as_slice())])).unwrap_err();
        assert!(err.to_string().contains("no .txt"), "{err}");
        let err = parse_zip(b"PK\x03\x04 truncated").unwrap_err();
        assert!(err.to_string().contains("not a readable"), "{err}");
        let two = zip_of(&[
            ("Balancepos20261030.txt", OCT.as_bytes()),
            ("Balancepos20260930.txt", SEP.as_bytes()),
        ]);
        let p = parse_zip(&two).unwrap();
        assert_eq!(p.as_of, d(2026, 10, 30));
        assert!(p.skipped[0].contains("2 .txt files"), "{:?}", p.skipped);
    }

    fn loaded() -> OwnershipDb {
        let mut db = OwnershipDb::in_memory().unwrap();
        for (name, bytes) in [
            ("BalanceposEfek20261030.zip", OCT_ZIP),
            ("Balancepos20260930.txt", SEP.as_bytes()),
        ] {
            let (p, fmt) = parse_any(name, bytes).unwrap();
            import(&mut db, &p, fmt, name, "sha").unwrap();
        }
        db
    }

    #[test]
    fn import_replaces_a_month_and_leaves_the_register_alone() {
        let mut db = OwnershipDb::in_memory().unwrap();
        let (p, fmt) = parse_any("BalanceposEfek20261030.zip", OCT_ZIP).unwrap();
        let r = import(&mut db, &p, fmt, "a.zip", "s1").unwrap();
        assert_eq!(
            (
                r.family.as_str(),
                r.release.rows,
                r.release.entities,
                r.other_securities
            ),
            ("composition", 6, 0, Some(7))
        );
        assert!(!r.replaced);
        let again = import(&mut db, &p, fmt, "b.zip", "s2").unwrap();
        assert!(again.replaced);
        assert_eq!(months(&db).unwrap(), [d(2026, 10, 30)]);
        let n: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM composition", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 6);
        // Nothing reaches the register of holders above 1 %.
        assert!(db.releases().unwrap().is_empty());
        assert_eq!(db.holders("BBCA", None).unwrap_err().exit_code(), 4);
        assert_eq!(read(&db, "BBCA", None).unwrap().source, "b.zip");
    }

    #[test]
    fn an_import_waits_for_one_in_progress_on_another_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ownership.sqlite");
        let mut other = OwnershipDb::open(&path).unwrap();
        let mut db = OwnershipDb::open(&path).unwrap();
        let (p, fmt) = parse_any("BalanceposEfek20261030.zip", OCT_ZIP).unwrap();
        let (holding, held) = std::sync::mpsc::channel();
        // Another connection is importing the same month: it holds the write
        // lock for a while, then commits.
        let writer = std::thread::spawn(move || {
            let tx = other
                .conn_mut()
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .unwrap();
            tx.execute(
                "INSERT INTO composition_releases (as_of, format, source, sha256, row_count, other_types, imported_at)
                 VALUES ('2026-10-30', 'zip', 'other.zip', 's0', 0, 0, 'now')",
                [],
            )
            .unwrap();
            holding.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            tx.commit().unwrap();
        });
        held.recv().unwrap();
        // A deferred transaction would read before the commit and fail with
        // "database is locked" on its first write.
        let r = import(&mut db, &p, fmt, "a.zip", "s1").unwrap();
        writer.join().unwrap();
        assert!(r.replaced, "the other import was committed first");
        assert_eq!(months(&db).unwrap(), [d(2026, 10, 30)]);
        assert_eq!(read(&db, "BBCA", None).unwrap().source, "a.zip");
    }

    #[tokio::test]
    async fn a_listed_month_imported_meanwhile_is_not_downloaded_again() {
        let mut db = loaded();
        let file = DiscoveredFile {
            as_of: d(2026, 10, 30),
            title: "Kepemilikan Efek 30 Oktober 2026".into(),
            url: "file:///no/such/BalanceposEfek20261030.zip".into(),
            format: "zip".into(),
            origin: ksei::ORIGIN.into(),
            sha256: None,
            source_url: None,
            family: "composition".into(),
            importable: true,
        };
        let got = import_listed(&mut db, &file, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(got.is_none(), "the URL would fail if it were read");
    }

    #[test]
    fn report_math_and_history() {
        let db = loaded();
        let r = read(&db, "BBCA", None).unwrap();
        assert_eq!(r.as_of, d(2026, 10, 30));
        assert_eq!(r.shares_scripless, 52_453_691_120);
        assert_eq!(r.scripless_pct, Some(42.5501));
        assert_eq!(r.local_pct, Some(32.7799));
        assert_eq!(r.foreign_pct, Some(67.2201));
        assert_eq!(r.price, Some(4980.0));
        let codes: Vec<&str> = r.by_type.iter().map(|t| t.code.as_str()).collect();
        assert_eq!(codes, TYPE_CODES);
        let id = r.by_type.iter().find(|t| t.code == "ID").unwrap();
        assert_eq!(id.investor_type, "INDIVIDUAL");
        assert_eq!(
            (id.local_pct, id.foreign_pct),
            (Some(22.9356), Some(0.6192))
        );
        let mf = r.by_type.iter().find(|t| t.code == "MF").unwrap();
        assert_eq!(mf.foreign_pct, Some(38.8787));
        let sum_local: i64 = r.by_type.iter().map(|t| t.local_shares).sum();
        assert_eq!(sum_local, r.local_shares);
        assert_eq!(r.history.len(), 2);
        assert_eq!(r.history[0].as_of, d(2026, 9, 30));
        assert_eq!(r.history[0].foreign_change_pp, None);
        assert_eq!(r.history[1].foreign_pct, Some(67.2201));
        let sep = r.history[0].foreign_pct.unwrap();
        assert_eq!(r.history[1].foreign_change_pp, Some(round4(67.2201 - sep)));
        assert!(r.history[1].foreign_change_pp.unwrap() < 0.0);

        let sep = read(&db, "BBCA", Some(d(2026, 9, 30))).unwrap();
        assert_eq!(sep.as_of, d(2026, 9, 30));
        assert_eq!(sep.history.len(), 2, "history covers every month");

        // A sliver of the shares in KSEI accounts still gives finite numbers.
        let sing = read(&db, "SING", None).unwrap();
        assert_eq!(sing.shares_scripless, 173_000);
        assert!(sing.scripless_pct.unwrap() > 0.0 && sing.scripless_pct.unwrap() < 0.1);
        assert_eq!(sing.price, Some(0.0));

        assert_eq!(read(&db, "ZZZZ", None).unwrap_err().exit_code(), 4);
        assert_eq!(
            read(&db, "ADCP02B", None).unwrap_err().exit_code(),
            4,
            "bonds are not stored"
        );
        let err = read(&db, "BBCA", Some(d(2026, 8, 31))).unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert!(err.to_string().contains("2026-09-30, 2026-10-30"), "{err}");
        let empty = OwnershipDb::in_memory().unwrap();
        assert!(read(&empty, "BBCA", None)
            .unwrap_err()
            .to_string()
            .contains("--family composition"));
    }

    #[test]
    fn newer_months_are_due_after_the_next_month_ends() {
        assert_eq!(next_month_end(d(2026, 10, 30)), d(2026, 11, 30));
        assert_eq!(next_month_end(d(2026, 11, 30)), d(2026, 12, 31));
        assert_eq!(next_month_end(d(2026, 12, 30)), d(2027, 1, 31));
        assert_eq!(next_month_end(d(2026, 1, 30)), d(2026, 2, 28));
    }

    #[tokio::test]
    async fn offline_reads_locally_and_never_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(crate::http::FixtureBackend::new());
        let fetcher = Arc::new(Fetcher::new(
            be.clone(),
            crate::cache::Cache::new(dir.path()),
            crate::http::CacheMode::Offline,
        ));
        let mut empty = OwnershipDb::in_memory().unwrap();
        let err = report(
            &mut empty,
            &fetcher,
            "BBCA",
            None,
            Duration::from_secs(5),
            d(2026, 11, 6),
        )
        .await
        .unwrap_err();
        assert_eq!(err.exit_code(), 5);
        let mut db = loaded();
        // Long after the next month ended: offline still reads what it has.
        let s = report(
            &mut db,
            &fetcher,
            "BBCA",
            None,
            Duration::from_secs(5),
            d(2027, 6, 1),
        )
        .await
        .unwrap();
        assert_eq!(s.provider, "local-db");
        assert_eq!(s.data.as_of, d(2026, 10, 30));
        let err = report(
            &mut db,
            &fetcher,
            "BBCA",
            Some(d(2026, 8, 31)),
            Duration::from_secs(5),
            d(2026, 11, 6),
        )
        .await
        .unwrap_err();
        assert_eq!(err.exit_code(), 5);
        assert_eq!(be.call_count(), 0);
    }

    #[tokio::test]
    async fn the_first_call_imports_the_newest_listed_month() {
        let dir = tempfile::tempdir().unwrap();
        let zip_path = dir.path().join("BalanceposEfek20261030.zip");
        std::fs::write(&zip_path, OCT_ZIP).unwrap();
        let url = format!(
            "file://{}",
            zip_path.display().to_string().replace('\\', "/")
        );
        let page = format!(
            "<p class=\"data-item-title\">Kepemilikan Efek 30 Oktober 2026</p> <a href=\"{url}\">Unduh</a> last_page:1"
        );
        let be = Arc::new(crate::http::FixtureBackend::new().route("kepemilikan-efek", 200, &page));
        let fetcher = Arc::new(Fetcher::new(
            be.clone(),
            crate::cache::Cache::new(dir.path().join("cache")),
            crate::http::CacheMode::Normal,
        ));
        let mut db = OwnershipDb::in_memory().unwrap();
        let t = Duration::from_secs(5);
        let s = report(&mut db, &fetcher, "BBCA", None, t, d(2026, 11, 6))
            .await
            .unwrap();
        assert_eq!(s.provider, "ksei");
        assert_eq!(
            s.warnings,
            ["imported KSEI composition 2026-10-30 from BalanceposEfek20261030.zip (6 stocks)"]
        );
        assert_eq!(s.data.scripless_pct, Some(42.5501));
        assert_eq!(s.data.source, url);
        // Later calls read the database; November's archive cannot exist yet.
        let s = report(&mut db, &fetcher, "TLKM", None, t, d(2026, 11, 6))
            .await
            .unwrap_err();
        assert_eq!(s.exit_code(), 4, "TLKM is not in the excerpt");
        assert_eq!(be.call_count(), 1);
        assert!(
            db.releases().unwrap().is_empty(),
            "the register is untouched"
        );
    }

    #[tokio::test]
    async fn online_checks_the_listing_only_when_a_newer_month_can_exist() {
        let dir = tempfile::tempdir().unwrap();
        let page =
            include_str!("../../tests/fixtures/ownership/ksei_kepemilikan_efek_page_excerpt.html");
        let be = Arc::new(crate::http::FixtureBackend::new().route("kepemilikan-efek", 200, page));
        let fetcher = Arc::new(Fetcher::new(
            be.clone(),
            crate::cache::Cache::new(dir.path()),
            crate::http::CacheMode::Normal,
        ));
        let mut db = loaded();
        // 2026-11-06: the November archive cannot exist yet, nothing is fetched.
        let s = report(
            &mut db,
            &fetcher,
            "BBCA",
            None,
            Duration::from_secs(5),
            d(2026, 11, 6),
        )
        .await
        .unwrap();
        assert_eq!((s.provider, be.call_count()), ("local-db", 0));
        // 2026-12-02: the listing is read; its newest month (Oct) is imported already.
        let s = report(
            &mut db,
            &fetcher,
            "BBCA",
            None,
            Duration::from_secs(5),
            d(2026, 12, 2),
        )
        .await
        .unwrap();
        assert_eq!((s.provider, be.call_count()), ("local-db", 1));
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        // A month the listing does not have is not found (page 2 adds nothing).
        let err = report(
            &mut db,
            &fetcher,
            "BBCA",
            Some(d(2026, 2, 27)),
            Duration::from_secs(5),
            d(2026, 11, 6),
        )
        .await
        .unwrap_err();
        assert_eq!(err.exit_code(), 4, "{err}");
        assert_eq!(months(&db).unwrap().len(), 2);
    }
}
