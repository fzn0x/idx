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
| equities_bbca.json | `Equities?ids=bn91jc` | profile, keyMetrics, companyMetrics, estimate, annualStatements, ownershipSummary |
| equities_tlkm.json | `Equities?ids=bn9o8m` | non-bank sample |
| equities_bogus_404.json | `Equities?ids=bogus1` | HTTP 404, body `[]` |

`Financials`, `KeyRatios`, `Overview`, `FinancialStatements` return 404 on this
service; statements come from Yahoo's timeseries endpoint instead.

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
