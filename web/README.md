# web/

Cloudflare Worker behind **https://ihsg.midoriresearch.com**.

- `public/` the market page (`index.html`) and the browser terminal (`terminal.html`)
  in Bahasa Indonesia, their English copies under `public/en/`, and the 404 page in
  both; plain HTML/CSS/ES modules, no bundler. `public/i18n.js` reads the page's
  `<html lang>` and gives the scripts their words and number style (5.050,43 or
  5,050.43); `public/styles.css` holds the Halaman Bursa tokens for the light and
  dark themes. The two home pages share one structure: change both together, and
  `test/tools.test.mjs` and `test/seo.test.mjs` check their command tables,
  language links and search tags.
- `public/route.js` reads a typed line before `public/terminal.js` runs it: the
  terminal's own lines pass through untouched; a line in the CLI's words (`idx
  stocks quote BBCA`, `idx -o json market overview`, the CLI's short flags) runs
  as the page's own, with a note saying so; and what the page cannot run (an MCP
  tool with `web: null`, a CLI-only command such as `watch`, a CLI flag without a
  twin here) answers with a notice: the CLI line with the user's arguments in
  the CLI's own words (`CLI_FORM`: bandar BBCA is `idx bandar detect BBCA`), the
  reason in the page's language, the install lines and the page's nearest
  commands. `test/route.test.mjs` checks it against the CLI's command tree in
  `src/cli/`, and `test/route-cli.test.mjs` reads the clap definitions there
  into a model and parses every CLI line a notice prints, as clap would.
- Docs: `scripts/docs.mjs` builds `/docs/` (README.md), `/docs/<page>` (one
  per `docs/*.md`), the same pages as Markdown at `/docs/<page>.md` with
  absolute links, `/llms.txt` (llmstxt.org index) and `/llms-full.txt` (every
  page in one file) into `public/docs/`, `public/llms.txt` and
  `public/llms-full.txt`. Those are generated and gitignored: wrangler's
  `[build]` step runs the script before every `wrangler dev` and
  `wrangler deploy`, and `node web/scripts/docs.mjs` (or `npm run docs` here)
  writes them for a local look. `scripts/md.mjs` is the small Markdown
  renderer behind it (no dependencies). The pages load only `public/docs.css`
  (the site's Halaman Bursa tokens, no script). The Worker serves `/docs*` and
  `/llms*.txt` (`run_worker_first`) to give the `.md` and `.txt` files
  `charset=utf-8` and `access-control-allow-origin: *`. A new `docs/*.md`
  file goes into `PAGES` in `scripts/docs.mjs` and `public/sitemap.xml`;
  `test/docs.test.mjs` fails until it does, and checks every page's tags,
  links and anchors, the llms.txt shape and that the exit codes agree with
  docs/json-output.md.
- `src/worker.js` entry: host check, security headers, `/api/*`, cron. Its
  `scheduled()` returns `runCron` (`src/cron.js`): the board refresh and the
  KSEI register sync, then a step of the bandar scan, inside one subrequest
  budget per invocation (`src/budget.js`: fetches, a GitHub release download
  counting two, and D1 queries). Fetches stop at 44 with the D1 queries
  counted, and the scan step's final D1 batch may add one, below the Free
  plan's 50.
  `src/headers.js` holds the security headers (CSP, nosniff, frame and
  referrer policy); `public/_headers` gives the same set to the static assets,
  which the assets layer serves without running the Worker.
  `test/headers.test.mjs` keeps the two equal and checks that every page
  works under the CSP (no inline scripts or handlers, styles only from this
  site and Google Fonts).
- `src/api.js` endpoints returning the CLI's JSON envelopes; D1-backed cache.
- `src/ipot.js` the IPOT broker summary and corporate actions (port of
  `src/providers/ipot`, names from `src/brokers.json`, generated from
  `data/brokers.csv`).
- `src/holidays.js` the trading holidays (port of `src/holidays.rs`;
  `src/holidays.json` is `data/holidays.csv`, kept equal by a test).
  The idx.co.id commands (market-wide foreign flow, market brokers, calendar
  and indices) stay CLI and MCP only: idx.co.id refuses
  requests from Cloudflare Workers. The web `foreign` reads a stock's foreign
  flow from IPOT instead (`src/ipot.js`).
- `src/bandar.js` the bandarmology rules (port of `src/analysis/bandar.rs`,
  the same reasons text pinned in both test suites); `src/bandar-scan.js` the
  scan behind `bandar screen` and `bandar broker`. From 16:45 WIB on a trading
  day of the bundled calendar the minute cron ranks the session's 100 most
  traded stocks (by value, quotes dated that session; once the next session
  has opened, those that traded since come first) and probes the one-day IPOT
  page of the first, then of the second and third in turn, skipping a code
  IPOT does not know. IPOT shows a closed session's totals without its broker
  lists until it publishes them, so the scan waits and probes again, every 10
  minutes until 21:00 WIB, every 30 after and every hour once a day has
  passed; a probe that fails is tried again after 5 minutes, doubling up to
  that gap, and an hour of failures logs an error each hour. Only a page
  without trades on a day the quote board shows none either closes the
  session. Then it reads up to three stocks a minute (fewer when IPOT is
  slow), inside the minute's first 45 seconds, each over 1d, 1w, 1m and 3m
  plus its daily candles, into the `bandar_scan` table
  (`migrations/0003_bandar.sql`; once a session's first pass ends, the newest
  two sessions and the one shown are kept), rows and state in one D1 batch,
  the state written only over the revision it read. A stock IPOT has not
  published (totals without lists, or no trades while the board shows it
  trading) is read again on the probe's schedule; one that failed is tried
  again in up to eight rounds, 10, 30, 60 and 120 minutes apart and then
  every 4 hours. Once the first pass is over the session is filed with what
  it got, again when a later read changes that, and for good at the next
  session's 16:45; it is shown once it holds half its stocks or more than the
  one shown (a session shown from before these records counts as holding
  none, so the first session scanned with them replaces it once it is filed
  with a stock stored), and the screen's and the broker's warnings say which
  session they show and why.
  `/api/bandar/status` shows the progress and the last record (`done_at` once
  the session is done, `filed_at` when its record was last filed); a
  maintainer can force a step with `/api/refresh?token=…&what=bandar` (a
  rescan of the shown session reads the same stocks again and keeps its
  record until it has stored as many).
  `/api/bandar` itself is live like
  `/api/broker`: up to 5 tickers, `daily` for one ticker and at most 23
  sessions.
- `public/chart.js` the price chart under a single-symbol `quote`: ranges 1D
  to 5Y from `/api/history`, closes over volume, a dashed previous close on
  1D, a crosshair readout (pointer, touch or arrow keys). Its layout
  functions are tested in `test/chart.test.mjs` on the recorded Yahoo charts.
- `src/yahoo.js` the Yahoo endpoints and parsers; `src/analysis.js` ports of
  the Rust indicators, verdict, growth/valuation/risk lenses, screener DSL and
  sentiment lexicon; `src/extras.js` ports of returns (with beta against a
  benchmark), dividends, news ordering and the ownership changes summary;
  `src/msn-insights.js` (ticker lookup, insights, peers) and `src/msn-screen.js`
  (the whole-market return windows, one POST) port `src/providers/msn` and are
  tested on the same `tests/fixtures/msn`. `/api/fundamental?lens=growth|valuation|risk`
  returns a lens, as MCP `idx_fundamental` does with `lens`.
- Endpoints under `/api`: `market`, `quote`, `history`, `intraday`, `profile`,
  `fundamental`, `earnings`, `financials`, `dividends`, `returns`, `technical`,
  `brief`, `compare`, `screen` (`preset`, `returns`), `news`, `sentiment`,
  `insights`, `broker`, `bandar`,
  `bandar/screen`, `bandar/status`, `bandar/broker`, `actions`, `holidays`,
  `instruments`, `members`, `ownership/ticker`,
  `ownership/entity`, `ownership/changes`, `ownership/foreign`,
  `ownership/releases`, `ownership/search`, `ownership/cross-holders`,
  `ownership/concentration`, `ownership/graph`, `universe`.
- `public/tools.js` the MCP tools with their CLI and web commands: the
  terminal's `tools` command and help buttons and the landing page's Daftar
  Perintah table use it; `test/tools.test.mjs` checks it against docs/mcp.md.
  `bandar/broker` reads the bandar scan; `instruments` and `members` serve the
  bundled lists.
- `migrations/` D1 schema. `test/` runs with `node --test web/test/*.mjs`.
- Search and sharing: canonical, Open Graph and Twitter tags on every page,
  JSON-LD on the landing page, `public/robots.txt` and `public/sitemap.xml`;
  API responses carry `x-robots-tag: noindex`. The share card
  `public/og.png` (1200x630) and `public/apple-touch-icon.png` are rendered
  from `og/card.html` and `og/icon.html` by `node web/scripts/og.mjs`
  (headless Chrome, Node 22+); re-run it after editing either source.
  `test/seo.test.mjs` checks the tags against the files.
- The cron (`* * * * *`) refreshes the whole quote board as one JSON row and
  the IHSG tape every minute in IDX hours, every 15 minutes otherwise (IDX
  holidays count as closed), which keeps writes far inside D1's free tier. A
  board older than that cadence allows (10 minutes open, 35 closed) is served
  as `stale` with a warning giving its time.
- Secrets: `ADMIN_TOKEN` (`wrangler secret put`) lets a maintainer force a
  refresh with `/api/refresh?token=…`.
- Ownership on the web: `src/ownership-sync.js` loads the KSEI register from
  the repository's `ownership-data` release (published by
  `.github/workflows/ownership.yml`) on the minute cron: index checked every
  30 minutes, one SHA-256-verified part per minute, newest month first, within
  `OWN_DAILY_WRITE_BUDGET`. No upload endpoint exists. Progress:
  `/api/ownership/status`; a maintainer can force a step with
  `/api/refresh?token=…&what=ownership`. Each holding is one holder's
  register lines summed by idx (`accounts`, plus `controller_or_affiliate`
  from the 30 Oct 2026 register; `migrations/0004_ownership_accounts.sql`).
  `ownership/search`, `cross-holders`, `concentration` and `graph` run
  `src/ownership.js` (the port of `src/ownership/analytics.rs`) over one
  month of `own_holdings`, one D1 read of the release each; search covers
  that month's spellings only, and there are no local `resolve` rules.
  A month reloads when its IDX file changes or, for an index written by idx
  0.8.2 or later, when its parts change (`parts_key`): after the mirror is
  rebuilt every kept month reloads once, newest first. Loading a month is
  about 21,600 writes and deleting the copy D1 holds costs the same, so a
  reload is about 43,000 and the budget (counted per UTC day, as D1 counts
  it) charges both. A reload starts only when it fits in what is left of the
  day, so its month is hidden for minutes, not until the next reset; with the
  default 60,000 that is one reload a day, six days for six months, each
  month readable (as its older copy) until then.

Access is domain-only: `workers_dev = false`, `preview_urls = false`, and the
Worker returns 404 for any Host other than `PUBLIC_HOST` (in `wrangler dev`,
send `Host: ihsg.midoriresearch.com` to reach `/api`, `/healthz` and the docs).

```sh
cd web
npx wrangler dev      # local; builds the docs first
npx wrangler d1 migrations apply idx --remote   # new tables first (0004 adds ownership accounts)
npx wrangler deploy   # needs `wrangler login` on the Midori Research account; builds the docs first
```

After a deploy, the docs answer like this:

```sh
curl -sI https://ihsg.midoriresearch.com/llms.txt      # 200, text/plain; charset=utf-8
curl -sI https://ihsg.midoriresearch.com/docs/mcp.md   # 200, text/markdown; charset=utf-8, access-control-allow-origin: *
curl -sI https://ihsg.midoriresearch.com/docs/mcp      # 200, text/html with the CSP
curl -sI https://ihsg.midoriresearch.com/docs          # 307 to /docs/
curl -sI https://ihsg.midoriresearch.com/              # 200 with the CSP (public/_headers)
```
