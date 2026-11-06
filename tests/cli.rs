//! End-to-end tests of the binary that never touch the network.
//! Live-provider checks live in `scripts/smoke.sh`.

use assert_cmd::Command;
use predicates::prelude::*;

fn idx(cache: &std::path::Path, config: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("idx").unwrap();
    // Keep the Stockbit session and the ownership database inside the
    // sandbox so a test never reads or writes the developer's real files.
    let dir = config.parent().unwrap_or_else(|| std::path::Path::new("."));
    cmd.env_clear()
        .env("IDX_CACHE_DIR", cache)
        .env("IDX_CONFIG", config)
        .env("IDX_STOCKBIT_SESSION", dir.join("stockbit-session.json"))
        .env("IDX_OWNERSHIP_DB", dir.join("ownership.sqlite"))
        .env("NO_COLOR", "1")
        .env_remove("IDX_STOCKBIT_ACCESS_TOKEN")
        .env_remove("IDX_STOCKBIT_REFRESH_TOKEN")
        // Safety net: these tests must never reach the network, whatever a
        // test does. A cache miss then fails with exit 5 instead of fetching.
        .env("IDX_OFFLINE", "1")
        // Keep the test machine's own settings out of the picture.
        .env_remove("IDX_PROVIDER")
        .env_remove("IDX_OUTPUT");
    // SystemRoot is needed on Windows for the TLS/crypto providers to initialise.
    if let Ok(v) = std::env::var("SystemRoot") {
        cmd.env("SystemRoot", v);
    }
    cmd
}

struct Sandbox {
    _dir: tempfile::TempDir,
    cache: std::path::PathBuf,
    config: std::path::PathBuf,
}

fn sandbox() -> Sandbox {
    let dir = tempfile::tempdir().unwrap();
    Sandbox {
        cache: dir.path().join("cache"),
        config: dir.path().join("config.toml"),
        _dir: dir,
    }
}

#[test]
fn help_lists_commands_and_exit_codes() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("stocks"))
        .stdout(predicate::str::contains("Exit codes"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "quote", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("idx stocks quote BBCA,BBRI,TLKM"));
}

#[test]
fn version_in_both_formats() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .arg("version")
        .assert()
        .success()
        .stdout(predicate::str::starts_with(format!(
            "idx {}",
            env!("CARGO_PKG_VERSION")
        )));
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "version"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn schema_outputs_are_valid_json_schema() {
    let sb = sandbox();
    for name in ["quotes", "history", "error", "quote", "candle"] {
        let out = idx(&sb.cache, &sb.config)
            .args(["schema", name])
            .output()
            .unwrap();
        assert!(out.status.success(), "{name}");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(
            v["$schema"].as_str().unwrap().contains("json-schema.org"),
            "{name}"
        );
    }
    idx(&sb.cache, &sb.config)
        .args(["schema", "nope"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("unknown schema"));
}

#[test]
fn usage_errors_exit_2() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["stocks", "quote"])
        .assert()
        .code(2);
    idx(&sb.cache, &sb.config)
        .args(["stocks", "quote", "BB$CA"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("invalid symbol"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "history", "BBCA", "--period", "7d"])
        .assert()
        .code(2);
    idx(&sb.cache, &sb.config)
        .args(["--offline", "--no-cache", "version"])
        .assert()
        .code(2);
}

#[test]
fn offline_with_empty_cache_exits_5_and_json_error_is_structured() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["--offline", "stocks", "quote", "BBCA"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("offline"))
        .stderr(predicate::str::contains("hint:"));

    let out = idx(&sb.cache, &sb.config)
        .args(["--offline", "-o", "json", "stocks", "history", "BBCA"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5));
    assert!(out.stdout.is_empty(), "errors never go to stdout");
    let v: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(v["error"]["code"], "offline_miss");
    assert_eq!(v["exit_code"], 5);
}

#[test]
fn config_init_set_get_roundtrip() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["config", "path"])
        .assert()
        .success()
        .stdout(predicate::str::contains("config.toml"));
    idx(&sb.cache, &sb.config)
        .args(["config", "init"])
        .assert()
        .success();
    assert!(sb.config.exists());
    idx(&sb.cache, &sb.config)
        .args(["config", "init"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("already exists"));
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "provider", "yahoo"])
        .assert()
        .success();
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "provider", "bloomberg"])
        .assert()
        .code(6);
    idx(&sb.cache, &sb.config)
        .args(["config", "get", "provider"])
        .assert()
        .success()
        .stdout("yahoo\n");
    idx(&sb.cache, &sb.config)
        .args(["config", "get"])
        .assert()
        .success()
        .stdout(predicate::str::contains("provider    : yahoo"));

    // Env beats file, CLI beats env.
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_PROVIDER", "auto")
        .args(["-o", "json", "config", "get"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["provider"], "auto");
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_PROVIDER", "auto")
        .args(["--provider", "yahoo", "-o", "json", "config", "get"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["provider"], "yahoo");
    assert_eq!(v["cache_dir"], sb.cache.display().to_string());
}

#[test]
fn new_stock_commands_have_help_and_fail_offline_cleanly() {
    let sb = sandbox();
    for (cmd, needle) in [
        ("profile", "Company profile"),
        ("financials", "--period"),
        ("earnings", "EPS"),
        ("fundamental", "verdict"),
        ("technical", "RSI"),
        ("compare", "Side-by-side"),
    ] {
        idx(&sb.cache, &sb.config)
            .args(["stocks", cmd, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains(needle));
    }
    // Offline with an empty cache: every command exits 5 with a clear message.
    for cmd in [
        "profile",
        "financials",
        "earnings",
        "fundamental",
        "technical",
    ] {
        idx(&sb.cache, &sb.config)
            .args(["--offline", "stocks", cmd, "BBCA"])
            .assert()
            .code(5)
            .stderr(predicate::str::contains("BBCA"));
    }
    idx(&sb.cache, &sb.config)
        .args(["--offline", "stocks", "compare", "BBCA", "BBRI"])
        .assert()
        .code(5);
    idx(&sb.cache, &sb.config)
        .args(["stocks", "compare", "BBCA"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("at least two"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "financials", "BBCA", "--period", "weekly"])
        .assert()
        .code(2);
}

#[test]
fn stockbit_surface_and_session_commands() {
    let sb = sandbox();
    // Help text for the group and the realtime-only command.
    idx(&sb.cache, &sb.config)
        .args(["stockbit", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("login"))
        .stdout(predicate::str::contains("orderbook"));
    idx(&sb.cache, &sb.config)
        .args(["stockbit", "orderbook", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("depth"));

    // No session yet: status says so, and reads error as a usage error (exit 2)
    // before any network — even the realtime order book and the provider quote.
    idx(&sb.cache, &sb.config)
        .args(["stockbit", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("logged in"))
        .stdout(predicate::str::contains("no"));
    idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stockbit", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"logged_in\": false"));
    idx(&sb.cache, &sb.config)
        .args(["stockbit", "orderbook", "BBCA"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("stockbit login"));
    idx(&sb.cache, &sb.config)
        .args(["--provider", "stockbit", "stocks", "quote", "BBCA"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("stockbit login"));

    // logout on an empty session is a clean no-op.
    idx(&sb.cache, &sb.config)
        .args(["stockbit", "logout"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No stored"));

    // A session file makes status report logged-in (presence of a refresh
    // token, not its validity); the dummy token is never sent anywhere.
    let session = sb.config.parent().unwrap().join("stockbit-session.json");
    std::fs::write(&session, r#"{"access":"a.b.c","refresh":"d.e.f"}"#).unwrap();
    idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stockbit", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"logged_in\": true"));
}

#[test]
fn orderbook_leaves_out_foreign_flow_stockbit_marks_stale() {
    let sb = sandbox();
    // BBCA during trading: has_foreign_bs false, so fbuy/fsell/fnet are the
    // previous session's (534.5B / 650.7B / -116.1B) and must not show.
    seed(
        &sb.cache,
        "https://exodus.stockbit.com/company-price-feed/v2/orderbook/companies/BBCA",
        "stockbit/orderbook_bbca_in_session.json",
    );
    // A dummy access token fresh until 2100 ({"alg":"none"}.{"exp":4102444800}),
    // so no refresh is attempted; the harness is offline, nothing is sent.
    let session = sb.config.parent().unwrap().join("stockbit-session.json");
    std::fs::write(
        &session,
        r#"{"access":"eyJhbGciOiJub25lIn0.eyJleHAiOjQxMDI0NDQ4MDB9.sig","refresh":"d.e.f"}"#,
    )
    .unwrap();

    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stockbit", "orderbook", "BBCA"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["schema"], "idx.orderbook.v1");
    for k in ["foreign_buy", "foreign_sell", "foreign_net"] {
        assert!(v["data"][k].is_null(), "{k}: {}", v["data"][k]);
    }
    assert_eq!(v["data"]["volume"], 134_744_100);
    assert_eq!(v["data"]["bids"].as_array().unwrap().len(), 22);
    assert_eq!(v["data"]["asks"].as_array().unwrap().len(), 64);
    let warnings = v["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w
            .as_str()
            .unwrap()
            .contains("Stockbit flags it as not current (has_foreign_bs false)")),
        "{warnings:?}"
    );

    let dash = idx::output::format::DASH;
    idx(&sb.cache, &sb.config)
        .args(["stockbit", "orderbook", "BBCA", "--depth", "3"])
        .assert()
        .success()
        .stdout(predicate::str::contains(format!(
            "foreign buy {dash} / sell {dash} / net {dash}"
        )))
        .stdout(predicate::str::contains("116.1B").not())
        .stderr(predicate::str::contains(
            "warning: foreign flow left empty: Stockbit flags it as not current (has_foreign_bs false)",
        ));
}

#[test]
fn screen_news_sentiment_surface() {
    let sb = sandbox();
    for (args, needle) in [
        (vec!["stocks", "screen", "--help"], "Keys: pe fpe pb"),
        (vec!["stocks", "news", "--help"], "Sources: kontan"),
        (
            vec!["stocks", "sentiment", "--help"],
            "Indonesian and English",
        ),
    ] {
        idx(&sb.cache, &sb.config)
            .args(&args)
            .assert()
            .success()
            .stdout(predicate::str::contains(needle));
    }
    // Filter syntax errors are usage errors, before any network.
    idx(&sb.cache, &sb.config)
        .args(["stocks", "screen", "-f", "pe 15"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("operator"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "screen", "-f", "pe<15", "--sort", "nope"])
        .assert()
        .code(2);
    idx(&sb.cache, &sb.config)
        .args(["stocks", "news", "--source", "detik"])
        .assert()
        .code(2);
    // Offline with nothing cached: screen is an offline miss, news reports no readable feed.
    idx(&sb.cache, &sb.config)
        .args(["--offline", "stocks", "screen", "-f", "pe<15"])
        .assert()
        .code(5);
    idx(&sb.cache, &sb.config)
        .args(["--offline", "-o", "json", "stocks", "news", "BBCA"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("not cached"));
}

#[test]
fn broker_command_surface() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["broker", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "idx broker BBCA --date 2026-10-16",
        ))
        .stdout(predicate::str::contains("IPOT"));
    idx(&sb.cache, &sb.config)
        .args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("broker"));
    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "broker"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.to_string().contains("foreign_net_value"));
    // Offline with an empty cache: exit 5, the latest-session walk stops at the first miss.
    idx(&sb.cache, &sb.config)
        .args(["broker", "BBCA"])
        .assert()
        .code(5);
    idx(&sb.cache, &sb.config)
        .args(["-o", "json", "broker", "BBCA", "--date", "2026-10-16"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("\"offline_miss\""));
    for args in [
        vec!["broker"],
        vec![
            "broker",
            "BBCA",
            "--date",
            "2026-10-16",
            "--from",
            "2026-10-01",
        ],
        vec!["broker", "BBCA", "--investor", "retail"],
        vec!["broker", "BBCA", "--date", "16-10-2026"],
        vec![
            "broker",
            "BBCA",
            "--from",
            "2026-10-16",
            "--to",
            "2026-10-01",
        ],
    ] {
        idx(&sb.cache, &sb.config).args(&args).assert().code(2);
    }
}

#[test]
fn no_loading_line_offline_even_when_forced() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .env("IDX_PROGRESS", "always")
        .args(["-o", "json", "market", "holidays", "--year", "2026"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}

#[test]
fn holidays_work_offline_from_the_bundled_list() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["market", "holidays", "--year", "2026"])
        .assert()
        .success()
        .stdout(predicate::str::contains("239 trading days"))
        .stdout(predicate::str::contains("Proklamasi Kemerdekaan"));
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "market", "holidays", "--year", "2027"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.holidays.v1");
    assert_eq!(v["provider"], "bundled");
    assert_eq!(v["data"]["trading_days"], 241);
    assert_eq!(v["data"]["holidays"][0]["date"], "2027-01-01");
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "csv", "market", "holidays", "--year", "2026"])
        .output()
        .unwrap();
    let csv = String::from_utf8(out.stdout).unwrap();
    assert!(csv.starts_with("date,weekday,name,name_en\n"), "{csv}");
    assert_eq!(csv.lines().count(), 23);
    idx(&sb.cache, &sb.config)
        .args(["market", "holidays", "--year", "2026", "--all"])
        .assert()
        .code(2);
}

#[test]
fn directory_commands_fall_back_offline_and_indices_need_the_cache() {
    let sb = sandbox();
    for (args, text) in [
        (
            vec!["stocks", "list", "--help"],
            "idx stocks list --board akselerasi",
        ),
        (
            vec!["market", "members", "--help"],
            "idx market members --search mandiri",
        ),
        (
            vec!["market", "indices", "--help"],
            "idx market indices LQ45 IDX30 IDX80",
        ),
    ] {
        idx(&sb.cache, &sb.config)
            .args(&args)
            .assert()
            .success()
            .stdout(predicate::str::contains(text));
    }
    for (name, field) in [
        ("instruments", "listing_date"),
        ("members", "licenses"),
        ("indices", "market_cap"),
    ] {
        let out = idx(&sb.cache, &sb.config)
            .args(["schema", name])
            .output()
            .unwrap();
        assert!(out.status.success(), "{name}");
        assert!(
            String::from_utf8_lossy(&out.stdout).contains(field),
            "{name}"
        );
    }
    // Offline with nothing cached: the bundled lists stand in, with a warning.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "list", "--search", "BBCA"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.instruments.v1");
    assert_eq!(v["provider"], "bundled");
    assert_eq!(v["data"]["instruments"][0]["symbol"], "BBCA");
    assert!(v["warnings"][0]
        .as_str()
        .unwrap()
        .contains("bundled universe"));
    idx(&sb.cache, &sb.config)
        .args(["market", "members", "-s", "YP"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MIRAE ASSET"))
        .stderr(predicate::str::contains("bundled member list"));
    idx(&sb.cache, &sb.config)
        .args(["market", "indices"])
        .assert()
        .code(5);
    idx(&sb.cache, &sb.config)
        .args(["market", "indices", "--date", "16-10-2026"])
        .assert()
        .code(2);
}

#[test]
fn calendar_command_surface() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["market", "calendar", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "idx market calendar --symbol BBCA",
        ));
    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "calendar"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("milestone"));
    idx(&sb.cache, &sb.config)
        .args(["market", "calendar"])
        .assert()
        .code(5);
    for args in [
        vec!["market", "calendar", "--type", "concerts"],
        vec![
            "market",
            "calendar",
            "--date",
            "2026-10-19",
            "--from",
            "2026-10-01",
        ],
        vec![
            "market",
            "calendar",
            "--from",
            "2026-01-01",
            "--to",
            "2026-12-31",
        ],
        vec!["market", "calendar", "--symbol", "not a ticker"],
    ] {
        idx(&sb.cache, &sb.config).args(&args).assert().code(2);
    }
}

#[test]
fn actions_command_surface() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["stocks", "actions", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "idx stocks actions ENRG --since 2025-01-01",
        ));
    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "actions"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let schema = String::from_utf8_lossy(&out.stdout);
    assert!(schema.contains("ratio_old") && schema.contains("warrants_rights"));
    idx(&sb.cache, &sb.config)
        .args([
            "-o",
            "json",
            "stocks",
            "actions",
            "BBCA",
            "--since",
            "2025-01-01",
        ])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("\"offline_miss\""));
    for args in [
        vec!["stocks", "actions"],
        vec!["stocks", "actions", "BBCA", "--since", "2025/01/01"],
        vec!["stocks", "actions", "^JKSE"],
    ] {
        idx(&sb.cache, &sb.config).args(&args).assert().code(2);
    }
}

/// Puts a recorded response in the sandbox cache under its URL, so an
/// offline run reads it as if it had been fetched.
fn seed(cache: &std::path::Path, url: &str, fixture: &str) {
    let body = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(fixture),
    )
    .unwrap();
    idx::cache::Cache::new(cache).put(url, 200, &body).unwrap();
}

const V7_BBCA_BBRI_TLKM: &str =
    "https://query1.finance.yahoo.com/v7/finance/quote?symbols=BBCA.JK,BBRI.JK,TLKM.JK";

#[test]
fn stale_copies_say_why() {
    let sb = sandbox();
    seed(
        &sb.cache,
        V7_BBCA_BBRI_TLKM,
        "yahoo/quote_v7_bbca_bbri_tlkm.json",
    );
    // Fresh: no reason, no notice.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "quote", "BBCA,BBRI,TLKM"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["stale"], false);
    assert!(v.get("stale_reason").is_none(), "{v}");

    // Older than the TTL and offline: stale, and why.
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_QUOTE_TTL_SECS", "0")
        .args(["-o", "json", "stocks", "quote", "BBCA,BBRI,TLKM"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stderr.is_empty(),
        "JSON mode keeps notices in the envelope"
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["cached"], true);
    assert_eq!(v["stale"], true);
    assert_eq!(v["stale_reason"], "offline");
    idx(&sb.cache, &sb.config)
        .env("IDX_QUOTE_TTL_SECS", "0")
        .args(["stocks", "quote", "BBCA,BBRI,TLKM"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "warning: showing stale cached data from 20",
        ))
        .stderr(predicate::str::contains(" WIB (--offline)"));
    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "quotes"])
        .output()
        .unwrap();
    let schema = String::from_utf8(out.stdout).unwrap();
    assert!(schema.contains("stale_reason") && schema.contains("fetch_failed"));
}

/// A loopback proxy address that refuses connections: a port that was just
/// free. Pointing HTTPS_PROXY at it makes every live fetch fail at once,
/// without anything leaving the machine.
fn refused_proxy() -> String {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    format!("http://127.0.0.1:{port}")
}

/// A Yahoo session with a recognisable crumb and an hour-old v7 copy for
/// BBCA,BBRI,TLKM, so a failed live fetch has to fall back to the copy.
fn seed_failed_fetch(cache: &std::path::Path) {
    let c = idx::cache::Cache::new(cache);
    c.put_json(
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
    c.put_at(
        V7_BBCA_BBRI_TLKM,
        200,
        &body,
        chrono::Utc::now() - chrono::Duration::hours(1),
    )
    .unwrap();
}

#[test]
fn a_failed_fetch_serves_the_copy_and_says_why_without_the_crumb() {
    let sb = sandbox();
    seed_failed_fetch(&sb.cache);
    let proxy = refused_proxy();
    // Online, but every request goes to a proxy that refuses it.
    let online = || {
        let mut cmd = idx(&sb.cache, &sb.config);
        cmd.env_remove("IDX_OFFLINE")
            .env("HTTPS_PROXY", &proxy)
            .env("HTTP_PROXY", &proxy)
            .env("ALL_PROXY", &proxy);
        cmd
    };
    let leaks = |text: &str| text.contains("crumb") || text.contains("SECRET");

    let out = online()
        .args(["-o", "json", "stocks", "quote", "BBCA,BBRI,TLKM"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.is_empty(), "no default-level log line: {stderr}");
    assert!(!leaks(&stdout), "{stdout}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["cached"], true);
    assert_eq!(v["stale"], true);
    assert_eq!(v["stale_reason"], "fetch_failed");
    assert_eq!(v["data"].as_array().unwrap().len(), 3);
    let warnings = v["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{v}");
    let w = warnings[0].as_str().unwrap();
    assert!(
        w.starts_with(
            "live fetch failed (error sending request for url (https://query1.finance.yahoo.com/v7/finance/quote): "
        ) && w.ends_with("); showing cached copy"),
        "{w}"
    );

    // Table mode: the cause and the stale notice on stderr, no crumb.
    let out = online()
        .args(["stocks", "quote", "BBCA,BBRI,TLKM"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stdout.contains("BBCA"), "{stdout}");
    assert!(
        stderr.contains("warning: live fetch failed (error sending request for url (https://query1.finance.yahoo.com/v7/finance/quote): "),
        "{stderr}"
    );
    assert!(
        stderr.contains("warning: showing stale cached data from 20") && stderr.contains(" WIB\n"),
        "{stderr}"
    );
    assert!(!stderr.contains("--offline"), "{stderr}");
    assert!(!leaks(&stdout) && !leaks(&stderr), "{stdout}\n{stderr}");
}

#[test]
fn quotes_flag_the_latest_session() {
    let sb = sandbox();
    seed(
        &sb.cache,
        "https://query1.finance.yahoo.com/v7/finance/quote?symbols=WIKA.JK,KAEF.JK,BBCA.JK",
        "yahoo/quote_v7_wika_suspended_kaef_bbca.json",
    );
    seed(
        &sb.cache,
        "https://query1.finance.yahoo.com/v7/finance/quote?symbols=BBCA.JK,^GSPC",
        "yahoo/quote_v7_bbca_gspc.json",
    );
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "quote", "WIKA,KAEF,BBCA"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let rows = v["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    // WIKA last traded on 2025-02-17; KAEF and BBCA on 2026-11-05, which is
    // behind or not depending on the day the test runs.
    assert_eq!(rows[0]["symbol"], "WIKA");
    assert_eq!(rows[0]["behind_session"], true);
    assert!(rows.iter().all(|r| r["behind_session"].is_boolean()), "{v}");

    idx(&sb.cache, &sb.config)
        .args(["stocks", "quote", "WIKA,KAEF,BBCA"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Behind the latest IDX session: WIKA (last trade 2025-02-17)",
        ))
        .stdout(predicate::str::contains(
            "suspended, not traded since, or the source lags",
        ));

    // Not an IDX instrument: no flag either way.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "quote", "BBCA,^GSPC"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"][1]["symbol"], "^GSPC");
    assert!(v["data"][1]["behind_session"].is_null(), "{v}");
    assert!(v["data"][0]["behind_session"].is_boolean());

    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "csv", "stocks", "quote", "WIKA,KAEF,BBCA"])
        .output()
        .unwrap();
    let csv = String::from_utf8(out.stdout).unwrap();
    let header = csv.lines().next().unwrap();
    assert!(
        header.ends_with(",market_time,source,behind_session"),
        "{header}"
    );
    assert!(
        csv.lines().nth(1).unwrap().ends_with(",yahoo:quote,true"),
        "{csv}"
    );

    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "quote"])
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("behind_session"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "quote", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("behind_session"));
}

#[test]
fn foreign_commands_read_the_exchange_summaries() {
    let sb = sandbox();
    seed(
        &sb.cache,
        "https://www.idx.co.id/primary/TradingSummary/GetStockSummary?date=20261016&start=0&length=9999",
        "idx/stocksummary_20261016_excerpt.json",
    );
    seed(
        &sb.cache,
        "https://www.idx.co.id/primary/ListedCompany/GetTradingInfoSS?code=BBCA&start=0&length=30",
        "idx/tradinginfoss_bbca_excerpt.json",
    );
    idx(&sb.cache, &sb.config)
        .args(["market", "foreign", "--date", "2026-10-16", "--top", "3"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Foreign flow · 2026-10-16 · 28 stocks",
        ))
        .stdout(predicate::str::contains("Foreign net selling"));
    let out = idx(&sb.cache, &sb.config)
        .args([
            "-o",
            "json",
            "market",
            "foreign",
            "--date",
            "2026-10-16",
            "--sort",
            "shares",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.market.foreign.v1");
    assert_eq!(v["provider"], "idx");
    assert_eq!(v["data"]["sort"], "shares");
    assert_eq!(v["data"]["net_buy"].as_array().unwrap().len(), 10);

    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "foreign", "BBCA", "--days", "30"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.foreign.v1");
    assert_eq!(v["data"]["to"], "2026-10-16");
    assert_eq!(v["data"]["days"].as_array().unwrap().len(), 30);
    assert!(v["data"]["totals"]["streak"].as_i64().unwrap() < 0);
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "csv", "stocks", "foreign", "BBCA", "--days", "30"])
        .output()
        .unwrap();
    let csv = String::from_utf8(out.stdout).unwrap();
    assert!(
        csv.starts_with("date,close,change,volume,value,foreign_buy"),
        "{csv}"
    );

    // Nothing cached for these.
    idx(&sb.cache, &sb.config)
        .args(["stocks", "foreign", "BBRI"])
        .assert()
        .code(5);
    for args in [
        vec!["stocks", "foreign", "BBCA", "--days", "0"],
        vec!["stocks", "foreign", "BBCA", "--days", "999"],
        vec!["market", "foreign", "--sort", "lots"],
    ] {
        idx(&sb.cache, &sb.config).args(&args).assert().code(2);
    }
    for name in ["foreign", "market-foreign"] {
        let out = idx(&sb.cache, &sb.config)
            .args(["schema", name])
            .output()
            .unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("foreign_net_value_at_close"));
    }
}

#[test]
fn market_brokers_rank_the_members() {
    let sb = sandbox();
    seed(
        &sb.cache,
        "https://www.idx.co.id/primary/TradingSummary/GetBrokerSummary?date=20261016&start=0&length=9999",
        "idx/brokersummary_20261016.json",
    );
    idx(&sb.cache, &sb.config)
        .args(["market", "brokers", "--date", "2026-10-16", "--top", "5"])
        .assert()
        .success()
        .stdout(predicate::str::contains("88 active · by value"))
        .stdout(predicate::str::contains("twice the market's"));
    let out = idx(&sb.cache, &sb.config)
        .args([
            "-o",
            "json",
            "market",
            "brokers",
            "--date",
            "2026-10-16",
            "--top",
            "0",
            "--sort",
            "volume",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.market.brokers.v1");
    assert_eq!(v["data"]["sort"], "volume");
    assert_eq!(v["data"]["brokers"].as_array().unwrap().len(), 88);
    assert_eq!(v["data"]["brokers"][0]["rank"], 1);
    idx(&sb.cache, &sb.config)
        .args(["market", "brokers", "--sort", "lots"])
        .assert()
        .code(2);
    idx(&sb.cache, &sb.config)
        .args(["market", "brokers", "--date", "2026-10-18"])
        .assert()
        .code(5);
}

#[test]
fn bandar_broker_surface() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["bandar", "broker", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "idx bandar broker CC --period 1m --scan 200",
        ))
        .stdout(predicate::str::contains("top ten"));
    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "bandar-broker"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("opposite_value"));
    // Offline with an empty cache: the latest-session probe misses first.
    idx(&sb.cache, &sb.config)
        .args(["bandar", "broker", "YP", "--symbols", "BBCA,BBRI"])
        .assert()
        .code(5);
    for args in [
        vec!["bandar", "broker"],
        vec!["bandar", "broker", "MIRAE", "--symbols", "BBCA"],
        vec!["bandar", "broker", "YP", "--period", "2w"],
    ] {
        idx(&sb.cache, &sb.config).args(&args).assert().code(2);
    }
}

#[test]
fn bandar_command_surface() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["bandar", "detect", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("idx bandar detect BBCA --daily"))
        .stdout(predicate::str::contains("big accumulation"));
    idx(&sb.cache, &sb.config)
        .args(["bandar", "screen", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("quiet-accumulation"))
        .stdout(predicate::str::contains("below-bandar"));
    for name in ["bandar", "bandar-screen"] {
        let out = idx(&sb.cache, &sb.config)
            .args(["schema", name])
            .output()
            .unwrap();
        assert!(out.status.success(), "{name}");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert!(v.to_string().contains("net_pct"), "{name}");
    }
    // Offline with an empty cache: exit 5.
    idx(&sb.cache, &sb.config)
        .args(["bandar", "detect", "BBCA"])
        .assert()
        .code(5);
    idx(&sb.cache, &sb.config)
        .args([
            "-o",
            "json",
            "bandar",
            "screen",
            "--preset",
            "big-accumulation",
        ])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("\"offline_miss\""));
    for args in [
        vec!["bandar"],
        vec!["bandar", "detect"],
        vec!["bandar", "detect", "BBCA", "--period", "2w"],
        vec!["bandar", "detect", "BBCA", "--top", "4"],
        vec![
            "bandar", "detect", "BBCA", "BBRI", "BMRI", "BBNI", "--daily",
        ],
        vec![
            "bandar",
            "detect",
            "BBCA",
            "--date",
            "2026-10-16",
            "--from",
            "2026-10-01",
        ],
        vec!["bandar", "screen", "--preset", "everything"],
        vec!["bandar", "screen", "-f", "pe<10"],
        vec!["bandar", "screen", "--sort", "phase"],
        vec!["bandar", "screen", "--asc", "--desc"],
    ] {
        idx(&sb.cache, &sb.config).args(&args).assert().code(2);
    }
}

#[test]
fn msn_provider_is_selectable() {
    let sb = sandbox();
    // --provider msn is accepted; offline with no cache -> exit 5 like Yahoo.
    idx(&sb.cache, &sb.config)
        .args(["--provider", "msn", "--offline", "stocks", "quote", "BBCA"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("BBCA"));
    // MSN does not serve history: explicit provider gets exit 6 with a hint.
    idx(&sb.cache, &sb.config)
        .args(["--provider", "msn", "stocks", "history", "BBCA"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("--provider auto"));
}

const QUOTE_SUMMARY: &str = "https://query2.finance.yahoo.com/v10/finance/quoteSummary";
const FUNDAMENTAL_MODULES: &str =
    "modules=assetProfile,summaryDetail,defaultKeyStatistics,financialData,price";

#[test]
fn negative_equity_is_adverse_in_the_merged_report() {
    // GIAA on 2026-11-06: Yahoo blanks debt/equity but keeps a negative book
    // value; `auto` asks MSN to fill the gaps, whose debt/equity is the
    // placeholder -999.9999.
    let sb = sandbox();
    seed(
        &sb.cache,
        &format!("{QUOTE_SUMMARY}/GIAA.JK?{FUNDAMENTAL_MODULES}"),
        "yahoo/quote_summary_fundamentals_giaa.json",
    );
    seed(
        &sb.cache,
        "https://services.bingapis.com/contentservices-finance.csautosuggest/api/v1/Query?query=GIAA&market=en-id",
        "msn/autosuggest_giaa.json",
    );
    seed(
        &sb.cache,
        "https://assets.msn.com/service/Finance/Equities?ids=bkgq5r",
        "msn/equities_giaa.json",
    );
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "fundamental", "GIAA"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let d = &v["data"];
    assert_eq!(v["schema"], "idx.fundamentals.v1");
    assert_eq!(d["sources"][1], "msn:equities", "{d}");
    assert!(d["health"]["debt_to_equity"].is_null(), "{d}");
    assert_eq!(d["per_share"]["book_value"], -0.001);
    assert_eq!(d["financial_currency"], "USD");
    assert_eq!(d["verdict"]["label"], "overvalued");
    assert_eq!(d["verdict"]["score"], -3);
    assert_eq!(d["verdict"]["inputs_available"], 5);
    let reasons = d["verdict"]["reasons"].to_string();
    assert!(
        reasons.contains("negative shareholders' equity"),
        "{reasons}"
    );
    assert!(!reasons.contains("conservative"), "{reasons}");
}

#[test]
fn usd_statements_do_not_score_pb() {
    // ITMG on 2026-11-06: an IDR price over USD statements (P/B 14103).
    let sb = sandbox();
    seed(
        &sb.cache,
        &format!("{QUOTE_SUMMARY}/ITMG.JK?{FUNDAMENTAL_MODULES}"),
        "yahoo/quote_summary_fundamentals_itmg.json",
    );
    let out = idx(&sb.cache, &sb.config)
        .args([
            "--provider",
            "yahoo",
            "-o",
            "json",
            "stocks",
            "fundamental",
            "ITMG",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let d = &v["data"];
    assert_eq!(d["currency"], "IDR");
    assert_eq!(d["financial_currency"], "USD");
    assert!(
        d["valuation"]["price_to_book"].as_f64().unwrap() > 14000.0,
        "the reported value is kept"
    );
    // #25: no P/B reason at all; the table's note says why P/B is not scored.
    let reasons = d["verdict"]["reasons"].to_string();
    assert!(!reasons.contains("P/B"), "{reasons}");
    assert_eq!(d["verdict"]["inputs_available"], 8);

    idx(&sb.cache, &sb.config)
        .args(["--provider", "yahoo", "stocks", "fundamental", "ITMG"])
        .assert()
        .success()
        .stdout(predicate::str::contains("14,103.31*"))
        .stdout(predicate::str::contains("1.72 USD"))
        .stdout(predicate::str::contains(
            "* Statements in USD, price in IDR",
        ));
    idx(&sb.cache, &sb.config)
        .args(["schema", "fundamentals"])
        .assert()
        .success()
        .stdout(predicate::str::contains("financial_currency"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "fundamental", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("financial_currency"));
}

#[test]
fn ownership_import_and_queries_work_offline() {
    let sb = sandbox();
    let db = sb.cache.join("own.sqlite");
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ownership/above1_20260831_excerpt.xlsx"
    );
    let prev = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ownership/above1_20260731_synthetic.xlsx"
    );
    let own = |sb: &Sandbox| {
        let mut c = idx(&sb.cache, &sb.config);
        c.env("IDX_OWNERSHIP_DB", &db);
        c
    };
    // Empty database: a clear not-found, exit 4.
    own(&sb)
        .args(["ownership", "ticker", "AADI"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("ownership import"));
    // ... except for a name that could never match: usage, exit 2.
    own(&sb)
        .args(["ownership", "entity", "!!!"])
        .assert()
        .code(2);

    own(&sb)
        .args(["-o", "json", "ownership", "import", "--file", prev])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema\": \"idx.ownership.import.v1\"",
        ))
        .stdout(predicate::str::contains("\"as_of\": \"2026-07-31\""));
    own(&sb)
        .args(["ownership", "import", "--file", fixture])
        .assert()
        .success()
        .stdout(predicate::str::contains("Imported 2026-08-31 (xlsx)"));

    own(&sb)
        .args(["ownership", "ticker", "aadi"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ADARO STRATEGIC INVESTMENTS"))
        .stdout(predicate::str::contains("as of 2026-08-31"));
    own(&sb)
        .args([
            "-o",
            "json",
            "ownership",
            "ticker",
            "AADI",
            "--as-of",
            "2026-07-31",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema\": \"idx.ownership.holders.v1\"",
        ))
        .stdout(predicate::str::contains("\"as_of\": \"2026-07-31\""));
    own(&sb)
        .args(["ownership", "entity", "alamtri"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ALAMTRI"));
    own(&sb)
        .args(["ownership", "entity", "al"])
        .assert()
        .code(2);
    // No letters or digits: would match every holder, so a usage error.
    own(&sb)
        .args(["ownership", "entity", "%%%"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("3 letters or digits"));
    own(&sb)
        .args([
            "-o",
            "json",
            "ownership",
            "changes",
            "--from",
            "2026-07-31",
            "--to",
            "2026-08-31",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\": \"new\""))
        .stdout(predicate::str::contains("\"kind\": \"exited\""))
        .stdout(predicate::str::contains("\"summary\""))
        .stdout(predicate::str::contains("\"foreign_net_pp\""));
    // Paging: one row from offset 1, summary and total unchanged.
    own(&sb)
        .args([
            "ownership",
            "changes",
            "--from",
            "2026-07-31",
            "--to",
            "2026-08-31",
            "--limit",
            "1",
            "--offset",
            "1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rows 2–2 of"))
        .stdout(predicate::str::contains("--offset 2"));
    own(&sb)
        .args(["ownership", "foreign", "AADI"])
        .assert()
        .success()
        .stdout(predicate::str::contains("disclosed foreign stake"))
        .stdout(predicate::str::contains("2026-08-31"));
    own(&sb)
        .args(["-o", "json", "ownership", "foreign", "--limit", "2"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema\": \"idx.ownership.foreign-flow.v1\"",
        ))
        .stdout(predicate::str::contains("\"from\": \"2026-07-31\""));
    // CSV: the main list of the payload, header first.
    own(&sb)
        .args(["-o", "csv", "ownership", "ticker", "AADI"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "as_of,symbol,issuer_name,holder,",
        ))
        .stdout(predicate::str::contains("2026-08-31,AADI,"));
    own(&sb)
        .args(["-o", "csv", "ownership", "releases"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("as_of,format,source,"));
    own(&sb)
        .args(["ownership", "releases"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2026-07-31"))
        .stdout(predicate::str::contains("2026-08-31"));
    own(&sb)
        .args(["ownership", "path"])
        .assert()
        .success()
        .stdout(format!("{}\n", db.display()));
    // Network modes refuse politely when offline.
    own(&sb)
        .args(["ownership", "import", "--latest"])
        .assert()
        .code(5);
    own(&sb).args(["ownership", "import"]).assert().code(2);
}

#[test]
fn ownership_sums_accounts_dates_rows_and_reads_controller_offline() {
    let sb = sandbox();
    let db = sb.cache.join("accounts.sqlite");
    let fx = |name: &str| {
        format!(
            "{}/tests/fixtures/ownership/{name}",
            env!("CARGO_MANIFEST_DIR")
        )
    };
    let own = |sb: &Sandbox| {
        let mut c = idx(&sb.cache, &sb.config);
        c.env("IDX_OWNERSHIP_DB", &db);
        c
    };
    for f in [
        "above1_20260831_accounts_excerpt.xlsx",
        "above1_20260930_accounts_excerpt.xlsx",
    ] {
        own(&sb)
            .args(["ownership", "import", "--file", &fx(f)])
            .assert()
            .success()
            .stdout(predicate::str::contains("positions"));
    }
    // KRESNA PRIMA INVEST: 9.44 % scrip + 4.01 % scripless, one position.
    own(&sb)
        .args(["-o", "json", "ownership", "ticker", "KREN"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"accounts\": 2"))
        .stdout(predicate::str::contains("\"pct\": 13.45"))
        .stdout(predicate::str::contains("\"controller_reported\": false"));
    own(&sb)
        .args(["ownership", "ticker", "TGUK"])
        .assert()
        .success()
        .stdout(predicate::str::contains("69.33%"))
        .stdout(predicate::str::contains("Accts"))
        .stdout(predicate::str::contains("Control").not());
    // DINASTI moved scrip to scripless with the same shares: no change row.
    own(&sb)
        .args([
            "-o",
            "json",
            "ownership",
            "changes",
            "--from",
            "2026-08-31",
            "--to",
            "2026-09-30",
            "--symbol",
            "TGUK",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"total_matches\": 1"))
        .stdout(predicate::str::contains("BUDIANTO"))
        .stdout(predicate::str::contains("\"accounts_from\": 1"));
    own(&sb)
        .args([
            "ownership",
            "changes",
            "--from",
            "2026-08-31",
            "--to",
            "2026-09-30",
            "--symbol",
            "KREN",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("changed (1→2 accts)"))
        .stdout(predicate::str::contains("+4.01"));
    own(&sb)
        .args(["-o", "json", "ownership", "releases"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"rows\": 46"))
        .stdout(predicate::str::contains("\"positions\": 43"))
        .stdout(predicate::str::contains("\"other_date_rows\": 0"));
    // CSV keeps the old columns first and appends the new ones.
    own(&sb)
        .args(["-o", "csv", "ownership", "ticker", "KREN"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "as_of,symbol,issuer_name,holder,",
        ))
        .stdout(predicate::str::contains(
            ",shares_total,pct,accounts,controller_or_affiliate\n",
        ));

    // Rows dated otherwise: kept, left out, warned about.
    own(&sb)
        .args([
            "ownership",
            "import",
            "--file",
            &fx("above1_mixed_dates_synthetic.xlsx"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 rows dated otherwise left out"))
        .stderr(predicate::str::contains(
            "2 rows dated 2026-01-30 (AAAA, CCCC)",
        ));
    own(&sb)
        .args(["ownership", "ticker", "CCCC", "--as-of", "2026-02-27"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("only on rows dated 2026-01-30"));
    own(&sb)
        .args(["ownership", "ticker", "AAAA", "--as-of", "2026-02-27"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 listed = 40.00%"))
        .stdout(predicate::str::contains("EMPAT JAYA").not())
        .stderr(predicate::str::contains("2026-01-30"));

    // Numbers stored as text in Indonesian notation.
    own(&sb)
        .args([
            "ownership",
            "import",
            "--file",
            &fx("above1_text_numbers_synthetic.xlsx"),
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("row 12: PERCENTAGE \"100,5\""))
        .stderr(predicate::str::contains(
            "row 13: TOTAL_HOLDING_SHARES \"10,5\"",
        ));
    own(&sb)
        .args([
            "-o",
            "json",
            "ownership",
            "ticker",
            "TXTA",
            "--as-of",
            "2026-03-31",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"pct\": 7.19"))
        .stdout(predicate::str::contains("\"shares_total\": 1234567"));

    // The 30 Oct 2026 register carries CONTROLLER_OR_AFFILIATE.
    own(&sb)
        .args([
            "ownership",
            "import",
            "--file",
            &fx("above1_20261030_excerpt.xlsx"),
        ])
        .assert()
        .success();
    own(&sb)
        .args(["-o", "json", "ownership", "ticker", "ZYRX"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"controller_reported\": true"))
        .stdout(predicate::str::contains(
            "\"controller_or_affiliate\": \"CONTROLLER\"",
        ));
    own(&sb)
        .args(["ownership", "ticker", "BBCA"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Control"))
        .stdout(predicate::str::contains("affiliate"));
    own(&sb)
        .args(["ownership", "entity", "%%%"])
        .assert()
        .code(2);
    // The new fields are part of the published schema.
    idx(&sb.cache, &sb.config)
        .args(["schema", "ownership-holders"])
        .assert()
        .success()
        .stdout(predicate::str::contains("controller_or_affiliate"))
        .stdout(predicate::str::contains("controller_reported"))
        .stdout(predicate::str::contains("accounts"));
}

#[test]
fn mcp_setup_prints_recipes_offline() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["mcp", "setup", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("muse"))
        .stdout(predicate::str::contains("codex"))
        .stdout(predicate::str::contains("chatgpt"));
    let out = idx(&sb.cache, &sb.config)
        .args(["mcp", "setup", "codex"])
        .assert()
        .success()
        .stdout(predicate::str::contains("codex mcp add idx -- "))
        .stdout(predicate::str::contains("[mcp_servers.idx]"))
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("idx.exe") || text.contains("/idx"),
        "absolute binary path expected:\n{text}"
    );
    idx(&sb.cache, &sb.config)
        .args(["-o", "json", "mcp", "setup", "muse"])
        .assert()
        .success()
        .stdout(predicate::str::contains("schema_version"))
        .stdout(predicate::str::contains("mcp_servers"))
        .stdout(predicate::str::contains("\"client\": \"muse\""));
    idx(&sb.cache, &sb.config)
        .args([
            "mcp",
            "setup",
            "cursor",
            "--http",
            "https://h.example/mcp",
            "--token",
            "t0k",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Bearer t0k"));
    idx(&sb.cache, &sb.config)
        .args(["mcp", "setup", "nope"])
        .assert()
        .code(2);
    // Binding a public address without a token is refused before anything listens.
    idx(&sb.cache, &sb.config)
        .args(["mcp", "--http", "0.0.0.0:0"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("--token"));
}

#[test]
fn watchlist_roundtrip() {
    let sb = sandbox();
    let file = sb.cache.join("watchlist.txt");
    let w = |sb: &Sandbox| {
        let mut c = idx(&sb.cache, &sb.config);
        c.env("IDX_WATCHLIST", &file);
        c
    };
    w(&sb)
        .args(["watch"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("watch add"));
    w(&sb)
        .args(["watch", "add", "bbca", "BBRI,tlkm"])
        .assert()
        .success()
        .stdout(predicate::str::contains("BBCA BBRI TLKM"));
    w(&sb)
        .args(["watch", "rm", "BBRI"])
        .assert()
        .success()
        .stdout(predicate::str::contains("BBCA TLKM"));
    w(&sb)
        .args(["-o", "json", "watch", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema\": \"idx.watchlist.v1\""))
        .stdout(predicate::str::contains("\"TLKM\""));
    assert!(file.exists());
    // Quoting the list goes through the provider: offline + empty cache = 5.
    w(&sb).args(["--offline", "watch"]).assert().code(5);
    w(&sb)
        .args(["watch", "clear"])
        .assert()
        .success()
        .stdout(predicate::str::contains("empty"));
}

#[test]
fn cache_commands() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["cache", "path"])
        .assert()
        .success()
        .stdout(format!("{}\n", sb.cache.display()));
    idx(&sb.cache, &sb.config)
        .args(["cache", "stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("files : 0"));
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "cache", "clear"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["removed"], 0);
}

#[test]
fn completions_generate() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("_idx"));
    idx(&sb.cache, &sb.config)
        .args(["completions", "powershell"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Register-ArgumentCompleter"));
}

// ---- output formats, stdin symbols, --quiet, exchange suffix ----------------

const YAHOO_QUOTE_3: &str =
    "https://query1.finance.yahoo.com/v7/finance/quote?symbols=BBCA.JK,BBRI.JK,TLKM.JK";
const YAHOO_QUOTE_PARTIAL: &str =
    "https://query1.finance.yahoo.com/v7/finance/quote?symbols=BBCA.JK,ZZZZ.JK";
const YAHOO_CHART_BBCA_1MO: &str =
    "https://query1.finance.yahoo.com/v8/finance/chart/BBCA.JK?range=1mo&interval=1d&includePrePost=false";

/// A sandbox whose cache holds Yahoo's quotes for BBCA, BBRI and TLKM, the
/// partial BBCA + ZZZZ answer and BBCA's one-month daily chart.
fn seeded() -> Sandbox {
    let sb = sandbox();
    seed(
        &sb.cache,
        YAHOO_QUOTE_3,
        "yahoo/quote_v7_bbca_bbri_tlkm.json",
    );
    seed(
        &sb.cache,
        YAHOO_QUOTE_PARTIAL,
        "yahoo/quote_v7_partial_bbca_zzzz.json",
    );
    seed(
        &sb.cache,
        YAHOO_CHART_BBCA_1MO,
        "yahoo/chart_bbca_1mo_1d.json",
    );
    sb
}

/// Every stdout line parsed as JSON; fails on a line that is not.
fn json_lines(stdout: &[u8]) -> Vec<serde_json::Value> {
    String::from_utf8(stdout.to_vec())
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{l:?}: {e}")))
        .collect()
}

#[test]
fn jsonl_streams_one_record_per_line() {
    let sb = seeded();
    // Holidays: one line per holiday of the year.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "jsonl", "market", "holidays", "--year", "2026"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let rows = json_lines(&out.stdout);
    assert_eq!(rows.len(), 22);
    assert!(rows.iter().all(|r| r["date"].is_string()));
    assert!(out.stderr.is_empty());
    // IDX_OUTPUT=jsonl is the same as the flag.
    let env = idx(&sb.cache, &sb.config)
        .env("IDX_OUTPUT", "jsonl")
        .args(["market", "holidays", "--year", "2026"])
        .output()
        .unwrap();
    assert_eq!(env.stdout, out.stdout);

    // Quotes: one per line, the same objects -o json carries.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "jsonl", "stocks", "quote", "BBCA", "BBRI", "TLKM"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let rows = json_lines(&out.stdout);
    let syms: Vec<&str> = rows.iter().map(|r| r["symbol"].as_str().unwrap()).collect();
    assert_eq!(syms, ["BBCA", "BBRI", "TLKM"]);
    let json = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "quote", "BBCA", "BBRI", "TLKM"])
        .output()
        .unwrap();
    let env: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(env["data"][1], rows[1]);

    // History: one candle per line.
    let out = idx(&sb.cache, &sb.config)
        .args([
            "-o", "jsonl", "stocks", "history", "BBCA", "--period", "1mo",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let rows = json_lines(&out.stdout);
    assert!(rows.len() > 10, "{}", rows.len());
    assert!(rows
        .iter()
        .all(|r| r["close"].is_number() && r["time"].is_string()));

    // A partial answer: the rows on stdout, the warning on stderr.
    idx(&sb.cache, &sb.config)
        .args(["-o", "jsonl", "stocks", "quote", "BBCA", "ZZZZ"])
        .assert()
        .success()
        .stdout(predicate::function(|s: &str| s.lines().count() == 1))
        .stderr(predicate::str::contains("warning: no data for ZZZZ"));

    // Values without an envelope: one line.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "jsonl", "version"])
        .output()
        .unwrap();
    let rows = json_lines(&out.stdout);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["version"], env!("CARGO_PKG_VERSION"));

    // An error: nothing on stdout, one JSON line on stderr, exit 5.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "jsonl", "stocks", "quote", "ASII"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5));
    assert!(out.stdout.is_empty());
    let err = json_lines(&out.stderr);
    assert_eq!(err.len(), 1);
    assert_eq!(err[0]["error"]["code"], "offline_miss");
    assert_eq!(err[0]["exit_code"], 5);
}

#[test]
fn jsonl_streams_ownership_changes() {
    let sb = sandbox();
    let db = sb.cache.join("own.sqlite");
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/ownership/");
    for f in [
        "above1_20260731_synthetic.xlsx",
        "above1_20260831_excerpt.xlsx",
    ] {
        idx(&sb.cache, &sb.config)
            .env("IDX_OWNERSHIP_DB", &db)
            .args(["ownership", "import", "--file", &format!("{dir}{f}")])
            .assert()
            .success();
    }
    let range = ["--from", "2026-07-31", "--to", "2026-08-31"];
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_OWNERSHIP_DB", &db)
        .args(["-o", "jsonl", "ownership", "changes"])
        .args(range)
        .output()
        .unwrap();
    assert!(out.status.success());
    let rows = json_lines(&out.stdout);
    let json = idx(&sb.cache, &sb.config)
        .env("IDX_OWNERSHIP_DB", &db)
        .args(["-o", "json", "ownership", "changes"])
        .args(range)
        .output()
        .unwrap();
    let env: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let changes = env["data"]["changes"].as_array().unwrap();
    assert!(!changes.is_empty());
    assert_eq!(&rows, changes);
}

#[test]
fn auto_output_is_json_when_piped() {
    let sb = sandbox();
    // assert_cmd captures stdout, so auto resolves to the JSON envelope.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "auto", "market", "holidays", "--year", "2026"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.holidays.v1");
    let env = idx(&sb.cache, &sb.config)
        .env("IDX_OUTPUT", "auto")
        .args(["market", "holidays", "--year", "2026"])
        .output()
        .unwrap();
    assert_eq!(env.stdout, out.stdout);

    // The config file accepts it; `config get` reports setting and result.
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "output", "auto"])
        .assert()
        .success();
    let out = idx(&sb.cache, &sb.config)
        .args(["config", "get", "output"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["output"], "auto");
    let out = idx(&sb.cache, &sb.config)
        .args(["config", "get"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["output"], "json");
    assert_eq!(v["output_setting"], "auto");
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "output", "yaml"])
        .assert()
        .code(6);

    // Errors follow the resolved format too.
    let out = idx(&sb.cache, &sb.config)
        .args(["stocks", "quote", "ZZZZ"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5));
    let e: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["error"]["code"], "offline_miss");
}

#[test]
fn error_format_follows_the_resolved_output() {
    let sb = sandbox();
    // The flag beats IDX_OUTPUT for the error as for the payload.
    idx(&sb.cache, &sb.config)
        .env("IDX_OUTPUT", "json")
        .args(["-o", "table", "stocks", "quote", "ZZZZ"])
        .assert()
        .code(5)
        .stderr(predicate::str::starts_with("error: "))
        .stderr(predicate::str::contains("hint:"));
    // A JSON default in the config file gives JSON errors.
    std::fs::write(&sb.config, "output = \"json\"\n").unwrap();
    let out = idx(&sb.cache, &sb.config)
        .args(["stocks", "quote", "ZZZZ"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(5));
    let e: serde_json::Value = serde_json::from_slice(&out.stderr).unwrap();
    assert_eq!(e["exit_code"], 5);
    // ...and the meta commands follow it as well.
    let out = idx(&sb.cache, &sb.config).arg("version").output().unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    std::fs::remove_file(&sb.config).unwrap();
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_OUTPUT", "json")
        .arg("version")
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_OUTPUT", "json")
        .arg("schema")
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.as_array().unwrap().contains(&"quotes".into()));
    // An invalid IDX_OUTPUT still fails a command (exit 6), reported as text.
    idx(&sb.cache, &sb.config)
        .env("IDX_OUTPUT", "yaml")
        .args(["market", "holidays"])
        .assert()
        .code(6)
        .stderr(predicate::str::starts_with(
            "error: config: unknown output format",
        ));
}

#[test]
fn symbols_from_stdin() {
    let sb = seeded();
    let quoted = |stdin: &str| -> Vec<String> {
        let out = idx(&sb.cache, &sb.config)
            .args(["-o", "jsonl", "stocks", "quote", "-"])
            .write_stdin(stdin)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        json_lines(&out.stdout)
            .iter()
            .map(|r| r["symbol"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(quoted("BBCA\nBBRI, TLKM\n"), ["BBCA", "BBRI", "TLKM"]);
    assert_eq!(
        quoted("# idx watchlist: one ticker per line\r\nBBCA\r\nBBRI\r\nTLKM\r\n"),
        ["BBCA", "BBRI", "TLKM"]
    );
    // idx's own JSON feeds back in, envelope or lines.
    let json = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "quote", "BBCA", "BBRI", "TLKM"])
        .output()
        .unwrap();
    let json = String::from_utf8(json.stdout).unwrap();
    assert_eq!(quoted(&json), ["BBCA", "BBRI", "TLKM"]);
    let lines = idx(&sb.cache, &sb.config)
        .args(["-o", "jsonl", "stocks", "quote", "BBCA", "BBRI", "TLKM"])
        .output()
        .unwrap();
    assert_eq!(
        quoted(&String::from_utf8(lines.stdout).unwrap()),
        ["BBCA", "BBRI", "TLKM"]
    );
    // `-` mixes with arguments, in order, deduplicated.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "jsonl", "stocks", "quote", "BBCA", "-"])
        .write_stdin("BBRI TLKM BBCA")
        .output()
        .unwrap();
    assert_eq!(json_lines(&out.stdout).len(), 3);

    // Usage errors, exit 2, before any request.
    for (args, stdin, needle) in [
        (
            vec!["stocks", "quote", "-"],
            "",
            "no symbols read from stdin",
        ),
        (vec!["stocks", "quote", "-", "-"], "BBCA", "only once"),
        (vec!["stocks", "quote", "-"], "B$CA", "invalid symbol"),
        (
            vec!["stocks", "compare", "-"],
            "{\"symbol\":",
            "does not parse",
        ),
        (
            vec!["bandar", "screen", "--symbols", "-"],
            "\n",
            "no symbols",
        ),
        (vec!["broker", "-"], "# nothing\n", "no symbols"),
    ] {
        idx(&sb.cache, &sb.config)
            .args(&args)
            .write_stdin(stdin)
            .assert()
            .code(2)
            .stderr(predicate::str::contains(needle));
    }
    // Nothing piped at all (stdin closed) is the same usage error.
    idx(&sb.cache, &sb.config)
        .args(["stocks", "quote", "-"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("no symbols read from stdin"));

    // The watchlist takes them too.
    let file = sb.cache.join("watchlist.txt");
    idx(&sb.cache, &sb.config)
        .env("IDX_WATCHLIST", &file)
        .args(["watch", "add", "-"])
        .write_stdin("bbca\nTLKM\n")
        .assert()
        .success()
        .stdout(predicate::str::contains("BBCA TLKM"));
    idx(&sb.cache, &sb.config)
        .env("IDX_WATCHLIST", &file)
        .args(["watch", "rm", "-"])
        .write_stdin("TLKM")
        .assert()
        .success()
        .stdout(predicate::str::contains("(1 tickers)"));
}

#[test]
fn idx_output_piped_back_ignores_the_default_exchange() {
    // idx writes IDX tickers bare, so its own output read back through `-`
    // stays on IDX even with another market as the default for typed codes.
    let sb = seeded();
    let file = sb.cache.join("watchlist.txt");
    let watch = |args: &[&str], stdin: Vec<u8>| -> serde_json::Value {
        let out = idx(&sb.cache, &sb.config)
            .env("IDX_WATCHLIST", &file)
            .env("IDX_EXCHANGE", "SI")
            .arg("-o")
            .arg("json")
            .args(args)
            .write_stdin(stdin)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let add = |args: &[&str]| watch(&[&["watch", "add"], args].concat(), vec![]);
    // Typed arguments follow IDX_EXCHANGE, so the IDX ones say .JK.
    let v = add(&["BBCA.JK", "TLKM.JK", "D05.SI"]);
    assert_eq!(
        v["data"]["symbols"],
        serde_json::json!(["BBCA", "TLKM", "D05.SI"])
    );
    // `-o jsonl watch list | watch rm -` empties the list.
    let listed = idx(&sb.cache, &sb.config)
        .env("IDX_WATCHLIST", &file)
        .args(["-o", "jsonl", "watch", "list"])
        .output()
        .unwrap();
    let v = watch(&["watch", "rm", "-"], listed.stdout);
    assert_eq!(v["data"]["symbols"], serde_json::json!([]));
    // So does the watchlist file itself.
    add(&["BBCA.JK", "TLKM.JK", "D05.SI"]);
    let saved = std::fs::read(&file).unwrap();
    let v = watch(&["watch", "rm", "-"], saved);
    assert_eq!(v["data"]["symbols"], serde_json::json!([]));
    // Typed text follows IDX_EXCHANGE, as the arguments do.
    let v = watch(&["watch", "add", "-"], b"D05\n".to_vec());
    assert_eq!(v["data"]["symbols"], serde_json::json!(["D05.SI"]));

    // A quotes envelope with BBCA quotes IDX's BBCA (the .JK quote is seeded).
    let envelope = r#"{"schema":"idx.quotes.v1","data":[
        {"symbol":"BBCA"},{"symbol":"BBRI"},{"symbol":"TLKM"}]}"#;
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_EXCHANGE", "SI")
        .args(["-o", "jsonl", "stocks", "quote", "-"])
        .write_stdin(envelope)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let symbols: Vec<String> = json_lines(&out.stdout)
        .iter()
        .map(|r| r["symbol"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(symbols, ["BBCA", "BBRI", "TLKM"]);
}

#[test]
fn quiet_silences_notes_but_not_the_payload_or_errors() {
    let sb = seeded();
    let run = |args: &[&str]| idx(&sb.cache, &sb.config).args(args).output().unwrap();
    // Same stdout and exit code, stderr empty only with --quiet.
    let loud = run(&["-o", "csv", "stocks", "quote", "BBCA", "ZZZZ"]);
    let quiet = run(&["-o", "csv", "--quiet", "stocks", "quote", "BBCA", "ZZZZ"]);
    assert!(loud.status.success() && quiet.status.success());
    assert_eq!(loud.stdout, quiet.stdout);
    assert!(String::from_utf8_lossy(&loud.stderr).contains("warning: no data for ZZZZ"));
    assert!(
        quiet.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&quiet.stderr)
    );
    let q = run(&["-o", "jsonl", "-q", "stocks", "quote", "BBCA", "ZZZZ"]);
    assert!(q.stderr.is_empty());
    assert_eq!(json_lines(&q.stdout).len(), 1);
    // The JSON envelope keeps its warnings.
    let out = run(&["-o", "json", "--quiet", "stocks", "quote", "BBCA", "ZZZZ"]);
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["warnings"][0], "no data for ZZZZ");
    // IDX_QUIET=1 does the same; -v overrides it.
    idx(&sb.cache, &sb.config)
        .env("IDX_QUIET", "1")
        .args(["stocks", "quote", "BBCA", "ZZZZ"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    idx(&sb.cache, &sb.config)
        .env("IDX_QUIET", "1")
        .args(["-v", "stocks", "quote", "BBCA", "ZZZZ"])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning: no data for ZZZZ"));
    // Errors and the hint still print.
    idx(&sb.cache, &sb.config)
        .args(["--quiet", "stocks", "quote", "ASII"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("error: "))
        .stderr(predicate::str::contains("hint:"));
    // Confirmations go quiet.
    idx(&sb.cache, &sb.config)
        .env("IDX_QUIET", "1")
        .args(["config", "set", "provider", "yahoo"])
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "provider", "auto"])
        .assert()
        .success()
        .stderr(predicate::str::contains("provider = auto"));
    idx(&sb.cache, &sb.config)
        .args(["--quiet", "cache", "clear"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
    // --quiet with -v is a usage error.
    idx(&sb.cache, &sb.config)
        .args(["-q", "-v", "version"])
        .assert()
        .code(2);
}

const YAHOO_CHART_D05_5D: &str =
    "https://query1.finance.yahoo.com/v8/finance/chart/D05.SI?range=5d&interval=1d&includePrePost=false";

#[test]
fn other_markets_on_the_yahoo_backed_commands() {
    let sb = seeded();
    seed(
        &sb.cache,
        YAHOO_CHART_D05_5D,
        "yahoo/chart_d05si_5d_1d.json",
    );
    let history = |c: &mut Command| -> serde_json::Value {
        let out = c.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    // An explicit suffix.
    let v = history(idx(&sb.cache, &sb.config).args([
        "-o", "json", "stocks", "history", "d05.si", "--period", "5d",
    ]));
    assert_eq!(v["data"]["symbol"], "D05.SI");
    assert_eq!(v["data"]["currency"], "SGD");
    assert_eq!(v["data"]["candles"].as_array().unwrap().len(), 5);
    // A default suffix for bare tickers.
    let v = history(
        idx(&sb.cache, &sb.config)
            .env("IDX_EXCHANGE", "SI")
            .args(["-o", "json", "stocks", "history", "D05", "--period", "5d"]),
    );
    assert_eq!(v["data"]["symbol"], "D05.SI");
    // The confirmation shows the value stored, not the one typed.
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "exchange", "idx"])
        .assert()
        .success()
        .stderr(predicate::str::contains("exchange = JK ("));
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "exchange", ".si"])
        .assert()
        .success()
        .stderr(predicate::str::contains("exchange = SI ("));
    idx(&sb.cache, &sb.config)
        .args(["config", "get", "exchange"])
        .assert()
        .success()
        .stdout("SI\n");
    let v = history(
        idx(&sb.cache, &sb.config)
            .args(["-o", "json", "stocks", "history", "D05", "--period", "5d"]),
    );
    assert_eq!(v["data"]["symbol"], "D05.SI");
    // `.JK` still means IDX whatever the default (the BBCA.JK chart is seeded).
    let v = history(idx(&sb.cache, &sb.config).args([
        "-o", "json", "stocks", "history", "bbca.jk", "--period", "1mo",
    ]));
    assert_eq!(v["data"]["symbol"], "BBCA");
    std::fs::remove_file(&sb.config).unwrap();

    // Quote: v7 is not cached here, so the chart fallback answers.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "quote", "D05.SI"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"][0]["symbol"], "D05.SI");
    assert_eq!(v["data"][0]["currency"], "SGD");
    assert_eq!(v["data"][0]["source"], "yahoo:chart");
    assert!(v["warnings"][0]
        .as_str()
        .unwrap()
        .contains("reduced fields"));

    // IDX-only commands refuse it by name; MSN cannot serve it.
    for args in [
        vec!["broker", "D05.SI"],
        vec!["ownership", "ticker", "D05.SI"],
        vec!["bandar", "detect", "D05.SI"],
        vec!["stocks", "brief", "D05.SI"],
    ] {
        idx(&sb.cache, &sb.config)
            .args(&args)
            .assert()
            .code(2)
            .stderr(predicate::str::contains("another exchange (SI)"));
    }
    idx(&sb.cache, &sb.config)
        .args(["--provider", "msn", "stocks", "quote", "D05.SI"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("outside IDX"));
    // A bad exchange setting is a config error.
    idx(&sb.cache, &sb.config)
        .env("IDX_EXCHANGE", "S1")
        .args(["stocks", "quote", "BBCA"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("exchange"));
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "exchange", "ABCDE"])
        .assert()
        .code(6);
}
