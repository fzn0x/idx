//! `idx mcp setup <client>`: the exact registration command or config
//! snippet for every agent harness we know, filled in with the absolute path
//! of the running binary (GUI apps rarely inherit a shell PATH).
//!
//! Formats were checked against each vendor's documentation in October 2026;
//! `docs/mcp.md` carries the same material with links. Keep the two in step.

use serde::Serialize;
use serde_json::json;

/// How the client should launch or reach idx.
#[derive(Debug, Clone)]
pub enum Launcher {
    /// Absolute path of the idx binary, run as `idx mcp`.
    Binary(String),
    /// `npx -y ihsg-cli mcp` (no install step).
    Npx,
    /// A running `idx mcp --http` endpoint, usually behind a tunnel.
    Http { url: String, token: Option<String> },
}

impl Launcher {
    fn command(&self) -> (String, Vec<String>) {
        match self {
            Launcher::Binary(p) => (p.clone(), vec!["mcp".into()]),
            Launcher::Npx => (
                "npx".into(),
                vec!["-y".into(), "ihsg-cli".into(), "mcp".into()],
            ),
            Launcher::Http { .. } => (String::new(), vec![]),
        }
    }
    fn shell(&self) -> String {
        let (c, a) = self.command();
        std::iter::once(c)
            .chain(a)
            .map(|s| {
                if s.contains(' ') {
                    format!("\"{s}\"")
                } else {
                    s
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    fn is_http(&self) -> bool {
        matches!(self, Launcher::Http { .. })
    }
    fn headers_json(&self) -> serde_json::Value {
        match self {
            Launcher::Http { token: Some(t), .. } => {
                json!({ "Authorization": format!("Bearer {t}") })
            }
            _ => json!({}),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Terminal agent with its own `mcp add` command or config file.
    Cli,
    /// IDE or editor extension.
    Ide,
    /// Desktop or terminal app configured through a settings screen.
    App,
    /// Hosted product that only connects to a URL.
    Remote,
}

#[derive(Debug, Clone, Serialize)]
pub struct Client {
    pub id: &'static str,
    pub aliases: &'static [&'static str],
    pub name: &'static str,
    pub kind: Kind,
    pub blurb: &'static str,
}

pub const CLIENTS: &[Client] = &[
    Client {
        id: "claude-code",
        aliases: &["claude"],
        name: "Claude Code",
        kind: Kind::Cli,
        blurb: "claude mcp add, or .mcp.json in the project",
    },
    Client {
        id: "claude-desktop",
        aliases: &["desktop"],
        name: "Claude Desktop",
        kind: Kind::App,
        blurb: "claude_desktop_config.json, or Settings > Connectors for a URL",
    },
    Client {
        id: "codex",
        aliases: &["openai"],
        name: "OpenAI Codex (CLI, IDE extension, app)",
        kind: Kind::Cli,
        blurb: "codex mcp add, or ~/.codex/config.toml",
    },
    Client {
        id: "muse",
        aliases: &["muse-code", "meta"],
        name: "Meta Muse Code",
        kind: Kind::Cli,
        blurb: "~/.config/muse/settings.json with schema_version 1",
    },
    Client {
        id: "antigravity",
        aliases: &["google"],
        name: "Google Antigravity (IDE and CLI)",
        kind: Kind::Cli,
        blurb: "~/.gemini/config/mcp_config.json, absolute command path",
    },
    Client {
        id: "gemini",
        aliases: &["gemini-cli"],
        name: "Gemini CLI",
        kind: Kind::Cli,
        blurb: "gemini mcp add -s user, or ~/.gemini/settings.json",
    },
    Client {
        id: "copilot-cli",
        aliases: &["copilot"],
        name: "GitHub Copilot CLI",
        kind: Kind::Cli,
        blurb: "copilot mcp add, or ~/.copilot/mcp-config.json",
    },
    Client {
        id: "vscode",
        aliases: &["code", "copilot-vscode"],
        name: "VS Code (Copilot agent mode)",
        kind: Kind::Ide,
        blurb: ".vscode/mcp.json, or code --add-mcp",
    },
    Client {
        id: "cursor",
        aliases: &[],
        name: "Cursor",
        kind: Kind::Ide,
        blurb: "~/.cursor/mcp.json or .cursor/mcp.json",
    },
    Client {
        id: "windsurf",
        aliases: &["codeium"],
        name: "Windsurf",
        kind: Kind::Ide,
        blurb: "~/.codeium/windsurf/mcp_config.json",
    },
    Client {
        id: "kiro",
        aliases: &["kiro-cli", "amazon-q", "q"],
        name: "Kiro (IDE and CLI, formerly Amazon Q Developer CLI)",
        kind: Kind::Cli,
        blurb: "~/.kiro/settings/mcp.json, or kiro-cli mcp add",
    },
    Client {
        id: "jetbrains",
        aliases: &["junie", "intellij"],
        name: "JetBrains AI Assistant / Junie",
        kind: Kind::Ide,
        blurb: "Settings > Tools > AI Assistant > MCP > + > As JSON",
    },
    Client {
        id: "zed",
        aliases: &[],
        name: "Zed",
        kind: Kind::Ide,
        blurb: "settings.json context_servers",
    },
    Client {
        id: "cline",
        aliases: &["roo", "roo-code", "kilo", "kilo-code"],
        name: "Cline / Roo Code / Kilo Code",
        kind: Kind::Ide,
        blurb: "MCP Servers > Configure (cline_mcp_settings.json / mcp_settings.json)",
    },
    Client {
        id: "continue",
        aliases: &[],
        name: "Continue",
        kind: Kind::Ide,
        blurb: "~/.continue/mcpServers/idx.yaml",
    },
    Client {
        id: "opencode",
        aliases: &[],
        name: "OpenCode",
        kind: Kind::Cli,
        blurb: "opencode.json mcp block, or opencode mcp add",
    },
    Client {
        id: "goose",
        aliases: &["block"],
        name: "Goose (Block)",
        kind: Kind::Cli,
        blurb: "goose configure, or ~/.config/goose/config.yaml",
    },
    Client {
        id: "warp",
        aliases: &[],
        name: "Warp",
        kind: Kind::App,
        blurb: "Settings > AI > MCP servers > + Add, or ~/.warp/.mcp.json",
    },
    Client {
        id: "droid",
        aliases: &["factory"],
        name: "Factory Droid",
        kind: Kind::Cli,
        blurb: "droid mcp add, or ~/.factory/mcp.json",
    },
    Client {
        id: "vibe",
        aliases: &["mistral", "mistral-vibe"],
        name: "Mistral Vibe",
        kind: Kind::Cli,
        blurb: "~/.vibe/config.toml [[mcp_servers]]",
    },
    Client {
        id: "qwen",
        aliases: &["qwen-code"],
        name: "Qwen Code",
        kind: Kind::Cli,
        blurb: "qwen mcp add, or ~/.qwen/settings.json",
    },
    Client {
        id: "crush",
        aliases: &["charm"],
        name: "Crush (Charm)",
        kind: Kind::Cli,
        blurb: "crush.json mcp block",
    },
    Client {
        id: "trae",
        aliases: &[],
        name: "Trae",
        kind: Kind::Ide,
        blurb: "AI Management > MCP > Add > Configure manually",
    },
    Client {
        id: "augment",
        aliases: &[],
        name: "Augment Code",
        kind: Kind::Ide,
        blurb: "Settings > MCP > Import from JSON",
    },
    Client {
        id: "chatgpt",
        aliases: &[
            "remote",
            "claude-web",
            "claude-ai",
            "perplexity",
            "devin",
            "manus",
        ],
        name: "ChatGPT, Claude.ai, Perplexity, Devin, Manus (remote only)",
        kind: Kind::Remote,
        blurb: "idx mcp --http behind a tunnel; add the URL as a connector",
    },
];

pub fn find(id: &str) -> Option<&'static Client> {
    let id = id.trim().to_ascii_lowercase();
    CLIENTS
        .iter()
        .find(|c| c.id == id || c.aliases.contains(&id.as_str()))
}

#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub title: String,
    /// Config file this snippet belongs in, if any.
    pub path: Option<String>,
    /// `sh`, `json`, `toml`, `yaml` or `text`.
    pub lang: &'static str,
    pub body: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Setup {
    pub client: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub launcher: String,
    pub steps: Vec<Step>,
    pub notes: Vec<String>,
}

fn pretty(v: serde_json::Value) -> String {
    serde_json::to_string_pretty(&v).unwrap_or_default()
}

fn step(title: &str, path: Option<&str>, lang: &'static str, body: String) -> Step {
    Step {
        title: title.to_string(),
        path: path.map(str::to_string),
        lang,
        body,
    }
}

/// `{"<key>": {"idx": <entry>}}` as pretty JSON.
fn wrapped(key: &str, entry: serde_json::Value) -> String {
    pretty(json!({ key: { "idx": entry } }))
}

fn stdio_entry(l: &Launcher, extra: serde_json::Value) -> serde_json::Value {
    let (c, a) = l.command();
    let mut v = json!({ "command": c, "args": a });
    if let (Some(dst), Some(src)) = (v.as_object_mut(), extra.as_object()) {
        for (k, val) in src {
            dst.insert(k.clone(), val.clone());
        }
    }
    v
}

fn http_url(l: &Launcher) -> &str {
    match l {
        Launcher::Http { url, .. } => url,
        _ => "",
    }
}

/// The registration recipe for `client`.
pub fn setup(client: &'static Client, l: &Launcher) -> Setup {
    let mut steps: Vec<Step> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let http = l.is_http();
    let url = http_url(l).to_string();
    let headers = l.headers_json();
    let token_header = match l {
        Launcher::Http { token: Some(t), .. } => Some(format!("Authorization: Bearer {t}")),
        _ => None,
    };
    let sh = l.shell();

    match client.id {
        "claude-code" => {
            if http {
                let mut cmd = format!("claude mcp add --transport http --scope user idx {url}");
                if let Some(h) = &token_header {
                    cmd.push_str(&format!(" --header \"{h}\""));
                }
                steps.push(step("Register (all projects)", None, "sh", cmd));
            } else {
                steps.push(step(
                    "Register (all projects)",
                    None,
                    "sh",
                    format!("claude mcp add --scope user idx -- {sh}"),
                ));
                steps.push(step(
                    "Or commit it with the project",
                    Some(".mcp.json"),
                    "json",
                    wrapped("mcpServers", stdio_entry(l, json!({}))),
                ));
            }
            notes.push(
                "Check with `claude mcp list`; inside a session `/mcp` shows the 39 tools.".into(),
            );
        }
        "claude-desktop" => {
            if http {
                steps.push(step(
                    "Settings > Connectors > Add custom connector",
                    None,
                    "text",
                    format!(
                        "Name: idx\nURL: {url}{}",
                        token_header
                            .as_ref()
                            .map(|h| format!("\nHeader: {h}"))
                            .unwrap_or_default()
                    ),
                ));
            } else {
                steps.push(step(
                    "Edit the config file and restart Claude Desktop",
                    Some("macOS ~/Library/Application Support/Claude/claude_desktop_config.json · Windows %APPDATA%\\Claude\\claude_desktop_config.json"),
                    "json",
                    wrapped("mcpServers", stdio_entry(l, json!({}))),
                ));
            }
        }
        "codex" => {
            if http {
                steps.push(step(
                    "Register",
                    None,
                    "sh",
                    format!(
                        "codex mcp add idx --url {url}{}",
                        token_header
                            .as_ref()
                            .map(|_| " --bearer-token-env-var IDX_MCP_TOKEN")
                            .unwrap_or_default()
                    ),
                ));
                notes.push("Export IDX_MCP_TOKEN with the token before starting Codex.".into());
            } else {
                steps.push(step(
                    "Register",
                    None,
                    "sh",
                    format!("codex mcp add idx -- {sh}"),
                ));
                let (c, a) = l.command();
                steps.push(step(
                    "Or edit the shared config (CLI, IDE extension and app read the same file)",
                    Some("~/.codex/config.toml"),
                    "toml",
                    format!(
                        "[mcp_servers.idx]\ncommand = {}\nargs = {}",
                        toml_str(&c),
                        toml_arr(&a)
                    ),
                ));
            }
            notes.push("Project-scoped .codex/config.toml works in trusted projects; `codex mcp list` verifies.".into());
        }
        "muse" => {
            let entry = if http {
                json!({ "transport": "streamable_http", "url": url, "headers": headers, "mode": "optional" })
            } else {
                {
                    let (c, a) = l.command();
                    json!({ "transport": "stdio", "command": c, "args": a, "mode": "optional" })
                }
            };
            steps.push(step(
                "Edit the settings file (schema_version is mandatory)",
                Some("~/.config/muse/settings.json"),
                "json",
                pretty(json!({ "schema_version": 1, "mcp_servers": { "idx": entry } })),
            ));
            notes.push("`mode: optional` lets Muse start even if idx is missing; the default `required` aborts the run. `/mcp` in a session lists servers.".into());
        }
        "antigravity" => {
            let entry = if http {
                json!({ "serverUrl": url, "headers": headers })
            } else {
                stdio_entry(l, json!({}))
            };
            steps.push(step(
                "Edit the global file, or .agents/mcp_config.json in the workspace",
                Some("~/.gemini/config/mcp_config.json"),
                "json",
                wrapped("mcpServers", entry),
            ));
            notes.push("Antigravity does not inherit your shell PATH and rejects a `type` field: keep the absolute command path. `/mcp` opens the manager.".into());
        }
        "gemini" => {
            if http {
                steps.push(step(
                    "Register (user scope)",
                    None,
                    "sh",
                    format!(
                        "gemini mcp add -s user -t http idx {url}{}",
                        token_header
                            .as_ref()
                            .map(|h| format!(" -H \"{h}\""))
                            .unwrap_or_default()
                    ),
                ));
            } else {
                steps.push(step(
                    "Register (user scope)",
                    None,
                    "sh",
                    format!("gemini mcp add -s user idx {sh}"),
                ));
                steps.push(step(
                    "Or edit the settings file",
                    Some("~/.gemini/settings.json"),
                    "json",
                    wrapped("mcpServers", stdio_entry(l, json!({}))),
                ));
            }
            notes.push("Free-tier Gemini CLI was replaced by Antigravity CLI in June 2026; see `idx mcp setup antigravity`.".into());
        }
        "copilot-cli" => {
            if http {
                let mut cmd = format!("copilot mcp add --transport http idx {url}");
                if let Some(h) = &token_header {
                    cmd.push_str(&format!(" --header \"{h}\""));
                }
                steps.push(step("Register", None, "sh", cmd));
            } else {
                steps.push(step(
                    "Register",
                    None,
                    "sh",
                    format!("copilot mcp add idx -- {sh}"),
                ));
                steps.push(step(
                    "Or edit the user config",
                    Some("~/.copilot/mcp-config.json"),
                    "json",
                    wrapped(
                        "mcpServers",
                        stdio_entry(l, json!({ "type": "local", "tools": ["*"] })),
                    ),
                ));
            }
            notes.push("A committed .mcp.json (mcpServers) at the repo root is read too; `copilot mcp list` verifies.".into());
        }
        "vscode" => {
            if http {
                steps.push(step(
                    "Workspace config",
                    Some(".vscode/mcp.json"),
                    "json",
                    wrapped(
                        "servers",
                        json!({ "type": "http", "url": url, "headers": headers }),
                    ),
                ));
            } else {
                let (c, a) = l.command();
                // Forward slashes keep the one-liner free of double escaping on Windows.
                let c = c.replace('\\', "/");
                steps.push(step(
                    "Add to your user profile",
                    None,
                    "sh",
                    format!(
                        "code --add-mcp \"{}\"",
                        json!({ "name": "idx", "command": c, "args": a })
                            .to_string()
                            .replace('"', "\\\"")
                    ),
                ));
                steps.push(step(
                    "Or workspace config",
                    Some(".vscode/mcp.json"),
                    "json",
                    wrapped("servers", stdio_entry(l, json!({ "type": "stdio" }))),
                ));
            }
            notes.push("Agent mode > Tools shows idx_* once the server starts. A portable .mcp.json (mcpServers) also works.".into());
        }
        "cursor" => {
            let entry = if http {
                json!({ "url": url, "headers": headers })
            } else {
                stdio_entry(l, json!({}))
            };
            steps.push(step(
                "Global, or .cursor/mcp.json in a project",
                Some("~/.cursor/mcp.json"),
                "json",
                wrapped("mcpServers", entry),
            ));
            notes.push(
                "Cursor Settings > MCP & Integrations lists the server and its tools.".into(),
            );
        }
        "windsurf" => {
            let entry = if http {
                json!({ "serverUrl": url, "headers": headers })
            } else {
                stdio_entry(l, json!({}))
            };
            steps.push(step(
                "Edit the config file",
                Some("~/.codeium/windsurf/mcp_config.json"),
                "json",
                wrapped("mcpServers", entry),
            ));
        }
        "kiro" => {
            if http {
                steps.push(step(
                    "Edit the config file",
                    Some("~/.kiro/settings/mcp.json"),
                    "json",
                    wrapped("mcpServers", json!({ "url": url, "headers": headers })),
                ));
            } else {
                steps.push(step(
                    "Edit the user file, or .kiro/settings/mcp.json in the workspace",
                    Some("~/.kiro/settings/mcp.json"),
                    "json",
                    wrapped("mcpServers", stdio_entry(l, json!({ "disabled": false }))),
                ));
                let (c, a) = l.command();
                steps.push(step(
                    "Or from the CLI",
                    None,
                    "sh",
                    format!(
                        "kiro-cli mcp add --name idx --command {} {}",
                        shell_quote(&c),
                        a.iter()
                            .map(|x| format!("--args {x}"))
                            .collect::<Vec<_>>()
                            .join(" ")
                    ),
                ));
            }
            notes.push("Amazon Q Developer CLI became Kiro CLI; older installs use `q mcp add` with the same flags and ~/.aws/amazonq/mcp.json.".into());
        }
        "jetbrains" => {
            let entry = if http {
                json!({ "url": url, "headers": headers })
            } else {
                stdio_entry(l, json!({}))
            };
            steps.push(step(
                "Settings > Tools > AI Assistant > Model Context Protocol (MCP) > + > As JSON",
                None,
                "json",
                wrapped("mcpServers", entry),
            ));
            notes.push("Applies to AI Assistant, Junie and Air in any JetBrains IDE.".into());
        }
        "zed" => {
            let entry = if http {
                json!({ "url": url, "headers": headers })
            } else {
                stdio_entry(l, json!({ "env": {} }))
            };
            steps.push(step(
                "Settings > AI > MCP Servers > Add Server, or edit settings.json",
                Some("~/.config/zed/settings.json"),
                "json",
                wrapped("context_servers", entry),
            ));
        }
        "cline" => {
            let entry = if http {
                json!({ "type": "streamableHttp", "url": url, "headers": headers, "disabled": false })
            } else {
                stdio_entry(l, json!({ "type": "stdio", "disabled": false }))
            };
            steps.push(step(
                "MCP Servers > Configure MCP Servers (opens the JSON)",
                Some("cline_mcp_settings.json (Cline) · mcp_settings.json (Roo Code, Kilo Code)"),
                "json",
                wrapped("mcpServers", entry),
            ));
        }
        "continue" => {
            let (c, a) = l.command();
            let body = if http {
                format!("name: idx\nversion: 0.0.1\nschema: v1\nmcpServers:\n  - name: idx\n    type: streamable-http\n    url: {url}{}", token_header.as_ref().map(|h| format!("\n    requestOptions:\n      headers:\n        {}", h)).unwrap_or_default())
            } else {
                format!("name: idx\nversion: 0.0.1\nschema: v1\nmcpServers:\n  - name: idx\n    command: {}\n    args: {}", yaml_str(&c), yaml_arr(&a))
            };
            steps.push(step(
                "Create the block file",
                Some("~/.continue/mcpServers/idx.yaml"),
                "yaml",
                body,
            ));
        }
        "opencode" => {
            let entry = if http {
                json!({ "type": "remote", "url": url, "headers": headers, "enabled": true })
            } else {
                let (c, a) = l.command();
                let mut cmd = vec![c];
                cmd.extend(a);
                json!({ "type": "local", "command": cmd, "enabled": true })
            };
            steps.push(step("Global config (or opencode.json in the project)", Some("~/.config/opencode/opencode.json"), "json", pretty(json!({ "$schema": "https://opencode.ai/config.json", "mcp": { "idx": entry } }))));
            notes.push("`opencode mcp add` walks through the same fields interactively; `opencode mcp list` verifies.".into());
        }
        "goose" => {
            if http {
                steps.push(step("Edit the config file", Some("~/.config/goose/config.yaml"), "yaml", format!("extensions:\n  idx:\n    enabled: true\n    type: streamable_http\n    name: idx\n    uri: {url}\n    timeout: 300{}", token_header.as_ref().map(|h| { let (k, v) = h.split_once(": ").unwrap_or(("Authorization", "")); format!("\n    headers:\n      {k}: {v}") }).unwrap_or_default())));
            } else {
                let (c, a) = l.command();
                steps.push(step("Interactive", None, "sh", format!("goose configure   # Add Extension > Command-line Extension > name: idx, command: {sh}")));
                steps.push(step("Or edit the config file", Some("~/.config/goose/config.yaml"), "yaml", format!("extensions:\n  idx:\n    enabled: true\n    type: stdio\n    name: idx\n    cmd: {}\n    args: {}\n    timeout: 300", yaml_str(&c), yaml_arr(&a))));
            }
        }
        "warp" => {
            let entry = if http {
                json!({ "url": url, "headers": headers })
            } else {
                stdio_entry(l, json!({}))
            };
            steps.push(step(
                "Settings > AI > Manage MCP servers > + Add (paste), or the file",
                Some("~/.warp/.mcp.json"),
                "json",
                wrapped("mcpServers", entry),
            ));
        }
        "droid" => {
            if http {
                steps.push(step(
                    "Register",
                    None,
                    "sh",
                    format!("droid mcp add idx {url} --type http"),
                ));
            } else {
                steps.push(step(
                    "Register",
                    None,
                    "sh",
                    format!("droid mcp add idx \"{sh}\""),
                ));
                steps.push(step(
                    "Or edit the user file (.factory/mcp.json for a project)",
                    Some("~/.factory/mcp.json"),
                    "json",
                    wrapped("mcpServers", stdio_entry(l, json!({ "type": "stdio" }))),
                ));
            }
            notes.push(
                "`droid mcp list` verifies; `/mcp` inside a session opens the manager.".into(),
            );
        }
        "vibe" => {
            let body = if http {
                format!(
                    "[[mcp_servers]]\nname = \"idx\"\ntransport = \"streamable-http\"\nurl = {}{}",
                    toml_str(&url),
                    token_header
                        .as_ref()
                        .map(|h| {
                            let (k, v) = h.split_once(": ").unwrap_or(("Authorization", ""));
                            format!("\nheaders = {{ {k} = {} }}", toml_str(v))
                        })
                        .unwrap_or_default()
                )
            } else {
                let (c, a) = l.command();
                format!("[[mcp_servers]]\nname = \"idx\"\ntransport = \"stdio\"\ncommand = {}\nargs = {}", toml_str(&c), toml_arr(&a))
            };
            steps.push(step(
                "Edit the user config (or ./.vibe/config.toml in a trusted project)",
                Some("~/.vibe/config.toml"),
                "toml",
                body,
            ));
            notes.push("Tools appear as idx_<tool>, e.g. idx_idx_brief.".into());
        }
        "qwen" => {
            if http {
                steps.push(step(
                    "Register (user scope)",
                    None,
                    "sh",
                    format!(
                        "qwen mcp add --scope user --transport http idx {url}{}",
                        token_header
                            .as_ref()
                            .map(|h| format!(" -H \"{h}\""))
                            .unwrap_or_default()
                    ),
                ));
            } else {
                steps.push(step(
                    "Register (user scope is the default)",
                    None,
                    "sh",
                    format!("qwen mcp add idx {sh}"),
                ));
                steps.push(step(
                    "Or edit the settings file",
                    Some("~/.qwen/settings.json"),
                    "json",
                    wrapped("mcpServers", stdio_entry(l, json!({}))),
                ));
            }
        }
        "crush" => {
            let entry = if http {
                json!({ "type": "http", "url": url, "headers": headers })
            } else {
                stdio_entry(l, json!({ "type": "stdio" }))
            };
            steps.push(step(
                "Project crush.json, or the global file",
                Some("~/.config/crush/crush.json"),
                "json",
                pretty(
                    json!({ "$schema": "https://charm.land/crush.json", "mcp": { "idx": entry } }),
                ),
            ));
        }
        "trae" => {
            let entry = if http {
                json!({ "url": url, "headers": headers })
            } else {
                stdio_entry(l, json!({}))
            };
            steps.push(step(
                "AI Management > MCP > Add > Configure manually (paste)",
                None,
                "json",
                wrapped("mcpServers", entry),
            ));
        }
        "augment" => {
            let entry = if http {
                json!({ "url": url, "headers": headers })
            } else {
                stdio_entry(l, json!({}))
            };
            steps.push(step(
                "Augment panel > Settings > MCP > Import from JSON (paste)",
                None,
                "json",
                wrapped("mcpServers", entry),
            ));
        }
        "chatgpt" => {
            let token = match l {
                Launcher::Http { token: Some(t), .. } => t.clone(),
                _ => "<token>".to_string(),
            };
            if !http {
                steps.push(step(
                    "1. Serve idx over HTTP on this machine",
                    None,
                    "sh",
                    format!(
                        "export IDX_MCP_TOKEN={token}\n{} --http 127.0.0.1:8765 --allowed-host '*'",
                        match l {
                            Launcher::Binary(p) => format!("{p} mcp"),
                            _ => "idx mcp".into(),
                        }
                    ),
                ));
                steps.push(step("2. Expose it with a tunnel (any HTTPS tunnel works)", None, "sh", "cloudflared tunnel --url http://127.0.0.1:8765   # prints https://<name>.trycloudflare.com".into()));
            }
            steps.push(step(
                "3. Add the connector in the product",
                None,
                "text",
                format!(
                    "URL: {}\nHeader: Authorization: Bearer {token}\n\nChatGPT: Settings > Apps & Connectors > Advanced > Developer mode > Create\nClaude.ai / Claude mobile: Settings > Connectors > Add custom connector\nPerplexity: Settings > Connectors > Add; Devin: Settings > MCP Marketplace > Add custom; Manus: Settings > Connectors",
                    if http { url.clone() } else { "https://<name>.trycloudflare.com/mcp".into() }
                ),
            ));
            notes.push("These products never launch local processes, so idx has to be reachable over HTTPS. Keep the token: the endpoint exposes your machine's network access to Yahoo and friends.".into());
        }
        _ => {}
    }

    Setup {
        client: client.id,
        name: client.name,
        kind: client.kind,
        launcher: if http { url } else { sh },
        steps,
        notes,
    }
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}
fn toml_arr(a: &[String]) -> String {
    format!(
        "[{}]",
        a.iter().map(|s| toml_str(s)).collect::<Vec<_>>().join(", ")
    )
}
fn yaml_str(s: &str) -> String {
    toml_str(s)
}
fn yaml_arr(a: &[String]) -> String {
    toml_arr(a)
}
fn shell_quote(s: &str) -> String {
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

/// Human-readable rendering of a setup.
pub fn render(s: &Setup) -> String {
    let mut out = format!("{} · idx MCP setup\n", s.name);
    for st in &s.steps {
        out.push('\n');
        out.push_str(&st.title);
        if let Some(p) = &st.path {
            out.push_str(&format!("  [{p}]"));
        }
        out.push('\n');
        for line in st.body.lines() {
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if !s.notes.is_empty() {
        out.push('\n');
        for n in &s.notes {
            out.push_str(&format!("Note: {n}\n"));
        }
    }
    out.push_str("\nVerify: ask the agent \"use idx_brief on BBCA\". The server lists 39 tools; idx_brief is the one-call starting point.\n");
    out
}

/// The client table for `--list`.
pub fn list_text() -> String {
    let mut out = String::from("client            kind    name\n");
    for c in CLIENTS {
        out.push_str(&format!(
            "{:<17} {:<7} {}{}\n",
            c.id,
            match c.kind {
                Kind::Cli => "cli",
                Kind::Ide => "ide",
                Kind::App => "app",
                Kind::Remote => "remote",
            },
            c.name,
            if c.aliases.is_empty() {
                String::new()
            } else {
                format!("  (also: {})", c.aliases.join(", "))
            }
        ));
    }
    out.push_str("\nUsage: idx mcp setup <client> [--npx] [--http URL [--token T]]\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_client_renders_with_the_binary_path() {
        let l = Launcher::Binary("/opt/idx/bin/idx".into());
        for c in CLIENTS {
            let s = setup(c, &l);
            assert!(!s.steps.is_empty(), "{} has no steps", c.id);
            let text = render(&s);
            assert!(text.contains("idx"), "{}", c.id);
            if c.kind != Kind::Remote {
                assert!(
                    text.contains("/opt/idx/bin/idx"),
                    "{} must carry the absolute path:\n{text}",
                    c.id
                );
            }
            // Every JSON snippet must parse.
            for st in &s.steps {
                if st.lang == "json" {
                    serde_json::from_str::<serde_json::Value>(&st.body)
                        .unwrap_or_else(|e| panic!("{}: invalid JSON {e}\n{}", c.id, st.body));
                }
            }
        }
        assert!(find("Claude").is_some());
        assert!(find("roo").is_some());
        assert!(find("nope").is_none());
    }

    #[test]
    fn muse_and_codex_match_their_documented_shapes() {
        let l = Launcher::Binary("C:\\Tools\\idx.exe".into());
        let muse = setup(find("muse").unwrap(), &l);
        let v: serde_json::Value = serde_json::from_str(&muse.steps[0].body).unwrap();
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["mcp_servers"]["idx"]["transport"], "stdio");
        assert_eq!(v["mcp_servers"]["idx"]["command"], "C:\\Tools\\idx.exe");
        let codex = setup(find("codex").unwrap(), &l);
        assert!(codex.steps[1].body.starts_with(
            "[mcp_servers.idx]\ncommand = \"C:\\\\Tools\\\\idx.exe\"\nargs = [\"mcp\"]"
        ));
        let http = Launcher::Http {
            url: "https://x.trycloudflare.com/mcp".into(),
            token: Some("s3cret".into()),
        };
        let cursor = setup(find("cursor").unwrap(), &http);
        let v: serde_json::Value = serde_json::from_str(&cursor.steps[0].body).unwrap();
        assert_eq!(
            v["mcpServers"]["idx"]["url"],
            "https://x.trycloudflare.com/mcp"
        );
        assert_eq!(
            v["mcpServers"]["idx"]["headers"]["Authorization"],
            "Bearer s3cret"
        );
        let npx = setup(find("claude-code").unwrap(), &Launcher::Npx);
        assert!(npx.steps[0].body.contains("npx -y ihsg-cli mcp"));
    }
}
