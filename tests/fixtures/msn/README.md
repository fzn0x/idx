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
