// node --test web/test/*.mjs (no network). The ownership sync step runs
// against SQLite (node:sqlite) with web/migrations applied, behind a small
// D1-shaped adapter; the mirror is a stubbed fetch.
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { ownershipSync, fitsBudget, partsKey, sha256Hex } from "../src/ownership-sync.js";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const migrations = join(root, "web", "migrations");
// Parts exported by scripts/ownership_mirror.py (tests/fixtures/ownership/README.md).
const partRows = (d) => JSON.parse(readFileSync(join(root, "tests", "fixtures", "ownership", `mirror_part_${d}_accounts_constructed.json`), "utf8"));

const sqlite = await import("node:sqlite").catch(() => null);
const skip = sqlite ? false : "node:sqlite is not available in this Node";

/** D1's prepare/bind/first/all/run/batch over an in-memory SQLite with the Worker's migrations. */
function d1() {
  const db = new sqlite.DatabaseSync(":memory:");
  for (const f of readdirSync(migrations).filter((f) => f.endsWith(".sql")).sort()) db.exec(readFileSync(join(migrations, f), "utf8"));
  // node:sqlite binds ?NNN only by name; rewrite them to $pNNN.
  const named = (sql) => sql.replace(/\?(\d+)/g, "$$p$1");
  const stmt = (sql, args) => {
    const params = Object.fromEntries(args.map((v, i) => [`p${i + 1}`, v === undefined ? null : v]));
    const s = () => db.prepare(named(sql));
    return {
      bind: (...a) => stmt(sql, a),
      first: async () => s().get(params) ?? null,
      all: async () => ({ results: s().all(params) }),
      run: async () => ({ meta: { changes: Number(s().run(params).changes) } }),
    };
  };
  const batch = async (list) => {
    db.exec("BEGIN");
    try {
      const out = [];
      for (const st of list) out.push(await st.run());
      db.exec("COMMIT");
      return out;
    } catch (e) {
      db.exec("ROLLBACK");
      throw e;
    }
  };
  return { db, prepare: (sql) => stmt(sql, []), batch };
}

const MIRROR = "https://mirror.test";
const today = () => new Date().toISOString().slice(0, 10);

/** Serves `files` (name -> text) as the mirror for the duration of `fn`. */
async function withMirror(files, fn) {
  const real = globalThis.fetch;
  globalThis.fetch = async (url) => {
    const name = String(url).slice(MIRROR.length + 1);
    return name in files ? new Response(files[name], { status: 200 }) : new Response("not found", { status: 404 });
  };
  try {
    return await fn();
  } finally {
    globalThis.fetch = real;
  }
}

/** A mirror release for `as_of` from rows split into two parts, as idx 0.8.2 writes it. */
async function mirrorRelease(as_of, rows) {
  const half = Math.floor(rows.length / 2);
  const texts = [JSON.stringify(rows.slice(0, half)), JSON.stringify(rows.slice(half))];
  const parts = await Promise.all(texts.map(async (t, i) => ({ file: `ksei-above1-${as_of}.part0${i + 1}.json`, sha256: await sha256Hex(t), rows: JSON.parse(t).length })));
  const release = {
    as_of, title: as_of, format: "xlsx", source_url: `https://idx.test/${as_of}.xlsx`, file: `ksei-above1-${as_of}.xlsx`, sha256: `file-${as_of}`,
    rows: rows.length, issuers: new Set(rows.map((r) => r.symbol)).size, entities: new Set(rows.map((r) => r.entity_key)).size,
    lines: rows.reduce((s, r) => s + (r.accounts || 1), 0), positions: rows.length, other_date_rows: 0, controller_column: false, parts,
  };
  return { release, files: Object.fromEntries(parts.map((p, i) => [p.file, texts[i]])) };
}

/** A month as D1 holds it after a load by an index from idx 0.8.1: no lines, no parts key, no accounts. */
function seedLegacy(DB, as_of, rows) {
  DB.db.prepare("INSERT INTO own_releases (as_of, format, source, rows, synced_at, sha256) VALUES (?, 'xlsx', 'legacy', ?, 1, ?)").run(as_of, rows.length, `file-${as_of}`);
  const ins = DB.db.prepare("INSERT INTO own_holdings (as_of, symbol, issuer_name, entity_key, holder, holder_raw, investor_type, local_foreign, shares_total, pct) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)");
  for (const r of rows) ins.run(as_of, r.symbol, r.issuer_name, r.entity_key, r.holder, r.holder_raw, r.investor_type, r.local_foreign, r.shares_total, r.pct);
}
const held = (DB, as_of) => DB.db.prepare("SELECT COUNT(*) AS n FROM own_holdings WHERE as_of = ?").get(as_of).n;
const released = (DB, as_of) => DB.db.prepare("SELECT * FROM own_releases WHERE as_of = ?").get(as_of) ?? null;
const setBudget = (DB, rows) => DB.db.prepare("INSERT OR REPLACE INTO kv (k, v, updated_at) VALUES ('own:budget', ?, 0)").run(JSON.stringify({ day: today(), rows }));
const budgetUsed = (DB) => JSON.parse(DB.db.prepare("SELECT v FROM kv WHERE k = 'own:budget'").get().v).rows;

test("fitsBudget: a step must fit what is left, unless it is bigger than a whole day", () => {
  assert.equal(fitsBudget(0, 43_000, 60_000), true);
  assert.equal(fitsBudget(17_000, 43_000, 60_000), true);
  assert.equal(fitsBudget(17_001, 43_000, 60_000), false);
  assert.equal(fitsBudget(0, 43_000, 30_000), true, "a fresh day starts a reload a low budget can never hold");
  assert.equal(fitsBudget(1, 43_000, 30_000), false);
});

test("ownership sync: a reload waits for budget without deleting, then charges the delete", { skip }, async () => {
  const DB = d1();
  const env = { DB, OWN_MIRROR_URL: MIRROR, OWN_DAILY_WRITE_BUDGET: "5000", OWN_KEEP_RELEASES: "6" };
  const sep = partRows("20260930");
  seedLegacy(DB, "2026-09-30", sep);
  const { release, files } = await mirrorRelease("2026-09-30", sep);
  const index = { schema: "idx.ownership.mirror.v1", releases: [release] };
  const reload = sep.length * 3 * 2; // delete the old copy + load the new one, three writes a row

  await withMirror({ "index.json": JSON.stringify(index), ...files }, async () => {
    // Not enough left today: nothing is deleted and the old copy stays readable.
    setBudget(DB, 5000 - reload + 1);
    const wait = await ownershipSync(env, { force: true });
    assert.equal(wait.next_load, "2026-09-30", JSON.stringify(wait));
    assert.deepEqual(wait.waiting_for_budget, { day: today(), used: 5000 - reload + 1, limit: 5000 });
    assert.equal(held(DB, "2026-09-30"), sep.length);
    assert.ok(released(DB, "2026-09-30"), "the month is still listed");

    // Enough left: the delete is charged with the first part.
    setBudget(DB, 5000 - reload);
    const first = await ownershipSync(env, { force: true });
    assert.deepEqual(first, { loading: "2026-09-30", part: 1, of: 2 });
    assert.equal(budgetUsed(DB), 5000 - reload + sep.length * 3 + release.parts[0].rows * 3);

    const done = await ownershipSync(env);
    assert.deepEqual(done, { committed: "2026-09-30", rows: sep.length });
    assert.equal(budgetUsed(DB), 5000, "delete and load together, exactly what the gate allowed");
    const rel = released(DB, "2026-09-30");
    assert.deepEqual([rel.rows, rel.lines, rel.parts_key], [sep.length, release.lines, partsKey(release)]);
    const kresna = DB.db.prepare("SELECT pct, accounts FROM own_holdings WHERE as_of = '2026-09-30' AND symbol = 'KREN' AND entity_key = 'KRESNA PRIMA INVEST'").get();
    assert.deepEqual([kresna.pct, kresna.accounts], [13.45, 2]);

    assert.deepEqual(await ownershipSync(env, { force: true }), { up_to_date: 1 });
  });
});

test("ownership sync: pruning a month charges its delete and waits when it does not fit", { skip }, async () => {
  const DB = d1();
  const env = { DB, OWN_MIRROR_URL: MIRROR, OWN_DAILY_WRITE_BUDGET: "5000", OWN_KEEP_RELEASES: "6" };
  const aug = partRows("20260831"), sep = partRows("20260930");
  seedLegacy(DB, "2026-08-31", aug);
  const { release, files } = await mirrorRelease("2026-09-30", sep);
  const index = { schema: "idx.ownership.mirror.v1", releases: [release] }; // August left the mirror

  await withMirror({ "index.json": JSON.stringify(index), ...files }, async () => {
    setBudget(DB, 5000 - aug.length * 3 + 1);
    const wait = await ownershipSync(env, { force: true });
    assert.equal(wait.pruning, "2026-08-31", JSON.stringify(wait));
    assert.equal(held(DB, "2026-08-31"), aug.length);

    setBudget(DB, 0);
    assert.deepEqual(await ownershipSync(env, { force: true }), { pruned: "2026-08-31" });
    assert.equal(held(DB, "2026-08-31"), 0);
    assert.equal(released(DB, "2026-08-31"), null);
    assert.equal(budgetUsed(DB), aug.length * 3);
  });
});
