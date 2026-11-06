// node --test web/test/*.mjs. src/ownership.js is the Worker's port of
// src/ownership/analytics.rs: these tests check the numbers the Rust tests
// check (src/ownership/db/analytics.rs) on the same two months, exported as
// mirror parts (tests/fixtures/ownership/README.md).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import * as own from "../src/ownership.js";
import { route } from "../src/api.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const part = (d) => JSON.parse(readFileSync(join(root, "tests", "fixtures", "ownership", `mirror_part_${d}_analytics_constructed.json`), "utf8"));
const AUG = part("20260831"), SEP = part("20260930");

test("entityKey matches ownership::normalize::entity_key", () => {
  const k = own.entityKey("PT ALAMTRI RESOURCES INDONESIA TBK");
  assert.equal(k, "ALAMTRI RESOURCES INDONESIA");
  assert.equal(own.entityKey("Alamtri  Resources Indonesia, Tbk."), k);
  assert.equal(own.entityKey("ALAMTRI RESOURCES INDONESIA TBK PT"), k);
  assert.equal(own.entityKey("PT"), "PT");
  assert.equal(own.entityKey("PT Tbk"), "PT TBK");
  assert.equal(own.entityKey("Société Générale, S.A."), "SOCIÉTÉ GÉNÉRALE S A");
  assert.equal(own.entityKey("IR. T. PERMADI RACHMAT"), "IR T PERMADI RACHMAT");
  // Every key in the export is already normalised.
  for (const r of [...AUG, ...SEP]) assert.equal(own.entityKey(r.holder_raw) === r.entity_key || own.entityKey(r.entity_key) === r.entity_key, true, r.entity_key);
});

test("search on one month: kinds, order and the 2-character rule", () => {
  const known = own.knownFromMonth(SEP, "2026-09-30");
  const permadi = own.search(known, SEP, "permadi");
  assert.deepEqual(permadi.map((m) => [m.entity_key, m.matched, m.issuers, m.total_pct]), [["T PERMADI RACHMAT", "contains", 3, 13.38]]);
  assert.deepEqual(Object.keys(permadi[0]), ["entity_key", "holder", "matched", "spellings", "issuers", "total_pct", "investor_type", "local_foreign", "last_seen"]);
  assert.equal(own.search(known, SEP, "salim anthoni")[0].matched, "words");
  assert.equal(own.search(known, SEP, "Anthoni  Salim")[0].matched, "exact");
  assert.equal(own.search(known, SEP, "salim")[0].entity_key, "ANTHONI SALIM");
  // Holders of August searched with September's positions: absent ones last.
  const both = [...own.knownFromMonth(SEP, "2026-09-30"), ...own.knownFromMonth(AUG, "2026-08-31").filter((k) => !SEP.some((r) => r.entity_key === k.entity_key))];
  const uob = own.search(both, SEP, "uob kay hian");
  assert.deepEqual(uob.map((m) => [m.entity_key, m.issuers]), [["UOB KAY HIAN PRIVATE LIMITED", 3], ["UOB KAY HIAN PTE LTD", 0]]);
  for (const bad of ["x", "%%", " ! "]) assert.throws(() => own.search(known, SEP, bad), own.UsageError);
});

test("search reads the spellings normalised once, with the same answers", () => {
  // A spelling whose key differs from its holder's (a merged name).
  const rows = [
    { symbol: "WGSH", entity_key: "WALDEN GLOBAL SERVICES", holder: "WALDEN GLOBAL SERVICES", holder_raw: "WALDEN GLOBAL SERVICES", shares_total: 1, pct: 33.14 },
    { symbol: "WGSH", entity_key: "WALDEN GLOBAL SERVICES", holder: "WALDEN GLOBAL SERVICES", holder_raw: "Walden Global Services Pte. Ltd.", shares_total: 1, pct: 1.86 },
  ];
  const known = own.knownFromMonth(rows, "2026-09-30");
  assert.deepEqual(known[0].norm, ["WALDEN GLOBAL SERVICES", "WALDEN GLOBAL SERVICES PTE LTD"]);
  const kinds = (k, q) => own.search(k, rows, q).map((m) => m.matched);
  const bare = known.map(({ norm, ...k }) => k);
  for (const [q, kind] of [["walden global services pte. ltd.", "spelling"], ["Walden Global Services Pte Ltd", "contains"], ["pte ltd", "contains"], ["ltd wald", "words"], ["services walden", "words"], ["walden", "prefix"], ["nobody", null]]) {
    assert.deepEqual(kinds(known, q), kind ? [kind] : [], q);
    assert.deepEqual(own.search(bare, rows, q), own.search(known, rows, q), q);
  }
  // Every holder of a real month: precomputed and computed spellings agree.
  const sep = own.knownFromMonth(SEP, "2026-09-30");
  for (const q of ["permadi", "salim anthoni", "uob kay hian", "persada", "tbk", "dwi muria"]) {
    assert.deepEqual(own.search(sep.map(({ norm, ...k }) => k), SEP, q), own.search(sep, SEP, q), q);
  }
});

test("cross-holders like the CLI", () => {
  const all = own.crossHolders(SEP, { min_issuers: 2 });
  assert.equal(all.length, 8);
  assert.deepEqual(all.slice(0, 3).map((h) => [h.entity_key, h.issuers, h.total_pct]), [
    ["DRS SURONO SUBEKTI", 3, 21.32], ["T PERMADI RACHMAT", 3, 13.38], ["UOB KAY HIAN PRIVATE LIMITED", 3, 8.77],
  ]);
  assert.deepEqual(all[0].positions.map((p) => [p.symbol, p.pct]), [["OBAT", 14.57], ["CASS", 5.71], ["ADES", 1.04]]);
  assert.deepEqual(Object.keys(all[0]), ["holder", "entity_key", "investor_type", "local_foreign", "issuers", "total_pct", "max_pct", "positions"]);
  assert.deepEqual(own.crossHolders(SEP, { min_issuers: 2, investor_type: " individual " }).map((h) => h.entity_key), ["DRS SURONO SUBEKTI", "T PERMADI RACHMAT", "LONG CAI"]);
  assert.deepEqual(own.crossHolders(SEP, { min_issuers: 2, origin: "foreign" }).map((h) => h.entity_key), ["UOB KAY HIAN PRIVATE LIMITED", "LONG CAI"]);
  assert.equal(own.crossHolders(SEP, { min_issuers: 3 }).length, 3);
});

test("concentration like the CLI", () => {
  const rows = own.rankConcentration(own.concentration(SEP, false), "hhi", false);
  assert.equal(rows.length, 16);
  assert.deepEqual([rows[0].symbol, rows[0].hhi, rows[0].top1_pct, rows[0].top3_pct, rows[0].undisclosed_pct, rows[0].holders_listed], ["ADES", 8345.9041, 91.35, 92.39, 7.61, 2]);
  assert.deepEqual([rows[15].symbol, rows[15].hhi], ["BOSS", 339.9018]);
  const bbca = rows.find((c) => c.symbol === "BBCA");
  assert.deepEqual([bbca.hhi, bbca.top1_pct, bbca.top3_pct, bbca.listed_pct, bbca.undisclosed_pct, bbca.holders_listed, bbca.top_holder, bbca.controller_pct],
    [3021.9114, 54.94, 57.16, 58.18, 41.82, 4, "PT DWIMURIA INVESTAMA ANDALAN", null]);
  assert.deepEqual(Object.keys(bbca), ["symbol", "issuer_name", "holders_listed", "top_holder", "top1_pct", "top3_pct", "listed_pct", "undisclosed_pct", "hhi", "controller_pct"]);
  const asc = own.rankConcentration(own.concentration(SEP, false), "top1", true);
  assert.equal(asc[0].symbol, "BOSS");
  // The stakes [40, 25, 10, 5]: HHI 2350, undisclosed 20.
  const s = (pct, i) => ({ symbol: "AAAA", entity_key: `H${i}`, holder: `H${i}`, shares_total: 1, pct });
  const c = own.concentration([40, 25, 10, 5].map(s), true)[0];
  assert.deepEqual([c.hhi, c.top1_pct, c.top3_pct, c.listed_pct, c.undisclosed_pct, c.controller_pct], [2350, 40, 75, 80, 20, 0]);
});

test("graph walk like the CLI", () => {
  const size = (root, depth, min = 0, max = 100) => { const w = own.walk(SEP, root, depth, min, max); return [w.issuers.length + w.holders.length, w.links.length, w.truncated]; };
  assert.deepEqual(size({ issuer: "AADI" }, 2), [13, 13, false]);
  assert.deepEqual(size({ issuer: "AADI" }, 3), [25, 27, false]);
  assert.deepEqual(size({ issuer: "AADI" }, 2, 5), [4, 3, false]);
  assert.deepEqual(size({ issuer: "BBCA" }, 1), [5, 4, false]);
  const permadi = own.walk(SEP, { holder: "T PERMADI RACHMAT" }, 2);
  assert.deepEqual([permadi.holders.length, permadi.issuers.length, permadi.links.length], [22, 3, 27]);
  const cap = own.walk(SEP, { issuer: "AADI" }, 2, 0, 5);
  assert.equal(cap.truncated, true);
  assert.deepEqual(cap.holders.map((h) => h.entity_key), ["ADARO STRATEGIC INVESTMENTS", "ALAMTRI RESOURCES INDONESIA", "GARIBALDI THOHIR", "SARATOGA INVESTAMA SEDAYA"]);
  const w = own.walk(SEP, { issuer: "AADI" }, 2);
  assert.deepEqual(Object.keys(w.links[0]), ["from", "to", "holder", "symbol", "shares_total", "pct", "hops"]);
  assert.deepEqual([w.links[0].from, w.links[0].to, w.links[0].pct], ["holder:ADARO STRATEGIC INVESTMENTS", "ticker:AADI", 41.1]);
  const amb = own.rootFor(SEP, "thohir", false, "THOHIR");
  assert.deepEqual(amb.root, { holder: "GARIBALDI THOHIR" });
  assert.match(amb.warning, /^graph starts at GARIBALDI THOHIR; 1 other holder matches "thohir"/);
  assert.equal(own.rootFor(SEP, "ZZZZ", false, "ZZZZ"), null);
  // A root whose stakes are all below minPct keeps its names, type and flag.
  const tall = own.walk(SEP, { issuer: "AADI" }, 1, 99);
  assert.deepEqual([tall.issuers[0].issuer_name, tall.holders.length, tall.links.length], ["ADARO ANDALAN INDONESIA Tbk", 0, 0]);
  const dw = own.walk(SEP, { holder: "DWIMURIA INVESTAMA ANDALAN" }, 2, 60).holders[0];
  assert.deepEqual([dw.holder, dw.investor_type, dw.local_foreign], ["PT DWIMURIA INVESTAMA ANDALAN", "CORPORATE", "L"]);
});

/** A D1 stand-in that answers by the first table named in the SQL. */
function fakeD1(tables) {
  const stmt = (sql) => {
    const t = Object.keys(tables).find((name) => sql.includes(name));
    const rows = () => tables[t] || [];
    const api = { bind: () => api, all: async () => ({ results: rows() }), first: async () => rows()[0] ?? null, run: async () => ({}) };
    return api;
  };
  return { prepare: stmt };
}
const env = { DB: fakeD1({ own_holdings: SEP.map((r) => ({ as_of: "2026-09-30", ...r })), own_releases: [{ as_of: "2026-09-30", lines: 142, other_date_rows: 0, controller_column: 0 }] }) };
const call = (path) => route(env, new URL(`https://x/api/ownership/${path}`));

test("the four endpoints return the CLI envelopes", async () => {
  const s = await call("search?q=permadi");
  assert.equal(s.schema, "idx.ownership.search.v1");
  assert.deepEqual(Object.keys(s.data), ["query", "as_of", "total_matches", "matches"]);
  assert.equal(s.data.matches[0].last_seen, "2026-09-30");
  assert.ok(s.warnings.some((w) => w.includes("2026-09-30 register only")));
  const none = await call("search?q=nobody%20at%20all");
  assert.deepEqual([none.data.total_matches, none.warnings.at(-1)], [0, 'no holder matching "nobody at all"']);

  const x = await call("cross-holders?origin=foreign&limit=1");
  assert.equal(x.schema, "idx.ownership.cross-holders.v1");
  assert.deepEqual(Object.keys(x.data), ["as_of", "min_issuers", "investor_type", "origin", "total_matches", "holders"]);
  assert.deepEqual([x.data.total_matches, x.data.holders.length, x.data.origin], [2, 1, "foreign"]);

  const c = await call("concentration?symbols=tcid,ZZZZ,bbca");
  assert.equal(c.schema, "idx.ownership.concentration.v1");
  assert.deepEqual(Object.keys(c.data), ["as_of", "sort", "ascending", "total_matches", "controller_reported", "rows"]);
  assert.deepEqual(c.data.rows.map((r) => r.symbol), ["TCID", "BBCA"]);
  assert.ok(c.warnings.includes("no holders above 1% recorded for ZZZZ as of 2026-09-30"));
  const top = await call("concentration?by=top3&asc=1&limit=2");
  assert.deepEqual([top.data.sort, top.data.ascending, top.data.total_matches, top.data.rows.length], ["top3", true, 16, 2]);

  const g = await call("graph?root=AADI");
  assert.equal(g.schema, "idx.ownership.graph.v1");
  assert.deepEqual(Object.keys(g.data), ["as_of", "root", "depth", "min_pct", "max_nodes", "truncated", "issuers", "holders", "links"]);
  assert.deepEqual([g.data.root, g.data.links.length], ["ticker:AADI", 13]);
  const capped = await call("graph?root=AADI&max_nodes=5");
  assert.ok(capped.warnings.includes("stopped at 5 nodes; raise --max-nodes or --min-pct"));
});

test("the endpoints check their arguments like the CLI", async () => {
  const fails = async (path, code) => {
    await assert.rejects(call(path), (e) => { assert.equal(e.code, code, `${path}: ${e.message}`); return true; });
  };
  await fails("search?q=x", "usage");
  await fails("cross-holders?origin=mars", "usage");
  await fails("cross-holders?min_issuers=0", "usage");
  await fails("concentration?by=nope", "usage");
  await fails("concentration?symbols=ZZZZ", "not_found");
  await fails("graph?root=AADI&depth=0", "usage");
  await fails("graph?root=AADI&depth=7", "usage");
  await fails("graph?root=AADI&min_pct=150", "usage");
  await fails("graph?root=AADI&max_nodes=2000", "usage");
  await fails("graph?root=ZZZZ", "not_found");
});
