<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/logo-dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="assets/logo-light.svg">
    <img src="assets/logo.svg" alt="IHSG: Indonesian market data for people and agents" width="480">
  </picture>
</p>

<p align="center">
  Indonesian stock market (IDX / IHSG) analysis for people and AI agents.<br>
  One binary: a CLI that prints tables, a JSON mode for scripts, and an MCP server for agents.
</p>

<p align="center">
  <a href="https://github.com/fzn0x/idx/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/fzn0x/idx/actions/workflows/ci.yml/badge.svg"></a>
  <a href="LICENSE"><img alt="MIT" src="https://img.shields.io/badge/license-MIT-16a34a.svg"></a>
  <a href="https://ihsg.midoriresearch.com"><img alt="Web terminal" src="https://img.shields.io/badge/web-ihsg.midoriresearch.com-0b0f14.svg"></a>
</p>

---

<p align="center">
  <a href="https://ihsg.midoriresearch.com"><img src="docs/screenshots/landing.png" alt="ihsg.midoriresearch.com: the live board, published every minute" width="100%"></a>
</p>

```sh
idx stocks quote BBCA,BBRI,TLKM
```

```
╭────────┬──────────────────────────────┬───────┬──────┬─────────┬───────┬───────┬───────┬───────────────┬─────────┬───────┬────────╮
│ Symbol ┆ Name                         ┆ Price ┆ Chg  ┆ Chg %   ┆ Open  ┆ High  ┆ Low   ┆ Volume        ┆ Mkt Cap ┆ P/E   ┆ Div %  │
╞════════╪══════════════════════════════╪═══════╪══════╪═════════╪═══════╪═══════╪═══════╪═══════════════╪═════════╪═══════╪════════╡
│ BBCA   ┆ PT Bank Central Asia Tbk     ┆ 5,175 ┆ -900 ┆ -14.81% ┆ 5,825 ┆ 5,825 ┆ 5,175 ┆ 1,241,098,200 ┆  635.7T ┆ 10.96 ┆  6.27% │
│ BBRI   ┆ PT Bank Rakyat Indonesia (P… ┆ 2,640 ┆ -460 ┆ -14.84% ┆ 2,640 ┆ 2,890 ┆ 2,640 ┆ 1,193,916,400 ┆  397.8T ┆  6.49 ┆ 13.48% │
│ TLKM   ┆ Perusahaan Perseroan (Perse… ┆ 1,900 ┆ -330 ┆ -14.80% ┆ 1,900 ┆ 1,985 ┆ 1,900 ┆   963,614,200 ┆  187.3T ┆ 10.98 ┆ 10.01% │
╰────────┴──────────────────────────────┴───────┴──────┴─────────┴───────┴───────┴───────┴───────────────┴─────────┴───────┴────────╯
Last trade 2026-10-16 16:14 WIB · market closed · delayed data
```

```sh
idx -o json stocks quote BBCA | jq '.data[0] | {price, change_pct, pe_trailing, dividend_yield_pct}'
```

```json
{ "price": 5175.0, "change_pct": -14.814815, "pe_trailing": 10.963983, "dividend_yield_pct": 6.27 }
```

<p align="center">
  <img src="docs/screenshots/cli.png" alt="idx stocks quote and idx stocks compare in a terminal" width="100%">
</p>

- **Free data, no keys.** Yahoo Finance and MSN Money for prices and
  fundamentals, the exchange's own idx.co.id summaries for foreign flow,
  member activity, indices and the corporate-action calendar, IDX's monthly
  KSEI register for ownership, Indo Premier's broker summary for who bought
  and sold (read as bandarmology by `idx bandar`) and its corporate-action
  pages, Kontan and CNBC Indonesia feeds for news. Every source sits behind a `Provider` trait, so
  one breaking does not take the tool down, and `auto` fills gaps in one
  source's fundamentals from the other.
- **Built for agents.** `-o json` emits a stable, schema-documented envelope
  (`idx schema quotes`), `-o jsonl` one record per line. Exit codes mean
  something, and `-` reads tickers from stdin so commands chain. `idx mcp`
  exposes every command as a typed MCP tool that returns the very same JSON.
- **Works offline.** Responses are cached with per-endpoint TTLs; `--offline`
  serves the cache instead of failing when the network is gone.
- **Ownership, not just prices.** KSEI's monthly register of holders above 1 %
  is imported into local SQLite with normalised entity names, so "who owns
  more than 1 % of BBCA" and "what did this fund change last month" are one
  command each. The monthly IDX files are mirrored to GitHub, so neither you
  nor the web terminal imports anything by hand.
- **Single static binary.** Rust, no runtime, Linux / macOS / Windows.

## Install

The package is called **`ihsg-cli`**; the command it installs is **`idx`**
(npm also adds `ihsg-cli` as an alias).

```sh
cargo install ihsg-cli                    # crates.io, builds from source (Rust 1.88+)
npm install -g github:fzn0x/idx           # npm, straight from this repository
npx github:fzn0x/idx stocks quote BBCA    # npm, no install
curl -fsSL https://raw.githubusercontent.com/fzn0x/idx/main/install.sh | sh   # Linux, macOS
```

npm installs a small launcher from this repository; the first `idx` command
downloads the prebuilt binary for your platform from the matching GitHub
release and checks it against the release's `SHA256SUMS` (so does
`install.sh`). The binary is stored next to the launcher, or in a per-user
cache when the global npm folder is not writable. On npm 12, which only
installs from git when allowed, add `--allow-git=all` to the command.

Windows without npm: download `idx-<version>-x86_64-pc-windows-msvc.zip` from
the [releases page](https://github.com/fzn0x/idx/releases). Static Linux
binaries are musl builds and run on any distribution. The development version installs
with `cargo install --git https://github.com/fzn0x/idx`.

Shell completions:

```sh
idx completions bash > ~/.local/share/bash-completion/completions/idx
idx completions zsh  > ~/.zfunc/_idx
idx completions fish > ~/.config/fish/completions/idx.fish
idx completions powershell | Out-String | Invoke-Expression
```

## Commands

| command | what it does |
| --- | --- |
| `stocks quote <SYMBOLS>` | Delayed quote for one or many tickers (`BBCA,BBRI` or `BBCA BBRI`; `.JK` optional; `-` reads them from stdin; another Yahoo Finance market by its suffix, e.g. `D05.SI`) |
| `stocks history <SYMBOL> [--period 1y] [--interval 1d] [--limit 30]` | OHLCV candles with a summary line. Periods `1d 5d 1mo 3mo 6mo 1y 2y 5y 10y ytd max`, intervals `5m 15m 30m 1h 1d 1wk 1mo` |
| `stocks profile <SYMBOL>` | Sector, industry, description, size, headquarters, management |
| `stocks financials <SYMBOL> [--period annual\|quarterly\|ttm] [--statement all\|income\|balance\|cash]` | Income statement, balance sheet, cash flow |
| `stocks earnings <SYMBOL>` | Quarterly EPS vs estimates with surprise, yearly revenue/earnings, next earnings date and consensus |
| `stocks dividends <SYMBOL>` | Every cash dividend (ex-date, split-adjusted amount) and split; yearly totals, TTM total and yield, payout streak |
| `stocks returns <SYMBOLS> [--benchmark ^JKSE]` | 1w/1m/3m/6m/YTD/1y/3y/5y returns, CAGR, volatility, max drawdown, best/worst day, 52-week position, and excess over IHSG |
| `stocks brief <SYMBOL>` | One page: quote, verdict, signal, returns vs IHSG, dividends, top holders, headlines, with plain-language key points |
| `stocks fundamental <SYMBOL>` | P/E, P/B, PEG, EV/EBITDA, ROE, ROA, margins, leverage, growth, dividend, analyst targets, plus a plain-language verdict that lists every rule it applied |
| `stocks technical <SYMBOL> [--period 1y]` | RSI, MACD, SMA/EMA 20/50/200, Bollinger, ATR, volume, support/resistance, trend, buy/sell/neutral signal with reasons |
| `stocks compare <SYMBOLS>` | Side-by-side fundamentals for two or more tickers |
| `stocks screen -f "pe<15,roe>15,sector=financial"` | Screener over ~950 IDX tickers: one batch quote for valuation/size/dividend keys, per-company fundamentals for the rest (capped, largest first) |
| `stocks news [SYMBOL] [--company-only] [--source kontan\|cnbcindonesia\|all]` | Headlines from Kontan and CNBC Indonesia RSS. With a ticker, the company's headlines come first, then market context fills the limit |
| `stocks sentiment <SYMBOL>` | Aggregate lexicon sentiment over the matching headlines, with counts, terms and the strongest headlines |
| `stocks actions <SYMBOL> [--since D]` | Corporate actions from IPOT, upcoming ones included: cash and stock dividends, splits, bonus shares, rights issues (amount, ratio old:new, cum, ex, recording and payment dates, status, announcement), shareholders' meetings (RUPS: time, venue, agenda), listed warrants and rights |
| `stocks foreign <SYMBOL> [--days 20]` | Foreign buying and selling per session from idx.co.id (shares, as IDX publishes them), net, net × close, totals and the net-buy/net-sell streak |
| `stocks list [--board B] [--search S]` | Listed stocks from idx.co.id with listing board, listing date and shares; the bundled universe stands in offline |
| `ownership import --latest\|--all\|--file F\|--url U` | Import KSEI's monthly ">1 % holders" register (XLSX or PDF) into local SQLite. `--latest`/`--all` read idx.co.id and fall back to the CI-maintained GitHub mirror |
| `ownership ticker <SYMBOL> [--as-of D]` | Holders above 1 % of a ticker |
| `ownership entity "<NAME>" [--as-of D]` | Every position of a person or institution, spelling variants grouped |
| `broker <SYMBOLS> [--date D \| --from D --to D] [--investor all\|foreign\|domestic] [--board all\|regular\|cash\|negotiated]` | Broker summary: the top 10 buyers and sellers of a ticker (ranked by lots), with broker name, class (local, foreign, BUMN), lots, value and average price, plus total value and foreign net. Latest session with trades by default; `-o csv` gives one row per broker |
| `bandar detect <SYMBOLS> [--period 1d\|1w\|1m\|3m] [--top 1\|3\|5\|10] [--daily]` | Bandarmology: are the top brokers accumulating or distributing? The top 1/3/5/10 buyers against as many sellers (bandar value and volume), net as a share of the value traded, the phase against the price move, the top buyers' average against the price, foreign flow; `--daily` adds the bandar movement session by session. See [Bandarmology](#bandarmology) |
| `bandar screen [--preset P] [-f FILTER] [--period P] [--scan N]` | Bandarmology screener over the 100 most traded stocks by value: presets `big-accumulation`, `accumulation`, `distribution`, `big-distribution`, `quiet-accumulation`, `quiet-distribution`, `foreign-accumulation`, `below-bandar`, or filters on `acc`, `net`, `fnet`, `fpct`, `gain`, `chg`, `turnover`, `conc`, `signal`, `phase` |
| `bandar broker <CODE> [--period P] [--scan N \| --symbols S]` | Broker activity: where one broker (e.g. `YP`) sits among the top 10 buyers or sellers of the 100 most traded stocks, with rank, lots, value, average, share of the stock's value and net where both sides are visible |
| `market overview [--top 10]` | IHSG, breadth, turnover, gainers, losers, most traded, heavyweights from one pass over the universe |
| `market foreign [--date D] [--top 10] [--sort value\|shares]` | Top foreign net buying and net selling across every stock for one session (idx.co.id), with the market's foreign totals |
| `market brokers [--date D] [--top 20] [--sort value\|volume\|frequency]` | Every exchange member's trading for one session, ranked, with each member's share of all members' value |
| `market indices [CODE...] [--date D]` | Every IDX index (IHSG, LQ45, IDX30, sector and thematic) for one session: close, change, range, constituents, value, market cap |
| `market calendar [--from D --to D \| --date D] [--symbol S] [--type T]` | The exchange calendar: dividend and rights dates (cum, ex, recording, payment), RUPS, public exposes, listings; bonds and buyback reports with `--type all`. Default: the next 14 days |
| `market holidays [--year Y \| --all] [--upcoming]` | Trading holidays (Kalender Libur Bursa, bundled), the year's trading days, whether today is a trading day and the next one |
| `market members [--search S]` | Exchange members (broker codes) with names and licences |
| `ownership changes --from D --to D [--symbol S] [--min-change PP] [--limit N --offset N]` | New holders, exits and position changes between two months, paged, with a summary (counts, foreign vs local net, by investor type, issuer flow, top moves) over every match |
| `ownership foreign [SYMBOL] [--from D --to D]` | Disclosed foreign (KSEI `A`) vs local stake: one issuer month by month, or every issuer's change between two months |
| `watch [add\|rm\|list\|clear\|returns]` | Plain-text watchlist next to the config; `idx watch` quotes it |
| `ownership releases\|discover\|path` | Imported months, files published on idx.co.id, database path |
| `mcp [--http ADDR --token T]` · `mcp setup <client>` | MCP server over stdio or streamable HTTP, and the registration recipe for 25 agent harnesses. 36 tools: `idx_brief`, `idx_market`, `idx_quote`, `idx_history`, `idx_profile`, `idx_financials`, `idx_earnings`, `idx_dividends`, `idx_returns`, `idx_fundamental`, `idx_technical`, `idx_compare`, `idx_screen`, `idx_news`, `idx_sentiment`, `idx_corporate_actions`, `idx_foreign`, `idx_instruments`, `idx_market_foreign`, `idx_market_brokers`, `idx_indices`, `idx_calendar`, `idx_holidays`, `idx_members`, `idx_broker`, `idx_bandar`, `idx_bandar_screen`, `idx_bandar_broker`, `idx_ownership_ticker`, `idx_ownership_entity`, `idx_ownership_changes`, `idx_ownership_foreign`, `idx_ownership_releases`, `idx_orderbook`, `idx_stockbit_status`, `idx_stockbit_login` |
| `schema [name]` | JSON Schema for every payload (`quotes`, `history`, `profile`, `fundamentals`, `compare`, `financials`, `earnings`, `dividends`, `returns`, `market`, `brief`, `technical`, `screen`, `news`, `sentiment`, `actions`, `foreign`, `instruments`, `market-foreign`, `market-brokers`, `indices`, `calendar`, `holidays`, `members`, `broker`, `bandar`, `bandar-screen`, `bandar-broker`, `ownership-*`, `watchlist`, `error`, ...) |
| `config init\|set\|get\|path` | Config file management |
| `cache stats\|clear\|path` | Response cache management |
| `completions <shell>` | bash, zsh, fish, powershell, elvish |
| `version` | Version, build and default paths |

Global flags work anywhere on the line:

| flag | env | meaning |
| --- | --- | --- |
| `-o, --output table\|json\|jsonl\|csv\|auto` | `IDX_OUTPUT` | Output format: tables for people, the JSON envelope for programs, one JSON record per line (`jsonl`), CSV rows of the payload's main list for spreadsheets; `auto` is a table on a terminal and the JSON envelope when piped (opt-in: the default stays `table`) |
| `--provider auto\|yahoo\|msn\|stockbit` | `IDX_PROVIDER` | Data source (`auto` tries Yahoo, then MSN, and merges fundamentals; `stockbit` is opt-in realtime from your own session, see [docs/stockbit.md](docs/stockbit.md)) |
| `--offline` | `IDX_OFFLINE=1` | Never touch the network |
| `--no-cache` | `IDX_NO_CACHE=1` | Ignore cached responses |
| `--timeout SECS` | `IDX_TIMEOUT_SECS` | Per-request timeout (default 15) |
| `--no-color` | `NO_COLOR` | Plain tables |
| `-v, -vv, -vvv` | `IDX_LOG` | Diagnostics on stderr, including every URL requested |
| `-q, --quiet` | `IDX_QUIET=1` | stderr carries errors only: no warnings, notes or loading line. stdout is unchanged and `-o json` keeps `warnings` in the envelope |
| | `IDX_EXCHANGE=SI` | Yahoo Finance suffix a bare ticker gets in `stocks quote`, `history`, `profile`, `financials`, `earnings`, `dividends`, `returns`, `fundamental`, `technical`, `compare` and `watch` (default `JK`, IDX). An explicit suffix (`D05.SI`) always works there; every other command reads IDX tickers only |
| | `IDX_PROGRESS=never\|always` | The loading line on stderr (`⠹ 6s · waiting on investasi.kontan.co.id 6s`) shows on a terminal only; `never` hides it, `always` forces it |

## For scripts and agents

```sh
idx -o json stocks history TLKM --period 1y --interval 1wk \
  | jq '.data.summary.change_pct'
idx -o jsonl market holidays --year 2026 | jq -r .date      # one record per line
idx -o jsonl stocks screen -f 'pe<10,roe>15' -n 5 | idx stocks compare -
echo BBCA BBRI | idx stocks quote -                         # tickers from stdin
idx -q -o csv stocks quote BBCA BBRI > quotes.csv           # stderr: errors only
```

- stdout carries exactly one JSON document (`-o json`) or one JSON record per
  line (`-o jsonl`); everything else goes to stderr. Errors are one JSON line
  on stderr in both modes.
- `-o auto` (or `IDX_OUTPUT=auto`) prints a table on a terminal and the JSON
  envelope when stdout is piped or captured.
- `-` in a ticker list (`stocks quote|returns|compare`, `broker`, `bandar`,
  `watch add|rm`) reads tickers from stdin: plain text (spaces, commas or lines,
  `#` comments) or idx's own `-o json` / `-o jsonl` output.
- The envelope records `provider`, `fetched_at`, `cached`, `stale` and `warnings`.
- Fields the source did not return are `null`. Nothing is estimated.
- Exit codes: `0` ok · `2` usage · `3` network/provider · `4` not found · `5` offline cache miss · `6` config.

Full field reference: [docs/json-output.md](docs/json-output.md).
Configuration and cache: [docs/configuration.md](docs/configuration.md).

### MCP server

`idx mcp` serves the Model Context Protocol over stdio, which is what every
local agent launches. `idx mcp --http` serves the same tools over streamable
HTTP for products that only connect to a URL. `idx mcp setup <client>` prints
the exact command or config snippet for your agent with the absolute path of
the binary filled in, because GUI apps rarely inherit your shell `PATH`:

```sh
idx mcp setup --list          # every supported client
idx mcp setup codex           # prints: codex mcp add idx -- /abs/path/idx mcp
idx mcp setup muse            # prints the settings.json block Meta's Muse Code expects
```

| agent | register idx |
| --- | --- |
| Claude Code | `claude mcp add --scope user idx -- idx mcp` |
| Claude Desktop | `claude_desktop_config.json` → `"mcpServers": {"idx": {"command": "idx", "args": ["mcp"]}}` |
| OpenAI Codex (CLI, IDE extension, app) | `codex mcp add idx -- idx mcp`, or `[mcp_servers.idx]` with `command`/`args` in `~/.codex/config.toml` |
| Meta Muse Code | `~/.config/muse/settings.json`: `{"schema_version": 1, "mcp_servers": {"idx": {"transport": "stdio", "command": "idx", "args": ["mcp"], "mode": "optional"}}}` |
| Google Antigravity (IDE, CLI) | `~/.gemini/config/mcp_config.json` → `mcpServers.idx` with an absolute `command` and no `type` field |
| Gemini CLI | `gemini mcp add -s user idx idx mcp` |
| GitHub Copilot CLI | `copilot mcp add idx -- idx mcp` |
| VS Code (Copilot agent mode) | `code --add-mcp '{"name":"idx","command":"idx","args":["mcp"]}'`, or `.vscode/mcp.json` → `servers.idx` with `"type": "stdio"` |
| Cursor · Windsurf · JetBrains · Warp · Trae · Augment | paste `{"mcpServers": {"idx": {"command": "idx", "args": ["mcp"]}}}` into the MCP settings (file paths in `idx mcp setup`) |
| Kiro (IDE and CLI, formerly Amazon Q Developer CLI) | `~/.kiro/settings/mcp.json`, or `kiro-cli mcp add --name idx --command idx --args mcp` |
| Zed | `settings.json` → `"context_servers": {"idx": {"command": "idx", "args": ["mcp"], "env": {}}}` |
| Cline · Roo Code · Kilo Code | MCP Servers → Configure → `mcpServers.idx` with `"type": "stdio"` |
| Continue | `~/.continue/mcpServers/idx.yaml` (`idx mcp setup continue` prints it) |
| OpenCode | `opencode.json` → `"mcp": {"idx": {"type": "local", "command": ["idx", "mcp"], "enabled": true}}` |
| Goose | `goose configure` → Add Extension → Command-line Extension → `idx mcp` |
| Factory Droid | `droid mcp add idx "idx mcp"` |
| Mistral Vibe | `~/.vibe/config.toml` → `[[mcp_servers]]` with `name = "idx"`, `transport = "stdio"`, `command`, `args` |
| Qwen Code | `qwen mcp add idx idx mcp` |
| Crush | `crush.json` → `"mcp": {"idx": {"type": "stdio", "command": "idx", "args": ["mcp"]}}` |
| ChatGPT · Claude.ai · Perplexity · Devin · Manus | remote only: `idx mcp --http 127.0.0.1:8765 --token T --allowed-host '*'` behind an HTTPS tunnel, then add `https://<host>/mcp` as a connector with header `Authorization: Bearer T` |

A bare `idx` in the table assumes the binary is on `PATH`; `idx mcp setup`
substitutes the absolute path, and `--npx` swaps in
`npx -y ihsg-cli mcp` (npm registry; from this repository use
`npx -y github:fzn0x/idx mcp`). Each entry's remote (HTTP) form and the
config file locations are in [docs/mcp.md](docs/mcp.md).

The server lists 36 tools with typed input schemas kept to the subset every
harness accepts (no nullable type arrays, no `format` or `default` keywords)
and one-paragraph descriptions. `idx_brief` is the one-call starting point;
each tool returns the same envelope as `-o json`, so an agent that learned the
CLI already knows the tools. Failures come back as tool errors carrying
`{error: {code, message}, exit_code}`.

## Bandarmology

"Bandar" is market slang for the big, concentrated money that moves a stock.
`idx bandar` reads it from the IPOT broker summary, whose top 10 buyers and
top 10 sellers are ranked by lots. For N = 1, 3, 5 and 10 the top N buyers'
value minus the top N sellers' value is the bandar value (in lots, the bandar
volume); its share of all value traded is the reading, with the headline on
the top 5 (`--top`):

| Top-N net, % of value | Signal | Phase when the price over the range is... |
|---|---|---|
| +15 or more | `big_accumulation` | up more than 2 %: `markup`; otherwise `quiet_accumulation` |
| +5 to +15 | `accumulation` | as above |
| -5 to +5 | `neutral` | `neutral` |
| -15 to -5 | `distribution` | down more than 2 %: `markdown`; otherwise `quiet_distribution` |
| -15 or less | `big_distribution` | as above |

Quiet accumulation is the usual "bandar buying quietly": concentrated buying
while the price stalls or falls. Each reading also gives the top buyers'
average price against the last price (`bandar_gain_pct`: below zero they sit
under water), how much of the value the top buyers took, the foreign net
flow, and the rules in plain words (`reasons`). `--daily` adds the session
series with its cumulative net and streak. The thresholds were set against
the 40 most traded stocks in October 2026, where about half of single
sessions read neutral. They are heuristics, not advice.

```sh
idx bandar detect BBCA                         # last week, top 5 vs top 5
idx bandar detect BBRI --period 1m --daily     # with the bandar movement
idx bandar screen --preset quiet-accumulation  # buying while the price stalls
idx bandar screen -f 'acc>=5,fpct>=5,gain<0'   # with foreigners, under the bandar's average
```

## Data sources

| source | used for | auth | notes |
| --- | --- | --- | --- |
| Yahoo Finance `v8/finance/chart` | history, quote fallback | none | IDX tickers carry the `.JK` suffix; another market's suffix (`D05.SI`) passes through for the Yahoo-backed commands |
| Yahoo Finance `v7/finance/quote` | quotes | anonymous cookie + crumb, cached 6 h | Falls back to the chart endpoint (fewer fields) if rejected |
| Yahoo Finance `v10/finance/quoteSummary` | profile, fundamentals, earnings | same session | Statement modules are empty at the source, so they are not used |
| Yahoo Finance `fundamentals-timeseries` | financials | none | Annual, quarterly and TTM statement items |
| idx.co.id "Data Kepemilikan Saham" page + `Media/*.xlsx` | ownership (KSEI monthly register) | none, but Cloudflare bot protection | Mirrored three times a day by GitHub Actions to the [`ownership-data`](https://github.com/fzn0x/idx/releases/tag/ownership-data) release (byte-identical files, SHA-256 in `index.json`). The CLI tries idx.co.id first, then the mirror. See [docs/ownership.md](docs/ownership.md) |
| Indo Premier (IPOT) `module/saham/include/data-brokersummary.php` | broker summary | none | Top 10 buyers and sellers per range, ranked by lots; figures of a million and more are rounded to one decimal by the source (`rounded: true`). Its nginx answers spurious 404s in bursts, so a 404 is retried with backoff. Broker names come from IDX's member list, bundled as `data/brokers.csv`; `bandar` computes its readings from these summaries |
| idx.co.id JSON endpoints behind its pages (`primary/TradingSummary/GetStockSummary`, `GetBrokerSummary`, `GetIndexSummary`, `ListedCompany/GetTradingInfoSS`, `Home/GetCalendar`, `StockData/GetSecuritiesStock`, `ExchangeMember/GetBrokerSearch`) | foreign flow, member activity, indices, the exchange calendar, listed stocks, members | none, but Cloudflare bot protection | Foreign trading is published in shares only; `foreign_net_value_at_close` (net shares × close) is a rupiah scale, not the traded value. Ordinary connections pass; data centres and CI runners are often refused, and the error says so. Closed sessions are cached for 30 days |
| Indo Premier (IPOT) `module/saham/include/corporateaction.php` | corporate actions, RUPS, warrants and rights | none | Same module and spurious-404 retry as the broker summary; repeated rows are dropped |
| idx.co.id trading-holiday calendar (published as an image per year) | trading holidays | none | Transcribed into `data/holidays.csv` (2026, 2027) and checked against IDX's monthly trading-day counts; `IDX_HOLIDAYS` overrides it |
| Kontan RSS (`investasi`, `keuangan`, `industri`, `insight`) and CNBC Indonesia RSS (`market`, `research`, `news`) | news, sentiment | none | Bisnis.com refuses non-browser clients; listed as an opt-in source |
| MSN Money `Finance/Quotes`, `Finance/Equities` | quotes, profile, fundamentals, analyst targets | MSN's own public web key (shipped; override with `IDX_MSN_API_KEY`) | Tickers resolve to MSN ids through its autosuggest service, filtered to exchange `XIDX` |
| Stockbit `exodus.stockbit.com` (`--provider stockbit`, opt-in) | realtime quote, history, live order book + foreign flow | **your own Stockbit session**, captured by `idx stockbit login` and stored locally | The one source that needs a credential and the one not in the `auto` chain. Volume is realtime; the order book carries the full depth ladder. See [docs/stockbit.md](docs/stockbit.md) |

Prices from the free sources are delayed as published; Stockbit is realtime for
your own account. This tool is for research and tooling; it is not investment
advice and not affiliated with IDX, KSEI, Yahoo, Microsoft or Stockbit.

## Web terminal

**https://ihsg.midoriresearch.com/terminal** runs the same commands in a
browser, no install: `brief BBCA`, `quote BBCA,BBRI`, `returns BBCA BBRI`,
`screen pe<10,div>5,mcap>5T --sort div`, `news ASII`, `ownership ticker BBCA`,
`ownership foreign`, `bandar BBCA --daily`, `bandar screen --preset
big-accumulation`, `bandar broker YP`, `actions BBCA`, `holidays --upcoming`,
`list --search bank`, `members`, and `tools` (all 33 MCP tools as buttons). The idx.co.id
commands (foreign flow, top brokers, the calendar, indices) are CLI and MCP
only, because idx.co.id refuses requests from Cloudflare Workers. `quote` with one symbol also draws a price chart (1D, 5D,
1M, 6M, YTD, 1Y, 5Y) with volume, the previous close and a crosshair readout.
The web `bandar screen` reads a scan the Worker runs after each close over the
session's 100 most traded stocks; the CLI scans live.
A Cloudflare Worker refreshes a D1 snapshot of the whole
IDX board every minute in trading hours and serves `/api/*` with the exact JSON
envelopes the CLI prints; everything else is fetched on demand and cached. The
Worker lives in [`web/`](web/) and answers on the custom domain only
(`workers_dev = false`, Host check in the Worker).

## Ownership data pipeline

Nobody imports the KSEI register by hand, and nothing outside the repository
can write it:

1. [`.github/workflows/ownership.yml`](.github/workflows/ownership.yml) runs at
   08:17, 14:17 and 20:17 WIB. It fetches the IDX "Data Kepemilikan Saham" page
   through a browser-impersonating client, downloads any new or re-published
   "Pemegang Saham di Atas 1%" workbook, imports every month with `idx` to
   prove it parses, and publishes the files plus `index.json` and parsed JSON
   parts to the [`ownership-data`](https://github.com/fzn0x/idx/releases/tag/ownership-data)
   prerelease. idx.co.id blocks cloud networks, GitHub's own runners included,
   so the job runs on a self-hosted runner (an always-on machine with an
   ordinary connection; set the repository variable `OWNERSHIP_RUNNER` to
   `self-hosted`). On GitHub's runners it keeps the mirror as is and turns red
   once the newest register is 45 days old.
2. The web terminal's Worker reads that release on its minute cron: it checks
   `index.json` every 30 minutes, verifies each part's SHA-256 and loads one
   part per minute into D1. A month appears only once all its rows are in and
   the count matches. The newest six months are kept. The Worker has no write
   endpoint at all.
3. `idx ownership import --latest` on your machine tries idx.co.id and, when
   Cloudflare refuses it, downloads the same file from the mirror and checks
   its SHA-256. `IDX_OWNERSHIP_SOURCE=mirror` skips idx.co.id.

<p align="center">
  <a href="https://ihsg.midoriresearch.com/terminal"><img src="docs/screenshots/terminal.png" alt="The idx web terminal showing technical BBRI" width="100%"></a>
</p>

## Development

```sh
cargo build
cargo test                                 # recorded fixtures, no network
cargo clippy --all-targets -- -D warnings
./scripts/smoke.sh                         # live providers
```

Architecture, conventions and "how to add a provider" live in
[AGENTS.md](AGENTS.md). Contributions: [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE)
