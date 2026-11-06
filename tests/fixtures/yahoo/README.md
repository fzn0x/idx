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
| chart_jkse_range_max_1d.json | `v8/finance/chart/%5EJKSE?range=max&interval=1d` | the bug behind `--period max`: `dataGranularity` "1mo", 440 monthly rows from 1990-05 although 1d was asked for (recorded 2026-11-06 06:57 WIB) |
| chart_jkse_period1_max_1d.json | `v8/finance/chart/%5EJKSE?period1=0&period2=9999999999&interval=1d` | what idx asks for now: 9,221 daily rows from 1990-04-06, 8,905 with prices (holidays and the unfinished last session are null); 1990-1995 rows are close-only (open = high = low = close, volume 0) (2026-11-06 06:57 WIB) |
| chart_jkse_2y_1d.json | `v8/finance/chart/%5EJKSE?range=2y&interval=1d` | 475 daily rows 2024-11-05 to 2026-11-05, 474 with prices (the last close is null); the `market astro --backtest --period 2y` fixture behind the literals in `src/analysis/astro.rs` and `web/test/astro.test.mjs` (2026-11-06 06:58 WIB) |
| chart_not_found.json | `v8/finance/chart/ZZZZ.JK` | HTTP 404 |
| chart_d05si_5d_1d.json | `v8/finance/chart/D05.SI?range=5d&interval=1d&includePrePost=false` | another market (DBS, Singapore): `meta.symbol` `D05.SI`, currency `SGD`, exchange `SES`, gmtoffset 28800; serves `stocks history D05.SI --period 5d` and the quote fallback (recorded 2026-11-06 07:01 WIB) |
| chart_ryto_5d_1d.json | `v8/finance/chart/RY.TO?range=5d&interval=1d&includePrePost=false` | a market west of Jakarta (Royal Bank of Canada, Toronto): gmtoffset -18000; recorded after the close, so `regularMarketTime` (16:00 EST on 2026-11-05) is already 2026-11-06 in WIB while that session's bar is stamped 09:30 EST; pins session dates to the exchange's clock (recorded 2026-11-06 10:03 WIB) |
| chart_bbca_si_404.json | `v8/finance/chart/BBCA.SI?range=5d&interval=1d&includePrePost=false` | HTTP 404: an IDX code under the wrong suffix is a clean not-found (recorded 2026-11-06 07:01 WIB) |
| chart_bbca_1d_1m.json | `v8/finance/chart/BBCA.JK?range=1d&interval=1m&includePrePost=false` | recorded 2026-11-06 06:57 WIB, before the open, so it is the 11-05 session: 435 rows 09:00 to 16:14, 333 with prices; the lunch break (12:00 to 13:29) and quiet minutes are null rows; the 09:00 bar has volume 0 (the opening auction's ~11.2M shares are in no bar: the bars sum to 155,142,100 against the day's 166,336,900); the pre-closing auction lands in the 16:00 bar |
| chart_bbca_1d_2m.json | same with `interval=2m` | same session, 218 rows, 168 with prices, same volume total |
| chart_jkse_1d_1m.json | `v8/finance/chart/^JKSE?range=1d&interval=1m&includePrePost=false` | IHSG, same session: 320 bars 09:00 to 16:00, volume 0 throughout (Yahoo has no index volume) |
| chart_bbca_1d_1m_open_empty.json | `v8/finance/chart/BBCA.JK?range=1d&interval=1m&includePrePost=false` | recorded 2026-11-06 09:00:40 WIB, just after the open: HTTP 200 with `meta` only, no `timestamp` and `indicators.quote` `[{}]`; the 1-day window stayed empty past 09:05 while the previous session had dropped out of it |
| chart_bbca_1d_1m_session.json | same as `chart_bbca_1d_1m.json` | recorded 2026-11-06 09:12:16 WIB, in session: rows 09:00 (volume 0), 09:01, a null 09:02 and a last row stamped 09:02:16, the last trade time (a snapshot, not a bar); `regularMarketTime` 09:02:16, so the chart runs about 10 minutes behind the clock (`exchangeDataDelayedBy` 10 in the v7 quote); the first row of the day appeared at 09:10 WIB |
| chart_bbca_5d_1m_open.json | same with `range=5d` | recorded 2026-11-06 09:02:50 WIB: 10-30 is gone, 11-02 to 11-05 remain, nothing for 11-06 yet; the 11-05 session matches `chart_bbca_1d_1m.json` bar for bar plus one row at 16:14:43 (the last trade time, volume 0) |
| chart_bbca_1mo_1m_error.json | `v8/finance/chart/BBCA.JK?range=1mo&interval=1m` | HTTP 422 `Unprocessable Entity`: "Only 8 days worth of 1m granularity data are allowed to be fetched per request." (2m allows 60 days) |
| quote_v7_bbca.json | `v7/finance/quote?symbols=BBCA.JK` | recorded 2026-11-06 08:43 WIB (`marketState` `PREPRE`): the 11-05 close matching `chart_bbca_1d_1m.json`; `exchangeDataDelayedBy` 10 |
| quote_v7_jkse.json | `v7/finance/quote?symbols=^JKSE` | same moment, IHSG: `regularMarketVolume` 0 |
| quote_v7_bbca_bbri_tlkm.json | `v7/finance/quote?symbols=BBCA.JK,BBRI.JK,TLKM.JK` | full quote rows |
| quote_v7_partial_bbca_zzzz.json | `v7/finance/quote?symbols=BBCA.JK,ZZZZ.JK` | unknown symbol silently omitted |
| quote_v7_unauthorized.json | `v7/finance/quote` without crumb | HTTP 401 |
| quote_v7_wika_suspended_kaef_bbca.json | `v7/finance/quote?symbols=WIKA.JK,WSKT.JK,SRIL.JK,KAEF.JK,ARMY.JK,BBCA.JK` | recorded 2026-11-06 06:58 WIB (before the open); WIKA, suspended since February 2025, still answers with `regularMarketTime` 2025-02-17 16:14:58 WIB; KAEF and BBCA carry the 2026-11-05 close; WSKT, SRIL and ARMY are omitted (unknown to Yahoo) |
| quote_v7_bbca_gspc.json | `v7/finance/quote?symbols=BBCA.JK,^GSPC` | recorded 2026-11-06 06:52 WIB; `^GSPC` is exchange SNP, America/New_York, stamped 2026-11-06 04:34 WIB: not an IDX instrument, so never behind the IDX session |
| quote_summary_bbca.json | `quoteSummary/BBCA.JK?modules=assetProfile,summaryDetail,defaultKeyStatistics,financialData` | profile + ratios (recorded 2026-10-16) |
| quote_summary_statements_bbca.json | `quoteSummary/BBCA.JK?modules=incomeStatementHistory,...,earnings,earningsHistory,earningsTrend,calendarEvents,price,recommendationTrend` | statement modules are gutted (only `endDate`); earnings modules are populated (2026-10-17) |
| quote_summary_fundamentals_bbca.json | `quoteSummary/BBCA.JK?modules=assetProfile,summaryDetail,defaultKeyStatistics,financialData,price` | bank: no EBITDA, no current ratio, `grossMargins` 0 |
| quote_summary_fundamentals_tlkm.json | same for TLKM.JK | non-bank: debtToEquity (percent), currentRatio, enterpriseToEbitda present |
| quote_summary_fundamentals_giaa.json | same for GIAA.JK | negative equity (recorded 2026-11-06 through idx 0.8.1's fetcher, raw body): `debtToEquity`, `priceToBook` and `returnOnEquity` are `{}` while `bookValue` is -0.001; `financialCurrency` USD, so per-share book value and revenue are USD against an IDR price |
| quote_summary_fundamentals_itmg.json | same for ITMG.JK | USD reporter (recorded 2026-11-06, same way): `financialCurrency` USD, price IDR; `priceToBook` 14103.308, `priceToSalesTrailing12Months` 13807.103, `enterpriseToEbitda` 91203.72 divide the IDR price by USD figures, while `trailingPE` 7.46 and `trailingEps` 3258.87 are consistent |
| quote_summary_earnings_tlkm.json | `quoteSummary/TLKM.JK?modules=earnings,earningsHistory,earningsTrend,calendarEvents,price` | earnings for a non-bank; `financialsChart.quarterly` 1Q2026 revenue padded as `{"raw":0,"fmt":null}`; `earningsTrend.trend[]` has the 0q, +1q, 0y and +1y consensus rows |
| quote_summary_earnings_tlkm_20261106.json | same request, recorded 2026-11-06 06:56 WIB | 1Q2026 revenue still padded `{"raw":0,"fmt":null}` (idx printed it as 0 at a 0.00% margin); the +1q `revenueEstimate` is padded the same way with 0 analysts |
| quote_summary_earnings_beef_20261106.json | same request for BEEF.JK, recorded 2026-11-06 09:35 WIB | a ticker no analyst covers: empty `earningsChart`/`financialsChart`, and `earningsTrend.trend[]` still has the four rows with empty EPS, padded `revenueEstimate` and 0 analysts |
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
