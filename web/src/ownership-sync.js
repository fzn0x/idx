// Loads the KSEI ">1 % holders" register into D1 from the GitHub mirror that
// .github/workflows/ownership.yml publishes (release tag `ownership-data`).
//
// Runs from the minute cron. There is no write endpoint: the Worker only
// reads index.json and the JSON parts from the repository's release, checks
// each part's SHA-256 against the index, and inserts it with one statement
// (`json_each` over the part text), one part per tick. A release becomes
// visible only after every part is in and the row count matches the index.
//
// Limits it respects: one D1 statement per part (D1 query caps), a daily
// rows-written budget (D1 free tier allows 100k per UTC day; every holding is
// three writes with its two indexes, and deleting one costs the same), and the
// newest OWN_KEEP_RELEASES months. Replacing a month D1 already holds deletes
// the old copy first, which hides that month until the new copy is in, so a
// reload starts only when the delete and the whole load fit in what is left
// of today's budget.
//
// A part row is one holder's position in one issuer, the holder's register
// lines already summed by idx (`accounts`). A month reloads when the IDX file
// changes (sha256) or when an index written by idx 0.8.2 or later lists other
// parts for it (`parts_key`), for example after the mirror is rebuilt with a
// newer parser.

import { fetchWithin } from "./net.js";

const DEFAULT_MIRROR = "https://github.com/fzn0x/idx/releases/download/ownership-data";
// Parts are a few MB at most; a stalled mirror gives up and the next tick retries.
const FETCH_TIMEOUT_MS = 30_000;
const SCHEMA = "idx.ownership.mirror.v1";
const CHECK_EVERY_SECS = 30 * 60;
const WRITES_PER_ROW = 3;
const UA = "idx-web (+https://ihsg.midoriresearch.com)";

export const mirrorBase = (env) => (env.OWN_MIRROR_URL || DEFAULT_MIRROR).replace(/\/+$/, "");
const keepReleases = (env) => Math.max(2, Number(env.OWN_KEEP_RELEASES || 6));
const dailyBudget = (env) => Math.max(5000, Number(env.OWN_DAILY_WRITE_BUDGET || 60000));
// D1's daily limits reset at 00:00 UTC, so the budget counts UTC days.
const utcDay = () => new Date().toISOString().slice(0, 10);

/**
 * Whether a step that writes `writes` rows may start when `used` of today's
 * `limit` is spent. A step bigger than a whole day's budget (a very low
 * OWN_DAILY_WRITE_BUDGET) starts on a fresh day instead of never. Pure, tested.
 */
export const fitsBudget = (used, writes, limit) => used + writes <= limit || (used === 0 && writes > limit);

async function kvGet(env, k) {
  const row = await env.DB.prepare("SELECT v FROM kv WHERE k = ?1").bind(k).first();
  return row ? JSON.parse(row.v) : null;
}
async function kvPut(env, k, v) {
  await env.DB.prepare("INSERT OR REPLACE INTO kv (k, v, updated_at) VALUES (?1, ?2, ?3)").bind(k, JSON.stringify(v), Math.floor(Date.now() / 1000)).run();
}
async function kvDel(env, k) {
  await env.DB.prepare("DELETE FROM kv WHERE k = ?1").bind(k).run();
}

async function todaysBudget(env) {
  const b = await kvGet(env, "own:budget");
  return b && b.day === utcDay() ? b : { day: utcDay(), rows: 0 };
}
async function charge(env, budget, writes) {
  budget.rows += writes;
  await kvPut(env, "own:budget", budget);
}
const waiting = (env, budget, extra) => ({ waiting_for_budget: { day: budget.day, used: budget.rows, limit: dailyBudget(env) }, ...extra });
/** Rows D1 holds for a month: a loaded copy or what an aborted load left. */
async function heldRows(env, asOf) {
  return (await env.DB.prepare("SELECT COUNT(*) AS n FROM own_holdings WHERE as_of = ?1").bind(asOf).first()).n;
}

async function fetchText(url, fetchFn) {
  const r = await fetchWithin(url, { headers: { "User-Agent": UA, Accept: "application/json, */*" }, cf: { cacheTtl: 0, cacheEverything: false } }, { ms: FETCH_TIMEOUT_MS, fetchFn });
  if (!r.ok) throw new Error(`${url}: HTTP ${r.status}`);
  return r.text();
}
export async function sha256Hex(text) {
  const d = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** The part SHA-256s of one index release, in order: what D1 stores as `parts_key`. */
export const partsKey = (r) => (r.parts || []).map((p) => p.sha256).join(",");

/**
 * Whether D1's copy of a month (`h`: {sha256, parts_key}) is the one the index
 * lists. An index from idx 0.8.1 or older (no `positions`) has parts without
 * summed accounts, so only the file hash counts there; once the index carries
 * `positions`, its parts must match too. That way a month loaded before the
 * mirror was rebuilt reloads once, and only from the rebuilt parts.
 */
const isCurrent = (h, r) => !!h && h.sha256 === r.sha256 && (r.positions == null || (h.parts_key ?? null) === partsKey(r));

/** Picks the next action from the mirror index and what D1 holds (as_of -> {sha256, parts_key}). Pure, tested. */
export function plan(index, have, keep) {
  if (index?.schema !== SCHEMA) throw new Error(`unexpected mirror schema ${index?.schema}`);
  const wanted = [...index.releases].sort((a, b) => b.as_of.localeCompare(a.as_of)).slice(0, keep);
  const wantedDates = new Set(wanted.map((r) => r.as_of));
  const prune = [...have.keys()].filter((d) => !wantedDates.has(d)).sort();
  if (prune.length) return { prune: prune[0] };
  const load = wanted.find((r) => !isCurrent(have.get(r.as_of), r));
  return load ? { load } : { upToDate: wanted.length };
}

const INSERT = `INSERT OR REPLACE INTO own_holdings
  (as_of, symbol, issuer_name, entity_key, holder, holder_raw, investor_type, local_foreign,
   nationality, domicile, shares_scripless, shares_scrip, shares_total, pct, accounts, controller_or_affiliate)
  SELECT ?1, json_extract(value, '$.symbol'), json_extract(value, '$.issuer_name'), json_extract(value, '$.entity_key'),
         json_extract(value, '$.holder'), json_extract(value, '$.holder_raw'), json_extract(value, '$.investor_type'),
         json_extract(value, '$.local_foreign'), json_extract(value, '$.nationality'), json_extract(value, '$.domicile'),
         json_extract(value, '$.shares_scripless'), json_extract(value, '$.shares_scrip'),
         json_extract(value, '$.shares_total'), json_extract(value, '$.pct'),
         json_extract(value, '$.accounts'), json_extract(value, '$.controller_or_affiliate')
  FROM json_each(?2)`;

/**
 * One step of the sync. `force` checks the mirror now instead of every 30
 * minutes; `fetchFn` stands in for fetch (the cron's counted one). Returns
 * what happened, for logs and the status endpoint.
 */
export async function ownershipSync(env, { force = false, fetchFn } = {}) {
  const now = Math.floor(Date.now() / 1000);
  const base = mirrorBase(env);
  let state = await kvGet(env, "own:sync");

  if (!state) {
    const checked = (await kvGet(env, "own:checked")) || 0;
    if (!force && now - checked < CHECK_EVERY_SECS) return { idle: true, next_check_in: CHECK_EVERY_SECS - (now - checked) };
    const index = JSON.parse(await fetchText(`${base}/index.json`, fetchFn));
    await kvPut(env, "own:checked", now);
    const have = new Map((await env.DB.prepare("SELECT as_of, sha256, parts_key FROM own_releases").all()).results.map((r) => [r.as_of, { sha256: r.sha256, parts_key: r.parts_key }]));
    const next = plan(index, have, keepReleases(env));
    if (next.upToDate != null) return { up_to_date: next.upToDate };
    const budget = await todaysBudget(env);
    if (next.prune) {
      const deletes = (await heldRows(env, next.prune)) * WRITES_PER_ROW;
      if (!fitsBudget(budget.rows, deletes, dailyBudget(env))) return waiting(env, budget, { pruning: next.prune });
      await env.DB.batch([
        env.DB.prepare("DELETE FROM own_releases WHERE as_of = ?1").bind(next.prune),
        env.DB.prepare("DELETE FROM own_holdings WHERE as_of = ?1").bind(next.prune),
      ]);
      await charge(env, budget, deletes);
      await kvDel(env, "own:checked"); // look again next tick
      return { pruned: next.prune };
    }
    const r = next.load;
    // Deleting D1's copy (a reload, or leftovers of an aborted load) costs as
    // much as loading one and hides the month until the new copy is in. Start
    // only when both fit today, so the month is back within minutes instead of
    // after the next reset; until then the old copy stays visible.
    const deletes = (await heldRows(env, r.as_of)) * WRITES_PER_ROW;
    if (!fitsBudget(budget.rows, deletes + r.rows * WRITES_PER_ROW, dailyBudget(env))) return waiting(env, budget, { next_load: r.as_of });
    await env.DB.batch([
      env.DB.prepare("DELETE FROM own_releases WHERE as_of = ?1").bind(r.as_of),
      env.DB.prepare("DELETE FROM own_holdings WHERE as_of = ?1").bind(r.as_of),
    ]);
    await charge(env, budget, deletes);
    state = {
      as_of: r.as_of, sha256: r.sha256, format: r.format, source: r.source_url || `${base}/${r.file}`,
      rows: r.rows, issuers: r.issuers ?? null, entities: r.entities ?? null, parts: r.parts, next: 0, started_at: now,
      // From an index written by idx 0.8.2 or later; null for older ones.
      lines: r.lines ?? null, other_date_rows: r.other_date_rows ?? null,
      controller_column: r.controller_column == null ? null : r.controller_column ? 1 : 0, parts_key: partsKey(r),
    };
    await kvPut(env, "own:sync", state);
  }

  const budget = await todaysBudget(env);
  const part = state.parts[state.next];
  if (budget.rows + part.rows * WRITES_PER_ROW > dailyBudget(env)) return waiting(env, budget, { loading: state.as_of });
  const text = await fetchText(`${base}/${part.file}`, fetchFn);
  if ((await sha256Hex(text)) !== part.sha256) {
    await kvDel(env, "own:sync");
    await kvDel(env, "own:checked");
    throw new Error(`${part.file} no longer matches the index it was listed in; restarting from the new index`);
  }
  await env.DB.prepare(INSERT).bind(state.as_of, text).run();
  await charge(env, budget, part.rows * WRITES_PER_ROW);
  state.next += 1;
  if (state.next < state.parts.length) {
    await kvPut(env, "own:sync", state);
    return { loading: state.as_of, part: state.next, of: state.parts.length };
  }

  const count = (await env.DB.prepare("SELECT COUNT(*) AS n FROM own_holdings WHERE as_of = ?1").bind(state.as_of).first()).n;
  await kvDel(env, "own:sync");
  if (count !== state.rows) {
    await env.DB.prepare("DELETE FROM own_holdings WHERE as_of = ?1").bind(state.as_of).run();
    await charge(env, budget, count * WRITES_PER_ROW);
    throw new Error(`${state.as_of}: loaded ${count} rows but the index lists ${state.rows}; discarded`);
  }
  await env.DB.prepare(
    "INSERT OR REPLACE INTO own_releases (as_of, format, source, rows, synced_at, sha256, issuers, entities, lines, other_date_rows, controller_column, parts_key) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
  ).bind(state.as_of, state.format, state.source, count, now, state.sha256, state.issuers, state.entities, state.lines ?? null, state.other_date_rows ?? null, state.controller_column ?? null, state.parts_key ?? null).run();
  await kvDel(env, "own:checked"); // next tick looks for the next month to load
  return { committed: state.as_of, rows: count };
}

/** What the terminal shows while the register is empty or loading. */
export async function syncStatus(env) {
  const [state, checked, budget] = await Promise.all([kvGet(env, "own:sync"), kvGet(env, "own:checked"), kvGet(env, "own:budget")]);
  return {
    mirror: mirrorBase(env),
    loading: state ? { as_of: state.as_of, part: state.next, of: state.parts.length, started_at: new Date(state.started_at * 1000).toISOString() } : null,
    checked_at: checked ? new Date(checked * 1000).toISOString() : null,
    writes_today: budget && budget.day === utcDay() ? budget.rows : 0,
    daily_write_budget: dailyBudget(env),
    keep_releases: keepReleases(env),
  };
}
