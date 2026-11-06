#!/usr/bin/env bash
# Live smoke test against the real providers. Not run by `cargo test`.
# Usage: scripts/smoke.sh [path/to/idx]
# Needs only bash and grep; every check fails loudly.
set -euo pipefail

IDX="${1:-}"
if [ -z "$IDX" ]; then
  cargo build --quiet
  IDX="target/debug/idx"
  [ -x "$IDX" ] || IDX="target/debug/idx.exe"
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
export IDX_CACHE_DIR="$TMP/cache"
export IDX_CONFIG="$TMP/config.toml"
export NO_COLOR=1

pass() { printf '  \033[32mok\033[0m  %s\n' "$1"; }
fail() { printf '  \033[31mFAIL\033[0m %s\n' "$1"; exit 1; }
# expect <file> <fixed-string> <label>
expect() { grep -qF -- "$2" "$1" && pass "$3" || { echo "--- $1"; cat "$1"; fail "$3"; }; }

echo "yahoo"
"$IDX" stocks quote BBCA,BBRI,TLKM >"$TMP/q.txt" || fail "quote table exit $?"
expect "$TMP/q.txt" "BBCA" "quote table has BBCA"
expect "$TMP/q.txt" "Last trade" "quote table has footer"

"$IDX" -o json stocks quote BBCA >"$TMP/q.json" || fail "quote json"
expect "$TMP/q.json" '"schema": "idx.quotes.v1"' "quote json schema"
expect "$TMP/q.json" '"symbol": "BBCA"' "quote json symbol"
expect "$TMP/q.json" '"source": "yahoo:quote"' "quote json came from the full v7 endpoint"
grep -q '"price": null' "$TMP/q.json" && fail "quote json price is null" || pass "quote json has price"

"$IDX" -o json stocks history TLKM --period 1mo >"$TMP/h.json" || fail "history json"
expect "$TMP/h.json" '"schema": "idx.history.v1"' "history json schema"
expect "$TMP/h.json" '"adj_close"' "history has candles"
n="$(grep -c '"close":' "$TMP/h.json")"
[ "$n" -gt 10 ] && pass "history has $n candles" || fail "history has only $n candles"

"$IDX" stocks history BBRI --period 5d --interval 1h --limit 3 >"$TMP/i.txt" || fail "intraday history"
expect "$TMP/i.txt" "Time (WIB)" "intraday history uses time column"

set +e
"$IDX" stocks quote ZZZZ >/dev/null 2>"$TMP/e.txt"; code=$?
set -e
[ "$code" = 4 ] && pass "unknown symbol exits 4" || fail "unknown symbol exit $code"
expect "$TMP/e.txt" "not found" "unknown symbol message"

"$IDX" --offline -o json stocks quote BBCA >"$TMP/off.json" || fail "offline cached quote"
expect "$TMP/off.json" '"cached": true' "offline served from cache"
expect "$TMP/q.json" '"behind_session": ' "quote json flags the session"
IDX_QUOTE_TTL_SECS=0 "$IDX" --offline -o json stocks quote BBCA >"$TMP/old.json" || fail "offline stale quote"
expect "$TMP/old.json" '"stale_reason": "offline"' "offline stale copy says why"
# A refused connection (closed local port as proxy): the stale copy names the failure, never the crumb.
HTTPS_PROXY=http://127.0.0.1:9 IDX_QUOTE_TTL_SECS=0 "$IDX" -o json stocks quote BBCA >"$TMP/down.json" 2>"$TMP/down.err" || fail "stale quote after a failed fetch"
expect "$TMP/down.json" '"stale_reason": "fetch_failed"' "failed fetch says why"
expect "$TMP/down.json" 'live fetch failed (' "failed fetch names the failure"
grep -q 'crumb=' "$TMP/down.json" "$TMP/down.err" && fail "the Yahoo crumb leaked" || pass "no crumb in output or logs"

"$IDX" -o json stocks quote BBCA,ZZZZ >"$TMP/p.json" || fail "partial quote"
expect "$TMP/p.json" 'no data for ZZZZ' "partial quote lists the unknown symbol in warnings"

echo "output formats, stdin, other markets"
"$IDX" -o jsonl stocks quote BBCA BBRI >"$TMP/q.jsonl" || fail "quote jsonl"
n="$(grep -c '^{"symbol":' "$TMP/q.jsonl")"
[ "$n" = 2 ] && pass "quote jsonl has one line per ticker" || fail "quote jsonl has $n lines"
printf 'BBCA\nBBRI, TLKM\n' | "$IDX" -o jsonl stocks quote - >"$TMP/stdin.jsonl" || fail "quote from stdin"
n="$(grep -c '^{"symbol":' "$TMP/stdin.jsonl")"
[ "$n" = 3 ] && pass "quote reads three tickers from stdin" || fail "stdin quote has $n lines"
"$IDX" -o json stocks quote D05.SI BBCA >"$TMP/sg.json" || fail "quote with another market"
expect "$TMP/sg.json" '"symbol": "D05.SI"' "v7 quote keeps the D05.SI suffix"
expect "$TMP/sg.json" '"symbol": "BBCA"' "v7 quote keeps IDX tickers bare"
"$IDX" -o jsonl stocks history D05.SI --period 5d >"$TMP/sg.jsonl" || fail "history jsonl D05.SI"
n="$(grep -c '"close":' "$TMP/sg.jsonl")"
[ "$n" -ge 3 ] && pass "D05.SI history has $n candles, one per line" || fail "D05.SI history has $n lines"

echo "fundamentals"
"$IDX" stocks fundamental BBCA >"$TMP/f.txt" || fail "fundamental table"
expect "$TMP/f.txt" "VALUATION" "fundamental table has sections"
expect "$TMP/f.txt" "Heuristic" "fundamental table has the verdict line"
"$IDX" -o json stocks fundamental TLKM >"$TMP/f.json" || fail "fundamental json"
expect "$TMP/f.json" '"schema": "idx.fundamentals.v1"' "fundamental json schema"
expect "$TMP/f.json" '"yahoo:quoteSummary"' "fundamental json came from Yahoo"
grep -q '"pe_trailing": null' "$TMP/f.json" && fail "TLKM P/E is null" || pass "fundamental json has P/E"
"$IDX" -o json stocks fundamental ITMG >"$TMP/fu.json" || fail "fundamental json (USD statements)"
expect "$TMP/fu.json" '"financial_currency": "USD"' "ITMG statements in USD"
grep -q 'P/B' "$TMP/fu.json" && fail "ITMG verdict lists a P/B reason" || pass "ITMG P/B not scored"
"$IDX" -o json stocks fundamental GIAA >"$TMP/fn.json" || fail "fundamental json (negative equity)"
grep -q '999\.9999' "$TMP/fn.json" && fail "MSN placeholder leaked into GIAA" || pass "GIAA has no MSN placeholder"
grep -q 'conservative balance sheet' "$TMP/fn.json" && fail "GIAA negative equity read as conservative" || pass "GIAA not conservative"

echo "lenses"
"$IDX" stocks risk TLKM >"$TMP/lr.txt" || fail "risk table"
expect "$TMP/lr.txt" "PRICE VS ^JKSE" "risk table compares with IHSG"
"$IDX" -o json stocks risk TLKM >"$TMP/lr.json" || fail "risk json"
expect "$TMP/lr.json" '"schema": "idx.risk.v1"' "risk json schema"
expect "$TMP/lr.json" '"rule": "beta"' "risk has a beta against IHSG"
"$IDX" -o json stocks valuation ITMG >"$TMP/lv.json" || fail "valuation json"
expect "$TMP/lv.json" 'not scored: statements in USD, price in IDR' "ITMG USD multiples not scored"
"$IDX" -o json stocks growth BBCA >"$TMP/lg.json" || fail "growth json"
expect "$TMP/lg.json" '"rule": "revenue_cagr"' "growth reads the fiscal years"
"$IDX" -o json stocks fundamental GIAA --lens risk >"$TMP/ln.json" || fail "risk lens (negative equity)"
expect "$TMP/ln.json" '"rule": "negative_equity"' "GIAA negative equity adds risk"

"$IDX" -o json stocks profile BBCA >"$TMP/pr.json" || fail "profile json"
expect "$TMP/pr.json" '"sector": "Financial Services"' "profile sector"

"$IDX" -o json stocks financials BBCA --period quarterly >"$TMP/fin.json" || fail "financials json"
expect "$TMP/fin.json" '"period": "quarterly"' "financials quarterly"
expect "$TMP/fin.json" '"net_income":' "financials have net income"

"$IDX" -o json stocks earnings BBCA >"$TMP/e.json" || fail "earnings json"
expect "$TMP/e.json" '"eps_actual":' "earnings have reported EPS"
expect "$TMP/e.json" '"period": "+1y"' "earnings have next year's consensus"
"$IDX" -o json stocks earnings BBCA --annual --estimates >"$TMP/ea.json" || fail "earnings filtered"
expect "$TMP/ea.json" '"scope": {' "filtered earnings record their scope"
expect "$TMP/ea.json" '"period_type": "annual"' "filtered earnings keep the annual consensus"
"$IDX" stocks financials BBCA -s income,cash >"$TMP/fs.txt" || fail "financials two statements"
expect "$TMP/fs.txt" "CASH FLOW" "financials table has the cash flow"

"$IDX" stocks technical BBRI >"$TMP/t.txt" || fail "technical table"
expect "$TMP/t.txt" "RSI (14)" "technical table has RSI"
expect "$TMP/t.txt" "Why:" "technical table explains the signal"

"$IDX" stocks compare BBCA,BBRI,TLKM >"$TMP/c.txt" || fail "compare table"
expect "$TMP/c.txt" "Verdict" "compare table has verdict row"

echo "msn"
"$IDX" --provider msn -o json stocks quote BBCA,TLKM >"$TMP/mq.json" || fail "msn quote"
expect "$TMP/mq.json" '"provider": "msn"' "msn quote provider"
expect "$TMP/mq.json" '"source": "msn:quotes"' "msn quote source"
"$IDX" --provider msn -o json stocks fundamental BBCA >"$TMP/mf.json" || fail "msn fundamental"
expect "$TMP/mf.json" '"msn:equities"' "msn fundamental source"
expect "$TMP/mf.json" '"recommendation":' "msn fundamental has analyst block"
"$IDX" -o json ownership institutions BBCA >"$TMP/mi.json" || fail "msn institutions"
expect "$TMP/mi.json" '"schema": "idx.ownership.institutions.v1"' "institutions json schema"
expect "$TMP/mi.json" '"kind": "fund"' "institutions list funds"
expect "$TMP/mi.json" '"signal": "' "institutions carry signals"
"$IDX" --provider msn -o json stocks history BBCA --period 1mo >"$TMP/mh.json" || fail "msn history (routed)"
expect "$TMP/mh.json" '"provider": "yahoo"' "msn history comes from yahoo"

echo "screen / news / sentiment"
"$IDX" stocks screen -f "mcap>100T,pe<20" --limit 10 >"$TMP/sc.txt" || fail "screen"
expect "$TMP/sc.txt" "Mkt Cap" "screen table renders"
expect "$TMP/sc.txt" "tickers quoted" "screen reports the funnel"
"$IDX" -o json stocks screen -f "roe>15,mcap>50T" --max-fundamentals 20 --limit 5 >"$TMP/sc.json" || fail "screen with fundamentals"
expect "$TMP/sc.json" '"fundamentals_checked":' "screen json has fundamentals count"
"$IDX" stocks news --limit 5 >"$TMP/nw.txt" || fail "news"
expect "$TMP/nw.txt" "Headline" "news table renders"
"$IDX" -o json stocks sentiment BBCA >"$TMP/se.json" || fail "sentiment"
expect "$TMP/se.json" '"schema": "idx.sentiment.v1"' "sentiment json schema"
expect "$TMP/se.json" '"label":' "sentiment json has a label"
"$IDX" -o json stocks news BBCA --limit 5 >"$TMP/nwc.json" || fail "news with symbol"
expect "$TMP/nwc.json" '"company_matches":' "news json reports company matches"

echo "msn insights, screen presets and return windows (0.8.2)"
"$IDX" -o json stocks insights BBCA >"$TMP/in.json" || fail "msn insights"
expect "$TMP/in.json" '"schema": "idx.insights.v1"' "insights json schema"
expect "$TMP/in.json" '"compared_with":' "insights carry benchmarks"
"$IDX" stocks insights GOTO --all >"$TMP/in.txt" || fail "msn insights table"
expect "$TMP/in.txt" "VALUATION" "insights table groups by category"
"$IDX" -o json stocks screen --preset top-performers --limit 5 >"$TMP/sp.json" || fail "screen preset"
expect "$TMP/sp.json" '"returns_source": "msn:screener"' "screen joined MSN return windows"
expect "$TMP/sp.json" '"return_1y_pct":' "screen rows carry the 1y return"
"$IDX" stocks screen --preset 52w-high --limit 5 >"$TMP/sh.txt" || fail "screen 52w-high"
expect "$TMP/sh.txt" "52w pos" "52w-high shows the range position"

echo "dividends, returns, market, brief (0.2.0)"
"$IDX" -o json stocks dividends BBCA >"$TMP/dv.json" || fail "dividends"
expect "$TMP/dv.json" '"schema": "idx.dividends.v1"' "dividends json schema"
expect "$TMP/dv.json" '"ttm_yield_pct":' "dividends json has a TTM yield"
"$IDX" -o json stocks returns BBCA BBRI >"$TMP/rt.json" || fail "returns"
expect "$TMP/rt.json" '"excess":' "returns json has benchmark excess"
"$IDX" -o csv stocks returns BBCA --no-benchmark >"$TMP/rt.csv" || fail "returns csv"
expect "$TMP/rt.csv" "returns.1y" "csv flattens nested columns"
"$IDX" -o json market overview --top 3 >"$TMP/mk.json" || fail "market overview"
expect "$TMP/mk.json" '"advancing":' "market json has breadth"
"$IDX" -o json stocks brief BBCA >"$TMP/br.json" || fail "brief"
expect "$TMP/br.json" '"key_points":' "brief json has key points"

echo "broker (ipot)"
"$IDX" -o json broker BBCA >"$TMP/bk.json" || fail "broker json"
expect "$TMP/bk.json" '"schema": "idx.broker.v1"' "broker json schema"
expect "$TMP/bk.json" '"provider": "ipot"' "broker json came from ipot"
expect "$TMP/bk.json" '"code":' "broker json has broker rows"
expect "$TMP/bk.json" '"name": "MANDIRI SEKURITAS"' "broker codes are named from the IDX member list"
"$IDX" broker BBRI --investor foreign -n 3 >"$TMP/bk.txt" || fail "broker table"
expect "$TMP/bk.txt" "Top sellers" "broker table has both sides"
set +e
"$IDX" broker ZZZZ --date 2026-10-16 >/dev/null 2>"$TMP/bke.txt"; code=$?
set -e
[ "$code" = 4 ] && pass "unknown broker ticker exits 4" || fail "unknown broker ticker exit $code"

echo "bandar (ipot + yahoo)"
"$IDX" -o json bandar detect BBCA --daily >"$TMP/bd.json" || fail "bandar detect json"
expect "$TMP/bd.json" '"schema": "idx.bandar.v1"' "bandar json schema"
expect "$TMP/bd.json" '"signal": "' "bandar reading has a signal"
expect "$TMP/bd.json" '"bandar_average":' "bandar reading has the top buyers' average"
expect "$TMP/bd.json" '"streak":' "bandar --daily has the session series"
"$IDX" bandar detect BBRI TLKM --period 1m --top 3 >"$TMP/bd.txt" || fail "bandar detect table"
expect "$TMP/bd.txt" "Net by tier" "bandar table has its reasons"
"$IDX" -o json bandar screen --preset accumulation --scan 20 >"$TMP/bs.json" || fail "bandar screen json"
expect "$TMP/bs.json" '"schema": "idx.bandar.screen.v1"' "bandar screen json schema"
expect "$TMP/bs.json" '"scanned": 20' "bandar screen scanned the 20 most traded"
"$IDX" bandar screen --preset big-distribution --period 1d --symbols BBCA,BBRI,TLKM >"$TMP/bs.txt" || fail "bandar screen table"
expect "$TMP/bs.txt" "Bandar screen · big-distribution" "bandar screen table header"

echo "corporate actions, broker activity (ipot)"
"$IDX" -o json stocks actions ENRG --since 2025-01-01 >"$TMP/ca.json" || fail "actions json"
expect "$TMP/ca.json" '"schema": "idx.actions.v1"' "actions json schema"
expect "$TMP/ca.json" '"kind": "rights_issue"' "ENRG's rights issue is read"
expect "$TMP/ca.json" '"code": "ENRG-R"' "the listed right is read"
"$IDX" stocks actions BBCA --since 2021-01-01 >"$TMP/ca.txt" || fail "actions table"
expect "$TMP/ca.txt" "stock split" "BBCA's 2021 split is listed"
"$IDX" bandar broker YP --scan 10 >"$TMP/bb.txt" || fail "bandar broker table"
expect "$TMP/bb.txt" "Top-ten buyer of" "bandar broker table has both sides"

echo "holidays (bundled)"
"$IDX" market holidays --year 2026 >"$TMP/hd.txt" || fail "holidays"
expect "$TMP/hd.txt" "239 trading days" "2026 has IDX's 239 trading days"

echo "idx.co.id (needs a connection Cloudflare lets through)"
set +e
"$IDX" -o json market indices LQ45 >"$TMP/ix.json" 2>"$TMP/ix.err"; code=$?
set -e
if [ "$code" = 0 ]; then
  expect "$TMP/ix.json" '"schema": "idx.indices.v1"' "indices json schema"
  expect "$TMP/ix.json" '"code": "LQ45"' "indices kept LQ45"
  "$IDX" -o json stocks foreign BBCA --days 5 >"$TMP/sf.json" || fail "stocks foreign"
  expect "$TMP/sf.json" '"foreign_net_value_at_close":' "stocks foreign has net x close"
  "$IDX" market foreign --top 3 >"$TMP/mf.txt" || fail "market foreign"
  expect "$TMP/mf.txt" "Foreign net selling" "market foreign has both lists"
  "$IDX" -o json market brokers --top 3 >"$TMP/mb.json" || fail "market brokers"
  expect "$TMP/mb.json" '"value_share_pct":' "market brokers has shares"
  "$IDX" -o json market calendar --days 30 >"$TMP/mc.json" || fail "market calendar"
  expect "$TMP/mc.json" '"schema": "idx.calendar.v1"' "calendar json schema"
  "$IDX" -o json stocks list --search BBCA >"$TMP/sl.json" || fail "stocks list"
  expect "$TMP/sl.json" '"provider": "idx"' "stocks list came from idx.co.id"
  "$IDX" market members --search mandiri >"$TMP/mm.txt" || fail "market members"
  expect "$TMP/mm.txt" "Perantara Pedagang Efek" "members carry licences"
else
  echo "  skip idx.co.id checks (exit $code): $(head -c 160 "$TMP/ix.err")"
fi

echo "stockbit (realtime, needs a login)"
# Seeds an access token from the environment so this never spends (rotates) a
# refresh token. Skipped unless IDX_STOCKBIT_ACCESS_TOKEN or a session exists.
set +e
"$IDX" -o json stockbit status >"$TMP/sbs.json" 2>/dev/null
"$IDX" --provider stockbit --no-cache -o json stocks quote BBCA >"$TMP/sbq.json" 2>"$TMP/sbq.err"; code=$?
set -e
if [ "$code" = 0 ]; then
  expect "$TMP/sbq.json" '"provider": "stockbit"' "stockbit quote provider"
  expect "$TMP/sbq.json" '"source": "stockbit:orderbook"' "stockbit quote source"
  "$IDX" --provider stockbit --no-cache -o json stocks history BBCA --period 1mo >"$TMP/sbh.json" || fail "stockbit history"
  expect "$TMP/sbh.json" '"schema": "idx.history.v1"' "stockbit history schema"
  "$IDX" --no-cache -o json stockbit orderbook BBCA >"$TMP/sbo.json" || fail "stockbit orderbook"
  expect "$TMP/sbo.json" '"schema": "idx.orderbook.v1"' "stockbit orderbook schema"
  expect "$TMP/sbo.json" '"foreign_net":' "stockbit orderbook has the foreign_net field"
  # While Stockbit marks the foreign flow as not current (seen during trading)
  # it is null and a warning names the flag. When the flag is true the figures
  # are kept.
  if grep -q '"foreign_net": null' "$TMP/sbo.json"; then
    expect "$TMP/sbo.json" 'has_foreign_bs' "stockbit orderbook says why foreign flow is empty"
  else
    pass "stockbit orderbook foreign flow marked current"
  fi
else
  echo "  skip stockbit checks (no session; set IDX_STOCKBIT_ACCESS_TOKEN or run \`idx stockbit login\`)"
fi

echo "ownership (local fixture)"
export IDX_OWNERSHIP_DB="$TMP/own.sqlite"
"$IDX" ownership import --file tests/fixtures/ownership/above1_20260831_excerpt.xlsx >"$TMP/oi.txt" || fail "ownership import"
expect "$TMP/oi.txt" "Imported 2026-08-31" "ownership import summary"
"$IDX" ownership ticker AADI >"$TMP/ot.txt" || fail "ownership ticker"
expect "$TMP/ot.txt" "ADARO STRATEGIC INVESTMENTS" "ownership ticker lists the top holder"
"$IDX" ownership import --file tests/fixtures/ownership/above1_20260930_accounts_excerpt.xlsx >"$TMP/oa.txt" || fail "ownership import (accounts excerpt)"
expect "$TMP/oa.txt" "43 positions" "ownership import sums a holder's accounts"
"$IDX" ownership ticker KREN >"$TMP/ok.txt" || fail "ownership ticker KREN"
expect "$TMP/ok.txt" "13.45%" "ownership ticker shows KRESNA PRIMA INVEST's two accounts summed"
"$IDX" ownership search kresna >"$TMP/os.txt" || fail "ownership search"
expect "$TMP/os.txt" "KRESNA PRIMA INVEST" "ownership search finds a holder by name"
"$IDX" ownership concentration KREN >"$TMP/oc.txt" || fail "ownership concentration"
expect "$TMP/oc.txt" "lower bound" "ownership concentration explains the HHI"
"$IDX" ownership graph KREN --dot >"$TMP/og.txt" || fail "ownership graph --dot"
expect "$TMP/og.txt" "digraph ownership" "ownership graph prints Graphviz"
set +e
"$IDX" ownership discover >"$TMP/od.txt" 2>"$TMP/od.err"; code=$?
set -e
if [ "$code" = 0 ]; then pass "ownership discover reached idx.co.id"; else echo "  skip ownership discover (exit $code): $(head -c 160 "$TMP/od.err")"; fi

echo "ksei (composition archive)"
"$IDX" ownership discover --family composition --limit 2 >"$TMP/kd.txt" || fail "ownership discover --family composition"
expect "$TMP/kd.txt" "BalanceposEfek" "ksei.co.id lists the Kepemilikan Efek archives"
"$IDX" -o json ownership composition BBCA >"$TMP/kc.json" || fail "ownership composition"
expect "$TMP/kc.json" '"schema": "idx.ownership.composition.v1"' "composition json schema"
expect "$TMP/kc.json" '"scripless_pct":' "composition reports the scripless share"
expect "$TMP/kc.json" '"code": "MF"' "composition lists the investor types"

echo "ownership snapshot (GitHub mirror)"
"$IDX" -o json ownership status >"$TMP/os.json" || fail "ownership status"
expect "$TMP/os.json" '"schema": "idx.ownership.status.v1"' "ownership status envelope"
expect "$TMP/os.json" '"mirror": "https://github.com/' "ownership status read the mirror"
IDX_OWNERSHIP_DB="$TMP/sync/ownership.sqlite" "$IDX" -o json ownership sync >"$TMP/osy.json" || fail "ownership sync"
expect "$TMP/osy.json" '"schema": "idx.ownership.snapshot-sync.v1"' "ownership sync envelope"
grep -qE '"action": "(installed|imported)"' "$TMP/osy.json" && pass "ownership sync filled an empty database" || { cat "$TMP/osy.json"; fail "ownership sync action"; }
grep -qF '"snapshot": null' "$TMP/osy.json" && echo "  note: the mirror has no snapshot for this schema yet; sync imported the monthly files"
IDX_OWNERSHIP_DB="$TMP/sync/ownership.sqlite" "$IDX" -o json ownership sync >"$TMP/osy2.json" || fail "ownership sync (again)"
expect "$TMP/osy2.json" '"action": "up_to_date"' "a second ownership sync has nothing to do"

echo "all live checks passed"
