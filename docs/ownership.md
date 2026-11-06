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
   carries on with the months already mirrored, and fails the run if the
   index lists a month whose file is missing from the release (it would
   otherwise drop that month);
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
rows-written budget (`OWN_DAILY_WRITE_BUDGET`, default 60,000 per UTC day,
inside D1's free tier of 100,000). It keeps `OWN_KEEP_RELEASES` months
(default 6). A month is published only after its row count matches the index;
a re-published IDX file (new SHA-256) is reloaded, and so is a month whose
parts changed in an index written by idx 0.8.2 or later (`parts_key`), so
every kept month reloads once after the mirror is rebuilt with summed
accounts. A reload first deletes the copy D1 holds, which costs as many writes
as loading it (about 21,600 each way for one month) and hides the month until
the new copy is in, so the budget charges deletes too and a reload starts
only when both fit in what is left of the day. With the default budget that
is one reload a day; months not reloaded yet stay readable as their older
copy, with a warning. `/api/ownership/status` shows progress. There is no
upload endpoint: the only input is the repository's own release.

Running the workflow by hand: Actions → "ownership data" → Run workflow
(tick "dry run" to parse without uploading). From any machine idx.co.id
accepts, with `gh` logged in:
`pip install curl_cffi && python scripts/ownership_mirror.py --idx target/debug/idx --repo fzn0x/idx`
(add `--dry-run` to change nothing).

## Database

Default path: `%LOCALAPPDATA%\idx\ownership.sqlite` on Windows,
`$XDG_DATA_HOME/idx/ownership.sqlite` or `~/.local/share/idx/ownership.sqlite`
elsewhere. Override with `IDX_OWNERSHIP_DB` or `ownership.db` in the config.

Tables (schema version 2, kept in `PRAGMA user_version`):

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
ownership-discover`.

## Caveats

- Holders below 1 % are not published, so `listed_pct` is below 100 %.
- Entity grouping is heuristic: it merges spelling variants, not legally
  distinct vehicles with similar names. The raw spelling is kept on every
  line. Two different holders whose names normalise alike in one ticker are
  summed into one position (they used to replace each other).
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
