# web/

Cloudflare Worker behind **https://ihsg.midoriresearch.com**.

- `public/` landing page (`index.html`) and the browser terminal (`terminal.html`),
  plain HTML/CSS/ES modules, no bundler.
- Docs: `scripts/docs.mjs` builds `/docs/` (README.md), `/docs/<page>` (one
  per `docs/*.md`), the same pages as Markdown at `/docs/<page>.md` with
  absolute links, `/llms.txt` (llmstxt.org index) and `/llms-full.txt` (every
  page in one file) into `public/docs/`, `public/llms.txt` and
  `public/llms-full.txt`. Those are generated and gitignored: wrangler's
  `[build]` step runs the script before every `wrangler dev` and
  `wrangler deploy`, and `node web/scripts/docs.mjs` (or `npm run docs` here)
  writes them for a local look. `scripts/md.mjs` is the small Markdown
  renderer behind it (no dependencies). The pages load only `public/docs.css`
  (the site's station-board tokens, no script). The Worker serves `/docs*` and
  `/llms*.txt` (`run_worker_first`) to give the `.md` and `.txt` files
  `charset=utf-8` and `access-control-allow-origin: *`. A new `docs/*.md`
  file goes into `PAGES` in `scripts/docs.mjs` and `public/sitemap.xml`;
  `test/docs.test.mjs` fails until it does, and checks every page's tags,
  links and anchors, the llms.txt shape and that the exit codes agree with
  docs/json-output.md.
- `src/worker.js` entry: host check, security headers, `/api/*`, cron.
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
- Search and sharing: canonical, Open Graph and Twitter tags on every page,
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
  `/api/refresh?token=…&what=ownership`.

Access is domain-only: `workers_dev = false`, `preview_urls = false`, and the
Worker returns 404 for any Host other than `PUBLIC_HOST` (in `wrangler dev`,
send `Host: ihsg.midoriresearch.com` to reach `/api`, `/healthz` and the docs).

```sh
cd web
npx wrangler dev      # local; builds the docs first
npx wrangler d1 migrations apply idx --remote   # new tables first (0003 adds bandar_scan)
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
