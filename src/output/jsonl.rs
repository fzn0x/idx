//! `-o jsonl` (JSON Lines): one compact JSON record per line on stdout.
//!
//! Each line is exactly the object `-o json` carries at that path (same keys,
//! same order, nulls kept). Which list of a payload becomes the lines is fixed
//! per schema in [`ROWS`], never guessed from the data: a payload that is one
//! record, or that holds several peer lists (an order book's bids and asks),
//! prints as one line. An empty list prints nothing. The envelope itself is
//! not printed; its warnings go to stderr like in table mode, and provenance
//! (`provider`, `fetched_at`, `cached`, `stale`) stays in `-o json`.

use serde_json::Value;

use crate::error::Result;

use super::json::{
    SCHEMA_ACTIONS, SCHEMA_BANDAR, SCHEMA_BANDAR_BROKER, SCHEMA_BANDAR_SCREEN, SCHEMA_BRIEF,
    SCHEMA_BROKER, SCHEMA_CALENDAR, SCHEMA_COMPARE, SCHEMA_DIVIDENDS, SCHEMA_EARNINGS,
    SCHEMA_FINANCIALS, SCHEMA_FOREIGN, SCHEMA_FUNDAMENTALS, SCHEMA_HISTORY, SCHEMA_HOLIDAYS,
    SCHEMA_INDICES, SCHEMA_INSIGHTS, SCHEMA_INSTRUMENTS, SCHEMA_MARKET, SCHEMA_MARKET_BROKERS,
    SCHEMA_MARKET_FOREIGN, SCHEMA_MEMBERS, SCHEMA_NEWS, SCHEMA_ORDERBOOK, SCHEMA_OWNERSHIP_CHANGES,
    SCHEMA_OWNERSHIP_COMPOSITION, SCHEMA_OWNERSHIP_CONCENTRATION, SCHEMA_OWNERSHIP_CROSS_HOLDERS,
    SCHEMA_OWNERSHIP_DISCOVER, SCHEMA_OWNERSHIP_ENTITY, SCHEMA_OWNERSHIP_FOREIGN,
    SCHEMA_OWNERSHIP_FOREIGN_FLOW, SCHEMA_OWNERSHIP_GRAPH, SCHEMA_OWNERSHIP_HOLDERS,
    SCHEMA_OWNERSHIP_IMPORT, SCHEMA_OWNERSHIP_INSTITUTIONS, SCHEMA_OWNERSHIP_RELEASES,
    SCHEMA_OWNERSHIP_RESOLUTION, SCHEMA_OWNERSHIP_RESOLUTIONS, SCHEMA_OWNERSHIP_RESOLVE_CANDIDATES,
    SCHEMA_OWNERSHIP_SEARCH, SCHEMA_OWNERSHIP_SNAPSHOT, SCHEMA_OWNERSHIP_SNAPSHOT_SYNC,
    SCHEMA_OWNERSHIP_STATUS, SCHEMA_PROFILE, SCHEMA_QUOTES, SCHEMA_RETURNS, SCHEMA_SCREEN,
    SCHEMA_SENTIMENT, SCHEMA_TECHNICAL, SCHEMA_WATCHLIST,
};

/// Where a payload's lines come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rows {
    /// `data` is the list: one line per item.
    Data,
    /// One line per item of `data.<field>`.
    Field(&'static str),
    /// `data` on one line: a single record, or several peer lists kept together.
    Whole,
}

/// The row rule of every schema. A new `SCHEMA_*` constant needs an entry
/// here; a unit test fails until it has one.
pub const ROWS: &[(&str, Rows)] = &[
    (SCHEMA_QUOTES, Rows::Data),
    (SCHEMA_COMPARE, Rows::Data),
    (SCHEMA_RETURNS, Rows::Data),
    (SCHEMA_BROKER, Rows::Data),
    (SCHEMA_BANDAR, Rows::Data),
    (SCHEMA_OWNERSHIP_RELEASES, Rows::Data),
    (SCHEMA_OWNERSHIP_IMPORT, Rows::Data),
    (SCHEMA_OWNERSHIP_DISCOVER, Rows::Data),
    (SCHEMA_OWNERSHIP_RESOLUTION, Rows::Data),
    (SCHEMA_OWNERSHIP_RESOLUTIONS, Rows::Data),
    (SCHEMA_HISTORY, Rows::Field("candles")),
    (SCHEMA_FINANCIALS, Rows::Field("periods")),
    // The payments; yearly totals, splits and the TTM figures stay in -o json.
    (SCHEMA_DIVIDENDS, Rows::Field("dividends")),
    (SCHEMA_BANDAR_SCREEN, Rows::Field("rows")),
    (SCHEMA_SCREEN, Rows::Field("rows")),
    (SCHEMA_OWNERSHIP_FOREIGN_FLOW, Rows::Field("rows")),
    // JSON strings, so `idx -o jsonl watch list | jq -r .` lists the tickers.
    (SCHEMA_WATCHLIST, Rows::Field("symbols")),
    (SCHEMA_HOLIDAYS, Rows::Field("holidays")),
    (SCHEMA_INSTRUMENTS, Rows::Field("instruments")),
    (SCHEMA_MEMBERS, Rows::Field("members")),
    (SCHEMA_INDICES, Rows::Field("indices")),
    (SCHEMA_CALENDAR, Rows::Field("events")),
    (SCHEMA_FOREIGN, Rows::Field("days")),
    (SCHEMA_MARKET_BROKERS, Rows::Field("brokers")),
    (SCHEMA_NEWS, Rows::Field("items")),
    // The statements; the peers stay in -o json.
    (SCHEMA_INSIGHTS, Rows::Field("insights")),
    (SCHEMA_OWNERSHIP_HOLDERS, Rows::Field("holders")),
    (SCHEMA_OWNERSHIP_INSTITUTIONS, Rows::Field("holders")),
    (SCHEMA_OWNERSHIP_ENTITY, Rows::Field("holdings")),
    (SCHEMA_OWNERSHIP_CHANGES, Rows::Field("changes")),
    (SCHEMA_OWNERSHIP_FOREIGN, Rows::Field("points")),
    (SCHEMA_OWNERSHIP_SEARCH, Rows::Field("matches")),
    (SCHEMA_OWNERSHIP_CROSS_HOLDERS, Rows::Field("holders")),
    (SCHEMA_OWNERSHIP_CONCENTRATION, Rows::Field("rows")),
    (
        SCHEMA_OWNERSHIP_RESOLVE_CANDIDATES,
        Rows::Field("candidates"),
    ),
    (SCHEMA_PROFILE, Rows::Whole),
    (SCHEMA_FUNDAMENTALS, Rows::Whole),
    (SCHEMA_TECHNICAL, Rows::Whole),
    (SCHEMA_BRIEF, Rows::Whole),
    (SCHEMA_SENTIMENT, Rows::Whole),
    // Peer lists: quarters/yearly/quarterly, actions/meetings/warrants_rights,
    // gainers/losers/..., net_buy/net_sell, bought/sold, bids/asks,
    // issuers/holders/links, by_type/history.
    (SCHEMA_EARNINGS, Rows::Whole),
    (SCHEMA_ACTIONS, Rows::Whole),
    (SCHEMA_MARKET, Rows::Whole),
    (SCHEMA_MARKET_FOREIGN, Rows::Whole),
    (SCHEMA_BANDAR_BROKER, Rows::Whole),
    (SCHEMA_ORDERBOOK, Rows::Whole),
    (SCHEMA_OWNERSHIP_GRAPH, Rows::Whole),
    (SCHEMA_OWNERSHIP_COMPOSITION, Rows::Whole),
    // installed/imported/kept/dropped, missing/outdated/local_only, one manifest.
    (SCHEMA_OWNERSHIP_SNAPSHOT_SYNC, Rows::Whole),
    (SCHEMA_OWNERSHIP_STATUS, Rows::Whole),
    (SCHEMA_OWNERSHIP_SNAPSHOT, Rows::Whole),
];

/// The row rule for a schema id, `None` if it has none.
pub fn rule(schema: &str) -> Option<Rows> {
    ROWS.iter().find(|(id, _)| *id == schema).map(|(_, r)| *r)
}

/// The values that become lines for a payload of `schema`. Also how stdin
/// input reads idx's own envelopes back (`idx ... | idx stocks quote -`).
/// A schema without a rule falls back to one line per item of an array
/// `data`, else one line; so does a `Field` rule whose field is not a list.
pub fn rows<'a>(schema: &str, data: &'a Value) -> Vec<&'a Value> {
    let list = match rule(schema) {
        Some(Rows::Whole) => None,
        Some(Rows::Field(f)) => data.get(f).and_then(Value::as_array),
        Some(Rows::Data) | None => data.as_array(),
    };
    match list {
        Some(items) => items.iter().collect(),
        None => vec![data],
    }
}

/// One compact JSON line per row of `data`, each ending in `\n`.
pub fn render(schema: &str, data: &Value) -> Result<String> {
    lines(rows(schema, data))
}

/// A value without an envelope: one line per item of an array, else one line.
pub fn render_value(v: &Value) -> Result<String> {
    match v.as_array() {
        Some(items) => lines(items.iter().collect()),
        None => lines(vec![v]),
    }
}

fn lines(rows: Vec<&Value>) -> Result<String> {
    let mut out = String::new();
    for r in rows {
        out.push_str(&serde_json::to_string(r)?);
        out.push('\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Candle, History, Interval, Period, Quote, Symbol};
    use crate::output::json::{schema_for, schema_names, to_pretty, Envelope};
    use crate::providers::Sourced;
    use serde_json::json;

    /// Every `pub const SCHEMA_*` id in json.rs, read from its source so a new
    /// constant cannot slip past [`ROWS`].
    fn schema_ids() -> Vec<String> {
        include_str!("json.rs")
            .lines()
            .filter_map(|l| l.trim().strip_prefix("pub const SCHEMA_"))
            .filter_map(|l| l.split('"').nth(1))
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn every_schema_has_a_row_rule() {
        let ids = schema_ids();
        assert!(ids.len() >= 38, "{ids:?}");
        for id in &ids {
            assert!(
                rule(id).is_some(),
                "{id} has no JSONL row rule; add it to output/jsonl.rs ROWS"
            );
        }
        for (i, (id, _)) in ROWS.iter().enumerate() {
            assert!(ids.iter().any(|x| x == id), "{id} is not a schema id");
            assert!(
                !ROWS[i + 1..].iter().any(|(other, _)| other == id),
                "{id} listed twice"
            );
        }
    }

    /// Resolves `{"$ref": "#/definitions/X"}` (or an `allOf` around one).
    fn resolve<'a>(root: &'a Value, v: &'a Value) -> &'a Value {
        if let Some(r) = v.get("$ref").and_then(Value::as_str) {
            let name = r.rsplit('/').next().unwrap();
            return resolve(root, &root["definitions"][name]);
        }
        if let Some(first) = v.get("allOf").and_then(|a| a.get(0)) {
            return resolve(root, first);
        }
        v
    }

    /// The rules agree with the JSON Schema of each payload: `Data` payloads
    /// are arrays, `Field` names an array property, `Whole` is an object.
    #[test]
    fn row_rules_match_the_payload_schemas() {
        for (id, rule) in ROWS {
            let name = id
                .trim_start_matches("idx.")
                .trim_end_matches(".v1")
                .replace('.', "-");
            assert!(schema_names().contains(&name.as_str()), "{id} -> {name}");
            let root = schema_for(&name).unwrap();
            let data = resolve(&root, &root["properties"]["data"]);
            match rule {
                Rows::Data => assert_eq!(data["type"], "array", "{id}"),
                Rows::Whole => assert_eq!(data["type"], "object", "{id}"),
                Rows::Field(f) => {
                    let field = resolve(&root, &data["properties"][f]);
                    assert_eq!(field["type"], "array", "{id}: data.{f}");
                }
            }
        }
    }

    fn quote(s: &str) -> Quote {
        Quote::empty(Symbol::parse(s).unwrap(), "t")
    }

    #[test]
    fn data_lists_print_one_line_per_item_in_json_key_order() {
        let data = serde_json::to_value(vec![quote("BBCA"), quote("BBRI")]).unwrap();
        let out = render(SCHEMA_QUOTES, &data).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(out.ends_with('\n'));
        for (line, sym) in lines.iter().zip(["BBCA", "BBRI"]) {
            let v: Value = serde_json::from_str(line).unwrap();
            assert_eq!(v["symbol"], sym);
            assert!(v["price"].is_null(), "nulls are kept");
        }
        // Same keys in the same order as the pretty -o json document.
        let env = Envelope::from_sourced(SCHEMA_QUOTES, Sourced::fresh(vec![quote("BBCA")], "t"));
        let pretty: Value = serde_json::from_str(&to_pretty(&env).unwrap()).unwrap();
        let json_keys: Vec<&String> = pretty["data"][0].as_object().unwrap().keys().collect();
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        let line_keys: Vec<&String> = first.as_object().unwrap().keys().collect();
        assert_eq!(json_keys, line_keys);
        assert!(lines[0].starts_with("{\"symbol\":\"BBCA\",\"name\":null,"));
    }

    #[test]
    fn field_lists_and_whole_records() {
        let t = crate::model::wib_from_unix(1_789_524_000).unwrap();
        let candle = Candle {
            time: t,
            open: 1.0,
            high: 2.0,
            low: 0.5,
            close: 1.5,
            adj_close: None,
            volume: 10,
        };
        let h = History {
            symbol: Symbol::parse("BBCA").unwrap(),
            name: Some("multi\nline".into()),
            currency: None,
            period: Period::M1,
            interval: Interval::Day1,
            candles: vec![candle.clone(), candle],
            summary: None,
        };
        let data = serde_json::to_value(&h).unwrap();
        let out = render(SCHEMA_HISTORY, &data).unwrap();
        assert_eq!(out.lines().count(), 2);
        for l in out.lines() {
            let v: Value = serde_json::from_str(l).unwrap();
            assert_eq!(v["close"], 1.5);
            assert!(v["adj_close"].is_null());
        }
        // A whole record is one line, and an embedded newline stays escaped.
        let out = render(SCHEMA_PROFILE, &data).unwrap();
        assert_eq!(out.lines().count(), 1);
        assert!(out.contains("multi\\nline"));
        // An empty list prints nothing.
        let empty = json!({"year": 2026, "holidays": []});
        assert_eq!(render(SCHEMA_HOLIDAYS, &empty).unwrap(), "");
        // Watchlist tickers are JSON strings, one per line.
        let w = json!({"path": "x", "symbols": ["BBCA", "TLKM"]});
        assert_eq!(
            render(SCHEMA_WATCHLIST, &w).unwrap(),
            "\"BBCA\"\n\"TLKM\"\n"
        );
    }

    #[test]
    fn unknown_schema_and_bare_values_fall_back() {
        let arr = json!([{"a": 1}, {"a": 2}]);
        assert_eq!(
            render("idx.nope.v1", &arr).unwrap(),
            "{\"a\":1}\n{\"a\":2}\n"
        );
        let obj = json!({"a": [1, 2]});
        assert_eq!(render("idx.nope.v1", &obj).unwrap(), "{\"a\":[1,2]}\n");
        // A Field rule whose field is missing prints the record rather than nothing.
        assert_eq!(render(SCHEMA_HISTORY, &obj).unwrap(), "{\"a\":[1,2]}\n");
        assert_eq!(render_value(&arr).unwrap(), "{\"a\":1}\n{\"a\":2}\n");
        assert_eq!(
            render_value(&json!({"version": "1"})).unwrap(),
            "{\"version\":\"1\"}\n"
        );
        assert_eq!(render_value(&json!([])).unwrap(), "");
    }
}
