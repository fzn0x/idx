// What one Worker invocation may spend on subrequests. Cloudflare's Free plan
// allows 50 per invocation, each hop of a redirect counted; its D1 limits
// page also caps D1 queries at 50 per invocation, and whether those share
// the first 50 is not settled. So the minute cron counts its fetches and its
// D1 queries against one limit below 50: a fetch that would pass it throws
// BudgetError without going out; D1 queries are only counted (each job
// keeps a few for its writes, see bandar-scan.js).

/** Subrequests one cron invocation spends, fetches and D1 queries together. */
export const CRON_SUBREQUESTS = 44;

export class BudgetError extends Error {
  constructor(message) {
    super(message);
    this.name = "BudgetError";
  }
}

/** Ours, or the runtime's own when a count was off ("Too many subrequests", D1's "Too many API requests"). */
export const isBudgetError = (e) => e instanceof BudgetError || /too many (subrequests|api requests)/i.test(String(e?.message ?? e));

/** Subrequests a fetch costs: a GitHub release download redirects once, to its asset host. */
export const hops = (url) => (/^https:\/\/github\.com\/[^/]+\/[^/]+\/releases\/download\//.test(String(url)) ? 2 : 1);

// The D1 statement a counted one wraps, for batch().
const INNER = Symbol("statement");

/**
 * A budget of `limit` subrequests. `fetch` is the counted fetch to hand the
 * jobs, `db(DB)` a D1 binding that counts its queries (a batch is one).
 * `fetchFn` is what actually goes out (the global fetch by default, read at
 * call time); `clock` gives milliseconds.
 */
export function budget(limit = CRON_SUBREQUESTS, { fetchFn = (u, init) => fetch(u, init), clock = () => Date.now() } = {}) {
  const started = clock();
  let fetches = 0, queries = 0;
  const used = () => fetches + queries;
  const count = () => { queries++; };
  const stmt = (s) => ({
    [INNER]: s,
    bind: (...a) => stmt(s.bind(...a)),
    first: (...a) => (count(), s.first(...a)),
    all: (...a) => (count(), s.all(...a)),
    run: (...a) => (count(), s.run(...a)),
    raw: (...a) => (count(), s.raw(...a)),
  });
  return {
    limit,
    used,
    left: () => limit - used(),
    counts: () => ({ fetch: fetches, d1: queries }),
    elapsedMs: () => clock() - started,
    fetch: async (url, init) => {
      const n = hops(url);
      if (used() + n > limit) throw new BudgetError(`subrequest budget spent (${used()} of ${limit})`);
      fetches += n;
      return fetchFn(url, init);
    },
    db: (DB) => ({
      prepare: (sql) => stmt(DB.prepare(sql)),
      batch: (list) => (count(), DB.batch(list.map((s) => s[INNER] ?? s))),
    }),
  };
}
