#!/usr/bin/env node
// Launcher: runs the idx binary with the same arguments and exit code, so
// `npx ihsg-cli stocks quote BBCA` and
// `claude mcp add idx -- npx -y ihsg-cli mcp` both work.
//
// If postinstall did not run (npm 12 blocks dependency scripts by default)
// the binary is downloaded and SHA-256 verified here, once, before the first
// command. Progress goes to stderr so stdout stays clean for JSON and for
// the MCP protocol.
"use strict";
const { spawnSync } = require("child_process");
const binary = require("../lib/binary");

async function main() {
  let bin = binary.installed();
  if (!bin) {
    try {
      bin = await binary.download((line) => console.error(line));
    } catch (e) {
      // A concurrent first run may have installed it in the meantime.
      bin = binary.installed();
      if (!bin) {
        console.error(`idx: binary missing and download failed: ${e.message}`);
        console.error(`idx: install it manually: curl -fsSL https://raw.githubusercontent.com/${binary.REPO}/main/install.sh | sh`);
        return 1;
      }
    }
  }
  const r = spawnSync(bin, process.argv.slice(2), { stdio: "inherit" });
  if (r.error) {
    console.error(`idx: ${r.error.message}`);
    return 1;
  }
  return r.status === null ? 1 : r.status;
}

main().then((code) => process.exit(code));
