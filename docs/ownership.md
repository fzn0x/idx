# Ownership data

Since March 2026 KSEI compiles, and IDX publishes monthly, the list of every
shareholder holding **more than 1 %** of a listed company ("Pemegang Saham di
Atas 1%"). `idx ownership` imports those files into a local SQLite database
and answers questions from it offline.

## Source files

| period | format | where |
| --- | --- | --- |
| 30 Oct 2026 onwards | XLSX, 13 columns (`CONTROLLER_OR_AFFILIATE` after `INVESTOR_CLASSIFICATION`) | idx.co.id, "Data Kepemilikan Saham" page, `Media/.../*-satu-persen.xlsx` |
| June to September 2026 | XLSX, 12 columns | same page |
| February to May 2026 | PDF announcement | idx.co.id announcements ("Pemegang Saham di atas 1% (KSEI)") |

Columns: `DATE, SHARE_CODE, ISSUER_NAME, INVESTOR_NAME, INVESTOR_CLASSIFICATION`
(`INVESTOR_TYPE` in the first releases), `LOCAL_FOREIGN, NATIONALITY, DOMICILE,
HOLDINGS_SCRIPLESS, HOLDINGS_SCRIP, TOTAL_HOLDING_SHARES, PERCENTAGE`, and from
the register as of 30 Oct 2026 `CONTROLLER_OR_AFFILIATE` (`CONTROLLER`,
`AFFILIATE OF THE CONTROLLER` or blank) after `INVESTOR_CLASSIFICATION`.
Columns are found by name, so both layouts import; the "di Atas 5 %" and
investor-type workbooks on the same page lack required columns and are
rejected with a clear message.

Each row carries its own `DATE`. A file is filed under its most common date
(`as_of`); rows with another date are kept but left out of that release's
holdings (see Database).

## Getting the files

idx.co.id is behind Cloudflare bot management, so a plain HTTP client gets a
challenge page. `ownership import --latest|--all` and `ownership discover` try,
in order:

1. a browser-shaped request (Chrome user agent and headers);
2. a `curl-impersonate` binary on `PATH` (`curl_chrome131`, `curl_chrome124`,
   `curl_chrome116`, `curl-impersonate-chrome`, `curl-impersonate`; override
   with `IDX_CURL_IMPERSONATE=/path/to/binary`);
3. the GitHub mirror (below): the same files, downloaded from the
   repository's `ownership-data` release and checked against the SHA-256 in
   its `index.json`. A warning says when the mirror was used.
   `IDX_OWNERSHIP_MIRROR=<url>` points at another mirror, `off` disables it;
   `IDX_OWNERSHIP_SOURCE=mirror` skips idx.co.id, `idx` never uses the mirror.

`--file <path>` and `--url <url>` import one file directly; `--url` also
accepts `file://` paths. `idx ownership discover` lists what is available,
where it comes from and which months are already imported.

## The mirror

`.github/workflows/ownership.yml` runs `scripts/ownership_mirror.py` at 08:17,
14:17 and 20:17 WIB. idx.co.id blocks cloud networks: GitHub-hosted runners
get a Cloudflare challenge even with a browser TLS fingerprint, and requests
from Cloudflare Workers are refused. The job therefore needs a runner on an
ordinary (home or office) connection:

1. Repository → Settings → Actions → Runners → New self-hosted runner, and
   follow the three commands on any always-on machine with Git, `gh` and
   Python 3.10+ (Windows, macOS or Linux). Install it as a service.
2. Repository → Settings → Secrets and variables → Actions → Variables:
   `OWNERSHIP_RUNNER` = `self-hosted`.

Without that variable the job runs on `ubuntu-latest`, cannot reach
idx.co.id, and only rebuilds the parts from the files already mirrored (so a
newer idx parser still reaches the web terminal); it fails once the newest
register is 45 days old, so a stale mirror shows up as a red run. Each run of
the script:

1. fetches the IDX page with [curl_cffi](https://github.com/lexiforest/curl_cffi)
   (browser TLS fingerprint), saves it, and parses it with
   `idx ownership discover` (one parser for CLI and CI); when that fails it
   carries on with the months already mirrored;
2. reuses files already mirrored, downloads new or re-published ones (a new
   IDX URL for the same month counts as re-published) and checks they are
   real XLSX/PDF files;
3. imports every month, oldest first, into a fresh database with
   `idx ownership import --file`; any parse failure fails the run, so a broken
   file never reaches the mirror;
4. exports each month as JSON parts of 1,000 rows (one row per holder and
   issuer, the holder's accounts summed; fields as in `idx ownership ticker
   -o json` plus `accounts` and `controller_or_affiliate`) and writes
   `index.json`. `rows` is the number of exported rows (positions), which the
   Worker checks; `lines` is the register lines behind them:

```json
{
  "schema": "idx.ownership.mirror.v1",
  "generated_at": "2026-11-07T01:17:40Z",
  "source_page": "https://www.idx.co.id/id/perusahaan-tercatat/data-kepemilikan-saham/",
  "releases": [
    { "as_of": "2026-09-30", "title": "Pemegang Saham di Atas 1% per 30 September 2026",
      "format": "xlsx", "source_url": "https://www.idx.co.id/Media/.../peng-2026-09-00024-satu-persen.xlsx",
      "file": "ksei-above1-2026-09-30.xlsx", "sha256": "804b3bb7…", "bytes": 563730,
      "rows": 7151, "issuers": 961, "entities": 5145,
      "lines": 7154, "positions": 7151, "other_date_rows": 0, "controller_column": false,
      "parts": [ { "file": "ksei-above1-2026-09-30.part01.json", "sha256": "…", "rows": 1000 } ] }
  ]
}
```

5. uploads only what changed, `index.json` last, and deletes assets the new
   index no longer lists. The release is a prerelease so it never becomes the
   repository's "latest" release.

The web terminal's Worker (`web/src/ownership-sync.js`) loads the parts into
D1 on its minute cron, newest month first, one part per minute, within a daily
rows-written budget (`OWN_DAILY_WRITE_BUDGET`, default 60,000, inside D1's free
tier). It keeps `OWN_KEEP_RELEASES` months (default 6). A month is published
only after its row count matches the index; a re-published IDX file (new
SHA-256) is reloaded, and so is a month whose parts changed in an index
written by idx 0.8.2 or later (`parts_key`), so every kept month reloads once
after the mirror is rebuilt with summed accounts. `/api/ownership/status`
shows progress. There is no upload endpoint: the only input is the
repository's own release.

Running the workflow by hand: Actions → "ownership data" → Run workflow
(tick "dry run" to parse without uploading). From any machine idx.co.id
accepts, with `gh` logged in:
`pip install curl_cffi && python scripts/ownership_mirror.py --idx target/debug/idx --repo fzn0x/idx`
(add `--dry-run` to change nothing).

## Database

Default path: `%LOCALAPPDATA%\idx\ownership.sqlite` on Windows,
`$XDG_DATA_HOME/idx/ownership.sqlite` or `~/.local/share/idx/ownership.sqlite`
elsewhere. Override with `IDX_OWNERSHIP_DB` or `ownership.db` in the config.

Tables (schema version 3, kept in `PRAGMA user_version`):

- `releases` one row per imported month (`as_of` unique; re-importing a month
  replaces it), with the source path/URL and SHA-256 of the file, the register
  lines stored (`row_count`), the `positions` built from them, the
  `other_date_rows` left out and whether the file had `CONTROLLER_OR_AFFILIATE`
  (`controller_column`).
- `entities` one row per normalised holder: upper-cased alphanumerics, single
  spaces, legal-form tokens `PT`, `TBK`, `PERSERO` removed. `PT ALAMTRI
  RESOURCES INDONESIA TBK` and `Alamtri Resources Indonesia, Tbk.` share one
  entity.
- `aliases` every raw spelling seen, pointing at its entity.
- `holding_lines` every register line as printed, keyed by (release, sheet
  row), with its own `report_date`, entity, investor type, controller flag,
  local/foreign, nationality, domicile, scripless/scrip/total shares and
  percent. Nothing is dropped here.
- `holdings` the positions every query reads: one row per (release, symbol,
  entity), built from the release's lines dated its `as_of`. A holder's
  lines are summed (shares and percent; `accounts` counts them); the holder's
  spelling is the largest line's, and type, controller flag, local/foreign,
  nationality and domicile come from the largest line that has a value.
- `resolutions` your spelling rules from `idx ownership resolve` (version 3):
  `merge` (one holder key folded into another) or `map` (one spelling moved to
  a holder), the holder it now belongs to, the issuer-months combined with
  `--fold` and when it was made. Every import applies them.

Why one holder has several lines: KSEI computes holdings per SID, and per
account for scrip held without an SID (the register's own disclaimer). A
holder typically appears once for its scripless C-BEST position and once for a
scrip (eBAE) account, often spelled differently and without the L/A flag or
investor type on the scrip line. idx 0.8.1 and older kept only the last of
those lines; for example KREN's KRESNA PRIMA INVEST showed 4.01 % instead of
9.44 % + 4.01 % = 13.45 % on 30 Sep 2026.

Percentages are rounded to four decimals on import to remove binary noise in
the source (`1.1499999999999999` becomes `1.15`). Numbers stored as text are
read per column: a share count may use `.`, `,` or spaces as thousands
separators and an all-zero decimal tail (`2.500.000,00`); in a percentage a
single `.` or `,` is the decimal mark (`7,19` is 7.19, `1,000` is 1.0). A
percentage outside 0 to 100 or a fractional share count skips the row with a
warning naming the cell. Nothing else is altered.

Upgrades: opening a database migrates it in place (one transaction; a file
written by a newer idx is refused). Months imported by idx 0.8.1 or older stay
readable but have no lines: their `positions` are blank, `accounts` is
`null`, and every query that reads them warns until they are re-imported with
`idx ownership import --all --force` (or `--file`).

## Queries

```sh
idx ownership ticker BBCA                       # holders above 1 %, newest month
idx ownership ticker BBCA --as-of 2026-07-31
idx ownership entity "Dwimuria Investama"       # every position of one holder
idx ownership changes --from 2026-07-31 --to 2026-08-31 --symbol BBCA
idx ownership changes --from 2026-02-27 --to 2026-08-31 --min-change 1 --limit 0
idx ownership changes --from 2026-07-31 --to 2026-08-31 --limit 100 --offset 100   # page 2
idx ownership foreign BBCA                      # foreign vs local share, month by month
idx ownership foreign                           # every issuer's foreign change, newest two months
idx ownership releases
```

`ticker` and `entity` list positions: a holder on several register lines
(scrip and scripless accounts) is one row with `accounts` > 1 (an `Accts`
column in the table). From the 30 Oct 2026 register each row also carries
`controller_or_affiliate`, and `controller_reported` says whether the release
has that column (a `Control` column in the table).

`changes` compares two imported months per (symbol, entity), each side's
accounts summed: `new` holders (absent before: crossed 1 % or newly listed),
`exited` holders (absent after) and `changed` positions, sorted by absolute
percentage-point move. Rows carry `local_foreign` (`L`/`A`), `accounts_from`,
`accounts_to` and `controller_or_affiliate`. A holder with the same shares on
both sides is not a change when its percent did not move, or, when a side has
several lines, moved by no more than their rounding (each line is printed to
0.01 pp, so `n` summed lines can drift by 0.005 × `n` pp: scrip converted to
scripless). With one line on each side any move counts, since the issued
share count changed. The `summary` block is computed over every match
before `--limit`/`--offset` paging: counts of new, exited, increased and
decreased holders, net percentage points split by foreign/local and by KSEI
investor type, per-issuer flow (with the foreign part), and the top rises and
falls. For a single symbol the net figures are exact; across the register they
are a flow indicator.

`foreign` sums the `L`/`A` flag KSEI prints on each listed holder. With a
symbol it gives one issuer's disclosed foreign percent, holder counts and
month-over-month change per release; without one it ranks every issuer by the
change in disclosed foreign stake between two releases. Sums cover holders
above 1 % only, so this is the *disclosed* foreign stake, not total foreign
ownership.

JSON schemas: `idx schema ownership-holders|ownership-entity|ownership-changes|
ownership-foreign|ownership-foreign-flow|ownership-releases|ownership-import|
ownership-discover|ownership-search|ownership-cross-holders|
ownership-concentration|ownership-graph|ownership-resolve-candidates|
ownership-resolution|ownership-resolutions`.

## Finding a holder

```sh
idx ownership search salim                     # case-insensitive, any spelling
idx ownership search "salim anthoni"           # words in any order
idx ownership search "uob kay hian" --limit 5
idx ownership entity --exact "ANTHONI SALIM"   # one holder's positions, by key
```

`search` matches the normalised name (`entity_key`) and every spelling KSEI
printed in any imported month. Each match says how it matched: `exact` (the
query normalises to the key), `spelling` (one of the printed spellings, case
and spacing ignored), `prefix`, `contains`, or `words` (every query word
starts a word of the key or a spelling, in any order). Exact and spelling
matches come first, then holders listed in the selected release (`--as-of`,
default newest) before those that left the register, then prefix, contains
and word matches, then the holders with more issuers. A query needs 2 letters
or digits. `entity --exact` takes the key and lists that one holder, where
plain `entity` lists every holder whose key contains the name.

## Holders across issuers

```sh
idx ownership cross-holders                      # holders above 1 % in at least 2 issuers
idx ownership cross-holders --type individual    # people only
idx ownership cross-holders --origin foreign --min-issuers 5
```

A holder's accounts in one issuer are summed first, so an issuer counts once.
`total_pct` adds stakes in different companies: it measures breadth and is
not a share of anything. `--type` is a case-insensitive substring of KSEI's
investor type (`mutual fund` matches `MUTUAL FUNDS`), `--origin` the `A`/`L`
flag; a holder without a type or flag never passes those filters. Custodian
and nominee accounts (securities companies, private banks) lead the ranking
because the register names the account holder, not the beneficial owner: on
30 Sep 2026 UOB KAY HIAN PRIVATE LIMITED is listed in 64 issuers. A holder
printed two ways counts as two until you merge them (below).

## Concentration

```sh
idx ownership concentration                     # most concentrated first, by HHI
idx ownership concentration --ascending         # most widely held first
idx ownership concentration --by top1 --limit 50
idx ownership concentration BBCA BBRI TLKM
```

Definition, per issuer and release:

- Holders: those listed above 1 % for the issuer, each holder's accounts
  summed into one position.
- `hhi` = the sum of the squared percentages of those holders, on the 0-100
  scale, so it runs from 0 to 10,000 (10,000 means one holder owns
  everything). Nothing is rounded before the last step.
- `top1_pct` and `top_holder` (ties by name), `top3_pct` (the three largest,
  fewer when fewer are listed), `listed_pct` (their sum), `holders_listed`.
- `undisclosed_pct` = max(0, 100 - `listed_pct`): what the register does not
  show. It is left out of the HHI (treated as fully dispersed), so the HHI is
  a lower bound of the full figure. It is not IDX's free float, a narrower
  regulatory figure.
- `controller_pct`: the stakes flagged CONTROLLER or AFFILIATE OF THE
  CONTROLLER, from the 30 Oct 2026 register on (`null` for earlier releases).

The reference CLI 0xrsydn/idx-cli uses the same definition over register rows;
idx sums a holder's accounts first (two accounts of 9.44 % and 4.01 % add
180.9 points, not 105.2), keeps the decimals (BBCA scores 3,021.91 rather than
3,021 on 30 Sep 2026) and calls the remainder `undisclosed_pct`. No upper
bound is published: KSEI lists accounts, so a holder below 1 % per account is
not guaranteed below 1 % in total.

## Network graph

```sh
idx ownership graph BBCA                       # holders of BBCA and their other holdings
idx ownership graph "Anthoni Salim" --depth 3
idx ownership graph AADI --min-pct 5           # major stakes only
idx ownership graph BBRI --dot | dot -Tsvg -o bbri.svg
```

The register is a bipartite graph: holders on one side, issuers on the other,
a link for each holding above 1 %. The walk starts at a ticker (or, when ROOT
is not a ticker with holders or `--holder` is given, at the holder whose key
ROOT normalises to, else at the best `search` match, with a warning naming
it) and goes breadth-first for `--depth` hops (1-6, default 2): from a ticker,
hop 1 is its holders and hop 2 their other issuers. Within a hop the strongest
links come first, so `--max-nodes` (default 100, at most 1000) keeps the
important part; reaching it sets `truncated` and warns. Links are every
holding between two reached nodes. Node ids are `ticker:<SYMBOL>` and
`holder:<entity_key>`, the same in every database. `--dot` prints a
deterministic Graphviz digraph (holders as ellipses, issuers as boxes, the
percent on each edge, the root in bold) and wins over `-o`; `-o csv` prints
the links.

Sizes on the 30 Sep 2026 register: BBCA at depth 2 has 13 nodes; DRS LO KHENG
HONG has 106 at depth 2 and 1,774 at depth 4; the custodian UOB KAY HIAN
reaches 452 at depth 2 (44 with `--min-pct 5`).

## Spelling changes and merges

KSEI sometimes prints one holder differently from month to month, and
`changes` then shows an exit plus a new holder. Between 31 Aug and 30 Sep
2026, SURONO SUBEKTI became DRS SURONO SUBEKTI (18 issuers), UOB KAY HIAN PTE
LTD became UOB KAY HIAN PRIVATE LIMITED (54 issuers), and in all 232 of the
1,451 change rows idx 0.8.1 reported were one name leaving an issuer and
another entering it with exactly the same share count.

```sh
idx ownership resolve candidates                # suggestions with their evidence
idx ownership resolve merge "DRS SURONO SUBEKTI" "SURONO SUBEKTI" --dry-run
idx ownership resolve merge "DRS SURONO SUBEKTI" "SURONO SUBEKTI"
idx ownership resolve map "IR. T. PERMADI RACHMAT" "T PERMADI RACHMAT"
idx ownership resolve list
idx ownership resolve undo 1
```

`candidates` (also `unresolved`) pairs holders on two kinds of evidence:
`renamed`, the same share count left one name and reached the other in the
same issuer between two consecutive imported months; and `similar_name`, the
same words once legal forms (`PT`, `TBK`, `LTD`, `PTE`, `LIMITED`,
`CORPORATION`, ...) and titles (`DRS`, `DR`, `IR`, `H`, `HJ`, ...) are dropped,
in any order (names of one word are left out). Pairs listed in one issuer in
one month are marked `listed_together`. These are suggestions, and both kinds
have false positives: an identical share count is also what a real block sale
looks like (MANDOM CORPORATION to KALON MIDCO HOLDINGS CO LTD in TCID), and
holders listed together are often different people (CAUDLE GAVIN ARNOLD and
GAVIN ARNOLD CAUDLE in PALM).

`merge KEEP MERGE...` moves every spelling and register line of each MERGE
holder to KEEP, in every imported month, and every later import follows (a
new spelling whose key was merged away joins KEEP too). `map SPELLING HOLDER`
moves one printed spelling (`--new` makes HOLDER a new holder). Holders are
named by key or display name, exactly; `search` shows both and every
spelling. When both are listed in the same issuer in one month the command
stops unless you pass `--fold`, which adds the stakes up. `--dry-run` reports
the counts and writes nothing. `undo ID` deletes a rule and moves back what it
moved; months with register lines are rebuilt from them, so even a fold
splits again exactly. A merge of a holder that had been merged into earlier
re-points those rules too, so undo reverts exactly one rule.

The rules live in the ownership database (`resolutions`), so they survive
re-imports (`import --all --force` re-applies them) but not a new database
file. Months imported by idx 0.8.1 or older have no register lines: a rule
edits their positions in place, a fold there cannot be split again, and a
warning asks to re-import them. The rules are local to the CLI and the MCP
server reading this file; the GitHub mirror and the web terminal keep the
automatic grouping.

## Caveats

- Holders below 1 % are not published, so `listed_pct` is below 100 %.
- Entity grouping is heuristic: it merges spelling variants, not legally
  distinct vehicles with similar names. The raw spelling is kept on every
  line. Two different holders whose names normalise alike in one ticker are
  summed into one position (they used to replace each other). Titles
  (DRS, IR) and word order are not normalised; `resolve` merges those by hand.
- The register names account holders: custodians and nominees (securities
  companies, private banks) hold for clients, so cross-holder rankings and
  graphs show them as hubs.
- The web terminal's `ownership search` covers the selected month only (its
  spellings), and it has no `resolve`: your rules stay in your database.
- Attributes of a summed position come from its largest line that has them:
  a scrip line without an L/A flag counts with its holder's flagged line in
  `ownership foreign`.
- Rows dated other than the release's `as_of` are kept but never summed into
  it; the import, `releases` (`other_date_rows`) and a not-found message say
  so. None of the 2026 IDX files mixes dates.
- `CONTROLLER_OR_AFFILIATE` comes from the issuer's Form E009 for the previous
  month, forwarded by IDX to KSEI, so it can lag a month.
- PDF imports depend on the text layer of the announcement; numbers are
  checked but a reformatted PDF may need the parser updated.
