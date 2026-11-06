#!/usr/bin/env node
// ihsg-mcp: `npx -y ihsg-mcp` runs `idx mcp`, the idx MCP server, without
// installing idx first. Every argument goes to `idx mcp`:
//
//   npx -y ihsg-mcp                          stdio server (what agents launch)
//   npx -y ihsg-mcp --http 127.0.0.1:8765    streamable HTTP for URL-only clients
//   npx -y ihsg-mcp setup cursor             the registration recipe, in the npx form
//
// The first run downloads the idx binary for this platform from the matching
// GitHub release and checks it against the release's SHA256SUMS
// (lib/binary.js, byte-identical to ihsg-cli's; a test compares them). stdout
// is the MCP channel, so every line this launcher prints goes to stderr.
"use strict";
const { spawn } = require("child_process");
const binary = require("../lib/binary");

/** `idx mcp <args>`. A bare `setup` would print the path of the binary in
 *  npx's cache, which npx may delete; ask for the `npx -y ihsg-mcp` form
 *  unless the caller chose a form (`--npx`, or `--http URL` for remote). */
function mcpArgs(argv) {
  const a = argv.slice(2);
  const http = a.some((x) => x === "--http" || x.startsWith("--http="));
  if (a.includes("setup") && !a.includes("--npx") && !http) a.push("--npx");
  return ["mcp", ...a];
}

async function main() {
  let bin = binary.installed();
  if (!bin) {
    try {
      bin = await binary.download((line) => console.error(line));
    } catch (e) {
      // A concurrent first start may have installed it in the meantime.
      bin = binary.installed();
      if (!bin) {
        console.error(`ihsg-mcp: the idx binary is missing and the download failed: ${e.message}`);
        console.error(`ihsg-mcp: install idx another way (https://github.com/${binary.REPO}#install) and register \`idx mcp\` instead`);
        process.exit(1);
      }
    }
  }
  const child = spawn(bin, mcpArgs(process.argv), { stdio: "inherit" });
  // A supervisor stopping the launcher stops the server too. Ctrl+C already
  // reaches the child through the terminal, so SIGINT only keeps node alive
  // until the child has exited. (During the first-run download no handler
  // is installed yet: a signal then ends the launcher before anything runs.)
  for (const sig of ["SIGTERM", "SIGHUP"]) process.on(sig, () => child.kill(sig));
  process.on("SIGINT", () => {});
  child.on("error", (e) => {
    console.error(`ihsg-mcp: cannot start ${bin}: ${e.message}`);
    process.exit(1);
  });
  child.on("exit", (code, signal) => {
    if (signal) {
      // End the way the child did, so the caller sees the same signal.
      process.removeAllListeners(signal);
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code === null ? 1 : code);
  });
}

if (require.main === module) main();
module.exports = { mcpArgs };
