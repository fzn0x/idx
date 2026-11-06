// Locates or downloads the idx release binary for this platform.
//
// Shared by both npm packages: ihsg-cli (npm/) and ihsg-mcp (npm-mcp/, which
// carries a byte-identical copy of this file; npm/test/launcher.test.mjs
// compares them, so edit npm/lib/binary.js and copy it over).
//
// Used by install.js (postinstall) and by the launchers (bin/idx.js,
// bin/ihsg-mcp.js) on first run, because npm 12 no longer runs dependency
// install scripts by default, installs from GitHub
// (`npm install -g github:fzn0x/idx`) ship no install script at all, and
// ihsg-mcp has none: the launcher fetches the binary on first use instead.
// Either path verifies SHA-256 against the release's SHA256SUMS before
// writing anything executable.
//
// The binary goes next to the package (its bin/ folder) when that folder is
// writable, otherwise into a per-user cache (root-owned global installs),
// one cache for both packages:
// %LOCALAPPDATA%\ihsg-cli\<version> on Windows, $XDG_CACHE_HOME or
// ~/.cache/ihsg-cli/<version> elsewhere; IDX_NPM_CACHE overrides it.
"use strict";
const fs = require("fs");
const os = require("os");
const path = require("path");
const https = require("https");
const crypto = require("crypto");
const zlib = require("zlib");
const { execFileSync } = require("child_process");

const pkg = require("../package.json");
const REPO = process.env.IDX_REPO || "fzn0x/idx";
const VERSION = process.env.IDX_VERSION || pkg.version;
const BIN_DIR = path.join(__dirname, "..", "bin");
const EXE = os.platform() === "win32" ? "idx.exe" : "idx";

function cacheDir() {
  if (process.env.IDX_NPM_CACHE) return path.join(process.env.IDX_NPM_CACHE, VERSION);
  const base = os.platform() === "win32"
    ? process.env.LOCALAPPDATA || path.join(os.homedir(), "AppData", "Local")
    : process.env.XDG_CACHE_HOME || path.join(os.homedir(), ".cache");
  return path.join(base, "ihsg-cli", VERSION);
}
const DIRS = [BIN_DIR, cacheDir()];

function target() {
  const plat = os.platform();
  const arch = os.arch();
  const a = arch === "x64" ? "x86_64" : arch === "arm64" ? "aarch64" : null;
  if (!a) throw new Error(`unsupported architecture ${arch}`);
  if (plat === "linux") return { triple: `${a}-unknown-linux-musl`, ext: "tar.gz" };
  if (plat === "darwin") return { triple: `${a}-apple-darwin`, ext: "tar.gz" };
  if (plat === "win32" && a === "x86_64") return { triple: "x86_64-pc-windows-msvc", ext: "zip" };
  throw new Error(`unsupported platform ${plat}/${arch}`);
}

// A download that stops sending (connection open, no bytes) fails after this
// long instead of hanging the install or the first run of `idx`. Our own
// timer, restarted per chunk: Node's socket timeout fires up to twice as late.
const IDLE_TIMEOUT_MS = 30000;

function get(url, redirects = 0) {
  return new Promise((resolve, reject) => {
    let timer;
    const done = (fn) => (v) => { clearTimeout(timer); fn(v); };
    const fail = done(reject);
    const idle = () => {
      clearTimeout(timer);
      timer = setTimeout(() => req.destroy(new Error(`${url}: no data for ${IDLE_TIMEOUT_MS / 1000}s`)), IDLE_TIMEOUT_MS);
    };
    const req = https
      .get(url, { headers: { "User-Agent": "idx-npm-installer" } }, (res) => {
        if ([301, 302, 303, 307, 308].includes(res.statusCode) && res.headers.location && redirects < 5) {
          res.resume();
          return done(resolve)(get(res.headers.location, redirects + 1));
        }
        if (res.statusCode !== 200) {
          res.resume();
          return fail(new Error(`${url}: HTTP ${res.statusCode}`));
        }
        const chunks = [];
        idle();
        res.on("data", (c) => { chunks.push(c); idle(); });
        res.on("end", () => done(resolve)(Buffer.concat(chunks)));
        res.on("error", fail);
        res.on("close", () => { if (!res.complete) fail(new Error(`${url}: download interrupted`)); });
      })
      .on("error", fail);
    idle();
  });
}

// Minimal tar reader: enough for the release archives (ustar, plain files).
function untar(buf, wantedName) {
  let off = 0;
  while (off + 512 <= buf.length) {
    const header = buf.subarray(off, off + 512);
    if (header.every((b) => b === 0)) break;
    const name = header.subarray(0, 100).toString().replace(/\0.*$/, "");
    const size = parseInt(header.subarray(124, 136).toString().replace(/\0.*$/, "").trim(), 8) || 0;
    const prefix = header.subarray(345, 500).toString().replace(/\0.*$/, "");
    const full = prefix ? `${prefix}/${name}` : name;
    off += 512;
    if (path.posix.basename(full) === wantedName && header[156] !== 53) {
      return buf.subarray(off, off + size);
    }
    off += Math.ceil(size / 512) * 512;
  }
  throw new Error(`${wantedName} not found in archive`);
}

/** Path of the binary if it is already installed, else null. */
function installed() {
  for (const dir of DIRS) {
    const p = path.join(dir, EXE);
    if (fs.existsSync(p)) return p;
  }
  return null;
}

/** First folder in DIRS we can write to. */
function writableDir() {
  for (const dir of DIRS) {
    try {
      fs.mkdirSync(dir, { recursive: true });
      fs.accessSync(dir, fs.constants.W_OK);
      return dir;
    } catch {
      // try the next one
    }
  }
  throw new Error(`no writable folder for the binary (tried ${DIRS.join(", ")}); set IDX_NPM_CACHE`);
}

/**
 * Downloads and verifies the binary for `VERSION`. `log` receives progress
 * lines (stderr for the launcher, stdout for postinstall). Resolves to the
 * binary path.
 */
async function download(log = () => {}) {
  if (VERSION === "0.0.0") throw new Error("development package version 0.0.0 has no release to download");
  const { triple, ext } = target();
  const name = `idx-${VERSION}-${triple}`;
  const asset = `${name}.${ext}`;
  const base = `https://github.com/${REPO}/releases/download/v${VERSION}`;
  log(`idx: downloading ${asset}`);
  const [archive, sums] = await Promise.all([get(`${base}/${asset}`), get(`${base}/SHA256SUMS`)]);
  const line = sums.toString().split("\n").find((l) => l.trim().endsWith(` ${asset}`) || l.trim().endsWith(`*${asset}`));
  if (!line) throw new Error(`${asset} missing from SHA256SUMS`);
  const expected = line.trim().split(/\s+/)[0];
  const actual = crypto.createHash("sha256").update(archive).digest("hex");
  if (expected !== actual) throw new Error(`checksum mismatch for ${asset}: expected ${expected}, got ${actual}`);

  const dest = path.join(writableDir(), EXE);
  const tmp = `${dest}.${process.pid}.tmp`;
  try {
    if (ext === "tar.gz") {
      fs.writeFileSync(tmp, untar(zlib.gunzipSync(archive), EXE), { mode: 0o755 });
    } else {
      unzipExe(archive, name, tmp);
    }
    place(tmp, dest);
  } finally {
    rmQuiet(tmp);
  }
  log(`idx: installed ${dest} (sha256 verified)`);
  return dest;
}

// Best-effort cleanup: Windows refuses to delete a file another process still
// holds, and a leftover must not fail the install.
function rmQuiet(p) {
  try {
    fs.rmSync(p, { recursive: true, force: true });
  } catch {
    // left behind
  }
}

/**
 * Windows: writes the release zip's idx.exe to `out`, handing the zip to
 * PowerShell rather than shipping a zip parser. Each run unpacks in its own
 * fresh temp folder, so concurrent first runs share no file, and the paths
 * reach PowerShell through the environment instead of the command text, so a
 * quote in %TEMP% (C:\Users\O'Brien\...) cannot break the command.
 */
function unzipExe(archive, name, out) {
  const work = fs.mkdtempSync(path.join(os.tmpdir(), `${name}-`));
  try {
    const zip = path.join(work, `${name}.zip`); // Expand-Archive wants .zip
    const dir = path.join(work, "out");
    fs.writeFileSync(zip, archive);
    execFileSync(
      "powershell",
      ["-NoProfile", "-Command", "Expand-Archive -Force -LiteralPath $env:IHSG_NPM_ZIP -DestinationPath $env:IHSG_NPM_OUT"],
      { env: { ...process.env, IHSG_NPM_ZIP: zip, IHSG_NPM_OUT: dir } },
    );
    fs.copyFileSync(path.join(dir, name, EXE), out);
  } finally {
    rmQuiet(work);
  }
}

/**
 * Moves the verified binary from `tmp` to `dest`. The rename is atomic, so a
 * concurrent first run never sees a half-written file. Windows refuses it
 * (EPERM) when a concurrent first run has already put `dest` in place and
 * started it; that run verified the same release, so its binary stays.
 */
function place(tmp, dest) {
  try {
    fs.renameSync(tmp, dest);
  } catch (e) {
    if (!fs.existsSync(dest)) throw e;
    rmQuiet(tmp);
  }
}

/** Installed path, downloading first when needed. */
async function ensure(log) {
  return installed() || download(log);
}

module.exports = { installed, download, ensure, target, place, unzipExe, REPO, VERSION, BIN_DIR, EXE, DIRS };
