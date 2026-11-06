// Port of src/ownership/analytics.rs: holder search, holders across issuers,
// ownership concentration (HHI) and the holder-issuer walk over one month of
// the KSEI register (D1 own_holdings rows, each a holder's accounts summed).
// Pure; the Worker API and test/ownership.test.mjs share it, and the tests
// check the same numbers as the Rust tests on the same fixtures.

export const r4 = (v) => { const r = Math.round(v * 1e4) / 1e4; return r === 0 ? 0 : r; };
const EPS = 1e-9;
const byPctDesc = (a, b) => b - a;
const cmp = (a, b) => (a < b ? -1 : a > b ? 1 : 0);

/** ownership::normalize::entity_key: upper-case letters and digits, legal forms PT/TBK/PERSERO dropped. */
export function entityKey(raw) {
  const toks = String(raw ?? "").toUpperCase().replace(/[^\p{Alphabetic}\p{N}]+/gu, " ").split(" ").filter(Boolean);
  const kept = toks.filter((t) => !["PT", "TBK", "PERSERO"].includes(t));
  return (kept.length ? kept : toks).join(" ");
}

const collapseUpper = (s) => String(s ?? "").split(/\s+/).filter(Boolean).join(" ").toUpperCase();
const alnum = (s) => [...s].filter((c) => /[\p{Alphabetic}\p{N}]/u.test(c)).length;

export class UsageError extends Error {}

/** The normalised query of a search; needs 2 letters or digits. */
export function searchKey(query) {
  const key = entityKey(query);
  if (alnum(key) < 2) throw new UsageError("search needs at least 2 letters or digits");
  return key;
}

export const KINDS = ["exact", "spelling", "prefix", "contains", "words"];

/** How a key and its spellings match the normalised query `qk` and the collapsed query `qs`. */
export function matchKind(qk, qs, key, spellings) {
  if (key === qk) return "exact";
  if (spellings.some((s) => collapseUpper(s) === qs)) return "spelling";
  if (key.startsWith(qk)) return "prefix";
  const norm = spellings.map(entityKey);
  if (key.includes(qk) || norm.some((n) => n.includes(qk))) return "contains";
  const words = new Set([...key.split(" "), ...norm.flatMap((n) => n.split(" "))]);
  if (qk.split(" ").every((w) => [...words].some((x) => x.startsWith(w)))) return "words";
  return null;
}

function presence(stakes) {
  const out = new Map();
  for (const s of stakes) {
    let p = out.get(s.entity_key);
    if (!p) { p = { symbols: new Set(), total: 0, largest: s }; out.set(s.entity_key, p); }
    p.symbols.add(s.symbol);
    p.total += s.pct;
    if (s.pct > p.largest.pct + EPS || (Math.abs(s.pct - p.largest.pct) <= EPS && s.symbol < p.largest.symbol)) p.largest = s;
  }
  return out;
}

/**
 * Mirrors analytics::search. `known`: [{entity_key, holder, spellings, last_seen}];
 * `present`: one month's stakes. Throws UsageError for a query without 2 letters or digits.
 */
export function search(known, present, query) {
  const qk = searchKey(query);
  const qs = collapseUpper(query);
  const here = presence(present);
  const out = [];
  for (const k of known) {
    const kind = matchKind(qk, qs, k.entity_key, k.spellings);
    if (!kind) continue;
    const p = here.get(k.entity_key);
    out.push({
      entity_key: k.entity_key, holder: k.holder, matched: kind, spellings: [...new Set(k.spellings)].sort(),
      issuers: p ? p.symbols.size : 0, total_pct: p ? r4(p.total) : 0,
      investor_type: p ? p.largest.investor_type ?? null : null, local_foreign: p ? p.largest.local_foreign ?? null : null,
      last_seen: k.last_seen,
    });
  }
  const weak = (m) => (m.matched === "exact" || m.matched === "spelling" ? 0 : 1);
  out.sort((a, b) => weak(a) - weak(b) || (a.issuers === 0) - (b.issuers === 0) || KINDS.indexOf(a.matched) - KINDS.indexOf(b.matched)
    || b.issuers - a.issuers || a.entity_key.length - b.entity_key.length || cmp(a.entity_key, b.entity_key));
  return out;
}

/** The holders of one month as `search` candidates: the web has that month's spellings only. */
export function knownFromMonth(rows, asOf) {
  const m = new Map();
  for (const r of rows) {
    let k = m.get(r.entity_key);
    if (!k) { k = { entity_key: r.entity_key, holder: r.holder, spellings: [], last_seen: asOf }; m.set(r.entity_key, k); }
    if (r.holder_raw && !k.spellings.includes(r.holder_raw)) k.spellings.push(r.holder_raw);
  }
  return [...m.values()];
}

export const typeMatches = (investorType, wanted) => investorType != null && collapseUpper(investorType).includes(collapseUpper(wanted));

/** Mirrors analytics::cross_holders. opts: {min_issuers, investor_type, origin: "foreign"|"local"|null}. */
export function crossHolders(stakes, { min_issuers = 2, investor_type = null, origin = null } = {}) {
  const flag = origin === "foreign" ? "A" : origin === "local" ? "L" : null;
  const groups = new Map();
  for (const s of stakes) {
    if (investor_type && !typeMatches(s.investor_type, investor_type)) continue;
    if (flag && s.local_foreign !== flag) continue;
    if (!groups.has(s.entity_key)) groups.set(s.entity_key, []);
    groups.get(s.entity_key).push(s);
  }
  const out = [];
  for (const [key, list] of groups) {
    list.sort((a, b) => byPctDesc(a.pct, b.pct) || cmp(a.symbol, b.symbol));
    const issuers = new Set(list.map((s) => s.symbol)).size;
    if (issuers < min_issuers) continue;
    const largest = list[0];
    out.push({
      holder: largest.holder, entity_key: key, investor_type: largest.investor_type ?? null, local_foreign: largest.local_foreign ?? null,
      issuers, total_pct: r4(list.reduce((t, s) => t + s.pct, 0)), max_pct: r4(largest.pct),
      positions: list.map((s) => ({ symbol: s.symbol, issuer_name: s.issuer_name ?? null, shares_total: s.shares_total, pct: r4(s.pct) })),
    });
  }
  out.sort((a, b) => b.issuers - a.issuers || byPctDesc(a.total_pct, b.total_pct) || cmp(a.holder, b.holder) || cmp(a.entity_key, b.entity_key));
  return out;
}

/** Mirrors analytics::concentration: one row per issuer, symbol order. */
export function concentration(stakes, controllerReported) {
  const bySymbol = new Map();
  for (const s of stakes) {
    if (!bySymbol.has(s.symbol)) bySymbol.set(s.symbol, []);
    bySymbol.get(s.symbol).push(s);
  }
  return [...bySymbol.keys()].sort().map((symbol) => {
    const list = bySymbol.get(symbol).sort((a, b) => byPctDesc(a.pct, b.pct) || cmp(a.holder, b.holder));
    const listed = list.reduce((t, s) => t + s.pct, 0);
    return {
      symbol, issuer_name: list.find((s) => s.issuer_name)?.issuer_name ?? null, holders_listed: list.length, top_holder: list[0].holder,
      top1_pct: r4(list[0].pct), top3_pct: r4(list.slice(0, 3).reduce((t, s) => t + s.pct, 0)), listed_pct: r4(listed),
      undisclosed_pct: r4(Math.max(0, 100 - listed)), hhi: r4(list.reduce((t, s) => t + s.pct * s.pct, 0)),
      controller_pct: controllerReported ? r4(list.filter((s) => s.controller_or_affiliate).reduce((t, s) => t + s.pct, 0)) : null,
    };
  });
}

export const SORTS = ["hhi", "top1", "top3"];

/** Mirrors analytics::rank_concentration. */
export function rankConcentration(rows, by = "hhi", ascending = false) {
  const value = (c) => (by === "top1" ? c.top1_pct : by === "top3" ? c.top3_pct : c.hhi);
  return rows.sort((a, b) => (ascending ? value(a) - value(b) : value(b) - value(a)) || cmp(a.symbol, b.symbol));
}

export const issuerId = (s) => `ticker:${s}`;
export const holderId = (k) => `holder:${k}`;

/** Mirrors analytics::walk. root: {issuer: "AADI"} or {holder: "<entity_key>"}. */
export function walk(stakes, root, depth = 2, minPct = 0, maxNodes = 100) {
  const kept = stakes.filter((s) => s.pct + EPS >= minPct);
  const byIssuer = new Map(), byHolder = new Map();
  for (const s of kept) {
    if (!byIssuer.has(s.symbol)) byIssuer.set(s.symbol, []);
    byIssuer.get(s.symbol).push(s);
    if (!byHolder.has(s.entity_key)) byHolder.set(s.entity_key, []);
    byHolder.get(s.entity_key).push(s);
  }
  for (const l of byIssuer.values()) l.sort((a, b) => byPctDesc(a.pct, b.pct) || cmp(a.entity_key, b.entity_key));
  for (const l of byHolder.values()) l.sort((a, b) => byPctDesc(a.pct, b.pct) || cmp(a.symbol, b.symbol));
  const start = root.issuer != null ? issuerId(root.issuer) : holderId(root.holder);
  const hops = new Map([[start, 0]]);
  const order = [start];
  let frontier = [start], truncated = false;
  walking: for (let hop = 1; hop <= depth; hop++) {
    const next = [];
    for (const node of frontier) {
      const neighbours = node.startsWith("ticker:")
        ? (byIssuer.get(node.slice(7)) || []).map((x) => holderId(x.entity_key))
        : (byHolder.get(node.slice(7)) || []).map((x) => issuerId(x.symbol));
      for (const n of neighbours) {
        if (hops.has(n)) continue;
        if (hops.size >= maxNodes) { truncated = true; break walking; }
        hops.set(n, hop); order.push(n); next.push(n);
      }
    }
    if (!next.length) break;
    frontier = next;
  }
  const issuers = [], holders = [];
  for (const id of order) {
    const h = hops.get(id);
    if (id.startsWith("ticker:")) {
      const symbol = id.slice(7);
      issuers.push({ id, symbol, issuer_name: (byIssuer.get(symbol) || []).find((x) => x.issuer_name)?.issuer_name ?? null, hops: h });
    } else {
      const key = id.slice(7);
      const largest = (byHolder.get(key) || [])[0];
      holders.push({ id, entity_key: key, holder: largest ? largest.holder : key, investor_type: largest?.investor_type ?? null, local_foreign: largest?.local_foreign ?? null, hops: h });
    }
  }
  const links = [];
  for (const s of kept) {
    const a = hops.get(holderId(s.entity_key)), b = hops.get(issuerId(s.symbol));
    if (a == null || b == null) continue;
    links.push({ from: holderId(s.entity_key), to: issuerId(s.symbol), holder: s.holder, symbol: s.symbol, shares_total: s.shares_total, pct: r4(s.pct), hops: Math.max(a, b) });
  }
  links.sort((a, b) => a.hops - b.hops || cmp(a.symbol, b.symbol) || byPctDesc(a.pct, b.pct) || cmp(a.from, b.from));
  return { truncated, issuers, holders, links };
}

/**
 * Mirrors OwnershipDb::graph's root choice: a ticker with holders (unless
 * forceHolder), else the holder whose key the root normalises to, else the
 * best search match among the month's holders. Returns {root, warning} or null.
 */
export function rootFor(stakes, rootRaw, forceHolder, symbol) {
  if (!forceHolder && symbol && stakes.some((s) => s.symbol === symbol)) return { root: { issuer: symbol }, warning: null };
  const key = entityKey(rootRaw);
  if (stakes.some((s) => s.entity_key === key)) return { root: { holder: key }, warning: null };
  try { searchKey(rootRaw); } catch { return null; }
  const found = search(knownFromMonth(stakes, null), stakes, rootRaw);
  if (!found.length) return null;
  const n = found.length - 1;
  const warning = n ? `graph starts at ${found[0].holder}; ${n} other holder${n === 1 ? " matches" : "s match"} "${String(rootRaw).trim()}" (see idx ownership search)` : null;
  return { root: { holder: found[0].entity_key }, warning };
}
