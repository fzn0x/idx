//! Corporate actions from `GET /corpaction/{type}[?symbol=CODE]`: one
//! request per kind, a stock's whole history with `symbol`, the market's
//! recent and announced actions without it.
//!
//! Row layouts as recorded by 0xrsydn/idx-cli (MIT) on 2026-11-03, copied in
//! `tests/fixtures/stockbit/corpaction_*.json`:
//!
//! - every answer is `{"data":{"<list key>":[rows], ["today":"YYYY-MM-DD"]},"message":..}`;
//!   the list key is the path type except `stock_reverse` (reverse split) and
//!   `tender` (tender offer), so the parser takes the first array it finds;
//! - entitlement rows (dividend, stock dividend, bonus, rights, splits) name
//!   their fields `<prefix>_cumdate`, `_exdate`, `_recdate`, `_paydate` or
//!   `_paymentdate`, `_old`, `_new`, `_ratio` (`"6 : 4"`), `_factor`; bonus rows
//!   reuse the `stocksplit_*` names. They are read by suffix (the shortest key
//!   wins, so `rightissue_factor` beats `rightissue_adj_factor`), which also
//!   covers the stock dividend and reverse split layouts no recording shows;
//! - warrants (`wrant_*`), RUPS (`rups_*`), public exposes (`puexp_*`), IPOs
//!   (`ipo_listing_date`, `ipo_price.final`, `ipo_data_detail.*`) and tender
//!   offers (`tender_*`) by name;
//! - placeholders mean "nothing": `""`, `"-"`, `0`, `"0"`, `0001-01-01 00:00:00`.

use chrono::{Datelike, NaiveDate};
use serde_json::{Map, Value};

use crate::error::{IdxError, Result};
use crate::model::{CorporateEvent, CorporateEventKind, Symbol};

use super::{EXODUS, NAME};

/// Stockbit's path segment for a kind.
fn path_type(kind: CorporateEventKind) -> &'static str {
    match kind {
        CorporateEventKind::Dividend => "dividend",
        CorporateEventKind::StockDividend => "stock_dividend",
        CorporateEventKind::Bonus => "bonus",
        CorporateEventKind::Rights => "rightissue",
        CorporateEventKind::Warrant => "warrant",
        CorporateEventKind::Split => "stocksplit",
        CorporateEventKind::ReverseSplit => "reversesplit",
        CorporateEventKind::Rups => "rups",
        CorporateEventKind::PublicExpose => "pubex",
        CorporateEventKind::Ipo => "ipo",
        CorporateEventKind::TenderOffer => "tenderoffer",
    }
}

/// The row id's key, where a recording shows one.
fn id_key(kind: CorporateEventKind) -> Option<&'static str> {
    Some(match kind {
        CorporateEventKind::Dividend => "dividend_id",
        CorporateEventKind::Bonus => "sahambonus_id",
        CorporateEventKind::Rights => "rightissue_id",
        CorporateEventKind::Warrant => "wrant_id",
        CorporateEventKind::Split => "stocksplit_id",
        CorporateEventKind::Rups => "rups_id",
        CorporateEventKind::PublicExpose => "puexp_id",
        CorporateEventKind::Ipo => "ipo_id",
        CorporateEventKind::TenderOffer => "tender_id",
        CorporateEventKind::StockDividend | CorporateEventKind::ReverseSplit => return None,
    })
}

/// The request URL for one kind; `code` is Stockbit's code for one stock.
pub fn url(kind: CorporateEventKind, code: Option<&str>) -> String {
    let mut u = format!("{EXODUS}/corpaction/{}", path_type(kind));
    if let Some(c) = code {
        u.push_str("?symbol=");
        u.push_str(c);
    }
    u
}

/// The rows of one answer. `data: null` or a list key holding `null` is an
/// empty list; no `data`, or a `data` holding objects but no list, is a
/// provider error.
pub fn parse(kind: CorporateEventKind, body: &str) -> Result<Vec<CorporateEvent>> {
    let v: Value = serde_json::from_str(body).map_err(|e| {
        IdxError::provider(NAME, format!("corporate actions response not JSON: {e}"))
    })?;
    let data = v
        .get("data")
        .ok_or_else(|| IdxError::provider(NAME, "corporate actions response had no data"))?;
    let rows: &[Value] = match data {
        Value::Null => &[],
        Value::Array(a) => a,
        Value::Object(m) => match m.values().find_map(Value::as_array) {
            Some(a) => a,
            None if m.values().any(Value::is_object) => {
                return Err(IdxError::provider(
                    NAME,
                    "corporate actions response had an unexpected layout",
                ))
            }
            None => &[],
        },
        _ => {
            return Err(IdxError::provider(
                NAME,
                "corporate actions response had an unexpected layout",
            ))
        }
    };
    Ok(rows
        .iter()
        .filter_map(Value::as_object)
        .filter_map(|r| row(kind, r))
        .collect())
}

fn row(kind: CorporateEventKind, r: &Map<String, Value>) -> Option<CorporateEvent> {
    use CorporateEventKind as K;
    let symbol = text(r.get("company_symbol")).and_then(|s| Symbol::parse(&s).ok())?;
    let field = |k: &str| r.get(k);
    let mut e = CorporateEvent::new(symbol, kind, NaiveDate::MIN);
    e.name = text(field("company_name"));
    e.note = text(field("event_note"));
    e.id = id_key(kind).and_then(|k| text(field(k)));
    let date = match kind {
        K::Dividend | K::StockDividend | K::Bonus | K::Rights | K::Split | K::ReverseSplit => {
            let s = |suffix: &str| by_suffix(r, suffix);
            e.cum_date = date(s("_cumdate"));
            e.ex_date = date(s("_exdate"));
            e.recording_date = date(s("_recdate"));
            e.distribution_date = date(s("_paydate")).or_else(|| date(s("_paymentdate")));
            e.ratio_old = number(s("_old"));
            e.ratio_new = number(s("_new"));
            if e.ratio_old.is_none() && e.ratio_new.is_none() {
                if let Some((old, new)) = text(s("_ratio")).and_then(|t| ratio(&t)) {
                    e.ratio_old = Some(old);
                    e.ratio_new = Some(new);
                }
            }
            e.factor = number(s("_factor"));
            if matches!(kind, K::Dividend | K::StockDividend) {
                e.currency = text(s("_currency")).map(|c| {
                    c.strip_prefix("CURRENCY_")
                        .unwrap_or(&c)
                        .to_ascii_uppercase()
                });
            }
            if kind == K::Dividend {
                e.dividend_per_share = number(s("_value"));
            }
            if kind == K::Rights {
                e.price = number(s("_price"));
                e.trading_start = date(s("_trading_start"));
                e.trading_end = date(s("_trading_end"));
            }
            e.cum_date.or(e.ex_date)
        }
        K::Warrant => {
            e.trading_start = date(field("wrant_trading_from"));
            e.trading_end = date(field("wrant_trading_end"));
            e.exercise_start = date(field("wrant_exc_from"));
            e.exercise_end = date(field("wrant_exc_end"));
            e.price = number(field("wrant_exc_price"));
            e.trading_start.or(e.exercise_start)
        }
        K::Rups => {
            e.time = text(field("rups_time"));
            e.venue = venue(field("rups_venue"));
            e.recording_date = date(field("rups_eligible_date"));
            date(field("rups_date"))
        }
        K::PublicExpose => {
            e.time = text(field("puexp_time"));
            e.venue = venue(field("puexp_venue"));
            date(field("puexp_date"))
        }
        K::Ipo => {
            e.listing_date = date(field("ipo_listing_date"));
            e.price = number(field("ipo_price").and_then(|p| p.get("final")));
            let detail = field("ipo_data_detail");
            let d = |k: &str| detail.and_then(|x| x.get(k));
            e.shares = number(d("shares"));
            e.shares_pct = number(d("percentage"));
            e.offer_start = date(d("offering_start"));
            e.offer_end = date(d("offering_end"));
            e.listing_date
        }
        K::TenderOffer => {
            e.offer_start = date(field("tender_start"));
            e.offer_end = date(field("tender_end"));
            e.distribution_date = date(field("tender_paydate"));
            e.price = number(field("tender_price"));
            e.shares = number(field("tender_shares"));
            e.shares_pct = number(field("tender_percentage"));
            e.offer_start.or(e.offer_end)
        }
    }?;
    e.date = date;
    Some(e)
}

/// The value under the shortest key ending in `suffix`.
fn by_suffix<'a>(r: &'a Map<String, Value>, suffix: &str) -> Option<&'a Value> {
    r.iter()
        .filter(|(k, _)| k.ends_with(suffix) && k.len() > suffix.len())
        .min_by_key(|(k, _)| k.len())
        .map(|(_, v)| v)
}

/// A non-empty string, not a placeholder (`-`, `0`).
fn text(v: Option<&Value>) -> Option<String> {
    let s = match v? {
        Value::String(s) => s.trim().to_string(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!s.is_empty() && s != "-" && s != "0").then_some(s)
}

/// Whitespace (newlines, tabs) collapsed to single spaces.
fn venue(v: Option<&Value>) -> Option<String> {
    text(v).map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// A positive finite number; `0`, `""` and `-` are nothing. Strings may carry
/// `Rp`, thousands commas and spaces.
fn number(v: Option<&Value>) -> Option<f64> {
    let n = match v? {
        Value::String(s) => {
            let t = s.trim();
            let t = t
                .strip_prefix("Rp")
                .or_else(|| t.strip_prefix("rp"))
                .unwrap_or(t)
                .replace(' ', "");
            super::parse::num(&Value::String(t))?
        }
        other => super::parse::num(other)?,
    };
    (n.is_finite() && n > 0.0).then_some(n)
}

/// `YYYY-MM-DD` from the first ten characters; years before 1900 are the
/// source's empty date (`0001-01-01 00:00:00`).
fn date(v: Option<&Value>) -> Option<NaiveDate> {
    let s = v?.as_str()?.trim();
    let d = NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()?;
    (d.year() >= 1900).then_some(d)
}

/// `"6 : 4"` into (6, 4).
fn ratio(s: &str) -> Option<(f64, f64)> {
    let (a, b) = s.split_once(':')?;
    let num = |x: &str| {
        x.trim()
            .replace(',', "")
            .parse::<f64>()
            .ok()
            .filter(|n| *n > 0.0)
    };
    Some((num(a)?, num(b)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use CorporateEventKind as K;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/stockbit/corpaction_{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn find<'a>(rows: &'a [CorporateEvent], sym: &str) -> &'a CorporateEvent {
        rows.iter()
            .find(|r| r.symbol.as_str() == sym)
            .unwrap_or_else(|| panic!("no {sym} row"))
    }

    #[test]
    fn urls_per_kind() {
        assert_eq!(
            url(K::Rights, Some("BBCA")),
            "https://exodus.stockbit.com/corpaction/rightissue?symbol=BBCA"
        );
        assert_eq!(
            url(K::TenderOffer, None),
            "https://exodus.stockbit.com/corpaction/tenderoffer"
        );
        let types: Vec<&str> = K::ALL.iter().map(|k| path_type(*k)).collect();
        assert_eq!(
            types,
            [
                "dividend",
                "stock_dividend",
                "bonus",
                "rightissue",
                "warrant",
                "stocksplit",
                "reversesplit",
                "rups",
                "pubex",
                "ipo",
                "tenderoffer"
            ]
        );
        for k in K::ALL {
            assert!(super::super::is_read_url(&url(k, Some("BBCA"))), "{k}");
        }
    }

    #[test]
    fn a_stocks_cash_dividends_back_to_2009() {
        let rows = parse(K::Dividend, &fixture("dividend_bbca")).unwrap();
        assert_eq!(rows.len(), 37);
        let first = &rows[0];
        assert_eq!(first.symbol.as_str(), "BBCA");
        assert_eq!(first.kind, K::Dividend);
        assert_eq!(first.date, d("2026-08-28"));
        assert_eq!(
            (
                first.cum_date,
                first.ex_date,
                first.recording_date,
                first.distribution_date
            ),
            (
                Some(d("2026-08-28")),
                Some(d("2026-08-31")),
                Some(d("2026-09-01")),
                Some(d("2026-09-16"))
            )
        );
        assert_eq!(first.dividend_per_share, Some(25.0));
        assert_eq!(first.currency.as_deref(), Some("IDR"));
        assert_eq!(first.id.as_deref(), Some("118073"));
        assert_eq!(
            (first.ratio_old, first.factor, first.price),
            (None, None, None)
        );
        assert_eq!(first.note, None);
        let last = rows.last().unwrap();
        assert_eq!(last.cum_date, Some(d("2009-06-09")));
        assert_eq!(last.dividend_per_share, Some(65.0));
    }

    #[test]
    fn market_dividends_keep_decimals() {
        let rows = parse(K::Dividend, &fixture("dividend")).unwrap();
        assert_eq!(rows.len(), 21);
        let midi = find(&rows, "MIDI");
        assert_eq!(midi.dividend_per_share, Some(3.8));
        assert_eq!(midi.cum_date, Some(d("2026-11-11")));
        assert_eq!(find(&rows, "ITMG").dividend_per_share, Some(1191.09));
    }

    #[test]
    fn bonus_rows_reuse_the_split_fields() {
        let rows = parse(K::Bonus, &fixture("bonus")).unwrap();
        let ppgl = find(&rows, "PPGL");
        assert_eq!((ppgl.ratio_old, ppgl.ratio_new), (Some(6.0), Some(4.0)));
        assert_eq!(ppgl.factor, Some(1.6667));
        assert_eq!(ppgl.date, d("2026-11-02"));
        assert_eq!(ppgl.ex_date, Some(d("2026-11-03")));
        assert_eq!(ppgl.distribution_date, Some(d("2026-11-25")));
        assert_eq!(ppgl.id.as_deref(), Some("7732"));
        // sahambonus_new_price is 0: not a price.
        assert_eq!(ppgl.price, None);
    }

    #[test]
    fn rights_take_the_plain_factor_and_the_trading_window() {
        let rows = parse(K::Rights, &fixture("rightissue")).unwrap();
        let bmas = find(&rows, "BMAS");
        assert_eq!((bmas.ratio_old, bmas.ratio_new), (Some(655.0), Some(104.0)));
        assert_eq!(bmas.price, Some(350.0));
        assert_eq!(bmas.factor, Some(1.15878));
        assert_eq!(
            (bmas.trading_start, bmas.trading_end),
            (Some(d("2026-11-27")), Some(d("2026-12-03")))
        );
        assert_eq!(bmas.date, d("2026-11-23"));
        assert_eq!(find(&rows, "MGLV").price, Some(8880.0));
    }

    #[test]
    fn splits_of_one_stock() {
        let rows = parse(K::Split, &fixture("stocksplit_bbca")).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].date, d("2021-10-12"));
        assert_eq!(
            (rows[0].ratio_old, rows[0].ratio_new, rows[0].factor),
            (Some(1.0), Some(5.0), Some(5.0))
        );
        assert_eq!(rows[2].date, d("2004-06-07"));
        assert_eq!(
            (rows[2].ratio_old, rows[2].ratio_new),
            (Some(1.0), Some(2.0))
        );
        assert!(!parse(K::Split, &fixture("stocksplit")).unwrap().is_empty());
    }

    #[test]
    fn warrants_company_and_structured() {
        let rows = parse(K::Warrant, &fixture("warrant")).unwrap();
        assert_eq!(rows.len(), 36);
        let brna = find(&rows, "BRNA");
        assert_eq!(brna.date, d("2026-10-16"));
        assert_eq!(brna.price, Some(800.0));
        assert_eq!(
            (brna.exercise_start, brna.exercise_end),
            (Some(d("2027-04-16")), Some(d("2029-10-15")))
        );
        assert_eq!(brna.trading_end, Some(d("2029-10-10")));
        assert_eq!(brna.id.as_deref(), Some("3570"));
        // A structured warrant under its own code.
        assert_eq!(find(&rows, "ASIIZPCN7A").kind, K::Warrant);
    }

    #[test]
    fn meetings_and_exposes() {
        let rows = parse(K::Rups, &fixture("rups")).unwrap();
        assert_eq!(rows.len(), 47);
        let arti = find(&rows, "ARTI");
        assert_eq!(arti.date, d("2026-12-09"));
        assert_eq!(arti.time.as_deref(), Some("10:30"));
        assert_eq!(arti.recording_date, Some(d("2026-11-12")));
        assert_eq!(arti.name, None, "company_name is empty");
        let bbca = parse(K::Rups, &fixture("rups_bbca")).unwrap();
        assert_eq!(bbca.len(), 8);
        assert!(bbca.iter().all(|r| r
            .venue
            .as_deref()
            .is_some_and(|v| !v.contains(['\n', '\t', '\r'])
                && !v.contains("  ")
                && v == v.trim())));
        let pubex = parse(K::PublicExpose, &fixture("pubex")).unwrap();
        let meja = find(&pubex, "MEJA");
        assert_eq!(meja.time.as_deref(), Some("13:00:00"));
        assert_eq!(meja.date, d("2026-11-24"));
        let bbca = parse(K::PublicExpose, &fixture("pubex_bbca")).unwrap();
        assert_eq!(bbca.len(), 6);
        assert_eq!(bbca[5].venue.as_deref(), Some("Public Expose Live 2021"));
    }

    #[test]
    fn ipos_with_and_without_terms() {
        let rows = parse(K::Ipo, &fixture("ipo")).unwrap();
        assert_eq!(rows.len(), 15);
        let rans = find(&rows, "RANS");
        assert_eq!(rans.price, Some(170.0));
        assert_eq!(rans.shares, Some(2_525_000_000.0));
        assert_eq!(rans.shares_pct, Some(20.02));
        assert_eq!(
            (rans.offer_start, rans.offer_end),
            (Some(d("2026-07-02")), Some(d("2026-07-08")))
        );
        assert_eq!(rans.listing_date, Some(d("2026-07-10")));
        assert_eq!(rans.date, d("2026-07-10"));
        assert!(rans.name.is_some());
        // An ETF listing: every term is a placeholder.
        let xkgs = find(&rows, "XKGS");
        assert_eq!(
            (
                xkgs.price,
                xkgs.shares,
                xkgs.shares_pct,
                xkgs.offer_start,
                xkgs.offer_end
            ),
            (None, None, None, None, None)
        );
        assert_eq!(xkgs.date, d("2026-09-26"));
    }

    #[test]
    fn tender_offers_under_their_own_list_key() {
        let rows = parse(K::TenderOffer, &fixture("tenderoffer")).unwrap();
        assert_eq!(rows.len(), 6);
        let auto = find(&rows, "AUTO");
        assert_eq!(auto.price, Some(3600.0));
        assert_eq!(auto.shares, Some(238_442_293.0));
        assert_eq!(auto.shares_pct, Some(4.95));
        assert_eq!(
            (auto.offer_start, auto.offer_end, auto.distribution_date),
            (
                Some(d("2026-10-13")),
                Some(d("2026-11-11")),
                Some(d("2026-11-18"))
            )
        );
        assert_eq!(auto.date, d("2026-10-13"));
        assert_eq!(auto.name.as_deref(), Some("Astra Otoparts Tbk."));
        assert_eq!(auto.id.as_deref(), Some("434"));
    }

    #[test]
    fn empty_lists_and_odd_layouts() {
        assert!(parse(K::StockDividend, &fixture("stock_dividend"))
            .unwrap()
            .is_empty());
        assert!(parse(K::ReverseSplit, &fixture("reversesplit"))
            .unwrap()
            .is_empty());
        assert!(parse(K::Dividend, r#"{"data":null}"#).unwrap().is_empty());
        assert!(parse(
            K::Dividend,
            r#"{"data":{"dividend":null,"today":"2026-11-03"}}"#
        )
        .unwrap()
        .is_empty());
        for bad in [
            r#"{"message":"x"}"#,
            r#"{"data":{"dividend":{"rows":[]}}}"#,
            r#"{"data":"x"}"#,
            "<html>",
        ] {
            let e = parse(K::Dividend, bad).unwrap_err();
            assert_eq!(e.exit_code(), 3, "{bad}");
        }
        // Rows without a code or a date are dropped.
        let rows = parse(
            K::Dividend,
            r#"{"data":{"dividend":[{"company_symbol":"","dividend_cumdate":"2026-01-02"},
                {"company_symbol":"BBCA","dividend_cumdate":"","dividend_exdate":""},
                {"company_symbol":"BBCA","dividend_exdate":"2026-01-05"}]}}"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].date,
            d("2026-01-05"),
            "the ex date without a cum date"
        );
    }

    #[test]
    fn unrecorded_layouts_by_suffix() {
        // Reverse split and stock dividend were empty on the recording day;
        // their fields are read by suffix.
        let rows = parse(
            K::ReverseSplit,
            r#"{"data":{"stock_reverse":[{"company_symbol":"ABCD","reverse_cumdate":"2026-12-01",
                "reverse_exdate":"2026-12-02","reverse_old":"10","reverse_new":"1",
                "reverse_factor":"0.1","reverse_new_price":0}]}}"#,
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!((r.kind, r.date), (K::ReverseSplit, d("2026-12-01")));
        assert_eq!(
            (r.ratio_old, r.ratio_new, r.factor),
            (Some(10.0), Some(1.0), Some(0.1))
        );
        assert_eq!((r.price, r.id.as_deref()), (None, None));
        let rows = parse(
            K::StockDividend,
            r#"{"data":{"stock_dividend":[{"company_symbol":"ABCD","stockdiv_cumdate":"2026-12-01",
                "stockdiv_ratio":"20 : 1","stockdiv_currency":"CURRENCY_IDR"}]}}"#,
        )
        .unwrap();
        assert_eq!(
            (rows[0].ratio_old, rows[0].ratio_new),
            (Some(20.0), Some(1.0))
        );
        assert_eq!(rows[0].currency.as_deref(), Some("IDR"));
        assert_eq!(rows[0].dividend_per_share, None);
    }

    #[test]
    fn a_usd_dividend_keeps_its_decimals_and_currency() {
        let rows = parse(
            K::Dividend,
            r#"{"data":{"dividend":[{"company_symbol":"ABCD","dividend_cumdate":"2026-12-01",
                "dividend_value":"0.0012","dividend_currency":"CURRENCY_USD",
                "dividend_value_formatted":"$ 0.0012","event_note":" interim "}]}}"#,
        )
        .unwrap();
        assert_eq!(rows[0].dividend_per_share, Some(0.0012));
        assert_eq!(rows[0].currency.as_deref(), Some("USD"));
        assert_eq!(rows[0].note.as_deref(), Some("interim"));
        assert_eq!(
            number(Some(&Value::String("Rp 1,191.09".into()))),
            Some(1191.09)
        );
        assert_eq!(number(Some(&Value::String("0".into()))), None);
        assert_eq!(number(Some(&serde_json::json!(0))), None);
        assert_eq!(
            date(Some(&Value::String("0001-01-01 00:00:00".into()))),
            None
        );
    }
}
