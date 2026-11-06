//! Read queries behind `ownership search | cross-holders | concentration |
//! graph`: load the positions of a release (one row
//! per holder and issuer, accounts summed), then rank them with the pure
//! functions in `ownership::analytics`.

use std::collections::{BTreeMap, HashMap};

use chrono::NaiveDate;

use super::{db_err, parse_date, OwnershipDb};
use crate::error::{IdxError, Result};
use crate::model::{
    ConcentrationReport, ConcentrationSort, CrossHoldersReport, GraphReport, SearchReport, Symbol,
};
use crate::ownership::analytics::{self, CrossFilter, Known, Root, Stake};
use crate::ownership::normalize;

/// Bounds of `ownership graph`.
pub const MAX_DEPTH: usize = 6;
pub const MAX_NODES: usize = 1000;

impl OwnershipDb {
    /// Positions of one release, by symbol and holder key.
    pub(crate) fn stakes(&self, release_id: i64) -> Result<Vec<Stake>> {
        let mut st = self
            .conn
            .prepare(
                "SELECT h.symbol, h.issuer_name, e.entity_key, e.display_name, h.investor_type,
                        h.local_foreign, h.controller_or_affiliate, h.shares_total, h.pct
                 FROM holdings h JOIN entities e ON e.id = h.entity_id
                 WHERE h.release_id = ?1 ORDER BY h.symbol, e.entity_key",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([release_id], |r| {
                Ok(Stake {
                    symbol: r.get(0)?,
                    issuer_name: r.get(1)?,
                    entity_key: r.get(2)?,
                    holder: r.get(3)?,
                    investor_type: r.get(4)?,
                    local_foreign: r.get(5)?,
                    controller_or_affiliate: r.get(6)?,
                    shares_total: r.get(7)?,
                    pct: r.get(8)?,
                })
            })
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// Every spelling recorded per entity id.
    fn spellings(&self) -> Result<HashMap<i64, Vec<String>>> {
        let mut st = self
            .conn
            .prepare("SELECT entity_id, raw_name FROM aliases ORDER BY raw_name")
            .map_err(db_err)?;
        let mut out: HashMap<i64, Vec<String>> = HashMap::new();
        let rows = st
            .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            .map_err(db_err)?;
        for row in rows {
            let (id, raw) = row.map_err(db_err)?;
            out.entry(id).or_default().push(raw);
        }
        Ok(out)
    }

    /// Holders with at least one position in any release, with their
    /// spellings and last release.
    fn known_holders(&self) -> Result<Vec<Known>> {
        let spellings = self.spellings()?;
        let mut st = self
            .conn
            .prepare(
                "SELECT e.id, e.entity_key, e.display_name, MAX(r.as_of)
                 FROM entities e
                 JOIN holdings h ON h.entity_id = e.id
                 JOIN releases r ON r.id = h.release_id
                 GROUP BY e.id ORDER BY e.entity_key",
            )
            .map_err(db_err)?;
        let rows = st
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(db_err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_err)?;
        Ok(rows
            .into_iter()
            .map(|(id, key, name, last)| Known {
                entity_key: key,
                holder: name,
                spellings: spellings.get(&id).cloned().unwrap_or_default(),
                last_seen: parse_date(&last),
            })
            .collect())
    }

    /// `ownership search`: holders of every imported release matching
    /// `query` (key or any spelling, words in any order); issuers are
    /// counted in the release for `as_of` (newest when `None`). Returns the
    /// report and its warnings.
    pub fn search(
        &self,
        query: &str,
        as_of: Option<NaiveDate>,
        limit: Option<usize>,
    ) -> Result<(SearchReport, Vec<String>)> {
        analytics::search_key(query)?;
        let (release_id, date) = self.release_for(as_of)?;
        let present = self.stakes(release_id)?;
        let mut matches = analytics::search(&self.known_holders()?, &present, query)?;
        let total = matches.len();
        if let Some(n) = limit {
            matches.truncate(n);
        }
        let mut warnings = Vec::new();
        if total == 0 {
            warnings.push(format!("no holder matching {:?}", query.trim()));
        }
        Ok((
            SearchReport {
                query: query.trim().to_string(),
                as_of: date,
                total_matches: total,
                matches,
            },
            warnings,
        ))
    }

    /// `ownership cross-holders`: holders ranked by the issuers they own
    /// more than 1 % of in one release.
    pub fn cross_holders(
        &self,
        filter: &CrossFilter,
        limit: Option<usize>,
        as_of: Option<NaiveDate>,
    ) -> Result<CrossHoldersReport> {
        if filter.min_issuers == 0 {
            return Err(IdxError::Usage("--min-issuers must be at least 1".into()));
        }
        let investor_type = filter
            .investor_type
            .as_deref()
            .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|t| !t.is_empty());
        let filter = CrossFilter {
            investor_type: investor_type.clone(),
            ..filter.clone()
        };
        let (release_id, date) = self.release_for(as_of)?;
        let mut holders = analytics::cross_holders(&self.stakes(release_id)?, &filter);
        let total = holders.len();
        if let Some(n) = limit {
            holders.truncate(n);
        }
        Ok(CrossHoldersReport {
            as_of: date,
            min_issuers: filter.min_issuers,
            investor_type,
            origin: filter.origin,
            total_matches: total,
            holders,
        })
    }

    /// `ownership concentration`: every issuer ranked by `by` (or the given
    /// symbols in their order, `limit` ignored). Returns the report and its
    /// warnings (symbols without holders).
    pub fn concentration(
        &self,
        symbols: Option<&[Symbol]>,
        by: ConcentrationSort,
        ascending: bool,
        limit: Option<usize>,
        as_of: Option<NaiveDate>,
    ) -> Result<(ConcentrationReport, Vec<String>)> {
        let (release_id, date) = self.release_for(as_of)?;
        let reported = self.controller_reported(release_id)?;
        let mut rows = analytics::concentration(&self.stakes(release_id)?, reported);
        let mut warnings = Vec::new();
        let total;
        match symbols {
            Some(list) => {
                let mut by_symbol: BTreeMap<String, _> =
                    rows.into_iter().map(|r| (r.symbol.clone(), r)).collect();
                rows = Vec::new();
                for s in list {
                    match by_symbol.remove(s.as_str()) {
                        Some(r) => rows.push(r),
                        None => warnings.push(format!(
                            "no holders above 1% recorded for {} as of {date}",
                            s.as_str()
                        )),
                    }
                }
                if rows.is_empty() {
                    return Err(IdxError::NotFound(format!(
                        "no holders above 1% recorded for {} as of {date}",
                        list.iter()
                            .map(Symbol::as_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
                total = rows.len();
            }
            None => {
                analytics::rank_concentration(&mut rows, by, ascending);
                total = rows.len();
                if let Some(n) = limit {
                    rows.truncate(n);
                }
            }
        }
        Ok((
            ConcentrationReport {
                as_of: date,
                sort: by,
                ascending,
                total_matches: total,
                controller_reported: reported,
                rows,
            },
            warnings,
        ))
    }

    /// `ownership graph`: the holder-issuer network around `root` (a ticker,
    /// or a holder's key or name; `force_holder` skips the ticker reading).
    /// Returns the report and its warnings (which holder an ambiguous name
    /// picked, a truncated walk).
    pub fn graph(
        &self,
        root: &str,
        force_holder: bool,
        depth: usize,
        min_pct: f64,
        max_nodes: usize,
        as_of: Option<NaiveDate>,
    ) -> Result<(GraphReport, Vec<String>)> {
        if !(1..=MAX_DEPTH).contains(&depth) {
            return Err(IdxError::Usage(format!("--depth must be 1 to {MAX_DEPTH}")));
        }
        if !(1..=MAX_NODES).contains(&max_nodes) {
            return Err(IdxError::Usage(format!(
                "--max-nodes must be 1 to {MAX_NODES}"
            )));
        }
        if !min_pct.is_finite() || !(0.0..=100.0).contains(&min_pct) {
            return Err(IdxError::Usage("--min-pct must be 0 to 100".into()));
        }
        if root.trim().is_empty() {
            return Err(IdxError::Usage("graph needs a ticker or a holder".into()));
        }
        let (release_id, date) = self.release_for(as_of)?;
        let stakes = self.stakes(release_id)?;
        let mut warnings = Vec::new();
        let start = self.graph_root(root, force_holder, &stakes, date, &mut warnings)?;
        let w = analytics::walk(&stakes, &start, depth, min_pct, max_nodes);
        if w.truncated {
            warnings.push(format!(
                "stopped at {max_nodes} nodes; raise --max-nodes or --min-pct"
            ));
        }
        Ok((
            GraphReport {
                as_of: date,
                root: start.id(),
                depth,
                min_pct,
                max_nodes,
                truncated: w.truncated,
                issuers: w.issuers,
                holders: w.holders,
                links: w.links,
            },
            warnings,
        ))
    }

    fn graph_root(
        &self,
        root: &str,
        force_holder: bool,
        stakes: &[Stake],
        date: NaiveDate,
        warnings: &mut Vec<String>,
    ) -> Result<Root> {
        if !force_holder {
            if let Ok(sym) = Symbol::parse(root) {
                if stakes.iter().any(|s| s.symbol == sym.as_str()) {
                    return Ok(Root::Issuer(sym.as_str().to_string()));
                }
            }
        }
        let key = normalize::entity_key(root);
        if stakes.iter().any(|s| s.entity_key == key) {
            return Ok(Root::Holder(key));
        }
        let not_found = || {
            IdxError::NotFound(format!(
                "no ticker or holder matching {:?} as of {date}",
                root.trim()
            ))
        };
        if analytics::search_key(root).is_err() {
            return Err(not_found());
        }
        let present: std::collections::HashSet<&str> =
            stakes.iter().map(|s| s.entity_key.as_str()).collect();
        let known: Vec<Known> = self
            .known_holders()?
            .into_iter()
            .filter(|k| present.contains(k.entity_key.as_str()))
            .collect();
        let found = analytics::search(&known, stakes, root)?;
        let first = found.first().ok_or_else(not_found)?;
        if found.len() > 1 {
            warnings.push(format!(
                "graph starts at {}; {} other holder{} {:?} (see idx ownership search)",
                first.holder,
                found.len() - 1,
                if found.len() == 2 {
                    " matches"
                } else {
                    "s match"
                },
                root.trim()
            ));
        }
        Ok(Root::Holder(first.entity_key.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{HolderOrigin, SearchMatchKind};
    use crate::ownership::ingest::parse_xlsx;

    const AUG: &[u8] =
        include_bytes!("../../../tests/fixtures/ownership/above1_20260831_analytics_excerpt.xlsx");
    const SEP: &[u8] =
        include_bytes!("../../../tests/fixtures/ownership/above1_20260930_analytics_excerpt.xlsx");
    const OCT: &[u8] =
        include_bytes!("../../../tests/fixtures/ownership/above1_20261030_excerpt.xlsx");
    const KREN_SEP: &[u8] =
        include_bytes!("../../../tests/fixtures/ownership/above1_20260930_accounts_excerpt.xlsx");

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn excerpts() -> OwnershipDb {
        let mut db = OwnershipDb::in_memory().unwrap();
        db.import(&parse_xlsx(AUG).unwrap(), "xlsx", "aug.xlsx", "a")
            .unwrap();
        db.import(&parse_xlsx(SEP).unwrap(), "xlsx", "sep.xlsx", "s")
            .unwrap();
        db
    }

    fn min2() -> CrossFilter {
        CrossFilter {
            min_issuers: 2,
            ..Default::default()
        }
    }

    #[test]
    fn every_read_needs_an_imported_release() {
        let db = OwnershipDb::in_memory().unwrap();
        let none = |e: IdxError| assert_eq!(e.exit_code(), 4, "{e}");
        none(db.search("salim", None, None).unwrap_err());
        none(db.cross_holders(&min2(), None, None).unwrap_err());
        none(
            db.concentration(None, ConcentrationSort::Hhi, false, None, None)
                .unwrap_err(),
        );
        none(db.graph("BBCA", false, 2, 0.0, 100, None).unwrap_err());
        // A query that could never match is a usage error first.
        assert_eq!(db.search("x", None, None).unwrap_err().exit_code(), 2);
    }

    #[test]
    fn search_ranks_spellings_words_and_presence() {
        let db = excerpts();
        let (r, w) = db.search("permadi", None, None).unwrap();
        assert!(w.is_empty());
        assert_eq!(r.as_of, date(2026, 9, 30));
        let rows: Vec<(&str, SearchMatchKind, usize, NaiveDate)> = r
            .matches
            .iter()
            .map(|m| (m.entity_key.as_str(), m.matched, m.issuers, m.last_seen))
            .collect();
        assert_eq!(
            rows,
            [
                (
                    "T PERMADI RACHMAT",
                    SearchMatchKind::Contains,
                    3,
                    date(2026, 9, 30)
                ),
                (
                    "IR T PERMADI RACHMAT",
                    SearchMatchKind::Contains,
                    0,
                    date(2026, 8, 31)
                ),
            ]
        );
        assert_eq!(r.matches[0].total_pct, 13.38);
        assert_eq!(r.matches[0].investor_type.as_deref(), Some("INDIVIDUAL"));
        assert_eq!(r.matches[1].spellings, ["IR. T. PERMADI RACHMAT"]);
        assert_eq!(r.matches[1].local_foreign, None);

        let first = |q: &str| {
            let (r, _) = db.search(q, None, None).unwrap();
            (r.matches[0].entity_key.clone(), r.matches[0].matched)
        };
        assert_eq!(
            first("salim"),
            ("ANTHONI SALIM".into(), SearchMatchKind::Contains)
        );
        assert_eq!(
            first("salim anthoni"),
            ("ANTHONI SALIM".into(), SearchMatchKind::Words)
        );
        assert_eq!(
            first("Anthoni  Salim"),
            ("ANTHONI SALIM".into(), SearchMatchKind::Exact)
        );
        let (uob, _) = db.search("uob kay hian", None, None).unwrap();
        let keys: Vec<(&str, usize)> = uob
            .matches
            .iter()
            .map(|m| (m.entity_key.as_str(), m.issuers))
            .collect();
        assert_eq!(
            keys,
            [
                ("UOB KAY HIAN PRIVATE LIMITED", 3),
                ("UOB KAY HIAN PTE LTD", 0)
            ]
        );
        // Counted in the August release instead.
        let (aug, _) = db
            .search("uob kay hian", Some(date(2026, 8, 31)), Some(1))
            .unwrap();
        assert_eq!(aug.total_matches, 2);
        assert_eq!(aug.matches.len(), 1);
        assert_eq!(aug.matches[0].entity_key, "UOB KAY HIAN PTE LTD");

        let (none, w) = db.search("nobody at all", None, None).unwrap();
        assert!(none.matches.is_empty());
        assert_eq!(w, ["no holder matching \"nobody at all\""]);
        assert_eq!(db.search("x", None, None).unwrap_err().exit_code(), 2);
    }

    #[test]
    fn entity_exact_matches_one_key() {
        let db = excerpts();
        let aug = Some(date(2026, 8, 31));
        let loose = db.entity("T PERMADI RACHMAT", aug).unwrap();
        assert_eq!(loose.entities, ["IR. T. PERMADI RACHMAT"]);
        assert_eq!(loose.holdings.len(), 3);
        let err = db
            .entity_matching("T PERMADI RACHMAT", aug, true)
            .unwrap_err();
        assert_eq!(err.exit_code(), 4);
        let exact = db
            .entity_matching("ir t permadi rachmat", aug, true)
            .unwrap();
        assert_eq!(exact.holdings.len(), 3);
        assert_eq!(
            db.entity_matching("%%%", aug, true)
                .unwrap_err()
                .exit_code(),
            2
        );
    }

    #[test]
    fn cross_holders_on_the_excerpts() {
        let db = excerpts();
        let all = |f: CrossFilter| db.cross_holders(&f, None, None).unwrap();
        let r = all(min2());
        assert_eq!(r.total_matches, 8);
        let top: Vec<(&str, usize, f64)> = r
            .holders
            .iter()
            .take(3)
            .map(|h| (h.entity_key.as_str(), h.issuers, h.total_pct))
            .collect();
        assert_eq!(
            top,
            [
                ("DRS SURONO SUBEKTI", 3, 21.32),
                ("T PERMADI RACHMAT", 3, 13.38),
                ("UOB KAY HIAN PRIVATE LIMITED", 3, 8.77),
            ]
        );
        let surono = &r.holders[0];
        assert_eq!(surono.max_pct, 14.57);
        let pos: Vec<(&str, f64)> = surono
            .positions
            .iter()
            .map(|p| (p.symbol.as_str(), p.pct))
            .collect();
        assert_eq!(pos, [("OBAT", 14.57), ("CASS", 5.71), ("ADES", 1.04)]);
        assert_eq!(
            (
                r.holders[2].investor_type.as_deref(),
                r.holders[2].local_foreign.as_deref()
            ),
            (Some("SECURITIES COMPANY"), Some("A"))
        );

        let people = all(CrossFilter {
            min_issuers: 2,
            investor_type: Some("  Individual ".into()),
            origin: None,
        });
        let keys: Vec<&str> = people
            .holders
            .iter()
            .map(|h| h.entity_key.as_str())
            .collect();
        assert_eq!(
            keys,
            ["DRS SURONO SUBEKTI", "T PERMADI RACHMAT", "LONG CAI"]
        );
        assert_eq!(people.investor_type.as_deref(), Some("Individual"));
        assert_eq!(people.holders[2].total_pct, 2.49);
        let foreign = all(CrossFilter {
            min_issuers: 2,
            investor_type: None,
            origin: Some(HolderOrigin::Foreign),
        });
        let keys: Vec<&str> = foreign
            .holders
            .iter()
            .map(|h| h.entity_key.as_str())
            .collect();
        assert_eq!(keys, ["UOB KAY HIAN PRIVATE LIMITED", "LONG CAI"]);
        let three = all(CrossFilter {
            min_issuers: 3,
            ..Default::default()
        });
        assert_eq!(three.total_matches, 3);
        let page = db.cross_holders(&min2(), Some(2), None).unwrap();
        assert_eq!((page.total_matches, page.holders.len()), (8, 2));
        assert_eq!(
            db.cross_holders(&CrossFilter::default(), None, None)
                .unwrap_err()
                .exit_code(),
            2,
            "--min-issuers 0"
        );
    }

    #[test]
    fn concentration_on_the_excerpts() {
        let db = excerpts();
        let (r, w) = db
            .concentration(None, ConcentrationSort::Hhi, false, None, None)
            .unwrap();
        assert!(w.is_empty());
        assert_eq!(r.total_matches, 16);
        assert!(!r.controller_reported);
        let ades = &r.rows[0];
        assert_eq!(
            (ades.symbol.as_str(), ades.hhi, ades.top1_pct, ades.top3_pct),
            ("ADES", 8345.9041, 91.35, 92.39)
        );
        assert_eq!((ades.undisclosed_pct, ades.holders_listed), (7.61, 2));
        assert_eq!(ades.top_holder, "WATER PARTNERS BOTTLING SA");
        assert_eq!(r.rows[15].symbol, "BOSS");
        assert_eq!(r.rows[15].hhi, 339.9018);
        let bbca = r.rows.iter().find(|c| c.symbol == "BBCA").unwrap();
        assert_eq!(
            (bbca.hhi, bbca.top1_pct, bbca.top3_pct, bbca.listed_pct),
            (3021.9114, 54.94, 57.16, 58.18)
        );
        assert_eq!((bbca.undisclosed_pct, bbca.holders_listed), (41.82, 4));
        assert_eq!(bbca.top_holder, "PT DWIMURIA INVESTAMA ANDALAN");
        assert_eq!(bbca.controller_pct, None);

        let (asc, _) = db
            .concentration(None, ConcentrationSort::Top1, true, Some(3), None)
            .unwrap();
        assert_eq!((asc.total_matches, asc.rows.len()), (16, 3));
        assert_eq!(asc.rows[0].symbol, "BOSS");
        assert!(asc.rows[0].top1_pct <= asc.rows[1].top1_pct);

        let wanted = Symbol::parse_list(["TCID", "ZZZZ", "bbca"]).unwrap();
        let (some, w) = db
            .concentration(Some(&wanted), ConcentrationSort::Hhi, false, Some(1), None)
            .unwrap();
        let syms: Vec<&str> = some.rows.iter().map(|c| c.symbol.as_str()).collect();
        assert_eq!(syms, ["TCID", "BBCA"], "requested order, limit ignored");
        assert_eq!(some.total_matches, 2);
        assert_eq!(
            w,
            ["no holders above 1% recorded for ZZZZ as of 2026-09-30"]
        );
        let unknown = Symbol::parse_list(["ZZZZ"]).unwrap();
        assert_eq!(
            db.concentration(Some(&unknown), ConcentrationSort::Hhi, false, None, None)
                .unwrap_err()
                .exit_code(),
            4
        );
    }

    #[test]
    fn concentration_counts_a_holders_accounts_once_and_reads_the_controller_flag() {
        let mut db = OwnershipDb::in_memory().unwrap();
        db.import(&parse_xlsx(KREN_SEP).unwrap(), "xlsx", "sep.xlsx", "k")
            .unwrap();
        let kren = Symbol::parse_list(["KREN"]).unwrap();
        let (r, _) = db
            .concentration(Some(&kren), ConcentrationSort::Hhi, false, None, None)
            .unwrap();
        assert_eq!(
            r.rows[0].holders_listed, 12,
            "KRESNA's two lines are one holder"
        );
        assert_eq!(r.rows[0].listed_pct, 54.41);

        db.import(&parse_xlsx(OCT).unwrap(), "xlsx", "oct.xlsx", "o")
            .unwrap();
        let bbca = Symbol::parse_list(["BBCA"]).unwrap();
        let (r, _) = db
            .concentration(Some(&bbca), ConcentrationSort::Hhi, false, None, None)
            .unwrap();
        assert!(r.controller_reported);
        let holders = db.holders("BBCA", None).unwrap();
        let flagged: f64 = holders
            .holders
            .iter()
            .filter(|h| h.controller_or_affiliate.is_some())
            .map(|h| h.pct)
            .sum();
        assert!(flagged > 50.0);
        assert_eq!(r.rows[0].controller_pct, Some(analytics::round4(flagged)));
        assert!(r.rows[0].controller_pct.unwrap() < r.rows[0].listed_pct);
    }

    #[test]
    fn graph_on_the_excerpts() {
        let db = excerpts();
        let size = |root: &str, depth: usize, min: f64, max: usize| {
            let (g, w) = db.graph(root, false, depth, min, max, None).unwrap();
            (
                g.issuers.len() + g.holders.len(),
                g.links.len(),
                g.truncated,
                w.len(),
            )
        };
        assert_eq!(size("AADI", 2, 0.0, 100), (13, 13, false, 0));
        assert_eq!(size("aadi", 3, 0.0, 100), (25, 27, false, 0));
        assert_eq!(size("AADI", 2, 5.0, 100), (4, 3, false, 0));
        assert_eq!(size("BBCA", 1, 0.0, 100), (5, 4, false, 0));
        let (g, _) = db
            .graph("T PERMADI RACHMAT", false, 2, 0.0, 100, None)
            .unwrap();
        assert_eq!(g.root, "holder:T PERMADI RACHMAT");
        assert_eq!(
            (g.holders.len(), g.issuers.len(), g.links.len()),
            (22, 3, 27)
        );

        let (cap, w) = db.graph("AADI", false, 2, 0.0, 5, None).unwrap();
        assert!(cap.truncated);
        assert_eq!(w, ["stopped at 5 nodes; raise --max-nodes or --min-pct"]);
        let keys: Vec<&str> = cap.holders.iter().map(|h| h.entity_key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "ADARO STRATEGIC INVESTMENTS",
                "ALAMTRI RESOURCES INDONESIA",
                "GARIBALDI THOHIR",
                "SARATOGA INVESTAMA SEDAYA"
            ]
        );

        let (amb, w) = db.graph("thohir", false, 1, 0.0, 100, None).unwrap();
        assert_eq!(amb.root, "holder:GARIBALDI THOHIR");
        assert_eq!(w.len(), 1);
        assert!(
            w[0].starts_with("graph starts at GARIBALDI THOHIR; 1 other holder matches"),
            "{w:?}"
        );

        assert_eq!(
            db.graph("ZZZZ", false, 2, 0.0, 100, None)
                .unwrap_err()
                .exit_code(),
            4
        );
        for (depth, min, max) in [
            (0, 0.0, 100),
            (7, 0.0, 100),
            (2, -1.0, 100),
            (2, 101.0, 100),
            (2, f64::NAN, 100),
            (2, 0.0, 0),
            (2, 0.0, 1001),
        ] {
            assert_eq!(
                db.graph("AADI", false, depth, min, max, None)
                    .unwrap_err()
                    .exit_code(),
                2,
                "{depth} {min} {max}"
            );
        }
    }
}
