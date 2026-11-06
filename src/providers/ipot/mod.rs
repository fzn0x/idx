//! Indo Premier Sekuritas (IPOT) broker summary: which brokers bought and
//! sold a ticker.
//!
//! Endpoint verified 2026-10-17 (see `tests/fixtures/ipot/`), no login or key:
//! `https://www.indopremier.com/module/saham/include/data-brokersummary.php`
//! `?code=BBCA&start=MM/DD/YYYY&end=MM/DD/YYYY&fd=all|F|D&board=all|RG|TN|NG`
//! - Answers an HTML fragment with the top 10 buyers and top 10 sellers,
//!   ranked by lots (not value: of two brokers the one with more lots comes
//!   first even when it traded less value), each broker's class, and range
//!   totals.
//! - Figures of a million and more are abbreviated to one decimal (`1.6 M`,
//!   `841.2 B`, `2.2 T`); smaller ones are exact.
//! - A range without trades (weekend, holiday) gives ten blank rows and
//!   zero totals; an unknown code gives `Invalid stock code!` with HTTP 200.
//! - In some stretches of seconds the server answers nginx's own HTTP 404
//!   page to a third or more of requests, then recovers; the same URL then
//!   succeeds. Since a real unknown code is a 200, a 404 is retried up to
//!   [`ATTEMPTS`] times with doubling pauses (under 4 s in all).
//! - Over a multi-day range that starts before the latest session and
//!   contains it, the footer's foreign net (`F. NVal`) counts that session
//!   twice (seen 2026-10-17 and 2026-11-06); value, lots and the broker lists
//!   are plain sums and a single session is always right.
//!   [`broker_summary_range`] rebuilds such a net from single sessions.
//!
//! - With `fd=F` or `fd=D` the footer's `T. Val`, `T.Lot` and `Avg` are that
//!   side's buying; `F. NVal` is the same on every page and ignores `board`
//!   (see `parse::Parsed::into_summary`). [`foreign_day`] reads a session's
//!   foreign buying and selling from the `fd=all` and `fd=F` pages.
//!
//! Broker names come from the bundled IDX member list (`crate::brokers`),
//! filled in by `service::broker`.
//!
//! Corporate actions, verified 2026-10-18, same module and quirks:
//! `.../corporateaction.php?code=BBCA&start=MM/DD/YYYY` answers the
//! "Corporate Action & RUPS" fragment with events from `start` on (upcoming
//! ones included); see `actions.rs`.

use std::sync::Arc;
use std::time::Duration;

use chrono::NaiveDate;

use crate::error::{IdxError, Result};
use crate::holidays;
use crate::http::{Fetched, Fetcher};
use crate::model::{BrokerBoard, BrokerInvestor, BrokerSummary, ForeignValueDay, Symbol};

use super::Sourced;

pub mod actions;
pub mod parse;

pub const NAME: &str = "ipot";
const ENDPOINT: &str = "https://www.indopremier.com/module/saham/include/data-brokersummary.php";
const ACTIONS_ENDPOINT: &str =
    "https://www.indopremier.com/module/saham/include/corporateaction.php";
/// Announcements are added and confirmed through the day.
const TTL_ACTIONS: Duration = Duration::from_secs(6 * 3600);

/// A range that ended before today no longer changes.
const TTL_CLOSED: Duration = Duration::from_secs(7 * 24 * 3600);
/// Today's summary moves with every trade.
const TTL_TODAY: Duration = Duration::from_secs(5 * 60);
/// Tries per request while the source answers its spurious 404.
pub const ATTEMPTS: usize = 5;
const RETRY_PAUSE: Duration = Duration::from_millis(if cfg!(test) { 1 } else { 250 });
/// Before a batch caller (the bandar screen) goes over its failures once
/// more, one at a time: a 404 stretch can outlast the per-request retries.
pub const RECOVER_PAUSE: Duration = Duration::from_millis(if cfg!(test) { 1 } else { 3000 });

pub fn url(
    symbol: &Symbol,
    from: NaiveDate,
    to: NaiveDate,
    investor: BrokerInvestor,
    board: BrokerBoard,
) -> String {
    let fd = match investor {
        BrokerInvestor::All => "all",
        BrokerInvestor::Foreign => "F",
        BrokerInvestor::Domestic => "D",
    };
    let board = match board {
        BrokerBoard::All => "all",
        BrokerBoard::Regular => "RG",
        BrokerBoard::Cash => "TN",
        BrokerBoard::Negotiated => "NG",
    };
    format!(
        "{ENDPOINT}?code={}&start={}&end={}&fd={fd}&board={board}",
        symbol.as_str(),
        from.format("%m/%d/%Y"),
        to.format("%m/%d/%Y"),
    )
}

/// Broker summary for `symbol` over `from..=to` (WIB dates). `today` decides
/// the cache lifetime.
///
/// A range that reaches today holds the session so far (during trading:
/// totals without broker lists, see [`ForeignSession::Pending`]). The cache
/// decides freshness at read time, so such a page is kept under its own key
/// with the short lifetime: on a later day, when the range has closed, the
/// page is read again instead of serving the partial copy for a week.
pub async fn broker_summary(
    fetcher: &Arc<Fetcher>,
    symbol: &Symbol,
    from: NaiveDate,
    to: NaiveDate,
    investor: BrokerInvestor,
    board: BrokerBoard,
    today: NaiveDate,
) -> Result<Sourced<BrokerSummary>> {
    let url = url(symbol, from, to, investor, board);
    let (ttl, key) = if to < today {
        (TTL_CLOSED, None)
    } else {
        (TTL_TODAY, Some(format!("{url}#open")))
    };
    let f = page(fetcher, &url, ttl, key.as_deref(), "broker summary", symbol).await?;
    let p = match parse::parse(&f.body)? {
        parse::Page::Invalid => {
            return Err(IdxError::NotFound(format!(
                "{symbol} is not a stock code IPOT knows"
            )))
        }
        parse::Page::Summary(p) => p,
    };
    let mut s = Sourced::fresh(
        p.into_summary(symbol.clone(), from, to, investor, board),
        NAME,
    );
    s.cached = f.cached;
    s.stale = f.stale;
    s.fetched_at = f.fetched_at;
    Ok(s)
}

/// Trading days [`broker_summary_range`] reads one by one from the end of a
/// range while it looks for the last two with trades. Past them the rest of
/// the range cannot hold the source's latest session.
const REBUILD_WALK: usize = 5;

/// The trading day before `date`.
fn previous_trading_day(date: NaiveDate) -> NaiveDate {
    holidays::last_trading_day(date.pred_opt().expect("date in range"))
}

/// `true` when the foreign net of this `from..=to` page may count the
/// source's latest session twice: a range of several days with trades that,
/// on the day it was fetched, reached one of the last two trading days. The
/// day it was fetched counts, not only `today`: a page cached while its range
/// was current keeps the doubled figure after the range has aged.
pub fn range_net_suspect(
    s: &Sourced<BrokerSummary>,
    from: NaiveDate,
    to: NaiveDate,
    today: NaiveDate,
) -> bool {
    if from >= to || s.data.is_empty() || s.data.foreign_net_value.is_none() {
        return false;
    }
    let fetched = s
        .fetched_at
        .with_timezone(&crate::model::wib())
        .date_naive()
        .min(today);
    to >= previous_trading_day(holidays::last_trading_day(fetched))
}

/// [`broker_summary`] over `from..=to` with a foreign net that holds.
///
/// The source's `F. NVal` for a range that starts before its latest session
/// and contains it counts that session twice. When the page may do so (see
/// [`range_net_suspect`]), the net is rebuilt as the sum of the last two
/// sessions with trades, each read on its own, and of the rest of the range,
/// which then ends before both: whether the source doubles its latest
/// completed session or the one still trading, neither is inside that rest.
/// A range that ended before the last two trading days costs no extra
/// request. One that reaches them usually costs three more (four when it
/// reaches a session still trading, which has no lists yet): the walk reads
/// up to [`REBUILD_WALK`] single sessions until two had trades, then the
/// rest, so six when the latest sessions had no trades, and more only when a
/// cached rest has to be split again. Most of them are cached.
/// Totals, lots and the broker lists stay those of the range page. A rebuild
/// that fails leaves the net `null` with a warning.
pub async fn broker_summary_range(
    fetcher: &Arc<Fetcher>,
    symbol: &Symbol,
    from: NaiveDate,
    to: NaiveDate,
    investor: BrokerInvestor,
    board: BrokerBoard,
    today: NaiveDate,
) -> Result<Sourced<BrokerSummary>> {
    let mut s = broker_summary(fetcher, symbol, from, to, investor, board, today).await?;
    if !range_net_suspect(&s, from, to, today) {
        return Ok(s);
    }
    let rebuilt = rebuild_net(fetcher, symbol, from, to, investor, board, today, &mut s).await;
    match rebuilt {
        Ok(net) => s.data.foreign_net_value = net,
        Err(e) => {
            s.data.foreign_net_value = None;
            s.warnings.push(format!(
                "{symbol}: no foreign net from {from} to {to}: the source counts the latest session twice over a range, and reading the sessions one by one failed ({e})"
            ));
        }
    }
    s.data.sell_value = parse::sell_value(investor, s.data.buy_value, s.data.foreign_net_value);
    Ok(s)
}

/// The foreign net of `from..=to` from pages that hold: single sessions
/// walked back from the end until two had trades, plus the rest of the range
/// (split again if that page is itself suspect, which only a copy cached
/// while it was current can be). Every page's provenance goes into `into`.
#[allow(clippy::too_many_arguments)]
async fn rebuild_net(
    fetcher: &Arc<Fetcher>,
    symbol: &Symbol,
    from: NaiveDate,
    to: NaiveDate,
    investor: BrokerInvestor,
    board: BrokerBoard,
    today: NaiveDate,
    into: &mut Sourced<BrokerSummary>,
) -> Result<Option<f64>> {
    let mut net = 0.0;
    let mut end = to.min(today);
    loop {
        let (mut walked, mut traded, mut earliest) = (0, 0, None);
        let mut day = end;
        while day >= from && traded < 2 && walked < REBUILD_WALK {
            if holidays::is_trading_day(day) {
                let one = broker_summary(fetcher, symbol, day, day, investor, board, today).await?;
                into.absorb(&one);
                let Some(v) = one.data.foreign_net_value else {
                    return Ok(None);
                };
                net += v;
                traded += usize::from(!one.data.is_empty());
                walked += 1;
                earliest = Some(day);
            }
            day = day.pred_opt().expect("date in range");
        }
        // No trading day in the range by the bundled calendar, yet the page
        // has trades: nothing to rebuild from.
        let Some(first) = earliest else {
            return Ok(None);
        };
        let rest_end = previous_trading_day(first);
        if rest_end < from {
            return Ok(Some(net));
        }
        let rest = broker_summary(fetcher, symbol, from, rest_end, investor, board, today).await?;
        into.absorb(&rest);
        if !range_net_suspect(&rest, from, rest_end, today) {
            return Ok(rest.data.foreign_net_value.map(|v| net + v));
        }
        end = rest_end;
    }
}

/// What [`foreign_day`] found for one date.
#[derive(Debug, Clone, PartialEq)]
pub enum ForeignSession {
    /// The stock did not trade that day.
    NoTrades,
    /// Today: the stock trades, but the source publishes the session only
    /// after the close. During the session the all-investor page carries the
    /// totals so far without broker lists, the foreign page is the empty
    /// template and `F. NVal` reads 0 (seen 2026-11-06 09:15 WIB), which is
    /// not known to be zero.
    Pending,
    /// `rounded`: the source abbreviated a figure.
    Traded { day: ForeignValueDay, rounded: bool },
}

/// One session's foreign trading from two summaries of `date`, both single
/// sessions (always right, unlike a range's foreign net):
/// - all investors: the value traded, lots and average price, and whether
///   the stock traded at all;
/// - foreign investors: their buying is that page's `T. Val` and `F. NVal`
///   the foreign net, so their selling is buying minus net. A foreign page
///   without trades on a past traded day means no foreigner traded: zeros.
///   Today it means the figures are not published yet.
///
/// Both pages share their cache with `idx broker --date` and `bandar --daily`.
pub async fn foreign_day(
    fetcher: &Arc<Fetcher>,
    symbol: &Symbol,
    date: NaiveDate,
    today: NaiveDate,
) -> Result<Sourced<ForeignSession>> {
    let all = broker_summary(
        fetcher,
        symbol,
        date,
        date,
        BrokerInvestor::All,
        BrokerBoard::All,
        today,
    )
    .await?;
    if all.data.is_empty() {
        // While a session trades, its page has the totals so far but no
        // broker lists and no foreign breakdown (seen 2026-11-06 09:15 WIB).
        let trading = date >= today && all.data.total_value.is_some_and(|v| v > 0.0);
        return Ok(all.map(|_| {
            if trading {
                ForeignSession::Pending
            } else {
                ForeignSession::NoTrades
            }
        }));
    }
    let foreign = broker_summary(
        fetcher,
        symbol,
        date,
        date,
        BrokerInvestor::Foreign,
        BrokerBoard::All,
        today,
    )
    .await?;
    let (a, f) = (&all.data, &foreign.data);
    if date >= today && f.is_empty() {
        let mut s = all.map(|_| ForeignSession::Pending);
        s.absorb(&foreign);
        return Ok(s);
    }
    let share = match (f.buy_value, f.sell_value, a.total_value) {
        (Some(b), Some(s), Some(v)) if v > 0.0 => Some((b + s) / (2.0 * v) * 100.0),
        _ => None,
    };
    let day = ForeignValueDay {
        date,
        value: a.total_value,
        lots: a.total_lots,
        average_price: a.average_price,
        foreign_buy_value: f.buy_value,
        foreign_sell_value: f.sell_value,
        foreign_net_value: f.foreign_net_value,
        foreign_share_pct: share,
    };
    let rounded = a.rounded || f.rounded;
    let mut s = all.map(|_| ForeignSession::Traded { day, rounded });
    s.absorb(&foreign);
    Ok(s)
}

/// One HTML fragment, retrying the spurious 404 stretches; any other
/// failure is a provider error naming `what`. `key` is the cache key when
/// not the URL.
async fn page(
    fetcher: &Arc<Fetcher>,
    url: &str,
    ttl: Duration,
    key: Option<&str>,
    what: &str,
    symbol: &Symbol,
) -> Result<Fetched> {
    let headers = [("Accept".to_string(), "text/html".to_string())];
    let mut f = fetcher.get(url, &headers, ttl, key).await?;
    for attempt in 1..ATTEMPTS {
        if f.status != 404 {
            break;
        }
        tracing::debug!(%symbol, attempt, "ipot answered 404, retrying");
        {
            let _pause = crate::progress::retrying(url, attempt);
            tokio::time::sleep(RETRY_PAUSE * (1 << (attempt - 1))).await;
        }
        f = fetcher.get(url, &headers, ttl, key).await?;
    }
    if !f.is_success() {
        return Err(IdxError::provider(
            NAME,
            format!("{what} for {symbol}: HTTP {}", f.status),
        ));
    }
    Ok(f)
}

pub fn actions_url(symbol: &Symbol, since: NaiveDate) -> String {
    format!(
        "{ACTIONS_ENDPOINT}?code={}&start={}",
        symbol.as_str(),
        since.format("%m/%d/%Y")
    )
}

/// Corporate actions, shareholders' meetings and listed warrants and rights
/// of `symbol` from `since` on. The company name is left for the caller.
pub async fn corporate_actions(
    fetcher: &Arc<Fetcher>,
    symbol: &Symbol,
    since: NaiveDate,
) -> Result<Sourced<crate::model::CorporateActions>> {
    let f = page(
        fetcher,
        &actions_url(symbol, since),
        TTL_ACTIONS,
        None,
        "corporate actions",
        symbol,
    )
    .await?;
    let p = match actions::parse(&f.body) {
        actions::Page::Invalid => {
            return Err(IdxError::NotFound(format!(
                "{symbol} is not a stock code IPOT knows"
            )))
        }
        actions::Page::Actions(p) => p,
    };
    let mut s = Sourced::fresh(
        crate::model::CorporateActions {
            symbol: symbol.clone(),
            name: None,
            since,
            actions: p.actions,
            meetings: p.meetings,
            warrants_rights: p.warrants_rights,
        },
        NAME,
    );
    s.cached = f.cached;
    s.stale = f.stale;
    s.fetched_at = f.fetched_at;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::Cache;
    use crate::http::{CacheMode, FixtureBackend};

    const DAY: &str = include_str!("../../../tests/fixtures/ipot/brokersummary_bbca_20261016.html");
    const INVALID: &str =
        include_str!("../../../tests/fixtures/ipot/brokersummary_zzzz_invalid.html");

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    fn fetcher(be: FixtureBackend) -> (Arc<Fetcher>, Arc<FixtureBackend>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let be = Arc::new(be);
        let f = Arc::new(Fetcher::new(
            be.clone(),
            Cache::new(dir.path()),
            CacheMode::Normal,
        ));
        (f, be, dir)
    }

    #[test]
    fn url_maps_filters_to_source_codes() {
        let s = Symbol::parse("bbca").unwrap();
        let u = url(
            &s,
            d(2026, 10, 12),
            d(2026, 10, 16),
            BrokerInvestor::Foreign,
            BrokerBoard::Regular,
        );
        assert_eq!(
            u,
            "https://www.indopremier.com/module/saham/include/data-brokersummary.php?code=BBCA&start=10/12/2026&end=10/16/2026&fd=F&board=RG"
        );
    }

    #[tokio::test]
    async fn fetches_parses_and_caches() {
        let (f, be, _d) =
            fetcher(FixtureBackend::new().route("data-brokersummary.php?code=BBCA", 200, DAY));
        let s = Symbol::parse("BBCA").unwrap();
        let day = d(2026, 10, 16);
        let got = broker_summary(
            &f,
            &s,
            day,
            day,
            BrokerInvestor::All,
            BrokerBoard::All,
            d(2026, 10, 17),
        )
        .await
        .unwrap();
        assert_eq!(got.provider, "ipot");
        assert_eq!(got.data.buyers[0].code, "MG");
        assert_eq!((got.data.from, got.data.to), (day, day));
        assert!(!got.cached);
        let again = broker_summary(
            &f,
            &s,
            day,
            day,
            BrokerInvestor::All,
            BrokerBoard::All,
            d(2026, 10, 17),
        )
        .await
        .unwrap();
        assert!(again.cached);
        assert_eq!(be.call_count(), 1);
    }

    #[tokio::test]
    async fn a_session_read_while_it_traded_is_read_again_once_it_closed() {
        let (f, be, _d) = fetcher(pages(&[(
            "11/06/2026",
            "11/06/2026",
            "brokersummary_bbca_20261106_insession.html",
        )]));
        let s = Symbol::parse("BBCA").unwrap();
        let day = d(2026, 11, 6);
        let read = |today: NaiveDate| {
            broker_summary(
                &f,
                &s,
                day,
                day,
                BrokerInvestor::All,
                BrokerBoard::All,
                today,
            )
        };
        // During the session: the totals so far, no broker lists yet.
        let live = read(day).await.unwrap();
        assert!(live.data.is_empty());
        assert_eq!(live.data.total_value, Some(89_500_000_000.0));
        assert!(
            read(day).await.unwrap().cached,
            "minutes later: the cached copy"
        );
        assert_eq!(be.call_count(), 1);
        // On Monday the 11-06 copy is not the closed session: read again,
        // then kept for the week.
        let monday = d(2026, 11, 9);
        assert!(!read(monday).await.unwrap().cached);
        assert!(read(monday).await.unwrap().cached);
        assert_eq!(be.call_count(), 2);
    }

    #[tokio::test]
    async fn unknown_code_is_not_found_and_http_errors_are_provider_errors() {
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route("code=ZZZZ", 200, INVALID)
                .route("code=BBRI", 503, "busy")
                .route("code=TLKM", 404, "Not Found"),
        );
        let day = d(2026, 10, 16);
        let err = broker_summary(
            &f,
            &Symbol::parse("ZZZZ").unwrap(),
            day,
            day,
            BrokerInvestor::All,
            BrokerBoard::All,
            day,
        )
        .await
        .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        let err = broker_summary(
            &f,
            &Symbol::parse("BBRI").unwrap(),
            day,
            day,
            BrokerInvestor::All,
            BrokerBoard::All,
            day,
        )
        .await
        .unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert_eq!(be.call_count(), 2, "only the spurious 404 is retried");

        let err = broker_summary(
            &f,
            &Symbol::parse("TLKM").unwrap(),
            day,
            day,
            BrokerInvestor::All,
            BrokerBoard::All,
            day,
        )
        .await
        .unwrap_err();
        assert_eq!(err.exit_code(), 3);
        assert!(err.to_string().contains("HTTP 404"), "{err}");
        assert_eq!(be.call_count(), 2 + ATTEMPTS);
    }

    /// Recorded pages, one route each: `(start, end, file)` with dates as
    /// the URL carries them.
    fn pages(list: &[(&str, &str, &str)]) -> FixtureBackend {
        let mut be = FixtureBackend::new();
        for (start, end, file) in list {
            let body = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/ipot")
                    .join(file),
            )
            .unwrap();
            be = be.route(&format!("code=BBCA&start={start}&end={end}&"), 200, &body);
        }
        be
    }

    async fn range(
        f: &Arc<Fetcher>,
        from: NaiveDate,
        to: NaiveDate,
        today: NaiveDate,
    ) -> Sourced<BrokerSummary> {
        broker_summary_range(
            f,
            &Symbol::parse("BBCA").unwrap(),
            from,
            to,
            BrokerInvestor::All,
            BrokerBoard::All,
            today,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_range_through_the_latest_session_gets_its_foreign_net_rebuilt() {
        // Recorded the weekend after: the range page counts 10-16 twice (-4.1 T).
        let (f, be, _d) = fetcher(pages(&[
            (
                "10/12/2026",
                "10/16/2026",
                "brokersummary_bbca_20261012_20261016.html",
            ),
            (
                "10/12/2026",
                "10/14/2026",
                "brokersummary_bbca_20261012_20261014.html",
            ),
            (
                "10/15/2026",
                "10/15/2026",
                "brokersummary_bbca_20261015.html",
            ),
            (
                "10/16/2026",
                "10/16/2026",
                "brokersummary_bbca_20261016.html",
            ),
        ]));
        let s = range(&f, d(2026, 10, 12), d(2026, 10, 16), d(2026, 10, 17)).await;
        // 190.8 B + 286.1 B - 2.3 T: the five sessions one by one.
        assert_eq!(s.data.foreign_net_value, Some(-1_823_100_000_000.0));
        assert_eq!(s.data.total_value, Some(9_900_000_000_000.0));
        assert_eq!(s.data.total_lots, Some(18_100_000.0));
        assert_eq!(s.data.buyers[0].code, "KZ", "lists stay the range page's");
        assert_eq!((s.data.from, s.data.to), (d(2026, 10, 12), d(2026, 10, 16)));
        assert!(s.warnings.is_empty(), "{:?}", s.warnings);
        assert_eq!(be.call_count(), 4);

        // Probed 2026-11-06 before the open (latest session 11-05): 595.4 B.
        let (f, be, _d) = fetcher(pages(&[
            (
                "11/02/2026",
                "11/05/2026",
                "brokersummary_bbca_20261102_20261105.html",
            ),
            (
                "11/02/2026",
                "11/03/2026",
                "brokersummary_bbca_20261102_20261103.html",
            ),
            (
                "11/04/2026",
                "11/04/2026",
                "brokersummary_bbca_20261104.html",
            ),
            (
                "11/05/2026",
                "11/05/2026",
                "brokersummary_bbca_20261105.html",
            ),
        ]));
        let s = range(&f, d(2026, 11, 2), d(2026, 11, 5), d(2026, 11, 6)).await;
        // -144.9 B + 133.3 B + 303.9 B
        assert_eq!(s.data.foreign_net_value, Some(292_300_000_000.0));
        assert_eq!(s.data.total_value, Some(5_000_000_000_000.0));
        assert_eq!(be.call_count(), 4);
    }

    #[tokio::test]
    async fn the_rebuild_costs_nothing_for_older_ranges_and_skips_an_empty_rest() {
        // Ends before the last two trading days: one request, net as published.
        let (f, be, _d) = fetcher(pages(&[(
            "11/02/2026",
            "11/04/2026",
            "brokersummary_bbca_20261102_20261104.html",
        )]));
        let s = range(&f, d(2026, 11, 2), d(2026, 11, 4), d(2026, 11, 9)).await;
        assert_eq!(s.data.foreign_net_value, Some(-11_700_000_000.0));
        assert_eq!(be.call_count(), 1);

        // The same 10-12..10-16 range read again on 11-06 was right: -1.9 T.
        let (f, be, _d) = fetcher(pages(&[(
            "10/12/2026",
            "10/16/2026",
            "brokersummary_bbca_20261012_20261016_later.html",
        )]));
        let s = range(&f, d(2026, 10, 12), d(2026, 10, 16), d(2026, 11, 6)).await;
        assert_eq!(s.data.foreign_net_value, Some(-1_900_000_000_000.0));
        assert_eq!(be.call_count(), 1);

        // Two sessions, both split off: no rest to read (740.1 B published).
        let (f, be, _d) = fetcher(pages(&[
            (
                "11/04/2026",
                "11/05/2026",
                "brokersummary_bbca_20261104_20261105.html",
            ),
            (
                "11/04/2026",
                "11/04/2026",
                "brokersummary_bbca_20261104.html",
            ),
            (
                "11/05/2026",
                "11/05/2026",
                "brokersummary_bbca_20261105.html",
            ),
        ]));
        let s = range(&f, d(2026, 11, 4), d(2026, 11, 5), d(2026, 11, 6)).await;
        // 133.3 B + 303.9 B
        assert_eq!(s.data.foreign_net_value, Some(437_200_000_000.0));
        assert_eq!(be.call_count(), 3);
    }

    #[tokio::test]
    async fn the_walk_skips_a_session_without_trades_and_a_failed_rebuild_is_a_warning() {
        // Friday 11-06 at 09:15 WIB: today's page has the totals so far but
        // no broker lists (and a foreign net of 0), so the last two sessions
        // with trades are 11-05 and 11-04. During the session the range page
        // is right (292.2 B); the rebuild agrees at the source's precision.
        let (f, be, _d) = fetcher(pages(&[
            (
                "11/02/2026",
                "11/06/2026",
                "brokersummary_bbca_20261102_20261106_insession.html",
            ),
            (
                "11/02/2026",
                "11/03/2026",
                "brokersummary_bbca_20261102_20261103.html",
            ),
            (
                "11/04/2026",
                "11/04/2026",
                "brokersummary_bbca_20261104.html",
            ),
            (
                "11/05/2026",
                "11/05/2026",
                "brokersummary_bbca_20261105.html",
            ),
            (
                "11/06/2026",
                "11/06/2026",
                "brokersummary_bbca_20261106_insession.html",
            ),
        ]));
        let s = range(&f, d(2026, 11, 2), d(2026, 11, 6), d(2026, 11, 6)).await;
        assert_eq!(s.data.foreign_net_value, Some(292_300_000_000.0));
        assert_eq!(s.data.total_value, Some(5_100_000_000_000.0));
        assert_eq!(be.call_count(), 5, "range, 11-06, 11-05, 11-04, the rest");

        // The 11-02..11-05 page read during the session is right too.
        let (f, _be, _d) = fetcher(pages(&[
            (
                "11/02/2026",
                "11/05/2026",
                "brokersummary_bbca_20261102_20261105_insession.html",
            ),
            (
                "11/02/2026",
                "11/03/2026",
                "brokersummary_bbca_20261102_20261103.html",
            ),
            (
                "11/04/2026",
                "11/04/2026",
                "brokersummary_bbca_20261104.html",
            ),
            (
                "11/05/2026",
                "11/05/2026",
                "brokersummary_bbca_20261105.html",
            ),
        ]));
        let s = range(&f, d(2026, 11, 2), d(2026, 11, 5), d(2026, 11, 6)).await;
        assert_eq!(s.data.foreign_net_value, Some(292_300_000_000.0));

        // A session that cannot be read: no net, said so; the rest stands.
        let (f, _be, _d) = fetcher(pages(&[
            (
                "11/02/2026",
                "11/05/2026",
                "brokersummary_bbca_20261102_20261105.html",
            ),
            (
                "11/05/2026",
                "11/05/2026",
                "brokersummary_bbca_20261105.html",
            ),
        ]));
        let s = range(&f, d(2026, 11, 2), d(2026, 11, 5), d(2026, 11, 6)).await;
        assert_eq!(s.data.foreign_net_value, None);
        assert_eq!(s.data.total_value, Some(5_000_000_000_000.0));
        assert!(
            s.warnings[0].contains("no foreign net from 2026-11-02 to 2026-11-05"),
            "{:?}",
            s.warnings
        );
    }

    #[test]
    fn a_page_cached_while_its_range_was_current_stays_suspect() {
        let page = |from: NaiveDate, to: NaiveDate, fetched: &str| {
            let html = include_str!(
                "../../../tests/fixtures/ipot/brokersummary_bbca_20261102_20261105.html"
            );
            let parse::Page::Summary(p) = parse::parse(html).unwrap() else {
                panic!("a summary")
            };
            let mut s = Sourced::fresh(
                p.into_summary(
                    Symbol::parse("BBCA").unwrap(),
                    from,
                    to,
                    BrokerInvestor::All,
                    BrokerBoard::All,
                ),
                NAME,
            );
            s.fetched_at = chrono::DateTime::parse_from_rfc3339(fetched)
                .unwrap()
                .with_timezone(&chrono::Utc);
            s
        };
        let (from, to) = (d(2026, 11, 2), d(2026, 11, 5));
        // Fetched Friday 07:00 WIB: the 5th is the latest session.
        let friday = page(from, to, "2026-11-06T00:00:00Z");
        assert!(range_net_suspect(&friday, from, to, d(2026, 11, 6)));
        // Still the cached copy the next Monday: the doubled net is frozen in it.
        assert!(range_net_suspect(&friday, from, to, d(2026, 11, 9)));
        // Fetched fresh on Monday, the range has aged out of the source's quirk.
        let monday = page(from, to, "2026-11-09T02:00:00Z");
        assert!(!range_net_suspect(&monday, from, to, d(2026, 11, 9)));
        // One session is always right.
        assert!(!range_net_suspect(&friday, to, to, d(2026, 11, 6)));
    }

    fn fixture(file: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/ipot")
                .join(file),
        )
        .unwrap()
    }

    /// The all-investor and foreign pages of one session.
    fn session(
        be: FixtureBackend,
        code: &str,
        mdy: &str,
        all: &str,
        foreign: &str,
    ) -> FixtureBackend {
        be.route(
            &format!("code={code}&start={mdy}&end={mdy}&fd=all&"),
            200,
            &fixture(all),
        )
        .route(
            &format!("code={code}&start={mdy}&end={mdy}&fd=F&"),
            200,
            &fixture(foreign),
        )
    }

    #[tokio::test]
    async fn a_session_of_foreign_flow_from_two_pages() {
        let be = session(
            FixtureBackend::new(),
            "BBCA",
            "10/16/2026",
            "brokersummary_bbca_20261016.html",
            "brokersummary_bbca_20261016_foreign.html",
        );
        let be = session(
            be,
            "BBCA",
            "11/05/2026",
            "brokersummary_bbca_20261105.html",
            "brokersummary_bbca_20261105_foreign.html",
        );
        let be = session(
            be,
            "TRUK",
            "11/05/2026",
            "brokersummary_truk_20261105.html",
            "brokersummary_truk_20261105_foreign.html",
        )
        .route(
            "code=BBCA&start=10/17/2026&end=10/17/2026&fd=all&",
            200,
            &fixture("brokersummary_bbca_20261017_no_trades.html"),
        );
        let be = session(
            be,
            "BBCA",
            "11/06/2026",
            "brokersummary_bbca_20261106_insession.html",
            "brokersummary_bbca_20261106_insession_foreign.html",
        );
        let (f, be, _d) = fetcher(be);
        let bbca = Symbol::parse("BBCA").unwrap();
        let day = |s: Sourced<ForeignSession>| match s.data {
            ForeignSession::Traded { day, rounded } => (day, rounded),
            other => panic!("a traded session, not {other:?}"),
        };

        let (d16, rounded) = day(foreign_day(&f, &bbca, d(2026, 10, 16), d(2026, 10, 17))
            .await
            .unwrap());
        assert!(rounded);
        assert_eq!(d16.date, d(2026, 10, 16));
        assert_eq!(d16.value, Some(6_600_000_000_000.0));
        assert_eq!(d16.lots, Some(12_600_000.0));
        assert_eq!(d16.average_price, Some(5_228.0));
        assert_eq!(d16.foreign_buy_value, Some(1_700_000_000_000.0));
        assert_eq!(d16.foreign_sell_value, Some(4_000_000_000_000.0));
        assert_eq!(d16.foreign_net_value, Some(-2_300_000_000_000.0));
        let share = d16.foreign_share_pct.unwrap();
        assert!((share - 5.7 / 13.2 * 100.0).abs() < 1e-9, "{share}");
        assert_eq!(be.call_count(), 2);
        // The same pages serve `idx broker --date`.
        broker_summary(
            &f,
            &bbca,
            d(2026, 10, 16),
            d(2026, 10, 16),
            BrokerInvestor::All,
            BrokerBoard::All,
            d(2026, 10, 17),
        )
        .await
        .unwrap();
        assert_eq!(be.call_count(), 2);

        // 11-05: 605.4 B bought, 303.9 B net, so 301.5 B sold, of 848.3 B.
        let (d5, _) = day(foreign_day(&f, &bbca, d(2026, 11, 5), d(2026, 11, 6))
            .await
            .unwrap());
        assert_eq!(d5.foreign_sell_value, Some(301_500_000_000.0));
        let share = d5.foreign_share_pct.unwrap();
        assert!((share - 906.9 / 1_696.6 * 100.0).abs() < 1e-9, "{share}");

        // TRUK traded 21.6 M on 11-05 and no foreigner did: zeros, not missing.
        let truk = Symbol::parse("TRUK").unwrap();
        let (t, _) = day(foreign_day(&f, &truk, d(2026, 11, 5), d(2026, 11, 6))
            .await
            .unwrap());
        assert_eq!(t.value, Some(21_600_000.0));
        assert_eq!(
            (
                t.foreign_buy_value,
                t.foreign_sell_value,
                t.foreign_net_value
            ),
            (Some(0.0), Some(0.0), Some(0.0))
        );
        assert_eq!(t.foreign_share_pct, Some(0.0));

        // No trades that day: no session, and no foreign page asked for.
        let before = be.call_count();
        let none = foreign_day(&f, &bbca, d(2026, 10, 17), d(2026, 10, 17))
            .await
            .unwrap();
        assert_eq!(none.data, ForeignSession::NoTrades);
        assert_eq!(be.call_count(), before + 1);

        // 11-06 at 09:15 WIB: 89.5 B traded so far, no broker lists yet.
        let before = be.call_count();
        let today = foreign_day(&f, &bbca, d(2026, 11, 6), d(2026, 11, 6))
            .await
            .unwrap();
        assert_eq!(today.data, ForeignSession::Pending);
        assert_eq!(be.call_count(), before + 1, "no foreign page asked for");
        // Its foreign page is the empty template as well.
        let foreign = broker_summary(
            &f,
            &bbca,
            d(2026, 11, 6),
            d(2026, 11, 6),
            BrokerInvestor::Foreign,
            BrokerBoard::All,
            d(2026, 11, 6),
        )
        .await
        .unwrap();
        assert!(foreign.data.is_empty());
        assert_eq!(foreign.data.buy_value, Some(0.0));
    }

    const ACTIONS_MEGA: &str =
        include_str!("../../../tests/fixtures/ipot/corporateaction_mega_20250101.html");
    const ACTIONS_INVALID: &str =
        include_str!("../../../tests/fixtures/ipot/corporateaction_zzzz_invalid.html");

    #[tokio::test]
    async fn corporate_actions_url_parse_and_unknown_code() {
        let s = Symbol::parse("mega").unwrap();
        assert_eq!(
            actions_url(&s, d(2025, 1, 1)),
            "https://www.indopremier.com/module/saham/include/corporateaction.php?code=MEGA&start=01/01/2025"
        );
        let (f, be, _d) = fetcher(
            FixtureBackend::new()
                .route("corporateaction.php?code=MEGA", 200, ACTIONS_MEGA)
                .route("corporateaction.php?code=ZZZZ", 200, ACTIONS_INVALID),
        );
        let got = corporate_actions(&f, &s, d(2025, 1, 1)).await.unwrap();
        assert_eq!(got.provider, "ipot");
        assert_eq!(got.data.symbol.as_str(), "MEGA");
        assert_eq!(got.data.since, d(2025, 1, 1));
        assert!(got.data.actions.len() >= 3);
        assert_eq!(got.data.meetings.len(), 2);
        let err = corporate_actions(&f, &Symbol::parse("ZZZZ").unwrap(), d(2025, 1, 1))
            .await
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        assert_eq!(be.call_count(), 2);
    }
}
