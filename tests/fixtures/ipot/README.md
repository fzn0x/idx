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

One investor side (`fd=F` foreign, `fd=D` domestic) and one board: the
footer's `T. Val`, `T.Lot` and `Avg` are that side's buying, and `F. NVal`
is the all-board foreign net on every page.

| file | request | recorded | notes |
| --- | --- | --- | --- |
| brokersummary_bbca_20261016_foreign.html | BBCA 10-16, `fd=F` | by 2026-10-18 | from 0xrsydn/idx-cli (MIT, added there 2026-10-18), `ipot_brokersummary_bbca_20261016_foreign_all.html`, byte-identical: `T. Val` 1.7 T (foreign buying), `F. NVal` -2.3 T |
| brokersummary_bbca_20261016_domestic.html | BBCA 10-16, `fd=D` | by 2026-10-18 | from 0xrsydn/idx-cli (MIT, added there 2026-10-18), `ipot_brokersummary_bbca_20261016_domestic_all.html`, byte-identical: `T. Val` 4.9 T (domestic buying) |
| brokersummary_bbca_20261016_foreign_regular.html | BBCA 10-16, `fd=F&board=RG` | by 2026-10-18 | from 0xrsydn/idx-cli (MIT, added there 2026-10-18), `ipot_brokersummary_bbca_20261016_foreign_regular.html`, byte-identical: `T. Val` 1.6 T, `F. NVal` still the all-board -2.3 T |
| brokersummary_bbca_20261105_foreign.html | BBCA 11-05, `fd=F` | 2026-11-06 06:49 | `T. Val` 605.4 B, `F. NVal` 303.9 B: foreign investors sold 301.5 B |
| brokersummary_bbca_20261105_domestic.html | BBCA 11-05, `fd=D` | 2026-11-06 06:51 | `T. Val` 242.8 B: with the foreign 605.4 B, the 848.3 B traded |

Foreign flow (`stocks foreign --source ipot`): each session's `fd=all` and
`fd=F` pages. During a session the source publishes today's totals so far but
no broker lists, an empty foreign page and `F. NVal` 0; ranges are right then
(the doubled net appears between the close and the next open). A closed
session's lists come some time after the close: at 16:45 WIB on 2026-11-06
the web scan still read that session without them.

| file | request | recorded | notes |
| --- | --- | --- | --- |
| brokersummary_bbca_20261012_foreign.html .. brokersummary_bbca_20261015_foreign.html | BBCA, `fd=F`, one session each | by 2026-10-18 | from 0xrsydn/idx-cli (MIT, added there 2026-10-18), `ipot_brokersummary_bbca_2026101X_foreign_all.html`, byte-identical; with the 10-16 page, buying 4,364.7 B, net -1,823.1 B over the week |
| brokersummary_bbca_20261102.html, brokersummary_bbca_20261103.html | BBCA, one session each | 2026-11-06 06:51 | `T. Val` 1.2 T and 1.5 T |
| brokersummary_bbca_20261102_foreign.html .. brokersummary_bbca_20261104_foreign.html | BBCA, `fd=F`, one session each | 2026-11-06 06:49 to 06:51 | foreign buying 792.1 B, 961.5 B, 955.7 B |
| brokersummary_truk_20261105.html, brokersummary_truk_20261105_foreign.html | TRUK 11-05, `fd=all` and `fd=F` | 2026-11-06 07:02 | the stock traded 21.6 M and no foreigner did: the foreign page is the empty template |
| brokersummary_bbca_20261106_insession.html, brokersummary_bbca_20261106_insession_foreign.html | BBCA 11-06, `fd=all` and `fd=F` | 2026-11-06 09:15 | during the session: `T. Val` 89.5 B so far, no broker lists, `F. NVal` 0, the foreign page empty |
| brokersummary_bbca_20261102_20261106_insession.html, brokersummary_bbca_20261102_20261105_insession.html | BBCA, 11-02 to 11-06 and 11-02 to 11-05 | 2026-11-06 09:15 | during the session both read 292.2 B: no session counted twice (11-02..11-05 read 595.4 B before the open) |
| brokersummary_bbca_20261106.html | BBCA 11-06 | 2026-11-08 07:02 | the same session once published: ten buyers and ten sellers, `T. Val` 1.0 T, `F. NVal` 219.0 B |
