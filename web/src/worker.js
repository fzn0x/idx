// idx web: Cloudflare Worker for ihsg.midoriresearch.com.
//
// - Serves the landing page and the web terminal from `public/` (the assets
//   layer answers those without running the Worker; `public/_headers` gives
//   them the same security headers, see src/headers.js).
// - `/api/*` answers with the same JSON envelopes as `idx -o json`, backed by
//   a D1 snapshot that `scheduled()` refreshes every minute in trading hours.
// - Domain-only: any Host other than PUBLIC_HOST gets a 404, so even an
//   accidental workers.dev or preview route exposes nothing.

import { route, ApiError, refreshSnapshot } from "./api.js";
import { ownershipSync } from "./ownership-sync.js";
import { bandarScanStep } from "./bandar-scan.js";
import { SECURITY_HEADERS } from "./headers.js";

// Constant-time comparison for the maintainer token.
function tokenOk(env, given) {
  const want = env.ADMIN_TOKEN || "";
  if (!want || typeof given !== "string" || given.length !== want.length) return false;
  let diff = 0;
  for (let i = 0; i < want.length; i++) diff |= want.charCodeAt(i) ^ given.charCodeAt(i);
  return diff === 0;
}

function json(body, status = 200, extra = {}) {
  return new Response(JSON.stringify(body, null, 2), {
    status,
    // noindex: API envelopes are data for the pages and the CLI, not search results.
    headers: { "content-type": "application/json; charset=utf-8", "cache-control": "no-store", "x-robots-tag": "noindex", ...SECURITY_HEADERS, ...extra },
  });
}

function errorResponse(e) {
  if (e instanceof ApiError || e?.code === "usage") {
    const exit = e.exit ?? 2, status = e.status ?? (e.code === "usage" ? 400 : 500);
    return json({ error: { code: e.code, message: e.message }, exit_code: exit }, status);
  }
  console.error("api error", e);
  return json({ error: { code: "provider", message: String(e?.message || e) }, exit_code: 3 }, 502);
}

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    if (url.hostname !== env.PUBLIC_HOST) {
      return new Response("Not found", { status: 404 });
    }
    if (url.pathname === "/healthz") {
      return new Response("ok", { headers: { "content-type": "text/plain" } });
    }
    if (url.pathname.startsWith("/api/")) {
      if (request.method !== "GET") return json({ error: { code: "usage", message: "GET only" }, exit_code: 2 }, 405);
      // Maintainer-only: force a snapshot refresh (needs the shared token).
      if (url.pathname === "/api/refresh") {
        if (!tokenOk(env, url.searchParams.get("token"))) return json({ error: { code: "usage", message: "forbidden" }, exit_code: 2 }, 403);
        try {
          if (url.searchParams.get("what") === "ownership") return json(await ownershipSync(env, { force: true }));
          if (url.searchParams.get("what") === "bandar") return json(await bandarScanStep(env, { force: true }));
          return json(await refreshSnapshot(env, { force: true }));
        } catch (e) { return errorResponse(e); }
      }
      try {
        const env_ = await route(env, url);
        const ttl = url.pathname === "/api/market" || url.pathname === "/api/quote" ? 20 : 60;
        return json(env_, 200, { "cache-control": `public, max-age=${ttl}` });
      } catch (e) {
        return errorResponse(e);
      }
    }
    const response = await env.ASSETS.fetch(request);
    const headers = new Headers(response.headers);
    for (const [k, v] of Object.entries(SECURITY_HEADERS)) headers.set(k, v);
    return new Response(response.body, { status: response.status, headers });
  },

  async scheduled(event, env, ctx) {
    ctx.waitUntil(
      refreshSnapshot(env)
        .then((r) => console.log("snapshot", JSON.stringify(r)))
        .catch((e) => console.error("snapshot failed", e)),
    );
    // KSEI register: one step per minute from the GitHub mirror (no write API).
    ctx.waitUntil(
      ownershipSync(env)
        .then((r) => { if (!r.idle) console.log("ownership", JSON.stringify(r)); })
        .catch((e) => console.error("ownership sync failed", e)),
    );
    // Bandarmology scan of the session's most traded stocks, after the close.
    ctx.waitUntil(
      bandarScanStep(env)
        .then((r) => { if (!r.idle) console.log("bandar", JSON.stringify(r)); })
        .catch((e) => console.error("bandar scan failed", e)),
    );
  },
};
