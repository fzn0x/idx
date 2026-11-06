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
