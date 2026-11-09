# Stockbit realtime (opt-in)

`idx` reads IDX data from free, keyless public endpoints. **Stockbit is the one
exception**: an opt-in provider that serves realtime quotes, history and a live
order book from [Stockbit](https://stockbit.com)'s own backend, carrying **your
own Stockbit account session**. It is never in the provider `auto` chain:
quotes and history use it only when you ask for it with `--provider stockbit`.
The same session also reads what only Stockbit has in one place: every corporate
action of a stock since its listing, the market's corporate-action calendar, and
the insider and 5 % holder movements filed with IDX and KSEI (`idx stockbit
actions`, `idx stockbit calendar`, `idx stockbit movements`). And `idx broker`
lists every broker that bought and sold a stock from it, by itself once you are
logged in (see **Broker summary**).

> This uses an unofficial, undocumented API with your personal account. It may
> conflict with Stockbit's Terms of Use, and automated access could get an
> account flagged. Use your own account, at your own risk, at a human-ish
> request rate. `idx` is not affiliated with or endorsed by Stockbit.

## Log in once

```sh
idx stockbit login
```

idx never asks for your Stockbit password. You sign in on stockbit.com itself,
and idx takes only what that sign-in leaves behind: the `credentialStorage`
cookie, which holds the session's two tokens. Whichever way it arrives, idx
first stores it (`0600`), then checks it with one request to Stockbit; when
Stockbit refuses it, the previous session is put back (or none, if there was
none) and nothing changes. When the session already stored still works and
idx can renew it, `login` says so and stops; `--force` signs in anyway.

**In a browser idx opens** (the default on a desktop): idx launches a
Chromium-family browser (Chrome, Edge, Brave; `--browser` or `IDX_BROWSER`
chooses one) on Stockbit's sign-in page, with a profile only idx uses
(`stockbit-chrome-profile` beside the session file; `--profile` moves it). You
sign in there, with whatever Stockbit asks (password, OTP, captcha). idx reads
nothing but cookies over the Chrome DevTools Protocol: it intercepts no request,
so the password never passes through idx. Once Stockbit accepts the session,
idx closes Stockbit's tab, deletes the cookie from its profile (so a later run
cannot pick up a pair idx has since rotated) and closes the browser. The
profile keeps Stockbit's other cookies, so the next sign-in may skip a step.
Google sign-in refuses browsers driven this way ("This browser or app may not
be secure"); use the local page instead.

**Through a local page** (`--page`; also chosen by itself over SSH, without a
display, or without a Chromium-family browser):

```sh
idx stockbit login --page                 # prints http://127.0.0.1:7878/login/<secret>
idx stockbit login --page --port 7878     # pin the port, e.g. for an SSH forward
```

idx serves a page on 127.0.0.1 only, at a path with a random secret, for the
browser you already use (any browser, Firefox included). Over SSH it also prints
the forward to run on the machine with the browser, for example
`ssh -N -L 7878:127.0.0.1:7878 you@server`; then open the URL there. On the
page:

1. Open a **private window** and sign in at stockbit.com (Google sign-in works
   here). idx then holds a session of its own while your everyday browser keeps
   its own.
2. Drag **Send to idx** to the bookmarks bar, then click that bookmark on the
   stockbit.com tab. It runs only on stockbit.com, reads `credentialStorage`
   there and opens the page with the value in the address fragment, which the
   browser never sends to any server. A bookmark from an earlier run carries an
   old secret: drag it again.
3. Or paste the cookie into the page (DevTools > Application > Cookies, or the
   clipboard button).

The page posts only that value to idx, which checks it with Stockbit and
answers on the page. Once the session is stored, the tab that sent it raises
an alert, and every tab still open on the page (the one you started from too)
turns to "Connected" with where to look next: the terminal where you ran
`idx stockbit login`, which prints the session's status, or, when the MCP
server opened the page, your agent. Close the private window afterwards
**without signing out**. The page answers only on its secret path, only to a loopback `Host`, and
takes a POST only from a loopback origin; it keeps no copy, sets `no-store`,
`no-referrer` and a CSP that allows nothing outside the page, accepts one
session and stops (or after `--timeout`, 300 seconds by default). Port 7878 is
the default; when it is taken idx picks a free one, `--port` pins it and
`--port 0` takes any.

**By pasting the cookie**, from any browser:

```sh
idx stockbit login --manual                  # paste it into this terminal
pbpaste | ssh you@host idx stockbit login    # or pipe it; a pipe on stdin means paste
```

Sign in at stockbit.com (a private window keeps your everyday browser's session
apart, see **One refresher per session**), open DevTools (F12) > Application >
Cookies > `https://stockbit.com` and copy the value of `credentialStorage`. A
bare refresh token works too. Close that window afterwards without signing out:
signing out there may end the session idx now holds.

The session is stored locally (see **Where the session lives**) and refreshed
for you from then on. Check it any time:

```sh
idx stockbit status        # logged in? when do the tokens expire?
idx stockbit logout        # delete the stored session
```

`status` reads the stored tokens' own expiry and sends nothing. The access
token lasts about a day and the refresh token about a week. idx refreshes
only when a request needs it (the access token has expired, expires within the
hour, or Stockbit refused it), and each refresh replaces both tokens, so a
request made while the access token is still good renews nothing. The refresh
token therefore runs out about a week after the last refresh, which can be up
to a day short of a week after the last use; using idx at least every six days
keeps the session working. Once the refresh token has run out too, `status`
says logged in `no`, with a note that the session expired and that
`idx stockbit login` renews it (`"logged_in": false, "expired": true` in JSON).
While only the access token can still be used (its refresh token expired, or
none is stored), logged in is `yes` with a note that the session stops working
when that token expires (`"renewable": false`). A session Stockbit ended early
(signed out elsewhere, or refreshed by another tool) still reads as logged in
until a request finds out: that request fails naming `idx stockbit login`, and
the process that sent it (a running `idx mcp` server, say) then reports that
Stockbit refused the refresh token; `idx stockbit status`, a process of its
own, cannot tell.

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

`stocks intraday` takes its bars where candles come from, so with
`--provider stockbit` or `history_provider = "stockbit"` it shows today's
realtime one-minute bars from your session during trading hours; the default
providers serve Yahoo's delayed bars without a login. When the day's window is
still empty, idx retries over five days only with a source whose five-day
window has the bars asked for: under `--provider stockbit` that is Yahoo (the
last session's bars, with a warning naming its date), while with
`history_provider = "stockbit"` there are no bars until Stockbit has some
(Stockbit's five-day window is hourly; the quote still answers).

The Stockbit provider covers `quote`, `history` and `orderbook`. The other
commands (fundamentals, financials, news, …) stay on the free providers; run
them without `--provider stockbit`, or keep `provider` on `auto` and set only
`history_provider = "stockbit"`.

## Corporate actions and calendar

```sh
idx stockbit actions BBCA                         # every kind but ipo, newest first
idx stockbit actions TLKM --kind dividend
idx stockbit actions BBRI --kind split,bonus,rights --from 2015-01-01
idx stockbit calendar                             # the next 14 days, oldest first
idx stockbit calendar --kind dividend --days 30
idx stockbit calendar --kind ipo,tender --from 2026-01-01 --to 2026-12-31
```

One structured row per action, with the dates and terms and no announcement
text (schema `idx.stockbit.actions.v1`). Kinds: `dividend`, `stock_dividend`,
`bonus`, `rights`, `warrant`, `split`, `reverse_split`, `rups` (alias
`meeting`), `public_expose` (`expose`), `ipo`, `tender_offer` (`tender`), or
`all`. Each kind is one request, a second apart, cached for 6 hours; a kind that
fails leaves a warning and the others still show. The pause holds across every
command and MCP tool call on the same session in one idx process, so parallel
tool calls queue a second apart too.

- A row's `date` is its cum date for dividends, bonuses, rights and splits (the
  ex date when there is none), else the event's first day: a warrant's first
  trading day, the meeting or expose day, an IPO's listing day, a tender offer's
  opening day.
- `stockbit actions` reads a stock's whole history: BBCA's cash dividends go
  back to 2009, its splits to 2004. `--from`/`--to` narrow it. The default is
  every kind but `ipo`.
- `stockbit calendar` reads the market-wide lists and keeps the rows dated in
  the range (at most 366 days; default today and the next 14), plus rights
  trading windows, tender offers and IPO offerings still open in it. The default
  is every kind but `warrant`: most market-wide warrant rows are structured
  warrants, listed under their own codes (`ASIIZPCN7A`), dozens a month.
- As recorded on 2026-11-03, the market-wide lists reach about a year back and
  as far ahead as announced, except RUPS and public exposes, whose market lists
  started on the recording day. The stock dividend and reverse split lists were
  empty that day; their rows are read by the field names the other entitlement
  rows use.
- `ratio_old : ratio_new` is as published (1 : 5 for a five-for-one split, 655
  held : 104 rights). `price` is the rights or warrant exercise price, the tender
  offer price, or the final IPO price, IDR per share. Anything Stockbit leaves
  empty or zero is `null`.

Without a login, `idx stocks actions` (IPOT, with the announcement texts) and
`idx market calendar` (the exchange's own calendar) cover similar ground.

## Insider and 5 % holder movements

```sh
idx stockbit movements BBCA                      # one stock, newest first
idx stockbit movements                           # the whole market
idx stockbit movements --source ksei --action sell
idx stockbit movements --from 2026-10-01 --to 2026-10-31 --pages 5
idx stockbit movements --holder 1000004481       # one holder, by the HOLDER ID column
```

Reported changes in a holder's stake (schema `idx.stockbit.movements.v1`), from
two kinds of filing that Stockbit shows in one feed:

- `idx`: insider disclosures to IDX. Directors, commissioners and other
  insiders, any size, with the trade price the insider filed.
- `ksei`: KSEI's daily changes of holders at 5 % and above. No price is filed;
  Stockbit shows an estimate, which idx keeps in `estimated_price` and
  `estimated_value` (shown as `~21` in the table), never in `price` or `value`.

`shares_from`, `shares_to` and `shares_change` are exact and signed;
`pct_from`, `pct_to` and `pct_change` are percent of the company as Stockbit
rounds them (an insider's 0.0003 % stays 0.0003). A rights issue the holder did
not take up shows as `shares_change: 0` with a negative `pct_change`. `--action`
filters `buy`, `sell`, `transfer`, `cross` (crossings between accounts: repos,
pledges) or `corporate-action` (rights issues, private placements, tender
offers and the like); each row's own `action` is Stockbit's label, lowercased.
`--holder` takes the digits of a row's `holder_id`, the same on every row of
that holder.

Stockbit serves 50 rows a page; `--pages` reads up to 10, one request each, a
second apart, cached for an hour. `more` says rows remain. The feed gains rows
at the top as filings arrive, so a row repeated across pages is kept once, a
page cached before the page above it was read is read again live (rows that
moved down past the page break would otherwise be on neither page; when it
cannot be read again, a warning says rows may be missing), and a later page
that fails keeps the rows already read (with a warning).

These are filings, not signals: a share plan reads as many insiders buying at
one price on one day (BBCA's directors on 2026-03-25 at 6,982), crossings name
securities firms as holders, and rows appear days after the change. The
monthly register of holders above 1 % (`idx ownership`, no login) is the other
view.

## Broker summary

```sh
idx broker BBCA                           # every broker, once logged in
idx broker BBCA --source stockbit -n 0    # Stockbit or nothing, every row in the table
idx broker BBRI --from 2026-11-02 --to 2026-11-06 --investor foreign
idx broker BBCA --source ipot             # IPOT's top 10, no login
```

`idx broker` reads Stockbit's broker summary by itself once a working session is
stored (what `idx stockbit status` calls logged in): `--source auto`, the
default, takes Stockbit then and IPOT otherwise, and `--source stockbit` fails,
naming `idx stockbit login`, when no session works. The schema stays IPOT's,
`idx.broker.v1`:

- idx asks for gross figures: each broker's own buying on the buy side and its
  own selling on the sell side, as IPOT publishes them. Stockbit's net mode
  lists each broker once, on the side where it was net; idx does not use it
  and refuses a reply in those figures.
- Every broker is listed. BBCA had 59 buyers and 67 sellers on 2026-11-06, and
  92 a side over the year to that day. `buyer_count` and `seller_count` are
  Stockbit's counts; `buyers_complete` and `sellers_complete` say whether each
  list holds them all. A cut list keeps its totals `null`, and the table says
  `Top buyers (5 of 59)`.
- Stockbit ranks a side by value; idx orders it by lots, as IPOT ranks, so
  `rank` means the same. Ordered so, Stockbit's first ten were IPOT's top 10
  for 2026-11-06 and for 2026-11-02 to 2026-11-06: the same brokers in the same
  order, with the same lots and values at IPOT's precision. Classes are
  Stockbit's own labels (`Asing`, `Lokal`, `Pemerintah`), which can differ from
  IPOT's: HD is foreign on Stockbit and local on IPOT.
- Figures are exact (`rounded: false`), and odd lots count as fractions of a
  lot (YU bought 108,898.5 lots that day). Totals are sums over the full lists:
  both sides came to 1,035,652,066,452 IDR. Stockbit's own totals are the buy
  side's at about eight significant digits and are not used.
- The foreign net comes from a second read, of foreign investors over the same
  range and board: their buying less their selling, 218,988,301,100 IDR for
  BBCA on 2026-11-06 (IPOT's `F. NVal`: 219.0 B). With `--board regular` it is
  the regular board's, which IPOT does not publish. A day without trades needs
  no second read; when the second read fails the net is `null` with a warning.
- A range is one read, aggregated by Stockbit, so no session is rebuilt one by
  one as IPOT's foreign net over a range is.
- Domestic investors and the cash and negotiated boards have not been recorded
  yet, so idx does not ask Stockbit for them: `auto` takes IPOT, with a note,
  and `--source stockbit` is a usage error.
- Under `auto`, a ticker Stockbit fails for (its CDN refusing this network, a
  session Stockbit ended) is read from IPOT with a warning; after a failure no
  other read could get past, the rest of the call is read from IPOT too. Each
  summary's `source` says which answered.
- Reads are paced a second apart like the other Stockbit lists: one or two per
  ticker, plus one per weekday without trades while looking for the latest
  session. A range that ended before today is cached for a week, one that
  reaches today for five minutes.

Without a working session, or with `--source ipot`, the summary is IPOT's top 10
as before; under `auto` a note then says a login lists every broker, when a list
reached IPOT's ten rows.

## MCP

The MCP server exposes five Stockbit tools alongside the rest:

- `idx_stockbit_status`: can the stored session work now (`logged_in`;
  `expired`, `renewable` and `reason` say why not, or why idx cannot renew it),
  and when do the tokens expire (never the token values). Call it before the
  others.
- `idx_orderbook`: the live depth ladder and foreign flow for one ticker.
- `idx_stockbit_actions`: one stock's corporate actions (`symbol`), or the
  market calendar without one; `kind`, `from`, `to`, `days` as on the CLI.
- `idx_stockbit_movements`: insider and 5 % holder movements; `symbol`,
  `from`, `to`, `holder`, `action`, `source`, `pages` as on the CLI.
- `idx_stockbit_login`: starts a sign-in. `method: page` returns at once with
  a URL on 127.0.0.1 (and the `ssh -N -L` forward when the server is reached
  over SSH) for the user to open in their own browser, as with `--page`; the
  agent then polls `idx_stockbit_status`, whose `sign_in` shows `waiting`,
  `signed_in` or `failed`. `method: browser` opens a browser **on the machine
  running the server** and waits, which only suits a local stdio server the user
  sits at. A session that already works, and that idx can renew, is reported
  without opening anything (`force` signs in anyway). The tool never asks for,
  and agents should never ask the user for, a password, a cookie or a token in
  the chat. Prefer telling the user to run `idx stockbit login` in a terminal
  themselves.

`idx_broker` reads the same session: its `source` (`auto`, `ipot`,
`stockbit`) works as `--source` does on the CLI, so an agent gets every broker
once the user is logged in.

To serve realtime quotes/history over MCP, run the server with the Stockbit
provider: `idx --provider stockbit mcp`. For Stockbit candles with the free
providers for everything else, register it with
`-e IDX_HISTORY_PROVIDER=stockbit` instead.

## Where the session lives

The session file holds the rotating tokens and is written with `0600`
permissions on Unix. By default it sits beside the *default* config file (e.g.
`%APPDATA%\idx\stockbit-session.json` on Windows,
`~/.config/idx/stockbit-session.json` on Linux), also when `--config` points
elsewhere. Move it with:

- `stockbit.session` in `config.toml`;
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

Inside idx, refreshes are serialised so that one session does have one
refresher: a process keeps one session store per session file (the CLI
handler, the `--provider stockbit` provider, every MCP tool call and every
MCP HTTP session share it; an MCP server takes it once when it starts and
holds it until it stops), and a refresh also takes a lock file beside the
session (`stockbit-session.lock`) so a CLI command and a running MCP server do
not refresh at the same moment. Inside both locks the file is read again: a
pair another holder just stored is used, not refreshed a second time. A lock
older than a minute counts as left behind by a crashed process and is taken
over. A refresh token Stockbit refuses is not presented again by the same
process; `idx stockbit login` is the way back.

## What `idx` talks to

All on `https://exodus.stockbit.com`, with `Authorization: Bearer <access JWT>`
and browser-shaped `origin`/`referer` headers:

| endpoint | used for |
| --- | --- |
| `GET /company-price-feed/v2/orderbook/companies/{SYM}` | quote (OHLC, volume, value, foreign flow only while `has_foreign_bs` is true) and the full depth ladder |
| `GET /charts/{SYM}/daily?timeframe=..&chart_type=PRICE_CHART_TYPE_CANDLE` | OHLCV candles |
| `GET /corpaction/{type}[?symbol={SYM}]` | corporate actions, one kind per request: `dividend`, `stock_dividend`, `bonus`, `rightissue`, `warrant`, `stocksplit`, `reversesplit`, `rups`, `pubex`, `ipo`, `tenderoffer` |
| `GET /insider/company/majorholder?page=N[&symbol=][&date_start=][&date_end=][&insider=][&action_type=][&source_type=]` | insider and 5 % holder movements, 50 rows a page |
| `GET /marketdetectors/{SYM}?transaction_type=TRANSACTION_TYPE_GROSS&market_board=MARKET_BOARD_ALL\|_REGULER&investor_type=INVESTOR_TYPE_ALL\|_FOREIGN&from=..&to=..&limit=1000` | the broker summary: every broker that bought and sold over the range (`idx broker`) |
| `GET /company-price-feed/market-time/session` | checks a session before `idx stockbit login` stores it (never from the cache) |
| `POST /login/refresh` | rotate the session (bearer = refresh token, empty body) |

The same API places orders and moves cash, so idx keeps an allowlist of the
read-only paths above and refuses any other URL before a token is read. A 401,
or a 403 with a JSON body, means Stockbit refused the access token: idx
refreshes once and retries once. A 403 with an HTML body is Stockbit's CDN
(CloudFront) refusing this client or network; a refresh cannot change that,
so idx reports it and does not retry. The same goes for `POST /login/refresh`:
only a 401, or a 403 with a JSON body, counts as a refused refresh token there;
an HTML 403 is reported as the block, and the token stays usable once it lifts.

Indices use Stockbit's own code (`IHSG`, `LQ45`), mapped from idx's caret form
(`^JKSE`, `^JKLQ45`). The token is never written to a log, an error message or a
cache key.
