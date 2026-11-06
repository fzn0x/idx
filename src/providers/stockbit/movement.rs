//! Shareholder movements from `GET /insider/company/majorholder`: reported
//! changes in a holder's stake, newest first, 50 rows a page.
//!
//! Layout as recorded by 0xrsydn/idx-cli (MIT) on 2026-11-03
//! (`tests/fixtures/stockbit/majorholder*.json`):
//! `{"data":{"is_more":bool,"movement":[rows]|null},"message":..}`. A row:
//! `id` (the **holder's** id, not the row's), `name`, `symbol`, `date`
//! (`"25 Mar 26"`), `previous` / `current` / `changes`, each
//! `{value:"193,206"|"+147,933", percentage:"0.0002"|"+0.07"}`, `action_type`
//! (`ACTION_TYPE_BUY`, `_SELL`, `_TRANSFER`, `_CROSS`, `_RIGHT_ISSUE`,
//! `_PRIVATE_PLACEMENT`, `_TENDER_OFFER`, ...), `data_source.type`
//! (`SOURCE_TYPE_IDX` insider disclosure, `SOURCE_TYPE_KSEI` 5 % holder),
//! `nationality`, `badges` (`SHAREHOLDER_BADGE_DIREKTUR`, `_KOMISARIS`),
//! `price.raw.value`, `value.raw.value` (IDR), `is_estimated_price`,
//! `purpose`, `broker_detail.{code,group}`.
//!
//! Query parameters, all optional: `page` (from 1), `symbol`, `date_start` /
//! `date_end` (YYYY-MM-DD), `insider` (holder id), `action_type`,
//! `source_type`.

use chrono::NaiveDate;
use serde_json::Value;

use crate::error::{IdxError, Result};
use crate::model::{HolderMovement, MovementFilter, MovementSource, Symbol};

use super::{stockbit_code, EXODUS, NAME};

/// What to ask the feed for.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MovementQuery {
    pub symbol: Option<Symbol>,
    pub from: Option<NaiveDate>,
    pub to: Option<NaiveDate>,
    /// A holder's id (digits), as in a row's `holder_id`.
    pub holder_id: Option<String>,
    pub action: Option<MovementFilter>,
    pub source: Option<MovementSource>,
}

impl MovementQuery {
    /// Rejects what the feed cannot answer, before any request: an index, a
    /// holder id that is not digits, a range that ends before it starts.
    pub fn validate(&self) -> Result<()> {
        if let Some(s) = &self.symbol {
            if s.is_index() {
                return Err(IdxError::Usage(format!(
                    "{s} is an index; holder movements belong to stocks"
                )));
            }
        }
        if let Some(h) = &self.holder_id {
            if h.is_empty() || !h.chars().all(|c| c.is_ascii_digit()) || h.len() > 20 {
                return Err(IdxError::Usage(format!(
                    "holder {h:?} is not a holder id: use the digits in the HOLDER ID column (holder_id in JSON)"
                )));
            }
        }
        if let (Some(f), Some(t)) = (self.from, self.to) {
            if f > t {
                return Err(IdxError::Usage(format!(
                    "the range starts ({f}) after it ends ({t})"
                )));
            }
        }
        Ok(())
    }
}

fn action_type(a: MovementFilter) -> &'static str {
    match a {
        MovementFilter::Buy => "ACTION_TYPE_BUY",
        MovementFilter::Sell => "ACTION_TYPE_SELL",
        MovementFilter::Transfer => "ACTION_TYPE_TRANSFER",
        MovementFilter::Cross => "ACTION_TYPE_CROSS",
        MovementFilter::CorporateAction => "ACTION_TYPE_CORPACTION",
    }
}

fn source_type(s: MovementSource) -> &'static str {
    match s {
        MovementSource::Idx => "SOURCE_TYPE_IDX",
        MovementSource::Ksei => "SOURCE_TYPE_KSEI",
    }
}

/// The URL of one page, parameters in a fixed order so it is a stable cache
/// key. Every value is a page number, a code, a date, a digit id or a fixed
/// name, so nothing needs escaping ([`MovementQuery::validate`] first).
pub fn url(q: &MovementQuery, page: u32) -> String {
    let mut u = format!("{EXODUS}/insider/company/majorholder?page={page}");
    if let Some(s) = &q.symbol {
        u.push_str(&format!("&symbol={}", stockbit_code(s)));
    }
    if let Some(f) = q.from {
        u.push_str(&format!("&date_start={f}"));
    }
    if let Some(t) = q.to {
        u.push_str(&format!("&date_end={t}"));
    }
    if let Some(h) = &q.holder_id {
        u.push_str(&format!("&insider={h}"));
    }
    if let Some(a) = q.action {
        u.push_str(&format!("&action_type={}", action_type(a)));
    }
    if let Some(s) = q.source {
        u.push_str(&format!("&source_type={}", source_type(s)));
    }
    u
}

/// One page: its rows and whether more follow. A row without a code, a
/// holder name or a readable date is skipped.
pub fn parse_page(body: &str) -> Result<(Vec<HolderMovement>, bool)> {
    let v: Value = serde_json::from_str(body).map_err(|e| {
        IdxError::provider(
            NAME,
            format!("shareholder movements response not JSON: {e}"),
        )
    })?;
    let data = v
        .get("data")
        .filter(|d| d.is_object())
        .ok_or_else(|| IdxError::provider(NAME, "shareholder movements response had no data"))?;
    let rows = data
        .get("movement")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(row).collect())
        .unwrap_or_default();
    let more = data
        .get("is_more")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok((rows, more))
}

fn row(r: &Value) -> Option<HolderMovement> {
    let symbol = text(r.get("symbol")).and_then(|s| Symbol::parse(&s).ok())?;
    let holder = text(r.get("name"))?;
    let date = NaiveDate::parse_from_str(&text(r.get("date"))?, "%d %b %y").ok()?;
    let shares = |k: &str| signed(r.get(k).and_then(|v| v.get("value")));
    let pct = |k: &str| percent(r.get(k).and_then(|v| v.get("percentage")));
    let raw = |k: &str| {
        positive(
            r.get(k)
                .and_then(|v| v.get("raw"))
                .and_then(|v| v.get("value")),
        )
    };
    let estimated = r.get("is_estimated_price").and_then(Value::as_bool) == Some(true);
    let (price, value) = (raw("price"), raw("value"));
    let label = |k: Option<&Value>, prefix: &str| {
        text(k).map(|s| s.strip_prefix(prefix).unwrap_or(&s).to_ascii_lowercase())
    };
    Some(HolderMovement {
        date,
        symbol,
        holder,
        holder_id: text(r.get("id")).filter(|s| s != "0"),
        roles: r
            .get("badges")
            .and_then(Value::as_array)
            .map(|b| {
                b.iter()
                    .filter_map(|x| text(Some(x)))
                    .map(|b| role(&b))
                    .collect()
            })
            .unwrap_or_default(),
        foreign: match text(r.get("nationality")).as_deref() {
            Some("NATIONALITY_TYPE_FOREIGN") => Some(true),
            Some("NATIONALITY_TYPE_LOCAL") => Some(false),
            _ => None,
        },
        action: label(r.get("action_type"), "ACTION_TYPE_"),
        source: label(
            r.get("data_source").and_then(|d| d.get("type")),
            "SOURCE_TYPE_",
        ),
        shares_from: shares("previous"),
        shares_to: shares("current"),
        shares_change: shares("changes"),
        pct_from: pct("previous"),
        pct_to: pct("current"),
        pct_change: pct("changes"),
        price: price.filter(|_| !estimated),
        estimated_price: price.filter(|_| estimated),
        value: value.filter(|_| !estimated),
        estimated_value: value.filter(|_| estimated),
        purpose: text(r.get("purpose")).filter(|p| p != "-"),
        broker: text(r.get("broker_detail").and_then(|b| b.get("code"))),
        broker_group: label(
            r.get("broker_detail").and_then(|b| b.get("group")),
            "BROKER_GROUP_",
        )
        .filter(|g| g != "unspecified"),
    })
}

/// `SHAREHOLDER_BADGE_DIREKTUR` is `director`; an unknown badge keeps its
/// own name, lowercased.
fn role(badge: &str) -> String {
    match badge.strip_prefix("SHAREHOLDER_BADGE_").unwrap_or(badge) {
        "DIREKTUR" => "director".into(),
        "KOMISARIS" => "commissioner".into(),
        other => other.to_ascii_lowercase(),
    }
}

fn text(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.trim().to_string()).filter(|s| !s.is_empty()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// `"+147,933"`, `"-10,000,000"`, `"0"` or a JSON number; exact shares.
fn signed(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => {
            let t = s.trim().replace(',', "");
            t.strip_prefix('+').unwrap_or(&t).parse().ok()
        }
        _ => None,
    }
}

/// `"+0.0001"`, `"-0.97"`, `"54.94"`, as the source rounds them.
fn percent(v: Option<&Value>) -> Option<f64> {
    let n = match v? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().replace(',', "").parse().ok()?,
        _ => return None,
    };
    n.is_finite().then_some(n)
}

/// A price or value; `0` means none.
fn positive(v: Option<&Value>) -> Option<f64> {
    super::parse::num(v?).filter(|n| n.is_finite() && *n > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/stockbit/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn find<'a>(rows: &'a [HolderMovement], holder: &str, sym: &str) -> &'a HolderMovement {
        rows.iter()
            .find(|r| r.holder == holder && r.symbol.as_str() == sym)
            .unwrap_or_else(|| panic!("no {holder} {sym}"))
    }

    #[test]
    fn an_insider_buy_with_its_filed_price() {
        let (rows, more) = parse_page(&fixture("majorholder_bbca")).unwrap();
        assert_eq!(rows.len(), 8);
        assert!(more);
        let r = find(&rows, "HENDRA TANUMIHARDJA", "BBCA");
        assert_eq!(r.date, d("2026-03-25"));
        assert_eq!(r.action.as_deref(), Some("buy"));
        assert_eq!(r.source.as_deref(), Some("idx"));
        assert_eq!(r.roles, ["director"]);
        assert_eq!(r.foreign, Some(false));
        assert_eq!(
            (r.shares_from, r.shares_to, r.shares_change),
            (Some(193_206), Some(341_139), Some(147_933))
        );
        // As the source rounds it: 0.0003 % stays 0.0003.
        assert_eq!(
            (r.pct_from, r.pct_to, r.pct_change),
            (Some(0.0002), Some(0.0003), Some(0.0001))
        );
        assert_eq!(r.price, Some(6982.0));
        assert_eq!(r.value, Some(1_032_868_206.0));
        assert_eq!((r.estimated_price, r.estimated_value), (None, None));
        assert_eq!(r.holder_id.as_deref(), Some("1000000439"));
        assert_eq!(
            (
                r.purpose.as_deref(),
                r.broker.as_deref(),
                r.broker_group.as_deref()
            ),
            (None, None, None)
        );
        assert_eq!(find(&rows, "TONNY KUSNADI", "BBCA").roles, ["commissioner"]);
        assert!(find(&rows, "A. WIDODO MULYONO", "BBCA").roles.is_empty());
    }

    #[test]
    fn a_ksei_sale_keeps_the_estimate_apart() {
        let (rows, more) = parse_page(&fixture("majorholder")).unwrap();
        assert_eq!(rows.len(), 8);
        assert!(more);
        let r = find(&rows, "RUDI R. SUTANTRA", "ATLA");
        assert_eq!(r.date, d("2026-10-30"));
        assert_eq!(r.action.as_deref(), Some("sell"));
        assert_eq!(r.source.as_deref(), Some("ksei"));
        assert_eq!(r.holder_id.as_deref(), Some("1000004481"));
        assert_eq!(r.shares_change, Some(-10_000_000));
        assert_eq!((r.pct_from, r.pct_change), (Some(40.81), Some(-0.16)));
        assert_eq!((r.price, r.value), (None, None));
        assert_eq!(r.estimated_price, Some(21.0));
        assert_eq!(r.estimated_value, Some(210_000_000.0));
        assert_eq!(r.broker.as_deref(), Some("AG"));
        assert_eq!(r.broker_group.as_deref(), Some("foreign"));
        // A rights issue dilutes without a change in shares.
        let b = find(&rows, "NUSANTARA UTAMA INVESTAMA", "BUVA");
        assert_eq!(b.action.as_deref(), Some("right_issue"));
        assert_eq!(b.shares_change, Some(0));
        assert_eq!(b.pct_change, Some(-0.97));
        assert_eq!((b.value, b.estimated_value), (None, None));
        assert_eq!(b.estimated_price, Some(353.0));
        assert_eq!(
            find(&rows, "WELLY THOMAS", "SGER").broker_group.as_deref(),
            Some("government")
        );
        let p = &rows[0];
        assert_eq!((p.symbol.as_str(), p.foreign), ("PEGE", Some(true)));
        assert_eq!(p.purpose.as_deref(), Some("Pembelian"));
    }

    #[test]
    fn a_last_page_and_other_labels() {
        let (rows, more) = parse_page(&fixture("majorholder_p2")).unwrap();
        assert!(!more);
        assert_eq!(rows.len(), 6);
        let c = &rows[0];
        assert_eq!(
            (c.symbol.as_str(), c.action.as_deref()),
            ("ENRG", Some("cross"))
        );
        assert_eq!(c.purpose.as_deref(), Some("Reverse REPO"));
        assert_eq!(c.price, Some(715.0));
        // A crossing whose price is only estimated.
        let m = &rows[1];
        assert_eq!((m.price, m.estimated_price), (None, Some(724.0)));
        let labels: Vec<_> = rows.iter().filter_map(|r| r.action.clone()).collect();
        assert!(
            labels.contains(&"private_placement".into()) && labels.contains(&"tender_offer".into())
        );
        let (rows, more) = parse_page(&fixture("majorholder_bbca_p2")).unwrap();
        assert_eq!((rows.len(), more), (4, false));
        assert_eq!(rows[0].date, d("2024-03-22"));
    }

    #[test]
    fn unknown_labels_pass_through_lowercased() {
        let body = r#"{"data":{"is_more":false,"movement":[{"id":"7","name":"X","symbol":"abcd",
            "date":"01 Feb 26","previous":{"value":"10","percentage":"1"},"current":{"value":"12","percentage":"1.2"},
            "changes":{"value":"+2","percentage":"+0.2"},"action_type":"ACTION_TYPE_BONUS_STOCK",
            "data_source":{"type":"SOURCE_TYPE_NEW"},"badges":["SHAREHOLDER_BADGE_PENGENDALI"],
            "nationality":"NATIONALITY_TYPE_UNSPECIFIED","price":{"raw":{"value":"0"}},
            "value":{"raw":{"value":"0"}},"is_estimated_price":false,"purpose":"-",
            "broker_detail":{"code":"","group":"BROKER_GROUP_LOCAL"}}]}}"#;
        let (rows, _) = parse_page(body).unwrap();
        let r = &rows[0];
        assert_eq!(r.symbol.as_str(), "ABCD");
        assert_eq!(r.action.as_deref(), Some("bonus_stock"));
        assert_eq!(r.source.as_deref(), Some("new"));
        assert_eq!(r.roles, ["pengendali"]);
        assert_eq!(r.foreign, None);
        assert_eq!((r.price, r.value, r.purpose.clone()), (None, None, None));
        assert_eq!(r.broker_group.as_deref(), Some("local"));
    }

    #[test]
    fn empty_and_broken_pages() {
        assert_eq!(
            parse_page(r#"{"data":{"is_more":false,"movement":null}}"#).unwrap(),
            (vec![], false)
        );
        assert_eq!(parse_page(r#"{"data":{}}"#).unwrap(), (vec![], false));
        for bad in [
            "<html>",
            r#"{"message":"Unauthorized"}"#,
            r#"{"data":null}"#,
            r#"{"data":[]}"#,
        ] {
            assert_eq!(parse_page(bad).unwrap_err().exit_code(), 3, "{bad}");
        }
        // Rows without a code, a name or a readable date are skipped.
        let (rows, _) = parse_page(
            r#"{"data":{"movement":[{"name":"A","symbol":"","date":"01 Feb 26"},
                {"name":"","symbol":"BBCA","date":"01 Feb 26"},
                {"name":"A","symbol":"BBCA","date":"2026-02-01"}]}}"#,
        )
        .unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn urls_and_validation() {
        let q = MovementQuery::default();
        assert_eq!(
            url(&q, 1),
            "https://exodus.stockbit.com/insider/company/majorholder?page=1"
        );
        let q = MovementQuery {
            symbol: Some(Symbol::parse("bbca").unwrap()),
            from: Some(d("2026-10-01")),
            to: Some(d("2026-10-31")),
            holder_id: Some("1000004481".into()),
            action: Some(MovementFilter::CorporateAction),
            source: Some(MovementSource::Ksei),
        };
        assert_eq!(
            url(&q, 3),
            "https://exodus.stockbit.com/insider/company/majorholder?page=3&symbol=BBCA&date_start=2026-10-01&date_end=2026-10-31&insider=1000004481&action_type=ACTION_TYPE_CORPACTION&source_type=SOURCE_TYPE_KSEI"
        );
        assert!(super::super::is_read_url(&url(&q, 3)));
        q.validate().unwrap();
        for bad in [
            MovementQuery {
                holder_id: Some("RUDI".into()),
                ..Default::default()
            },
            MovementQuery {
                holder_id: Some("1&page=9".into()),
                ..Default::default()
            },
            MovementQuery {
                from: Some(d("2026-11-01")),
                to: Some(d("2026-10-01")),
                ..Default::default()
            },
            MovementQuery {
                symbol: Some(Symbol::parse("^JKSE").unwrap()),
                ..Default::default()
            },
        ] {
            let e = bad.validate().unwrap_err();
            assert_eq!(e.exit_code(), 2, "{bad:?}");
        }
        let e = MovementQuery {
            holder_id: Some("RUDI".into()),
            ..Default::default()
        }
        .validate()
        .unwrap_err();
        assert!(e.to_string().contains("HOLDER ID"), "{e}");
    }
}
