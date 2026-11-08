# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.8.3] - 2026-11-13

### Fixed

- idx.co.id's Cloudflare challenge no longer fails a command at the first
  try: a challenged request is asked again for up to 30 s, a refusal serves
  the cached copy marked stale, an empty cached day no longer passes for a
  day without trading, and the `blocked` error says what to do and names the
  IPOT alternative, in CLI flags or MCP arguments (#103).
- `market brokers`: when idx.co.id refuses `--source idx`, the error names
  `--source ipot` (MCP `source: "ipot"`) where IPOT can answer, and `auto`
  no longer waits out idx.co.id's 30 s of challenge retries when IPOT can
  stand in (#113).
- `stockbit status` and `idx_stockbit_status` say whether the stored session
  can work: `logged_in` is false once both tokens have expired, with new
  optional `renewable`, `expired` and `reason`, and `stockbit login` signs in
  again instead of stopping as already signed in (#100).
- `stocks screen` marks a P/B over statements in another currency (USD
  reporters such as BSSR and GEMS) and names those companies; `pb` and `ps`
  filters and sorts read such ratios as missing. Quotes gain an optional
  `financial_currency`, screen rows `currency` and `financial_currency` (#108).
- Counted text agrees with its count in the brief, `stocks news`, the
  screen and other warnings ("1 current headline mentions the company")
  (#107).
- Web `bandar screen` and `bandar broker`: the scan waits for IPOT to publish
  each session instead of marking it a holiday, tries failed stocks again for
  hours, keeps the minute cron inside one subrequest budget, and its warnings
  say which session is shown and why (#102).
- Web terminal: offered commands in notices and the help table's usage lines
  wrap only at spaces (#105, #112); an answer that mixes cached and fetched
  parts reads "partly cached" instead of "live" (#106); verdict and sentiment
  labels read as in the CLI ("fairly valued", "insufficient data") (#111);
  the CSP allows Cloudflare's analytics beacon (#96).

### Added

- `broker --source auto|ipot|stockbit` and the MCP argument `source`: with a
  working Stockbit session (`idx stockbit login`), every broker that bought
  and sold, not only IPOT's top 10 per side; `auto` uses Stockbit when logged
  in, otherwise IPOT with a note. `idx.broker.v1` gains optional
  `buyer_count`, `seller_count`, `buyers_complete`, `sellers_complete` and
  `source` (#115).
- `market foreign --source idx|ipot` and the MCP argument `source`: the 100
  most traded stocks ranked by foreign net in rupiah, with IPOT's market
  totals, in the new schema `idx.market.foreign.value.v1`; the web terminal
  runs it from the bandar scan (#116).
- `market brokers --source auto|idx|ipot` and the MCP argument `source`:
  IPOT's market-wide broker ranking of its latest session, which `auto` uses
  when idx.co.id refuses, cannot be reached or fails; new optional
  `buy_value`, `sell_value`, `net_value`, `kind` and `rounded` (#104).
- Web terminal: CLI and MCP only commands explain why they do not run on the
  page, with the CLI line, its JSON form and the install command, in the
  page's language (#101).
- Web terminal: `market brokers` (`brokers`) runs on the page from IPOT's
  ranking, with the same envelope as `idx -o json market brokers --source
  ipot` (#114).
- `stockbit login --page` and the MCP page sign-in: once Stockbit accepts the
  session, the tab that sent it raises an alert and every tab open on the
  page turns to "Connected" with where to look next, the terminal or your
  agent (#117).
- README: install from npm as `ihsg-cli` (#95).

## [0.8.2] - 2026-11-07

### Fixed

- Fundamentals verdict: negative shareholders' equity scores against the
  company and sets the P/B, ROE and debt/equity rules aside, and multiples
  that divide an IDR price by USD statements are marked and not scored
  (new optional `financial_currency`) (#70).
- Ownership register: a holder's several KSEI accounts in one issuer are
  summed (`accounts`), every row keeps its own date, and the 30 Oct 2026
  controller/affiliate flag is stored (`controller_or_affiliate`); web D1
  migration `0004_ownership_accounts.sql` (#77).
- Stockbit order book: foreign buy/sell/net are left empty, with a warning,
  while Stockbit marks them as not current (`has_foreign_bs`) (#67).
- Tables print Yahoo's market state in words (`open`, `pre-market`,
  `after hours`, `closed`) instead of codes such as `POSTPOST`; JSON keeps
  the code (#89).

### Added

- Data freshness: quotes carry `behind_session` when their last trade
  predates the latest IDX session, and stale envelopes say why
  (`stale_reason`: `offline` or `fetch_failed`) (#71).
- CLI input and output: `-o jsonl`, opt-in `-o auto`, `-` reads tickers from
  stdin, `--quiet`, and other Yahoo Finance markets by suffix (`D05.SI`,
  `IDX_EXCHANGE`) in the Yahoo-backed commands (#74).
- `ownership search`, `cross-holders`, `concentration` (HHI), `graph` and
  `resolve`, with MCP tools `idx_ownership_search`, `idx_ownership_rankings`
  and `idx_ownership_graph` (#84).
- `ownership composition` from KSEI's monthly "Kepemilikan Efek" archive,
  `ownership discover --family`, and the JSON error code `blocked` (exit 3)
  when Cloudflare refuses a request; MCP `idx_ownership_composition` (#83).
- `ownership sync` and `ownership status`: a prebuilt SQLite snapshot
  published on the `ownership-data` mirror; MCP `idx_ownership_snapshot`
  (#82).
- `ownership institutions`: institutional and fund holders with
  buyer/seller signals from MSN; MCP `idx_ownership_institutions` (#69).
- `stocks insights` (MSN's rated statements and peers) and screener presets
  with MSN return-window keys (`ret1w`..`ret1y`, `range`); MCP
  `idx_insights` (#76).
- `stocks growth`, `stocks valuation` and `stocks risk` lenses, and a `lens`
  argument on `idx_fundamental` (#81).
- `history_provider` setting, earnings filters (`--annual`, `--quarterly`,
  `--reported`, `--estimates`) and several statements per
  `stocks financials --statement` (#72).
- `stocks foreign --source ipot` with `--from`/`--to`: foreign flow in
  rupiah from IPOT, also in the web terminal; a range's foreign net is
  rebuilt from single sessions, and a one-side broker summary reports what
  that side bought and sold (#78).
- `stocks intraday`: the latest session by the minute (1m and 2m bars), with
  the quote and VWAP; MCP `idx_intraday` (#75).
- Stockbit `actions`, `calendar` and `movements` on your own session, and a
  local sign-in page (`stockbit login --page`); MCP `idx_stockbit_actions`
  and `idx_stockbit_movements` (#85).
- `market astro`: the sky, weton and shio for IHSG with a descriptive
  backtest of its daily returns; MCP `idx_astro`. `--period max` with daily
  or weekly bars now returns them instead of Yahoo's monthly ones (#80).
- Nix flake with packages, apps, a dev shell and checks, built in CI (#66).
- `ihsg-mcp` npm package: `npx -y ihsg-mcp` runs `idx mcp` (#73).
- Web docs pages with Markdown twins, `llms.txt` and `llms-full.txt`, and
  one security header set for the Worker and static assets (#79).

### Changed

- Site redesign ("Halaman Bursa"): a bilingual market page, Bahasa
  Indonesia at `/` and English at `/en/`, with a stock search on the home
  page, the terminal at `/terminal`, plain page titles, search metadata
  (canonical and hreflang links, structured data, a sitemap) and a GitHub
  star button; the theme follows the device (#68, #89).
- A new idx mark and README logo in Material 3 Expressive style, with the
  wordmark IDX (#88, #89).
- The MCP server lists 47 tools, 36 in 0.8.1 (#84, #83, #82, #69, #76, #75,
  #85, #80).
- Minimum Rust version 1.89 (#66).
- The ownership database moves to schema version 4 (register lines,
  resolution rules, composition tables); older files are migrated on first
  open (#77, #84, #83).

## [0.8.1] - 2026-10-18

### Fixed

- Landing page and web terminal: the Stockbit tools (`idx_orderbook`,
  `idx_stockbit_status`, `idx_stockbit_login`) are now listed in the one
  shared tools source (`web/public/tools.js`) and `docs/mcp.md`, not only
  the landing-page table, so the three stay in step. The terminal's
  tool-list wording and count no longer call the CLI-only tools
  "idx.co.id" now that the Stockbit tools are among them. No change to the
  `idx` binary.

## [0.8.0] - 2026-10-18

### Added

- Stockbit realtime provider (opt-in, `--provider stockbit`): realtime
  quotes, history and a live order book from `exodus.stockbit.com`,
  carrying your own Stockbit session. It is the one source that needs a
  credential and the one not in the `auto` chain. See
  [docs/stockbit.md](docs/stockbit.md).
- `idx stockbit login` signs in through your browser and captures the
  session automatically over the Chrome DevTools Protocol (Chrome, Edge,
  Brave; `IDX_BROWSER` chooses one), reusing an already-signed-in profile
  when there is one. `--manual` pastes the `credentialStorage` cookie
  instead. `idx stockbit status` and `idx stockbit logout` manage the
  stored session; the tokens are written `0600`, refreshed automatically,
  and never printed to a log, an error or a cache key.
- `idx stockbit orderbook <SYMBOL>`: the full depth ladder (bids and asks,
  sizes in shares) plus session volume, value and foreign buy/sell/net.
  New `idx.orderbook.v1` JSON shape (`idx schema orderbook`).
- Three MCP tools (36 in total): `idx_orderbook`, `idx_stockbit_status`
  and `idx_stockbit_login`. Run the server with `--provider stockbit` for
  realtime quotes and history through the existing tools.
- Config `[stockbit] session` and the environment variables
  `IDX_STOCKBIT_SESSION`, `IDX_STOCKBIT_REFRESH_TOKEN`,
  `IDX_STOCKBIT_ACCESS_TOKEN` (seed an unattended session) and
  `IDX_BROWSER`.
- Landing page: the Stockbit tools are listed in the command table and the
  agents card, marked CLI and MCP only.

### Notes

- Stockbit's `/login/refresh` rotates the refresh token, so one session
  must have a single refresher: do not share one Stockbit login between
  `idx` and another tool, or they will log each other out.
- Realtime Stockbit data runs in the CLI and MCP only, not the web
  terminal, which cannot carry a personal account session.

## [0.7.2] - 2026-10-18

### Added

- CLI loading line: while a command waits on the network, one line on
  stderr shows the elapsed time and the hosts still pending, longest wait
  first (`⠹ 6s · waiting on investasi.kontan.co.id 6s, www.cnbcindonesia.com
  1s · 3 answered`), and IPOT's 404 retries as `www.indopremier.com retry 2`.
  It appears only after 300 ms, so cache hits print nothing, only when stderr
  is a terminal and without `-v`, so scripts, agents and `idx mcp` never see
  it. `IDX_PROGRESS=never` turns it off; `IDX_PROGRESS=always` shows it where
  stderr does not report a terminal (mintty). Ownership downloads report too.
- Web terminal loading state: a green stretch runs along the entry's rule
  and a line says what the command waits on (`waiting on the Kontan and CNBC
  Indonesia feeds · 6s`). Past 5 s, sources with known stalls say how the API
  bounds them: news, sentiment and brief name the 8 s feed deadline, broker,
  bandar and actions IPOT's 404 retries. The caption adds `took 7.5s` to any
  answer slower than a second.

### Fixed

- Web terminal: a revealed entry's last lines no longer sit under the sticky
  command prompt.

## [0.7.1] - 2026-10-18

### Fixed

- Release notes: the release workflow stopped at the empty `[Unreleased]`
  heading above the version, so GitHub releases v0.2.0 to v0.7.0 carried
  only the install line. It now takes the version's own changelog section
  (falling back to `[Unreleased]` only when there is none), and the notes of
  every earlier release have been filled in from this changelog.

## [0.7.0] - 2026-10-18

### Added

- idx.co.id provider (`src/providers/idx/`): the JSON endpoints behind the
  exchange's own pages, no key. A Cloudflare refusal is a provider error that
  says so; fixtures in `tests/fixtures/idx/`.
- `stocks foreign SYMBOL` and `market foreign`: foreign buying and selling per
  session and across the market (shares, as IDX publishes them, plus net ×
  close as a rupiah scale). MCP `idx_foreign`, `idx_market_foreign`.
- `market brokers`: every exchange member's day ranked by value, volume or
  frequency. MCP `idx_market_brokers`.
- `bandar broker CODE`: where one broker is a top-10 buyer or seller across
  the most traded stocks (IPOT). MCP `idx_bandar_broker`.
- `stocks actions SYMBOL`: corporate actions from IPOT (dividends, splits,
  bonus shares, rights with ratio and dates), RUPS, warrants and rights.
  MCP `idx_corporate_actions`.
- `market calendar`: the exchange calendar (dividend and rights dates, RUPS,
  public exposes, listings; bonds and buybacks on request), each entry
  classified. MCP `idx_calendar`.
- `market holidays`: bundled trading holidays for 2026 and 2027 (checked
  against IDX's trading-day counts), today's status and the next trading day.
  `IDX_HOLIDAYS` overrides the list. MCP `idx_holidays`.
- `market indices`, `stocks list`, `market members`: the daily index board,
  listed stocks and exchange members from idx.co.id; the last two fall back to
  the bundled lists. MCP `idx_indices`, `idx_instruments`, `idx_members`.
- 33 MCP tools.
- Web terminal: `actions`, `holidays`, `bandar broker` (from the nightly scan),
  `list` and `members` (bundled lists) and `tools`. `help` ends with a button
  for each of the 33 MCP tools; the five idx.co.id tools show their CLI
  command, since idx.co.id refuses requests from the Worker.
- Landing page: a Daftar Perintah table with every MCP tool, its CLI command
  and a link that runs it in the terminal.

### Fixed

- `bandar screen --symbols` offline with nothing cached exits 5 (offline miss)
  instead of 4.

## [0.6.1] - 2026-10-18

### Fixed

- Web terminal: `news`, `sentiment` and `brief` no longer hang when a feed
  accepts the connection and never answers, as Kontan's RSS hosts did. Every
  outbound call of the Worker (Yahoo, IPOT, the news feeds, the ownership
  mirror) now has a deadline that covers the body; a silent news feed is
  skipped for five minutes in favour of its cached copy; and the browser
  reports a request with no answer after 60 s instead of leaving it pending.
- `stocks news`, `stocks sentiment`, `stocks brief` and the matching MCP
  tools: a feed that times out is remembered for five minutes, so later calls
  serve its cached copy (with a warning saying why) at once instead of each
  waiting out `--timeout`.
- Network errors name a timeout as such ("timed out waiting for <url>") and
  carry their underlying cause instead of reqwest's bare "error sending
  request for url".
- npm launcher: a binary download that stops sending fails after 30 s
  instead of hanging the install or the first run.
- `install.sh`: downloads give up after 20 s without a connection or 60 s
  without data.

## [0.6.0] - 2026-10-18

### Fixed

- `broker`: the IPOT summary ranks its top 10 buyers and sellers by lots, not
  by value as the help, the docs and the JSON Schema said. Of two brokers
  the one with more lots comes first even when it traded less value.

### Added

- `idx bandar detect <SYMBOLS>`: bandarmology on the broker summary. For the
  top 1, 3, 5 and 10 buyers against as many sellers: bandar value and volume
  (net value and lots), net as a share of the value traded and a signal
  (`big_accumulation` at +15 %, `accumulation` at +5 %, `distribution` at
  -5 %, `big_distribution` at -15 %), the phase against the price move
  (`quiet_accumulation`, `markup`, `quiet_distribution`, `markdown`), the top
  buyers' share and average price against the last price, foreign flow and
  the reasons. `--period 1d|1w|1m|3m` (default 1w) or explicit dates, `--top`,
  `--investor`, `--board`; `--daily` adds the session-by-session movement
  with its cumulative net and streak. JSON schema `idx.bandar.v1`.
- `idx bandar screen`: the bandarmology screener over the 100 most traded
  stocks by value (`--scan`, or `--symbols`), with presets `big-accumulation`,
  `accumulation`, `distribution`, `big-distribution`, `quiet-accumulation`,
  `quiet-distribution`, `foreign-accumulation` and `below-bandar`, and
  filters on `acc`, `net`, `lots`, `fnet`, `fpct`, `gain`, `chg`, `turnover`,
  `conc`, `price`, `signal` and `phase`. JSON schema `idx.bandar.screen.v1`.
- MCP tools `idx_bandar` and `idx_bandar_screen` (23 tools now).
- Web: `/api/bandar`, `/api/bandar/screen` and `bandar` / `bandar screen` in
  the terminal. The web screen reads a scan the Worker's cron runs after each
  close (from 16:45 WIB) over the session's 100 most traded stocks, a few per
  minute, into D1 (`migrations/0003_bandar.sql`); `/api/bandar/status` shows
  its progress.
- Web terminal: `quote` with a single symbol draws a price chart under the
  table, with ranges 1D, 5D, 1M, 6M, YTD, 1Y and 5Y, a volume strip, the
  previous close on 1D and a crosshair readout of each candle (pointer, touch
  or arrow keys).

## [0.5.0] - 2026-10-17

### Added

- `idx broker <SYMBOLS>`: which brokers bought and sold a ticker. The top 10
  buyers and top 10 sellers by value from Indo Premier's (IPOT) public broker
  summary, each with the broker code, name, class (local, foreign, BUMN),
  lots, value and average price, plus the range's total value, lots, average
  price and foreign net value. `--date`, `--from/--to`, `--investor
  all|foreign|domestic`, `--board all|regular|cash|negotiated`, up to 10
  tickers per call; without dates each ticker gets its latest session with
  trades. JSON schema `idx.broker.v1`; `-o csv` writes one row per broker.
  The source rounds figures of a million and more to one decimal, so those
  summaries carry `rounded: true`.
- `data/brokers.csv`: IDX's exchange-member list (89 brokers), bundled to
  name the codes.
- MCP tool `idx_broker` (21 tools now), `/api/broker` and a `broker` command
  in the web terminal.
- Web: Open Graph / Twitter share card, canonical URLs, JSON-LD,
  `robots.txt`, `sitemap.xml` and an apple-touch icon on
  ihsg.midoriresearch.com; API responses are `noindex`.

### Changed

- Package metadata and the licence credit Midori Research
  (desk@midoriresearch.com) instead of a personal name and address.

## [0.4.0] - 2026-10-17

### Added

- Published as **`ihsg-cli`** on crates.io (`cargo install ihsg-cli`) and npm
  (`npm i -g ihsg-cli`, `npx ihsg-cli`); the command is still `idx`. The
  release workflow publishes both on every tag once `CRATES_PUBLISH` /
  `NPM_PUBLISH` are enabled (token secret or Trusted Publishing).
- Ownership data pipeline: `.github/workflows/ownership.yml` and
  `scripts/ownership_mirror.py` mirror every IDX "Pemegang Saham di Atas 1%"
  workbook to the `ownership-data` prerelease, with `index.json` (original
  URL, SHA-256, counts) and parsed JSON parts. idx.co.id blocks cloud
  networks, so the job runs on a self-hosted runner (`OWNERSHIP_RUNNER`); on
  GitHub's runners it only alarms when the mirror is 45 days stale. The
  mirror holds every register from February to September 2026.
- The web terminal loads the register by itself from that release (one
  verified part per minute, newest six months, D1 write budget); new
  `/api/ownership/status`. The manual `web/scripts/ownership-sync.mjs` is gone.
- `idx ownership import --latest|--all` and `discover` fall back to the GitHub
  mirror when idx.co.id refuses the request, checking each file's SHA-256;
  `IDX_OWNERSHIP_MIRROR` overrides or disables it, `IDX_OWNERSHIP_SOURCE`
  forces either side. `file://` URLs are accepted.
  `idx.ownership.discover.v1` entries gain `origin`, `sha256`, `source_url`.

### Changed

- Logo: the wordmark is now "IHSG" with the tagline "Indonesian market data
  for people and agents".
- Web ownership rows now carry nationality, domicile and scripless/scrip
  share counts like the CLI.

## [0.3.0] - 2026-10-17

### Added

- `idx mcp setup <client>`: the registration command or config snippet for
  25 agent harnesses (Claude Code, Claude Desktop, OpenAI Codex, Meta Muse
  Code, Google Antigravity, Gemini CLI, GitHub Copilot CLI, VS Code, Cursor,
  Windsurf, Kiro, JetBrains, Zed, Cline/Roo/Kilo, Continue, OpenCode, Goose,
  Warp, Factory Droid, Mistral Vibe, Qwen Code, Crush, Trae, Augment, and the
  remote-only products), with the binary's absolute path filled in; `--npx`
  and `--http URL --token T` variants; `-o json` for scripts.
- `idx mcp --http ADDR [--token T] [--allowed-host H]`: the same 20 tools
  over MCP streamable HTTP at `/mcp` for ChatGPT, Claude.ai connectors,
  Perplexity, Devin, Manus and other URL-only clients. Bearer auth, Host
  allow-list, `/healthz`.
- README and docs/mcp.md: install guide per agent.

## [0.2.0] - 2026-10-17

### Added

- `stocks brief SYMBOL` and MCP `idx_brief`: quote, verdict, signal, returns
  vs IHSG, dividend facts, top holders and headlines in one call, with
  plain-language `key_points` and a `sections_failed` list.
- `stocks dividends SYMBOL` / `idx_dividends`: Yahoo chart events
  (`events=div,splits`), yearly totals, TTM total and yield, payout streak.
- `stocks returns SYMBOLS` / `idx_returns`: 1w..5y and YTD returns, CAGR,
  volatility, max drawdown, best/worst day, 52-week position, and excess over
  a benchmark index (`^JKSE` by default). Index codes now parse everywhere.
- `market overview` / `idx_market`: IHSG, breadth, turnover, gainers, losers,
  most traded, heavyweights.
- `ownership foreign [SYMBOL]` / `idx_ownership_foreign`: disclosed foreign vs
  local stake per issuer over time, or every issuer's change between months.
- `ownership changes --offset` paging and a `summary` block (counts, foreign/
  local and investor-type net, issuer flow, top rises and falls) over every
  match; rows carry `local_foreign`.
- `-o csv` for every command.
- `idx watch add|rm|list|clear|returns` and `idx watch` for a local watchlist.
- Web terminal and landing page at https://ihsg.midoriresearch.com with a
  minute-refreshed D1 snapshot API (`web/`), now serving the new endpoints
  (`dividends`, `returns`, `brief`, `ownership/foreign`) too.

### Changed

- `stocks news SYMBOL` lists the company's headlines first and then fills the
  limit with market headlines (`scope` field, `company_matches`);
  `--company-only` restores the strict filter. Sentiment uses company items only.
- MCP tool input schemas are rewritten into the subset every harness accepts
  (no `$schema`, no `format`, no nullable type arrays), so the tools register
  in Gemini and OpenAI-strict clients as well as Claude.
- README logo respects GitHub dark mode and no longer clips its tagline.

### Fixed

- npm wrapper: npm 12 blocks dependency install scripts by default, so the
  package arrived without a binary. The launcher now downloads and verifies
  the binary on first run when postinstall did not.

## [0.1.0] - 2026-10-17

### Added

- `stocks quote <SYMBOLS>`: delayed quotes with price, change, OHLC, volume,
  market cap, P/E, dividend yield and more, from Yahoo Finance. Falls back to
  the chart endpoint (reduced fields) when the quote endpoint is unavailable.
- `stocks history <SYMBOL> --period --interval --limit`: OHLCV candles with a
  summary line.
- `-o json`: stable envelope (`idx.quotes.v1`, `idx.history.v1`) with
  provenance (`provider`, `fetched_at`, `cached`, `stale`, `warnings`).
- `idx schema <name>`: JSON Schema for every JSON shape.
- Local response cache with per-endpoint TTL, `--offline`, `--no-cache`, and
  `idx cache stats|clear|path`.
- `stocks screen --filter "pe<15,roe>15,sector=financial"`: screener over the
  bundled IDX universe (955 issuers, `data/universe.csv`). Quote-level keys
  come from batched Yahoo quotes; fundamentals keys from per-company calls
  capped at `--max-fundamentals` (largest first). Reports how many tickers
  each stage kept.
- `stocks news [SYMBOL]` and `stocks sentiment SYMBOL`: Kontan and CNBC
  Indonesia RSS feeds, ticker/company-name matching, transparent lexicon
  sentiment (terms are listed). Bisnis.com is opt-in because it blocks
  non-browser clients.
- MCP tools `idx_screen`, `idx_news`, `idx_sentiment`.
- `ownership import|ticker|entity|changes|releases|discover|path`: KSEI's
  monthly "Pemegang Saham di Atas 1%" register parsed from the IDX XLSX (or
  the older PDF text layer) into local SQLite, with entity-name normalisation
  so spelling variants of one holder group together. Downloads try a
  browser-shaped request, then `curl-impersonate`, else explain the manual
  path (idx.co.id is behind Cloudflare).
- MCP tools `idx_ownership_ticker`, `idx_ownership_entity`,
  `idx_ownership_changes`, `idx_ownership_releases`.
- `idx mcp`: MCP server over stdio (rmcp) exposing `idx_quote`, `idx_history`,
  `idx_profile`, `idx_financials`, `idx_earnings`, `idx_fundamental`,
  `idx_technical` and `idx_compare`, each returning the `-o json` envelope.
- `stocks profile`, `stocks financials` (annual / quarterly / TTM), `stocks
  earnings`, `stocks fundamental` (ratios plus a transparent rule-based
  verdict), `stocks technical` (RSI, MACD, SMA/EMA, Bollinger, ATR, levels,
  trend, signal) and `stocks compare`.
- MSN Money provider (`--provider msn`): quotes, profile, fundamentals and
  analyst targets. `auto` now tries Yahoo then MSN and merges fundamentals
  field by field, never averaging.
- Yahoo quoteSummary and fundamentals-timeseries endpoints.
- `web/`: Cloudflare Worker serving the landing page at
  https://ihsg.midoriresearch.com, reachable on the custom domain only.
- Config file with `idx config init|set|get|path`; precedence flags > env > file > defaults.
- Shell completions for bash, zsh, fish, PowerShell and elvish.
- Stable exit codes (0 ok, 2 usage, 3 network/provider, 4 not found,
  5 offline cache miss, 6 config).
