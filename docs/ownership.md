# Ownership data

Since March 2026 KSEI compiles, and IDX publishes monthly, the list of every
shareholder holding **more than 1 %** of a listed company ("Pemegang Saham di
Atas 1%"). `idx ownership` imports those files into a local SQLite database
and answers questions from it offline.

KSEI also publishes, on its own site, a monthly composition of every
security's holders by investor type, local and foreign ("Kepemilikan Efek").
It names no holders; idx keeps it in separate tables and reads it with
`idx ownership composition` (see [KSEI composition](#ksei-composition-kepemilikan-efek)).

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
rejected with a clear message. `idx ownership discover --family` lists them
all:

| `--family` | title on the page | file | importable |
| --- | --- | --- | --- |
| `above1` (default) | Pemegang Saham di Atas 1% per 31 Agustus 2026 | idx.co.id `Media/.../peng-YYYY-MM-NNNNN-satu-persen.xlsx` | yes |
| `above5` | Pemegang Saham di Atas 5% per 23 September 2026 (a daily register: holders above 5 %, per account, with the day's change) | idx.co.id `peng-YYYY-MM-DD-NNNNN-lima-persen.xlsx` | no, listed only |
| `investor-type` | Data Kepemilikan Saham Perusahaan Tercatat per Tipe Investor per 31 Agustus 2026 | idx.co.id `peng-YYYY-MM-NNNNN-tipe.xlsx` | no, listed only |
| `classification` | Data Kepemilikan Saham Perusahaan Tercatat Berdasarkan Klasifikasi Investor per 31 Agustus 2026 | idx.co.id `peng-YYYY-MM-NNNNN-klasifikasi.xlsx` | no, listed only |
| `composition` | Kepemilikan Efek 30 Oktober 2026 | ksei.co.id `storage/Download/BalanceposEfekYYYYMMDD.zip` | yes |
| `all` | every family above | | |

```sh
idx ownership discover                                 # the above-1 % register, every month listed
idx ownership discover --family all --limit 2           # every family, newest two each
idx ownership discover --family composition --limit 24  # two years of KSEI archives
```

`--limit` counts per family (0: all). Without it every idx.co.id file is
listed, as before, and one ksei.co.id page (eight months). Each entry carries
`family` and `importable` in JSON. A family whose source fails becomes a
warning while the others answer; the command fails only when every family
failed (with the JSON code `blocked` when Cloudflare refused idx.co.id).

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

When Cloudflare refuses idx.co.id, with a challenge ("Just a moment...",
response header `cf-mitigated: challenge`) or a block page ("Attention
Required! | Cloudflare"), and no curl-impersonate binary gets through, the
error has the JSON code `blocked` (exit 3, as any network failure) and names
the kind of refusal and the ways out: retry later or from another network,
the mirror, curl-impersonate, or a download in a browser plus
`import --file`. `--latest`, `--all` and `discover` still fall back to the
mirror, and the warning says which refusal it was. Any other failure (an
outage, a 404) keeps its HTTP status and is not called Cloudflare.

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

## KSEI composition (Kepemilikan Efek)

KSEI lists one archive per month at
<https://www.ksei.co.id/id/publikasi/data-dan-statistik/kepemilikan-efek>
(eight months per page, newest first; `?page=N` for older ones), a few days
after the month's last trading day: `BalanceposEfek<YYYYMMDD>.zip`, about
125 KB, the date being that last trading day. The ZIP holds one
pipe-separated text file, `Balancepos<YYYYMMDD>.txt`, CRLF line endings:

```text
Date|Code|Type|Sec. Num|Price|Local IS|Local CP|Local PF|Local IB|Local ID|Local MF|Local SC|Local FD|Local OT|Total|Foreign IS|Foreign CP|Foreign PF|Foreign IB|Foreign ID|Foreign MF|Foreign SC|Foreign FD|Foreign OT|Total
30-OCT-2026|BBCA|EQUITY|123275050000|4980|2310613490|752250260|302459300|517672400|12030567470|1072720030|119977978|72753960|15254200|17194269088|666444425|575351467|4718402998|2382605437|324800480|20393323810|457281188|320342605|5420869622|35259422032
```

One row per security: shares listed (`Sec. Num`), the price, and the shares
held by local and by foreign holders in nine investor types (IS insurance, CP
corporate, PF pension fund, IB financial institution, ID individual, MF
mutual fund, SC securities company, FD foundation, OT others), each side with
its total. idx stores the `EQUITY` rows (1,005 on 30 Oct 2026) and counts the
other types (bonds, sukuk, warrants, rights, funds, crowdfunding) without
storing them. The header must match exactly; another layout is refused, not
guessed at.

**Scripless only.** The buckets cover shares held in KSEI accounts. Shares
still held as certificates (scrip) are outside the archive: BBCA's rows add
up to 42.55 % of its listed shares on 30 Oct 2026, because its controlling
holder's 54.94 % stake is scrip. Percentages are therefore taken over the
scripless total, and `scripless_pct` (scripless shares over listed shares) is
always shown next to them. Never read the composition as a percent of listed
shares.

```sh
idx ownership composition BBCA                     # newest month, nine types, local vs foreign
idx ownership composition TLKM --as-of 2026-09-30
idx ownership import --latest --family composition  # newest archive not imported yet
idx ownership import --all --family composition     # every listed month (up to 10 pages)
idx ownership import --file BalanceposEfek20261030.zip    # or the TXT inside
idx ownership import --url https://www.ksei.co.id/storage/Download/BalanceposEfek20261030.zip
```

`ownership composition` downloads the newest month on first use, and later
asks ksei.co.id again only once a newer month can exist (after the end of
the month following the newest imported one; the listing is cached 12 h).
`--as-of` downloads a listed month that is missing. `--offline` never
downloads. The envelope's `provider` is `ksei` when the call imported,
`local-db` otherwise, and a warning names what was imported. The report gives
the stock's local and foreign shares and percents, the nine types in KSEI's
order, and every imported month with the foreign percent's change in
percentage points (`idx.ownership.composition.v1`). `-o csv` writes one row
per investor type (`symbol, as_of, code, investor_type, local_shares,
foreign_shares, local_pct, foreign_pct`) however many months are imported;
the history is in the table and the JSON.

Composition imports never touch the register: `releases`, `ticker`,
`entity`, `changes` and `foreign` read exactly what they read before, and a
composition month and a register month can share a date. In an import report
a composition month has `family: "composition"`, `rows` and `issuers` = the
stocks stored, `entities` 0, `other_securities` the rows of other types, and
`positions`, `other_date_rows`, `controller_column` null.

Archives are read in memory, never unpacked to disk. A ZIP is refused when an
entry's path leaves the archive (absolute, a drive, `..`), when it holds more
than 16 entries, or when its text file unpacks to more than 64 MiB; a
download stops at 32 MiB. Rows with a bad cell (a date, a count with
separators, a non-positive `Sec. Num`) are skipped with a warning naming the
line; a row whose types do not add up to KSEI's total keeps the total, with a
warning.

ksei.co.id has no bot protection (nginx and S3, any user agent). The web
terminal does not carry the composition yet: a Worker has not been checked
against ksei.co.id, so the tool is CLI and MCP only.

## Database

Default path: `%LOCALAPPDATA%\idx\ownership.sqlite` on Windows,
`$XDG_DATA_HOME/idx/ownership.sqlite` or `~/.local/share/idx/ownership.sqlite`
elsewhere. Override with `IDX_OWNERSHIP_DB` or `ownership.db` in the config.

Tables (schema version 3, kept in `PRAGMA user_version`; version 3 adds the
two composition tables):

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
- `composition_releases` one row per imported KSEI composition month
  (`as_of` unique; re-importing replaces it), with the format (`zip` or
  `txt`), source, SHA-256, the stocks stored and the other securities left
  out.
- `composition` one row per (month, stock): shares listed, price, the nine
  local and nine foreign investor-type counts and both totals, as printed.

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
ownership-discover|ownership-composition`.

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
