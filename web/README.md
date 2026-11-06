# web/

Cloudflare Worker behind **https://ihsg.midoriresearch.com**.

- `public/` landing page (`index.html`) and the browser terminal (`terminal.html`),
  plain HTML/CSS/ES modules, no build step.
- `src/worker.js` entry: host check, security headers, `/api/*`, cron.
- `src/api.js` endpoints returning the CLI's JSON envelopes; D1-backed cache.
- `src/ipot.js` the IPOT broker summary and corporate actions (port of
  `src/providers/ipot`, names from `src/brokers.json`, generated from
  `data/brokers.csv`).
- `src/holidays.js` the trading holidays (port of `src/holidays.rs`;
  `src/holidays.json` is `data/holidays.csv`, kept equal by a test).
  The idx.co.id commands (foreign flow, market brokers, calendar, indices,
  stock list, members) stay CLI and MCP only: idx.co.id refuses requests from
  Cloudflare Workers.
- `src/bandar.js` the bandarmology rules (port of `src/analysis/bandar.rs`,
  the same reasons text pinned in both test suites); `src/bandar-scan.js` the
  nightly scan behind `bandar screen`: from 16:45 WIB on a session day the
  minute cron reads the session's 100 most traded stocks (by value, from the
  quote board), four per minute, each over 1d, 1w, 1m and 3m plus its daily
  candles, into the `bandar_scan` table (`migrations/0003_bandar.sql`, the
  newest two sessions kept). A holiday is noticed by a probe and skipped; a
  stock whose fetch failed is tried once more at the end of the pass.
  `/api/bandar/status` shows the progress; a maintainer can force a step with
  `/api/refresh?token=…&what=bandar`. `/api/bandar` itself is live like
  `/api/broker`: up to 5 tickers, `daily` for one ticker and at most 23
  sessions.
- `public/chart.js` the price chart under a single-symbol `quote`: ranges 1D
  to 5Y from `/api/history`, closes over volume, a dashed previous close on
  1D, a crosshair readout (pointer, touch or arrow keys). Its layout
  functions are tested in `test/chart.test.mjs` on the recorded Yahoo charts.
- `src/yahoo.js` the Yahoo endpoints and parsers; `src/analysis.js` ports of
  the Rust indicators, verdict, screener DSL and sentiment lexicon;
  `src/extras.js` ports of returns, dividends, news ordering and the
  ownership changes summary.
- Endpoints under `/api`: `market`, `quote`, `history`, `profile`,
  `fundamental`, `earnings`, `financials`, `dividends`, `returns`, `technical`,
  `brief`, `compare`, `screen`, `news`, `sentiment`, `broker`, `bandar`,
  `bandar/screen`, `bandar/status`, `bandar/broker`, `actions`, `holidays`,
  `instruments`, `members`, `ownership/ticker`,
  `ownership/entity`, `ownership/changes`, `ownership/foreign`,
  `ownership/releases`, `universe`.
- `public/tools.js` the 33 MCP tools with their CLI and web commands: the
  terminal's `tools` command and help buttons and the landing page's Daftar
  Perintah table use it; `test/tools.test.mjs` checks it against docs/mcp.md.
  `bandar/broker` reads the nightly scan; `instruments` and `members` serve the
  bundled lists.
- `migrations/` D1 schema. `test/` runs with `node --test web/test/*.mjs`.
- Search and sharing: canonical, Open Graph and Twitter tags on both pages,
  JSON-LD on the landing page, `public/robots.txt` and `public/sitemap.xml`;
  API responses carry `x-robots-tag: noindex`. The share card
  `public/og.png` (1200x630) and `public/apple-touch-icon.png` are rendered
  from `og/card.html` and `og/icon.html` by `node web/scripts/og.mjs`
  (headless Chrome, Node 22+); re-run it after editing either source.
  `test/seo.test.mjs` checks the tags against the files.
- The cron (`* * * * *`) refreshes the whole quote board as one JSON row and
  the IHSG tape every minute in IDX hours, every 15 minutes otherwise, which
  keeps writes far inside D1's free tier.
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
Worker returns 404 for any Host other than `PUBLIC_HOST`.

```sh
cd web
npx wrangler dev      # local
npx wrangler d1 migrations apply idx --remote   # new tables first (0004 adds ownership accounts)
npx wrangler deploy   # needs `wrangler login` on the Midori Research account
```
