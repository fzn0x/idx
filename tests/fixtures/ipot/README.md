# IPOT fixtures

Raw HTML fragments from Indo Premier's public `module/saham/include/` pages,
recorded with curl and a browser User-Agent (no login, no key). Broker
summaries come from
`data-brokersummary.php?code=CODE&start=MM/DD/YYYY&end=MM/DD/YYYY&fd=all|F|D&board=all|RG|TN|NG`;
`fd=all` and `board=all` unless the notes say otherwise. Dates are WIB.

The footer's foreign net (`F. NVal`) over a range that starts before the
source's latest session and contains it counts that session twice; the
`*_20261012_20261016.html` range pages were recorded the weekend after
2026-10-16 and carry that doubled net. `ipot::broker_summary_range` and
`web/src/ipot.js` rebuild such nets from the single sessions below.

| file | request | recorded | notes |
| --- | --- | --- | --- |
| brokersummary_bbca_20261012.html .. brokersummary_bbca_20261016.html | BBCA, one session each, 10-12 to 10-16 | 2026-10-17 | `T. Val` 587.1 B, 738.2 B, 1.1 T, 912.0 B, 6.6 T; `F. NVal` -55.9 B, 30.1 B, 216.6 B, 286.1 B, -2.3 T |
| brokersummary_bbca_20261012_20261016.html | BBCA, 10-12 to 10-16 | 2026-10-17 | `F. NVal` -4.1 T: the 16th counted twice (the sessions sum to -1,823.1 B) |
| brokersummary_bbca_20261012_20261016_later.html | BBCA, 10-12 to 10-16 | 2026-11-06 06:52 | the same range once it had aged: `F. NVal` -1.9 T, the sessions at the source's precision |
| brokersummary_bbca_20261012_20261014.html | BBCA, 10-12 to 10-14 | 2026-11-06 07:11 | `F. NVal` 190.8 B, equal to the sum of its sessions |
| brokersummary_bbca_20261017_no_trades.html | BBCA, Saturday 10-17 | 2026-10-17 | no trades: ten blank rows, zero totals |
| brokersummary_bbca_20261102_20261103.html | BBCA, 11-02 to 11-03 | 2026-11-06 07:11 | `F. NVal` -144.9 B, equal to the sum of its sessions |
| brokersummary_bbca_20261102_20261104.html | BBCA, 11-02 to 11-04 | 2026-11-06 07:05 | `F. NVal` -11.7 B, right (the latest session was 11-05) |
| brokersummary_bbca_20261102_20261105.html | BBCA, 11-02 to 11-05 | 2026-11-06 06:51 | `F. NVal` 595.4 B: 11-05 counted twice (the sessions sum to 292.3 B) |
| brokersummary_bbca_20261104_20261105.html | BBCA, 11-04 to 11-05 | 2026-11-06 06:52 | `F. NVal` 740.1 B: 11-05 counted twice (the sessions sum to 437.2 B) |
| brokersummary_bbca_20261104.html, brokersummary_bbca_20261105.html | BBCA, one session each | 2026-11-06 06:49 | `F. NVal` 133.3 B and 303.9 B |
| brokersummary_brpt_20261012_20261016.html, brokersummary_tlkm_20261012_20261016.html | BRPT, TLKM, 10-12 to 10-16 | 2026-10-17 | `F. NVal` 119.8 B and -319.8 B, the 16th counted twice |
| brokersummary_brpt_20261012_20261014.html, brokersummary_brpt_20261015.html, brokersummary_brpt_20261016.html | BRPT, 10-12 to 10-14, then one session each | 2026-11-06 08:52 | `F. NVal` 17.1 B, -14.3 B, 57.8 B |
| brokersummary_tlkm_20261012_20261014.html, brokersummary_tlkm_20261015.html, brokersummary_tlkm_20261016.html | TLKM, 10-12 to 10-14, then one session each | 2026-11-06 08:52 | `F. NVal` -96.6 B, -17.9 B, -90.2 B |
| brokersummary_zzzz_invalid.html | `code=ZZZZ` | 2026-10-17 | `Invalid stock code!`, HTTP 200 |
| corporateaction_*.html | `corporateaction.php?code=CODE&start=MM/DD/YYYY` | 2026-10-18 | corporate actions, RUPS, warrants and rights (see `actions.rs`) |
