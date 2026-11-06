//! Parsers for the monthly "Pemegang Saham di Atas 1%" files.
//!
//! XLSX (June 2026 onwards): a disclaimer block, then a header row with the
//! twelve columns below, then one row per holder. Dates are Excel serials
//! formatted `dd-mmm-yyyy`, share counts are numbers, percentages are floats
//! with binary noise (`1.1499999999999999`). Feb/Mar 2026 files label the
//! fifth column `INVESTOR_TYPE` and use two-letter codes. From the register as
//! of 30 Oct 2026 a thirteenth column, `CONTROLLER_OR_AFFILIATE`, follows
//! `INVESTOR_CLASSIFICATION`; columns are found by name, so both layouts load.
//!
//! Every row keeps its own `DATE` and its sheet row (`line`). A holder can
//! sit on several rows of one ticker (one per SID, one per scrip account
//! without an SID); the database sums them, the parser keeps them apart.
//!
//! PDF (Feb to May 2026): the same table printed by a PDF generator; the text
//! layer is parsed line by line.
//!
//! Numbers printed as text (PDF tokens, or XLSX cells stored as strings) go
//! through [`parse_share_count`] and [`parse_percent`], which tell Indonesian
//! notation (`1.234.567`, `7,19`) from English (`1,234,567`, `7.19`) per
//! column. A percentage outside 0..=100 or a fractional share count skips the
//! row with a warning instead of storing a wrong number.
//!
//! Fixture: `tests/fixtures/ownership/`.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use calamine::{Data, Reader};
use chrono::NaiveDate;

use crate::error::{IdxError, Result};

use super::normalize;

pub const EXPECTED_HEADER: [&str; 12] = [
    "DATE",
    "SHARE_CODE",
    "ISSUER_NAME",
    "INVESTOR_NAME",
    "INVESTOR_CLASSIFICATION",
    "LOCAL_FOREIGN",
    "NATIONALITY",
    "DOMICILE",
    "HOLDINGS_SCRIPLESS",
    "HOLDINGS_SCRIP",
    "TOTAL_HOLDING_SHARES",
    "PERCENTAGE",
];

/// The optional column that registers from 30 Oct 2026 add.
pub const CONTROLLER_HEADER: &str = "CONTROLLER_OR_AFFILIATE";

/// One parsed line before it is stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Where the line sits in the source: the sheet row (XLSX, 1-based) or
    /// the text line (PDF, 1-based). Unique within a file.
    pub line: u32,
    /// The line's own `DATE` (usually the file's `as_of`).
    pub as_of: NaiveDate,
    pub symbol: String,
    pub issuer_name: Option<String>,
    pub holder_raw: String,
    pub investor_type: Option<String>,
    pub local_foreign: Option<String>,
    pub nationality: Option<String>,
    pub domicile: Option<String>,
    pub shares_scripless: Option<i64>,
    pub shares_scrip: Option<i64>,
    pub shares_total: i64,
    pub pct: f64,
    /// `CONTROLLER` or `AFFILIATE OF THE CONTROLLER` (registers from 30 Oct
    /// 2026), `None` when blank or when the file has no such column.
    pub controller_or_affiliate: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    /// The most common row date: the release this file is filed under.
    pub as_of: NaiveDate,
    pub rows: Vec<Row>,
    /// Lines that were skipped and why (surfaced as warnings).
    pub skipped: Vec<String>,
    /// Whether the file has the `CONTROLLER_OR_AFFILIATE` column.
    pub controller_column: bool,
}

impl Parsed {
    pub fn issuers(&self) -> usize {
        self.rows
            .iter()
            .map(|r| r.symbol.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// Rows dated other than `as_of`, grouped by date (oldest first): the
    /// date, how many rows, and their distinct symbols. The database keeps
    /// them but leaves them out of the release's holdings.
    pub fn other_dates(&self) -> Vec<(NaiveDate, usize, Vec<String>)> {
        let mut by_date: BTreeMap<NaiveDate, (usize, BTreeSet<&str>)> = BTreeMap::new();
        for r in self.rows.iter().filter(|r| r.as_of != self.as_of) {
            let e = by_date.entry(r.as_of).or_default();
            e.0 += 1;
            e.1.insert(r.symbol.as_str());
        }
        by_date
            .into_iter()
            .map(|(d, (n, syms))| (d, n, syms.into_iter().map(str::to_string).collect()))
            .collect()
    }

    /// One warning per other date, e.g. "the file also lists 2 rows dated
    /// 2026-01-30 (AAAA, CCCC); they are kept but are not part of the
    /// 2026-02-27 holdings". Empty for a single-date file.
    pub fn other_date_warnings(&self) -> Vec<String> {
        self.other_dates()
            .into_iter()
            .map(|(d, n, syms)| other_date_note("the file", n, d, &syms, self.as_of))
            .collect()
    }
}

/// Wording shared by the import warning and the database's release notes.
pub fn other_date_note(
    what: &str,
    rows: usize,
    date: NaiveDate,
    symbols: &[String],
    as_of: NaiveDate,
) -> String {
    const SHOWN: usize = 5;
    let mut list = symbols
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if symbols.len() > SHOWN {
        list.push_str(&format!(" and {} more", symbols.len() - SHOWN));
    }
    let (it, are) = if rows == 1 {
        ("it is", "is")
    } else {
        ("they are", "are")
    };
    format!(
        "{what} also lists {rows} row{} dated {date} ({list}); {it} kept but {are} not part of the {as_of} holdings",
        if rows == 1 { "" } else { "s" },
    )
}

/// Excel serial day to a date (1900 system).
pub fn excel_date(serial: f64) -> Option<NaiveDate> {
    let base = NaiveDate::from_ymd_opt(1899, 12, 30)?;
    base.checked_add_days(chrono::Days::new(serial.floor() as u64))
}

/// Percent with the float noise rounded away (`1.1499999999999999` -> `1.15`).
pub fn clean_pct(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

fn cell_text(c: &Data) -> String {
    match c {
        Data::String(s) => s.trim().to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 {
                format!("{}", *f as i64)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::DateTime(d) => d.as_f64().to_string(),
        Data::DateTimeIso(s) | Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#{e:?}"),
        Data::Empty => String::new(),
    }
}

/// Splits `s` on `sep` and checks thousands grouping: the first group has 1-3
/// digits, every later group exactly 3.
fn thousands(s: &str, sep: char) -> Option<i64> {
    let groups: Vec<&str> = s.split(sep).collect();
    let digits = |g: &str| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit());
    let first_ok = groups[0].len() <= 3 && digits(groups[0]);
    if !first_ok || !groups[1..].iter().all(|g| g.len() == 3 && digits(g)) {
        return None;
    }
    groups.concat().parse().ok()
}

/// Share count printed as text. Spaces (including U+00A0 and U+202F) are
/// dropped, then:
/// - digits only: the number;
/// - one separator kind (`.` or `,`) used once and followed by exactly 3
///   digits, or used several times: thousands groups (first group 1-3 digits,
///   every later group exactly 3), so `1.234.567`, `1,234,567` and `1,000`;
/// - one separator kind used once and not followed by 3 digits, or both kinds
///   present: the last separator is the decimal mark and the decimals must all
///   be `0` (`2.500.000,00`, `1234,00`); the integer part follows the
///   thousands rule with the other separator.
///
/// Anything else (`%`, a sign, letters, `10,5`) or an empty string is `None`.
pub fn parse_share_count(raw: &str) -> Option<i64> {
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    if s.is_empty()
        || !s
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    {
        return None;
    }
    let dots = s.matches('.').count();
    let commas = s.matches(',').count();
    let integer = |int: &str, sep: Option<char>| -> Option<i64> {
        match sep {
            Some(sep) if int.contains(sep) => thousands(int, sep),
            _ if int.is_empty() => None,
            _ => int.parse().ok(),
        }
    };
    let decimal_tail = |at: usize, other: Option<char>| -> Option<i64> {
        let (int, dec) = (&s[..at], &s[at + 1..]);
        if dec.is_empty() || !dec.bytes().all(|b| b == b'0') {
            return None;
        }
        integer(int, other)
    };
    match (dots, commas) {
        (0, 0) => s.parse().ok(),
        (n, 0) | (0, n) => {
            let sep = if dots > 0 { '.' } else { ',' };
            let at = s.find(sep)?;
            if n > 1 || s.len() - at - 1 == 3 {
                thousands(&s, sep)
            } else {
                decimal_tail(at, None)
            }
        }
        _ => {
            let at = s.rfind(['.', ','])?;
            let mark = s[at..].chars().next()?;
            let other = if mark == '.' { ',' } else { '.' };
            if s[..at].contains(mark) {
                return None;
            }
            decimal_tail(at, Some(other))
        }
    }
}

/// Percentage printed as text; spaces and a trailing `%` are ignored. One
/// separator (`.` or `,`) used once is the decimal mark: a share of issued
/// capital never needs a thousands separator, so `1,000` is 1.0. With both
/// kinds the last one is the decimal mark and the other is dropped. One kind
/// repeated (`1.234.5`) is `None`, and so is anything outside 0..=100.
pub fn parse_percent(raw: &str) -> Option<f64> {
    let s: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let s = s.strip_suffix('%').unwrap_or(&s);
    if !s.bytes().any(|b| b.is_ascii_digit())
        || !s
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ',')
    {
        return None;
    }
    let dots = s.matches('.').count();
    let commas = s.matches(',').count();
    let text = match (dots, commas) {
        (0, 0) => s.to_string(),
        (1, 0) => s.to_string(),
        (0, 1) => s.replace(',', "."),
        (_, 0) | (0, _) => return None,
        _ => {
            let at = s.rfind(['.', ','])?;
            let mark = s[at..].chars().next()?;
            if s[..at].contains(mark) {
                return None;
            }
            let other = if mark == '.' { ',' } else { '.' };
            s.replace(other, "").replace(',', ".")
        }
    };
    let v: f64 = text.parse().ok()?;
    (v.is_finite() && (0.0..=100.0).contains(&v)).then_some(v)
}

/// A share-count cell: numbers as stored (non-negative, rounded), text through
/// [`parse_share_count`].
fn cell_shares(c: &Data) -> Option<i64> {
    let v = match c {
        Data::Float(f) => *f,
        Data::Int(i) => *i as f64,
        Data::DateTime(d) => d.as_f64(),
        Data::String(s) => return parse_share_count(s),
        _ => return None,
    };
    (v.is_finite() && v >= 0.0).then(|| v.round() as i64)
}

/// A percentage cell: numbers as stored, text through [`parse_percent`]; both
/// must lie within 0..=100.
fn cell_pct(c: &Data) -> Option<f64> {
    let v = match c {
        Data::Float(f) => *f,
        Data::Int(i) => *i as f64,
        Data::String(s) => return parse_percent(s),
        _ => return None,
    };
    (v.is_finite() && (0.0..=100.0).contains(&v)).then_some(v)
}

/// The cell as quoted in a warning (`"10,5"`, `"150"`, or `empty`).
fn quoted(c: &Data) -> String {
    match cell_text(c) {
        t if t.is_empty() => "empty".to_string(),
        t => format!("{t:?}"),
    }
}

fn cell_date(c: &Data) -> Option<NaiveDate> {
    match c {
        Data::DateTime(d) => excel_date(d.as_f64()),
        Data::Float(f) => excel_date(*f),
        Data::Int(i) => excel_date(*i as f64),
        Data::DateTimeIso(s) => NaiveDate::parse_from_str(&s[..s.len().min(10)], "%Y-%m-%d").ok(),
        Data::String(s) => parse_text_date(s),
        _ => None,
    }
}

/// `31-Aug-2026`, `31 Agustus 2026`, `2026-08-31`, `31/08/2026`.
pub fn parse_text_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    for fmt in [
        "%d-%b-%Y", "%d-%b-%y", "%Y-%m-%d", "%d/%m/%Y", "%d %b %Y", "%d-%m-%Y",
    ] {
        if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
            return Some(d);
        }
    }
    parse_indonesian_date(s)
}

/// `31 Agustus 2026` -> 2026-08-31.
pub fn parse_indonesian_date(s: &str) -> Option<NaiveDate> {
    const MONTHS: [&str; 12] = [
        "januari",
        "februari",
        "maret",
        "april",
        "mei",
        "juni",
        "juli",
        "agustus",
        "september",
        "oktober",
        "november",
        "desember",
    ];
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    let day: u32 = parts[0].parse().ok()?;
    let month = MONTHS
        .iter()
        .position(|m| m.eq_ignore_ascii_case(parts[1]))? as u32
        + 1;
    let year: i32 = parts[2].parse().ok()?;
    NaiveDate::from_ymd_opt(year, month, day)
}

/// Where each column sits: the twelve required ones in `EXPECTED_HEADER`
/// order, and the optional controller flag.
struct Columns {
    pos: [usize; 12],
    controller: Option<usize>,
}

fn header_columns(row: &[Data]) -> Option<Columns> {
    let labels: Vec<String> = row
        .iter()
        .map(|c| {
            let mut l = cell_text(c).to_uppercase().replace(' ', "_");
            if l == "INVESTOR_TYPE" {
                l = "INVESTOR_CLASSIFICATION".into();
            }
            l
        })
        .collect();
    let mut pos = [0usize; 12];
    for (slot, want) in pos.iter_mut().zip(EXPECTED_HEADER) {
        *slot = labels.iter().position(|l| l == want)?;
    }
    Some(Columns {
        pos,
        controller: labels.iter().position(|l| l == CONTROLLER_HEADER),
    })
}

/// Parses an XLSX workbook from memory. Rejects workbooks that are not the
/// above-1 % register (the above-5 % and investor-type files share the page).
pub fn parse_xlsx(bytes: &[u8]) -> Result<Parsed> {
    let mut wb = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
        .map_err(|e| IdxError::Usage(format!("not a readable XLSX workbook: {e}")))?;
    let sheet_names = wb.sheet_names().to_vec();
    let first = sheet_names
        .first()
        .cloned()
        .ok_or_else(|| IdxError::Usage("workbook has no sheets".into()))?;
    let range = wb
        .worksheet_range(&first)
        .map_err(|e| IdxError::Usage(format!("cannot read sheet {first:?}: {e}")))?;

    // `rows()` starts at the first used row; `line` is the absolute sheet row.
    let first_row = range.start().map(|(r, _)| r).unwrap_or(0);
    let mut rows_iter = range.rows().enumerate();
    let mut columns: Option<Columns> = None;
    for (_, row) in rows_iter.by_ref().take(60) {
        if let Some(c) = header_columns(row) {
            columns = Some(c);
            break;
        }
    }
    let cols = columns.ok_or_else(|| {
        IdxError::Usage(
            "this workbook is not the \"Pemegang Saham di Atas 1%\" register: the 12-column header (DATE, SHARE_CODE, ..., PERCENTAGE) was not found in the first 60 rows"
                .into(),
        )
    })?;

    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    for (i, row) in rows_iter {
        let line = first_row + i as u32 + 1;
        let get = |k: usize| row.get(cols.pos[k]).unwrap_or(&Data::Empty);
        let symbol = cell_text(get(1)).to_uppercase();
        let holder_raw = cell_text(get(3));
        if symbol.is_empty() && holder_raw.is_empty() {
            continue;
        }
        if symbol.is_empty() || holder_raw.is_empty() {
            skipped.push(format!("row {line}: missing symbol or holder"));
            continue;
        }
        let Some(as_of) = cell_date(get(0)) else {
            skipped.push(format!("row {line}: unreadable date"));
            continue;
        };
        let Some(total) = cell_shares(get(10)) else {
            skipped.push(format!(
                "row {line}: TOTAL_HOLDING_SHARES {} is not a whole number of shares",
                quoted(get(10))
            ));
            continue;
        };
        let Some(pct) = cell_pct(get(11)) else {
            skipped.push(format!(
                "row {line}: PERCENTAGE {} is not a percentage between 0 and 100",
                quoted(get(11))
            ));
            continue;
        };
        let controller_or_affiliate = cols
            .controller
            .and_then(|k| row.get(k))
            .and_then(|c| normalize::controller_or_affiliate(&cell_text(c)));
        rows.push(Row {
            line,
            as_of,
            symbol,
            issuer_name: normalize::opt(&cell_text(get(2))),
            holder_raw,
            investor_type: normalize::investor_type(&cell_text(get(4))),
            local_foreign: normalize::local_foreign(&cell_text(get(5))),
            nationality: normalize::opt(&cell_text(get(6))),
            domicile: normalize::opt(&cell_text(get(7))),
            shares_scripless: cell_shares(get(8)),
            shares_scrip: cell_shares(get(9)),
            shares_total: total,
            pct: clean_pct(pct),
            controller_or_affiliate,
        });
    }
    finish(rows, skipped, cols.controller.is_some())
}

fn finish(rows: Vec<Row>, skipped: Vec<String>, controller_column: bool) -> Result<Parsed> {
    if rows.is_empty() {
        return Err(IdxError::Usage("no holder rows found in the file".into()));
    }
    // The file is one snapshot; take the most common date as its as_of.
    let mut counts: std::collections::HashMap<NaiveDate, usize> = Default::default();
    for r in &rows {
        *counts.entry(r.as_of).or_default() += 1;
    }
    let as_of = counts
        .into_iter()
        .max_by_key(|(d, n)| (*n, *d))
        .map(|(d, _)| d)
        .expect("rows is non-empty");
    Ok(Parsed {
        as_of,
        rows,
        skipped,
        controller_column,
    })
}

/// KSEI investor-type codes used in the PDF releases.
const TYPE_CODES: &[&str] = &["CP", "ID", "IB", "MF", "SC", "IS", "PF", "FD", "OT"];

/// Parses the text layer of one of the PDF releases (Feb to May 2026). Each
/// data line reads `<dd-Mon-yyyy> <CODE> <issuer> <holder> <type> <L|A>
/// <nationality?> <domicile> <scripless> <scrip> <total> <pct>`; the two name
/// columns can contain spaces, so the line is split from both ends.
pub fn parse_pdf_text(text: &str) -> Result<Parsed> {
    let mut rows = Vec::new();
    let mut skipped = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        if toks.len() < 9 {
            continue;
        }
        let Some(as_of) = parse_text_date(toks[0]) else {
            continue;
        };
        let symbol = toks[1].to_uppercase();
        if symbol.len() > 12
            || !symbol
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            continue;
        }
        let n = toks.len();
        let (Some(pct), Some(total), Some(scrip), Some(scripless)) = (
            parse_percent(toks[n - 1]),
            parse_share_count(toks[n - 2]),
            parse_share_count(toks[n - 3]),
            parse_share_count(toks[n - 4]),
        ) else {
            skipped.push(format!("line {}: numeric tail not recognised", i + 1));
            continue;
        };
        // Between the names and the numbers the register prints, when known,
        // `<type> <L|A> [nationality] <domicile>`; scrip-only (eBAE) rows omit
        // some or all of them, so every one of these columns is optional.
        let middle_end = n - 4;
        let mut lf_idx = None;
        for j in (3..middle_end).rev() {
            let t = toks[j].to_uppercase();
            if t == "L" || t == "A" {
                lf_idx = Some(j);
                break;
            }
        }
        let (names_end, inv_type, lf, nationality, domicile) = match lf_idx {
            Some(lf) => {
                let prev = toks[lf - 1].to_uppercase();
                let has_type = TYPE_CODES.contains(&prev.as_str());
                let names_end = if has_type { lf - 1 } else { lf };
                let tail = &toks[lf + 1..middle_end];
                let (nat, dom) = match tail.len() {
                    0 => (None, None),
                    1 => (None, Some(tail[0].to_string())),
                    _ => (
                        Some(tail[..tail.len() - 1].join(" ")),
                        Some(tail[tail.len() - 1].to_string()),
                    ),
                };
                (
                    names_end,
                    if has_type { Some(prev) } else { None },
                    Some(toks[lf].to_uppercase()),
                    nat,
                    dom,
                )
            }
            None => (middle_end, None, None, None, None),
        };
        let middle = &toks[2..names_end];
        if middle.is_empty() {
            skipped.push(format!("line {}: no names", i + 1));
            continue;
        }
        // Issuer names end with "Tbk" in the register; split after the first one
        // (holder names may contain TBK further right).
        let split = middle
            .iter()
            .position(|t| t.eq_ignore_ascii_case("tbk") || t.eq_ignore_ascii_case("tbk."))
            .map(|p| p + 1)
            .filter(|p| *p < middle.len())
            .unwrap_or(1);
        let issuer = middle[..split].join(" ");
        let holder = middle[split..].join(" ");
        rows.push(Row {
            line: i as u32 + 1,
            as_of,
            symbol,
            issuer_name: normalize::opt(&issuer),
            holder_raw: holder,
            investor_type: inv_type.as_deref().and_then(normalize::investor_type),
            local_foreign: lf.as_deref().and_then(normalize::local_foreign),
            nationality: nationality.as_deref().and_then(normalize::opt),
            domicile: domicile.as_deref().and_then(normalize::opt),
            shares_scripless: Some(scripless),
            shares_scrip: Some(scrip),
            shares_total: total,
            pct: clean_pct(pct),
            controller_or_affiliate: None,
        });
    }
    finish(rows, skipped, false)
}

/// Extracts the text layer of a PDF and parses it.
pub fn parse_pdf(bytes: &[u8]) -> Result<Parsed> {
    let text = pdf_extract::extract_text_from_mem(bytes)
        .map_err(|e| IdxError::Usage(format!("cannot read PDF text: {e}")))?;
    parse_pdf_text(&text)
}

/// Picks the parser from the file extension or the magic bytes.
pub fn parse_any(name: &str, bytes: &[u8]) -> Result<(Parsed, &'static str)> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".xlsx") || bytes.starts_with(b"PK\x03\x04") {
        return parse_xlsx(bytes).map(|p| (p, "xlsx"));
    }
    if lower.ends_with(".pdf") || bytes.starts_with(b"%PDF") {
        return parse_pdf(bytes).map(|p| (p, "pdf"));
    }
    Err(IdxError::Usage(format!(
        "{name}: expected an .xlsx or .pdf ownership report"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    const XLSX: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20260831_excerpt.xlsx");
    const XLSX_PREV: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20260731_synthetic.xlsx");
    const XLSX_TEXT: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_text_numbers_synthetic.xlsx");
    const XLSX_OCT: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_20261030_excerpt.xlsx");
    const XLSX_MIXED: &[u8] =
        include_bytes!("../../tests/fixtures/ownership/above1_mixed_dates_synthetic.xlsx");

    #[test]
    fn parses_the_real_excerpt() {
        let p = parse_xlsx(XLSX).unwrap();
        assert_eq!(p.as_of, NaiveDate::from_ymd_opt(2026, 8, 31).unwrap());
        assert!(p.rows.len() > 10, "{}", p.rows.len());
        assert!(p.skipped.is_empty(), "{:?}", p.skipped);
        let first = &p.rows[0];
        assert_eq!(first.symbol, "AADI");
        assert_eq!(first.holder_raw, "PT PERSADA CAPITAL INVESTAMA");
        assert_eq!(first.investor_type.as_deref(), Some("CORPORATE"));
        assert_eq!(first.local_foreign.as_deref(), Some("L"));
        assert_eq!(first.nationality, None, "column G is blank in the source");
        assert_eq!(first.domicile.as_deref(), Some("INDONESIA"));
        assert_eq!(first.shares_total, 119_887_170);
        assert_eq!(first.pct, 1.54);
        let noisy = p
            .rows
            .iter()
            .find(|r| r.holder_raw == "PT TRINUGRAHA THOHIR")
            .unwrap();
        assert_eq!(noisy.pct, 1.15, "float noise is rounded away");
        let bank = p
            .rows
            .iter()
            .find(|r| r.holder_raw == "SARATOGA INVESTAMA SEDAYA TBK")
            .unwrap();
        assert_eq!(bank.investor_type.as_deref(), Some("PRIVATE BANK"));
        assert!(p.issuers() >= 2);
    }

    #[test]
    fn synthetic_previous_month_parses() {
        let p = parse_xlsx(XLSX_PREV).unwrap();
        assert_eq!(p.as_of, NaiveDate::from_ymd_opt(2026, 7, 31).unwrap());
        assert!(!p.controller_column);
        assert!(p.other_dates().is_empty());
    }

    #[test]
    fn twelve_column_registers_have_no_controller_flag() {
        let p = parse_xlsx(XLSX).unwrap();
        assert!(!p.controller_column);
        assert!(p.rows.iter().all(|r| r.controller_or_affiliate.is_none()));
        assert!(p.other_date_warnings().is_empty());
        // Lines are distinct sheet rows, in file order.
        assert!(p.rows.windows(2).all(|w| w[0].line < w[1].line));
    }

    #[test]
    fn october_register_carries_the_controller_flag() {
        let p = parse_xlsx(XLSX_OCT).unwrap();
        assert_eq!(p.as_of, NaiveDate::from_ymd_opt(2026, 10, 30).unwrap());
        assert_eq!(p.rows.len(), 11, "{:?}", p.skipped);
        assert!(p.skipped.is_empty(), "{:?}", p.skipped);
        assert!(p.controller_column);
        let by = |name: &str| p.rows.iter().find(|r| r.holder_raw == name).unwrap();
        let dwimuria = by("PT DWIMURIA INVESTAMA ANDALAN");
        assert_eq!(
            dwimuria.line, 708,
            "the excerpt keeps the original sheet rows"
        );
        assert_eq!(
            dwimuria.controller_or_affiliate.as_deref(),
            Some("CONTROLLER")
        );
        assert_eq!(dwimuria.pct, 54.94);
        for name in [
            "PT CATURGUWIRATNA SUMAPALA",
            "PT TRICIPTA MANDHALA GUMILANG",
        ] {
            assert_eq!(
                by(name).controller_or_affiliate.as_deref(),
                Some("AFFILIATE OF THE CONTROLLER"),
                "{name}"
            );
        }
        let salim = by("ANTHONI SALIM");
        assert_eq!(salim.controller_or_affiliate, None);
        assert_eq!(salim.nationality.as_deref(), Some("INDONESIAN"));
        assert_eq!(salim.local_foreign.as_deref(), Some("L"));
        assert_eq!(salim.pct, 1.15);
        let timothy = by("TIMOTHY SIDDIK SHU");
        assert_eq!(timothy.symbol, "ZYRX");
        assert_eq!(timothy.line, 7152);
        assert_eq!(
            timothy.controller_or_affiliate.as_deref(),
            Some("CONTROLLER")
        );
        assert_eq!(timothy.shares_total, 752_076_700);
        assert_eq!(timothy.shares_scripless, Some(2_076_700));
        assert_eq!(timothy.shares_scrip, Some(750_000_000));
        assert_eq!(timothy.pct, 56.41);
        assert_eq!(timothy.investor_type.as_deref(), Some("INDIVIDUAL"));
    }

    #[test]
    fn mixed_dates_keep_each_row_date() {
        let p = parse_xlsx(XLSX_MIXED).unwrap();
        let feb = NaiveDate::from_ymd_opt(2026, 2, 27).unwrap();
        let jan = NaiveDate::from_ymd_opt(2026, 1, 30).unwrap();
        assert_eq!(p.as_of, feb, "the most common date names the release");
        assert_eq!(p.rows.len(), 5);
        assert_eq!(
            p.rows.iter().map(|r| (r.line, r.as_of)).collect::<Vec<_>>(),
            vec![(6, feb), (7, feb), (8, jan), (9, feb), (10, jan)]
        );
        assert_eq!(
            p.other_dates(),
            vec![(jan, 2, vec!["AAAA".to_string(), "CCCC".to_string()])]
        );
        assert_eq!(
            p.other_date_warnings(),
            vec!["the file also lists 2 rows dated 2026-01-30 (AAAA, CCCC); they are kept but are not part of the 2026-02-27 holdings".to_string()]
        );
        assert_eq!(
            p.rows[3].local_foreign.as_deref(),
            Some("A"),
            "F is foreign"
        );
    }

    #[test]
    fn other_date_note_lists_five_symbols() {
        let d = NaiveDate::from_ymd_opt(2026, 1, 30).unwrap();
        let as_of = NaiveDate::from_ymd_opt(2026, 2, 27).unwrap();
        let syms: Vec<String> = ["A", "B", "C", "D", "E", "F", "G"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            other_date_note("the file", 9, d, &syms, as_of),
            "the file also lists 9 rows dated 2026-01-30 (A, B, C, D, E and 2 more); they are kept but are not part of the 2026-02-27 holdings"
        );
        assert_eq!(
            other_date_note("the file", 1, d, &syms[..1], as_of),
            "the file also lists 1 row dated 2026-01-30 (A); it is kept but is not part of the 2026-02-27 holdings"
        );
    }

    #[test]
    fn rejects_other_workbooks_and_garbage() {
        assert!(parse_xlsx(b"not a zip").is_err());
        let err = parse_any("x.csv", b"a,b,c").unwrap_err();
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    #[allow(clippy::excessive_precision)]
    fn dates_and_percent_helpers() {
        assert_eq!(
            excel_date(46265.0),
            Some(NaiveDate::from_ymd_opt(2026, 8, 31).unwrap())
        );
        assert_eq!(
            parse_text_date("27-Feb-2026"),
            Some(NaiveDate::from_ymd_opt(2026, 2, 27).unwrap())
        );
        assert_eq!(
            parse_indonesian_date("31 Agustus 2026"),
            Some(NaiveDate::from_ymd_opt(2026, 8, 31).unwrap())
        );
        assert_eq!(clean_pct(1.1499999999999999), 1.15);
        assert_eq!(clean_pct(54.9412), 54.9412);
    }

    #[test]
    #[allow(clippy::excessive_precision)]
    fn text_numbers_by_column() {
        // (input, share count, percent)
        let cases: &[(&str, Option<i64>, Option<f64>)] = &[
            ("1234567", Some(1_234_567), None),
            ("1.234.567", Some(1_234_567), None),
            ("1,234,567", Some(1_234_567), None),
            ("1 234 567", Some(1_234_567), None),
            ("1\u{a0}234\u{202f}567", Some(1_234_567), None),
            ("1.234", Some(1_234), Some(1.234)),
            ("2.500.000,00", Some(2_500_000), None),
            ("1,234.00", Some(1_234), None),
            ("1234,00", Some(1_234), None),
            ("10,5", None, Some(10.5)),
            ("12.34.567", None, None),
            ("7,19", None, Some(7.19)),
            ("7.19", None, Some(7.19)),
            ("7,19%", None, Some(7.19)),
            ("54,94%", None, Some(54.94)),
            (" 54.94 % ", None, Some(54.94)),
            ("1,000", Some(1_000), Some(1.0)),
            ("100,5", None, None),
            ("100", Some(100), Some(100.0)),
            ("0", Some(0), Some(0.0)),
            ("1.1499999999999999", None, Some(1.1499999999999999)),
            ("1.234,5", None, None),
            (".5", None, Some(0.5)),
            ("-5", None, None),
            ("", None, None),
            ("abc", None, None),
            ("%", None, None),
            ("5%4", None, None),
        ];
        for (input, shares, pct) in cases {
            assert_eq!(parse_share_count(input), *shares, "shares {input:?}");
            assert_eq!(parse_percent(input), *pct, "percent {input:?}");
        }
        assert_eq!(
            clean_pct(parse_percent("1.1499999999999999").unwrap()),
            1.15
        );
    }

    #[test]
    fn text_number_cells_import_or_skip_with_a_reason() {
        let p = parse_xlsx(XLSX_TEXT).unwrap();
        assert_eq!(p.as_of, NaiveDate::from_ymd_opt(2026, 3, 31).unwrap());
        let got: Vec<(&str, i64, f64)> = p
            .rows
            .iter()
            .map(|r| (r.holder_raw.as_str(), r.shares_total, r.pct))
            .collect();
        assert_eq!(
            got,
            vec![
                ("KOMA DESIMAL", 282_886_300, 7.19),
                ("TITIK RIBUAN", 1_234_567, 1.5),
                ("KOMA RIBUAN", 1_234_567, 12.5),
                ("DESIMAL NOL", 2_500_000, 3.25),
                ("SERIBU PERSEN", 100_000, 1.0),
                ("PERSEN PENUH", 5_494_000, 54.94),
            ]
        );
        let titik = &p.rows[1];
        assert_eq!(titik.shares_scripless, Some(1_234_567));
        assert_eq!(titik.shares_scrip, Some(0));
        assert_eq!(
            p.skipped,
            vec![
                "row 12: PERCENTAGE \"100,5\" is not a percentage between 0 and 100".to_string(),
                "row 13: TOTAL_HOLDING_SHARES \"10,5\" is not a whole number of shares".to_string(),
            ]
        );
    }

    #[test]
    fn pdf_text_lines_parse() {
        let text = "\
Data Kepemilikan Saham Perusahaan Terbuka di atas 1%
DATE SHARE_CODE ISSUER_NAME INVESTOR_NAME INVESTOR_TYPE LOCAL_FOREIGN NATIONALITY DOMICILE HOLDINGS_SCRIPLESS HOLDINGS_SCRIP TOTAL_HOLDING_SHARES PERCENTAGE
27-Feb-2026 AADI ADARO ANDALAN INDONESIA Tbk ADARO STRATEGIC INVESTMENTS CP L INDONESIA 3200142830 0 3200142830 41.10
27-Feb-2026 AADI ADARO ANDALAN INDONESIA Tbk ALAMTRI RESOURCES INDONESIA TBK PT CP L INDONESIA INDONESIA 1197023942 0 1197023942 15.37
27-Feb-2026 BBCA BANK CENTRAL ASIA Tbk PT DWIMURIA INVESTAMA ANDALAN CP L INDONESIA INDONESIA 67729950000 0 67729950000 54.94
27-Feb-2026 ABBA MAHAKA MEDIA Tbk MEDIAHUIS IRELAND LIMITED 0 282.886.300 282.886.300 7,19
27-Feb-2026 AKPI ARGHA KARYA PRIMA INDUSTRY Tbk NAWA PANDUTA CP L 0 92.133.534 92.133.534 15,05
Halaman 1 dari 73
";
        let p = parse_pdf_text(text).unwrap();
        assert_eq!(p.rows.len(), 5, "{:?}", p.skipped);
        assert_eq!(p.as_of, NaiveDate::from_ymd_opt(2026, 2, 27).unwrap());
        assert!(!p.controller_column);
        assert_eq!(
            p.rows.iter().map(|r| r.line).collect::<Vec<_>>(),
            vec![3, 4, 5, 6, 7],
            "text line numbers"
        );
        assert_eq!(
            p.rows[0].issuer_name.as_deref(),
            Some("ADARO ANDALAN INDONESIA Tbk")
        );
        assert_eq!(p.rows[0].holder_raw, "ADARO STRATEGIC INVESTMENTS");
        assert_eq!(p.rows[0].nationality, None);
        assert_eq!(p.rows[0].domicile.as_deref(), Some("INDONESIA"));
        assert_eq!(p.rows[1].holder_raw, "ALAMTRI RESOURCES INDONESIA TBK PT");
        assert_eq!(p.rows[1].nationality.as_deref(), Some("INDONESIA"));
        assert_eq!(p.rows[2].investor_type.as_deref(), Some("CORPORATE"));
        assert_eq!(p.rows[2].pct, 54.94);
        assert_eq!(p.rows[2].shares_total, 67_729_950_000);
        // Scrip-only row: no type, L/A or domicile, dotted thousands, comma decimal.
        assert_eq!(p.rows[3].holder_raw, "MEDIAHUIS IRELAND LIMITED");
        assert_eq!(p.rows[3].investor_type, None);
        assert_eq!(p.rows[3].local_foreign, None);
        assert_eq!(p.rows[3].shares_scrip, Some(282_886_300));
        assert_eq!(p.rows[3].pct, 7.19);
        // Type and L/A but no domicile.
        assert_eq!(p.rows[4].holder_raw, "NAWA PANDUTA");
        assert_eq!(p.rows[4].investor_type.as_deref(), Some("CORPORATE"));
        assert_eq!(p.rows[4].domicile, None);
    }
}
