# Configuration

Precedence, highest first: **command-line flags → `IDX_*` environment
variables → config file → built-in defaults**.

## Config file

| platform | path |
| --- | --- |
| Linux, macOS | `$XDG_CONFIG_HOME/idx/config.toml`, else `~/.config/idx/config.toml` |
| Windows | `%APPDATA%\idx\config.toml` |

Override the location with `--config PATH` or `IDX_CONFIG=PATH`.

```sh
idx config init                       # write a commented template
idx config set provider yahoo
idx config set history_provider stockbit   # candles from your Stockbit session
idx config set cache.quote_ttl_secs 120
idx config get                        # effective settings after all layers
idx config get provider               # one key, as stored in the file
idx config path
```

```toml
provider = "auto"      # auto | yahoo | msn | stockbit
history_provider = "auto"  # auto | yahoo | stockbit: where candles come from
output = "table"       # auto | table | json | jsonl | csv
color = "auto"         # auto | always | never
timeout_secs = 15
exchange = "JK"        # Yahoo suffix for bare tickers in the Yahoo-backed commands

[cache]
dir = "/custom/cache/dir"
quote_ttl_secs = 60
history_ttl_secs = 3600

[stockbit]
# session = "/path/to/stockbit-session.json"   # `idx stockbit login` writes it
```

## Environment variables

| variable | flag | meaning |
| --- | --- | --- |
| `IDX_PROVIDER` | `--provider` | `auto`, `yahoo`, `msn`, `stockbit` |
| `IDX_HISTORY_PROVIDER` | `--history-provider` | `auto` (default), `yahoo`, `stockbit`: where candles come from, see below |
| `IDX_OUTPUT` | `-o, --output` | `table` (default), `json`, `jsonl`, `csv`, or `auto`: a table when stdout is a terminal, `json` when it is piped or captured |
| `IDX_QUIET` | `-q, --quiet` | `1`/`true`/`yes`/`on`: stderr carries errors only (no warnings, notes or loading line); ignored when `-v` is given. No config key |
| `IDX_EXCHANGE` | | Yahoo Finance suffix a bare ticker gets in `stocks quote`, `history`, `profile`, `financials`, `earnings`, `dividends`, `returns`, `fundamental`, `technical`, `compare` and `watch`: `JK` (default, IDX), `SI`, `HK`, ... One to four letters, an optional leading dot, any case; `IDX` means `JK`. Config key `exchange` |
| `IDX_COLOR` | | `auto`, `always`, `never` |
| `NO_COLOR` | `--no-color` | any non-empty value disables colour |
| `IDX_PROGRESS` | | `auto` (default): a loading line on stderr while a command waits on the network, naming the hosts it waits on; only when stderr is a terminal and without `-v`. `never` turns it off, `always` shows it where stderr does not report a terminal (mintty, some IDE consoles) |
| `IDX_TIMEOUT_SECS` | `--timeout` | per-request timeout |
| `IDX_CACHE_DIR` | `--cache-dir` | cache location |
| `IDX_OFFLINE` | `--offline` | `1`/`true`: never touch the network |
| `IDX_NO_CACHE` | `--no-cache` | `1`/`true`: ignore cached responses |
| `IDX_QUOTE_TTL_SECS` | | cache lifetime for quotes |
| `IDX_HISTORY_TTL_SECS` | | cache lifetime for history (intraday bars are capped at 120 s) |
| `IDX_CONFIG` | `--config` | config file path |
| `IDX_LOG` | `-v` | tracing filter, e.g. `idx=debug` |
| `IDX_HOLIDAYS` | | CSV (`date,name,name_en`) replacing the bundled trading-holiday list (`data/holidays.csv`) |
| `IDX_STOCKBIT_SESSION` | | Stockbit session file (`idx stockbit login` writes it); default beside the config file. See [stockbit.md](stockbit.md) |
| `IDX_STOCKBIT_REFRESH_TOKEN` | | Seeds an unattended Stockbit session (the refresh token); the session file wins once it exists |
| `IDX_STOCKBIT_ACCESS_TOKEN` | | Optional access token to seed alongside the refresh token |
| `IDX_BROWSER` | | Browser executable `idx stockbit login` launches (default: the first Chrome/Edge/Brave found) |

## Output on a terminal and in a pipe

`table` stays the default whether stdout is a terminal or not, so existing
`idx ... > file` and `| grep` pipelines keep reading tables. Opt in to
`output = "auto"` (or `IDX_OUTPUT=auto`) for a table on a terminal and the
JSON envelope whenever stdout is piped or captured, which suits agents that
forget `-o json`. `idx config get` shows the setting next to what it resolves
to (`output_setting` and `output` in its JSON).

Errors and the commands that need no data source (`version`, `schema`,
`config`, `mcp setup`) resolve the format the same way, flag first, then
`IDX_OUTPUT`, then the config file; a layer that does not parse is skipped
there, so a broken setting still gets a readable error.

## Other markets (`exchange`)

Yahoo Finance lists IDX shares under `.JK`, which idx adds for you. The
Yahoo-backed commands also take a ticker from another Yahoo market with its
suffix (`idx stocks history D05.SI`), or bare once `exchange` says which
market (`idx config set exchange SI`, then `idx stocks history D05`). `.JK`
always means IDX, and every other command (brief, screen, news, broker,
bandar, foreign, actions, market, ownership, stockbit) reads IDX tickers only
and refuses `D05.SI` with exit 2. MSN and Stockbit cover IDX only, so with
`--provider auto` Yahoo answers for other markets. Saved watchlist entries
keep their suffix (`D05.SI`), so changing `exchange` later does not change
them. For the same reason idx's own output piped back through `-` (`-o json`,
`-o jsonl`, a watchlist file) reads a bare code as IDX; only typed text on
stdin follows `exchange`. MCP tools take explicit suffixes but do not use
`IDX_EXCHANGE`: a bare code is always IDX there. An invalid value still stops
`idx mcp` from starting, as any invalid setting does.

## History provider

`history_provider` decides where candles come from, separately from
`provider`. Every command that reads candles follows it: `stocks history`,
`technical`, `returns`, `brief`, `bandar` price moves, `watch returns` and the
MCP tools behind them.

| `provider` | `history_provider` | candles from |
| --- | --- | --- |
| `auto`, `yahoo` | `auto`, `yahoo` | Yahoo (unchanged) |
| `msn` | `auto`, `yahoo` | Yahoo: MSN has no price history |
| `stockbit` | `auto` | Stockbit where its bars are the ones asked for (daily over `1mo` to `1y`, hourly over `5d`, weekly over `5y`, and same-day intraday), Yahoo otherwise; Yahoo also when Stockbit fails (no session, expired token, network) |
| `stockbit` | `yahoo` | Yahoo |
| any | `stockbit` | Stockbit, no fallback (needs `idx stockbit login`, exit 2 without it) |

Stockbit picks the bar size from the window, so under `provider = "stockbit"`
a `2y` or `5y` daily request would come back weekly and feed weekly returns
into daily analytics; `auto` sends those to Yahoo and says so in a warning.
A fallback after a Stockbit failure is a warning too
(`stockbit history failed (...); candles from yahoo`), and the JSON envelope's
`provider` names the source the candles came from. `history_provider =
"stockbit"` with the default `provider = "auto"` keeps the free providers for
quotes and fundamentals and takes only candles from Stockbit; a request it does
not serve as asked then carries a warning naming the bars and span it returned
(`--period 1mo --interval 1h` comes back as one week of hourly bars).

## Cache

| platform | path |
| --- | --- |
| Linux, macOS | `$XDG_CACHE_HOME/idx`, else `~/.cache/idx` |
| Windows | `%LOCALAPPDATA%\idx\cache` |

Each response is stored as one JSON file named by the SHA-256 of its cache
key. Freshness is judged against the TTL in effect when you *read*, so changing
a TTL applies immediately.

- `--offline`: serve whatever is cached, fresh or stale; exit 5 on a miss.
- `--no-cache`: always fetch; the result is still written for later.
- `--offline` with an entry older than its TTL: the entry is served with
  `stale: true` and `stale_reason: "offline"`.
- Network failure with a stale entry present: the stale entry is served with
  `stale: true`, `stale_reason: "fetch_failed"` and a warning naming the
  failure (`live fetch failed (...); showing cached copy`). Table mode prints
  the warnings and `warning: showing stale cached data from <fetch time> WIB`
  to stderr.
- idx.co.id's bot protection (Cloudflare) refusing a request counts as such a
  failure. A challenged request is asked again after 5, 10 and 15 s (four
  tries, at most 30 s of waiting per request; the loading line shows
  `www.idx.co.id retry 1`); one met by a block page is not. A refusal is
  never cached.
  An empty copy of a day's stock, broker or index summary does not stand in
  for a failed fetch, as it may be from before the summary was out: the
  error stands and names the copy.
  With nothing cached the error is `blocked` (exit 3) and says what to do;
  under `--no-cache` it says the cache was skipped and that `--offline`
  serves a copy if there is one.
- Network error text and warnings never include a URL's query string, and
  cache keys never hold a session token, so the Yahoo crumb and the MSN key
  stay out of every message; `-vv` debug logging still prints full request
  URLs.
- News feeds: a feed that times out or refuses the connection is not asked
  again for five minutes. Until then its cached copy is served with a warning
  naming the failure, or the warning alone when nothing is cached, so one
  silent feed does not make every call wait out `--timeout`. `--no-cache`
  asks anyway.

```sh
idx cache stats
idx cache clear
idx cache path
```

The Yahoo session (anonymous cookie + crumb) is cached for six hours under the
same directory. `idx cache clear` removes it too.
