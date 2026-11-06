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
| chart_d05si_5d_1d.json | `v8/finance/chart/D05.SI?range=5d&interval=1d&includePrePost=false` | another market (DBS, Singapore): `meta.symbol` `D05.SI`, currency `SGD`, exchange `SES`, gmtoffset 28800; serves `stocks history D05.SI --period 5d` and the quote fallback (recorded 2026-11-06 07:01 WIB) |
| chart_ryto_5d_1d.json | `v8/finance/chart/RY.TO?range=5d&interval=1d&includePrePost=false` | a market west of Jakarta (Royal Bank of Canada, Toronto): gmtoffset -18000; recorded after the close, so `regularMarketTime` (16:00 EST on 2026-11-05) is already 2026-11-06 in WIB while that session's bar is stamped 09:30 EST; pins session dates to the exchange's clock (recorded 2026-11-06 10:03 WIB) |
| chart_bbca_si_404.json | `v8/finance/chart/BBCA.SI?range=5d&interval=1d&includePrePost=false` | HTTP 404: an IDX code under the wrong suffix is a clean not-found (recorded 2026-11-06 07:01 WIB) |
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
| quote_summary_earnings_tlkm.json | `quoteSummary/TLKM.JK?modules=earnings,earningsHistory,earningsTrend,calendarEvents,price` | earnings for a non-bank |
| quote_summary_not_found.json | `quoteSummary/ZZZZ.JK` | HTTP 404, `error.code` "Not Found" |
| timeseries_bbca.json | `ws/fundamentals-timeseries/v1/finance/timeseries/BBCA.JK?type=annual...,quarterly...,trailing...` | 4 annual years, ~5 quarters, TTM; bank: no EBITDA/GrossProfit/OperatingIncome series |
| timeseries_tlkm.json | same for TLKM.JK | non-bank: EBITDA, EBIT, GrossProfit, OperatingIncome, NetDebt present |
| timeseries_not_found.json | same for ZZZZ.JK | HTTP 200, series with `meta` only and no data |
