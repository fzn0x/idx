// The minute cron (wrangler.toml [triggers]). One invocation runs the quote
// board refresh and the KSEI register sync together, then one step of the
// bandar scan with what is left of the invocation's subrequest budget
// (src/budget.js), and resolves only once all three are done: the handler's
// own promise has the cron's wall time, while work left to ctx.waitUntil
// after scheduled() returns has no documented limit of its own.

import { refreshSnapshot, marketIsOpen } from "./api.js";
import { ownershipSync } from "./ownership-sync.js";
import { bandarScanStep } from "./bandar-scan.js";
import { budget, CRON_SUBREQUESTS } from "./budget.js";

/** A register step that loaded or deleted part of a month: the D1 work of a whole minute. */
const ownershipWrote = (r) => r != null && (r.part != null || r.committed != null || r.pruned != null);

const settle = (p) => p.then((value) => ({ status: "fulfilled", value }), (reason) => ({ status: "rejected", reason }));

/**
 * One cron minute. `now` is the minute it runs for; `fetchFn` stands in for
 * fetch and `limit` for the budget (tests). Returns each job's result and the
 * subrequests spent; when a job failed it throws once all three have run, so
 * the Cron event records the failure.
 */
export async function runCron(env, { now = new Date(), fetchFn, limit = CRON_SUBREQUESTS } = {}) {
  const b = budget(limit, fetchFn ? { fetchFn } : {});
  const cenv = { ...env, DB: b.db(env.DB) };
  const [snapshot, ownership] = await Promise.all([
    settle(refreshSnapshot(cenv, { now, fetchFn: b.fetch })),
    settle(ownershipSync(cenv, { fetchFn: b.fetch })),
  ]);
  if (snapshot.status === "fulfilled") console.log("snapshot", JSON.stringify(snapshot.value));
  else console.error("snapshot failed", snapshot.reason);
  if (ownership.status === "rejected") console.error("ownership sync failed", ownership.reason);
  else if (!ownership.value.idle) console.log("ownership", JSON.stringify(ownership.value));
  // The scan reads no stock in a minute the register spent on a part, nor
  // after the 15-minute board refresh outside trading hours (16:45 among
  // them: the probe alone then); in trading hours the board refreshes every
  // minute and the budget alone decides.
  const refreshed = snapshot.status === "fulfilled" && !snapshot.value.skipped;
  const stocks = !(ownership.status === "fulfilled" && ownershipWrote(ownership.value)) && (!refreshed || marketIsOpen(now));
  const bandar = await settle(bandarScanStep(cenv, { now, budget: b, stocks }));
  if (bandar.status === "rejected") console.error("bandar scan failed", bandar.reason);
  else if (!bandar.value.idle) console.log("bandar", JSON.stringify({ ...bandar.value, subrequests: b.counts() }));
  const failed = [["snapshot", snapshot], ["ownership sync", ownership], ["bandar scan", bandar]].filter(([, r]) => r.status === "rejected");
  if (failed.length) throw new Error(failed.map(([name, r]) => `${name} failed: ${r.reason?.message ?? r.reason}`).join("; "));
  return { snapshot: snapshot.value, ownership: ownership.value, bandar: bandar.value, subrequests: b.counts() };
}
