# MCP server

`idx mcp` serves the Model Context Protocol over stdin/stdout. It is the same
binary and the same code path as the CLI: every tool calls the provider layer
and returns the envelope documented in [json-output.md](json-output.md).

## Register idx with your agent

Run `idx mcp setup <client>` and paste what it prints: it fills in the
absolute path of the binary (GUI apps rarely inherit your shell `PATH`), and
`-o json` gives the same steps as data. `idx mcp setup --list` names every
client; `--npx` swaps the path for `npx -y ihsg-cli mcp`;
`--http URL [--token T]` produces the remote form of the same entry.

The snippets below use a bare `idx` for brevity. Formats were checked against
each vendor's documentation in October 2026; where a vendor offers both a CLI
command and a file, both are shown.

### Claude Code

```sh
claude mcp add --scope user idx -- idx mcp        # all projects
claude mcp add --scope project idx -e IDX_PROVIDER=yahoo -- idx mcp
```

Or commit `.mcp.json` in the repository:

```json
{ "mcpServers": { "idx": { "command": "idx", "args": ["mcp"] } } }
```

### Claude Desktop

`~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or
`%APPDATA%\Claude\claude_desktop_config.json` (Windows), then restart:

```json
{ "mcpServers": { "idx": { "command": "/abs/path/idx", "args": ["mcp"] } } }
```

For a remote endpoint use Settings → Connectors → Add custom connector.

### OpenAI Codex (CLI, IDE extension, desktop app)

```sh
codex mcp add idx -- idx mcp
```

All three read the same `~/.codex/config.toml` (`.codex/config.toml` in a
trusted project also works):

```toml
[mcp_servers.idx]
command = "idx"
args = ["mcp"]
```

### Meta Muse Code

`~/.config/muse/settings.json`. `schema_version` is mandatory; `mode:
optional` lets a session start even when idx is missing (the default
`required` aborts the run). `/mcp` inside a session lists servers.

```json
{
  "schema_version": 1,
  "mcp_servers": {
    "idx": { "transport": "stdio", "command": "/abs/path/idx", "args": ["mcp"], "mode": "optional" }
  }
}
```

Remote form: `{ "transport": "streamable_http", "url": "https://…/mcp", "headers": { "Authorization": "Bearer T" } }`.

### Google Antigravity (IDE and CLI)

`~/.gemini/config/mcp_config.json`, or `.agents/mcp_config.json` in the
workspace. Antigravity does not inherit your shell PATH and rejects a `type`
field, so keep the absolute command path. `/mcp` opens the manager.

```json
{ "mcpServers": { "idx": { "command": "/abs/path/idx", "args": ["mcp"] } } }
```

Remote form uses `"serverUrl"`.

### Gemini CLI

Free-tier Gemini CLI was replaced by Antigravity CLI in June 2026; paid
installs still read `~/.gemini/settings.json`.

```sh
gemini mcp add -s user idx idx mcp
```

### GitHub Copilot CLI

```sh
copilot mcp add idx -- idx mcp
```

Or `~/.copilot/mcp-config.json` (a committed `.mcp.json` is read too):

```json
{ "mcpServers": { "idx": { "type": "local", "command": "idx", "args": ["mcp"], "tools": ["*"] } } }
```

### VS Code (Copilot agent mode)

```sh
code --add-mcp "{\"name\":\"idx\",\"command\":\"idx\",\"args\":[\"mcp\"]}"
```

Or `.vscode/mcp.json` (note the `servers` key, not `mcpServers`):

```json
{ "servers": { "idx": { "type": "stdio", "command": "idx", "args": ["mcp"] } } }
```

### Cursor

`~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (project):

```json
{ "mcpServers": { "idx": { "command": "idx", "args": ["mcp"] } } }
```

### Windsurf

`~/.codeium/windsurf/mcp_config.json`, same `mcpServers` shape; remote form
uses `"serverUrl"`.

### Kiro (IDE and CLI; formerly Amazon Q Developer CLI)

`~/.kiro/settings/mcp.json` (user) or `.kiro/settings/mcp.json` (workspace):

```json
{ "mcpServers": { "idx": { "command": "idx", "args": ["mcp"], "disabled": false } } }
```

```sh
kiro-cli mcp add --name idx --command idx --args mcp     # older installs: q mcp add …
```

### JetBrains AI Assistant / Junie / Air

Settings → Tools → AI Assistant → Model Context Protocol (MCP) → + → As JSON,
paste the `mcpServers` snippet above.

### Zed

`settings.json` (Settings → AI → MCP Servers → Add Server also writes it):

```json
{ "context_servers": { "idx": { "command": "idx", "args": ["mcp"], "env": {} } } }
```

### Cline, Roo Code, Kilo Code

MCP Servers → Configure MCP Servers opens the JSON (`cline_mcp_settings.json`
or `mcp_settings.json`):

```json
{ "mcpServers": { "idx": { "type": "stdio", "command": "idx", "args": ["mcp"], "disabled": false } } }
```

### Continue

`~/.continue/mcpServers/idx.yaml`:

```yaml
name: idx
version: 0.0.1
schema: v1
mcpServers:
  - name: idx
    command: idx
    args: ["mcp"]
```

### OpenCode

`~/.config/opencode/opencode.json` (or `opencode.json` in the project);
`opencode mcp add` asks the same questions interactively:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": { "idx": { "type": "local", "command": ["idx", "mcp"], "enabled": true } }
}
```

### Goose

`goose configure` → Add Extension → Command-line Extension → name `idx`,
command `idx mcp`. Or `~/.config/goose/config.yaml`:

```yaml
extensions:
  idx:
    enabled: true
    type: stdio
    name: idx
    cmd: idx
    args: ["mcp"]
    timeout: 300
```

### Warp

Settings → AI → Manage MCP servers → + Add, paste the `mcpServers` snippet;
or `~/.warp/.mcp.json`.

### Factory Droid

```sh
droid mcp add idx "idx mcp"
```

Or `~/.factory/mcp.json` (`.factory/mcp.json` in a project):

```json
{ "mcpServers": { "idx": { "type": "stdio", "command": "idx", "args": ["mcp"] } } }
```

### Mistral Vibe

`~/.vibe/config.toml` (or `./.vibe/config.toml` in a trusted project). Tools
appear as `idx_<tool>`:

```toml
[[mcp_servers]]
name = "idx"
transport = "stdio"
command = "idx"
args = ["mcp"]
```

### Qwen Code

```sh
qwen mcp add idx idx mcp          # user scope by default; ~/.qwen/settings.json
```

### Crush (Charm)

`crush.json` in the project or `~/.config/crush/crush.json`:

```json
{ "$schema": "https://charm.land/crush.json", "mcp": { "idx": { "type": "stdio", "command": "idx", "args": ["mcp"] } } }
```

### Trae, Augment Code

Trae: AI Management → MCP → Add → Configure manually. Augment: panel →
Settings → MCP → Import from JSON. Both take the `mcpServers` snippet.

### ChatGPT, Claude.ai, Perplexity, Devin, Manus (remote only)

These products never launch local processes, so serve idx over HTTPS:

```sh
export IDX_MCP_TOKEN=$(openssl rand -hex 16)
idx mcp --http 127.0.0.1:8765 --allowed-host '*'
cloudflared tunnel --url http://127.0.0.1:8765     # or ngrok, tailscale funnel, …
```

Then add `https://<tunnel-host>/mcp` with header `Authorization: Bearer
<token>`: ChatGPT → Settings → Apps & Connectors → Advanced → Developer mode →
Create; Claude.ai → Settings → Connectors → Add custom connector; Perplexity →
Settings → Connectors; Devin → Settings → MCP Marketplace → Add custom; Manus →
Settings → Connectors. `idx mcp setup chatgpt --http URL --token T` prints the
filled-in version.

All `IDX_*` environment variables and the config file apply to every client,
so `--offline` behaviour, cache location, TTLs and provider choice can be set
per agent (`-e IDX_PROVIDER=yahoo` in the CLI forms, `env` in the files).

## HTTP transport

`idx mcp --http ADDR` serves MCP streamable HTTP at `http://ADDR/mcp` and a
plain `GET /healthz`. It is the same server and the same 42 tools; several
local agents can share one process and one cache this way.

- `--token T` (or `IDX_MCP_TOKEN`) requires `Authorization: Bearer T` on
  `/mcp`. It is mandatory when binding anything other than a loopback
  address and strongly recommended behind a tunnel.
- The `Host` header is checked against loopback names by default (DNS
  rebinding protection). Pass `--allowed-host my.tunnel.example` for a tunnel,
  or `--allowed-host '*'` to accept any host.
- Port 0 picks a free port; the chosen address is printed on stderr.
- Stop with Ctrl-C. Logs go to stderr as with stdio.

## Tools

| tool | arguments | returns |
| --- | --- | --- |
| `idx_brief` | `symbol` | `idx.brief.v1`: quote, fundamentals + verdict, technicals + signal, returns vs IHSG, dividend facts, top holders, headlines + sentiment, `key_points`, `sections_failed`. Start here. |
| `idx_market` | `top?`, `min_market_cap?` | `idx.market.v1`: IHSG quote, breadth, turnover, gainers, losers, most traded, heavyweights |
| `idx_market_foreign` | `date?`, `top?` (default 10), `sort?` (`value` \| `shares`) | `idx.market.foreign.v1`: top foreign net buying and selling across every stock for one session |
| `idx_market_brokers` | `date?`, `top?` (default 20, 0 = all), `sort?` (`value` \| `volume` \| `frequency`) | `idx.market.brokers.v1`: every exchange member's day, ranked, with its share of all members' value |
| `idx_indices` | `codes?` ("LQ45,IDX30"), `date?` | `idx.indices.v1`: every IDX index for one session |
| `idx_calendar` | `date?` or `from?`/`to?`, `days?` (default 14), `symbol?`, `types?` (`all`, or `dividend,rights,...`) | `idx.calendar.v1`: dividend and rights dates, RUPS, public exposes, listings (bonds and buybacks with `all`) |
| `idx_holidays` | `year?` (0 = every year), `upcoming?` | `idx.holidays.v1`: trading holidays, trading days, today and the next trading day (bundled, offline) |
| `idx_members` | `search?` | `idx.members.v1`: exchange members (broker codes) with licences |
| `idx_quote` | `symbols` ("BBCA,BBRI"; `^JKSE` is IHSG) | `idx.quotes.v1`; each row's `behind_session` flags a last trade dated before the latest IDX session (suspended, not traded, or the source lags) |
| `idx_history` | `symbol`, `period?`, `interval?`, `limit?` (default 60 most recent candles, 0 = all) | `idx.history.v1` |
| `idx_profile` | `symbol` | `idx.profile.v1` |
| `idx_financials` | `symbol`, `period?` (`annual` \| `quarterly` \| `ttm`) | `idx.financials.v1` |
| `idx_earnings` | `symbol` | `idx.earnings.v1` |
| `idx_dividends` | `symbol` | `idx.dividends.v1`: payments, splits, yearly totals, TTM total/yield, streak |
| `idx_returns` | `symbols`, `benchmark?` (`^JKSE` default, `none`) | `idx.returns.v1`: windows, CAGR, volatility, drawdowns, 52-week, excess over the benchmark |
| `idx_fundamental` | `symbol` | `idx.fundamentals.v1` (verdict included; when `financial_currency` differs from `currency`, P/B, P/S and the EV multiples mix USD and IDR and P/B is not scored) |
| `idx_technical` | `symbol`, `period?` | `idx.technical.v1` |
| `idx_compare` | `symbols` | `idx.compare.v1` |
| `idx_screen` | `filter`, `sort?`, `limit?`, `max_fundamentals?` | `idx.screen.v1` |
| `idx_news` | `symbol?`, `sources?`, `limit?`, `company_only?` | `idx.news.v1`: company items first (`scope=company`), then market context |
| `idx_sentiment` | `symbol`, `sources?` | `idx.sentiment.v1` |
| `idx_corporate_actions` | `symbol`, `since?` (default a year ago) | `idx.actions.v1`: dividends, splits, bonus, rights (amount, ratio, cum/ex/recording/payment dates, status), RUPS, warrants and rights, upcoming included (IPOT) |
| `idx_foreign` | `symbol`, `days?` (default 20, at most 250) | `idx.foreign.v1`: foreign buy, sell and net per session (shares), net × close, totals and streak (idx.co.id) |
| `idx_instruments` | `board?`, `search?` | `idx.instruments.v1`: listed stocks with board, listing date and shares |
| `idx_broker` | `symbols`, `date?` or `from?`/`to?`, `investor?` (`all` \| `foreign` \| `domestic`), `board?` (`all` \| `regular` \| `cash` \| `negotiated`) | `idx.broker.v1`: top 10 buyers and sellers (ranked by lots) with broker names and class, totals, foreign net; latest session with trades when no dates are given |
| `idx_bandar` | `symbols`, `period?` (`1d` \| `1w` default \| `1m` \| `3m`), `date?` or `from?`/`to?`, `top?` (1, 3, 5 default, 10), `investor?`, `board?`, `daily?` | `idx.bandar.v1`: bandarmology per ticker: top-N net as a share of value, signal and phase, tiers, the top buyers' average against the price, foreign flow, reasons; `daily` adds the session series |
| `idx_bandar_screen` | `preset?`, `filter?`, `period?`, `top?`, `sort?`, `ascending?`, `limit?`, `scan?` (default 100), `symbols?` | `idx.bandar.screen.v1`: the most traded stocks the top brokers accumulate or distribute |
| `idx_bandar_broker` | `code`, `period?`, `limit?` (default 20), `scan?` (default 100), `symbols?` | `idx.bandar.broker.v1`: where one broker is a top-10 buyer or seller across the scanned stocks, with net where both sides are visible |
| `idx_ownership_ticker` | `symbol`, `as_of?` | `idx.ownership.holders.v1` |
| `idx_ownership_entity` | `name`, `as_of?`, `exact?` (match one `entity_key`) | `idx.ownership.entity.v1` |
| `idx_ownership_changes` | `from`, `to`, `symbol?`, `min_pct_change?`, `limit?`, `offset?` | `idx.ownership.changes.v1`: `summary` over every match, one page of rows |
| `idx_ownership_foreign` | `symbol?`, `from?`, `to?`, `limit?` | `idx.ownership.foreign.v1` (one issuer over time) or `idx.ownership.foreign-flow.v1` (every issuer between two months) |
| `idx_ownership_releases` | none | `idx.ownership.releases.v1` |
| `idx_ownership_search` | `query`, `limit?` (default 20, 0 = all, at most 100), `as_of?` | `idx.ownership.search.v1`: holders matching a name (every spelling, words in any order) with their `entity_key`, spellings, issuers held and `last_seen` |
| `idx_ownership_rankings` | `view?` (`holders` default \| `issuers`); holders: `min_issuers?` (default 2), `investor_type?`, `origin?` (`foreign` \| `local`); issuers: `symbols?`, `by?` (`hhi` default \| `top1` \| `top3`), `ascending?`; `limit?` (default 25, 0 = all), `as_of?` | `idx.ownership.cross-holders.v1` (holders by the issuers they own above 1 %) or `idx.ownership.concentration.v1` (issuers by HHI, top-1 and top-3 stakes, undisclosed percent, controller stake) |
| `idx_ownership_graph` | `root` (ticker or holder), `depth?` (1-6, default 2), `min_pct?`, `max_nodes?` (default 100, at most 1000), `holder?`, `as_of?` | `idx.ownership.graph.v1`: issuers and holders with hop counts and the links between them; `truncated` when `max_nodes` stopped the walk |
| `idx_ownership_composition` | `symbol`, `as_of?` | `idx.ownership.composition.v1`: KSEI's monthly archive, local vs foreign shares in nine investor types as a percent of the scripless shares, `scripless_pct`, and every imported month. Downloads the newest month from ksei.co.id on first use (not with `--offline`) |
| `idx_ownership_snapshot` | `action` (`status` \| `sync`), `force?` (sync only) | `status`: `idx.ownership.status.v1`, the local database, the snapshot it came from, the mirror and what sync would do (writes nothing). `sync`: `idx.ownership.snapshot-sync.v1`, installs the prebuilt snapshot or imports the missing months; months only the local database has are kept unless `force` (a `.bak` copy is kept) |
| `idx_ownership_institutions` | `symbol` | `idx.ownership.institutions.v1`: institutions and funds holding the stock (MSN, live, no import), each with shares, % of shares outstanding, the change since its own previous report and a signal (`new`, `buyer`, `seller`, `exited`, `unchanged`), plus institutional, fund and free-float percentages and signal counts per list |
| `idx_orderbook` | `symbol` | `idx.orderbook.v1`: realtime depth ladder (bids/asks, shares), last/OHLC, volume, value and foreign buy/sell/net (null, with a warning, while Stockbit marks them as not current, seen during trading). Needs a Stockbit login (`--provider stockbit`) |
| `idx_stockbit_status` | none | whether a Stockbit session is stored and when its tokens expire (never the token values) |
| `idx_stockbit_login` | `timeout?` | opens a browser on the machine running the server to sign in to Stockbit and store the session; local (stdio) servers only |

The last three carry your own Stockbit session (opt-in realtime); see
[stockbit.md](stockbit.md).

Paging: `idx_ownership_changes` returns `total_matches` and `offset`; ask for
the next page with `offset = offset + limit` until `offset + rows >= total_matches`.
The `summary` block never depends on the page, so one call is enough to
describe a month.

Ownership rows are positions: a holder's register lines (scrip and scripless
accounts) are summed, with `accounts` saying how many, and from the 30 Oct
2026 register each row carries `controller_or_affiliate`
(`controller_reported` on the report says whether the release has it).
`idx_ownership_entity` needs at least 3 letters or digits in `name`.

To look a holder up, call `idx_ownership_search` first and pass the
`entity_key` it returns to `idx_ownership_entity` with `exact: true`, or to
`idx_ownership_graph` as `root`. `idx_ownership_rankings` passes view-specific
arguments to one view only. An argument of the other view is a usage error
unless it holds its default: `symbols`, `by` other than `hhi` or
`ascending: true` with `view: "holders"`, and `min_issuers` other than 2,
`investor_type` or `origin` with `view: "issuers"`. The spelling rules of
`idx ownership resolve` (merging holders KSEI printed two ways) change the
user's database, so they stay in the CLI; the MCP tools read the result.

`idx_ownership_composition` reads a second dataset: KSEI's "Kepemilikan Efek"
archive, which counts every holder and names none. It covers the shares held
in KSEI accounts only, so its percents are of those (`scripless_pct` says how
much of the listed shares that is). The first call downloads the newest month
(about 125 KB) into the same local database; later calls read it and ask
ksei.co.id again only once a newer month can exist. `provider` is `ksei` when
the call imported a month, `local-db` otherwise.

An empty ownership database is filled by `idx_ownership_snapshot` with
`action: "sync"`: one download of the SQLite file CI builds from the mirrored
IDX files (SHA-256 checked, swapped in as one SQLite transaction, so other
calls reading the file are not disturbed), then any newer monthly file. It
needs the network (an `offline_miss` error with `--offline`); `action:
"status"` works offline and reports the local part. See
[ownership.md](ownership.md#snapshot-and-sync).

## Harness compatibility

Input schemas are rewritten before they are advertised: no `$schema`, no
`format` (`uint32`, `double`), and optional fields are typed `"string"` not
`["string", "null"]` (they are simply absent from `required`). Claude accepts
either form; Gemini function declarations and OpenAI strict tool mode reject
the latter, and a unit test plus the stdio test keep it that way.

Tool results are a single `text` content block holding the pretty-printed
envelope. Failures set `isError: true` and the text is the CLI error object:

```json
{"error":{"code":"not_found","message":"not found: ZZZZ: Quote not found for symbol: ZZZZ.JK"},"exit_code":4}
```

`code` values and their meaning are listed in json-output.md. `blocked`
(exit 3) means the source's bot protection (Cloudflare, idx.co.id) refused
this network: retry later, rely on cached data, or for the ownership register
let the GitHub mirror serve it.

## Behaviour notes

- Logs go to stderr (`-v` raises the level, `IDX_LOG` overrides). stdout
  carries only protocol frames.
- Responses are cached exactly like the CLI, so an agent calling
  `idx_quote` twice within the quote TTL hits the network once.
- A cached copy older than its TTL is served when the source fails (or the
  server runs with `--offline`): the envelope then has `stale: true`,
  `stale_reason` (`fetch_failed` or `offline`) and, after a failure, a warning
  naming it.
- The server exits cleanly when the client closes stdin.
- Verdicts and signals are heuristics; the tool descriptions and the server
  instructions say so, and so should any agent that repeats them.
- Tickers are IDX codes. The ten Yahoo-backed tools (`idx_quote`,
  `idx_history`, `idx_profile`, `idx_financials`, `idx_earnings`,
  `idx_dividends`, `idx_returns`, `idx_fundamental`, `idx_technical`,
  `idx_compare`) also take a ticker from another Yahoo Finance market with its
  suffix (`D05.SI`), returned with that suffix; every other tool refuses one
  with a `usage` error. The server does not use `IDX_EXCHANGE`, so a bare
  code always means IDX, though an invalid value fails startup like any
  other invalid setting (exit 6).

## Verifying

```sh
claude mcp get idx            # health check: "Connected"
cargo test --test mcp         # JSON-RPC over stdio without network
```
