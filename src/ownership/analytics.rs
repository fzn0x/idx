//! Pure analytics over the positions of a release (no SQLite): holder
//! search, holders across issuers, ownership concentration (HHI), the
//! holder-issuer walk and its Graphviz form.
//!
//! `db::analytics` loads the positions and calls these. The web terminal
//! has a JavaScript port (`web/src/ownership.js`) tested on the same
//! fixtures, so both faces rank and round alike.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use chrono::NaiveDate;

use crate::error::{IdxError, Result};
use crate::model::{
    Concentration, ConcentrationSort, CrossHolder, CrossPosition, GraphHolder, GraphIssuer,
    GraphLink, GraphReport, HolderOrigin, SearchMatch, SearchMatchKind,
};

use super::normalize;

/// One holder's position in one issuer (accounts already summed), as the
/// analytics read it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stake {
    pub symbol: String,
    pub issuer_name: Option<String>,
    pub entity_key: String,
    /// Display name.
    pub holder: String,
    pub investor_type: Option<String>,
    pub local_foreign: Option<String>,
    pub controller_or_affiliate: Option<String>,
    pub shares_total: i64,
    pub pct: f64,
}

/// Rounds to 4 decimals, never `-0.0`.
pub fn round4(v: f64) -> f64 {
    let r = (v * 10_000.0).round() / 10_000.0;
    if r == 0.0 {
        0.0
    } else {
        r
    }
}

const EPS: f64 = 1e-9;

/// Largest first: percent descending, then the given tie-break.
fn by_pct_desc(a: f64, b: f64) -> std::cmp::Ordering {
    b.partial_cmp(&a).unwrap_or(std::cmp::Ordering::Equal)
}

// ---- search -------------------------------------------------------------------

/// A holder the database knows, for [`search`].
#[derive(Debug, Clone, PartialEq)]
pub struct Known {
    pub entity_key: String,
    pub holder: String,
    /// Every spelling recorded for the holder.
    pub spellings: Vec<String>,
    /// Newest release that lists the holder.
    pub last_seen: NaiveDate,
}

fn collapse_upper(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// The normalised query of a search; it needs 2 letters or digits, or it
/// would match every holder.
pub fn search_key(query: &str) -> Result<String> {
    let key = normalize::entity_key(query);
    if key.chars().filter(|c| c.is_alphanumeric()).count() < 2 {
        return Err(IdxError::Usage(
            "search needs at least 2 letters or digits".into(),
        ));
    }
    Ok(key)
}

/// How `key` (with its recorded spellings) matches the normalised query
/// `qk` and the collapsed upper-case query `qs`; `None` when it does not.
pub fn match_kind(qk: &str, qs: &str, key: &str, spellings: &[String]) -> Option<SearchMatchKind> {
    if key == qk {
        return Some(SearchMatchKind::Exact);
    }
    if spellings.iter().any(|s| collapse_upper(s) == qs) {
        return Some(SearchMatchKind::Spelling);
    }
    if key.starts_with(qk) {
        return Some(SearchMatchKind::Prefix);
    }
    let norm: Vec<String> = spellings.iter().map(|s| normalize::entity_key(s)).collect();
    if key.contains(qk) || norm.iter().any(|n| n.contains(qk)) {
        return Some(SearchMatchKind::Contains);
    }
    let words: HashSet<&str> = key
        .split(' ')
        .chain(norm.iter().flat_map(|n| n.split(' ')))
        .collect();
    if qk
        .split(' ')
        .all(|w| words.iter().any(|x| x.starts_with(w)))
    {
        return Some(SearchMatchKind::Words);
    }
    None
}

/// What one holder has in a release: issuer count, summed percent and the
/// type and flag of its largest position.
struct Presence<'a> {
    symbols: BTreeSet<&'a str>,
    total: f64,
    largest: &'a Stake,
}

fn presence(stakes: &[Stake]) -> HashMap<&str, Presence<'_>> {
    let mut out: HashMap<&str, Presence> = HashMap::new();
    for s in stakes {
        let p = out.entry(s.entity_key.as_str()).or_insert(Presence {
            symbols: BTreeSet::new(),
            total: 0.0,
            largest: s,
        });
        p.symbols.insert(s.symbol.as_str());
        p.total += s.pct;
        if s.pct > p.largest.pct + EPS
            || ((s.pct - p.largest.pct).abs() <= EPS && s.symbol < p.largest.symbol)
        {
            p.largest = s;
        }
    }
    out
}

/// Every known holder matching `query`, ranked: exact and spelling matches
/// first, then holders present in `present` (one release's positions)
/// before absent ones, then prefix, contains and word matches, then more
/// issuers, then the shorter key, then the key.
pub fn search(known: &[Known], present: &[Stake], query: &str) -> Result<Vec<SearchMatch>> {
    let qk = search_key(query)?;
    let qs = collapse_upper(query);
    let here = presence(present);
    let mut out: Vec<SearchMatch> = known
        .iter()
        .filter_map(|k| {
            let kind = match_kind(&qk, &qs, &k.entity_key, &k.spellings)?;
            let p = here.get(k.entity_key.as_str());
            let mut spellings = k.spellings.clone();
            spellings.sort();
            spellings.dedup();
            Some(SearchMatch {
                entity_key: k.entity_key.clone(),
                holder: k.holder.clone(),
                matched: kind,
                spellings,
                issuers: p.map(|p| p.symbols.len()).unwrap_or(0),
                total_pct: p.map(|p| round4(p.total)).unwrap_or(0.0),
                investor_type: p.and_then(|p| p.largest.investor_type.clone()),
                local_foreign: p.and_then(|p| p.largest.local_foreign.clone()),
                last_seen: k.last_seen,
            })
        })
        .collect();
    let strong =
        |k: SearchMatchKind| !matches!(k, SearchMatchKind::Exact | SearchMatchKind::Spelling);
    out.sort_by(|a, b| {
        strong(a.matched)
            .cmp(&strong(b.matched))
            .then((a.issuers == 0).cmp(&(b.issuers == 0)))
            .then(a.matched.cmp(&b.matched))
            .then(b.issuers.cmp(&a.issuers))
            .then(a.entity_key.len().cmp(&b.entity_key.len()))
            .then(a.entity_key.cmp(&b.entity_key))
    });
    Ok(out)
}

// ---- cross-holders ------------------------------------------------------------

/// Filters of [`cross_holders`].
#[derive(Debug, Clone, Default)]
pub struct CrossFilter {
    pub min_issuers: usize,
    /// Case-insensitive substring of the KSEI investor type.
    pub investor_type: Option<String>,
    pub origin: Option<HolderOrigin>,
}

/// Whether `investor_type` contains `wanted` (case and spacing ignored);
/// a missing type never matches.
pub fn type_matches(investor_type: Option<&str>, wanted: &str) -> bool {
    investor_type.is_some_and(|t| collapse_upper(t).contains(&collapse_upper(wanted)))
}

/// Holders of `stakes` (one release) grouped by key, filtered, kept when
/// they hold at least `min_issuers` issuers, sorted by issuers, then summed
/// percent, then name.
pub fn cross_holders(stakes: &[Stake], f: &CrossFilter) -> Vec<CrossHolder> {
    let mut groups: BTreeMap<&str, Vec<&Stake>> = BTreeMap::new();
    for s in stakes {
        if let Some(t) = &f.investor_type {
            if !type_matches(s.investor_type.as_deref(), t) {
                continue;
            }
        }
        if let Some(o) = f.origin {
            if s.local_foreign.as_deref() != Some(o.flag()) {
                continue;
            }
        }
        groups.entry(s.entity_key.as_str()).or_default().push(s);
    }
    let mut out: Vec<CrossHolder> = groups
        .into_iter()
        .filter_map(|(key, mut list)| {
            list.sort_by(|a, b| by_pct_desc(a.pct, b.pct).then(a.symbol.cmp(&b.symbol)));
            let issuers = list
                .iter()
                .map(|s| s.symbol.as_str())
                .collect::<BTreeSet<_>>()
                .len();
            if issuers < f.min_issuers {
                return None;
            }
            let largest = list[0];
            Some(CrossHolder {
                holder: largest.holder.clone(),
                entity_key: key.to_string(),
                investor_type: largest.investor_type.clone(),
                local_foreign: largest.local_foreign.clone(),
                issuers,
                total_pct: round4(list.iter().map(|s| s.pct).sum()),
                max_pct: round4(largest.pct),
                positions: list
                    .iter()
                    .map(|s| CrossPosition {
                        symbol: s.symbol.clone(),
                        issuer_name: s.issuer_name.clone(),
                        shares_total: s.shares_total,
                        pct: round4(s.pct),
                    })
                    .collect(),
            })
        })
        .collect();
    out.sort_by(|a, b| {
        b.issuers
            .cmp(&a.issuers)
            .then(by_pct_desc(a.total_pct, b.total_pct))
            .then(a.holder.cmp(&b.holder))
            .then(a.entity_key.cmp(&b.entity_key))
    });
    out
}

// ---- concentration ------------------------------------------------------------

/// One row per issuer (symbol order): the listed holders' stakes, their
/// sum, the undisclosed rest, the HHI (sum of squared stakes, 0-10,000) and,
/// when the release reports it, the controller group's stake.
pub fn concentration(stakes: &[Stake], controller_reported: bool) -> Vec<Concentration> {
    let mut by_symbol: BTreeMap<&str, Vec<&Stake>> = BTreeMap::new();
    for s in stakes {
        by_symbol.entry(s.symbol.as_str()).or_default().push(s);
    }
    by_symbol
        .into_iter()
        .map(|(symbol, mut list)| {
            list.sort_by(|a, b| by_pct_desc(a.pct, b.pct).then(a.holder.cmp(&b.holder)));
            let listed: f64 = list.iter().map(|s| s.pct).sum();
            Concentration {
                symbol: symbol.to_string(),
                issuer_name: list.iter().find_map(|s| s.issuer_name.clone()),
                holders_listed: list.len(),
                top_holder: list[0].holder.clone(),
                top1_pct: round4(list[0].pct),
                top3_pct: round4(list.iter().take(3).map(|s| s.pct).sum()),
                listed_pct: round4(listed),
                undisclosed_pct: round4((100.0 - listed).max(0.0)),
                hhi: round4(list.iter().map(|s| s.pct * s.pct).sum()),
                controller_pct: controller_reported.then(|| {
                    round4(
                        list.iter()
                            .filter(|s| s.controller_or_affiliate.is_some())
                            .map(|s| s.pct)
                            .sum(),
                    )
                }),
            }
        })
        .collect()
}

/// Sorts by `by`, largest first (smallest with `ascending`), ties by symbol.
pub fn rank_concentration(rows: &mut [Concentration], by: ConcentrationSort, ascending: bool) {
    let value = |c: &Concentration| match by {
        ConcentrationSort::Hhi => c.hhi,
        ConcentrationSort::Top1 => c.top1_pct,
        ConcentrationSort::Top3 => c.top3_pct,
    };
    rows.sort_by(|a, b| {
        let o = value(a)
            .partial_cmp(&value(b))
            .unwrap_or(std::cmp::Ordering::Equal);
        (if ascending { o } else { o.reverse() }).then(a.symbol.cmp(&b.symbol))
    });
}

// ---- graph --------------------------------------------------------------------

pub fn issuer_id(symbol: &str) -> String {
    format!("ticker:{symbol}")
}

pub fn holder_id(entity_key: &str) -> String {
    format!("holder:{entity_key}")
}

/// Where a walk starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Root {
    Issuer(String),
    Holder(String),
}

impl Root {
    pub fn id(&self) -> String {
        match self {
            Root::Issuer(s) => issuer_id(s),
            Root::Holder(k) => holder_id(k),
        }
    }
}

/// The nodes and links a walk reached.
#[derive(Debug, Clone, PartialEq)]
pub struct Walk {
    pub truncated: bool,
    pub issuers: Vec<GraphIssuer>,
    pub holders: Vec<GraphHolder>,
    pub links: Vec<GraphLink>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Node<'a> {
    Issuer(&'a str),
    Holder(&'a str),
}

/// Breadth-first walk over the bipartite holder-issuer graph of one
/// release, from `root`, for `depth` hops, over holdings of at least
/// `min_pct` percent. Within a hop, nodes expand in the order they were
/// reached and each node's neighbours by percent (largest first), so
/// `max_nodes` keeps the strongest links; reaching it stops the walk with
/// `truncated`. Links are every holding between two reached nodes.
pub fn walk(stakes: &[Stake], root: &Root, depth: usize, min_pct: f64, max_nodes: usize) -> Walk {
    let kept: Vec<&Stake> = stakes.iter().filter(|s| s.pct + EPS >= min_pct).collect();
    let mut by_issuer: HashMap<&str, Vec<&Stake>> = HashMap::new();
    let mut by_holder: HashMap<&str, Vec<&Stake>> = HashMap::new();
    for s in &kept {
        by_issuer.entry(s.symbol.as_str()).or_default().push(s);
        by_holder.entry(s.entity_key.as_str()).or_default().push(s);
    }
    for list in by_issuer.values_mut() {
        list.sort_by(|a, b| by_pct_desc(a.pct, b.pct).then(a.entity_key.cmp(&b.entity_key)));
    }
    for list in by_holder.values_mut() {
        list.sort_by(|a, b| by_pct_desc(a.pct, b.pct).then(a.symbol.cmp(&b.symbol)));
    }
    let start = match root {
        Root::Issuer(s) => Node::Issuer(s.as_str()),
        Root::Holder(k) => Node::Holder(k.as_str()),
    };
    let mut hops: HashMap<Node, usize> = HashMap::from([(start, 0)]);
    let mut order: Vec<Node> = vec![start];
    let mut frontier = vec![start];
    let mut truncated = false;
    'walk: for hop in 1..=depth {
        let mut next = Vec::new();
        for node in &frontier {
            let neighbours: Vec<Node> = match node {
                Node::Issuer(s) => by_issuer
                    .get(s)
                    .map(|l| {
                        l.iter()
                            .map(|x| Node::Holder(x.entity_key.as_str()))
                            .collect()
                    })
                    .unwrap_or_default(),
                Node::Holder(k) => by_holder
                    .get(k)
                    .map(|l| l.iter().map(|x| Node::Issuer(x.symbol.as_str())).collect())
                    .unwrap_or_default(),
            };
            for n in neighbours {
                if hops.contains_key(&n) {
                    continue;
                }
                if hops.len() >= max_nodes {
                    truncated = true;
                    break 'walk;
                }
                hops.insert(n, hop);
                order.push(n);
                next.push(n);
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    let mut issuers = Vec::new();
    let mut holders = Vec::new();
    for n in &order {
        let h = hops[n];
        match n {
            Node::Issuer(s) => issuers.push(GraphIssuer {
                id: issuer_id(s),
                symbol: s.to_string(),
                issuer_name: by_issuer
                    .get(s)
                    .and_then(|l| l.iter().find_map(|x| x.issuer_name.clone())),
                hops: h,
            }),
            Node::Holder(k) => {
                let largest = by_holder.get(k).and_then(|l| l.first());
                holders.push(GraphHolder {
                    id: holder_id(k),
                    entity_key: k.to_string(),
                    holder: largest
                        .map(|x| x.holder.clone())
                        .unwrap_or_else(|| k.to_string()),
                    investor_type: largest.and_then(|x| x.investor_type.clone()),
                    local_foreign: largest.and_then(|x| x.local_foreign.clone()),
                    hops: h,
                })
            }
        }
    }
    let mut links: Vec<GraphLink> = kept
        .iter()
        .filter_map(|s| {
            let a = hops.get(&Node::Holder(s.entity_key.as_str()))?;
            let b = hops.get(&Node::Issuer(s.symbol.as_str()))?;
            Some(GraphLink {
                from: holder_id(&s.entity_key),
                to: issuer_id(&s.symbol),
                holder: s.holder.clone(),
                symbol: s.symbol.clone(),
                shares_total: s.shares_total,
                pct: round4(s.pct),
                hops: (*a).max(*b),
            })
        })
        .collect();
    links.sort_by(|a, b| {
        a.hops
            .cmp(&b.hops)
            .then(a.symbol.cmp(&b.symbol))
            .then(by_pct_desc(a.pct, b.pct))
            .then(a.from.cmp(&b.from))
    });
    Walk {
        truncated,
        issuers,
        holders,
        links,
    }
}

/// Escapes a Graphviz quoted string: backslash, double quote, newline.
pub fn dot_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// The walk as a Graphviz digraph: issuers as boxes, holders as ellipses,
/// one edge per holding (holder -> issuer, labelled with the percent), the
/// root in bold. Deterministic: nodes by hops, issuers first, then id;
/// edges by hops, issuer, percent (largest first), holder.
pub fn to_dot(r: &GraphReport) -> String {
    let mut out = String::from("digraph ownership {\n");
    out.push_str(&format!(
        "  // idx ownership graph from {} · KSEI register as of {} · holdings above 1 % · depth {}{}\n",
        dot_escape(&r.root).replace("\\n", " "),
        r.as_of,
        r.depth,
        if r.min_pct > 0.0 {
            format!(" · min {}%", r.min_pct)
        } else {
            String::new()
        }
    ));
    out.push_str("  rankdir=LR;\n");
    out.push_str("  node [fontname=\"Helvetica\", fontsize=10];\n");
    out.push_str("  edge [fontname=\"Helvetica\", fontsize=9];\n");
    let mut nodes: Vec<(usize, u8, String, String)> = Vec::new();
    for i in &r.issuers {
        let label = match &i.issuer_name {
            Some(n) => format!("{}\n{}", i.symbol, n),
            None => i.symbol.clone(),
        };
        nodes.push((
            i.hops,
            0,
            i.id.clone(),
            format!("label=\"{}\", shape=box", dot_escape(&label)),
        ));
    }
    for h in &r.holders {
        nodes.push((
            h.hops,
            1,
            h.id.clone(),
            format!("label=\"{}\", shape=ellipse", dot_escape(&h.holder)),
        ));
    }
    nodes.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
    for (_, _, id, attrs) in &nodes {
        let bold = if *id == r.root { ", style=bold" } else { "" };
        out.push_str(&format!("  \"{}\" [{attrs}{bold}];\n", dot_escape(id)));
    }
    let mut edges: Vec<&GraphLink> = r.links.iter().collect();
    edges.sort_by(|a, b| {
        a.hops
            .cmp(&b.hops)
            .then(a.to.cmp(&b.to))
            .then(by_pct_desc(a.pct, b.pct))
            .then(a.from.cmp(&b.from))
    });
    for l in edges {
        out.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"{:.2}%\"];\n",
            dot_escape(&l.from),
            dot_escape(&l.to),
            l.pct
        ));
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stake(symbol: &str, key: &str, pct: f64) -> Stake {
        Stake {
            symbol: symbol.into(),
            issuer_name: Some(format!("{symbol} Tbk")),
            entity_key: key.into(),
            holder: key.into(),
            investor_type: None,
            local_foreign: None,
            controller_or_affiliate: None,
            shares_total: (pct * 1_000.0) as i64,
            pct,
        }
    }

    fn typed(s: Stake, investor_type: &str, flag: &str) -> Stake {
        Stake {
            investor_type: Some(investor_type.into()),
            local_foreign: Some(flag.into()),
            ..s
        }
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn hhi_top_stakes_and_the_undisclosed_rest() {
        let stakes: Vec<Stake> = [40.0, 25.0, 10.0, 5.0]
            .iter()
            .enumerate()
            .map(|(i, p)| stake("AAAA", &format!("H{i}"), *p))
            .collect();
        let rows = concentration(&stakes, false);
        assert_eq!(rows.len(), 1);
        let c = &rows[0];
        assert_eq!(c.hhi, 2350.0);
        assert_eq!((c.top1_pct, c.top3_pct), (40.0, 75.0));
        assert_eq!((c.listed_pct, c.undisclosed_pct), (80.0, 20.0));
        assert_eq!((c.holders_listed, c.top_holder.as_str()), (4, "H0"));
        assert_eq!(c.controller_pct, None, "no controller column");

        // Two accounts summed into one position count once: 13.45^2.
        let kren = concentration(&[stake("KREN", "KRESNA", 13.45)], false);
        assert_eq!(kren[0].hhi, 180.9025);
        assert_eq!(kren[0].top3_pct, 13.45);

        // More than 100 % listed (rounding in the source): nothing undisclosed.
        let over = concentration(
            &[stake("OVER", "A", 60.0), stake("OVER", "B", 40.01)],
            false,
        );
        assert_eq!(over[0].undisclosed_pct, 0.0);
        assert_eq!(over[0].hhi, 5200.8001);

        // Ties on the largest stake go to the name, ascending.
        let tie = concentration(
            &[stake("TIE", "ZETA", 10.0), stake("TIE", "ALFA", 10.0)],
            false,
        );
        assert_eq!(tie[0].top_holder, "ALFA");
    }

    #[test]
    fn controller_pct_sums_the_flagged_stakes() {
        let mut a = stake("BBCA", "DWIMURIA", 54.94);
        a.controller_or_affiliate = Some("CONTROLLER".into());
        let mut b = stake("BBCA", "TRICIPTA", 1.07);
        b.controller_or_affiliate = Some("AFFILIATE OF THE CONTROLLER".into());
        let c = stake("BBCA", "ANTHONI SALIM", 1.15);
        let rows = concentration(&[a, b, c.clone()], true);
        assert_eq!(rows[0].controller_pct, Some(56.01));
        assert_eq!(concentration(&[c], true)[0].controller_pct, Some(0.0));
    }

    #[test]
    fn concentration_ranks_by_the_chosen_key() {
        let stakes = vec![
            stake("AAAA", "X", 50.0),
            stake("BBBB", "Y", 30.0),
            stake("BBBB", "Z", 30.0),
            stake("CCCC", "W", 50.0),
        ];
        let mut rows = concentration(&stakes, false);
        rank_concentration(&mut rows, ConcentrationSort::Hhi, false);
        let order: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(
            order,
            ["AAAA", "CCCC", "BBBB"],
            "hhi 2500, 2500 (symbol), 1800"
        );
        rank_concentration(&mut rows, ConcentrationSort::Top3, false);
        assert_eq!(rows[0].symbol, "BBBB");
        rank_concentration(&mut rows, ConcentrationSort::Top1, true);
        let order: Vec<&str> = rows.iter().map(|r| r.symbol.as_str()).collect();
        assert_eq!(order, ["BBBB", "AAAA", "CCCC"]);
    }

    #[test]
    fn cross_holders_filter_count_and_sort() {
        let stakes = vec![
            typed(stake("AAAA", "ALPHA", 10.0), "INDIVIDUAL", "L"),
            typed(stake("BBBB", "ALPHA", 5.0), "INDIVIDUAL", "L"),
            typed(stake("AAAA", "BETA", 2.0), "MUTUAL FUNDS", "A"),
            typed(stake("BBBB", "BETA", 3.0), "MUTUAL FUNDS", "A"),
            typed(stake("CCCC", "BETA", 4.0), "MUTUAL FUNDS", "A"),
            stake("CCCC", "GAMMA", 40.0),
        ];
        let all = cross_holders(
            &stakes,
            &CrossFilter {
                min_issuers: 2,
                ..Default::default()
            },
        );
        let keys: Vec<&str> = all.iter().map(|h| h.entity_key.as_str()).collect();
        assert_eq!(keys, ["BETA", "ALPHA"]);
        let beta = &all[0];
        assert_eq!((beta.issuers, beta.total_pct, beta.max_pct), (3, 9.0, 4.0));
        let order: Vec<&str> = beta.positions.iter().map(|p| p.symbol.as_str()).collect();
        assert_eq!(order, ["CCCC", "BBBB", "AAAA"]);
        assert_eq!(beta.investor_type.as_deref(), Some("MUTUAL FUNDS"));

        let funds = cross_holders(
            &stakes,
            &CrossFilter {
                min_issuers: 1,
                investor_type: Some(" mutual  fund".into()),
                origin: None,
            },
        );
        assert_eq!(funds.len(), 1, "substring, case and spacing ignored");
        let local = cross_holders(
            &stakes,
            &CrossFilter {
                min_issuers: 1,
                investor_type: None,
                origin: Some(HolderOrigin::Local),
            },
        );
        assert_eq!(local.len(), 1, "GAMMA has no flag and never matches");
        assert_eq!(local[0].entity_key, "ALPHA");
        let one = cross_holders(
            &stakes,
            &CrossFilter {
                min_issuers: 1,
                ..Default::default()
            },
        );
        assert_eq!(one.len(), 3);
        assert_eq!(one[2].entity_key, "GAMMA");
    }

    fn known(key: &str, spellings: &[&str], last: NaiveDate) -> Known {
        Known {
            entity_key: key.into(),
            holder: key.into(),
            spellings: spellings.iter().map(|s| s.to_string()).collect(),
            last_seen: last,
        }
    }

    #[test]
    fn search_kinds_and_order() {
        let sep = date(2026, 9, 30);
        let aug = date(2026, 8, 31);
        let all = vec![
            known("ANTHONI SALIM", &["ANTHONI SALIM"], sep),
            known("T PERMADI RACHMAT", &["T. PERMADI RACHMAT"], sep),
            known("IR T PERMADI RACHMAT", &["IR. T. PERMADI RACHMAT"], aug),
            known(
                "UOB KAY HIAN PRIVATE LIMITED",
                &["UOB KAY HIAN PRIVATE LIMITED"],
                sep,
            ),
            known("UOB KAY HIAN PTE LTD", &["UOB KAY HIAN PTE LTD"], aug),
        ];
        let present = vec![
            stake("BBCA", "ANTHONI SALIM", 1.15),
            stake("AADI", "T PERMADI RACHMAT", 3.48),
            stake("ASSA", "T PERMADI RACHMAT", 4.8),
            stake("ARTA", "UOB KAY HIAN PRIVATE LIMITED", 1.33),
        ];
        let kinds = |q: &str| -> Vec<(String, SearchMatchKind)> {
            search(&all, &present, q)
                .unwrap()
                .into_iter()
                .map(|m| (m.entity_key, m.matched))
                .collect()
        };
        assert_eq!(
            kinds("anthoni salim"),
            [("ANTHONI SALIM".to_string(), SearchMatchKind::Exact)]
        );
        assert_eq!(kinds("salim")[0].1, SearchMatchKind::Contains);
        assert_eq!(kinds("salim anthoni")[0].1, SearchMatchKind::Words);
        assert_eq!(kinds("sal anth")[0].1, SearchMatchKind::Words);
        let permadi = search(&all, &present, "permadi").unwrap();
        assert_eq!(permadi.len(), 2);
        assert_eq!(permadi[0].entity_key, "T PERMADI RACHMAT");
        assert_eq!((permadi[0].issuers, permadi[0].total_pct), (2, 8.28));
        assert_eq!(permadi[1].entity_key, "IR T PERMADI RACHMAT");
        assert_eq!((permadi[1].issuers, permadi[1].last_seen), (0, aug));
        assert_eq!(permadi[1].investor_type, None);
        // A recorded spelling beats a present holder.
        assert_eq!(
            kinds("ir.  t. permadi rachmat")[0],
            ("IR T PERMADI RACHMAT".to_string(), SearchMatchKind::Exact),
            "normalises to the key"
        );
        let uob = kinds("uob kay hian");
        assert_eq!(uob[0].0, "UOB KAY HIAN PRIVATE LIMITED", "present first");
        assert_eq!(uob[1].0, "UOB KAY HIAN PTE LTD");
        assert!(uob.iter().all(|(_, k)| *k == SearchMatchKind::Prefix));
        assert!(kinds("nobody here").is_empty());
        for bad in ["x", "%%", " ! ", "a."] {
            let err = search(&all, &present, bad).unwrap_err();
            assert_eq!(err.exit_code(), 2, "{bad:?}");
        }
        assert_eq!(
            match_kind(
                "SURONO SUBEKTI",
                "DRS. SURONO SUBEKTI",
                "DRS SURONO SUBEKTI",
                &["DRS. SURONO SUBEKTI".into()]
            ),
            Some(SearchMatchKind::Spelling)
        );
    }

    fn graph_stakes() -> Vec<Stake> {
        vec![
            stake("AADI", "ADARO", 41.1),
            stake("AADI", "ALAMTRI", 15.37),
            stake("AADI", "PERMADI", 3.48),
            stake("AADI", "PERSADA", 1.54),
            stake("TAPG", "PERSADA", 33.07),
            stake("TAPG", "PERMADI", 5.1),
            stake("TAPG", "TRIPUTRA", 24.03),
            stake("ASSA", "PERMADI", 4.8),
            stake("ASSA", "TRIPUTRA", 1.25),
            stake("BBCA", "DWIMURIA", 54.94),
        ]
    }

    #[test]
    fn walk_hops_links_and_caps() {
        let s = graph_stakes();
        let root = Root::Issuer("AADI".into());
        let one = walk(&s, &root, 1, 0.0, 100);
        assert_eq!(
            (one.issuers.len(), one.holders.len(), one.links.len()),
            (1, 4, 4)
        );
        assert!(!one.truncated);
        let two = walk(&s, &root, 2, 0.0, 100);
        assert_eq!((two.issuers.len(), two.holders.len()), (3, 4));
        // AADI x4, TAPG (PERSADA, PERMADI), ASSA (PERMADI): links between reached nodes only.
        assert_eq!(two.links.len(), 7);
        let syms: Vec<&str> = two.issuers.iter().map(|i| i.symbol.as_str()).collect();
        assert_eq!(
            syms,
            ["AADI", "TAPG", "ASSA"],
            "PERMADI's issuers by percent"
        );
        assert!(two.links.iter().all(|l| l.hops <= 2));
        assert_eq!(two.links[0].hops, 1);
        let three = walk(&s, &root, 3, 0.0, 100);
        assert_eq!(three.holders.len(), 5, "TRIPUTRA at hop 3");
        assert_eq!(
            three.links.len(),
            9,
            "the link between two visited nodes is kept"
        );
        assert!(three.links.iter().all(|l| l.symbol != "BBCA"));

        let big = walk(&s, &root, 2, 5.0, 100);
        assert_eq!(
            (big.issuers.len(), big.holders.len(), big.links.len()),
            (1, 2, 2)
        );

        let capped = walk(&s, &root, 2, 0.0, 3);
        assert!(capped.truncated);
        let keys: Vec<&str> = capped
            .holders
            .iter()
            .map(|h| h.entity_key.as_str())
            .collect();
        assert_eq!(keys, ["ADARO", "ALAMTRI"], "strongest links first");
        let exact = walk(&s, &root, 1, 0.0, 5);
        assert!(
            !exact.truncated,
            "the cap was reached with nothing left to add"
        );

        let holder = walk(&s, &Root::Holder("PERMADI".into()), 1, 0.0, 100);
        assert_eq!(holder.holders[0].hops, 0);
        let syms: Vec<&str> = holder.issuers.iter().map(|i| i.symbol.as_str()).collect();
        assert_eq!(syms, ["TAPG", "ASSA", "AADI"]);
        assert_eq!(holder.links[0].from, "holder:PERMADI");
        assert_eq!(holder.links[0].to, "ticker:AADI");
    }

    #[test]
    fn dot_is_deterministic_and_escaped() {
        let mut s = graph_stakes();
        s.push(Stake {
            holder: "QUOTE \"Q\" \\ BACK\nSLASH".into(),
            ..stake("BBCA", "QUOTE Q BACK SLASH", 2.0)
        });
        let w = walk(&s, &Root::Issuer("BBCA".into()), 1, 0.0, 100);
        let report = GraphReport {
            as_of: date(2026, 9, 30),
            root: "ticker:BBCA".into(),
            depth: 1,
            min_pct: 0.0,
            max_nodes: 100,
            truncated: w.truncated,
            issuers: w.issuers,
            holders: w.holders,
            links: w.links,
        };
        let dot = to_dot(&report);
        assert!(dot.starts_with("digraph ownership {\n"));
        assert!(dot.ends_with("}\n"));
        assert!(dot.contains("\"ticker:BBCA\" [label=\"BBCA\\nBBCA Tbk\", shape=box, style=bold];"));
        assert!(
            dot.contains("[label=\"QUOTE \\\"Q\\\" \\\\ BACK\\nSLASH\", shape=ellipse];"),
            "{dot}"
        );
        assert!(dot.contains("\"holder:DWIMURIA\" -> \"ticker:BBCA\" [label=\"54.94%\"];"));
        let dw = dot.find("holder:DWIMURIA\" ->").unwrap();
        let q = dot.find("holder:QUOTE Q BACK SLASH\" ->").unwrap();
        assert!(dw < q, "edges by percent");
        assert_eq!(dot, to_dot(&report));
    }

    #[test]
    fn keys_match_the_web_port() {
        // web/test/ownership.test.mjs asserts the same keys for entityKey.
        assert_eq!(
            normalize::entity_key("Soci\u{e9}t\u{e9} G\u{e9}n\u{e9}rale, S.A."),
            "SOCI\u{c9}T\u{c9} G\u{c9}N\u{c9}RALE S A"
        );
        assert_eq!(
            normalize::entity_key("IR. T. PERMADI RACHMAT"),
            "IR T PERMADI RACHMAT"
        );
    }
}
