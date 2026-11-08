// node --test web/test/*.mjs (no network). A terminal result's head says how
// the answer was served: cached (every response it was built from came from
// the cache), live (fetched for this request) or partly cached. An envelope
// built from several responses is cached only when all of them were and
// carries the oldest one's fetched_at (Sourced::absorb in the CLI), so an
// uncached answer fetched before it was asked mixes both. public/app.js holds
// the rule (cacheState) and api() keeps when each answer was asked, on the
// server's clock, for a request it sent past the browser's cache; the
// terminal asks every command that way and words the head and the machine
// line from it (provenance(), lifted out of its source here as
// terminal.test.mjs does).
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

/** api() over a stub fetch, asked as the terminal asks: the body, the response's Date header (or none) and how long the request takes. */
async function served(body, date, ms = 0, opts = { fresh: true }) {
  const realFetch = globalThis.fetch;
  globalThis.fetch = async () => {
    if (ms) await new Promise((r) => setTimeout(r, ms));
    return new Response(JSON.stringify(body), { status: 200, headers: { "content-type": "application/json", ...(date ? { date } : {}) } });
  };
  try { return await api("foreign?symbol=BBCA", opts); } finally { globalThis.fetch = realFetch; }
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

  // A plain request may be the browser's copy, which says nothing of when it was asked.
  const plain = await served(answer(false, new Date().toISOString()), new Date().toUTCString(), 0, {});
  assert.equal(askedAt(plain), undefined);
});

/**
 * The browser's HTTP cache in front of a stub Worker: within the max-age the
 * Worker sends (20 or 60 s, with no validators) it answers a request with its
 * copy at once, under the Date the copy was sent with, unless the request asks
 * past it. `answers` are the Worker's, one per request that reaches it.
 */
function browserCache(answers) {
  const kept = new Map(), reached = [];
  const fetch = async (url, init = {}) => {
    const copy = kept.get(url);
    if (copy && Date.now() < copy.until && !["no-cache", "no-store", "reload"].includes(init.cache)) return copy.res.clone();
    reached.push(url);
    const res = answers.shift()();
    if (init.cache !== "no-store") kept.set(url, { res: res.clone(), until: Date.now() + 60_000 });
    return res;
  };
  return { fetch, kept, reached };
}

test("a command asked again within the Worker's max-age gets the Worker's answer, not the browser's copy", async () => {
  const realFetch = globalThis.fetch;
  const now = Math.floor(Date.now() / 1000) * 1000;
  const iso = (ms) => new Date(ms).toISOString().replace(/\.\d{3}Z$/, "Z");
  const reply = (body, date) => () => new Response(JSON.stringify(body), {
    status: 200, headers: { "content-type": "application/json", "cache-control": "public, max-age=60", date: new Date(date).toUTCString() },
  });
  const path = "foreign?symbol=BBCA";
  try {
    // A moment ago this took 3 s, every part fetched for it (dated from its
    // start), and the answer stayed in the browser's cache. Asked again (the
    // language switch, Run again, Back), the Worker reads it from D1.
    const slow = browserCache([reply(answer(true, iso(now - 3000)), now)]);
    slow.kept.set(`/api/${path}`, { res: reply(answer(false, iso(now - 3000)), now)(), until: Date.now() + 60_000 });
    globalThis.fetch = slow.fetch;
    const again = await api(path, { fresh: true });
    assert.deepEqual(slow.reached, [`/api/${path}`], "the request reached the Worker");
    assert.equal(cacheState(again), "cached", "the Worker's word, not partly cached");

    // A fast live answer asked again: the Worker serves its D1 copy, which is cached, not live.
    const fast = browserCache([reply(answer(false, iso(now)), now), reply(answer(true, iso(now)), now)]);
    globalThis.fetch = fast.fetch;
    assert.equal(cacheState(await api(path, { fresh: true })), "live");
    assert.equal(cacheState(await api(path, { fresh: true })), "cached");
    assert.equal(fast.reached.length, 2);

    // A plain request may still take the browser's copy (the universe, the status bar, the chart's ranges).
    await api(path);
    assert.equal(fast.reached.length, 2);
  } finally {
    globalThis.fetch = realFetch;
  }
});

test("the terminal asks every command past the browser's cache", () => {
  const src = readFileSync(join(web, "public", "terminal.js"), "utf8");
  assert.match(src, /^const ask = \(path\) => api\(path, \{ fresh: true \}\);$/m);
  // run() routes the line; runWeb() asks for its answer.
  const fn = (name) => src.match(new RegExp(`^async function ${name}\\(line\\) \\{[\\s\\S]*?^\\}$`, "m"))?.[0];
  assert.ok(fn("run") && fn("runWeb"), "run() and runWeb() found in terminal.js");
  const body = fn("run") + fn("runWeb");
  assert.equal(body.match(/\bapi\(/g), null, "no command request that the browser may answer from its copy");
  const asks = body.match(/\bawait ask\(/g)?.length ?? 0;
  assert.ok(asks >= 30, `${asks} requests through ask()`);
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
