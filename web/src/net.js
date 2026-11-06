// Outbound fetch with a deadline. A source can accept the connection and
// never answer (Kontan, Oct 2026); without a deadline the Worker waits with
// it and the browser's request stays pending. The body is read inside the
// deadline and handed back buffered, so callers keep using r.ok, r.status,
// r.headers, r.json() and r.text().

export const DEFAULT_TIMEOUT_MS = 10_000;

/**
 * Cuts the query string and fragment of every http(s) URL in `text`, so a
 * warning never carries a session crumb or key (the CLI's redact_urls).
 */
export const redactUrls = (text) => String(text).replace(/(https?:\/\/[^\s)"'<>?#]*)[?#][^\s)"'<>]*/g, "$1");
const NULL_BODY = new Set([101, 204, 205, 304]);

const hostOf = (u) => { try { return new URL(u).host; } catch { return String(u); } };

export async function fetchWithin(url, init = {}, { ms = DEFAULT_TIMEOUT_MS, fetchFn = fetch } = {}) {
  const signal = AbortSignal.timeout(ms);
  try {
    const r = await fetchFn(url, { ...init, signal });
    const body = NULL_BODY.has(r.status) ? null : await r.arrayBuffer();
    // The body is already decoded; its encoding and length no longer apply.
    const headers = new Headers(r.headers);
    headers.delete("content-encoding");
    headers.delete("content-length");
    return new Response(body, { status: r.status, statusText: r.statusText, headers });
  } catch (e) {
    if (signal.aborted || e?.name === "TimeoutError") throw new Error(`no answer within ${ms / 1000}s from ${hostOf(url)}`);
    throw e;
  }
}
