// node web/scripts/og.mjs
//
// Renders the social card and the touch icon from their HTML sources:
//   og/card.html -> public/og.png               (1200x630, og:image)
//   og/icon.html -> public/apple-touch-icon.png (180x180)
// Drives headless Chrome over the DevTools protocol so the capture waits for
// the web fonts (`--screenshot` does not, and on Windows returns before the
// file exists). Needs Node 22+ (global WebSocket), Chrome or Chromium
// (CHROME=/path overrides) and network for Google Fonts.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const web = join(dirname(fileURLToPath(import.meta.url)), "..");
const JOBS = [
  { src: "card.html", out: "og.png", width: 1200, height: 630, fonts: ["800 124px Doto", '400 25px "Martian Mono"', '700 21px "Martian Mono"'] },
  { src: "icon.html", out: "apple-touch-icon.png", width: 180, height: 180, fonts: [] },
];

function findChrome() {
  if (process.env.CHROME) return process.env.CHROME;
  const candidates = {
    win32: [
      "C:/Program Files/Google/Chrome/Application/chrome.exe",
      "C:/Program Files (x86)/Google/Chrome/Application/chrome.exe",
      "C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe",
    ],
    darwin: ["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/Applications/Chromium.app/Contents/MacOS/Chromium"],
    linux: ["/usr/bin/chromium", "/usr/bin/chromium-browser", "/usr/bin/google-chrome", "/usr/bin/google-chrome-stable"],
  }[process.platform] ?? [];
  const found = candidates.find((c) => existsSync(c));
  if (!found) throw new Error("no Chrome or Chromium found; set CHROME=/path/to/chrome");
  return found;
}

// Chrome writes "<port>\n<browser ws path>" here once DevTools is listening.
async function devtoolsUrl(profile) {
  const file = join(profile, "DevToolsActivePort");
  for (let i = 0; i < 150; i++) {
    if (existsSync(file)) {
      const [port, path] = readFileSync(file, "utf8").split("\n");
      if (port && path) return `ws://127.0.0.1:${port}${path}`;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error("Chrome did not open a DevTools port within 15 s");
}

function connect(url) {
  const ws = new WebSocket(url);
  let seq = 0;
  const pending = new Map();
  const waiters = [];
  const closed = new Promise((res) => ws.addEventListener("close", res));
  ws.addEventListener("message", (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.id && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(`${msg.error.message} (${msg.error.code})`));
      else resolve(msg.result);
    } else if (msg.method) {
      const i = waiters.findIndex((w) => w.method === msg.method && w.sessionId === msg.sessionId);
      if (i >= 0) waiters.splice(i, 1)[0].resolve(msg.params);
    }
  });
  return new Promise((resolve, reject) => {
    ws.addEventListener("error", () => reject(new Error(`cannot connect to ${url}`)));
    ws.addEventListener("open", () =>
      resolve({
        send: (method, params = {}, sessionId) =>
          new Promise((res, rej) => {
            const id = ++seq;
            pending.set(id, { resolve: res, reject: rej });
            ws.send(JSON.stringify({ id, method, params, sessionId }));
          }),
        once: (method, sessionId) => new Promise((res) => waiters.push({ method, sessionId, resolve: res })),
        closed,
      }),
    );
  });
}

async function render(cdp, session, job) {
  await cdp.send("Emulation.setDeviceMetricsOverride", { width: job.width, height: job.height, deviceScaleFactor: 1, mobile: false }, session);
  const loaded = cdp.once("Page.loadEventFired", session);
  await cdp.send("Page.navigate", { url: pathToFileURL(join(web, "og", job.src)).href }, session);
  await loaded;
  const { result } = await cdp.send(
    "Runtime.evaluate",
    {
      expression: `(async (fonts) => {
        await Promise.all(fonts.map((f) => document.fonts.load(f)));
        await document.fonts.ready;
        return fonts.filter((f) => !document.fonts.check(f));
      })(${JSON.stringify(job.fonts)})`,
      awaitPromise: true,
      returnByValue: true,
    },
    session,
  );
  if (result.value?.length) throw new Error(`${job.src}: fonts did not load: ${result.value.join(", ")}`);
  const shot = await cdp.send("Page.captureScreenshot", { format: "png" }, session);
  writeFileSync(join(web, "public", job.out), Buffer.from(shot.data, "base64"));
  console.log(`public/${job.out} ${job.width}x${job.height}`);
}

const profile = mkdtempSync(join(tmpdir(), "idx-og-"));
const chrome = spawn(
  findChrome(),
  ["--headless=new", "--disable-gpu", "--hide-scrollbars", "--no-first-run", "--no-default-browser-check", `--user-data-dir=${profile}`, "--remote-debugging-port=0", "about:blank"],
  { stdio: "ignore" },
);
let cdp;
try {
  cdp = await connect(await devtoolsUrl(profile));
  const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
  await cdp.send("Page.enable", {}, sessionId);
  for (const job of JOBS) await render(cdp, sessionId, job);
} finally {
  // Chrome drops the socket once it has shut down and released the profile.
  if (cdp) {
    cdp.send("Browser.close").catch(() => {});
    await Promise.race([cdp.closed, new Promise((r) => setTimeout(r, 5000))]);
  }
  chrome.kill();
  // Its helpers can still flush the profile for a moment after that.
  for (let i = 0; ; i++) {
    try {
      rmSync(profile, { recursive: true, force: true });
      break;
    } catch {
      if (i === 40) { console.error(`og: could not remove ${profile}; delete it by hand`); break; }
      await new Promise((r) => setTimeout(r, 250));
    }
  }
}
