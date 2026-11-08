# Using idx from an agent

This page is for AI agents and the people who set them up. idx answers
questions about stocks listed on the Indonesia Stock Exchange (IDX, the IHSG
composite): quotes, history, fundamentals, technicals, dividends, broker
activity, the KSEI register of holders above 1 % and news. An agent reaches
it two ways:

- **MCP.** `idx mcp` serves the commands as typed tools. This suits
  chat-style agents; [mcp.md](mcp.md) has the registration for each client.
- **The CLI.** For agents that run shell commands: add `-o json` and parse
  stdout.

Both return the same JSON envelope, so what an agent learns from one applies
to the other.

## Rules for calling idx

- Tickers are IDX codes such as `BBCA` or `TLKM`. The `.JK` suffix is
  optional; `^JKSE` is the IHSG composite.
- Start with `idx stocks brief <SYMBOL>` (MCP: `idx_brief`): quote,
  fundamentals with a verdict, technicals with a signal, returns against
  IHSG, dividends, top holders and headlines in one call.
- With `-o json`, stdout holds exactly one JSON document; diagnostics go to
  stderr. Parse the JSON rather than the tables.
- Check `stale` and `warnings` in the envelope before quoting a number.
  `stale: true` means a cached copy older than its lifetime was served
  because the network failed, the source's bot protection refused the
  request, or `--offline` was set.
- A field the source did not publish is `null`. Say the value is
  unavailable; do not estimate it.
- The fundamentals verdict, the technical signal, news sentiment and the
  bandarmology readings are rule-based heuristics, and each lists the rules
  that fired. Repeat them as heuristics, not as advice.
- Prices from the free sources are delayed as published.
- `idx schema <name>` prints the JSON Schema of a payload;
  [json-output.md](json-output.md) documents every field.

## Exit codes

| exit | `error.code` | what to do |
| --- | --- | --- |
| 0 | | success |
| 1 | `internal`, `io` | unexpected; report it |
| 2 | `usage` | fix the command, a flag or the symbol |
| 3 | `network`, `provider`, `blocked` | a source failed or refused; retry later. `blocked`: its bot protection (Cloudflare) refused the request; for idx.co.id's data a challenged request is already retried for 30 s and a cached copy is served instead when there is one, so the error means none was. It usually passes within a minute: retry then, and from another network if it keeps refusing (data centres, VPNs and CI runners are refused more often), or take the alternative the message names (`stocks foreign --source ipot`, `bandar screen --period 1d --sort fnet`, `market brokers --source ipot` for IPOT's latest session); for the ownership register, the GitHub mirror |
| 4 | `not_found` | the symbol or record does not exist |
| 5 | `offline_miss` | `--offline` was set and nothing is cached |
| 6 | `config`, `unsupported` | fix the config, or drop `--provider` so the automatic chain answers |

Errors are one JSON object on stderr, for example
`{"error":{"code":"not_found","message":"..."},"exit_code":4}`. Over MCP the
same object comes back as a tool error (`isError: true`).

## Recipes

```sh
idx -o json stocks quote BBCA,BBRI,TLKM | jq '.data[] | {symbol, price, change_pct}'
idx -o json stocks fundamental BBCA | jq '.data.verdict'
idx -o json stocks history TLKM --period 1y --interval 1wk | jq '.data.summary'
idx -o json stocks screen -f 'pe<10,div>5,mcap>5T' --sort div
idx -o json bandar detect BBRI --period 1m
idx -o json ownership ticker BBCA       # after the import below
```

The KSEI register of holders above 1 % lives in a local SQLite database.
Import it once, and again when a new month is out:

```sh
idx ownership import --latest
```

## Agent-led install

A person can hand this section to a coding agent: "Install idx and register
its MCP server with yourself by following
https://ihsg.midoriresearch.com/docs/agent-guide.md".

1. Do not use `sudo`; install into the user's home directory. Before
   changing a file outside the current project, show the user the change.
2. Install the binary with the first option that works:
   - Linux or macOS: `curl -fsSL https://raw.githubusercontent.com/fzn0x/idx/main/install.sh | sh`
     (checks the SHA-256, installs into `~/.local/bin`).
   - Windows: download `idx-<version>-x86_64-pc-windows-msvc.zip` from
     https://github.com/fzn0x/idx/releases and put `idx.exe` on `PATH`.
   - With Node.js: `npm install -g ihsg-cli`.
   - With a Rust toolchain: `cargo install ihsg-cli --locked`.
   - With Nix (Linux): `nix profile install github:fzn0x/idx`.
3. Check it: `idx version`, then `idx -o json stocks quote BBCA`. Exit 0 is
   success; exit 3 means the network or a source failed.
4. Register the MCP server: `idx mcp setup --list` names the supported
   clients, and `idx mcp setup <client>` prints the exact command or config
   snippet with the binary's absolute path (`-o json` gives the steps as
   data). Merge into existing config files; never replace them.
5. Optionally fill the ownership register: `idx ownership sync` (one download
   of the prebuilt database, or the monthly files when there is none yet).
6. Tell the user what was installed and how, which file changed, and whether
   the agent needs a restart to load the tools. Suggest a first question,
   such as "Use idx_brief on BBCA".

## Machine-readable docs

- https://ihsg.midoriresearch.com/llms.txt indexes these pages.
- https://ihsg.midoriresearch.com/llms-full.txt has every page in one file.
- Every page is Markdown at its `.md` address, for example
  https://ihsg.midoriresearch.com/docs/mcp.md; drop the `.md` for HTML.
