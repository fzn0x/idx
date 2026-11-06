# Stockbit realtime (opt-in)

`idx` reads IDX data from free, keyless public endpoints. **Stockbit is the one
exception**: an opt-in provider that serves realtime quotes, history and a live
order book from [Stockbit](https://stockbit.com)'s own backend, carrying **your
own Stockbit account session**. It is never in the `auto` chain and is used only
when you ask for it with `--provider stockbit`.

> This uses an unofficial, undocumented API with your personal account. It may
> conflict with Stockbit's Terms of Use, and automated access could get an
> account flagged. Use your own account, at your own risk, at a human-ish
> request rate. `idx` is not affiliated with or endorsed by Stockbit.

## Log in once

```sh
idx stockbit login
```

This launches a Chromium-family browser (Chrome, Edge, Brave; set `IDX_BROWSER`
to choose one) pointed at Stockbit's login page. You sign in — password, OTP,
whatever Stockbit asks — and `idx` captures the session token off the network
automatically. If you are already signed in in that browser profile, it reuses
that session in a couple of seconds.

The session is stored locally (see **Where the session lives**) and refreshed
for you from then on. Check it any time:

```sh
idx stockbit status        # logged in? when do the tokens expire?
idx stockbit logout        # delete the stored session
```

No Chromium browser, or a login that will not drive? Paste the token yourself:

```sh
idx stockbit login --manual
```

It prints where to copy the `credentialStorage` cookie from (DevTools →
Application → Cookies → `https://stockbit.com`) and reads it from stdin.

## Use it

Once logged in, point any quote/history command at the provider:

```sh
idx --provider stockbit stocks quote BBCA BBRI      # realtime, not delayed
idx --provider stockbit stocks history BBCA --period 1mo
idx --provider stockbit stocks intraday BBCA        # today's one-minute bars, realtime
idx config set provider stockbit                    # make it the default
idx config set history_provider stockbit            # or: candles only, free providers for the rest
```

`history_provider` (`--history-provider`, `IDX_HISTORY_PROVIDER`) picks where
candles come from apart from `provider`. With `history_provider = "stockbit"`
the free providers keep serving quotes, fundamentals, financials and the rest
while history, technicals, returns and the brief take Stockbit's candles. Under
`provider = "stockbit"` the default `history_provider = "auto"` keeps on
Stockbit only the requests whose bars Stockbit returns as asked (daily over
`1mo` to `1y`, hourly over `5d`, weekly over `5y`, same-day intraday) and sends
the rest (`2y` or `5y` daily, `1y` weekly, any monthly bars) to Yahoo with a
note; when
the session fails it falls back to Yahoo with a warning. `history_provider =
"stockbit"` keeps every window on Stockbit. See
[configuration.md](configuration.md#history-provider).

And read the live order book (Stockbit-only — the free sources have no depth):

```sh
idx stockbit orderbook BBCA            # depth ladder + session foreign flow
idx stockbit orderbook BBRI --depth 5
idx -o json stockbit orderbook TLKM | jq '.data.bids[0]'
```

Sizes in the order book are **shares** (one IDX lot is 100 shares); values are
IDR. History candle volumes are converted from Stockbit's lots to shares so they
match the other providers. Stockbit chooses the candle granularity and span from
the window (`1d` is one-minute; `5d`, and intraday intervals over any longer
period, hourly over one week; `1mo` to `1y` daily; `2y` weekly over three years;
`5y` and longer weekly over five years), and a warning says so whenever those
bars are not the `--interval` asked for or that span is not the `--period`
(`--period 1y --interval 1wk` gets daily bars and a warning; `--period 1mo
--interval 1h` gets one week of hourly bars and a warning, since the candles
are not trimmed to the period; `--period 5d --interval 1h` gets the hourly bars
it asked for and none).

Foreign flow in the order book (`foreign_buy`, `foreign_sell`, `foreign_net`)
comes with a flag, `has_foreign_bs`. While it is `false` the figures are not the
current session's: a BBCA book recorded in session 2 on 2026-11-03 carried a
foreign net of -116.1B, which is IPOT's figure for 2026-11-02 (2026-11-03 itself
closed at -28.8B). So idx keeps the three figures only while Stockbit marks them
current (`true`, as in idx's own books recorded after the close) and otherwise
leaves them empty, with a warning that says why (on stderr in a table, in
`warnings` in JSON). A missing or non-boolean flag counts as not current. The
`false` book came from another account (recorded by 0xrsydn/idx-cli), and its
`orderbook_active_feature_mobile` differs too (`ORDER_BOOK_FEATURE_UNSPECIFIED`,
where idx's books say `ORDER_BOOK_FEATURE_FOREIGN_BS`), so it is still open
whether the flag follows the session clock or an account feature. idx reads only
the flag, so either way the figures show only when Stockbit marks them current.

`stocks intraday` uses whichever provider is selected, so with
`--provider stockbit` it shows today's realtime one-minute bars from your
session during trading hours (none outside them; the quote still answers),
while the default providers serve Yahoo's delayed bars without a login.

The Stockbit provider covers `quote`, `history` and `orderbook`. The other
commands (fundamentals, financials, news, …) stay on the free providers; run
them without `--provider stockbit`, or keep `provider` on `auto` and set only
`history_provider = "stockbit"`.

## MCP

The MCP server exposes three Stockbit tools alongside the rest:

- `idx_stockbit_status` — is a session stored, and when do the tokens expire
  (never the token values). Call it before `idx_orderbook`.
- `idx_orderbook` — the live depth ladder and foreign flow for one ticker.
- `idx_stockbit_login` — opens a browser **on the machine running the server**
  for the user to sign in; only meaningful for a local stdio server the user is
  sitting at, never a remote HTTP one. Prefer telling the user to run
  `idx stockbit login` in a terminal themselves.

To serve realtime quotes/history over MCP, run the server with the Stockbit
provider: `idx --provider stockbit mcp`. For Stockbit candles with the free
providers for everything else, register it with
`-e IDX_HISTORY_PROVIDER=stockbit` instead.

## Where the session lives

The session file holds the rotating tokens and is written with `0600`
permissions on Unix. By default it sits beside the config file (e.g.
`%APPDATA%\idx\stockbit-session.json` on Windows,
`~/.config/idx/stockbit-session.json` on Linux). Override it:

- `--config`-relative default, or `stockbit.session` in `config.toml`;
- `IDX_STOCKBIT_SESSION=/path/to/session.json`.

For an unattended deployment (a server, the web worker) seed the tokens through
the environment instead of logging in interactively:

- `IDX_STOCKBIT_REFRESH_TOKEN` — the refresh token (required);
- `IDX_STOCKBIT_ACCESS_TOKEN` — an access token to start with (optional).

The file always wins over the env seeds; the seeds only fill an empty session.

## One refresher per session

Stockbit's `POST /login/refresh` **rotates** the refresh token: every refresh
mints a new one and retires the token it was given. So a single session must
have exactly one process refreshing it. If you point two tools (say `idx` and
another bot) at the *same* Stockbit session, they will retire each other's token
and both get logged out. Give each its own `idx stockbit login` / account, or
let one own the refresh and feed the other its current token.

## What `idx` talks to

All on `https://exodus.stockbit.com`, with `Authorization: Bearer <access JWT>`
and browser-shaped `origin`/`referer` headers:

| endpoint | used for |
| --- | --- |
| `GET /company-price-feed/v2/orderbook/companies/{SYM}` | quote (OHLC, volume, value, foreign flow only while `has_foreign_bs` is true) and the full depth ladder |
| `GET /charts/{SYM}/daily?timeframe=..&chart_type=PRICE_CHART_TYPE_CANDLE` | OHLCV candles |
| `POST /login/refresh` | rotate the session (bearer = refresh token, empty body) |

Indices use Stockbit's own code (`IHSG`, `LQ45`), mapped from idx's caret form
(`^JKSE`, `^JKLQ45`). The token is never written to a log, an error message or a
cache key.
