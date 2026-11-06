# AGENTS.md

Guidance for AI agents (and humans) working on this repository. `CLAUDE.md`
imports this file.

## What this is

`idx` is a Rust CLI, MCP server and web terminal for Indonesian stock market
(IDX / IHSG) analysis. One binary serves three audiences:

- **People** run `idx stocks quote BBCA` and read a table.
- **Scripts and agents** run `idx -o json stocks quote BBCA` and parse a stable envelope.
- **MCP clients** (Claude Code, Cursor, ...) run `idx mcp` and call the same
  capabilities as typed tools.

Data comes only from free public endpoints, with no API keys. Every response is
cached locally. `--offline` serves stale cache instead of failing.

## Package name

The crate and the npm package are published as `ihsg-cli`; the binary and the
library keep the short name `idx` (`[lib] name = "idx"` in Cargo.toml), so
code says `use idx::...` and users type `idx`.

## Layout

```
src/
  main.rs            thin binary: parse args, init tracing, map errors to exit codes
  lib.rs             module map
  cli/               clap definitions + handlers (mod.rs builds Context; one file per group);
                     input.rs reads `-` symbol lists from stdin (text or idx's own JSON/JSONL)
  providers/         Provider trait, Chain fallback, ProviderKind; one subdir per source; history.rs
                     routes candles by history_provider (auto | yahoo | stockbit) over the selected provider
    yahoo/           chart, v7 quote (cookie/crumb session), quoteSummary (summary.rs), timeseries.rs
    msn/             autosuggest lookup -> Quotes / Equities (public web key); insights.rs (api.msn.com
                     insights, peers via Quotes), screener.rs (whole-market return windows, one cached POST)
    ipot/            Indo Premier broker summary (HTML fragment, parse.rs; spurious 404s retried; a range's
                     foreign net rebuilt from single sessions, broker_summary_range; one session's
                     foreign flow in rupiah from the fd=all and fd=F pages, foreign_day) and
                     corporate actions, RUPS, warrants/rights (actions.rs; nested announcement tables)
    idx/             idx.co.id JSON endpoints: stock/broker/index daily summaries, per-stock history,
                     exchange calendar (parse.rs classifies entries), securities, members; Cloudflare
                     403 becomes a provider error
    stockbit/        OPT-IN realtime (exodus.stockbit.com), the one provider that carries a credential
                     (the user's own session) and the one not in the Auto chain: orderbook endpoint ->
                     quote + full depth ladder (model/orderbook.rs), charts/daily -> candles; auth.rs is
                     the rotating-token session store (file 0600 + env seed, POST /login/refresh), login.rs
                     captures the token from a browser over the Chrome DevTools Protocol (ws, no TLS)
  analysis/          technical.rs (indicators + signal), fundamental.rs (verdict), screen.rs (filter DSL,
                     presets), sentiment.rs (lexicon), returns.rs (windows, CAGR, drawdown, volatility, beta
                     against a benchmark on shared dates), lens.rs (growth, valuation and risk lenses),
                     dividends.rs (totals, TTM yield, streak), bandar.rs (bandarmology: tiers, signal,
                     phase, screen filters and presets); no network
  universe.rs        bundled data/universe.csv (IDX issuers) + ownership-db issuers; IDX_UNIVERSE override
  brokers.rs         bundled data/brokers.csv (IDX exchange members: code -> name)
  holidays.rs        bundled data/holidays.csv (trading holidays, transcribed from IDX's yearly image);
                     is_trading_day / next / last / latest_session (16:15 WIB close, behind_session);
                     IDX_HOLIDAYS override
    news/            Kontan + CNBC Indonesia RSS feeds through the Fetcher (rss crate)
  service.rs         provider call + analysis, shared by cli/ and mcp/ (fundamentals, technical, compare,
                     screen, news ordering, returns, dividends, market_overview, brief, broker, bandar,
                     bandar_screen, bandar_broker); service/exchange.rs: holidays, instruments,
                     members, indices, calendar, corporate actions, foreign flow, market brokers;
                     service/lenses.rs: growth, valuation and risk lenses (fundamentals, statements,
                     a year of candles for the stock and IHSG)
  ownership/         KSEI >1% register: discover.rs (idx.co.id page and its families, Cloudflare strategy,
                     `discover --family`), mirror.rs (GitHub mirror fallback), ingest.rs (XLSX/PDF, text
                     numbers, controller column), normalize.rs (entity keys), db.rs (SQLite schema with
                     `PRAGMA user_version` migrations, register lines and summed positions, queries;
                     db/analytics.rs the analytics queries, db/resolve.rs the `resolutions` rules and the
                     import's entity resolver), analytics.rs (pure: search, cross-holders, HHI concentration,
                     graph walk and DOT, spelling candidates); snapshot.rs (prebuilt SQLite snapshot: build,
                     manifest checks, the sync decision, install through the backup API, status); KSEI's
                     composition archive: ksei.rs (ksei.co.id "Kepemilikan Efek" listing + ZIP download),
                     composition.rs (ZIP/TXT parser with zip-slip and size limits, its own tables, report);
                     mod.rs query wrappers shared by cli/, mcp/ and the brief. Imported once (or synced),
                     queried locally.
  mcp/               rmcp server; one #[tool] per command, same JSON envelope as -o json;
                     schema.rs rewrites input schemas into the subset every harness accepts;
                     http.rs streamable-HTTP transport (axum, bearer token); clients.rs the
                     per-agent registration recipes behind `idx mcp setup` (keep docs/mcp.md in step)
  model/             provider-agnostic domain types = the JSON contract (Quote, Candle, History, ...,
                     dividends.rs, returns.rs, market.rs, brief.rs, watchlist.rs, institutions.rs, lenses.rs)
  output/            table*.rs (comfy-table), json.rs (Envelope + JSON Schema), csv.rs (-o csv),
                     jsonl.rs (-o jsonl: which list each schema streams, ROWS), format.rs (numbers);
                     mod.rs OutputSetting (auto resolved against the terminal) and print_notice
                     (stderr lines --quiet hides); cli/mod.rs emit() picks the format for every handler
  http/              Fetcher: cache in front of a Backend (reqwest live / FixtureBackend tests);
                     cloudflare.rs tells a Cloudflare challenge or block page from an HTTP error (`blocked`)
  progress.rs        the CLI's loading line on stderr: hosts in flight and their wait; fetcher and
                     IPOT retries report to it, only the CLI turns it on (stderr a terminal, no -v)
  cache/             content-addressed file cache, TTL decided by the caller at read time
  config/            flags > IDX_* env > config.toml > defaults
  error.rs           IdxError with stable exit codes
tests/
  cli.rs             assert_cmd tests of the binary, no network
  mcp.rs             spawns `idx mcp`, speaks JSON-RPC over stdio, offline
  fixtures/<provider>/  recorded raw responses used by unit tests
scripts/smoke.sh     hits live providers; run manually, never in CI's test job
docs/                json-output.md, configuration.md, mcp.md, ownership.md
web/                 Cloudflare Worker for ihsg.midoriresearch.com: landing page, browser terminal, D1 board;
                     src/analysis.js, src/extras.js and src/bandar.js are JS ports of src/analysis/* tested on
                     the same fixtures, src/ownership.js of src/ownership/analytics.rs; src/bandar-scan.js is
                     the nightly scan behind the web bandar screen
npm/                 ihsg-cli wrapper on npm: lib/binary.js downloads + verifies the release
                     (postinstall when allowed, otherwise on first run)
scripts/ownership_mirror.py  CI scraper behind .github/workflows/ownership.yml: mirrors the KSEI
                     register to the `ownership-data` prerelease (raw files, index.json, JSON parts) and
                     publishes the prebuilt snapshot (`snapshot-v{V}.json` + `ownership-v{V}-*.sqlite.gz`)
```

CLI groups: `stocks` (quote, history, profile, financials, earnings, dividends, returns,
fundamental, growth, valuation, risk, technical, brief, compare, screen, news, sentiment, actions,
foreign, list),
`market` (overview, indices, foreign, brokers, calendar, holidays, members),
`ownership` (import, ticker, entity, search, cross-holders, concentration, graph, resolve, changes, foreign,
composition, institutions, releases, sync, status, discover, path; hidden build-snapshot),
`broker`, `bandar` (detect, screen, broker), `stockbit` (login, status, logout, orderbook — opt-in
realtime), `watch`, `mcp`, `schema`, `config`, `cache`, `completions`, `version`.

## Rules that keep the tool trustworthy

1. **Never invent fields.** If a source does not return a value, the field is
   `null`. No estimates, no fallbacks to another metric under the same name.
2. **No network in `cargo test`.** Record a raw response into
   `tests/fixtures/<provider>/` and parse that. Use `http::FixtureBackend` to
   test request flow (sessions, retries, fallbacks).
3. **Tables to stdout, diagnostics to stderr, non-zero exit on failure.**
   Exit codes are documented in `src/error.rs` and `--help`; do not reuse them
   for new meanings. Informational stderr lines (warnings, notes,
   confirmations) go through `output::print_notice` so `--quiet` hides them;
   errors go through `report_error`, and instructions the user must act on
   through `output::print_stderr`.
4. **JSON is a contract.** Payload shapes are versioned by the `schema` field
   (`idx.quotes.v1`). Adding an optional field is fine; renaming or removing
   one means a new version string. Every shape must be reachable through
   `idx schema <name>` (`output/json.rs`).
5. **Providers are interchangeable.** Nothing outside `src/providers/<name>/`
   may know about a source's URL or field names. The one exception is
   `src/ownership/`, which *is* the source module for the KSEI registers
   (idx.co.id and ksei.co.id).
6. **Cache keys exclude secrets.** Session tokens (crumbs, cookies) never form
   part of a cache key; pass an explicit `cache_key` to `Fetcher::get`.

## Adding a provider

1. Probe the live endpoint by hand first (curl). Save representative raw
   responses, including an error and a not-found case, into
   `tests/fixtures/<name>/`.
2. Create `src/providers/<name>/mod.rs` with a struct holding
   `Arc<Fetcher>` and `Ttls`. Implement `Provider` (`providers/mod.rs`). Put pure
   parsers in `parse.rs` and test them against the fixtures.
3. Fetch through `self.fetcher.get(url, headers, ttl, cache_key)`, or
   `self.fetcher.post(url, headers, body, ttl, cache_key)` for a read-only
   endpoint that takes its parameters in the body (the key is required). Map HTTP 404
   or a "not found" body to `IdxError::NotFound`; other failures to
   `IdxError::provider(NAME, ..)`. Leave the default `Unsupported` for
   capabilities the source lacks; `Chain` skips those.
4. Add a `ProviderKind` variant, wire it in `providers::build`, and decide its
   place in the `Auto` chain order.
5. Document the endpoints at the top of the module and in `README.md`'s data
   sources table. Add a line to `scripts/smoke.sh`.

## Adding a command

1. Add the variant and args to the relevant `src/cli/*.rs` enum. Give it an
   `after_help` with examples; agents read `--help`.
2. Add any new domain type to `src/model/` with `Serialize + JsonSchema`, a new
   `SCHEMA_*` constant and `schema_for` entry in `output/json.rs`, its row rule
   in `output/jsonl.rs` `ROWS` (a unit test fails without one), and a table
   renderer in `output/table.rs`.
3. Handlers call the provider, then hand the result to `emit()`, which picks
   the format. Warnings go to stderr in table, csv and jsonl mode and into
   `envelope.warnings` in JSON mode. A value without an envelope prints
   through `output::print_value`. A symbol list goes through
   `cli::input::symbol_args` so `-` reads stdin; Yahoo-backed commands parse
   with `Symbol::parse_on(raw, &ctx.settings.exchange)`, every other one with
   `Symbol::parse` (IDX only).
4. Cover the parsing with fixtures and the CLI surface in `tests/cli.rs`.
5. Add a matching `#[tool]` in `src/mcp/mod.rs` with a one-paragraph
   description and a typed argument struct, returning the same envelope.
   Put shared logic in `src/service.rs`, never in the handler. Keep argument
   types to strings, numbers, booleans and string arrays; `mcp/schema.rs`
   strips nullability and formats, and the tests fail on anything else.
6. Mirror it in the web terminal when it fits (`web/src/api.js` endpoint,
   `web/public/terminal.js` command) so the three faces stay in step.

## Development loop

```sh
cargo build
cargo test                       # fixtures only
cargo clippy --all-targets -- -D warnings
cargo fmt
./scripts/smoke.sh               # live providers, needs network
./target/debug/idx -vv stocks quote BBCA   # -vv prints every request to stderr
```

Commits follow Conventional Commits (`feat:`, `fix:`, `docs:`, `test:`,
`chore:`, `ci:`). Keep each commit buildable.

## Milestones

1. ✅ Scaffold, `stocks quote`, `stocks history` via Yahoo, table + JSON, cache
2. ✅ MSN provider, `profile`, `financials`, `earnings`, `fundamental`, `technical`, `compare`
3. ✅ MCP server over stdio wrapping 1–2 (verified from Claude Code)
4. ✅ KSEI ownership ingest (XLSX + PDF text layer) into SQLite + queries
5. ✅ Screener, news, sentiment
6. ✅ Release pipeline (static binaries, install.sh, npm wrapper), web terminal + landing page
