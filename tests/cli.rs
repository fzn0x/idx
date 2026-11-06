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
        .stdout(predicate::str::contains("Exit codes"))
        .stdout(predicate::str::contains("JSON code blocked"));
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
        .args(["stocks", "screen", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Presets: top-performers"))
        .stdout(predicate::str::contains("ret1y"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "screen", "--preset", "nope"])
        .assert()
        .code(2);
    idx(&sb.cache, &sb.config)
        .args(["stocks", "screen", "--asc", "--desc"])
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
fn broker_one_investor_side_carries_its_buying_and_selling() {
    let sb = sandbox();
    seed(
        &sb.cache,
        "https://www.indopremier.com/module/saham/include/data-brokersummary.php?code=BBCA&start=10/16/2026&end=10/16/2026&fd=F&board=all",
        "ipot/brokersummary_bbca_20261016_foreign.html",
    );
    let out = idx(&sb.cache, &sb.config)
        .args([
            "-o",
            "json",
            "broker",
            "BBCA",
            "--date",
            "2026-10-16",
            "--investor",
            "foreign",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let s = &v["data"][0];
    assert_eq!(s["investor"], "foreign");
    assert!(s["total_value"].is_null(), "{s}");
    assert!(s["total_lots"].is_null() && s["average_price"].is_null());
    assert_eq!(s["buy_value"], 1.7e12);
    assert_eq!(s["sell_value"], 4.0e12);
    assert_eq!(s["foreign_net_value"], -2.3e12);
    idx(&sb.cache, &sb.config)
        .args([
            "broker",
            "BBCA",
            "--date",
            "2026-10-16",
            "--investor",
            "foreign",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Foreign investors bought 1.70T · sold 4.00T · foreign net -2.30T",
        ));
    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "broker"])
        .output()
        .unwrap();
    let schema = String::from_utf8_lossy(&out.stdout);
    assert!(schema.contains("buy_value") && schema.contains("sell_value"));
}

#[test]
fn stocks_foreign_from_ipot_in_rupiah() {
    let sb = sandbox();
    for day in ["12", "13", "14", "15", "16"] {
        for (fd, suffix) in [("all", ""), ("F", "_foreign")] {
            seed(
                &sb.cache,
                &format!("https://www.indopremier.com/module/saham/include/data-brokersummary.php?code=BBCA&start=10/{day}/2026&end=10/{day}/2026&fd={fd}&board=all"),
                &format!("ipot/brokersummary_bbca_202610{day}{suffix}.html"),
            );
        }
    }
    let week = [
        "--source",
        "ipot",
        "--from",
        "2026-10-12",
        "--to",
        "2026-10-16",
    ];
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "foreign", "BBCA"])
        .args(week)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.foreign.value.v1");
    assert_eq!(v["provider"], "ipot");
    let data = &v["data"];
    assert_eq!(data["days"].as_array().unwrap().len(), 5);
    assert_eq!(data["rounded"], true);
    assert_eq!(data["totals"]["foreign_buy_value"], 4.3647e12);
    assert_eq!(data["totals"]["foreign_sell_value"], 6.1878e12);
    assert_eq!(data["totals"]["foreign_net_value"], -1.8231e12);
    assert_eq!(data["totals"]["streak"], -1);
    assert_eq!(data["days"][4]["foreign_sell_value"], 4.0e12);

    idx(&sb.cache, &sb.config)
        .args(["stocks", "foreign", "BBCA"])
        .args(week)
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "foreign flow (IPOT, all boards) · 2026-10-12 to 2026-10-16 · 5 sessions",
        ))
        .stdout(predicate::str::contains("foreign share 53.1%"))
        .stdout(predicate::str::contains("1 net-sell session(s) in a row"));
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "csv", "stocks", "foreign", "BBCA"])
        .args(week)
        .output()
        .unwrap();
    let csv = String::from_utf8(out.stdout).unwrap();
    assert!(
        csv.starts_with("date,value,lots,average_price,foreign_buy_value"),
        "{csv}"
    );

    for args in [
        vec![
            "stocks",
            "foreign",
            "BBCA",
            "--days",
            "3",
            "--from",
            "2026-10-12",
        ],
        vec!["stocks", "foreign", "BBCA", "--to", "2026-10-16"],
        vec![
            "stocks",
            "foreign",
            "BBCA",
            "--from",
            "2026-01-01",
            "--to",
            "2099-01-01",
        ],
        vec![
            "stocks",
            "foreign",
            "BBCA",
            "--from",
            "2026-10-16",
            "--to",
            "2026-10-12",
        ],
        vec![
            "stocks", "foreign", "BBCA", "--source", "ipot", "--days", "61",
        ],
        vec!["stocks", "foreign", "BBCA", "--source", "rti"],
        vec!["stocks", "foreign", "^JKSE", "--source", "ipot"],
    ] {
        idx(&sb.cache, &sb.config).args(&args).assert().code(2);
    }
    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "foreign-value"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let schema = String::from_utf8_lossy(&out.stdout);
    assert!(schema.contains("foreign_buy_value") && schema.contains("foreign_share_pct"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "foreign", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--source ipot"));
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
    // MSN does not serve dividends: explicit provider gets exit 6 with a hint.
    // (History is routed to Yahoo; see history_provider_routes_candles.)
    idx(&sb.cache, &sb.config)
        .args(["--provider", "msn", "stocks", "dividends", "BBCA"])
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

const CHART: &str = "https://query1.finance.yahoo.com/v8/finance/chart";
const TIMESERIES: &str =
    "https://query2.finance.yahoo.com/ws/fundamentals-timeseries/v1/finance/timeseries";

/// Seeds what the lenses read for one ticker, recorded 2026-11-06: the
/// quoteSummary fundamentals, the annual statements, and a year of daily
/// candles for the ticker and (unless `with_index` is false) for IHSG.
fn seed_lens(cache: &std::path::Path, sym: &str, statements: &str, with_index: bool) {
    let lower = sym.to_ascii_lowercase();
    seed(
        cache,
        &format!("{QUOTE_SUMMARY}/{sym}.JK?{FUNDAMENTAL_MODULES}"),
        &format!("yahoo/quote_summary_fundamentals_{lower}.json"),
    );
    seed(
        cache,
        &format!(
            "{TIMESERIES}/{sym}.JK?type={}",
            idx::providers::yahoo::timeseries::types_param(idx::model::StatementPeriod::Annual)
        ),
        &format!("yahoo/{statements}"),
    );
    seed(
        cache,
        &format!("{CHART}/{sym}.JK?range=1y&interval=1d&includePrePost=false"),
        &format!("yahoo/chart_{lower}_1y_1d.json"),
    );
    if with_index {
        seed(
            cache,
            &format!("{CHART}/^JKSE?range=1y&interval=1d&includePrePost=false"),
            "yahoo/chart_jkse_1y_1d.json",
        );
    }
}

fn json_of(cache: &std::path::Path, config: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let out = idx(cache, config).args(args).output().unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap()
}

fn rule_ids(reading: &serde_json::Value) -> Vec<(String, i64)> {
    reading["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            (
                r["rule"].as_str().unwrap().to_string(),
                r["points"].as_i64().unwrap(),
            )
        })
        .collect()
}

#[test]
fn lens_commands_have_help_and_schemas() {
    let sb = sandbox();
    for (cmd, needle) in [
        ("growth", "idx stocks growth BBCA"),
        ("valuation", "idx stocks valuation ITMG"),
        ("risk", "idx stocks risk GIAA"),
    ] {
        idx(&sb.cache, &sb.config)
            .args(["stocks", cmd, "--help"])
            .assert()
            .success()
            .stdout(predicate::str::contains(needle))
            .stdout(predicate::str::contains("Heuristic"));
        let v = json_of(&sb.cache, &sb.config, &["schema", cmd]);
        assert!(v["$schema"].as_str().unwrap().contains("json-schema.org"));
        assert!(v.to_string().contains("inputs_available"), "{cmd}");
    }
    idx(&sb.cache, &sb.config)
        .args(["stocks", "fundamental", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--lens <LENS>"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "fundamental", "BBCA", "--lens", "bogus"])
        .assert()
        .code(2);
    // Nothing cached: the lenses fail like the other stock commands.
    for cmd in ["growth", "valuation", "risk"] {
        idx(&sb.cache, &sb.config)
            .args(["--offline", "stocks", cmd, "BBCA"])
            .assert()
            .code(5)
            .stderr(predicate::str::contains("BBCA"));
    }
}

#[test]
fn risk_lens_reads_a_year_against_ihsg() {
    let sb = sandbox();
    seed_lens(&sb.cache, "TLKM", "timeseries_tlkm.json", true);
    let v = json_of(
        &sb.cache,
        &sb.config,
        &["-o", "json", "stocks", "risk", "TLKM"],
    );
    assert_eq!(v["schema"], "idx.risk.v1");
    assert!(v.get("warnings").is_none(), "{v}");
    let d = &v["data"];
    assert_eq!(d["reading"]["label"], "moderate");
    assert_eq!(d["reading"]["score"], 2);
    assert_eq!(
        rule_ids(&d["reading"]),
        [
            ("leverage".to_string(), 0),
            ("liquidity".into(), 1),
            ("losses".into(), 0),
            ("beta".into(), 0),
            ("volatility".into(), 1),
            ("drawdown".into(), 0),
        ]
    );
    assert_eq!(d["price"]["benchmark"], "^JKSE");
    assert_eq!(d["price"]["observations"], 238);
    assert_eq!(d["price"]["zero_returns"], 19);
    assert_eq!(d["price"]["to"], "2026-11-04");
    assert!((d["price"]["beta"].as_f64().unwrap() - 1.122).abs() < 0.003);

    // --lens on `stocks fundamental` is the same reading.
    let same = json_of(
        &sb.cache,
        &sb.config,
        &[
            "-o",
            "json",
            "stocks",
            "fundamental",
            "TLKM",
            "--lens",
            "Risk",
        ],
    );
    assert_eq!(same["schema"], "idx.risk.v1", "--lens ignores case");
    assert_eq!(same["data"], v["data"]);

    idx(&sb.cache, &sb.config)
        .args(["stocks", "risk", "TLKM"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TLKM · "))
        .stdout(predicate::str::contains("PRICE VS ^JKSE (1Y DAILY)"))
        .stdout(predicate::str::contains("238 daily returns"))
        .stdout(predicate::str::contains("19 of 238 (close unchanged)"))
        .stdout(predicate::str::contains("MODERATE (score +2, 6 rules)"))
        .stdout(predicate::str::contains(
            "  +1  volatility 49.2% a year is 1.68x IHSG's 29.3% (over 1.5x)",
        ))
        .stdout(predicate::str::contains("Points add risk."));
}

#[test]
fn risk_lens_without_ihsg_keeps_the_balance_sheet() {
    let sb = sandbox();
    seed_lens(&sb.cache, "TLKM", "timeseries_tlkm.json", false);
    let v = json_of(
        &sb.cache,
        &sb.config,
        &[
            "--provider",
            "yahoo",
            "-o",
            "json",
            "stocks",
            "risk",
            "TLKM",
        ],
    );
    let warnings = v["warnings"].to_string();
    assert!(
        warnings.contains("benchmark ^JKSE unavailable"),
        "{warnings}"
    );
    let d = &v["data"];
    assert!(d["price"]["beta"].is_null());
    assert!(d["price"]["volatility_pct"].as_f64().unwrap() > 40.0);
    assert_eq!(d["reading"]["inputs_available"], 3);
    assert!(d["reading"]["notes"]
        .to_string()
        .contains("benchmark ^JKSE unavailable"));
}

#[test]
fn growth_and_valuation_lenses_offline() {
    let sb = sandbox();
    seed_lens(&sb.cache, "TLKM", "timeseries_tlkm.json", true);
    let g = json_of(
        &sb.cache,
        &sb.config,
        &["-o", "json", "stocks", "growth", "TLKM"],
    );
    assert_eq!(g["schema"], "idx.growth.v1");
    assert_eq!(g["data"]["reading"]["label"], "mixed");
    assert_eq!(g["data"]["annual"].as_array().unwrap().len(), 4);
    assert_eq!(g["data"]["sources"][1], "yahoo:timeseries");
    idx(&sb.cache, &sb.config)
        .args(["stocks", "growth", "TLKM"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Fiscal year"))
        .stdout(predicate::str::contains("MIXED (score +0, 6 rules)"))
        .stdout(predicate::str::contains("Points favour growth."));

    let v = json_of(
        &sb.cache,
        &sb.config,
        &["-o", "json", "stocks", "valuation", "TLKM"],
    );
    assert_eq!(v["schema"], "idx.valuation.v1");
    assert_eq!(v["data"]["reading"]["label"], "fair");
    let same = json_of(
        &sb.cache,
        &sb.config,
        &[
            "-o",
            "json",
            "stocks",
            "fundamental",
            "TLKM",
            "--lens",
            "valuation",
        ],
    );
    assert_eq!(same["data"], v["data"]);
    idx(&sb.cache, &sb.config)
        .args(["stocks", "valuation", "TLKM"])
        .assert()
        .success()
        .stdout(predicate::str::contains("EV/EBITDA"))
        .stdout(predicate::str::contains("FAIR (score +2, 7 rules)"))
        .stdout(predicate::str::contains(
            "dividend yield 10.0% is high, but the payout (125%) exceeds earnings",
        ));
}

#[test]
fn lenses_on_negative_equity_and_usd_statements() {
    // GIAA: negative equity, losses, a weak year against IHSG.
    let sb = sandbox();
    seed_lens(&sb.cache, "GIAA", "timeseries_annual_giaa.json", true);
    let args = ["--provider", "yahoo", "-o", "json", "stocks"];
    let r = json_of(
        &sb.cache,
        &sb.config,
        &[&args[..], &["risk", "GIAA"]].concat(),
    );
    assert_eq!(r["data"]["reading"]["label"], "high");
    assert_eq!(r["data"]["reading"]["score"], 6);
    assert_eq!(r["data"]["negative_equity"], true);
    let g = json_of(
        &sb.cache,
        &sb.config,
        &[&args[..], &["growth", "GIAA"]].concat(),
    );
    assert_eq!(g["data"]["statements_currency"], "USD");
    assert!(g["data"]["reading"]["rules"]
        .to_string()
        .contains("net income turned from a profit in FY2022 to a loss in FY2025"));

    // ITMG: USD statements, so P/B, P/S and EV/EBITDA are shown, not scored.
    seed_lens(&sb.cache, "ITMG", "timeseries_annual_itmg.json", true);
    let v = json_of(
        &sb.cache,
        &sb.config,
        &[&args[..], &["valuation", "ITMG"]].concat(),
    );
    let d = &v["data"];
    assert_eq!(d["reading"]["label"], "cheap");
    assert_eq!(d["financial_currency"], "USD");
    assert!(
        d["price_to_book"].as_f64().unwrap() > 14000.0,
        "as reported"
    );
    assert_eq!(
        rule_ids(&d["reading"]),
        [
            ("pe_trailing".to_string(), 2),
            ("peg".into(), 1),
            ("dividend_yield".into(), 1),
            ("analyst_target".into(), 0),
        ]
    );
    let notes = d["reading"]["notes"].to_string();
    assert!(
        notes.contains("P/B 14103.31 not scored: statements in USD, price in IDR"),
        "{notes}"
    );
    idx(&sb.cache, &sb.config)
        .args(["--provider", "yahoo", "stocks", "valuation", "ITMG"])
        .assert()
        .success()
        .stdout(predicate::str::contains("14,103.31*"))
        .stdout(predicate::str::contains(
            "* Statements in USD, price in IDR",
        ));
    let r = json_of(
        &sb.cache,
        &sb.config,
        &[&args[..], &["risk", "ITMG"]].concat(),
    );
    assert_eq!(r["data"]["reading"]["label"], "low");
}

#[test]
fn ownership_institutions_reads_msn_offline() {
    let sb = sandbox();
    // Cache keys leave MSN's api key out, so the seeded entries are what a
    // live run would have stored.
    let lookup =
        "https://services.bingapis.com/contentservices-finance.csautosuggest/api/v1/Query?query=";
    seed(
        &sb.cache,
        &format!("{lookup}JTPE&market=en-id"),
        "msn/autosuggest_jtpe.json",
    );
    seed(
        &sb.cache,
        "https://assets.msn.com/service/Finance/Equities?ids=bkgwu2",
        "msn/equities_jtpe_all_signals.json",
    );
    seed(
        &sb.cache,
        &format!("{lookup}ATAP&market=en-id"),
        "msn/autosuggest_atap.json",
    );
    seed(
        &sb.cache,
        "https://assets.msn.com/service/Finance/Equities?ids=bwzup2",
        "msn/equities_atap_empty_holders.json",
    );

    idx(&sb.cache, &sb.config)
        .args(["ownership", "institutions", "jtpe"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "JTPE · PT Jasuindo Tiga Perkasa Tbk · institutional and fund holders (MSN)",
        ))
        .stdout(predicate::str::contains(
            "Funds (5): 1 new, 0 buying, 1 selling, 2 exited",
        ))
        .stdout(predicate::str::contains("exited"));

    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "ownership", "institutions", "JTPE"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.ownership.institutions.v1");
    assert_eq!(v["provider"], "msn");
    assert_eq!(v["cached"], true);
    assert_eq!(v["data"]["flow"]["funds"]["exited"], 2);
    assert_eq!(
        v["data"]["flow"]["institutions"]["net_shares_change"],
        -3_556_219
    );
    assert_eq!(v["data"]["holders"].as_array().unwrap().len(), 9);
    assert_eq!(v["data"]["holders"][0]["kind"], "institution");
    assert!(v["data"]["holders"][0]["pct_portfolio"].is_null());
    assert_eq!(v["data"]["holders"][6]["signal"], "new");
    assert!(v.get("warnings").is_none());

    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "csv", "ownership", "institutions", "JTPE"])
        .output()
        .unwrap();
    let csv = String::from_utf8(out.stdout).unwrap();
    assert!(
        csv.starts_with("kind,rank,name,shares,shares_change,pct_outstanding,"),
        "{csv}"
    );
    assert_eq!(csv.lines().count(), 10, "header plus every holder: {csv}");

    // MSN lists nobody: an empty report, a warning, exit 0.
    idx(&sb.cache, &sb.config)
        .args(["ownership", "institutions", "ATAP"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "MSN lists no institutional or fund holders for ATAP.",
        ))
        .stderr(predicate::str::contains(
            "warning: MSN lists no institutional",
        ));
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "ownership", "institutions", "ATAP"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["holders"].as_array().unwrap().len(), 0);
    assert_eq!(v["data"]["float_pct"], 15.0635);
    assert!(v["data"]["institutions_pct"].is_null());
    assert_eq!(
        v["warnings"][0],
        "MSN lists no institutional or fund holders for ATAP"
    );
    // CSV keeps the holder layout: the same header as JTPE, no rows.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "csv", "ownership", "institutions", "ATAP"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("{}\n", csv.lines().next().unwrap())
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("warning: MSN lists no institutional"));

    // Nothing cached: offline miss. A provider without the data: exit 6.
    idx(&sb.cache, &sb.config)
        .args(["ownership", "institutions", "BBRI"])
        .assert()
        .code(5);
    idx(&sb.cache, &sb.config)
        .args(["--provider", "yahoo", "ownership", "institutions", "JTPE"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("institutional holders"));
    idx(&sb.cache, &sb.config)
        .args(["ownership", "institutions", "BB$CA"])
        .assert()
        .code(2);
    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "ownership-institutions"])
        .output()
        .unwrap();
    let schema = String::from_utf8_lossy(&out.stdout);
    assert!(schema.contains("pct_outstanding_change"), "{schema}");
    assert!(schema.contains("unchanged"), "{schema}");
}

#[test]
fn screen_presets_offline() {
    let sb = sandbox();
    let dir = sb.config.parent().unwrap();
    let universe = dir.join("u.csv");
    std::fs::write(&universe, "symbol,name\nBBCA,\nBBRI,\nTLKM,\n").unwrap();
    let u = universe.to_str().unwrap();
    // Keep the developer's own ownership database out of the universe.
    let db = dir.join("none.sqlite");
    let run = |args: &[&str]| {
        let out = idx(&sb.cache, &sb.config)
            .env("IDX_OWNERSHIP_DB", &db)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out.stdout
    };
    let json = |args: &[&str]| -> serde_json::Value { serde_json::from_slice(&run(args)).unwrap() };
    let symbols = |v: &serde_json::Value| -> Vec<String> {
        v["data"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["symbol"].as_str().unwrap().to_string())
            .collect()
    };
    seed(
        &sb.cache,
        "https://query1.finance.yahoo.com/v7/finance/quote?symbols=BBCA.JK,BBRI.JK,TLKM.JK",
        "yahoo/quote_v7_bbca_bbri_tlkm.json",
    );

    // Presets that need no return windows run on the quotes alone.
    let v = json(&[
        "-o",
        "json",
        "stocks",
        "screen",
        "--preset",
        "low-pe",
        "--universe",
        u,
    ]);
    assert_eq!(v["data"]["preset"], "low-pe");
    assert_eq!(v["data"]["filters"], serde_json::json!(["pe>0"]));
    assert_eq!(v["data"]["sort"], "pe");
    assert_eq!(v["data"]["descending"], false);
    assert!(v["data"]["returns_source"].is_null());
    assert_eq!(symbols(&v), ["BBRI", "BBCA", "TLKM"]);
    // TLKM closed at its 52-week low.
    let v = json(&[
        "-o",
        "json",
        "stocks",
        "screen",
        "--preset",
        "52w-low",
        "--universe",
        u,
    ]);
    assert_eq!(symbols(&v), ["TLKM"]);
    assert_eq!(v["data"]["rows"][0]["range_52w_pct"], 0.0);
    // No preset and no filter: the largest companies.
    let v = json(&["-o", "json", "stocks", "screen", "--universe", u]);
    assert_eq!(symbols(&v), ["BBCA", "BBRI", "TLKM"]);
    assert_eq!(v["data"]["sort"], "mcap");

    // A return key without the market list cached: an offline miss.
    idx(&sb.cache, &sb.config)
        .env("IDX_OWNERSHIP_DB", &db)
        .args([
            "stocks",
            "screen",
            "--preset",
            "top-performers",
            "--universe",
            u,
        ])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("MSN market list"));
    // --returns is optional: a warning, the rows stay.
    let v = json(&[
        "-o",
        "json",
        "stocks",
        "screen",
        "--returns",
        "--universe",
        u,
    ]);
    assert_eq!(symbols(&v).len(), 3);
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w
        .as_str()
        .unwrap()
        .starts_with("return windows unavailable")));

    seed(
        &sb.cache,
        "https://assets.msn.com/service/Finance/Screener?region=id&values=quote",
        "msn/screener_region_id_limit12.json",
    );
    let v = json(&[
        "-o",
        "json",
        "stocks",
        "screen",
        "--preset",
        "top-performers",
        "--universe",
        u,
    ]);
    assert_eq!(v["data"]["preset"], "top-performers");
    assert_eq!(v["data"]["returns_source"], "msn:screener");
    assert_eq!(v["data"]["sort"], "ret1y");
    assert_eq!(symbols(&v), ["BBRI", "BBCA", "TLKM"]);
    let y1: Vec<f64> = v["data"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["return_1y_pct"].as_f64().unwrap())
        .collect();
    assert_eq!(y1, [-31.9095, -41.092, -43.7853]);
    assert!(v["warnings"].as_array().unwrap().iter().any(|w| w
        .as_str()
        .unwrap()
        .starts_with("MSN listed 12 of 887 stocks")));

    let v = json(&[
        "-o",
        "json",
        "stocks",
        "screen",
        "--preset",
        "top-performers",
        "--sort",
        "ret3m",
        "--universe",
        u,
    ]);
    assert_eq!(symbols(&v), ["BBRI", "BBCA", "TLKM"]);
    assert_eq!(v["data"]["rows"][2]["return_3m_pct"], -26.5683);
    let v = json(&[
        "-o",
        "json",
        "stocks",
        "screen",
        "-f",
        "ret1y<-40",
        "--asc",
        "--sort",
        "ytd",
        "--universe",
        u,
    ]);
    assert_eq!(symbols(&v), ["TLKM", "BBCA"]);
    assert_eq!(v["data"]["descending"], false);

    let table = String::from_utf8(run(&[
        "stocks",
        "screen",
        "--preset",
        "top-performers",
        "--sort",
        "ret3m",
        "--universe",
        u,
    ]))
    .unwrap();
    assert!(table.contains("Screen top-performers"), "{table}");
    assert!(table.contains("3M %"), "{table}");
    assert!(table.contains("-10.26%"), "{table}");
    assert!(table.contains("as reported by msn:screener"), "{table}");
    let table = String::from_utf8(run(&[
        "stocks",
        "screen",
        "--preset",
        "52w-low",
        "--universe",
        u,
    ]))
    .unwrap();
    assert!(table.contains("Screen 52w-low: range<=5"), "{table}");
    assert!(table.contains("52w pos"), "{table}");

    // Yahoo alone has no market-wide return windows: unsupported, exit 6.
    idx(&sb.cache, &sb.config)
        .env("IDX_OWNERSHIP_DB", &db)
        .args([
            "--provider",
            "yahoo",
            "stocks",
            "screen",
            "--preset",
            "top-performers",
            "--universe",
            u,
        ])
        .assert()
        .code(6);
}

#[test]
fn insights_offline() {
    let sb = sandbox();
    idx(&sb.cache, &sb.config)
        .args(["stocks", "insights", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MSN Money"))
        .stdout(predicate::str::contains("compared_with"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "sentiment", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("crowd sentiment is not used"));
    // Nothing cached yet: an offline miss naming the ticker.
    idx(&sb.cache, &sb.config)
        .args(["stocks", "insights", "BBCA"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("BBCA"));

    seed(
        &sb.cache,
        "https://services.bingapis.com/contentservices-finance.csautosuggest/api/v1/Query?query=BBCA&market=en-id",
        "msn/autosuggest_bbca.json",
    );
    seed(
        &sb.cache,
        "https://api.msn.com/msn/v0/pages/finance/insights?ids=bn91jc",
        "msn/insights_bbca.json",
    );
    // Peers not cached: the report still comes, with a warning and no peers.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "insights", "BBCA"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["peers"].as_array().unwrap().len(), 0);
    assert!(v["warnings"][0]
        .as_str()
        .unwrap()
        .contains("could not resolve MSN's peer list"));

    seed(
        &sb.cache,
        "https://assets.msn.com/service/Finance/Quotes?ids=bkfmar,bkfgqh,cj53lh,bkgttc",
        "msn/quotes_bbca_insight_peers.json",
    );
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "insights", "bbca"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.insights.v1");
    assert_eq!(v["provider"], "msn");
    assert!(v.get("warnings").is_none(), "{v}");
    let d = &v["data"];
    assert_eq!(d["symbol"], "BBCA");
    assert_eq!(d["good"], 17);
    assert_eq!(d["bad"], 4);
    assert_eq!(d["not_rated"], 9);
    assert_eq!(d["updated"], "2026-11-05T17:51:31Z");
    assert_eq!(
        d["peers"],
        serde_json::json!(["BBKP", "BABP", "SUPA", "INPC"])
    );
    assert_eq!(d["insights"].as_array().unwrap().len(), 30);
    let pb = &d["insights"][2];
    assert_eq!(pb["name"], "P/B ratio");
    assert_eq!(pb["status"], "bad");
    assert_eq!(pb["criteria"], "lower_is_better");
    assert_eq!(pb["compared_with"], "peers");

    let table = idx(&sb.cache, &sb.config)
        .args(["stocks", "insights", "BBCA"])
        .output()
        .unwrap();
    let text = String::from_utf8(table.stdout).unwrap();
    assert!(text.contains("VALUATION"), "{text}");
    assert!(text.contains("peers BBKP, BABP, SUPA, INPC"), "{text}");
    assert!(
        text.contains("9 statements MSN could not rate are hidden"),
        "{text}"
    );
    assert!(
        !text.contains("Unable to compare with industry average"),
        "{text}"
    );
    let all = idx(&sb.cache, &sb.config)
        .args(["stocks", "insights", "BBCA", "--all"])
        .output()
        .unwrap();
    let text = String::from_utf8(all.stdout).unwrap();
    assert!(
        text.contains("Unable to compare with industry average"),
        "{text}"
    );
    let csv = idx(&sb.cache, &sb.config)
        .args(["-o", "csv", "stocks", "insights", "BBCA"])
        .output()
        .unwrap();
    let csv = String::from_utf8(csv.stdout).unwrap();
    assert!(csv.starts_with("category,name,statement"), "{csv}");

    // Stockbit serves no insights: exit 6.
    idx(&sb.cache, &sb.config)
        .args(["--provider", "stockbit", "stocks", "insights", "BBCA"])
        .assert()
        .code(6);
    let schema = idx(&sb.cache, &sb.config)
        .args(["schema", "insights"])
        .output()
        .unwrap();
    assert!(schema.status.success());
    let s = String::from_utf8_lossy(&schema.stdout);
    assert!(
        s.contains("compared_with") && s.contains("not_rated"),
        "{s}"
    );
}

#[test]
fn earnings_filters_apply_to_every_format() {
    let sb = sandbox();
    seed(
        &sb.cache,
        "https://query2.finance.yahoo.com/v10/finance/quoteSummary/TLKM.JK?modules=earnings,earningsHistory,earningsTrend,calendarEvents,price",
        "yahoo/quote_summary_earnings_tlkm.json",
    );
    let run = |args: &[&str]| -> std::process::Output {
        let out = idx(&sb.cache, &sb.config).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    };
    let json = |extra: &[&str]| -> serde_json::Value {
        let mut args = vec!["-o", "json", "stocks", "earnings", "TLKM"];
        args.extend_from_slice(extra);
        serde_json::from_slice(&run(&args).stdout).unwrap()
    };
    let periods = |v: &serde_json::Value| -> Vec<String> {
        v["data"]["estimates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["period"].as_str().unwrap().to_string())
            .collect()
    };

    // Unfiltered: every row, the four consensus rows, scope null.
    let v = json(&[]);
    assert_eq!(periods(&v), ["0q", "+1q", "0y", "+1y"]);
    assert_eq!(v["data"]["quarters"].as_array().unwrap().len(), 4);
    assert!(v["data"]["scope"].is_null());
    assert!(v["data"]["next"].is_object());

    // --annual: the yearly rows and the 0y/+1y consensus, next null, scope set.
    let v = json(&["--annual"]);
    let d = &v["data"];
    assert_eq!(d["quarters"], serde_json::json!([]));
    assert_eq!(d["quarterly"], serde_json::json!([]));
    assert_eq!(d["yearly"].as_array().unwrap().len(), 4);
    assert_eq!(periods(&v), ["0y", "+1y"]);
    assert!(d["next"].is_null());
    assert_eq!(
        d["scope"],
        serde_json::json!({"annual": true, "quarterly": false, "reported": true, "estimates": true})
    );
    assert!(d["ex_dividend_date"].is_string());

    // The reference CLI's names are visible aliases.
    let v = json(&["--forecast"]);
    assert_eq!(v["data"]["quarters"], serde_json::json!([]));
    assert_eq!(periods(&v).len(), 4);
    assert!(v["data"]["next"].is_object());
    let v = json(&["--history", "--quarterly"]);
    assert_eq!(v["data"]["estimates"], serde_json::json!([]));
    assert_eq!(v["data"]["yearly"], serde_json::json!([]));
    assert_eq!(v["data"]["quarterly"].as_array().unwrap().len(), 4);
    // Both flags of a pair: no filter.
    let v = json(&["--annual", "--quarterly", "--reported", "--estimates"]);
    assert!(v["data"]["scope"].is_null());

    // Table: the quarter consensus only.
    let out = run(&["stocks", "earnings", "TLKM", "--quarterly", "--estimates"]);
    let t = String::from_utf8(out.stdout).unwrap();
    assert!(t.contains("showing quarterly estimates"), "{t}");
    assert!(t.contains("this quarter (0q)"), "{t}");
    assert!(t.contains("next quarter (+1q)"), "{t}");
    assert!(!t.contains("this year"), "{t}");
    assert!(!t.contains("2022"), "{t}");
    assert!(t.contains("Next earnings"), "{t}");

    // CSV: the filtered payload's main list is the consensus.
    let out = run(&[
        "-o",
        "csv",
        "stocks",
        "earnings",
        "TLKM",
        "--annual",
        "--estimates",
    ]);
    let csv = String::from_utf8(out.stdout).unwrap();
    let mut lines = csv.lines();
    assert!(
        lines
            .next()
            .unwrap()
            .starts_with("period,period_type,end_date"),
        "{csv}"
    );
    assert_eq!(lines.count(), 2, "{csv}");

    idx(&sb.cache, &sb.config)
        .args(["stocks", "earnings", "TLKM", "--weekly"])
        .assert()
        .code(2);
}

#[test]
fn uncovered_ticker_has_no_consensus_rows() {
    // Recorded 2026-11-06: Yahoo sends BEEF four empty consensus rows.
    let sb = sandbox();
    seed(
        &sb.cache,
        "https://query2.finance.yahoo.com/v10/finance/quoteSummary/BEEF.JK?modules=earnings,earningsHistory,earningsTrend,calendarEvents,price",
        "yahoo/quote_summary_earnings_beef_20261106.json",
    );
    let run = |args: &[&str]| -> String {
        let out = idx(&sb.cache, &sb.config)
            .args(args)
            .args(["stocks", "earnings", "BEEF"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let v: serde_json::Value = serde_json::from_str(&run(&["-o", "json"])).unwrap();
    assert_eq!(v["data"]["estimates"], serde_json::json!([]));
    let t = run(&[]);
    assert!(!t.contains("Consensus"), "{t}");
    // CSV keeps its one flattened row of the payload when no list has rows.
    let csv = run(&["-o", "csv"]);
    let mut lines = csv.lines();
    assert!(lines.next().unwrap().starts_with("symbol,"), "{csv}");
    assert_eq!(lines.count(), 1, "{csv}");
}

#[test]
fn financials_show_several_statements() {
    let sb = sandbox();
    seed(
        &sb.cache,
        &format!(
            "https://query2.finance.yahoo.com/ws/fundamentals-timeseries/v1/finance/timeseries/TLKM.JK?type={}",
            idx::providers::yahoo::timeseries::types_param(idx::model::StatementPeriod::Annual)
        ),
        "yahoo/timeseries_tlkm.json",
    );
    let table = |args: &[&str]| -> String {
        let out = idx(&sb.cache, &sb.config)
            .args(["stocks", "financials", "TLKM"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    };
    let all = table(&[]);
    for s in ["INCOME STATEMENT", "BALANCE SHEET", "CASH FLOW"] {
        assert!(all.contains(s), "{s}");
    }
    for args in [
        &["-s", "income,cash"][..],
        &["--statement", "income", "--statement", "cashflow"],
        &["-s", "cash-flow,income"],
    ] {
        let t = table(args);
        assert!(t.contains("INCOME STATEMENT"), "{args:?}");
        assert!(t.contains("CASH FLOW"), "{args:?}");
        assert!(!t.contains("BALANCE SHEET"), "{args:?}");
    }
    let t = table(&["-s", "balance"]);
    assert!(t.contains("BALANCE SHEET") && !t.contains("INCOME STATEMENT"));
    // `all` anywhere in the list means every statement.
    let t = table(&["-s", "balance,all"]);
    assert!(t.contains("INCOME STATEMENT") && t.contains("CASH FLOW"));
    idx(&sb.cache, &sb.config)
        .args(["stocks", "financials", "TLKM", "-s", "bogus"])
        .assert()
        .code(2);
    // JSON keeps all three statements: they are columns of each period.
    let out = idx(&sb.cache, &sb.config)
        .args(["-o", "json", "stocks", "financials", "TLKM", "-s", "income"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let p = &v["data"]["periods"][0];
    assert!(
        p["balance"].is_object() && p["cash_flow"].is_object(),
        "{p}"
    );
}

#[test]
fn history_provider_routes_candles() {
    let sb = sandbox();
    seed(
        &sb.cache,
        "https://query1.finance.yahoo.com/v8/finance/chart/BBCA.JK?range=1mo&interval=1d&includePrePost=false",
        "yahoo/chart_bbca_1mo_1d.json",
    );
    seed(
        &sb.cache,
        "https://query1.finance.yahoo.com/v8/finance/chart/TLKM.JK?range=1y&interval=1wk&includePrePost=false",
        "yahoo/chart_tlkm_1y_1wk.json",
    );
    let json = |args: &[&str]| -> serde_json::Value {
        let out = idx(&sb.cache, &sb.config).args(args).output().unwrap();
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };

    // MSN has no history: candles come from Yahoo (this used to exit 6).
    let v = json(&[
        "--provider",
        "msn",
        "-o",
        "json",
        "stocks",
        "history",
        "BBCA",
        "--period",
        "1mo",
    ]);
    assert_eq!(v["provider"], "yahoo");
    assert_eq!(v["data"]["symbol"], "BBCA");
    assert!(v.get("warnings").is_none(), "{v}");

    // Stockbit without a session: falls back to Yahoo and says why.
    let v = json(&[
        "--provider",
        "stockbit",
        "-o",
        "json",
        "stocks",
        "history",
        "BBCA",
        "--period",
        "1mo",
    ]);
    assert_eq!(v["provider"], "yahoo");
    let w = v["warnings"][0].as_str().unwrap();
    assert!(w.starts_with("stockbit history failed"), "{w}");
    assert!(w.contains("idx stockbit login"), "{w}");
    // Table mode prints the same warning on stderr.
    idx(&sb.cache, &sb.config)
        .args([
            "--provider",
            "stockbit",
            "stocks",
            "history",
            "BBCA",
            "--period",
            "1mo",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("warning: stockbit history failed"));

    // Pinned to Stockbit: no fallback, the usage error stands.
    idx(&sb.cache, &sb.config)
        .args([
            "--provider",
            "stockbit",
            "--history-provider",
            "stockbit",
            "stocks",
            "history",
            "BBCA",
            "--period",
            "1mo",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("stockbit login"));
    idx(&sb.cache, &sb.config)
        .env("IDX_HISTORY_PROVIDER", "stockbit")
        .args(["stocks", "history", "BBCA", "--period", "1mo"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("stockbit login"));
    // ...and the flag beats the environment.
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_HISTORY_PROVIDER", "stockbit")
        .args([
            "--history-provider",
            "yahoo",
            "-o",
            "json",
            "stocks",
            "history",
            "BBCA",
            "--period",
            "1mo",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["provider"], "yahoo");

    // Stockbit charts a year in daily bars: a weekly request goes to Yahoo with a note.
    let v = json(&[
        "--provider",
        "stockbit",
        "-o",
        "json",
        "stocks",
        "history",
        "TLKM",
        "--period",
        "1y",
        "--interval",
        "1wk",
    ]);
    assert_eq!(v["provider"], "yahoo");
    assert_eq!(v["data"]["interval"], "1wk");
    let w = v["warnings"][0].as_str().unwrap();
    assert!(w.contains("daily bars"), "{w}");
    assert!(w.contains("candles from yahoo"), "{w}");

    // Bad values: the flag is a usage error, the env and file are config errors.
    idx(&sb.cache, &sb.config)
        .args(["--history-provider", "nope", "stocks", "history", "BBCA"])
        .assert()
        .code(2);
    idx(&sb.cache, &sb.config)
        .env("IDX_HISTORY_PROVIDER", "msn")
        .args(["stocks", "history", "BBCA"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("auto, yahoo or stockbit"));
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "history_provider", "msn"])
        .assert()
        .code(6);
    idx(&sb.cache, &sb.config)
        .args(["config", "set", "history_provider", "stockbit"])
        .assert()
        .success();
    idx(&sb.cache, &sb.config)
        .args(["config", "get", "history_provider"])
        .assert()
        .success()
        .stdout(
            "stockbit
",
        );
    idx(&sb.cache, &sb.config)
        .args(["config", "get"])
        .assert()
        .success()
        .stdout(predicate::str::contains("history     : stockbit"));
    let v = json(&["-o", "json", "config", "get"]);
    assert_eq!(v["history_provider"], "stockbit");
    let v = json(&["--history-provider", "auto", "-o", "json", "config", "get"]);
    assert_eq!(v["history_provider"], "auto");
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
    // --force combines with --latest/--all (idx 0.8.1 refused it as a usage error).
    own(&sb)
        .args(["ownership", "import", "--all", "--force"])
        .assert()
        .code(5);
    own(&sb)
        .args(["ownership", "import", "--all", "--latest"])
        .assert()
        .code(2);
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

/// The analytics excerpts (16 issuers, 31 Aug and 30 Sep 2026) imported into
/// the sandbox's own database.
fn analytics_db(sb: &Sandbox) -> std::path::PathBuf {
    let db = sb.cache.join("analytics.sqlite");
    for month in ["20260831", "20260930"] {
        let file = format!(
            "{}/tests/fixtures/ownership/above1_{month}_analytics_excerpt.xlsx",
            env!("CARGO_MANIFEST_DIR")
        );
        idx(&sb.cache, &sb.config)
            .env("IDX_OWNERSHIP_DB", &db)
            .args(["ownership", "import", "--file", &file])
            .assert()
            .success();
    }
    db
}

#[test]
fn ownership_analytics_work_offline() {
    let sb = sandbox();
    let empty = sb.cache.join("empty.sqlite");
    idx(&sb.cache, &sb.config)
        .env("IDX_OWNERSHIP_DB", &empty)
        .args(["ownership", "search", "salim"])
        .assert()
        .code(4)
        .stderr(predicate::str::contains("ownership import"));
    idx(&sb.cache, &sb.config)
        .args(["ownership", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("search"))
        .stdout(predicate::str::contains("cross-holders"))
        .stdout(predicate::str::contains("concentration"))
        .stdout(predicate::str::contains("graph"));

    let db = analytics_db(&sb);
    let own = || {
        let mut c = idx(&sb.cache, &sb.config);
        c.env("IDX_OWNERSHIP_DB", &db);
        c
    };

    // search
    own()
        .args(["ownership", "search", "permadi"])
        .assert()
        .success()
        .stdout(predicate::str::contains("T PERMADI RACHMAT"))
        .stdout(predicate::str::contains("IR. T. PERMADI RACHMAT"))
        .stdout(predicate::str::contains(
            "idx ownership entity --exact \"T PERMADI RACHMAT\" --as-of 2026-09-30",
        ));
    // A first match that left the register: the hint names its last month,
    // and the command it prints works.
    own()
        .args(["ownership", "search", "ir. t. permadi rachmat"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "idx ownership entity --exact \"IR T PERMADI RACHMAT\" --as-of 2026-08-31",
        ));
    own()
        .args([
            "ownership",
            "entity",
            "--exact",
            "IR T PERMADI RACHMAT",
            "--as-of",
            "2026-08-31",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IR. T. PERMADI RACHMAT"));
    own()
        .args(["-o", "json", "ownership", "search", "salim", "anthoni"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema\": \"idx.ownership.search.v1\"",
        ))
        .stdout(predicate::str::contains("\"matched\": \"words\""))
        .stdout(predicate::str::contains(
            "\"entity_key\": \"ANTHONI SALIM\"",
        ));
    own()
        .args(["ownership", "search", "x"])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("2 letters or digits"));
    own()
        .args(["ownership", "search", "nobody", "at", "all"])
        .assert()
        .success()
        .stderr(predicate::str::contains("no holder matching"));
    // entity --exact: the substring search finds IR. T. PERMADI RACHMAT in
    // August, the exact key does not.
    own()
        .args([
            "ownership",
            "entity",
            "T PERMADI RACHMAT",
            "--as-of",
            "2026-08-31",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("IR. T. PERMADI RACHMAT"));
    own()
        .args([
            "ownership",
            "entity",
            "T PERMADI RACHMAT",
            "--as-of",
            "2026-08-31",
            "--exact",
        ])
        .assert()
        .code(4);

    // cross-holders
    own()
        .args(["ownership", "cross-holders"])
        .assert()
        .success()
        .stdout(predicate::str::contains("DRS SURONO SUBEKTI"))
        .stdout(predicate::str::contains("8 holders"))
        .stdout(predicate::str::contains("breadth measure"));
    let out = own()
        .args([
            "-o",
            "json",
            "ownership",
            "cross-holders",
            "--origin",
            "foreign",
            "--min-tickers",
            "2",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.ownership.cross-holders.v1");
    assert_eq!(v["data"]["total_matches"], 2);
    assert!(!String::from_utf8_lossy(&out.stdout).contains("\"local_foreign\": \"L\""));
    own()
        .args(["ownership", "cross-holders", "--origin", "mars"])
        .assert()
        .code(2);

    // concentration
    own()
        .args(["-o", "json", "ownership", "concentration", "BBCA"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema\": \"idx.ownership.concentration.v1\"",
        ))
        .stdout(predicate::str::contains("\"hhi\": 3021.9114"))
        .stdout(predicate::str::contains("\"undisclosed_pct\": 41.82"));
    own()
        .args(["ownership", "concentration", "--limit", "3"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ADES"))
        .stdout(predicate::str::contains("8,345.90"))
        .stdout(predicate::str::contains("lower bound"));
    own()
        .args(["ownership", "concentration", "--by", "nope"])
        .assert()
        .code(2);
    own()
        .args(["ownership", "concentration", "ZZZZ"])
        .assert()
        .code(4);

    // graph
    own()
        .args(["ownership", "graph", "AADI"])
        .assert()
        .success()
        .stdout(predicate::str::contains("T PERMADI RACHMAT"))
        .stdout(predicate::str::contains("13 links"));
    own()
        .args(["ownership", "graph", "AADI", "--dot"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("digraph ownership {"))
        .stdout(predicate::str::contains(
            "\"holder:ADARO STRATEGIC INVESTMENTS\" -> \"ticker:AADI\" [label=\"41.10%\"];",
        ));
    // --dot wins over -o json.
    own()
        .args(["-o", "json", "ownership", "graph", "AADI", "--dot"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("digraph ownership {"));
    // CSV prints the links.
    own()
        .args(["-o", "csv", "ownership", "graph", "AADI"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with("from,to,holder,symbol,"));
    own()
        .args(["ownership", "graph", "AADI", "--max-nodes", "5"])
        .assert()
        .success()
        .stdout(predicate::str::contains("truncated"))
        .stderr(predicate::str::contains("stopped at 5 nodes"));
    // A root whose stakes are all below --min-pct keeps its names, type and flag.
    let out = own()
        .args([
            "-o",
            "json",
            "ownership",
            "graph",
            "AADI",
            "--min-pct",
            "99",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v["data"]["issuers"][0]["issuer_name"],
        "ADARO ANDALAN INDONESIA Tbk"
    );
    assert_eq!(v["data"]["links"].as_array().unwrap().len(), 0);
    let out = own()
        .args([
            "-o",
            "json",
            "ownership",
            "graph",
            "PT Dwimuria Investama Andalan",
            "--min-pct",
            "60",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let root = &v["data"]["holders"][0];
    assert_eq!(root["holder"], "PT DWIMURIA INVESTAMA ANDALAN");
    assert_eq!(root["investor_type"], "CORPORATE");
    assert_eq!(root["local_foreign"], "L");
    own().args(["ownership", "graph", "ZZZZ"]).assert().code(4);
    own()
        .args(["ownership", "graph", "AADI", "--depth", "0"])
        .assert()
        .code(2);

    // Every new shape is reachable through `idx schema`.
    for name in [
        "ownership-search",
        "ownership-cross-holders",
        "ownership-concentration",
        "ownership-graph",
    ] {
        own().args(["schema", name]).assert().success();
    }
}

#[test]
fn ownership_resolve_merges_and_undoes_offline() {
    let sb = sandbox();
    let db = analytics_db(&sb);
    let own = || {
        let mut c = idx(&sb.cache, &sb.config);
        c.env("IDX_OWNERSHIP_DB", &db);
        c
    };
    let changes = || {
        let out = own()
            .args([
                "ownership",
                "changes",
                "--from",
                "2026-08-31",
                "--to",
                "2026-09-30",
                "--limit",
                "0",
            ])
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout).to_string()
    };

    own()
        .args(["ownership", "resolve", "candidates"])
        .assert()
        .success()
        .stdout(predicate::str::contains("SURONO SUBEKTI"))
        .stdout(predicate::str::contains("8 pairs"))
        .stdout(predicate::str::contains("suggestions"));
    // The reference CLI's name for it still works.
    own()
        .args(["-o", "json", "ownership", "resolve", "unresolved"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema\": \"idx.ownership.resolve-candidates.v1\"",
        ));
    assert!(changes().contains("SURONO"));
    own()
        .args([
            "ownership",
            "resolve",
            "merge",
            "DRS SURONO SUBEKTI",
            "SURONO SUBEKTI",
            "--dry-run",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Would merge SURONO SUBEKTI"))
        .stdout(predicate::str::contains("dry run"));
    assert!(changes().contains("SURONO"), "a dry run writes nothing");
    own()
        .args([
            "-o",
            "json",
            "ownership",
            "resolve",
            "merge",
            "DRS SURONO SUBEKTI",
            "SURONO SUBEKTI",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema\": \"idx.ownership.resolution.v1\"",
        ))
        .stdout(predicate::str::contains("\"lines_moved\": 3"));
    assert!(!changes().contains("SURONO"));
    own()
        .args(["ownership", "resolve", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("#1"))
        .stdout(predicate::str::contains("merge"));
    own()
        .args(["ownership", "resolve", "undo", "#1"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Undid merge (#1)"));
    assert!(changes().contains("SURONO"));
    own()
        .args(["ownership", "resolve", "undo", "1"])
        .assert()
        .code(4);

    // Listed together in WGSH: refused without --fold.
    own()
        .args([
            "ownership",
            "resolve",
            "merge",
            "WALDEN GLOBAL SERVICES",
            "WALDEN GLOBAL SERVICES PTE LTD",
        ])
        .assert()
        .code(2)
        .stderr(predicate::str::contains("--fold"));
    own()
        .args([
            "ownership",
            "resolve",
            "map",
            "IR. T. PERMADI RACHMAT",
            "T PERMADI RACHMAT",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Mapped spelling"));
    assert!(!changes().contains("PERMADI"));
    own()
        .args(["-o", "json", "ownership", "resolve", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\": \"map\""))
        .stdout(predicate::str::contains("\"id\": 2"));
    for name in [
        "ownership-resolve-candidates",
        "ownership-resolution",
        "ownership-resolutions",
    ] {
        own().args(["schema", name]).assert().success();
    }
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

#[test]
fn ownership_composition_and_families_work_offline() {
    let sb = sandbox();
    let db = sb.cache.join("composition.sqlite");
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
    // Nothing imported and --offline (the test harness): no download.
    own(&sb)
        .args(["ownership", "composition", "BBCA"])
        .assert()
        .code(5)
        .stderr(predicate::str::contains("--family composition"));
    // The ZIP and the TXT inside it are both detected from their content.
    let out = own(&sb)
        .args([
            "-o",
            "json",
            "ownership",
            "import",
            "--file",
            &fx("ksei_balancepos_20261030_excerpt_constructed.zip"),
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.ownership.import.v1");
    assert_eq!(v["data"][0]["family"], "composition");
    assert_eq!(v["data"][0]["release"]["rows"], 6);
    assert_eq!(v["data"][0]["release"]["format"], "zip");
    assert_eq!(v["data"][0]["other_securities"], 7);
    own(&sb)
        .args([
            "ownership",
            "import",
            "--file",
            &fx("ksei_balancepos_20260930_excerpt.txt"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Imported KSEI composition 2026-09-30 (txt): 6 stocks, 7 other securities left out",
        ));
    own(&sb)
        .args(["ownership", "composition", "bbca"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "BBCA · KSEI composition 2026-10-30",
        ))
        .stdout(predicate::str::contains("scripless"))
        .stdout(predicate::str::contains("42.55% of listed"))
        .stdout(predicate::str::contains("ID individual"))
        .stdout(predicate::str::contains("Imported months"));
    let out = own(&sb)
        .args(["-o", "json", "ownership", "composition", "BBCA"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["schema"], "idx.ownership.composition.v1");
    assert_eq!(v["provider"], "local-db");
    assert_eq!(v["data"]["scripless_pct"], 42.5501);
    assert_eq!(v["data"]["foreign_pct"], 67.2201);
    assert_eq!(v["data"]["history"].as_array().unwrap().len(), 2);
    assert!(v["data"]["history"][0]["foreign_change_pp"].is_null());
    let out = own(&sb)
        .args([
            "-o",
            "json",
            "ownership",
            "composition",
            "BBCA",
            "--as-of",
            "2026-09-30",
        ])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["as_of"], "2026-09-30");
    own(&sb)
        .args(["-o", "csv", "ownership", "composition", "BBCA"])
        .assert()
        .success()
        .stdout(predicate::str::starts_with(
            "symbol,as_of,code,investor_type,local_shares,foreign_shares,local_pct,foreign_pct\n\
             BBCA,2026-10-30,IS,INSURANCE,",
        ));
    own(&sb)
        .args(["ownership", "composition", "ZZZZ"])
        .assert()
        .code(4);
    // A month that is not imported needs ksei.co.id.
    own(&sb)
        .args(["ownership", "composition", "BBCA", "--as-of", "2026-08-31"])
        .assert()
        .code(5);
    // The register of holders above 1 % is untouched.
    let out = own(&sb)
        .args(["-o", "json", "ownership", "releases"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"], serde_json::json!([]));
    own(&sb)
        .args(["ownership", "ticker", "BBCA"])
        .assert()
        .code(4);
    // A register workbook is still a register.
    own(&sb)
        .args([
            "-o",
            "json",
            "ownership",
            "import",
            "--file",
            &fx("above1_20260831_excerpt.xlsx"),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"family\": \"above1\""));
    // Families and limits.
    own(&sb)
        .args(["ownership", "discover", "--family", "nope"])
        .assert()
        .code(2);
    own(&sb)
        .args(["ownership", "discover", "--family", "composition"])
        .assert()
        .code(5);
    own(&sb)
        .args(["ownership", "discover", "--family", "all", "--limit", "2"])
        .assert()
        .code(5);
    own(&sb).args(["ownership", "discover"]).assert().code(5);
    own(&sb)
        .args(["ownership", "import", "--latest", "--family", "composition"])
        .assert()
        .code(5);
    own(&sb)
        .args([
            "ownership",
            "import",
            "--family",
            "composition",
            "--file",
            &fx("ksei_balancepos_20261030_excerpt_constructed.zip"),
        ])
        .assert()
        .code(2);
    own(&sb)
        .args(["ownership", "discover", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("investor-type"))
        .stdout(predicate::str::contains("--limit"));
    let out = idx(&sb.cache, &sb.config)
        .args(["schema", "ownership-composition"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v["$schema"].as_str().unwrap().contains("json-schema.org"));
    assert!(v.to_string().contains("scripless_pct"));
}

#[test]
fn ownership_composition_csv_keeps_its_columns_however_many_months() {
    let sb = sandbox();
    let db = sb.cache.join("composition.sqlite");
    let sep = include_str!("fixtures/ownership/ksei_balancepos_20260930_excerpt.txt");
    let dir = sb.cache.join("months");
    std::fs::create_dir_all(&dir).unwrap();
    // Ten months, one more than the nine investor types: September's excerpt
    // dated January to October.
    let months = [
        ("20260130", "30-JAN-2026"),
        ("20260227", "27-FEB-2026"),
        ("20260331", "31-MAR-2026"),
        ("20260430", "30-APR-2026"),
        ("20260529", "29-MAY-2026"),
        ("20260630", "30-JUN-2026"),
        ("20260731", "31-JUL-2026"),
        ("20260831", "31-AUG-2026"),
        ("20260930", "30-SEP-2026"),
        ("20261030", "30-OCT-2026"),
    ];
    for (name, cell) in months {
        let path = dir.join(format!("Balancepos{name}.txt"));
        std::fs::write(&path, sep.replace("30-SEP-2026", cell)).unwrap();
        idx(&sb.cache, &sb.config)
            .env("IDX_OWNERSHIP_DB", &db)
            .args(["ownership", "import", "--file"])
            .arg(&path)
            .assert()
            .success();
    }
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_OWNERSHIP_DB", &db)
        .args(["-o", "json", "ownership", "composition", "BBCA"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["data"]["history"].as_array().unwrap().len(), 10);
    let out = idx(&sb.cache, &sb.config)
        .env("IDX_OWNERSHIP_DB", &db)
        .args(["-o", "csv", "ownership", "composition", "BBCA"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let csv = String::from_utf8(out.stdout).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "symbol,as_of,code,investor_type,local_shares,foreign_shares,local_pct,foreign_pct"
    );
    assert_eq!(lines.len(), 10, "a header and the nine types:\n{csv}");
    assert!(lines[9].starts_with("BBCA,2026-10-30,OT,OTHERS,"), "{csv}");
}

/// A one-shot HTTP server on 127.0.0.1 answering every request with the
/// response for its path: Cloudflare's challenge and block pages and an
/// ordinary outage, without leaving the machine.
fn refusal_stub() -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let challenge = include_str!("fixtures/idx/cloudflare_challenge_synthetic.html");
    let block = include_str!("fixtures/idx/cloudflare_blocked.html");
    let handle = std::thread::spawn(move || {
        for _ in 0..3 {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                match s.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => buf.extend_from_slice(&chunk[..n]),
                }
            }
            let req = String::from_utf8_lossy(&buf);
            let (status, headers, body) = if req.starts_with("GET /challenge") {
                (
                    "403 Forbidden",
                    "cf-mitigated: challenge\r\nserver: cloudflare\r\n",
                    challenge,
                )
            } else if req.starts_with("GET /block") {
                ("403 Forbidden", "server: cloudflare\r\n", block)
            } else {
                (
                    "503 Service Unavailable",
                    "server: nginx\r\n",
                    "<html><head><title>503 Service Temporarily Unavailable</title></head></html>",
                )
            };
            let _ = write!(
                s,
                "HTTP/1.1 {status}\r\n{headers}content-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    (base, handle)
}

#[test]
fn a_cloudflare_refusal_is_blocked_and_an_outage_is_not() {
    let sb = sandbox();
    let (base, server) = refusal_stub();
    let discover = |path: &str| {
        let mut c = idx(&sb.cache, &sb.config);
        // Loopback only: the page is the stub above, the mirror is never
        // asked (an explicit page is not second-guessed).
        c.env_remove("IDX_OFFLINE")
            .env("IDX_OWNERSHIP_PAGE_URL", format!("{base}{path}"))
            .env("IDX_OWNERSHIP_MIRROR", "off")
            .env("IDX_CURL_IMPERSONATE", "idx-test-no-such-curl-impersonate")
            .args(["-o", "json", "ownership", "discover"]);
        let out = c.output().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&out.stderr)
            .unwrap_or_else(|_| panic!("{}", String::from_utf8_lossy(&out.stderr)));
        (out.status.code(), v)
    };
    let (code, v) = discover("/challenge");
    assert_eq!(code, Some(3));
    assert_eq!(v["error"]["code"], "blocked", "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("a Cloudflare challenge (HTTP 403, cf-mitigated: challenge)"),
        "{msg}"
    );
    assert!(msg.contains("no curl-impersonate binary"), "{msg}");
    let (code, v) = discover("/block");
    assert_eq!(
        (code, &v["error"]["code"]),
        (Some(3), &serde_json::json!("blocked"))
    );
    assert!(v["error"]["message"]
        .as_str()
        .unwrap()
        .contains("a Cloudflare block page (HTTP 403)"));
    let (code, v) = discover("/maintenance");
    assert_eq!(code, Some(3));
    assert_eq!(v["error"]["code"], "network", "{v}");
    let msg = v["error"]["message"].as_str().unwrap();
    assert!(
        msg.contains("HTTP 503") && !msg.contains("Cloudflare"),
        "{msg}"
    );
    server.join().unwrap();
}

/// A `file://` URL for a local directory, as `IDX_OWNERSHIP_MIRROR` takes it.
fn file_url(dir: &std::path::Path) -> String {
    let p = dir.display().to_string().replace('\\', "/");
    if p.starts_with('/') {
        format!("file://{p}")
    } else {
        format!("file:///{p}")
    }
}

fn json_out(out: &[u8]) -> serde_json::Value {
    serde_json::from_slice(out).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(out)))
}

#[test]
fn ownership_snapshot_sync_and_status_work_offline() {
    let sb = sandbox();
    let root = sb.cache.parent().unwrap().join("snap");
    let fx = |name: &str| {
        std::path::PathBuf::from(format!(
            "{}/tests/fixtures/ownership/{name}",
            env!("CARGO_MANIFEST_DIR")
        ))
    };
    let july = fx("above1_20260731_synthetic.xlsx");
    let august = fx("above1_20260831_excerpt.xlsx");
    let manifest = format!("snapshot-v{}.json", idx::ownership::SCHEMA_VERSION);
    let with_db = |db: &std::path::Path| {
        let mut c = idx(&sb.cache, &sb.config);
        c.env("IDX_OWNERSHIP_DB", db);
        c
    };
    // A sync reads a file:// mirror: no network, so the offline safety net
    // is lifted for those calls only.
    let sync_cmd = |db: &std::path::Path, mirror: &std::path::Path| {
        let mut c = with_db(db);
        c.env_remove("IDX_OFFLINE")
            .env("IDX_OWNERSHIP_MIRROR", file_url(mirror));
        c
    };
    let import = |db: &std::path::Path, files: &[&std::path::Path]| {
        for f in files {
            with_db(db)
                .args(["ownership", "import", "--file"])
                .arg(f)
                .assert()
                .success();
        }
    };
    // A mirror directory: the snapshot of the first `in_snapshot` files, all
    // `files` as raw files and index.json.
    let make_mirror_lagging = |name: &str,
                               files: &[(&str, &std::path::Path)],
                               in_snapshot: usize|
     -> std::path::PathBuf {
        let dir = root.join(name);
        let src = root.join(format!("{name}-src.sqlite"));
        let paths: Vec<&std::path::Path> =
            files.iter().map(|(_, p)| *p).take(in_snapshot).collect();
        import(&src, &paths);
        with_db(&src)
            .args(["ownership", "build-snapshot", "--out"])
            .arg(&dir)
            .args(["--generated-at", "2026-11-06T00:00:00Z"])
            .assert()
            .success()
            .stdout(predicate::str::contains("Built ownership-v"));
        let mut releases = Vec::new();
        for (as_of, path) in files {
            let bytes = std::fs::read(path).unwrap();
            let file = format!("ksei-above1-{as_of}.xlsx");
            std::fs::write(dir.join(&file), &bytes).unwrap();
            releases.push(serde_json::json!({
                "as_of": as_of, "format": "xlsx", "file": file,
                "sha256": idx::ownership::sha256_hex(&bytes),
            }));
        }
        let index = serde_json::json!({
            "schema": "idx.ownership.mirror.v1",
            "generated_at": "2026-11-06T00:00:00Z",
            "releases": releases,
        });
        std::fs::write(dir.join("index.json"), index.to_string()).unwrap();
        dir
    };
    let make_mirror = |name: &str, files: &[(&str, &std::path::Path)]| {
        make_mirror_lagging(name, files, files.len())
    };
    let both = make_mirror("both", &[("2026-07-31", &july), ("2026-08-31", &august)]);

    // Fresh database: one sync installs every month, a second does nothing.
    let live = root.join("live").join("ownership.sqlite");
    let out = sync_cmd(&live, &both)
        .args(["-o", "json", "ownership", "sync"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_out(&out.stdout);
    assert_eq!(v["schema"], "idx.ownership.snapshot-sync.v1");
    assert_eq!(v["provider"], "github-mirror");
    assert_eq!(v["data"]["action"], "installed");
    assert_eq!(v["data"]["releases"], 2);
    assert_eq!(v["data"]["newest"], "2026-08-31");
    assert_eq!(
        v["data"]["installed"],
        serde_json::json!(["2026-07-31", "2026-08-31"])
    );
    assert!(v["data"]["backup"].is_null());
    with_db(&live)
        .args(["ownership", "ticker", "AADI"])
        .assert()
        .success()
        .stdout(predicate::str::contains("ADARO STRATEGIC INVESTMENTS"));
    sync_cmd(&live, &both)
        .args(["ownership", "sync"])
        .assert()
        .success()
        .stdout(predicate::str::contains("is up to date"));

    // Status: installed snapshot, the mirror's state, nothing to do.
    let out = sync_cmd(&live, &both)
        .args(["-o", "json", "ownership", "status"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v = json_out(&out.stdout);
    assert_eq!(v["schema"], "idx.ownership.status.v1");
    assert_eq!(v["data"]["exists"], true);
    assert_eq!(v["data"]["releases"], 2);
    assert_eq!(v["data"]["installed_snapshot"]["releases"], 2);
    assert_eq!(v["data"]["remote"]["newest"], "2026-08-31");
    assert_eq!(v["data"]["remote"]["snapshot"]["newest"], "2026-08-31");
    assert_eq!(v["data"]["sync"], "up_to_date");
    // Offline: the local part only, still exit 0; sync refuses (5); no mirror (6).
    let out = with_db(&live)
        .args(["-o", "json", "ownership", "status"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v = json_out(&out.stdout);
    assert!(v["data"]["remote"].is_null());
    assert!(v["data"]["sync"].is_null());
    assert!(v["warnings"][0].as_str().unwrap().contains("offline"));
    with_db(&live).args(["ownership", "sync"]).assert().code(5);
    sync_cmd(&live, &both)
        .env("IDX_OWNERSHIP_MIRROR", "off")
        .args(["ownership", "sync"])
        .assert()
        .code(6);
    // Status never creates a database.
    let nothing = root.join("nothing.sqlite");
    with_db(&nothing)
        .args(["ownership", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not created yet"));
    assert!(!nothing.exists());

    // A database holding a month the snapshot lacks is kept, unless --force.
    let july_only = make_mirror("july", &[("2026-07-31", &july)]);
    let mine = root.join("mine.sqlite");
    import(&mine, &[&july, &august]);
    let out = sync_cmd(&mine, &july_only)
        .args(["-o", "json", "ownership", "sync"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v = json_out(&out.stdout);
    assert_eq!(v["data"]["action"], "up_to_date");
    assert_eq!(v["data"]["kept"], serde_json::json!(["2026-08-31"]));
    assert_eq!(v["data"]["releases"], 2);
    assert!(v["warnings"][0].as_str().unwrap().contains("--force"));
    let out = sync_cmd(&mine, &july_only)
        .args(["-o", "json", "ownership", "sync", "--force"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v = json_out(&out.stdout);
    assert_eq!(v["data"]["action"], "updated");
    assert_eq!(v["data"]["dropped"], serde_json::json!(["2026-08-31"]));
    assert_eq!(v["data"]["releases"], 1);
    let bak = std::path::PathBuf::from(v["data"]["backup"].as_str().unwrap());
    assert!(bak.ends_with("mine.sqlite.bak"));
    let out = with_db(&bak)
        .args(["-o", "json", "ownership", "status"])
        .output()
        .unwrap();
    assert_eq!(
        json_out(&out.stdout)["data"]["releases"],
        2,
        "the backup holds both months"
    );

    // A database built by local imports of the same files: replaced once, then up to date.
    let local = root.join("local.sqlite");
    import(&local, &[&july, &august]);
    for expected in ["updated", "up_to_date"] {
        let out = sync_cmd(&local, &both)
            .args(["-o", "json", "ownership", "sync"])
            .output()
            .unwrap();
        assert!(out.status.success());
        assert_eq!(json_out(&out.stdout)["data"]["action"], expected);
    }

    // A snapshot behind the mirror: installed, then the newer month imported.
    let lag = make_mirror_lagging("lag", &[("2026-07-31", &july), ("2026-08-31", &august)], 1);
    let behind = root.join("behind.sqlite");
    let out = sync_cmd(&behind, &lag)
        .args(["-o", "json", "ownership", "sync"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_out(&out.stdout);
    assert_eq!(v["data"]["action"], "installed");
    assert_eq!(v["data"]["installed"], serde_json::json!(["2026-07-31"]));
    assert_eq!(v["data"]["imported"], serde_json::json!(["2026-08-31"]));
    assert_eq!(v["data"]["newest"], "2026-08-31");
    let out = sync_cmd(&behind, &lag)
        .args(["-o", "json", "ownership", "sync"])
        .output()
        .unwrap();
    let v = json_out(&out.stdout);
    assert_eq!(v["data"]["action"], "up_to_date");
    assert_eq!(v["data"]["kept"], serde_json::json!([]));

    // A corrupt download: provider error (exit 3), database untouched.
    let corrupt = root.join("corrupt");
    std::fs::create_dir_all(&corrupt).unwrap();
    for e in std::fs::read_dir(&both).unwrap() {
        let p = e.unwrap().path();
        let mut bytes = std::fs::read(&p).unwrap();
        if p.to_string_lossy().ends_with(".sqlite.gz") {
            let mid = bytes.len() / 2;
            bytes[mid] ^= 0xff;
        }
        std::fs::write(corrupt.join(p.file_name().unwrap()), bytes).unwrap();
    }
    let older = root.join("older.sqlite");
    import(&older, &[&july]);
    let before = std::fs::read(&older).unwrap();
    sync_cmd(&older, &corrupt)
        .args(["ownership", "sync"])
        .assert()
        .code(3)
        .stderr(predicate::str::contains("SHA-256"));
    assert_eq!(std::fs::read(&older).unwrap(), before);
    assert!(!root.join("older.sqlite.bak").exists());

    // No snapshot for this schema on the mirror: the monthly files are imported.
    std::fs::remove_file(corrupt.join(&manifest)).unwrap();
    let raw = root.join("raw.sqlite");
    let out = sync_cmd(&raw, &corrupt)
        .args(["-o", "json", "ownership", "sync"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_out(&out.stdout);
    assert_eq!(v["data"]["action"], "imported");
    assert!(v["data"]["snapshot"].is_null());
    assert_eq!(
        v["data"]["imported"],
        serde_json::json!(["2026-07-31", "2026-08-31"])
    );
    assert!(v["warnings"][0].as_str().unwrap().contains("no snapshot"));

    // The shapes are published; build-snapshot stays out of --help.
    for name in [
        "ownership-status",
        "ownership-snapshot-sync",
        "ownership-snapshot",
    ] {
        idx(&sb.cache, &sb.config)
            .args(["schema", name])
            .assert()
            .success()
            .stdout(predicate::str::contains("\"$schema\""));
    }
    idx(&sb.cache, &sb.config)
        .args(["ownership", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("status"))
        .stdout(predicate::str::contains("build-snapshot").not());
    idx(&sb.cache, &sb.config)
        .args(["ownership", "sync", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--force"))
        .stdout(predicate::str::contains("Examples"));
}
