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
| chart_tlkm_1y_1d.json | `v8/finance/chart/TLKM.JK?range=1y&interval=1d&includePrePost=false` | a year of daily bars (last 2026-11-05) behind the risk lens; recorded 2026-11-06 (see below) |
| chart_jkse_1y_1d.json | same for `^JKSE` (IHSG, no `.JK`) | the risk lens's benchmark; its last bar is 2026-11-04, a session behind the shares, so the lens aligns on shared dates; recorded 2026-11-06 |
| chart_bbca_1y_1d.json | same for BBCA.JK | bank risk case; recorded 2026-11-06 |
| chart_giaa_1y_1d.json | same for GIAA.JK | high-risk case (volatility 2.3x IHSG's, deeper fall); recorded 2026-11-06 |
| chart_itmg_1y_1d.json | same for ITMG.JK | low-risk case (beta about 0.34); recorded 2026-11-06 |
| chart_bbri_5d_1h.json | `v8/finance/chart/BBRI.JK?range=5d&interval=1h` | intraday |
| chart_not_found.json | `v8/finance/chart/ZZZZ.JK` | HTTP 404 |
| quote_v7_bbca_bbri_tlkm.json | `v7/finance/quote?symbols=BBCA.JK,BBRI.JK,TLKM.JK` | full quote rows |
| quote_v7_partial_bbca_zzzz.json | `v7/finance/quote?symbols=BBCA.JK,ZZZZ.JK` | unknown symbol silently omitted |
| quote_v7_unauthorized.json | `v7/finance/quote` without crumb | HTTP 401 |
| quote_summary_bbca.json | `quoteSummary/BBCA.JK?modules=assetProfile,summaryDetail,defaultKeyStatistics,financialData` | profile + ratios (recorded 2026-10-16) |
| quote_summary_statements_bbca.json | `quoteSummary/BBCA.JK?modules=incomeStatementHistory,...,earnings,earningsHistory,earningsTrend,calendarEvents,price,recommendationTrend` | statement modules are gutted (only `endDate`); earnings modules are populated (2026-10-17) |
| quote_summary_fundamentals_bbca.json | `quoteSummary/BBCA.JK?modules=assetProfile,summaryDetail,defaultKeyStatistics,financialData,price` | bank: no EBITDA, no current ratio, `grossMargins` 0 |
| quote_summary_fundamentals_tlkm.json | same for TLKM.JK | non-bank: debtToEquity (percent), currentRatio, enterpriseToEbitda present |
| quote_summary_fundamentals_giaa.json | same for GIAA.JK | negative equity (recorded 2026-11-06 through idx 0.8.1's fetcher, raw body): `debtToEquity`, `priceToBook` and `returnOnEquity` are `{}` while `bookValue` is -0.001; `financialCurrency` USD, so per-share book value and revenue are USD against an IDR price |
| quote_summary_fundamentals_itmg.json | same for ITMG.JK | USD reporter (recorded 2026-11-06, same way): `financialCurrency` USD, price IDR; `priceToBook` 14103.308, `priceToSalesTrailing12Months` 13807.103, `enterpriseToEbitda` 91203.72 divide the IDR price by USD figures, while `trailingPE` 7.46 and `trailingEps` 3258.87 are consistent |
| quote_summary_earnings_tlkm.json | `quoteSummary/TLKM.JK?modules=earnings,earningsHistory,earningsTrend,calendarEvents,price` | earnings for a non-bank |
| quote_summary_not_found.json | `quoteSummary/ZZZZ.JK` | HTTP 404, `error.code` "Not Found" |
| timeseries_bbca.json | `ws/fundamentals-timeseries/v1/finance/timeseries/BBCA.JK?type=annual...,quarterly...,trailing...` | 4 annual years, ~5 quarters, TTM; bank: no EBITDA/GrossProfit/OperatingIncome series |
| timeseries_tlkm.json | same for TLKM.JK | non-bank: EBITDA, EBIT, GrossProfit, OperatingIncome, NetDebt present |
| timeseries_annual_giaa.json | `ws/fundamentals-timeseries/...` for GIAA.JK, `type=annual...` (the `annual` item list idx sends) | USD statements; net income 3.74e9 in FY2022 to -3.22e8 in FY2025 (profit to loss); recorded 2026-11-06 |
| timeseries_annual_itmg.json | same for ITMG.JK | USD statements; revenue fell in each of FY2023 to FY2025; recorded 2026-11-06 |
| timeseries_not_found.json | same for ZZZZ.JK | HTTP 200, series with `meta` only and no data |

The 1y daily charts and the annual timeseries files were recorded 2026-11-06
between 06:50 and 07:05 WIB (before the open) through idx 0.8.1's own fetcher with
an isolated cache, and copied byte for byte from that cache. The timeseries
request also carries `period1`/`period2` (six years back to tomorrow), which are
not part of the cache key.
