# ihsg-cli

npm wrapper for [idx](https://github.com/fzn0x/idx), the Indonesian stock
market (IDX / IHSG) CLI and MCP server for people and AI agents. The prebuilt
binary for your platform is downloaded from the matching GitHub release and
verified against the release's `SHA256SUMS`: at `postinstall` when npm runs
install scripts, or on the first command when it does not (npm 12 disables
dependency scripts by default; `--ignore-scripts` and some CI images do too).
Set `IDX_SKIP_DOWNLOAD=1` to always defer to first run.

```sh
npx ihsg-cli stocks quote BBCA
npm i -g ihsg-cli && idx stocks brief TLKM

# MCP server for any agent, without a global install (the ihsg-mcp package)
claude mcp add idx -- npx -y ihsg-mcp
npx -y ihsg-mcp setup --list      # Codex, Muse Code, Cursor, Copilot, Gemini, ...
```

The package installs two commands that run the same binary: `idx` and
`ihsg-cli`. For agents only, the `ihsg-mcp` package runs `idx mcp` and shares
this package's downloader (and its per-user cache). Platforms: Linux x64/arm64
(static musl), macOS x64/arm64, Windows x64. Prefer `cargo install ihsg-cli`
or `install.sh` if you do not want Node involved. MIT.
