//! End-to-end check of `idx mcp` over stdio: JSON-RPC framing, initialize,
//! tools/list and a tools/call, without any network (IDX_OFFLINE=1, or a
//! loopback proxy that refuses every request).

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

fn spawn_server(dir: &std::path::Path) -> std::process::Child {
    spawn_server_with(
        dir,
        &[
            ("IDX_OFFLINE", "1"),
            // Every cached quote is older than its TTL: stale, served offline.
            ("IDX_QUOTE_TTL_SECS", "0"),
        ],
    )
}

fn spawn_server_with(dir: &std::path::Path, env: &[(&str, &str)]) -> std::process::Child {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin("idx"));
    cmd.arg("mcp")
        .env_clear()
        .env("IDX_CACHE_DIR", dir.join("cache"))
        .env("IDX_CONFIG", dir.join("config.toml"))
        .env("IDX_OWNERSHIP_DB", dir.join("ownership.sqlite"))
        .envs(env.iter().copied())
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Ok(v) = std::env::var("SystemRoot") {
        cmd.env("SystemRoot", v);
    }
    cmd.spawn().expect("spawn idx mcp")
}

/// Puts a recorded response in the server's cache under its URL.
fn seed(dir: &std::path::Path, url: &str, fixture: &str) {
    let body = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fixture),
    )
    .unwrap();
    idx::cache::Cache::new(dir.join("cache"))
        .put(url, 200, &body)
        .unwrap();
}

/// The JSON envelope inside a successful tool result.
fn tool_body(r: &serde_json::Value) -> serde_json::Value {
    assert_ne!(r["result"]["isError"], true, "{r}");
    serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// BBCA's recorded IPOT pages of 10-12..10-16 (all investors and foreign)
/// in the server's cache, under the URLs it asks for.
fn seed_ipot_week(dir: &std::path::Path) {
    let cache = idx::cache::Cache::new(dir.join("cache"));
    for day in ["12", "13", "14", "15", "16"] {
        for (fd, suffix) in [("all", ""), ("F", "_foreign")] {
            let body = std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/ipot")
                    .join(format!("brokersummary_bbca_202610{day}{suffix}.html")),
            )
            .unwrap();
            let url = format!(
                "https://www.indopremier.com/module/saham/include/data-brokersummary.php?code=BBCA&start=10/{day}/2026&end=10/{day}/2026&fd={fd}&board=all"
            );
            cache.put(&url, 200, &body).unwrap();
        }
    }
}

#[test]
fn mcp_server_speaks_json_rpc_over_stdio() {
    let dir = tempfile::tempdir().unwrap();
    seed(
        dir.path(),
        "https://query1.finance.yahoo.com/v7/finance/quote?symbols=BBCA.JK,BBRI.JK,TLKM.JK",
        "yahoo/quote_v7_bbca_bbri_tlkm.json",
    );
    seed(
        dir.path(),
        "https://query1.finance.yahoo.com/v7/finance/quote?symbols=WIKA.JK,KAEF.JK,BBCA.JK",
        "yahoo/quote_v7_wika_suspended_kaef_bbca.json",
    );
    seed_ipot_week(dir.path());
    seed(
        dir.path(),
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
    let instructions = r["result"]["instructions"].as_str().unwrap();
    assert!(instructions.contains("idx -o json"));
    assert!(instructions.contains("stale_reason"));
    assert!(
        instructions.contains("D05.SI"),
        "other markets are explained"
    );

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
    assert_eq!(names.len(), 44, "{names:?}");
    assert!(names.contains(&"idx_ownership_institutions"), "{names:?}");
    assert!(names.contains(&"idx_insights"), "{names:?}");
    assert!(names.contains(&"idx_intraday"), "{names:?}");
    assert!(names.contains(&"idx_broker"), "{names:?}");
    assert!(names.contains(&"idx_bandar"), "{names:?}");
    assert!(names.contains(&"idx_bandar_screen"), "{names:?}");
    assert!(names.contains(&"idx_ownership_ticker"), "{names:?}");
    for name in [
        "idx_ownership_search",
        "idx_ownership_rankings",
        "idx_ownership_graph",
    ] {
        assert!(names.contains(&name), "{names:?}");
    }
    assert!(names.contains(&"idx_ownership_composition"), "{names:?}");
    assert!(names.contains(&"idx_ownership_snapshot"), "{names:?}");
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
    let screen = tools.iter().find(|t| t["name"] == "idx_screen").unwrap();
    for arg in ["preset", "filter", "sort", "ascending", "returns", "limit"] {
        assert!(
            screen["inputSchema"]["properties"][arg].is_object(),
            "idx_screen.{arg}"
        );
    }
    assert_eq!(
        screen["inputSchema"]["properties"]["ascending"]["type"],
        "boolean"
    );
    let required = screen["inputSchema"]["required"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert!(
        !required.contains(&serde_json::json!("filter")),
        "filter is optional now: {required:?}"
    );
    let insights = tools.iter().find(|t| t["name"] == "idx_insights").unwrap();
    assert!(insights["inputSchema"]["required"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("symbol")));
    // The growth, valuation and risk lenses are an argument, not more tools.
    let fundamental = tools
        .iter()
        .find(|t| t["name"] == "idx_fundamental")
        .unwrap();
    assert_eq!(
        fundamental["inputSchema"]["properties"]["lens"]["type"],
        "string"
    );
    assert!(fundamental["description"]
        .as_str()
        .unwrap()
        .contains("idx.risk.v1"));
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

    // A lens with nothing cached is the same structured cache miss.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": {"name": "idx_fundamental", "arguments": {"symbol": "TLKM", "lens": "risk"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["id"], 5);
    assert_eq!(r["result"]["isError"], true);
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "offline_miss", "{body}");

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

    // An entity name without 3 letters or digits would match every holder.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 5, "method": "tools/call",
        "params": {"name": "idx_ownership_entity", "arguments": {"name": "!!!"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["id"], 5);
    assert_eq!(r["result"]["isError"], true);
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["exit_code"], 2, "{body}");
    assert!(body.to_string().contains("3 letters or digits"), "{body}");

    // A cached copy older than its TTL, served offline: stale, and why.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 6, "method": "tools/call",
        "params": {"name": "idx_quote", "arguments": {"symbols": "BBCA,BBRI,TLKM"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let body = tool_body(&next());
    assert_eq!(body["schema"], "idx.quotes.v1");
    assert_eq!(body["cached"], true);
    assert_eq!(body["stale"], true);
    assert_eq!(body["stale_reason"], "offline");
    assert_eq!(body["data"].as_array().unwrap().len(), 3);

    // WIKA, suspended since February 2025, is behind the latest session.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": "idx_quote", "arguments": {"symbols": "WIKA,KAEF,BBCA"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let body = tool_body(&next());
    let rows = body["data"].as_array().unwrap();
    assert_eq!(rows[0]["symbol"], "WIKA");
    assert_eq!(rows[0]["behind_session"], true);
    assert!(rows.iter().all(|r| r["behind_session"].is_boolean()));
    assert!(quote["description"]
        .as_str()
        .unwrap()
        .contains("behind_session"));

    // Another market's ticker: accepted by the Yahoo-backed tools (offline
    // here, so a cache miss rather than a usage error), refused by IDX-only ones.
    let body = |r: &serde_json::Value| -> serde_json::Value {
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 8, "method": "tools/call",
        "params": {"name": "idx_quote", "arguments": {"symbols": "D05.SI"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(body(&r)["error"]["code"], "offline_miss");
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": {"name": "idx_broker", "arguments": {"symbols": "D05.SI"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["result"]["isError"], true);
    let b = body(&r);
    assert_eq!(b["error"]["code"], "usage");
    assert!(b["error"]["message"]
        .as_str()
        .unwrap()
        .contains("another exchange (SI)"));

    // The new ownership tools answer from the (empty) sandbox database.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 10, "method": "tools/call",
        "params": {"name": "idx_ownership_search", "arguments": {"query": "salim"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["id"], 10);
    assert_eq!(r["result"]["isError"], true);
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "not_found", "{body}");
    let rankings = tools
        .iter()
        .find(|t| t["name"] == "idx_ownership_rankings")
        .unwrap();
    assert_eq!(
        rankings["inputSchema"]["properties"]["view"]["type"],
        "string"
    );

    // KSEI's composition: nothing imported and IDX_OFFLINE=1, so no download.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 11, "method": "tools/call",
        "params": {"name": "idx_ownership_composition", "arguments": {"symbol": "BBCA"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["id"], 11);
    assert_eq!(r["result"]["isError"], true);
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "offline_miss", "{body}");
    assert_eq!(body["exit_code"], 5, "{body}");

    // The snapshot tool: status works offline (local part only), sync does not.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 12, "method": "tools/call",
        "params": {"name": "idx_ownership_snapshot", "arguments": {"action": "status"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["id"], 12);
    assert_ne!(r["result"]["isError"], true, "{r}");
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["schema"], "idx.ownership.status.v1");
    assert!(body["data"]["remote"].is_null(), "{body}");
    assert!(body["data"]["supported_schema_version"].is_i64(), "{body}");
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 13, "method": "tools/call",
        "params": {"name": "idx_ownership_snapshot", "arguments": {"action": "sync"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["result"]["isError"], true);
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "offline_miss", "{body}");
    assert_eq!(body["exit_code"], 5);

    // Foreign flow from IPOT, read from the seeded cache.
    let foreign = tools.iter().find(|t| t["name"] == "idx_foreign").unwrap();
    for arg in ["days", "from", "to", "source"] {
        assert!(
            foreign["inputSchema"]["properties"][arg].is_object(),
            "idx_foreign takes {arg}"
        );
    }
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 14, "method": "tools/call",
        "params": {"name": "idx_foreign", "arguments": {
            "symbol": "BBCA", "source": "ipot", "from": "2026-10-12", "to": "2026-10-16"
        }}
    });
    writeln!(stdin, "{call}").unwrap();
    let r = next();
    assert_eq!(r["id"], 14);
    assert_ne!(r["result"]["isError"], true, "{r}");
    let body: serde_json::Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["schema"], "idx.foreign.value.v1");
    assert_eq!(body["provider"], "ipot");
    assert_eq!(body["data"]["days"].as_array().unwrap().len(), 5);
    assert_eq!(body["data"]["totals"]["foreign_net_value"], -1.8231e12);
    assert_eq!(body["data"]["rounded"], true);

    for (id, args) in [
        (15, serde_json::json!({"symbol": "BBCA", "source": "rti"})),
        (
            16,
            serde_json::json!({"symbol": "BBCA", "days": 3, "from": "2026-10-12"}),
        ),
        (
            17,
            serde_json::json!({"symbol": "BBCA", "source": "ipot", "days": 61}),
        ),
    ] {
        let call = serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "idx_foreign", "arguments": args}
        });
        writeln!(stdin, "{call}").unwrap();
        let r = next();
        assert_eq!(r["id"], id);
        assert_eq!(r["result"]["isError"], true, "{r}");
        let body: serde_json::Value =
            serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["error"]["code"], "usage", "{body}");
    }

    // Minute bars from the seeded chart; the quote is not cached, so it is a
    // warning and `quote` is null rather than a failed call.
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 18, "method": "tools/call",
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

#[test]
fn idx_quote_says_why_a_failed_fetch_served_a_stale_copy() {
    let dir = tempfile::tempdir().unwrap();
    // A session with a recognisable crumb and an hour-old v7 copy.
    let cache = idx::cache::Cache::new(dir.path().join("cache"));
    cache
        .put_json(
            "yahoo:session",
            &serde_json::json!({
                "cookie": "A3=SECRETCOOKIE",
                "crumb": "SECRETCRUMB",
                "created_at": chrono::Utc::now(),
            }),
        )
        .unwrap();
    let body = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/yahoo/quote_v7_bbca_bbri_tlkm.json"),
    )
    .unwrap();
    cache
        .put_at(
            "https://query1.finance.yahoo.com/v7/finance/quote?symbols=BBCA.JK,BBRI.JK,TLKM.JK",
            200,
            &body,
            chrono::Utc::now() - chrono::Duration::hours(1),
        )
        .unwrap();
    // Online, but every request goes to a loopback port that was just free
    // and so refuses it: nothing leaves the machine.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let proxy = format!("http://127.0.0.1:{port}");
    let mut child = spawn_server_with(
        dir.path(),
        &[
            ("HTTPS_PROXY", &proxy),
            ("HTTP_PROXY", &proxy),
            ("ALL_PROXY", &proxy),
        ],
    );
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
    let next = || -> String {
        rx.recv_timeout(Duration::from_secs(30))
            .expect("server response within 30s")
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
    next();
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#
    )
    .unwrap();
    let call = serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "tools/call",
        "params": {"name": "idx_quote", "arguments": {"symbols": "BBCA,BBRI,TLKM"}}
    });
    writeln!(stdin, "{call}").unwrap();
    let line = next();
    assert!(
        !line.contains("crumb") && !line.contains("SECRET"),
        "{line}"
    );
    let body = tool_body(&serde_json::from_str(&line).unwrap());
    assert_eq!(body["cached"], true);
    assert_eq!(body["stale"], true);
    assert_eq!(body["stale_reason"], "fetch_failed");
    assert_eq!(body["data"].as_array().unwrap().len(), 3);
    let warnings = body["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{body}");
    let w = warnings[0].as_str().unwrap();
    assert!(
        w.starts_with(
            "live fetch failed (error sending request for url (https://query1.finance.yahoo.com/v7/finance/quote): "
        ) && w.ends_with("); showing cached copy"),
        "{w}"
    );

    drop(stdin);
    let out = child.wait_with_output().expect("server exits");
    assert!(out.status.success(), "exit {:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("crumb") && !stderr.contains("SECRET"),
        "{stderr}"
    );
}
