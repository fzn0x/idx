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
//!
//! The market-wide broker ranking, verified 2026-11-08, same module (plain
//! nginx, no Cloudflare, no cookie needed) and the same spurious 404s, seen
//! on both pages that day and retried:
//! - `.../ma-topbroker.php` takes no parameters (`start` and `end` are
//!   ignored) and ranks every broker of IPOT's latest session by value, with
//!   value bought and sold, net, shares and trades; brokers without trades
//!   are listed with zeros. Past sessions cannot be asked for. See
//!   `topbroker.rs`.
//! - The page carries no date. `.../statistic.php?code=BBCA` dates it: its
//!   `Today` row carries the day of the stock's high and low, BBCA being
//!   one of the most traded stocks. [`top_brokers`] reads both pages.
//! - On Sunday 2026-11-08 the ranking matched idx.co.id's broker summary of
//!   2026-11-06 once JB, listed apart as `[CC] BJB Sekuritas Jawa Barat`, is
//!   counted in CC. When the two pages roll over to a new session, and
//!   whether the ranking moves during a session, has not been measured: a
//!   ranking read on a trading day from 08:58 to 19:00 WIB cannot be dated
//!   and is refused, and so is one with trades dated a session that had not
//!   opened when it was read.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, NaiveDate, Timelike, Utc};

use crate::error::{IdxError, Result};
use crate::holidays;
use crate::http::{Fetched, Fetcher};
use crate::model::{BrokerBoard, BrokerInvestor, BrokerSummary, ForeignValueDay, Symbol};

use super::Sourced;

pub mod actions;
pub mod parse;
pub mod topbroker;

pub const NAME: &str = "ipot";
const ENDPOINT: &str = "https://www.indopremier.com/module/saham/include/data-brokersummary.php";
const ACTIONS_ENDPOINT: &str =
    "https://www.indopremier.com/module/saham/include/corporateaction.php";
/// Announcements are added and confirmed through the day.
const TTL_ACTIONS: Duration = Duration::from_secs(6 * 3600);
const TOP_BROKERS_ENDPOINT: &str =
    "https://www.indopremier.com/module/saham/include/ma-topbroker.php";
/// The page that dates the ranking.
const SESSION_ENDPOINT: &str =
    "https://www.indopremier.com/module/saham/include/statistic.php?code=BBCA";
/// Seconds after midnight WIB, on a trading day, from before the
/// pre-opening (08:45) until well after the close (16:15) when the ranking
/// may change between two reads.
const TOP_BROKERS_BUSY: (u32, u32) = (8 * 3600 + 30 * 60, 19 * 3600);
const TTL_TOP_BROKERS_BUSY: Duration = Duration::from_secs(5 * 60);
/// The longest a copy is kept outside the busy hours.
const TTL_TOP_BROKERS_QUIET: Duration = Duration::from_secs(6 * 3600);
/// The ranking and its date count as one read when fetched this close.
const READ_TOGETHER: Duration = Duration::from_secs(60);
/// A session's first trades: the pre-opening match starts at 08:58:00 WIB
/// (IDX trading hours, Kep-00158/BEI/10-2026).
const FIRST_TRADES_SECS: u32 = 8 * 3600 + 58 * 60;
/// Seconds after midnight WIB, on a trading day, when a ranking cannot be
/// dated: from the session's first trades until the busy hours end. When
/// the two pages roll over to a new session has not been measured, so then
/// either may show the new session while the other still shows the last.
const UNDATED: (u32, u32) = (FIRST_TRADES_SECS, TOP_BROKERS_BUSY.1);

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
/// page is read again instead of serving the partial copy for a week. A
/// closed range whose cached page still shows trades without broker lists
/// (see [`unpublished`]) gets the short lifetime too.
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
    let read = |f: &Fetched| -> Result<BrokerSummary> {
        match parse::parse(&f.body)? {
            parse::Page::Invalid => Err(IdxError::NotFound(format!(
                "{symbol} is not a stock code IPOT knows"
            ))),
            parse::Page::Summary(p) => {
                Ok(p.into_summary(symbol.clone(), from, to, investor, board))
            }
        }
    };
    let mut f = page(fetcher, &url, ttl, key.as_deref(), "broker summary", symbol).await?;
    let mut data = read(&f)?;
    // Read before the source published it: such a copy lasts minutes.
    if to < today && f.cached && unpublished(&data) {
        f = page(fetcher, &url, TTL_TODAY, None, "broker summary", symbol).await?;
        data = read(&f)?;
    }
    Ok(Sourced::from_fetched(data, NAME, &f))
}

/// `true` when a page shows trades in its footer but no broker lists: the
/// source has not published the range yet. Seen for a session still trading
/// (2026-11-06 09:15 WIB); when a closed session gets its lists is not
/// probed, so a past date read that way is treated the same.
pub fn unpublished(s: &BrokerSummary) -> bool {
    s.is_empty() && s.buy_value.is_some_and(|v| v > 0.0)
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
    /// The stock trades (or traded), but the source has not published the
    /// session: it does so only after the close. During the session the
    /// all-investor page carries the totals so far without broker lists, the
    /// foreign page is the empty template and `F. NVal` reads 0 (seen
    /// 2026-11-06 09:15 WIB), which is not known to be zero. A past date
    /// read that way (see [`unpublished`]) is pending too.
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
/// An all-investor page with trades but no broker lists is
/// [`ForeignSession::Pending`] on any date.
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
        let pending = unpublished(&all.data);
        return Ok(all.map(|_| {
            if pending {
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
/// failure is a provider error naming `what` for `subject` (a stock code,
/// or the market for the ranking). `key` is the cache key when not the URL.
async fn page(
    fetcher: &Arc<Fetcher>,
    url: &str,
    ttl: Duration,
    key: Option<&str>,
    what: &str,
    subject: &(dyn std::fmt::Display + Sync),
) -> Result<Fetched> {
    let headers = [("Accept".to_string(), "text/html".to_string())];
    let mut f = fetcher.get(url, &headers, ttl, key).await?;
    for attempt in 1..ATTEMPTS {
        if f.status != 404 {
            break;
        }
        tracing::debug!(%subject, attempt, "ipot answered 404, retrying");
        {
            let _pause = crate::progress::retrying(url, attempt);
            tokio::time::sleep(RETRY_PAUSE * (1 << (attempt - 1))).await;
        }
        f = fetcher.get(url, &headers, ttl, key).await?;
    }
    if !f.is_success() {
        return Err(IdxError::provider(
            NAME,
            format!("{what} for {subject}: HTTP {}", f.status),
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
    Ok(Sourced::from_fetched(
        crate::model::CorporateActions {
            symbol: symbol.clone(),
            name: None,
            since,
            actions: p.actions,
            meetings: p.meetings,
            warrants_rights: p.warrants_rights,
        },
        NAME,
        &f,
    ))
}

/// How long a copy of the ranking (and of the page that dates it) is good
/// for at `now`: five minutes on a trading day inside [`TOP_BROKERS_BUSY`];
/// outside it six hours at most, and never a copy read before those hours
/// last ended, so the settled figures are read once after them.
pub fn top_brokers_ttl(now: DateTime<Utc>) -> Duration {
    let local = now.with_timezone(&crate::model::wib());
    let (today, secs) = (local.date_naive(), local.time().num_seconds_from_midnight());
    let (opens, closes) = TOP_BROKERS_BUSY;
    let trading = holidays::is_trading_day(today);
    if trading && (opens..closes).contains(&secs) {
        return TTL_TOP_BROKERS_BUSY;
    }
    let last = if trading && secs >= closes {
        today
    } else {
        holidays::last_trading_day(today.pred_opt().expect("date in range"))
    };
    let ended = last
        .and_time(chrono::NaiveTime::from_num_seconds_from_midnight_opt(closes, 0).expect("time"))
        .and_local_timezone(crate::model::wib())
        .single()
        .expect("a fixed offset");
    (now - ended.with_timezone(&Utc))
        .to_std()
        .unwrap_or_default()
        .min(TTL_TOP_BROKERS_QUIET)
}

fn read_together(a: &Fetched, b: &Fetched) -> bool {
    (a.fetched_at - b.fetched_at)
        .abs()
        .to_std()
        .is_ok_and(|d| d <= READ_TOGETHER)
}

/// When `f` was read: a copy keeps the time it was fetched, a page fetched
/// by this call counts as read at `now`.
fn read_at(f: &Fetched, now: DateTime<Utc>) -> DateTime<Utc> {
    if f.cached {
        f.fetched_at.min(now)
    } else {
        now
    }
}

/// Every broker's trading in IPOT's latest session, ranked by value, dated
/// by BBCA's statistics page, with the brokers IPOT lists apart folded into
/// the member they trade under (a warning names each). `now` decides the
/// cache lifetime ([`top_brokers_ttl`]).
///
/// The two pages count only as one read: when one comes from the network
/// and the other from the cache, or two copies were read apart, both are
/// read again. When they still differ (a fetch failed and an older copy
/// stood in), the ranking cannot be dated and the call fails. The pair is
/// judged by when it was read, not by when it is served (a copy can be
/// served offline or after a failed fetch, days later). A pair read on a
/// trading day between 08:58 and 19:00 WIB ([`UNDATED`]) cannot be dated
/// and fails, and so does a ranking with trades dated a session that had
/// not opened then (before 08:58), which means the pages rolled over at
/// different times.
pub async fn top_brokers(
    fetcher: &Arc<Fetcher>,
    now: DateTime<Utc>,
) -> Result<Sourced<topbroker::Ranking>> {
    let ttl = top_brokers_ttl(now);
    let ranking = |ttl| {
        page(
            fetcher,
            TOP_BROKERS_ENDPOINT,
            ttl,
            None,
            "broker ranking",
            &"the market",
        )
    };
    let dating = |ttl| {
        page(
            fetcher,
            SESSION_ENDPOINT,
            ttl,
            None,
            "session date",
            &"BBCA",
        )
    };
    let mut ranked = ranking(ttl).await?;
    let mut dated = dating(if ranked.cached { ttl } else { Duration::ZERO }).await?;
    if !read_together(&ranked, &dated) {
        ranked = ranking(Duration::ZERO).await?;
        dated = dating(Duration::ZERO).await?;
    }
    if !read_together(&ranked, &dated) {
        let failed = ranked.failure.as_deref().or(dated.failure.as_deref());
        return Err(IdxError::provider(
            NAME,
            format!(
                "the broker ranking and the page that dates it were read at different times, so the ranking has no date{}",
                failed.map(|w| format!(" ({w})")).unwrap_or_default()
            ),
        ));
    }
    let read = read_at(&ranked, now).min(read_at(&dated, now));
    let local = read.with_timezone(&crate::model::wib());
    let (day, clock) = (local.date_naive(), local.format("%H:%M"));
    let secs = local.time().num_seconds_from_midnight();
    if holidays::is_trading_day(day) && (UNDATED.0..UNDATED.1).contains(&secs) {
        return Err(IdxError::provider(
            NAME,
            format!(
                "the broker ranking was read at {clock} WIB on {day}, in a trading day's hours (08:58 to 19:00 WIB), when it and the page that dates it can be on different sessions (when they roll over has not been measured), so it cannot be dated"
            ),
        ));
    }
    let date = topbroker::session_date(&dated.body).ok_or_else(|| {
        IdxError::provider(
            NAME,
            "BBCA's statistics page has no Today row to date the broker ranking",
        )
    })?;
    if !holidays::is_trading_day(date) {
        return Err(IdxError::provider(
            NAME,
            format!(
                "the page that dates the broker ranking names {date}, which is not a trading day"
            ),
        ));
    }
    let mut data = topbroker::ranking(&ranked.body, date)?;
    let traded = data.brokers.iter().any(topbroker::TopBroker::traded);
    // A date whose session had not opened when the pages were read cannot
    // have trades, and one whose session had ended cannot be without them.
    let opened = date < day || date == day && secs >= FIRST_TRADES_SECS;
    if !opened && traded {
        return Err(IdxError::provider(
            NAME,
            format!(
                "the broker ranking read at {clock} WIB on {day} is dated {date}, a session that had not opened then (trading starts at 08:58 WIB), yet it shows trades: the ranking and its date disagree"
            ),
        ));
    }
    if !traded && date <= holidays::latest_session(read) {
        return Err(IdxError::provider(
            NAME,
            format!(
                "the broker ranking read at {clock} WIB on {day} shows no trades, yet the page that dates it names {date}, a session that had ended then: the two pages are on different sessions"
            ),
        ));
    }
    let folded = topbroker::fold_listed_apart(&mut data);
    let mut s = Sourced::from_fetched(data, NAME, &ranked);
    s.absorb(&Sourced::from_fetched((), NAME, &dated));
    for f in folded {
        s.warnings.push(format!(
            "{} ({}) is counted in {}, as idx.co.id counts it; IPOT lists it apart",
            f.code, f.name, f.into
        ));
    }
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
    async fn a_closed_session_read_before_it_was_published_is_not_kept_for_the_week() {
        // 11-05 as if read before the source published it: totals, no broker
        // lists (the shape of 11-06 at 09:15 WIB).
        let early = fixture("brokersummary_bbca_20261106_insession.html");
        let (f, be, dir) = fetcher(pages(&[(
            "11/05/2026",
            "11/05/2026",
            "brokersummary_bbca_20261105.html",
        )]));
        let s = Symbol::parse("BBCA").unwrap();
        let (day, today) = (d(2026, 11, 5), d(2026, 11, 6));
        let key = url(&s, day, day, BrokerInvestor::All, BrokerBoard::All);
        let cache = Cache::new(dir.path());
        let ago = |secs: i64| chrono::Utc::now() - chrono::Duration::seconds(secs);
        let read = || {
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
        // Cached an hour ago: read again, and the published page is kept.
        cache.put_fetched_at(&key, 200, &early, ago(3600)).unwrap();
        let got = read().await.unwrap();
        assert!(!got.cached);
        assert!(!got.data.is_empty());
        assert!(read().await.unwrap().cached);
        assert_eq!(be.call_count(), 1);

        // Cached a minute ago: served, and its session is pending, not
        // a day without trades.
        cache.put_fetched_at(&key, 200, &early, ago(60)).unwrap();
        let got = read().await.unwrap();
        assert!(got.cached && unpublished(&got.data));
        let session = foreign_day(&f, &s, day, today).await.unwrap();
        assert_eq!(session.data, ForeignSession::Pending);
        assert_eq!(be.call_count(), 1);
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

    const TOP: &str = include_str!("../../../tests/fixtures/ipot/topbroker_20261106.html");
    const STATISTIC: &str =
        include_str!("../../../tests/fixtures/ipot/statistic_bbca_20261106.html");

    /// A WIB wall-clock time.
    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(&format!("{s}+07:00"))
            .unwrap()
            .with_timezone(&Utc)
    }

    fn ranking_pages() -> FixtureBackend {
        FixtureBackend::new()
            .route(TOP_BROKERS_ENDPOINT, 200, TOP)
            .route(SESSION_ENDPOINT, 200, STATISTIC)
    }

    #[test]
    fn the_ranking_is_kept_minutes_in_trading_hours_and_hours_outside() {
        let min = |n: u64| Duration::from_secs(n * 60);
        for (now, want, why) in [
            ("2026-11-06T08:29:00", min(360), "Friday before 08:30"),
            ("2026-11-06T08:30:00", min(5), "from 08:30"),
            ("2026-11-06T15:00:00", min(5), "during the session"),
            ("2026-11-06T18:59:59", min(5), "until 19:00"),
            (
                "2026-11-06T19:00:30",
                Duration::from_secs(30),
                "only copies read after 19:00",
            ),
            ("2026-11-06T23:00:00", min(240), "Friday night"),
            ("2026-11-07T12:00:00", min(360), "Saturday"),
            ("2026-11-09T07:00:00", min(360), "Monday before 08:30"),
            ("2026-08-17T10:00:00", min(360), "Independence Day"),
        ] {
            assert_eq!(top_brokers_ttl(at(now)), want, "{now}: {why}");
        }
    }

    #[tokio::test]
    async fn the_ranking_comes_with_the_session_it_is_for() {
        let (f, be, _d) = fetcher(ranking_pages());
        let sunday = at("2026-11-08T10:00:00");
        let s = top_brokers(&f, sunday).await.unwrap();
        assert_eq!(s.provider, "ipot");
        assert_eq!(s.data.date, d(2026, 11, 6));
        assert_eq!(s.data.brokers.len(), 90, "JB is counted in CC");
        assert!(s.data.rounded);
        assert_eq!(
            s.warnings,
            ["JB ([CC] BJB Sekuritas Jawa Barat) is counted in CC, as idx.co.id counts it; IPOT lists it apart"]
        );
        assert_eq!(be.call_count(), 2);
        let again = top_brokers(&f, sunday).await.unwrap();
        assert!(again.cached);
        assert_eq!(be.call_count(), 2, "both pages from the cache");

        // Read in a trading day's hours, the two pages can be on different
        // sessions: the ranking cannot be dated.
        for during in [
            "2026-11-06T08:58:00",
            "2026-11-06T10:00:00",
            "2026-11-06T16:15:00",
            "2026-11-06T18:59:59",
        ] {
            let (f, _be, _d) = fetcher(ranking_pages());
            let err = top_brokers(&f, at(during)).await.unwrap_err();
            assert_eq!(err.code(), "provider");
            assert!(
                err.to_string().contains("so it cannot be dated"),
                "{during}: {err}"
            );
        }
        let (f, _be, _d) = fetcher(ranking_pages());
        let err = top_brokers(&f, at("2026-11-06T10:00:00"))
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "ipot: the broker ranking was read at 10:00 WIB on 2026-11-06, in a trading day's hours (08:58 to 19:00 WIB), when it and the page that dates it can be on different sessions (when they roll over has not been measured), so it cannot be dated"
        );
        // After the busy hours, and on a weekday holiday: dated.
        for after in ["2026-11-06T19:00:00", "2026-12-24T10:00:00"] {
            let (f, _be, _d) = fetcher(ranking_pages());
            let s = top_brokers(&f, at(after)).await.unwrap();
            assert_eq!(s.warnings.len(), 1, "{after}: only the fold");
        }

        // Dated a session that has not opened, the trades are another's.
        for early in ["2026-11-06T08:57:59", "2026-11-05T20:00:00"] {
            let (f, _be, _d) = fetcher(ranking_pages());
            let err = top_brokers(&f, at(early)).await.unwrap_err();
            assert_eq!(err.code(), "provider");
            assert!(err.to_string().contains("disagree"), "{early}: {err}");
        }
    }

    #[tokio::test]
    async fn the_ranking_and_its_date_are_read_together() {
        let ago = |mins: i64| chrono::Utc::now() - chrono::Duration::minutes(mins);
        let seed = |cache: &Cache, top: i64, statistic: i64| {
            cache
                .put_fetched_at(TOP_BROKERS_ENDPOINT, 200, TOP, ago(top))
                .unwrap();
            cache
                .put_fetched_at(SESSION_ENDPOINT, 200, STATISTIC, ago(statistic))
                .unwrap();
        };
        let sunday = at("2026-11-08T10:00:00");

        // Two copies, both within six hours but read an hour apart: both
        // are read again.
        let (f, be, dir) = fetcher(ranking_pages());
        seed(&Cache::new(dir.path()), 5, 65);
        let s = top_brokers(&f, sunday).await.unwrap();
        assert!(!s.cached);
        assert_eq!(be.call_count(), 2);

        // Offline they cannot be: the ranking has no date.
        let dir = tempfile::tempdir().unwrap();
        seed(&Cache::new(dir.path()), 5, 65);
        let offline = Arc::new(Fetcher::new(
            Arc::new(FixtureBackend::new()),
            Cache::new(dir.path()),
            CacheMode::Offline,
        ));
        let err = top_brokers(&offline, sunday).await.unwrap_err();
        assert_eq!(err.code(), "provider");
        assert!(err.to_string().contains("read at different times"), "{err}");

        // Copies older than six hours, and the date page cannot be read: its
        // old copy does not date the new ranking.
        let (f, _be, dir) = fetcher(FixtureBackend::new().route(TOP_BROKERS_ENDPOINT, 200, TOP));
        seed(&Cache::new(dir.path()), 7 * 60, 7 * 60);
        let err = top_brokers(&f, sunday).await.unwrap_err();
        assert_eq!(err.code(), "provider");
        assert!(err.to_string().contains("no route for"), "{err}");
    }

    /// A fetcher serving `top` and `statistic` as copies read at `read` (a
    /// WIB time): from the cache alone when `offline`, otherwise because
    /// both live fetches fail.
    fn copies_read_at(
        read: &str,
        top: &str,
        statistic: &str,
        offline: bool,
    ) -> (Arc<Fetcher>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache
            .put_fetched_at(TOP_BROKERS_ENDPOINT, 200, top, at(read))
            .unwrap();
        cache
            .put_fetched_at(SESSION_ENDPOINT, 200, statistic, at(read))
            .unwrap();
        let mode = if offline {
            CacheMode::Offline
        } else {
            CacheMode::Normal
        };
        let f = Fetcher::new(Arc::new(FixtureBackend::new()), cache, mode);
        (Arc::new(f), dir)
    }

    #[tokio::test]
    async fn a_copy_is_judged_by_when_its_pages_were_read() {
        // Read on Friday at 10:00, served on Sunday from the cache, offline
        // or because IPOT is down: still read in trading hours.
        for offline in [true, false] {
            let (f, _d) = copies_read_at("2026-11-06T10:00:00", TOP, STATISTIC, offline);
            let err = top_brokers(&f, at("2026-11-08T10:00:00"))
                .await
                .unwrap_err();
            assert!(
                err.to_string()
                    .contains("read at 10:00 WIB on 2026-11-06, in a trading day's hours"),
                "offline {offline}: {err}"
            );
        }
        // Read that evening, the same copies are the whole session.
        let (f, _d) = copies_read_at("2026-11-06T19:30:00", TOP, STATISTIC, true);
        let s = top_brokers(&f, at("2026-11-08T10:00:00")).await.unwrap();
        assert!(s.stale && s.cached);
        assert_eq!(s.data.date, d(2026, 11, 6));

        // Read at 08:57, dated that day and showing trades: refused, and
        // still refused when the same copies are served after the open.
        for served in ["2026-11-06T09:00:00", "2026-11-08T10:00:00"] {
            let (f, _d) = copies_read_at("2026-11-06T08:57:00", TOP, STATISTIC, true);
            let err = top_brokers(&f, at(served)).await.unwrap_err();
            assert!(
                err.to_string().contains(
                    "read at 08:57 WIB on 2026-11-06 is dated 2026-11-06, a session that had not opened then"
                ),
                "{served}: {err}"
            );
        }

        // Read at 08:57 and dated the session before: the whole of it.
        let thursday = STATISTIC.replacen("2026-11-06", "2026-11-05", 2);
        let (f, _d) = copies_read_at("2026-11-06T08:57:00", TOP, &thursday, true);
        let s = top_brokers(&f, at("2026-11-06T09:00:00")).await.unwrap();
        assert_eq!(s.data.date, d(2026, 11, 5));
        assert_eq!(s.warnings.len(), 1, "only the fold: {:?}", s.warnings);
    }

    #[tokio::test]
    async fn a_date_that_cannot_be_the_rankings_session_is_refused() {
        let sunday = at("2026-11-08T10:00:00");
        let empty = {
            let start = TOP.find("<tbody>").unwrap() + 7;
            format!("{}{}", &TOP[..start], &TOP[TOP.find("</tbody>").unwrap()..])
        };
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route(TOP_BROKERS_ENDPOINT, 200, &empty)
                .route(SESSION_ENDPOINT, 200, STATISTIC),
        );
        // Reset for a new session while the date page names the last one,
        // which had trades: the pages are on different sessions.
        let err = top_brokers(&f, sunday).await.unwrap_err();
        assert_eq!(err.code(), "provider");
        assert!(
            err.to_string().contains(
                "shows no trades, yet the page that dates it names 2026-11-06, a session that had ended then"
            ),
            "{err}"
        );
        // Before the open, a ranking without trades dated that day is right.
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route(TOP_BROKERS_ENDPOINT, 200, &empty)
                .route(SESSION_ENDPOINT, 200, STATISTIC),
        );
        let s = top_brokers(&f, at("2026-11-06T08:00:00")).await.unwrap();
        assert!(s.data.brokers.is_empty());

        // A Saturday is no session.
        let saturday = STATISTIC.replacen("2026-11-06", "2026-11-07", 2);
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route(TOP_BROKERS_ENDPOINT, 200, TOP)
                .route(SESSION_ENDPOINT, 200, &saturday),
        );
        let err = top_brokers(&f, sunday).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("names 2026-11-07, which is not a trading day"),
            "{err}"
        );
    }

    #[tokio::test]
    async fn a_ranking_without_its_date_or_an_http_error_is_a_provider_error() {
        let sunday = at("2026-11-08T10:00:00");
        let (f, _be, _d) = fetcher(
            FixtureBackend::new()
                .route(TOP_BROKERS_ENDPOINT, 200, TOP)
                .route(SESSION_ENDPOINT, 200, "Invalid stock code!"),
        );
        let err = top_brokers(&f, sunday).await.unwrap_err();
        assert_eq!((err.code(), err.exit_code()), ("provider", 3));
        assert!(err.to_string().contains("no Today row"), "{err}");

        let (f, _be, _d) = fetcher(FixtureBackend::new().route(TOP_BROKERS_ENDPOINT, 503, "busy"));
        let err = top_brokers(&f, sunday).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "ipot: broker ranking for the market: HTTP 503"
        );
    }
}
