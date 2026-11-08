# Stockbit fixtures

Recorded 2026-10-18 (a Sunday, so the market was closed and every number is Friday
2026-10-16's close) against `https://exodus.stockbit.com`, GET only, with the headers
`authorization: Bearer <access JWT>`, `accept: application/json`,
`origin: https://stockbit.com`, `referer: https://stockbit.com/` and a Chrome user agent.
The token belongs to a real account. Responses were checked for its username, email,
full name and user id, and for per-user flags such as `is_holding_exist`, before they
were copied here. Nothing in this directory is a credential.

One file, `orderbook_bbca_in_session.json`, was recorded during trading by the
reference CLI 0xrsydn/idx-cli (MIT License, Copyright (c) 2026 idx-cli contributors)
and is copied unchanged (same bytes as its
`crates/idx-core/tests/fixtures/stockbit_exodus_orderbook_bbca_in_session.json`); it is
credited in its row. Its keys are the same as `orderbook_bbca.json`'s, with no user,
account, holding or token field.

| file | request | notes |
| --- | --- | --- |
| orderbook_bbca.json | `/company-price-feed/v2/orderbook/companies/BBCA` | quote fields (`lastprice`, `open`, `high`, `low`, `previous`, `volume` in SHARES, `value` IDR) plus the depth: `bid[]` / `offer[]` of `{price, que_num, volume}` as strings, volume in SHARES (the offer levels sum to `total_bid_offer.offer.raw_lot`, which is therefore shares too despite its name). BBCA closed at ARB with no bids, so `bid` is `[]`. `has_foreign_bs: true`: `fbuy` / `fsell` / `fnet` (IDR) are that day's foreign flow |
| orderbook_bbca_in_session.json | `/company-price-feed/v2/orderbook/companies/BBCA`, recorded in session 2 on 2026-11-03 by 0xrsydn/idx-cli (MIT), copied unchanged | `has_foreign_bs: false` (`orderbook_active_feature_mobile: ORDER_BOOK_FEATURE_UNSPECIFIED`): `fbuy` / `fsell` / `fnet` (and the `foreign` / `domestic` percentages) are 2026-11-02's, not the session's. `fnet` -116.1B is IPOT's BBCA foreign net for 2026-11-02; 2026-11-03 closed at -28.8B. Volume 134,744,100 shares and value 667.0B are the session's own. 22 bid / 64 offer levels summing to `total_bid_offer.*.raw_lot` (shares) |
| orderbook_ihsg.json | `/company-price-feed/v2/orderbook/companies/IHSG` | indices answer on the same route: OHLC, volume in shares, empty book |
| orderbook_zzzz.json | same, `ZZZZ` | HTTP 404 `{"message":"Company not found","error_type":"NOT_FOUND"}` |
| chart_candle_bbca_1m.json | `/charts/BBCA/daily?timeframe=1m&chart_type=PRICE_CHART_TYPE_CANDLE` | daily bars; `prices[]` of `{date (epoch ms, bar start WIB), formatted_date, open, high, low, value (= close), volume (LOTS)}` as strings |
| chart_candle_bbca_1w.json | `timeframe=1w&is_include_previous_historical=true`, candle | HOURLY bars (`interval_in_minutes: 60`), 09:00 .. 16:00 plus the 16:14 closing print; the hourly volumes of 2026-10-16 sum to that day's daily volume (12,410,982 lots) |
| chart_candle_bbca_1y.json | `timeframe=1y`, candle | daily bars for a year (240) |
| chart_candle_bbca_3y.json | `timeframe=3y`, candle | WEEKLY bars dated by the week's first session; `5y` is weekly too |
| chart_candle_bbca_1d.json | `timeframe=1d`, candle | `prices: []` outside the session |
| chart_candle_zzzz_1m.json | `/charts/ZZZZ/daily?...` | HTTP 404 `NOT_FOUND` |
| market_session.json | `/company-price-feed/market-time/session` | `detail.regular.state_name`, e.g. `STATE_NAME_MARKET_CLOSED` |
| unauthorized.json | any route with an invalid bearer | HTTP 401 `{"error_type":"UNAUTHORIZED","message":"Unauthorized"}` |

## Broker summary, recorded by idx on 2026-11-08

Recorded on Sunday 2026-11-08 between 14:39 and 14:43 WIB through the idx binary itself
(its Stockbit provider and headers, a fresh cache directory; the bodies were copied out of
that cache unchanged), one request at a time, a few seconds apart, twelve in all. The session
belongs to a real account. The replies carry no user field at all: every key was listed and
the bodies were searched for JWTs, `bearer`, `token`, `cookie`, `session`, `user`, `@`,
`is_holding_exist` and `is_price_alert_exist`; there is none, so nothing was scrubbed.

Every request is `GET /marketdetectors/{SYM}?transaction_type=TRANSACTION_TYPE_GROSS&market_board=MARKET_BOARD_ALL&investor_type=INVESTOR_TYPE_ALL&from=YYYY-MM-DD&to=YYYY-MM-DD&limit=100`
unless its row says otherwise. The reply is
`{"message", "data":{"bandar_detector":{..}, "broker_summary":{"brokers_buy":[..], "brokers_sell":[..], "symbol"}, "from", "to"}}`.
A buy row: `netbs_broker_code`, `netbs_stock_code`, `netbs_date` (`YYYYMMDD`, the range's first
day), `type` (`Asing`, `Lokal`, `Pemerintah`), `freq`, `blot` (lots), `blotv` (shares), `bval`
and `bvalv` (IDR), `netbs_buy_avg_price`; a sell row has `slot`, `slotv`, `sval`, `svalv`,
`netbs_sell_avg_price` instead. Figures are strings (`"687161"`, `"3.524875059e+11"`,
`"108898.5"`); in the empty reply they are JSON numbers (`0`). `bandar_detector` holds
`total_buyer` and `total_seller` (brokers on each side, whatever `limit` cut), `value`,
`volume` and `average` (the buy side's totals at about eight significant digits:
1,035,652,040,000 where the rows sum to 1,035,652,066,452) and Stockbit's own readings
(`top1` .. `top10`, `avg`, `avg5`, `broker_accdist`, `number_broker_buysell`).

In gross mode every broker that bought is a buy row and every broker that sold a sell row,
`blot` is `blotv / 100` and `bval` is `bvalv`, both sides sum to the value traded, and
the rows are ranked by value (KZ's 857.7 B before ZP's 852.9 B over 11-02 .. 11-06,
although ZP has more lots). Lots are fractional where odd lots traded (YU 108,898.5 on all
boards, 108,898 on the regular board). Ordered by lots, the first ten rows of each side
are IPOT's top 10 for the same day and for the same range, in the same order, with the
same lots and values at IPOT's precision (and for the day the same averages); IPOT calls
HD local where Stockbit says `Asing`.

| file | request | notes |
| --- | --- | --- |
| marketdetectors_bbca_20261106.json | BBCA, 2026-11-06 | 59 buyers and 67 sellers, `total_buyer` 59, `total_seller` 67; both sides sum to 1,035,652,066,452 IDR and 2,027,579.79 lots. The same bytes came back with `limit=1000` and with no `limit` at all |
| marketdetectors_bbca_20261106_net.json | same, `transaction_type=TRANSACTION_TYPE_NET` | net mode: each broker once, on the side where it was net (16 buyers, 55 sellers); `blot` / `bval` are the net (negative on the sell side), `blotv` / `bvalv` and the average stay that side's gross (KZ net 356,958 lots of 687,161 bought). Not what idx asks for |
| marketdetectors_bbca_20261102_20261106.json | BBCA, 2026-11-02 to 2026-11-06 | one aggregate over the five sessions (81 buyers, 78 sellers, 5,999,367,547,273 IDR; IPOT: 6.0 T); `netbs_date` is 20261102 on every row |
| marketdetectors_bbca_20251106_20261106.json | BBCA, 2025-11-06 to 2026-11-06, `limit=1000` | 92 buyers and 92 sellers, all listed: a large `limit` returns every broker |
| marketdetectors_bbca_20261106_regular.json | BBCA, 2026-11-06, `market_board=MARKET_BOARD_REGULER` | regular board only: 817,283,116,500 IDR, whole lots |
| marketdetectors_bbca_20261106_foreign.json | BBCA, 2026-11-06, `investor_type=INVESTOR_TYPE_FOREIGN` | foreign investors: 23 buyers for 786,531,414,962 IDR, 22 sellers for 567,543,113,862; their net, 218,988,301,100, is IPOT's `F. NVal` of 219.0 B |
| marketdetectors_bbca_20261106_limit5.json | BBCA, 2026-11-06, `limit=5` | five rows a side while `total_buyer` stays 59 and `total_seller` 67: the counts show a cut |
| marketdetectors_cbre_20261106.json | CBRE, 2026-11-06 | a thin day: 25 buyers, 4 sellers, 1,311,477,000 IDR |
| marketdetectors_bbca_20261107_no_trades.json | BBCA, Saturday 2026-11-07 | HTTP 200 with empty lists and zeros (as JSON numbers) |
| marketdetectors_zzzz.json | ZZZZ, 2026-11-06 | HTTP 404 `{"message":"Perusahaan tidak ditemukan","error_type":"NOT_FOUND"}`; the fetcher caches 2xx replies only, so this one was copied from the body idx printed |

## Recorded by 0xrsydn/idx-cli

The files in this table were recorded by the reference CLI 0xrsydn/idx-cli (MIT
License, Copyright (c) 2026 idx-cli contributors) on 2026-11-03 with a real session,
GET only, and are copied byte for byte from its `crates/idx-core/tests/fixtures/` at
commit be9d17b (file name there: `stockbit_exodus_<name>`, or `stockbit_<name>` for the
block page). They were checked for user data (user ids, e-mail addresses, names of the
account holder, `is_holding_exist`, JWTs) before they were copied: there is none.

| file | request | notes |
| --- | --- | --- |
| corpaction_dividend.json | `/corpaction/dividend` | market-wide cash dividends, 21 rows, cum dates 2025-11-12 .. 2026-11-11, plus `"today":"2026-11-03"`. Row fields `dividend_{cumdate,exdate,recdate,paydate,value,currency,id}` (`value` a string, `currency` `CURRENCY_IDR`), `event_note`, `lastprice` |
| corpaction_dividend_bbca.json | `/corpaction/dividend?symbol=BBCA` | BBCA's cash dividends, 37 rows, 2026-08-28 back to 2009-06-09 |
| corpaction_stock_dividend.json | `/corpaction/stock_dividend` | `{"stock_dividend":[]}`: empty on the recording day |
| corpaction_bonus.json | `/corpaction/bonus` | bonus shares: `sahambonus_{ratio,id}` (ratio `"6 : 4"`) and the dates and terms under `stocksplit_*` names (`cumdate`, `exdate`, `recdate`, `paymentdate`, `old`, `new`, `factor`); `sahambonus_new_price: 0` is a placeholder |
| corpaction_rightissue.json | `/corpaction/rightissue` | `rightissue_{cumdate,exdate,recdate,old,new,ratio,price,factor,trading_start,trading_end,id}`; `price` a number; decoys `rightissue_adj_factor`, `_price_factor`, `_price_adj` are 0 |
| corpaction_warrant.json | `/corpaction/warrant` | 36 rows: BRNA's company warrant and 35 structured warrants whose `company_symbol` is the warrant's own 10-character code (`ASIIZPCN7A`); `wrant_{trading_from,trading_end,exc_from,exc_end,exc_price,id}` |
| corpaction_stocksplit.json | `/corpaction/stocksplit` | market-wide splits, `stocksplit_{cumdate,exdate,recdate,old,new,ratio,factor,id}` |
| corpaction_stocksplit_bbca.json | `/corpaction/stocksplit?symbol=BBCA` | BBCA's splits: 2021 1 : 5, 2008 and 2004 1 : 2 |
| corpaction_reversesplit.json | `/corpaction/reversesplit` | `{"stock_reverse":[]}`: the list key is not the path type; empty on the recording day |
| corpaction_rups.json | `/corpaction/rups` | 47 meetings, 2026-11-03 .. 2026-12-09: the market list starts on the recording day. `rups_{date,time,venue,eligible_date,id}`, `company_name` often `""` |
| corpaction_rups_bbca.json | `/corpaction/rups?symbol=BBCA` | BBCA's meetings back to 2020; venues contain newlines and tabs |
| corpaction_pubex.json | `/corpaction/pubex` | public exposes 2026-11-03 .. 2026-11-24, `puexp_{date,time,venue,id}` (`time` `"13:00:00"`) |
| corpaction_pubex_bbca.json | `/corpaction/pubex?symbol=BBCA` | BBCA's six exposes back to 2021; tabs in a venue |
| corpaction_ipo.json | `/corpaction/ipo` | 15 listings 2025-11-06 .. 2026-09-26: `ipo_listing_date`, `company_name`, `ipo_price.final`, `ipo_data_detail.{shares,percentage,offering_start,offering_end}`; ETF listings carry the placeholders `0`, `""` and `0001-01-01 00:00:00` |
| corpaction_tenderoffer.json | `/corpaction/tenderoffer` | `{"tender":[..]}` (list key `tender`): `tender_{start,end,paydate,price,shares,percentage,id}`, `company_name`; AUTO at 3,600 for 238,442,293 shares (4.95 %) |
| majorholder.json | `/insider/company/majorholder?page=1` | market-wide movements, `is_more: true`; an excerpt (8 rows) of a 50-row page. Row: `id` (the holder's id, not the row's), `name`, `symbol`, `date` (`"02 Nov 26"`), `previous` / `current` / `changes` (`value` shares as `"+2,500,000"`, `percentage` as `"+0.07"`), `action_type` (`ACTION_TYPE_BUY`, `_SELL`, `_TRANSFER`, `_RIGHT_ISSUE`), `data_source.type` (`SOURCE_TYPE_IDX` / `_KSEI`), `nationality`, `badges`, `price.raw.value`, `value.raw.value`, `is_estimated_price` (true on the KSEI rows, whose `price_formatted` is empty), `purpose`, `broker_detail.{code,group}`. BUVA's rights-issue row: `changes.value "0"` with `percentage "-0.97"` |
| majorholder_p2.json | `/insider/company/majorholder?page=2` | `is_more: false`; 6 rows: crossings (ENRG, purpose `Reverse REPO`; one with an estimated price), `PRIVATE_PLACEMENT` (purpose `PMTHMETD`), `TENDER_OFFER` |
| majorholder_bbca.json | `/insider/company/majorholder?page=1&symbol=BBCA` | 8 director and commissioner buys at 6,982 on 2026-03-25 (a share plan); `badges` `SHAREHOLDER_BADGE_DIREKTUR` / `_KOMISARIS`; stakes such as `"0.0003"` |
| majorholder_bbca_p2.json | `/insider/company/majorholder?page=2&symbol=BBCA` | `is_more: false`; 4 rows from 2024-03-22 at 10,108 |
| blocked_403.html | any exodus route, from a network Stockbit's CDN refuses | HTTP 403 HTML page from CloudFront ("The request could not be satisfied"). Not a refused token: a refresh cannot help, so idx reports it and does not retry (a refused token is a 401, or a 403 with a JSON body such as `unauthorized.json`) |

Constructed, not recorded (the tokens inside are dummies signed by nobody):

| file | stands for | why constructed |
| --- | --- | --- |
| login_refresh_constructed.json | `POST /login/refresh` (bearer = refresh token, body `{}`) | recording one spends a real refresh token; the shape is the one a long-running bot reads every day (`data.access.token`, `data.refresh.token`). The parser searches structurally, so a wrapper level more or less still parses |
| credential_storage_cookie_constructed.txt | the `credentialStorage` cookie stockbit.com keeps after a login | URL-encoded `{"state":{"access":{"token","expired_at"},"refresh":{...},"user":{...}},"version":0}`, the layout documented by github.com/ino-xious/stockbit-mcp; the parser also accepts bare JWT strings in `access` / `refresh` |
