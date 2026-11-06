// node --test web/test/*.mjs (no network). The Star button's count: /api/repo
// reads GitHub at most once an hour, sends a User-Agent and never a token,
// and a refusal leaves the page with a plain Star link instead of an error.
import { test } from "node:test";
import assert from "node:assert/strict";

import { repo, parseRepo, route, REPO_TTL_SECS, GITHUB_UA } from "../src/api.js";

// The cache table as D1 would hold it: k -> { v, expires_at, fetched_at }.
function memoryDb() {
  const rows = new Map();
  return {
    rows,
    prepare: (sql) => ({ bind: (...a) => ({
      first: async () => (sql.startsWith("SELECT") ? rows.get(a[0]) ?? null : null),
      run: async () => { if (sql.startsWith("INSERT")) rows.set(a[0], { v: a[1], expires_at: a[2], fetched_at: a[3] }); return {}; },
    }) }),
  };
}

// The fields GET https://api.github.com/repos/fzn0x/idx answered with on 2026-11-06, trimmed.
const BODY = {
  id: 1, name: "idx", full_name: "fzn0x/idx", private: false, html_url: "https://github.com/fzn0x/idx",
  stargazers_count: 172, watchers_count: 172, forks_count: 23, open_issues_count: 0, default_branch: "main",
};

function stubGitHub(answer) {
  const calls = [];
  const fn = async (url, init) => {
    calls.push({ url: String(url), headers: new Headers(init?.headers), signal: init?.signal });
    return answer();
  };
  return { fn, calls };
}

async function withFetch(fn, body) {
  const real = globalThis.fetch;
  globalThis.fetch = fn;
  try { return await body(); } finally { globalThis.fetch = real; }
}

test("the star and fork counts come straight from the repository", () => {
  assert.deepEqual(parseRepo(BODY), { repo: "fzn0x/idx", url: "https://github.com/fzn0x/idx", stars: 172, forks: 23 });
  assert.deepEqual(parseRepo({ stargazers_count: 0 }), { repo: "fzn0x/idx", url: "https://github.com/fzn0x/idx", stars: 0, forks: null });
  assert.throws(() => parseRepo({ message: "API rate limit exceeded" }), /without a star count/);
  assert.throws(() => parseRepo(null), /without a star count/);
});

test("GitHub is asked once an hour, with a User-Agent and no token", async () => {
  const gh = stubGitHub(() => new Response(JSON.stringify(BODY), { status: 200, headers: { "content-type": "application/json" } }));
  const env = { DB: memoryDb() };
  await withFetch(gh.fn, async () => {
    const first = await repo(env);
    assert.equal(first.schema, "idx.repo.v1");
    assert.equal(first.provider, "github");
    assert.equal(first.cached, false);
    assert.match(first.fetched_at, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
    assert.deepEqual(first.data, { repo: "fzn0x/idx", url: "https://github.com/fzn0x/idx", stars: 172, forks: 23 });

    const again = await route(env, new URL("https://ihsg.midoriresearch.com/api/repo"));
    assert.equal(again.cached, true);
    assert.deepEqual(again.data, first.data);
  });
  assert.equal(gh.calls.length, 1, "the second read is served from the cache");
  const [call] = gh.calls;
  assert.equal(call.url, "https://api.github.com/repos/fzn0x/idx");
  assert.equal(call.headers.get("user-agent"), GITHUB_UA);
  assert.equal(call.headers.get("authorization"), null);
  assert.ok(call.signal instanceof AbortSignal, "the request carries a deadline");
  const row = env.DB.rows.get("repo:fzn0x/idx");
  assert.equal(row.expires_at - row.fetched_at, REPO_TTL_SECS);
  assert.equal(REPO_TTL_SECS, 3600);
});

test("a refusal is an error the page can ignore, remembered so GitHub is not asked again at once", async () => {
  const gh = stubGitHub(() => new Response('{"message":"API rate limit exceeded"}', { status: 403 }));
  const env = { DB: memoryDb() };
  await withFetch(gh.fn, async () => {
    await assert.rejects(repo(env), (e) => e.code === "provider" && /HTTP 403/.test(e.message));
    await assert.rejects(repo(env), /HTTP 403; next try in \d+s/);
  });
  assert.equal(gh.calls.length, 1);
});

test("an expired count is still shown, marked stale, when GitHub is down", async () => {
  const gh = stubGitHub(() => { throw new TypeError("fetch failed"); });
  const env = { DB: memoryDb() };
  const now = Math.floor(Date.now() / 1000);
  env.DB.rows.set("repo:fzn0x/idx", { v: JSON.stringify({ repo: "fzn0x/idx", url: "https://github.com/fzn0x/idx", stars: 170, forks: 22 }), expires_at: now - 5, fetched_at: now - 3605 });
  const out = await withFetch(gh.fn, () => repo(env));
  assert.equal(out.stale, true);
  assert.equal(out.data.stars, 170);
  assert.match(out.warnings[0], /live fetch failed \(fetch failed\)/);
});
