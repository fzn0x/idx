//! End-to-end check of `idx mcp` over stdio: JSON-RPC framing, initialize,
//! tools/list and a tools/call, without any network (IDX_OFFLINE=1).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Puts a recorded response in the server's cache under its URL, so the
/// offline server answers from it as if it had been fetched.
fn seed(cache: &std::path::Path, url: &str, fixture: &str) {
    let body = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fixture),
    )
    .unwrap();
    idx::cache::Cache::new(cache).put(url, 200, &body).unwrap();
}

fn spawn_server(dir: &std::path::Path) -> std::process::Child {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("idx"));
    cmd.arg("mcp")
        .env_clear()
        .env("IDX_CACHE_DIR", dir.join("cache"))
        .env("IDX_CONFIG", dir.join("config.toml"))
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
    seed(
        &dir.path().join("cache"),
        "https://query1.finance.yahoo.com/v8/finance/chart/BBCA.JK?range=1d&interval=1m&includePrePost=false",
        "yahoo/chart_bbca_1d_1m.json",
    );
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
    assert_eq!(names.len(), 37, "{names:?}");
    assert!(names.contains(&"idx_intraday"), "{names:?}");
    assert!(names.contains(&"idx_broker"), "{names:?}");
    assert!(names.contains(&"idx_bandar"), "{names:?}");
    assert!(names.contains(&"idx_bandar_screen"), "{names:?}");
    assert!(names.contains(&"idx_ownership_ticker"), "{names:?}");
    assert!(names.contains(&"idx_holidays"), "{names:?}");
    assert!(names.contains(&"idx_orderbook"), "{names:?}");
    assert!(names.contains(&"idx_stockbit_status"), "{names:?}");
    assert!(names.contains(&"idx_stockbit_login"), "{names:?}");
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

    // Minute bars from the seeded chart; the quote is not cached, so it is a
    // warning and `quote` is null rather than a failed call.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": {"name": "idx_intraday", "arguments": {"symbol": "BBCA", "limit": 30}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_ne!(r["result"]["isError"], true, "{r}");
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["schema"], "idx.intraday.v1");
    assert_eq!(body["data"]["session"], "2026-11-05");
    assert_eq!(body["data"]["interval"], "1m");
    assert_eq!(body["data"]["bars"].as_array().unwrap().len(), 30);
    assert_eq!(body["data"]["summary"]["bars"], 333);
    assert_eq!(body["data"]["summary"]["bars_volume"], 155_142_100);
    assert_eq!(body["data"]["summary"]["vwap"], 5089.98);
    assert!(body["data"]["quote"].is_null());
    let warnings = body["warnings"].to_string();
    assert!(warnings.contains("no quote for BBCA"), "{warnings}");
    assert!(warnings.contains("truncated to the last 30"), "{warnings}");

    drop(stdin);
    let status = child.wait().expect("server exits after stdin closes");
    assert!(status.success(), "exit {status:?}");
}
