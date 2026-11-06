# MSN Money fixtures

Recorded 2026-10-17 with curl, User-Agent Chrome 130. Base URL
`https://assets.msn.com/service/Finance/<endpoint>?apikey=<key>&cm=en-id&ocid=finance-utils-peregrine&it=web&scn=ANON&wrapodata=false`.
The key is MSN's own public web-client key (not user-issued); without it the
service answers 401.

| file | request | notes |
| --- | --- | --- |
| autosuggest_bbca.json | `services.bingapis.com/contentservices-finance.csautosuggest/api/v1/Query?query=BBCA&market=en-id` | `data.stocks[]` are JSON *strings*; filter `ExMicCode == "XIDX"`, take `SecId` |
| autosuggest_goto_ambiguous.json | same, `query=GOTO` | several exchanges match; the XIDX row is not first |
| autosuggest_zzzz_no_idx_match.json | same, `query=ZZZZ` | matches exist but none on XIDX → not found |
| quotes_bbca.json | `Quotes?ids=bn91jc` | one id → flat array of one Quote |
| quotes_bbca_bbri_tlkm.json | `Quotes?ids=bn91jc,bn91p2,bn9o8m` | several ids → array of arrays, one inner array per id |
| quotes_bbca_with_bogus_ids.json | `Quotes?ids=bn91jc,bogus1,a1b2c3` | unknown ids silently dropped |
| quotes_bbca_wika_kaef.json | `Quotes?ids=bn91jc,bn9q77,bkgvp2` | recorded 2026-11-06 06:58 WIB; WIKA (suspended) has `price` and `timeLastTraded` null; BBCA and KAEF are stamped 2026-11-05T09:15:00Z (16:15 WIB) |
| equities_bbca.json | `Equities?ids=bn91jc` | profile, keyMetrics, companyMetrics, estimate, annualStatements, ownershipSummary |
| equities_tlkm.json | `Equities?ids=bn9o8m` | non-bank sample |
| equities_bogus_404.json | `Equities?ids=bogus1` | HTTP 404, body `[]` |
| autosuggest_giaa.json | `...csautosuggest/api/v1/Query?query=GIAA&market=en-id` | recorded 2026-11-06 (see below); the XIDX row carries SecId `bkgq5r` |
| equities_giaa.json | `Equities?ids=bkgq5r` | negative equity, recorded 2026-11-06 (see below): `keyMetrics.debtToEquityRatio` and `companyMetrics.debtEquityRatio` are the placeholder -999.9999, `bookValuePerShare` -9.829, no `priceToBookRatio` or `returnOnEquity`; `annualStatements` show ROE positive on net losses (2020, 2021, 2024, 2025) |

Recorded 2026-11-06 between 07:00 and 07:05 WIB, same client and query, for
`ownership institutions` (the `ownershipSummary` block; `equities_bbca.json`
and `equities_tlkm.json` above carry it too):

| file | request | notes |
| --- | --- | --- |
| autosuggest_jtpe.json | `csautosuggest/api/v1/Query?query=JTPE&market=en-id` | one XIDX match, `SecId` `bkgwu2` |
| autosuggest_atap.json | same, `query=ATAP` | XIDX (`bwzup2`) and a NEO Exchange listing with the same ticker |
| equities_jtpe_all_signals.json | `Equities?ids=bkgwu2` | 4 institutions and 5 funds covering every signal: unchanged, seller, buyer, new (change equals the holding), exited (0 shares after a sale) |
| equities_atap_empty_holders.json | `Equities?ids=bwzup2` | `ownershipSummary` with float and insider percentages only and empty holder lists |
| equities_star_no_ownership.json | `Equities?ids=bkhrbh` | no `ownershipSummary` block at all |

`Financials`, `KeyRatios`, `Overview`, `FinancialStatements` return 404 on this
service; statements come from Yahoo's timeseries endpoint instead.

The GIAA files were recorded 2026-11-06 (about 06:48 WIB) through idx 0.8.1's own
fetcher with an isolated cache and copied byte for byte from that cache; the
request is the same as above (the api key is not part of the body or the name).

## Insights (`api.msn.com`)

Recorded 2026-11-06 06:52-07:01 WIB (before the session, so the statements
reflect Thursday 2026-11-05's close) with curl, User-Agent Chrome 130, same
public key. URL:
`https://api.msn.com/msn/v0/pages/finance/insights?ids=<secid>&apikey=<key>&cm=en-id&ocid=finance-utils-peregrine&it=web&scn=ANON&wrapodata=false`.
`assets.msn.com/service/Finance/Insights` does not exist (404/503).

| file | request | notes |
| --- | --- | --- |
| insights_bbca.json | `insights?ids=bn91jc` | one object, 30 items: 17 `good`, 4 `bad`, 9 `NA`; `peers` = bkfmar, bkfgqh, cj53lh, bkgttc; `symbo` (sic) |
| insights_goto_neutral.json | `insights?ids=c5tv8m` | 13 good, 2 bad, 1 `neutral`, 14 NA |
| insights_bogus_404.json | `insights?ids=bogus1` | HTTP 404, `{"responses":[{"error":{"code":"NoDataFounded",...}}],"copyright":...}` |
| quotes_bbca_insight_peers.json | `Quotes?ids=bkfmar,bkfgqh,cj53lh,bkgttc` | BBCA's peers resolve to BBKP, BABP, SUPA, INPC (all `XIDX`) |

Benchmark objects seen on BBCA, TLKM, GOTO, STAR and ATAP (same pattern on all
five): `AggregateData` `<id>peers` (16 items) is the peer group, another
`AggregateData` id such as `ididbankingservices` (5) the industry, `Earning` (6)
and `History` (1) the company's own past, `Quote` (1) the price (analyst
target) and `Constant` (1) a threshold (analyst rating). Unrated (`NA`) items
whose value and benchmark are both 0 carry MSN's placeholder for missing data;
idx reports them as null.

MSN's crowd sentiment (`assets.msn.com/service/Finance/SentimentBrowser`,
needs `flightId`) answered `[]` for BBCA and TLKM in the en-us, en-id, id-id,
en-sg, en-gb and en-ww markets, for DBS on SGX and the TLK ADR; only US names
such as MSFT had data. It is not used and no fixture is kept.

## Screener (`Finance/Screener`, POST)

Recorded 2026-11-06 06:54-07:01 WIB (Thursday 2026-11-05's close) with curl,
User-Agent Chrome 130, `Content-Type: application/json`, same public key and
common query. `POST https://assets.msn.com/service/Finance/Screener?apikey=<key>&cm=en-id&ocid=finance-utils-peregrine&it=web&scn=ANON&wrapodata=false`.
A GET answers 500 (`Strategy not available ...`), no key 401, a malformed body 500.

idx sends one request, the whole Indonesian market with the quote block only:

```json
{"filter":[{"key":"st_reg_id","keyGroup":"st_reg_","isRange":false}],"returnValueType":["quote"],"screenerType":"stock","limit":2000,"pageIndex":0}
```

It answered `count` 887 with 887 quotes (1.27 MB, ordered by market cap; not
kept as a fixture). `limit` is honoured and `count` stays the full total;
`pageIndex` is ignored. Each quote carries `return1Week`, `return1Month`,
`return3Month`, `return6Month`, `returnYTD` and `return1Year` in percent.

| file | request body | notes |
| --- | --- | --- |
| screener_region_id_limit12.json | the body above with `"limit":12` | `count` 887, 12 quotes (BBCA, BYAN, DCII, BBRI, BREN, BMRI, AMMN, TLKM, MORA, DSSA, ASII, BRPT): the truncated case |
| screener_52wkhi_id_complete.json | MSN's `st_list_52wkhi` list for Indonesia, quote block only (exact body not kept) | `count` 38 = 38 quotes, a complete list; IBST lacks 1w..6m, GLOB every window, MTPS all but 1y |
| screener_lowpe_id_empty.json | MSN's `st_list_lowpe` list for Indonesia (exact body not kept) | `{"count":0,...}`: the low-P/E list is empty for Indonesia; the empty-list case |

MSN's own Indonesian preset lists are not used by idx: `st_list_topperfs` is
the first 133 rows of a stale ordering (it holds MORA, down 85 % YTD),
`st_list_lowpe`, `st_list_largecap` and `st_list_mostactive` are empty, and
`st_list_52wkhi` holds stocks at 0 to 27 % of their 52-week range.
