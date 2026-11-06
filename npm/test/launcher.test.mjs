// node --test npm/test/*.mjs (no network). The two npm packages: ihsg-cli
// (npm/, runs `idx`) and ihsg-mcp (npm-mcp/, runs `idx mcp`) share one
// downloader, and the MCP launcher must keep stdout for the protocol.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn, spawnSync } from "node:child_process";
import { once } from "node:events";
import { chmodSync, cpSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const root = join(dirname(fileURLToPath(import.meta.url)), "..", "..");
const read = (...p) => readFileSync(join(root, ...p), "utf8");
const pkg = (dir) => JSON.parse(read(dir, "package.json"));
const { mcpArgs } = createRequire(import.meta.url)(join(root, "npm-mcp", "bin", "ihsg-mcp.js"));

test("ihsg-mcp ships ihsg-cli's downloader unchanged, at the same version", () => {
  assert.equal(
    read("npm-mcp", "lib", "binary.js"),
    read("npm", "lib", "binary.js"),
    "edit npm/lib/binary.js, then copy it to npm-mcp/lib/binary.js",
  );
  const cargo = read("Cargo.toml").match(/^version = "([^"]+)"/m)[1];
  assert.equal(pkg("npm-mcp").version, cargo);
  assert.equal(pkg("npm").version, cargo);
  const m = pkg("npm-mcp");
  assert.equal(m.name, "ihsg-mcp");
  assert.deepEqual(m.bin, { "ihsg-mcp": "bin/ihsg-mcp.js" });
  assert.equal(m.repository.directory, "npm-mcp");
  assert.deepEqual([...m.files].sort(), ["README.md", "bin/ihsg-mcp.js", "lib/"]);
  assert.equal(m.scripts?.postinstall, undefined, "the launcher downloads on first run");
  for (const k of ["license", "os", "cpu", "engines", "homepage", "bugs", "author"]) {
    assert.deepEqual(m[k], pkg("npm")[k], k);
  }
});

test("arguments go to `idx mcp`; setup prints the npx form unless asked otherwise", () => {
  const a = (...x) => mcpArgs(["node", "ihsg-mcp", ...x]);
  assert.deepEqual(a(), ["mcp"]);
  assert.deepEqual(a("--http", "127.0.0.1:8765", "--token", "t"), ["mcp", "--http", "127.0.0.1:8765", "--token", "t"]);
  assert.deepEqual(a("--provider", "yahoo", "--offline"), ["mcp", "--provider", "yahoo", "--offline"]);
  assert.deepEqual(a("setup", "cursor"), ["mcp", "setup", "cursor", "--npx"]);
  assert.deepEqual(a("setup", "--list"), ["mcp", "setup", "--list", "--npx"]);
  assert.deepEqual(a("-o", "json", "setup", "codex"), ["mcp", "-o", "json", "setup", "codex", "--npx"]);
  assert.deepEqual(a("setup", "cursor", "--npx"), ["mcp", "setup", "cursor", "--npx"]);
  assert.deepEqual(a("setup", "chatgpt", "--http", "https://h/mcp"), ["mcp", "setup", "chatgpt", "--http", "https://h/mcp"]);
  assert.deepEqual(a("setup", "cursor", "--http=https://h/mcp"), ["mcp", "setup", "cursor", "--http=https://h/mcp"]);
});

// A copy of a package's launcher, downloader and manifest in a temp folder, so
// a binary left in the checkout's bin/ by a manual run never answers instead.
function stage(dir, launcher) {
  const out = join(mkdtempSync(join(tmpdir(), "ihsg-npm-")), dir);
  mkdirSync(join(out, "bin"), { recursive: true });
  cpSync(join(root, dir, "bin", launcher), join(out, "bin", launcher));
  cpSync(join(root, dir, "lib"), join(out, "lib"), { recursive: true });
  cpSync(join(root, dir, "package.json"), join(out, "package.json"));
  return join(out, "bin", launcher);
}

// A stand-in binary in the per-user cache; by default it prints its
// arguments and exits 7.
function fakeCache(script = 'echo "$@"\nexit 7\n') {
  const dir = mkdtempSync(join(tmpdir(), "ihsg-npm-"));
  const ver = pkg("npm-mcp").version;
  mkdirSync(join(dir, ver), { recursive: true });
  const bin = join(dir, ver, "idx");
  writeFileSync(bin, `#!/bin/sh\n${script}`);
  chmodSync(bin, 0o755);
  return dir;
}

test("the launchers run the cached binary with the exit code it returns", { skip: process.platform === "win32" && "POSIX stand-in binary" }, () => {
  const env = { ...process.env, IDX_NPM_CACHE: fakeCache() };
  delete env.IDX_VERSION;
  const run = (launcher, ...args) => spawnSync(process.execPath, [launcher, ...args], { env, encoding: "utf8" });

  const mcp = run(stage("npm-mcp", "ihsg-mcp.js"), "--http", "127.0.0.1:0", "--provider", "yahoo");
  assert.equal(mcp.status, 7);
  assert.equal(mcp.stdout, "mcp --http 127.0.0.1:0 --provider yahoo\n");
  assert.equal(mcp.stderr, "", "nothing but the server may write while it runs");

  const setup = run(stage("npm-mcp", "ihsg-mcp.js"), "setup", "cursor");
  assert.equal(setup.stdout, "mcp setup cursor --npx\n");

  const cli = run(stage("npm", "idx.js"), "stocks", "quote", "BBCA");
  assert.equal(cli.status, 7);
  assert.equal(cli.stdout, "stocks quote BBCA\n");
});

test("a supervisor's SIGTERM reaches the server, and the server's signal ends the launcher", { skip: process.platform === "win32" && "POSIX signals" }, async () => {
  // Stopped by a supervisor: the launcher passes SIGTERM on and exits as idx does.
  const env = { ...process.env, IDX_NPM_CACHE: fakeCache("trap 'echo term; exit 3' TERM\necho ready\nwhile :; do sleep 0.05; done\n") };
  delete env.IDX_VERSION;
  const child = spawn(process.execPath, [stage("npm-mcp", "ihsg-mcp.js")], { env, stdio: ["ignore", "pipe", "pipe"] });
  let out = "";
  await new Promise((resolve) => child.stdout.on("data", (d) => {
    out += d;
    if (out.includes("ready")) resolve();
  }));
  child.kill("SIGTERM");
  const [code] = await once(child, "exit");
  assert.equal(code, 3);
  assert.equal(out, "ready\nterm\n");

  // idx ended by a signal: the launcher ends by the same signal.
  env.IDX_NPM_CACHE = fakeCache("kill -TERM $$\n");
  const r = spawnSync(process.execPath, [stage("npm-mcp", "ihsg-mcp.js")], { env, encoding: "utf8" });
  assert.equal(r.signal, "SIGTERM");
  assert.equal(r.stdout, "");
});

test("a failed first-run download says why on stderr and leaves stdout empty", () => {
  const env = { ...process.env, IDX_NPM_CACHE: mkdtempSync(join(tmpdir(), "ihsg-npm-")), IDX_VERSION: "0.0.0" };
  const r = spawnSync(process.execPath, [stage("npm-mcp", "ihsg-mcp.js")], { env, encoding: "utf8" });
  assert.equal(r.status, 1);
  assert.equal(r.stdout, "");
  assert.match(r.stderr, /download failed: development package version 0\.0\.0/);
  assert.match(r.stderr, /register `idx mcp` instead/);
});
