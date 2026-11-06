//! End-to-end check of `idx mcp` over stdio: JSON-RPC framing, initialize,
//! tools/list and a tools/call, without any network (IDX_OFFLINE=1).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn spawn_server(dir: &std::path::Path) -> std::process::Child {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("idx"));
    cmd.arg("mcp")
        .env_clear()
        .env("IDX_CACHE_DIR", dir.join("cache"))
        .env("IDX_CONFIG", dir.join("config.toml"))
        // The default session path sits beside the default config file, not
        // IDX_CONFIG's; keep the Stockbit tools off the developer's real login.
        .env("IDX_STOCKBIT_SESSION", dir.join("stockbit-session.json"))
        .env("IDX_OFFLINE", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Ok(v) = std::env::var("SystemRoot") {
        cmd.env("SystemRoot", v);
    }
    cmd.spawn().expect("spawn idx mcp")
}

#[test]
fn mcp_server_speaks_json_rpc_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = spawn_server(dir.path());
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();

    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let next = || -> serde_json::Value {
        let line = rx
            .recv_timeout(Duration::from_secs(30))
            .expect("server response within 30s");
        serde_json::from_str(&line).expect("valid JSON line")
    };

    let init = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "idx-test", "version": "0"}
        }
    });
    writeln!(stdin, "{init}").unwrap();
    let r = next();
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["serverInfo"]["name"], "idx");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    assert!(r["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("idx -o json"));

    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();

    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    let r = next();
    assert_eq!(r["id"], 2);
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"idx_quote"), "{names:?}");
    assert!(names.contains(&"idx_fundamental"), "{names:?}");
    assert_eq!(names.len(), 38, "{names:?}");
    assert!(names.contains(&"idx_broker"), "{names:?}");
    assert!(names.contains(&"idx_bandar"), "{names:?}");
    assert!(names.contains(&"idx_bandar_screen"), "{names:?}");
    assert!(names.contains(&"idx_ownership_ticker"), "{names:?}");
    assert!(names.contains(&"idx_holidays"), "{names:?}");
    assert!(names.contains(&"idx_orderbook"), "{names:?}");
    assert!(names.contains(&"idx_stockbit_status"), "{names:?}");
    assert!(names.contains(&"idx_stockbit_login"), "{names:?}");
    assert!(names.contains(&"idx_stockbit_actions"), "{names:?}");
    assert!(names.contains(&"idx_stockbit_movements"), "{names:?}");
    let quote = tools.iter().find(|t| t["name"] == "idx_quote").unwrap();
    assert_eq!(
        quote["inputSchema"]["properties"]["symbols"]["type"],
        "string"
    );
    assert!(quote["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("symbols")));
    // Portable schemas: no nullable type arrays, formats or $schema keys,
    // which Gemini and OpenAI-strict harnesses reject.
    fn walk(name: &str, v: &serde_json::Value) {
        match v {
            serde_json::Value::Null => panic!("{name}: null value in schema"),
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    assert!(
                        !matches!(k.as_str(), "default" | "format" | "$schema"),
                        "{name}: {k} keyword"
                    );
                    if k == "type" {
                        assert!(!v.is_array(), "{name}: type array");
                    }
                    walk(name, v);
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|v| walk(name, v)),
            _ => {}
        }
    }
    for t in tools {
        walk(t["name"].as_str().unwrap(), &t["inputSchema"]);
    }
    let news = tools.iter().find(|t| t["name"] == "idx_news").unwrap();
    assert_eq!(
        news["inputSchema"]["properties"]["limit"]["type"],
        "integer"
    );
    assert_eq!(
        news["inputSchema"]["properties"]["company_only"]["type"],
        "boolean"
    );

    // Offline with an empty cache: a structured tool error, not a crash.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": {"name": "idx_quote", "arguments": {"symbols": "BBCA"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["id"], 3);
    assert_eq!(r["result"]["isError"], true);
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "offline_miss");
    assert_eq!(body["exit_code"], 5);

    // A bad symbol is a usage error, still structured.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 4, "method": "tools/call",
        "params": {"name": "idx_quote", "arguments": {"symbols": "BB$CA"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("usage"));

    // A Stockbit tool without a session: a usage error naming the login,
    // before any request (the sandbox has no session file).
    for (id, args) in [
        (5, serde_json::json!({"symbol": "BBCA"})),
        (6, serde_json::json!({"kind": "dividend", "days": 30})),
        (
            8,
            serde_json::json!({"symbol": "BBCA", "pages": 2, "source": "ksei"}),
        ),
    ] {
        let tool = if id == 8 {
            "idx_stockbit_movements"
        } else {
            "idx_stockbit_actions"
        };
        let call = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": tool, "arguments": args}
        });
        writeln!(stdin, "{call}").unwrap();
        let r = next();
        assert_eq!(r["id"], id);
        assert_eq!(r["result"]["isError"], true);
        let body: serde_json::Value =
            serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["error"]["code"], "usage", "{body}");
        assert!(
            body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("idx stockbit login"),
            "{body}"
        );
    }
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": "idx_stockbit_actions", "arguments": {"symbol": "BBCA", "kind": "nope"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown corporate action kind"));
    // Bad filters are usage errors before the session is even looked at.
    for (id, args, needle) in [
        (9, serde_json::json!({"holder": "RUDI"}), "HOLDER ID"),
        (10, serde_json::json!({"action": "short"}), "unknown action"),
        (
            11,
            serde_json::json!({"pages": 11}),
            "pages must be 1 to 10",
        ),
    ] {
        let call = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "idx_stockbit_movements", "arguments": args}
        });
        writeln!(stdin, "{call}").unwrap();
        let r = next();
        assert_eq!(r["id"], id);
        assert_eq!(r["result"]["isError"], true);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(
            text.contains("\"usage\"") && text.contains(needle),
            "{text}"
        );
    }
    // A calendar end past the last date chrono holds is a usage error, not
    // a panic that would take the server down.
    for (id, args, needle) in [
        (
            14,
            serde_json::json!({"days": 4_000_000_000u32}),
            "at most 366 days",
        ),
        (
            15,
            serde_json::json!({"from": "+262142-12-31"}),
            "out of range",
        ),
    ] {
        let call = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "idx_stockbit_actions", "arguments": args}
        });
        writeln!(stdin, "{call}").unwrap();
        let r = next();
        assert_eq!(r["id"], id);
        assert_eq!(r["result"]["isError"], true, "{r}");
        let body: serde_json::Value =
            serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["error"]["code"], "usage", "{body}");
        assert!(
            body["error"]["message"].as_str().unwrap().contains(needle),
            "{body}"
        );
    }

    // The sign-in tool refuses an unknown method before opening anything;
    // status carries no sign_in while none was started.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 12, "method": "tools/call",
        "params": {"name": "idx_stockbit_login", "arguments": {"method": "carrier-pigeon"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("unknown method"));
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 13, "method": "tools/call",
        "params": {"name": "idx_stockbit_status", "arguments": {}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["logged_in"], false);
    assert!(body.get("sign_in").is_none(), "{body}");
    let login = tools
        .iter()
        .find(|t| t["name"] == "idx_stockbit_login")
        .unwrap();
    for arg in ["method", "port", "force", "timeout"] {
        assert!(login["inputSchema"]["properties"][arg].is_object(), "{arg}");
    }

    drop(stdin);
    let status = child.wait().expect("server exits after stdin closes");
    assert!(status.success(), "exit {status:?}");
}
