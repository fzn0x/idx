//! Reads from the user's Stockbit session that take several requests: one
//! stock's corporate actions or the market's calendar (one request per kind),
//! and pages of insider and 5 % holder movements.
//! Shared by the CLI and the MCP server like the rest of `service`.
//!
//! The provider paces these requests a second apart (cached answers cost no
//! pause) and stops at the first error no later request can get past (no
//! session, a refused session, a CDN block).

use chrono::NaiveDate;

use crate::error::{IdxError, Result};
use crate::model::{CorporateEvent, CorporateEventKind, CorporateEvents, HolderMovements, Symbol};
use crate::providers::stockbit::movement::MovementQuery;
use crate::providers::stockbit::{self, Stockbit};
use crate::providers::Sourced;
use crate::universe::Issuer;

/// How far ahead `stockbit calendar` looks when no end is given.
pub const STOCKBIT_CALENDAR_DEFAULT_DAYS: u32 = 14;
/// The longest calendar range, in days.
pub const STOCKBIT_CALENDAR_MAX_DAYS: i64 = 366;

/// Every kind's rows, one request per kind in order. A kind that fails
/// leaves a warning; when every kind fails, the first error is returned.
async fn kinds_of(
    sb: &Stockbit,
    kinds: &[CorporateEventKind],
    symbol: Option<&Symbol>,
) -> Result<Sourced<Vec<CorporateEvent>>> {
    let mut out: Option<Sourced<Vec<CorporateEvent>>> = None;
    let mut first_err: Option<IdxError> = None;
    let mut warnings = Vec::new();
    for &kind in kinds {
        match sb.corporate_actions(kind, symbol).await {
            Ok(s) => match &mut out {
                None => out = Some(s),
                Some(o) => {
                    o.absorb(&s);
                    o.data.extend(s.data);
                }
            },
            Err(e) if stockbit::is_fatal(&e) => {
                if out.is_none() {
                    return Err(e);
                }
                warnings.push(format!("{kind} and the kinds after it: {e}"));
                break;
            }
            Err(e) => {
                warnings.push(format!("{kind}: {e}"));
                first_err.get_or_insert(e);
            }
        }
    }
    let mut out = match (out, first_err) {
        (Some(o), _) => o,
        (None, Some(e)) => return Err(e),
        (None, None) => return Err(IdxError::Usage("no corporate action kinds given".into())),
    };
    for w in warnings {
        if !out.warnings.contains(&w) {
            out.warnings.push(w);
        }
    }
    Ok(out)
}

/// The most pages of movements one call reads.
pub const STOCKBIT_MOVEMENT_MAX_PAGES: u32 = 10;

/// Pages 1..=`pages` of holder movements, one at a time, stopping at the
/// last page. A row repeated across pages (the feed gains rows at the top
/// while it is paged) is kept once, and a page cached before the page above
/// it is read again live. A later page that fails keeps the rows read so
/// far, with `more` and a warning; a failing first page is the error.
pub async fn stockbit_movements(
    sb: &Stockbit,
    q: &MovementQuery,
    pages: u32,
) -> Result<Sourced<HolderMovements>> {
    q.validate()?;
    if !(1..=STOCKBIT_MOVEMENT_MAX_PAGES).contains(&pages) {
        return Err(IdxError::Usage(format!(
            "pages must be 1 to {STOCKBIT_MOVEMENT_MAX_PAGES}, not {pages}"
        )));
    }
    let mut out = sb
        .movements_page(q, 1)
        .await?
        .map(|(rows, more)| HolderMovements {
            symbol: q.symbol.clone(),
            from: q.from,
            to: q.to,
            holder_id: q.holder_id.clone(),
            action: q.action,
            source: q.source,
            pages_read: 1,
            more,
            movements: rows,
        });
    let key = |m: &crate::model::HolderMovement| {
        (
            m.holder_id.clone(),
            m.holder.clone(),
            m.symbol.clone(),
            m.date,
            m.action.clone(),
            m.shares_from,
            m.shares_to,
        )
    };
    let mut seen: std::collections::HashSet<_> = out.data.movements.iter().map(key).collect();
    // When the page above was read. A page cached before it is an older
    // snapshot: rows that have since moved down past the page break would be
    // on neither page, so such a page is read again live.
    let mut above_at = out.fetched_at;
    let mut page = 2;
    while out.data.more && page <= pages {
        let read = match sb.movements_page(q, page).await {
            Ok(s) if s.cached && s.fetched_at < above_at => {
                sb.movements_page_live(q, page).await.or(Ok(s))
            }
            other => other,
        };
        match read {
            Ok(s) => {
                if s.fetched_at < above_at {
                    out.warnings.push(format!(
                        "page {page} is an older copy than page {} (it could not be read again); rows between them may be missing",
                        page - 1
                    ));
                }
                above_at = s.fetched_at;
                out.absorb(&s);
                let (rows, more) = s.data;
                for r in rows {
                    if seen.insert(key(&r)) {
                        out.data.movements.push(r);
                    }
                }
                out.data.pages_read = page;
                out.data.more = more;
            }
            Err(e) => {
                out.warnings.push(format!(
                    "page {page} failed ({e}); showing pages 1-{}",
                    page - 1
                ));
                out.data.more = true;
                break;
            }
        }
        page += 1;
    }
    Ok(out)
}

/// One stock's corporate actions of `kinds` (newest first), optionally only
/// those dated `from..=to`.
pub async fn stockbit_actions(
    sb: &Stockbit,
    universe: &[Issuer],
    symbol: &Symbol,
    kinds: &[CorporateEventKind],
    from: Option<NaiveDate>,
    to: Option<NaiveDate>,
) -> Result<Sourced<CorporateEvents>> {
    if symbol.is_index() {
        return Err(IdxError::Usage(format!(
            "{symbol} is an index; corporate actions belong to stocks"
        )));
    }
    if let (Some(f), Some(t)) = (from, to) {
        if f > t {
            return Err(IdxError::Usage(format!(
                "the range starts ({f}) after it ends ({t})"
            )));
        }
    }
    let mut s = kinds_of(sb, kinds, Some(symbol)).await?;
    // A structured warrant on the stock carries its own code, which starts
    // with the stock's; anything else is not this stock's.
    let before = s.data.len();
    s.data
        .retain(|e| e.symbol.as_str().starts_with(symbol.as_str()));
    if s.data.len() < before {
        s.warnings.push(format!(
            "left out {} rows Stockbit returned for other codes",
            before - s.data.len()
        ));
    }
    let total = s.data.len();
    s.data
        .retain(|e| from.is_none_or(|f| e.date >= f) && to.is_none_or(|t| e.date <= t));
    s.data
        .sort_by(|a, b| b.date.cmp(&a.date).then(a.kind.cmp(&b.kind)));
    let name = universe
        .iter()
        .find(|i| &i.symbol == symbol)
        .and_then(|i| i.name.clone());
    Ok(s.map(|actions| CorporateEvents {
        symbol: Some(symbol.clone()),
        name,
        from,
        to,
        kinds: kinds.to_vec(),
        total,
        actions,
    }))
}

/// What `stockbit calendar` asks for.
#[derive(Debug, Clone, Default)]
pub struct StockbitCalendarQuery {
    /// First day; default today (WIB).
    pub from: Option<NaiveDate>,
    /// Last day; default `from` plus `days`.
    pub to: Option<NaiveDate>,
    /// Look-ahead when `to` is not given; default
    /// [`STOCKBIT_CALENDAR_DEFAULT_DAYS`].
    pub days: Option<u32>,
    /// Kinds to read; empty means [`CorporateEventKind::CALENDAR_DEFAULT`].
    pub kinds: Vec<CorporateEventKind>,
}

/// `true` when the row belongs in `from..=to`: dated inside it, or a rights
/// trading window, a tender offer or an IPO offering still open in it.
/// Warrants count by their date only (their windows run for years).
fn in_range(e: &CorporateEvent, from: NaiveDate, to: NaiveDate) -> bool {
    use CorporateEventKind as K;
    if e.date >= from && e.date <= to {
        return true;
    }
    let window = match e.kind {
        K::Rights => (e.trading_start, e.trading_end),
        K::TenderOffer | K::Ipo => (e.offer_start, e.offer_end),
        _ => return false,
    };
    match window {
        (Some(s), Some(t)) => s <= to && t >= from,
        _ => false,
    }
}

/// The market's corporate actions of `q.kinds` in a date range, oldest first.
pub async fn stockbit_calendar(
    sb: &Stockbit,
    q: &StockbitCalendarQuery,
    today: NaiveDate,
) -> Result<Sourced<CorporateEvents>> {
    let from = q.from.unwrap_or(today);
    if q.to.is_some() && q.days.is_some() {
        return Err(IdxError::Usage(
            "give either an end date (to) or a number of days, not both".into(),
        ));
    }
    let days = q.days.unwrap_or(STOCKBIT_CALENDAR_DEFAULT_DAYS);
    if i64::from(days) > STOCKBIT_CALENDAR_MAX_DAYS {
        return Err(IdxError::Usage(format!(
            "a calendar range covers at most {STOCKBIT_CALENDAR_MAX_DAYS} days, not {days}"
        )));
    }
    let to = match q.to {
        Some(t) => t,
        // Checked: `NaiveDate + Days` panics past the last representable day.
        None => from
            .checked_add_days(chrono::Days::new(u64::from(days)))
            .ok_or_else(|| IdxError::Usage(format!("{from} plus {days} days is out of range")))?,
    };
    if from > to {
        return Err(IdxError::Usage(format!(
            "the range starts ({from}) after it ends ({to})"
        )));
    }
    if (to - from).num_days() > STOCKBIT_CALENDAR_MAX_DAYS {
        return Err(IdxError::Usage(format!(
            "a calendar range covers at most {STOCKBIT_CALENDAR_MAX_DAYS} days"
        )));
    }
    let kinds = if q.kinds.is_empty() {
        CorporateEventKind::CALENDAR_DEFAULT.to_vec()
    } else {
        q.kinds.clone()
    };
    let mut s = kinds_of(sb, &kinds, None).await?;
    let total = s.data.len();
    s.data.retain(|e| in_range(e, from, to));
    s.data.sort_by(|a, b| {
        a.date
            .cmp(&b.date)
            .then_with(|| a.symbol.cmp(&b.symbol))
            .then(a.kind.cmp(&b.kind))
    });
    Ok(s.map(|actions| CorporateEvents {
        symbol: None,
        name: None,
        from: Some(from),
        to: Some(to),
        kinds,
        total,
        actions,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::FixtureBackend;
    use crate::providers::stockbit::tests::with_backend;
    use std::time::Duration;
    use CorporateEventKind as K;

    fn fx(name: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/stockbit/{name}.json",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn sym(s: &str) -> Symbol {
        Symbol::parse(s).unwrap()
    }

    fn sb(be: FixtureBackend) -> (Stockbit, std::sync::Arc<FixtureBackend>, tempfile::TempDir) {
        let (sb, be, dir) = with_backend(be);
        (sb.with_pace(Duration::ZERO), be, dir)
    }

    #[tokio::test]
    async fn a_stocks_kinds_merge_newest_first() {
        let (sb, be, _d) = sb(FixtureBackend::new()
            .route(
                "corpaction/dividend?symbol=BBCA",
                200,
                &fx("corpaction_dividend_bbca"),
            )
            .route(
                "corpaction/stocksplit?symbol=BBCA",
                200,
                &fx("corpaction_stocksplit_bbca"),
            ));
        let universe = [Issuer {
            symbol: sym("BBCA"),
            name: Some("Bank Central Asia Tbk".into()),
        }];
        let got = stockbit_actions(
            &sb,
            &universe,
            &sym("BBCA"),
            &[K::Split, K::Dividend],
            None,
            None,
        )
        .await
        .unwrap();
        let ev = &got.data;
        assert_eq!(ev.total, 40);
        assert_eq!(ev.actions.len(), 40);
        assert_eq!(ev.name.as_deref(), Some("Bank Central Asia Tbk"));
        assert_eq!(ev.kinds, [K::Split, K::Dividend]);
        assert!(ev.actions.windows(2).all(|w| w[0].date >= w[1].date));
        assert_eq!(ev.actions[0].date, d("2026-08-28"));
        assert_eq!(got.provider, "stockbit");
        assert!(!got.cached);
        // The bearer went out; nothing was refreshed.
        assert_eq!(be.calls_matching("corpaction/"), 2);
        assert_eq!(be.calls_matching("login/refresh"), 0);
        let bearer = be.headers.lock().unwrap()[0]
            .iter()
            .any(|(k, v)| k == "authorization" && v.starts_with("Bearer "));
        assert!(bearer);

        // A date range keeps total and filters the rows.
        let got = stockbit_actions(
            &sb,
            &universe,
            &sym("BBCA"),
            &[K::Split, K::Dividend],
            Some(d("2021-01-01")),
            Some(d("2021-12-31")),
        )
        .await
        .unwrap();
        assert_eq!(got.data.total, 40);
        assert!(got
            .data
            .actions
            .iter()
            .all(|e| e.date.format("%Y").to_string() == "2021"));
        assert!(got.data.actions.iter().any(|e| e.kind == K::Split));
        // Second time round both kinds come from the cache: no request.
        assert_eq!(be.calls_matching("corpaction/"), 2);
        assert!(got.cached);
    }

    #[tokio::test]
    async fn a_failing_kind_is_a_warning_and_all_failing_an_error() {
        let (sb, _be, _d) = sb(FixtureBackend::new()
            .route(
                "corpaction/dividend?symbol=BBCA",
                200,
                &fx("corpaction_dividend_bbca"),
            )
            .route("corpaction/rups?symbol=BBCA", 500, r#"{"message":"boom"}"#));
        let got = stockbit_actions(
            &sb,
            &[],
            &sym("BBCA"),
            &[K::Dividend, K::Rups, K::Bonus],
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(got.data.actions.len(), 37);
        assert!(
            got.warnings
                .iter()
                .any(|w| w.starts_with("rups: ") && w.contains("HTTP 500")),
            "{:?}",
            got.warnings
        );
        assert!(
            got.warnings.iter().any(|w| w.starts_with("bonus: ")),
            "{:?}",
            got.warnings
        );

        let e = stockbit_actions(&sb, &[], &sym("BBCA"), &[K::Rups], None, None)
            .await
            .unwrap_err();
        assert_eq!(e.exit_code(), 3);
        let e = stockbit_actions(&sb, &[], &sym("^JKSE"), &[K::Rups], None, None)
            .await
            .unwrap_err();
        assert_eq!(e.exit_code(), 2);
        let e = stockbit_actions(
            &sb,
            &[],
            &sym("BBCA"),
            &[K::Rups],
            Some(d("2026-02-01")),
            Some(d("2026-01-01")),
        )
        .await
        .unwrap_err();
        assert_eq!(e.exit_code(), 2);
    }

    #[tokio::test]
    async fn a_cdn_block_stops_at_the_first_kind() {
        let blocked = std::fs::read_to_string(format!(
            "{}/tests/fixtures/stockbit/blocked_403.html",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let (sb, be, _d) = sb(FixtureBackend::new().route("corpaction/", 403, &blocked));
        let e = stockbit_calendar(&sb, &StockbitCalendarQuery::default(), d("2026-11-03"))
            .await
            .unwrap_err();
        assert!(e.to_string().contains("CDN"), "{e}");
        assert_eq!(be.calls_matching("corpaction/"), 1);
    }

    #[tokio::test]
    async fn a_cdn_block_on_refresh_stops_and_keeps_the_token() {
        let blocked = std::fs::read_to_string(format!(
            "{}/tests/fixtures/stockbit/blocked_403.html",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let be = std::sync::Arc::new(
            FixtureBackend::new()
                .route("login/refresh", 403, &blocked)
                .route("corpaction/", 200, &fx("corpaction_dividend")),
        );
        let fetcher = std::sync::Arc::new(crate::http::Fetcher::new(
            be.clone(),
            crate::cache::Cache::new(dir.path().join("cache")),
            crate::http::CacheMode::Normal,
        ));
        // Only a refresh token: every request needs a refresh first.
        let store = std::sync::Arc::new(crate::providers::stockbit::SessionStore::new(
            dir.path().join("session.json"),
            None,
            Some("eyJ0.eyJleHAiOjF9.dummy".into()),
        ));
        let sb = Stockbit::new(fetcher, Default::default(), store).with_pace(Duration::ZERO);
        for n in 1..=2 {
            let e = stockbit_calendar(&sb, &StockbitCalendarQuery::default(), d("2026-11-03"))
                .await
                .unwrap_err();
            assert!(e.to_string().contains("HTTP 403 from its CDN"), "{e}");
            assert!(!e.to_string().contains("rejected"), "{e}");
            // One refresh per command, no kind after it; the token is not
            // taken for refused, so the next command tries again.
            assert_eq!(be.calls_matching("login/refresh"), n);
            assert_eq!(be.calls_matching("corpaction/"), 0);
        }
    }

    #[tokio::test]
    async fn without_a_session_nothing_is_sent() {
        let dir = tempfile::tempdir().unwrap();
        let be = std::sync::Arc::new(FixtureBackend::new());
        let fetcher = std::sync::Arc::new(crate::http::Fetcher::new(
            be.clone(),
            crate::cache::Cache::new(dir.path().join("cache")),
            crate::http::CacheMode::Normal,
        ));
        let store = std::sync::Arc::new(crate::providers::stockbit::SessionStore::new(
            dir.path().join("none.json"),
            None,
            None,
        ));
        let sb = Stockbit::new(fetcher, Default::default(), store);
        let e = stockbit_actions(&sb, &[], &sym("BBCA"), &K::STOCK_DEFAULT, None, None)
            .await
            .unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().contains("stockbit login"), "{e}");
        assert_eq!(be.call_count(), 0);
    }

    #[tokio::test]
    async fn the_calendar_keeps_open_offers_and_sorts_oldest_first() {
        let (sb, _be, _d) = sb(FixtureBackend::new()
            .route("corpaction/tenderoffer", 200, &fx("corpaction_tenderoffer"))
            .route("corpaction/warrant", 200, &fx("corpaction_warrant"))
            .route("corpaction/rightissue", 200, &fx("corpaction_rightissue"))
            .route("corpaction/dividend", 200, &fx("corpaction_dividend")));
        let q = StockbitCalendarQuery {
            from: Some(d("2026-11-05")),
            to: Some(d("2026-11-12")),
            days: None,
            kinds: vec![K::TenderOffer, K::Warrant, K::Rights, K::Dividend],
        };
        let got = stockbit_calendar(&sb, &q, d("2026-11-03")).await.unwrap();
        let ev = &got.data;
        assert_eq!(
            (ev.from, ev.to),
            (Some(d("2026-11-05")), Some(d("2026-11-12")))
        );
        assert!(ev.symbol.is_none());
        assert!(ev.total > ev.actions.len());
        assert!(ev.actions.windows(2).all(|w| w[0].date <= w[1].date));
        // AUTO's tender offer opened 2026-10-13 and runs to 2026-11-11.
        let auto = ev
            .actions
            .iter()
            .find(|e| e.symbol.as_str() == "AUTO")
            .unwrap();
        assert_eq!(auto.date, d("2026-10-13"));
        // MGLV's rights trade 2026-11-10..23: inside by its window.
        assert!(ev
            .actions
            .iter()
            .any(|e| e.symbol.as_str() == "MGLV" && e.kind == K::Rights));
        // BRNA's warrants trade from 2026-10-16 until 2029: dated before the
        // range, so out despite the open window.
        assert!(!ev.actions.iter().any(|e| e.symbol.as_str() == "BRNA"));
        // MIDI's dividend cum date 2026-11-11 is inside.
        assert!(ev.actions.iter().any(|e| e.symbol.as_str() == "MIDI"));
    }

    #[tokio::test]
    async fn calendar_ranges() {
        let (sb, be, _d) =
            sb(FixtureBackend::new().route("corpaction/dividend", 200, &fx("corpaction_dividend")));
        let one = |from, to, days| StockbitCalendarQuery {
            from,
            to,
            days,
            kinds: vec![K::Dividend],
        };
        let got = stockbit_calendar(&sb, &one(None, None, None), d("2026-11-03"))
            .await
            .unwrap();
        assert_eq!(
            (got.data.from, got.data.to),
            (Some(d("2026-11-03")), Some(d("2026-11-17")))
        );
        let got = stockbit_calendar(&sb, &one(None, None, Some(30)), d("2026-11-03"))
            .await
            .unwrap();
        assert_eq!(got.data.to, Some(d("2026-12-03")));
        for bad in [
            one(Some(d("2026-11-03")), Some(d("2026-11-02")), None),
            one(Some(d("2026-01-01")), Some(d("2027-01-03")), None),
            one(None, None, Some(367)),
            one(None, Some(d("2026-12-01")), Some(5)),
            // Past the last date chrono can hold: a usage error, not a panic.
            one(None, None, Some(u32::MAX)),
            one(Some(NaiveDate::MAX), None, None),
            one(Some(NaiveDate::MAX), None, Some(1)),
        ] {
            let e = stockbit_calendar(&sb, &bad, d("2026-11-03"))
                .await
                .unwrap_err();
            assert_eq!(e.exit_code(), 2, "{bad:?}");
        }
        // 366 days is fine; the defaults read every kind but warrants.
        stockbit_calendar(
            &sb,
            &one(Some(d("2026-01-01")), Some(d("2027-01-02")), None),
            d("2026-11-03"),
        )
        .await
        .unwrap();
        let got = stockbit_calendar(&sb, &StockbitCalendarQuery::default(), d("2026-11-03"))
            .await
            .unwrap();
        assert_eq!(got.data.kinds, K::CALENDAR_DEFAULT);
        assert_eq!(be.calls_matching("corpaction/warrant"), 0);
    }

    const P1: &str = "majorholder?page=1";
    const P2: &str = "majorholder?page=2";

    #[tokio::test]
    async fn movements_read_pages_until_the_last() {
        let (sb, be, _d) = sb(FixtureBackend::new()
            .route(P1, 200, &fx("majorholder"))
            .route(P2, 200, &fx("majorholder_p2")));
        let got = stockbit_movements(&sb, &MovementQuery::default(), 5)
            .await
            .unwrap();
        let m = &got.data;
        assert_eq!((m.pages_read, m.more), (2, false));
        assert_eq!(m.movements.len(), 14);
        assert_eq!(be.calls_matching("majorholder?page="), 2);
        assert!(got.warnings.is_empty(), "{:?}", got.warnings);
        // One page asked for: the first, with more to come.
        let got = stockbit_movements(&sb, &MovementQuery::default(), 1)
            .await
            .unwrap();
        assert_eq!((got.data.pages_read, got.data.more), (1, true));
        assert_eq!(got.data.movements.len(), 8);
        assert!(got.cached);
    }

    #[tokio::test]
    async fn movements_drop_rows_repeated_across_pages() {
        // The feed gained rows while it was paged: page 2 repeats page 1.
        let (sb, _be, _d) = sb(FixtureBackend::new()
            .route(P1, 200, &fx("majorholder"))
            .route(P2, 200, &fx("majorholder")));
        let got = stockbit_movements(&sb, &MovementQuery::default(), 2)
            .await
            .unwrap();
        assert_eq!(got.data.movements.len(), 8);
        assert_eq!((got.data.pages_read, got.data.more), (2, true));
    }

    #[tokio::test]
    async fn a_later_page_cached_before_the_one_above_is_read_again() {
        let q = MovementQuery::default();
        let p2_url = crate::providers::stockbit::movement::url(&q, 2);
        // Page 2 sits in the cache from an earlier snapshot; page 1 is live.
        let (p, be, _d) = sb(FixtureBackend::new()
            .route(P1, 200, &fx("majorholder"))
            .route(P2, 200, &fx("majorholder_p2")));
        p.fetcher()
            .cache()
            .put(&p2_url, 200, &fx("majorholder_p2"))
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let got = stockbit_movements(&p, &q, 2).await.unwrap();
        assert_eq!(be.calls_matching(P1), 1);
        assert_eq!(be.calls_matching(P2), 1, "page 2 read again live");
        assert_eq!(got.data.movements.len(), 14);
        assert!(!got.cached);
        assert!(got.warnings.is_empty(), "{:?}", got.warnings);
        // Both cached now, page 2 after page 1: nothing is read again.
        let got = stockbit_movements(&p, &q, 2).await.unwrap();
        assert!(got.cached);
        assert_eq!(be.call_count(), 2);

        // When page 2 cannot be read again, the older copy is kept and the
        // gap is named.
        let (p, be, _d) = sb(FixtureBackend::new().route(P1, 200, &fx("majorholder")));
        p.fetcher()
            .cache()
            .put(&p2_url, 200, &fx("majorholder_p2"))
            .unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let got = stockbit_movements(&p, &q, 2).await.unwrap();
        assert_eq!(be.calls_matching(P2), 1);
        assert_eq!(got.data.movements.len(), 14);
        assert_eq!((got.data.pages_read, got.data.more), (2, false));
        assert!(
            got.warnings
                .iter()
                .any(|w| w.starts_with("page 2 is an older copy than page 1")),
            "{:?}",
            got.warnings
        );
    }

    #[tokio::test]
    async fn a_failing_later_page_keeps_the_rows_read() {
        let (sb, _be, _d) = sb(FixtureBackend::new().route(P1, 200, &fx("majorholder_bbca")));
        let q = MovementQuery {
            symbol: Some(sym("BBCA")),
            ..Default::default()
        };
        let got = stockbit_movements(&sb, &q, 3).await.unwrap();
        assert_eq!(got.data.movements.len(), 8);
        assert_eq!((got.data.pages_read, got.data.more), (1, true));
        assert_eq!(got.data.symbol, Some(sym("BBCA")));
        assert!(
            got.warnings
                .iter()
                .any(|w| w.starts_with("page 2 failed (") && w.ends_with("showing pages 1-1")),
            "{:?}",
            got.warnings
        );
        // A failing first page is the error.
        let (sb, _be, _d) = sb_empty();
        assert_eq!(
            stockbit_movements(&sb, &q, 1)
                .await
                .unwrap_err()
                .exit_code(),
            3
        );
        for (q, pages) in [
            (
                MovementQuery {
                    holder_id: Some("RUDI".into()),
                    ..Default::default()
                },
                1,
            ),
            (MovementQuery::default(), 0),
            (MovementQuery::default(), 11),
        ] {
            let e = stockbit_movements(&sb, &q, pages).await.unwrap_err();
            assert_eq!(e.exit_code(), 2, "{q:?} {pages}");
        }
    }

    fn sb_empty() -> (Stockbit, std::sync::Arc<FixtureBackend>, tempfile::TempDir) {
        sb(FixtureBackend::new())
    }
}
