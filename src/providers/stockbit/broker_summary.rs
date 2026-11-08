//! Broker summary from `GET /marketdetectors/{SYM}`: every broker that
//! bought a stock and every broker that sold it over a date range, with
//! lots, value and average price.
//!
//! Recorded 2026-11-08 (`tests/fixtures/stockbit/marketdetectors_*.json`).
//! Query: `transaction_type` (`TRANSACTION_TYPE_GROSS` | `_NET`),
//! `market_board` (`MARKET_BOARD_ALL` | `_REGULER`), `investor_type`
//! (`INVESTOR_TYPE_ALL` | `_FOREIGN`), `from` and `to` (YYYY-MM-DD, one
//! aggregate over the range), `limit` (rows per side; 1000 was accepted and a
//! year of BBCA, 92 brokers a side, came back whole).
//! Reply: `data.broker_summary.brokers_buy[]` / `brokers_sell[]`, rows of
//! `netbs_broker_code`, `type` (`Asing`, `Lokal`, `Pemerintah`), `freq`,
//! `blot` / `slot` (lots), `blotv` / `slotv` (shares), `bval` / `sval` and
//! `bvalv` / `svalv` (IDR), `netbs_buy_avg_price` / `netbs_sell_avg_price`;
//! figures are strings (`"3.524875059e+11"`), or JSON numbers in an empty
//! reply. `data.bandar_detector.total_buyer` / `total_seller` count the
//! brokers on each side whatever `limit` cut; `data.from` / `data.to` echo the
//! range. An unknown code is HTTP 404 `error_type: NOT_FOUND`; a day without
//! trades is HTTP 200 with empty lists.
//!
//! idx asks for gross figures only: in gross mode each side lists every
//! broker's own buying or selling (`blot` = `blotv / 100`, `bval` = `bvalv`),
//! which is what `idx.broker.v1` means. Net mode lists each broker once, on
//! the side where it was net, with negative sells; a reply in those figures
//! is refused. Stockbit ranks a side by value; idx orders it by lots (ties
//! in Stockbit's order) so `rank` keeps its meaning. Totals are sums over the
//! full lists, so they are exact and agree with the rows; Stockbit's own
//! totals (`bandar_detector.value`, `volume`, `average`) are the buy side's,
//! at about eight significant digits, and are not used.
//!
//! The domestic investor type and the cash and negotiated boards have not
//! been recorded, so [`serves`] leaves them to IPOT.

use chrono::NaiveDate;
use serde_json::Value;

use crate::error::{IdxError, Result};
use crate::model::{
    BrokerBoard, BrokerFlow, BrokerInvestor, BrokerKind, BrokerSummary, BrokerSummarySource, Symbol,
};

use super::parse::num;
use super::{EXODUS, NAME};

/// Rows asked for per side: more than IDX has members, and a limit
/// Stockbit was seen to accept.
pub const LIMIT: u32 = 1000;

/// `true` for the filters whose replies have been recorded: all or foreign
/// investors, all boards or the regular board.
pub fn serves(investor: BrokerInvestor, board: BrokerBoard) -> bool {
    investor != BrokerInvestor::Domestic && matches!(board, BrokerBoard::All | BrokerBoard::Regular)
}

/// The URL for `code` (Stockbit's code) over `from..=to`, gross figures,
/// parameters in a fixed order so it is a stable cache key with no secret.
/// Only for filters [`serves`] accepts.
pub fn url(
    code: &str,
    from: NaiveDate,
    to: NaiveDate,
    investor: BrokerInvestor,
    board: BrokerBoard,
) -> String {
    let board = match board {
        BrokerBoard::Regular => "MARKET_BOARD_REGULER",
        _ => "MARKET_BOARD_ALL",
    };
    let investor = match investor {
        BrokerInvestor::Foreign => "INVESTOR_TYPE_FOREIGN",
        _ => "INVESTOR_TYPE_ALL",
    };
    format!(
        "{EXODUS}/marketdetectors/{code}?transaction_type=TRANSACTION_TYPE_GROSS\
         &market_board={board}&investor_type={investor}&from={from}&to={to}&limit={LIMIT}"
    )
}

/// One side of the reply: rows ordered by lots, the count Stockbit gives and
/// whether every broker is listed.
struct Side {
    rows: Vec<BrokerFlow>,
    count: Option<u32>,
    complete: bool,
}

impl Side {
    /// The side's value and lots, summed over the full list; `None` when
    /// the list is cut or a row lacks the figure.
    fn sums(&self) -> (Option<f64>, Option<f64>) {
        if !self.complete {
            return (None, None);
        }
        let sum = |f: fn(&BrokerFlow) -> Option<f64>| -> Option<f64> {
            self.rows.iter().map(f).sum::<Option<f64>>()
        };
        // Lots are hundredths at most (shares / 100); drop the float noise.
        let lots = sum(|r| r.lots).map(|l| (l * 100.0).round() / 100.0);
        (sum(|r| r.value), lots)
    }
}

fn figure(r: &Value, key: &str) -> Option<f64> {
    r.get(key).and_then(num).filter(|v| v.is_finite())
}

fn kind(r: &Value) -> Option<BrokerKind> {
    match r.get("type").and_then(Value::as_str).map(str::trim) {
        Some("Asing") => Some(BrokerKind::Foreign),
        Some("Lokal") => Some(BrokerKind::Local),
        Some("Pemerintah") => Some(BrokerKind::StateOwned),
        _ => None,
    }
}

/// `true` when `a` and `b` are the same figure up to the reply's float
/// formatting: gross rows carry it twice (lots and shares, value twice).
fn same(a: f64, b: f64) -> bool {
    (a - b).abs() <= 0.5_f64.max(b.abs() * 1e-9)
}

/// `brokers_buy` (prefix `b`) or `brokers_sell` (`s`). A row without a code
/// is left out, which leaves the side incomplete against its count.
fn side(data: &Value, list: &str, p: &str, avg: &str, total: &str) -> Result<Side> {
    let raw = data
        .get("broker_summary")
        .and_then(|b| b.get(list))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut rows = Vec::with_capacity(raw.len());
    for r in raw {
        let Some(code) = r
            .get("netbs_broker_code")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|c| !c.is_empty())
        else {
            continue;
        };
        let lots = figure(r, &format!("{p}lot"));
        let value = figure(r, &format!("{p}val"));
        let gross = lots.is_none_or(|l| l >= 0.0)
            && value.is_none_or(|v| v >= 0.0)
            && match (lots, figure(r, &format!("{p}lotv"))) {
                (Some(l), Some(shares)) => same(l * 100.0, shares),
                _ => true,
            }
            && match (value, figure(r, &format!("{p}valv"))) {
                (Some(v), Some(gross)) => same(v, gross),
                _ => true,
            };
        if !gross {
            return Err(IdxError::provider(
                NAME,
                format!(
                    "broker summary: {code}'s row in {list} is not in gross figures (lots {}, value {}); idx asks for gross figures only",
                    lots.map_or("?".into(), |l| l.to_string()),
                    value.map_or("?".into(), |v| v.to_string()),
                ),
            ));
        }
        rows.push(BrokerFlow {
            rank: 0,
            code: code.to_string(),
            name: None,
            kind: kind(r),
            lots,
            value,
            average_price: figure(r, avg),
        });
    }
    // Stockbit ranks by value; rank 1 is the most lots. The sort is stable,
    // so ties keep Stockbit's order.
    rows.sort_by(|a, b| {
        b.lots
            .unwrap_or(f64::NEG_INFINITY)
            .total_cmp(&a.lots.unwrap_or(f64::NEG_INFINITY))
    });
    for (i, r) in rows.iter_mut().enumerate() {
        r.rank = i as u32 + 1;
    }
    let count = data
        .get("bandar_detector")
        .and_then(|b| b.get(total))
        .and_then(num)
        .filter(|n| *n >= 0.0 && n.fract() == 0.0)
        .map(|n| n as u32);
    let complete = count.is_some_and(|n| rows.len() >= n as usize);
    Ok(Side {
        rows,
        count,
        complete,
    })
}

fn date(data: &Value, key: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(data.get(key)?.as_str()?, "%Y-%m-%d").ok()
}

/// One reply as an `idx.broker.v1` summary (names left to the caller).
/// `from` and `to` stand in when the reply does not echo its range. For all
/// investors `foreign_net_value` is `null` (0 without trades): a reply for
/// all investors does not carry it, see `Stockbit::broker_summary`.
pub fn parse(
    symbol: &Symbol,
    from: NaiveDate,
    to: NaiveDate,
    investor: BrokerInvestor,
    board: BrokerBoard,
    body: &str,
) -> Result<BrokerSummary> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| IdxError::provider(NAME, format!("broker summary response not JSON: {e}")))?;
    let data = v
        .get("data")
        .filter(|d| d.get("broker_summary").is_some_and(Value::is_object))
        .ok_or_else(|| IdxError::provider(NAME, "broker summary response had no broker_summary"))?;
    let buy = side(
        data,
        "brokers_buy",
        "b",
        "netbs_buy_avg_price",
        "total_buyer",
    )?;
    let sell = side(
        data,
        "brokers_sell",
        "s",
        "netbs_sell_avg_price",
        "total_seller",
    )?;
    let (buy_value, buy_lots) = buy.sums();
    let (sell_value, _) = sell.sums();
    let empty = buy.rows.is_empty() && sell.rows.is_empty() && buy.complete && sell.complete;
    let all = investor == BrokerInvestor::All;
    let less = |a: Option<f64>, b: Option<f64>| Some(a? - b?);
    let foreign_net_value = match investor {
        BrokerInvestor::All => empty.then_some(0.0),
        // What foreign investors bought less what they sold; domestic
        // investors' own net is the opposite of the foreign net.
        BrokerInvestor::Foreign => less(buy_value, sell_value),
        BrokerInvestor::Domestic => less(sell_value, buy_value),
    };
    Ok(BrokerSummary {
        symbol: symbol.clone(),
        name: None,
        from: date(data, "from").unwrap_or(from),
        to: date(data, "to").unwrap_or(to),
        investor,
        board,
        total_value: buy_value.filter(|_| all),
        total_lots: buy_lots.filter(|_| all),
        foreign_net_value,
        average_price: match (buy_value, buy_lots) {
            (Some(v), Some(l)) if all && l > 0.0 => Some(v / (l * 100.0)),
            _ => None,
        },
        buy_value,
        sell_value,
        rounded: false,
        buyer_count: buy.count,
        seller_count: sell.count,
        buyers_complete: buy.complete,
        sellers_complete: sell.complete,
        buyers: buy.rows,
        sellers: sell.rows,
        source: BrokerSummarySource::Stockbit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: &str =
        include_str!("../../../tests/fixtures/stockbit/marketdetectors_bbca_20261106.json");
    const NET: &str =
        include_str!("../../../tests/fixtures/stockbit/marketdetectors_bbca_20261106_net.json");
    const WEEK: &str = include_str!(
        "../../../tests/fixtures/stockbit/marketdetectors_bbca_20261102_20261106.json"
    );
    const YEAR: &str = include_str!(
        "../../../tests/fixtures/stockbit/marketdetectors_bbca_20251106_20261106.json"
    );
    const REGULAR: &str =
        include_str!("../../../tests/fixtures/stockbit/marketdetectors_bbca_20261106_regular.json");
    const FOREIGN: &str =
        include_str!("../../../tests/fixtures/stockbit/marketdetectors_bbca_20261106_foreign.json");
    const CUT: &str =
        include_str!("../../../tests/fixtures/stockbit/marketdetectors_bbca_20261106_limit5.json");
    const THIN: &str =
        include_str!("../../../tests/fixtures/stockbit/marketdetectors_cbre_20261106.json");
    const SATURDAY: &str = include_str!(
        "../../../tests/fixtures/stockbit/marketdetectors_bbca_20261107_no_trades.json"
    );
    const ZZZZ: &str = include_str!("../../../tests/fixtures/stockbit/marketdetectors_zzzz.json");
    const IPOT_DAY: &str =
        include_str!("../../../tests/fixtures/ipot/brokersummary_bbca_20261106.html");

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn day(body: &str, investor: BrokerInvestor, board: BrokerBoard) -> BrokerSummary {
        let sym = Symbol::parse("BBCA").unwrap();
        parse(&sym, d(2026, 11, 6), d(2026, 11, 6), investor, board, body).unwrap()
    }

    fn all(body: &str) -> BrokerSummary {
        day(body, BrokerInvestor::All, BrokerBoard::All)
    }

    fn codes(rows: &[BrokerFlow]) -> Vec<&str> {
        rows.iter().map(|r| r.code.as_str()).collect()
    }

    #[test]
    fn the_url_asks_for_gross_figures_in_a_fixed_order() {
        let u = url(
            "BBCA",
            d(2026, 11, 2),
            d(2026, 11, 6),
            BrokerInvestor::All,
            BrokerBoard::All,
        );
        assert_eq!(
            u,
            "https://exodus.stockbit.com/marketdetectors/BBCA?transaction_type=TRANSACTION_TYPE_GROSS&market_board=MARKET_BOARD_ALL&investor_type=INVESTOR_TYPE_ALL&from=2026-11-02&to=2026-11-06&limit=1000"
        );
        let u = url(
            "BBCA",
            d(2026, 11, 6),
            d(2026, 11, 6),
            BrokerInvestor::Foreign,
            BrokerBoard::Regular,
        );
        assert!(
            u.contains("&market_board=MARKET_BOARD_REGULER&investor_type=INVESTOR_TYPE_FOREIGN&"),
            "{u}"
        );
        assert!(serves(BrokerInvestor::All, BrokerBoard::All));
        assert!(serves(BrokerInvestor::Foreign, BrokerBoard::Regular));
        assert!(!serves(BrokerInvestor::Domestic, BrokerBoard::All));
        assert!(!serves(BrokerInvestor::All, BrokerBoard::Cash));
        assert!(!serves(BrokerInvestor::All, BrokerBoard::Negotiated));
    }

    #[test]
    fn every_broker_on_each_side_ordered_by_lots() {
        let s = all(DAY);
        assert_eq!(s.source, BrokerSummarySource::Stockbit);
        assert_eq!((s.from, s.to), (d(2026, 11, 6), d(2026, 11, 6)));
        assert_eq!((s.buyers.len(), s.sellers.len()), (59, 67));
        assert_eq!((s.buyer_count, s.seller_count), (Some(59), Some(67)));
        assert!(s.buyers_complete && s.sellers_complete);
        assert!(!s.rounded, "the figures are exact");
        let b = &s.buyers[0];
        assert_eq!((b.rank, b.code.as_str()), (1, "KZ"));
        assert_eq!(b.kind, Some(BrokerKind::Foreign));
        assert_eq!(b.lots, Some(687_161.0));
        assert_eq!(b.value, Some(352_487_505_900.0));
        assert_eq!(b.average_price, Some(5_129.620364077705));
        assert_eq!(b.name, None, "names are the service's job");
        assert_eq!(
            s.buyers[3].lots,
            Some(108_898.5),
            "odd lots count as fractions"
        );
        assert_eq!(s.sellers[0].code, "CC");
        assert_eq!(s.sellers[0].kind, Some(BrokerKind::StateOwned));
        assert_eq!(s.buyers[5].kind, Some(BrokerKind::Local));
        let ranks: Vec<u32> = s.sellers.iter().map(|r| r.rank).collect();
        assert_eq!(ranks, (1..=67).collect::<Vec<u32>>());
        let lots: Vec<f64> = s.buyers.iter().map(|r| r.lots.unwrap()).collect();
        assert!(lots.windows(2).all(|w| w[0] >= w[1]), "{lots:?}");
    }

    #[test]
    fn totals_are_the_sums_of_the_full_lists() {
        let s = all(DAY);
        assert_eq!(s.total_value, Some(1_035_652_066_452.0));
        assert_eq!(s.total_lots, Some(2_027_579.79));
        assert_eq!((s.buy_value, s.sell_value), (s.total_value, s.total_value));
        let avg = s.average_price.unwrap();
        assert!((avg - 5_107.824).abs() < 0.001, "{avg}");
        assert_eq!(s.foreign_net_value, None, "a second reply carries it");
    }

    #[test]
    fn the_first_ten_by_lots_are_ipots_top_ten() {
        use crate::providers::ipot::parse as ipot;
        let ipot::Page::Summary(p) = ipot::parse(IPOT_DAY).unwrap() else {
            panic!("IPOT page")
        };
        let s = all(DAY);
        for (sb, ip) in [(&s.buyers, &p.buyers), (&s.sellers, &p.sellers)] {
            assert_eq!(codes(&sb[..10]), codes(ip));
            for (a, b) in sb.iter().zip(ip.iter()) {
                assert_eq!(a.rank, b.rank);
                assert_eq!(a.lots.unwrap().round(), b.lots.unwrap(), "{}", a.code);
                let (v, w) = (a.value.unwrap(), b.value.unwrap());
                assert!((v - w).abs() <= 0.05e9, "{}: {v} {w}", a.code);
                let (x, y) = (a.average_price.unwrap(), b.average_price.unwrap());
                assert!((x - y).abs() <= 0.5, "{}: {x} {y}", a.code);
            }
        }
    }

    #[test]
    fn a_range_is_one_aggregate_and_lots_not_value_decide_the_order() {
        let sym = Symbol::parse("BBCA").unwrap();
        let s = parse(
            &sym,
            d(2026, 11, 2),
            d(2026, 11, 6),
            BrokerInvestor::All,
            BrokerBoard::All,
            WEEK,
        )
        .unwrap();
        assert_eq!((s.from, s.to), (d(2026, 11, 2), d(2026, 11, 6)));
        assert_eq!((s.buyer_count, s.seller_count), (Some(81), Some(78)));
        assert_eq!(s.total_value, Some(5_999_367_547_273.0));
        // Stockbit lists KZ (857.7 B) first; ZP traded less value but more lots.
        assert_eq!(codes(&s.buyers[..2]), ["ZP", "KZ"]);
        assert!(s.buyers[1].value > s.buyers[0].value);
        assert_eq!(
            codes(&s.sellers[..10]),
            ["CC", "AK", "KZ", "BK", "ZP", "YU", "XL", "SQ", "YP", "XC"]
        );
        let y = parse(
            &sym,
            d(2025, 11, 6),
            d(2026, 11, 6),
            BrokerInvestor::All,
            BrokerBoard::All,
            YEAR,
        )
        .unwrap();
        assert_eq!((y.buyers.len(), y.sellers.len()), (92, 92));
        assert!(y.buyers_complete && y.sellers_complete);
    }

    #[test]
    fn one_investor_side_is_its_buying_and_selling() {
        let f = day(FOREIGN, BrokerInvestor::Foreign, BrokerBoard::All);
        assert_eq!((f.buyer_count, f.seller_count), (Some(23), Some(22)));
        assert_eq!(
            (f.total_value, f.total_lots, f.average_price),
            (None, None, None)
        );
        assert_eq!(f.buy_value, Some(786_531_414_962.0));
        assert_eq!(f.sell_value, Some(567_543_113_862.0));
        // IPOT's F. NVal for the day: 219.0 B.
        assert_eq!(f.foreign_net_value, Some(218_988_301_100.0));
    }

    #[test]
    fn the_regular_board_alone() {
        let r = day(REGULAR, BrokerInvestor::All, BrokerBoard::Regular);
        assert_eq!(r.board, BrokerBoard::Regular);
        assert_eq!(r.total_value, Some(817_283_116_500.0));
        assert_eq!(r.total_lots, Some(1_601_528.0));
        assert_eq!(codes(&r.buyers[..2]), ["AK", "KZ"]);
    }

    #[test]
    fn a_cut_list_is_not_complete_and_has_no_totals() {
        let s = all(CUT);
        assert_eq!((s.buyers.len(), s.sellers.len()), (5, 5));
        assert_eq!((s.buyer_count, s.seller_count), (Some(59), Some(67)));
        assert!(!s.buyers_complete && !s.sellers_complete);
        assert_eq!(
            (s.total_value, s.total_lots, s.average_price),
            (None, None, None)
        );
        assert_eq!((s.buy_value, s.sell_value), (None, None));
    }

    #[test]
    fn a_thin_day_and_a_day_without_trades() {
        let t = parse(
            &Symbol::parse("CBRE").unwrap(),
            d(2026, 11, 6),
            d(2026, 11, 6),
            BrokerInvestor::All,
            BrokerBoard::All,
            THIN,
        )
        .unwrap();
        assert_eq!((t.buyers.len(), t.sellers.len()), (25, 4));
        assert!(t.buyers_complete && t.sellers_complete);
        assert_eq!(codes(&t.sellers), ["XL", "AZ", "XC", "YP"]);
        assert_eq!(t.total_value, Some(1_311_477_000.0));

        let sym = Symbol::parse("BBCA").unwrap();
        let sat = d(2026, 11, 7);
        let e = parse(
            &sym,
            sat,
            sat,
            BrokerInvestor::All,
            BrokerBoard::All,
            SATURDAY,
        )
        .unwrap();
        assert!(e.is_empty());
        assert_eq!((e.from, e.to), (sat, sat));
        assert_eq!((e.buyer_count, e.seller_count), (Some(0), Some(0)));
        assert!(e.buyers_complete && e.sellers_complete);
        assert_eq!((e.total_value, e.total_lots), (Some(0.0), Some(0.0)));
        assert_eq!(e.average_price, None, "nothing traded");
        assert_eq!(
            e.foreign_net_value,
            Some(0.0),
            "no trades, no foreign trades"
        );
    }

    #[test]
    fn net_figures_and_garbage_are_refused() {
        let e = parse(
            &Symbol::parse("BBCA").unwrap(),
            d(2026, 11, 6),
            d(2026, 11, 6),
            BrokerInvestor::All,
            BrokerBoard::All,
            NET,
        )
        .unwrap_err();
        assert_eq!(e.exit_code(), 3);
        assert!(e.to_string().contains("gross"), "{e}");
        let sym = Symbol::parse("BBCA").unwrap();
        for body in ["<html>", r#"{"message":"ok"}"#, r#"{"data":{}}"#] {
            let e = parse(
                &sym,
                d(2026, 11, 6),
                d(2026, 11, 6),
                BrokerInvestor::All,
                BrokerBoard::All,
                body,
            )
            .unwrap_err();
            assert_eq!(e.exit_code(), 3, "{body}");
        }
        assert!(super::super::parse::is_not_found(ZZZZ));
    }
}
