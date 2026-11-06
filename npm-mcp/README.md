# ihsg-mcp

The [idx](https://github.com/fzn0x/idx) MCP server for the Indonesian stock
market (IDX / IHSG), without installing idx first. `npx -y ihsg-mcp` runs
`idx mcp`: quotes, history, fundamentals, technicals, dividends, broker flow
and bandarmology, the KSEI register of holders above 1 % and news, as typed
tools that return the same JSON as `idx -o json`. Free public data, no API keys.

```sh
claude mcp add --scope user idx -- npx -y ihsg-mcp
codex mcp add idx -- npx -y ihsg-mcp
```

Most clients take the same entry in their MCP config:

```json
{ "mcpServers": { "idx": { "command": "npx", "args": ["-y", "ihsg-mcp"] } } }
```

On Windows use `"command": "cmd", "args": ["/c", "npx", "-y", "ihsg-mcp"]`
(clients start commands without a shell there, and `npx` is a `.cmd`
script); in a command line that is `claude mcp add --scope user idx -- cmd /c npx -y ihsg-mcp`.
In Git Bash, which turns a lone `/c` into `C:/`, start that line with
`MSYS_NO_PATHCONV=1` or write `cmd //c`. Keep the plain `npx` form in a
config file that macOS or Linux users share, such as a committed `.mcp.json`.

GUI apps that do not inherit your shell `PATH` (common on macOS with Node from
nvm, fnm, Volta or Homebrew) fail with `spawn npx ENOENT`: give them the
absolute path from `command -v npx` as `command` and an `env` block whose
`PATH` holds that folder (npx starts `node` through `PATH`).

`npx -y ihsg-mcp setup <client>` prints the exact command or file for Codex,
Muse Code, Cursor, Copilot, Gemini, Zed and the other supported clients, in
this npx form; `npx -y ihsg-mcp setup --list` names them.

Everything after the package name goes to `idx mcp`, and `IDX_*` environment
variables (from your shell or the client's `env` block) apply:
`npx -y ihsg-mcp --http 127.0.0.1:8765 --token T` serves streamable HTTP,
`npx -y ihsg-mcp --provider yahoo` picks a source.

The first start downloads the idx binary for your platform (about 7 MB) from
the matching GitHub release and checks it against the release's `SHA256SUMS`;
later starts reuse it. Download messages go to stderr: stdout carries only the
protocol. Recent npx versions ask the registry at each start and move to a
new release by themselves (the matching binary is fetched the same way); if
npx keeps an outdated copy, run `npx -y ihsg-mcp@latest`. Platforms: Linux
x64/arm64 (static musl), macOS x64/arm64, Windows x64; Node 18 or newer.

The full CLI is the `ihsg-cli` package or `cargo install ihsg-cli`. Tool list
and client details: https://github.com/fzn0x/idx/blob/main/docs/mcp.md. MIT,
by Midori Research.
