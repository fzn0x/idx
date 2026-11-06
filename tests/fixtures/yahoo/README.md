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
| chart_jkse_range_max_1d.json | `v8/finance/chart/%5EJKSE?range=max&interval=1d` | the bug behind `--period max`: `dataGranularity` "1mo", 440 monthly rows from 1990-05 although 1d was asked for (recorded 2026-11-06 06:57 WIB) |
| chart_jkse_period1_max_1d.json | `v8/finance/chart/%5EJKSE?period1=0&period2=9999999999&interval=1d` | what idx asks for now: 9,221 daily rows from 1990-04-06, 8,905 with prices (holidays and the unfinished last session are null); 1990-1995 rows are close-only (open = high = low = close, volume 0) (2026-11-06 06:57 WIB) |
| chart_jkse_2y_1d.json | `v8/finance/chart/%5EJKSE?range=2y&interval=1d` | 475 daily rows 2024-11-05 to 2026-11-05, 474 with prices (the last close is null); the `market astro --backtest --period 2y` fixture behind the literals in `src/analysis/astro.rs` and `web/test/astro.test.mjs` (2026-11-06 06:58 WIB) |
| chart_not_found.json | `v8/finance/chart/ZZZZ.JK` | HTTP 404 |
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
