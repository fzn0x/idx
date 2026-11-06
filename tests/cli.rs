//! End-to-end tests of the binary that never touch the network.
//! Live-provider checks live in `scripts/smoke.sh`.

use assert_cmd::Command;
use predicates::prelude::*;

fn idx(cache: &std::path::Path, config: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("idx").unwrap();
    // Keep the Stockbit session inside the sandbox so a test never reads or
    // writes the developer's real login.
    let session = config
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join("stockbit-session.json");
    cmd.env_clear()
        .env("IDX_CACHE_DIR", cache)
        .env("IDX_CONFIG", config)
        .env("IDX_STOCKBIT_SESSION", session)
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
    let v = json(&[
        "--history-provider",
        "yahoo",
        "-o",
        "json",
        "stocks",
        "history",
        "BBCA",
        "--period",
        "1mo",
    ]);
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
