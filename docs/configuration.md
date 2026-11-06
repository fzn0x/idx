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
output = "table"       # table | json | csv
color = "auto"         # auto | always | never
timeout_secs = 15

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
| `IDX_OUTPUT` | `-o, --output` | `table`, `json` |
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
quotes and fundamentals and takes only candles from Stockbit.

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
- Network failure with a stale entry present: the stale entry is served and
  `stale: true` is set (table mode prints a warning to stderr).
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
