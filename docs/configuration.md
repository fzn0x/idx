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
idx config set cache.quote_ttl_secs 120
idx config get                        # effective settings after all layers
idx config get provider               # one key, as stored in the file
idx config path
```

```toml
provider = "auto"      # auto | yahoo | msn | stockbit
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
- Error text, warnings and log lines never include a URL's query string, so
  the Yahoo crumb and the MSN key stay out of them; `-vv` debug logging still
  prints full request URLs.
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
