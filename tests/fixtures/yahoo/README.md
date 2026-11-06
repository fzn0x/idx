# Yahoo Finance fixtures

Recorded with curl against the unofficial endpoints, IDX tickers with the `.JK`
suffix. `v7/finance/quote` and `v10/finance/quoteSummary` need the cookie +
crumb session (`providers/yahoo/session.rs`); `v8/finance/chart` and
`ws/fundamentals-timeseries` do not.

| file | request | notes |
| --- | --- | --- |
| chart_bbca_1mo_1d.json | `v8/finance/chart/BBCA.JK?range=1mo&interval=1d` | current-day bar has a null close |
| chart_tlkm_1y_1wk.json | `v8/finance/chart/TLKM.JK?range=1y&interval=1wk` | includes `adjclose` |
| chart_events_bbca_max.json | `v8/finance/chart/BBCA.JK?range=max&interval=1mo&events=div,splits` | `events.dividends` (47, keyed by unix ex-date, split-adjusted amounts) and `events.splits` (2:1 2008, 5:1 2021); recorded 2026-10-17 |
| chart_bbri_5d_1h.json | `v8/finance/chart/BBRI.JK?range=5d&interval=1h` | intraday |
| chart_not_found.json | `v8/finance/chart/ZZZZ.JK` | HTTP 404 |
| chart_bbca_1d_1m.json | `v8/finance/chart/BBCA.JK?range=1d&interval=1m&includePrePost=false` | recorded 2026-11-06 06:57 WIB, before the open, so it is the 11-05 session: 435 rows 09:00 to 16:14, 333 with prices; the lunch break (12:00 to 13:29) and quiet minutes are null rows; the 09:00 bar has volume 0 (the opening auction's ~11.2M shares are in no bar: the bars sum to 155,142,100 against the day's 166,336,900); the pre-closing auction lands in the 16:00 bar |
| chart_jkse_1d_1m.json | `v8/finance/chart/^JKSE?range=1d&interval=1m&includePrePost=false` | IHSG, same session: 320 bars 09:00 to 16:00, volume 0 throughout (Yahoo has no index volume) |
| chart_bbca_1d_1m_open_empty.json | `v8/finance/chart/BBCA.JK?range=1d&interval=1m&includePrePost=false` | recorded 2026-11-06 09:00:40 WIB, just after the open: HTTP 200 with `meta` only, no `timestamp` and `indicators.quote` `[{}]`; the 1-day window stayed empty past 09:05 while the previous session had dropped out of it |
| chart_bbca_5d_1m_open.json | same with `range=5d` | recorded 2026-11-06 09:02:50 WIB: 10-30 is gone, 11-02 to 11-05 remain, nothing for 11-06 yet; the 11-05 session matches `chart_bbca_1d_1m.json` bar for bar plus one row at 16:14:43 (the last trade time, volume 0) |
| chart_bbca_1mo_1m_error.json | `v8/finance/chart/BBCA.JK?range=1mo&interval=1m` | HTTP 422 `Unprocessable Entity`: "Only 8 days worth of 1m granularity data are allowed to be fetched per request." (2m allows 60 days) |
| quote_v7_bbca_bbri_tlkm.json | `v7/finance/quote?symbols=BBCA.JK,BBRI.JK,TLKM.JK` | full quote rows |
| quote_v7_partial_bbca_zzzz.json | `v7/finance/quote?symbols=BBCA.JK,ZZZZ.JK` | unknown symbol silently omitted |
| quote_v7_unauthorized.json | `v7/finance/quote` without crumb | HTTP 401 |
| quote_summary_bbca.json | `quoteSummary/BBCA.JK?modules=assetProfile,summaryDetail,defaultKeyStatistics,financialData` | profile + ratios (recorded 2026-10-16) |
| quote_summary_statements_bbca.json | `quoteSummary/BBCA.JK?modules=incomeStatementHistory,...,earnings,earningsHistory,earningsTrend,calendarEvents,price,recommendationTrend` | statement modules are gutted (only `endDate`); earnings modules are populated (2026-10-17) |
| quote_summary_fundamentals_bbca.json | `quoteSummary/BBCA.JK?modules=assetProfile,summaryDetail,defaultKeyStatistics,financialData,price` | bank: no EBITDA, no current ratio, `grossMargins` 0 |
| quote_summary_fundamentals_tlkm.json | same for TLKM.JK | non-bank: debtToEquity (percent), currentRatio, enterpriseToEbitda present |
| quote_summary_earnings_tlkm.json | `quoteSummary/TLKM.JK?modules=earnings,earningsHistory,earningsTrend,calendarEvents,price` | earnings for a non-bank |
| quote_summary_not_found.json | `quoteSummary/ZZZZ.JK` | HTTP 404, `error.code` "Not Found" |
| timeseries_bbca.json | `ws/fundamentals-timeseries/v1/finance/timeseries/BBCA.JK?type=annual...,quarterly...,trailing...` | 4 annual years, ~5 quarters, TTM; bank: no EBITDA/GrossProfit/OperatingIncome series |
| timeseries_tlkm.json | same for TLKM.JK | non-bank: EBITDA, EBIT, GrossProfit, OperatingIncome, NetDebt present |
| timeseries_not_found.json | same for ZZZZ.JK | HTTP 200, series with `meta` only and no data |
