//! Symbols from stdin: `-` in a ticker list reads the tickers piped in, so
//! commands chain (`echo BBCA BBRI | idx stocks quote -`,
//! `idx -o jsonl stocks screen -f "pe<10" | idx stocks compare -`).
//!
//! Text input: tickers separated by spaces, commas or newlines; `#` starts a
//! comment to the end of the line, so a watchlist file works; a UTF-8 BOM and
//! CRLF line ends are ignored. JSON input (first non-blank character `{`, `[`
//! or `"`): one document or JSON Lines. An idx envelope gives its rows (the same
//! rows `-o jsonl` prints, [`jsonl::rows`]), an array its items, anything else
//! is itself a row; a row that is a string is a ticker, an object gives its
//! `symbol`.

use std::io::{IsTerminal, Read};

use serde_json::Value;

use crate::error::{IdxError, Result};
use crate::output::jsonl;

/// The symbol arguments with a `-` replaced by the tickers on stdin. Leaves
/// the list untouched (and stdin unread) when there is no `-`.
pub fn symbol_args(raw: &[String]) -> Result<Vec<String>> {
    expand(raw, std::io::stdin().is_terminal(), || {
        let mut text = String::new();
        std::io::stdin().read_to_string(&mut text).map(|_| text)
    })
}

/// [`symbol_args`] with stdin injected, for tests.
pub fn expand(
    raw: &[String],
    stdin_is_terminal: bool,
    read: impl FnOnce() -> std::io::Result<String>,
) -> Result<Vec<String>> {
    let pieces: Vec<&str> = raw
        .iter()
        .flat_map(|r| r.split(|c: char| c == ',' || c.is_whitespace()))
        .filter(|p| !p.is_empty())
        .collect();
    match pieces.iter().filter(|p| **p == "-").count() {
        0 => return Ok(raw.to_vec()),
        1 => {}
        _ => {
            return Err(IdxError::Usage(
                "`-` may appear only once in a symbol list".into(),
            ))
        }
    }
    if stdin_is_terminal {
        return Err(IdxError::Usage(
            "`-` reads symbols from stdin, but stdin is a terminal; pipe them in, \
             e.g. echo BBCA BBRI | idx stocks quote -"
                .into(),
        ));
    }
    let text = read().map_err(|e| IdxError::Usage(format!("could not read stdin: {e}")))?;
    let piped = symbols_from_input(&text)?;
    let mut out = Vec::with_capacity(pieces.len() + piped.len());
    for p in pieces {
        if p == "-" {
            out.extend(piped.iter().cloned());
        } else {
            out.push(p.to_string());
        }
    }
    Ok(out)
}

/// The tickers in text or JSON read from stdin, in order (not yet
/// validated or deduplicated; `Symbol::parse_list` does both).
pub fn symbols_from_input(text: &str) -> Result<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let json = text.trim_start().starts_with(['{', '[', '"']);
    let out = if json {
        symbols_from_json(text)?
    } else {
        text.lines()
            .flat_map(|line| {
                line.split('#')
                    .next()
                    .unwrap_or("")
                    .split(|c: char| c == ',' || c.is_whitespace())
            })
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    };
    if out.is_empty() {
        return Err(IdxError::Usage(if json {
            "no symbols read from stdin: the JSON has no `symbol` fields".into()
        } else {
            "no symbols read from stdin".into()
        }));
    }
    Ok(out)
}

fn symbols_from_json(text: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for doc in serde_json::Deserializer::from_str(text).into_iter::<Value>() {
        let doc = doc.map_err(|e| {
            IdxError::Usage(format!("stdin looks like JSON but does not parse: {e}"))
        })?;
        let rows: Vec<&Value> = match (doc.get("schema").and_then(Value::as_str), doc.get("data")) {
            (Some(schema), Some(data)) => jsonl::rows(schema, data),
            _ => match doc.as_array() {
                Some(items) => items.iter().collect(),
                None => vec![&doc],
            },
        };
        for row in rows {
            let sym = match row {
                Value::String(s) => Some(s.as_str()),
                Value::Object(m) => m.get("symbol").and_then(Value::as_str),
                _ => None,
            };
            if let Some(s) = sym.map(str::trim).filter(|s| !s.is_empty()) {
                out.push(s.to_string());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Quote, Symbol};
    use crate::output::json::{to_pretty, Envelope, SCHEMA_QUOTES};
    use crate::providers::Sourced;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn from(text: &str) -> Vec<String> {
        symbols_from_input(text).unwrap()
    }

    #[test]
    fn text_splits_on_spaces_commas_and_lines() {
        assert_eq!(from("BBCA\nBBRI, TLKM\n"), ["BBCA", "BBRI", "TLKM"]);
        assert_eq!(from("bbca bbri\r\ntlkm\r\n"), ["bbca", "bbri", "tlkm"]);
        assert_eq!(from("\u{feff}BBCA\n"), ["BBCA"]);
        let watchlist = "# idx watchlist: one ticker per line\nBBCA\nBBRI   # banks\n\nTLKM\n";
        assert_eq!(from(watchlist), ["BBCA", "BBRI", "TLKM"]);
    }

    #[test]
    fn json_envelopes_lines_and_strings() {
        let quotes = vec![
            Quote::empty(Symbol::parse("BBCA").unwrap(), "t"),
            Quote::empty(Symbol::parse("BBRI").unwrap(), "t"),
        ];
        let env = Envelope::from_sourced(SCHEMA_QUOTES, Sourced::fresh(quotes, "t"));
        assert_eq!(from(&to_pretty(&env).unwrap()), ["BBCA", "BBRI"]);
        // JSON Lines: objects with a symbol, and plain strings.
        assert_eq!(
            from("{\"symbol\":\"TLKM\",\"price\":1}\n{\"symbol\":\"ASII\"}\n"),
            ["TLKM", "ASII"]
        );
        assert_eq!(from("\"BBCA\"\n\"TLKM\"\n"), ["BBCA", "TLKM"]);
        assert_eq!(from("[\"BBCA\", {\"symbol\": \"BMRI\"}]"), ["BBCA", "BMRI"]);
        // An envelope's rows follow the schema's JSONL rule.
        let screen = r#"{"schema":"idx.screen.v1","data":{"total_matches":2,
            "rows":[{"symbol":"BBCA"},{"symbol":"BBNI"}]}}"#;
        assert_eq!(from(screen), ["BBCA", "BBNI"]);
        let watch = r#"{"schema":"idx.watchlist.v1","data":{"path":"x","symbols":["TLKM"]}}"#;
        assert_eq!(from(watch), ["TLKM"]);
    }

    #[test]
    fn bad_or_empty_input_is_a_usage_error() {
        for text in [
            "",
            "  \n# only a comment\n",
            "{\"name\":\"no symbol\"}",
            "[]",
        ] {
            let e = symbols_from_input(text).unwrap_err();
            assert_eq!(e.exit_code(), 2, "{text:?}");
            assert!(e.to_string().contains("no symbols read from stdin"), "{e}");
        }
        let e = symbols_from_input("{\"symbol\": ").unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().contains("does not parse"), "{e}");
    }

    #[test]
    fn dash_expands_in_place_once() {
        let read = || Ok("BBRI\nBMRI\n".to_string());
        assert_eq!(
            expand(&args(&["BBCA", "-", "TLKM"]), false, read).unwrap(),
            ["BBCA", "BBRI", "BMRI", "TLKM"]
        );
        // Inside a comma list too.
        assert_eq!(
            expand(&args(&["BBCA,-"]), false, || Ok("ASII".into())).unwrap(),
            ["BBCA", "ASII"]
        );
        // No dash: stdin is never read, the arguments pass through.
        let untouched = expand(&args(&["BBCA,BBRI"]), true, || panic!("read")).unwrap();
        assert_eq!(untouched, ["BBCA,BBRI"]);
        let e = expand(&args(&["-", "-"]), false, || Ok("BBCA".into())).unwrap_err();
        assert!(e.to_string().contains("only once"), "{e}");
        let e = expand(&args(&["-"]), true, || panic!("must not block")).unwrap_err();
        assert_eq!(e.exit_code(), 2);
        assert!(e.to_string().contains("stdin is a terminal"), "{e}");
        let e = expand(&args(&["-"]), false, || Ok(String::new())).unwrap_err();
        assert!(e.to_string().contains("no symbols read from stdin"), "{e}");
        let e = expand(&args(&["-"]), false, || {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "not utf-8",
            ))
        })
        .unwrap_err();
        assert_eq!(e.exit_code(), 2);
    }
}
