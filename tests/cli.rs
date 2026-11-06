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
    // MSN does not serve history: explicit provider gets exit 6 with a hint.
    idx(&sb.cache, &sb.config)
        .args(["--provider", "msn", "stocks", "history", "BBCA"])
        .assert()
        .code(6)
        .stderr(predicate::str::contains("--provider auto"));
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
