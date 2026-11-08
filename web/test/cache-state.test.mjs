// node --test web/test/*.mjs (no network). A terminal result's head says how
// the answer was served: cached (every response it was built from came from
// the cache), live (fetched for this request) or partly cached. An envelope
// built from several responses is cached only when all of them were and
// carries the oldest one's fetched_at (Sourced::absorb in the CLI), so an
// uncached answer fetched before it was asked mixes both. public/app.js holds
// the rule (cacheState) and api() keeps when each answer was asked, on the
// server's clock; public/terminal.js words the head and the machine line from
// it (provenance(), lifted out of its source here as terminal.test.mjs does).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

import { api, askedAt, cacheState, CACHE_STATES } from "../public/app.js";

const web = join(dirname(fileURLToPath(import.meta.url)), "..");
const answer = (cached, fetched_at) => ({ schema: "idx.foreign.value.v1", provider: "ipot", fetched_at, cached, stale: false, data: { symbol: "BBCA" } });
const DASHES = new RegExp(`[${String.fromCharCode(0x2013, 0x2014)}]`);

test("cached, live or partly cached, from the envelope and when it was asked", () => {
  const asked = Date.parse("2026-11-08T04:00:00Z");
  // Every part from the cache, however old.
  assert.equal(cacheState(answer(true, "2026-11-01T05:11:00Z"), asked), "cached");
  assert.equal(cacheState(answer(true, "2026-11-08T04:00:00Z"), asked), "cached");
  // Fetched for this request: from the moment it was asked on, give or take whole seconds.
  assert.equal(cacheState(answer(false, "2026-11-08T04:00:00Z"), asked), "live");
  assert.equal(cacheState(answer(false, "2026-11-08T04:00:09Z"), asked), "live");
  assert.equal(cacheState(answer(false, "2026-11-08T03:59:58Z"), asked), "live");
  // Uncached, yet its oldest part was fetched before the request: that part came from the cache.
  assert.equal(cacheState(answer(false, "2026-11-08T03:59:57Z"), asked), "partly");
  // The page's own case: foreign BBCA, rows to 11-06, one session's pages read on 1 Nov.
  assert.equal(cacheState(answer(false, "2026-11-01T05:11:00Z"), asked), "partly");
  // A slow answer fetched in full while it ran (30 s; its parts are dated from its start) is live.
  const answered = Date.parse("2026-11-08T04:00:30Z");
  assert.equal(cacheState(answer(false, "2026-11-08T04:00:01Z"), answered - 30_000), "live");
  // Without a time it was asked, nothing says a part is older.
  assert.equal(cacheState(answer(false, "2026-11-01T05:11:00Z")), "live");
});

/** api() over a stub fetch: the body, the response's Date header (or none) and how long the request takes. */
async function served(body, date, ms = 0) {
  const realFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    if (ms) await new Promise((r) => setTimeout(r, ms));
    return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json", ...(date ? { date } : {}) } });
  };
  try { return await api("foreign?symbol=BBCA"); } finally { globalThis.fetch = realFetch; }
}

test("api() keeps when each answer was asked on the server's clock, beside the envelope", async () => {
  const day = 86400e3;
  // This machine's clock runs a day ahead of the server's: an answer fetched
  // as it was asked stays live, judged by the server's Date.
  const behind = new Date(Math.floor((Date.now() - day) / 1000) * 1000);
  const live = answer(false, behind.toISOString().replace(/\.\d{3}Z$/, "Z"));
  const got = await served(live, behind.toUTCString(), 40);
  assert.deepEqual(got, live, "the envelope stays the CLI's");
  assert.equal(JSON.stringify(got), JSON.stringify(live), "nothing added to it");
  const asked = askedAt(got);
  assert.ok(asked <= behind.getTime() - 30 && asked > behind.getTime() - 5000, `asked ${new Date(asked).toISOString()} is the Date less the wait`);
  assert.equal(cacheState(got), "live");

  // Two days behind it: a part cached a day before the request is still older than the request.
  const ahead = new Date(Math.floor((Date.now() + 2 * day) / 1000) * 1000);
  const mixed = answer(false, new Date(ahead.getTime() - day).toISOString().replace(/\.\d{3}Z$/, "Z"));
  assert.equal(cacheState(await served(mixed, ahead.toUTCString())), "partly");

  // No Date header: this machine's clock stands in.
  const before = Date.now();
  const bare = await served(answer(false, new Date(before - day).toISOString()), null);
  assert.ok(askedAt(bare) <= Date.now() && askedAt(bare) >= before - 1000);
  assert.equal(cacheState(bare), "partly");
  assert.equal(askedAt({ ...bare }), undefined, "kept for the envelope api() returned, not a copy");
});

test("the head's words come in both languages, without dashes", () => {
  assert.deepEqual(Object.keys(CACHE_STATES), ["cached", "partly", "live"]);
  assert.deepEqual(CACHE_STATES.cached, { id: "dari cache", en: "cached", fetched: { id: "diambil", en: "fetched" } });
  assert.deepEqual(CACHE_STATES.partly, { id: "sebagian dari cache", en: "partly cached", fetched: { id: "bagian tertua diambil", en: "oldest part fetched" } });
  assert.deepEqual(CACHE_STATES.live, { id: "langsung", en: "live", fetched: { id: "diambil", en: "fetched" } });
  for (const s of Object.values(CACHE_STATES)) for (const w of [s.id, s.en, s.fetched.id, s.fetched.en]) assert.ok(!DASHES.test(w), w);
});

test("the terminal's head, machine line and sentence say the same, in both languages", () => {
  const src = readFileSync(join(web, "public", "terminal.js"), "utf8");
  const fn = src.match(/^function provenance\(env\) \{[\s\S]*?^\}$/m)?.[0];
  assert.ok(fn, "provenance() found in terminal.js");
  const start = src.indexOf("async function execute(");
  const exec = src.slice(start, src.indexOf("\n}\n", start));
  assert.ok(/p = provenance\(env\)/.test(exec) && /e\.meta\.textContent = `\$\{p\.meta\}/.test(exec) && /machineLine\(e\.mline, p\.fields, p\.sentence\)/.test(exec),
    "execute() takes the head and the machine line from provenance()");
  // Stand-ins: the state comes with the stub envelope; fmt.when marks where the WIB time goes.
  const make = (lang) => new Function("cacheState", "CACHE_STATES", "fmt", "t", `${fn}\nreturn provenance;`)(
    (env) => env.state, CACHE_STATES, { when: (iso) => `<${iso}>` }, (id, en) => (lang === "id" ? id : en));
  const at = "2026-11-01T05:11:00Z";
  const cases = {
    en: {
      cached: "ipot · cached · fetched <2026-11-01T05:11:00Z>",
      partly: "ipot · partly cached · stale · oldest part fetched <2026-11-01T05:11:00Z>",
      live: "ipot · live · fetched <2026-11-01T05:11:00Z>",
    },
    id: {
      cached: "ipot · dari cache · diambil <2026-11-01T05:11:00Z>",
      partly: "ipot · sebagian dari cache · kedaluwarsa · bagian tertua diambil <2026-11-01T05:11:00Z>",
      live: "ipot · langsung · diambil <2026-11-01T05:11:00Z>",
    },
  };
  for (const [lang, want] of Object.entries(cases)) {
    const provenance = make(lang);
    for (const state of Object.keys(CACHE_STATES)) {
      const p = provenance({ ...answer(state === "cached", at), stale: state === "partly", state });
      const s = CACHE_STATES[state];
      assert.equal(p.meta, want[state], `${lang} ${state}`);
      // The machine line keeps the English word, as it does on both pages.
      assert.deepEqual(p.fields, ["idx.foreign.value.v1", "ipot", s.en, state === "partly" ? "stale" : null, at], `${lang} ${state}`);
      assert.ok(p.sentence.includes(`, ${s[lang]}`) && p.sentence.includes(`${s.fetched[lang]} ${at}.`), `${lang} ${state}: ${p.sentence}`);
    }
  }
});
