//! Plain-text renderers for `ownership sync`, `ownership status` and the
//! hidden `ownership build-snapshot`.

use chrono::NaiveDate;

use crate::model::{
    SnapshotManifest, SnapshotSummary, StatusReport, SyncAction, SyncNeeded, SyncReport,
};

fn size(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1e6)
    } else {
        format!("{:.0} kB", (bytes as f64 / 1e3).max(1.0))
    }
}

fn months(n: u64) -> String {
    format!("{n} month{}", if n == 1 { "" } else { "s" })
}

fn dates(d: &[NaiveDate]) -> String {
    d.iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn snapshot(s: &SnapshotSummary) -> String {
    format!(
        "the snapshot of {} ({}, {} to {}, {})",
        s.generated_at.format("%Y-%m-%d %H:%M UTC"),
        months(s.releases),
        s.oldest,
        s.newest,
        size(s.bytes)
    )
}

/// `ownership sync`: one or two sentences.
pub fn sync(r: &SyncReport) -> String {
    let mut out = String::new();
    let snap = r.snapshot.as_ref().map(snapshot).unwrap_or_default();
    match r.action {
        SyncAction::Installed => {
            out.push_str(&format!("Installed {snap} into {}.\n", r.database));
        }
        SyncAction::Updated => {
            out.push_str(&format!("Replaced {} with {snap}", r.database));
            match &r.backup {
                Some(b) => out.push_str(&format!("; the previous database is at {b}.\n")),
                None => out.push_str(".\n"),
            }
        }
        SyncAction::Imported => out.push_str(&format!(
            "Imported {} from the mirror's monthly files into {}.\n",
            dates(&r.imported),
            r.database
        )),
        SyncAction::UpToDate => {
            out.push_str(&format!("{} is up to date.\n", r.database));
        }
    }
    if matches!(r.action, SyncAction::Installed | SyncAction::Updated) && !r.imported.is_empty() {
        out.push_str(&format!(
            "Imported {} from the mirror's monthly files (newer than the snapshot).\n",
            dates(&r.imported)
        ));
    }
    if !r.kept.is_empty() {
        out.push_str(&format!(
            "Kept {} (only in the local database); --force replaces them.\n",
            dates(&r.kept)
        ));
    }
    if !r.dropped.is_empty() {
        out.push_str(&format!(
            "Dropped {} (still in the backup).\n",
            dates(&r.dropped)
        ));
    }
    match r.newest {
        Some(n) => out.push_str(&format!("{}, newest {n}.\n", months(r.releases))),
        None => out.push_str("No months imported.\n"),
    }
    out
}

/// `ownership status`: five labelled lines.
pub fn status(r: &StatusReport) -> String {
    let mut rows: Vec<(&str, String)> = Vec::new();
    rows.push((
        "Database",
        if r.exists {
            format!(
                "{} ({}, schema {})",
                r.database,
                size(r.bytes.unwrap_or(0)),
                r.db_schema_version
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "?".into())
            )
        } else {
            format!("{} (not created yet)", r.database)
        },
    ));
    rows.push((
        "Local",
        match (r.oldest, r.newest) {
            (Some(o), Some(n)) => format!("{}, {o} to {n}", months(r.releases)),
            _ => "no months imported".into(),
        },
    ));
    rows.push((
        "Snapshot",
        match &r.installed_snapshot {
            Some(s) => format!(
                "installed from the snapshot of {} ({}{})",
                s.generated_at
                    .as_deref()
                    .map(|g| match g.parse::<chrono::DateTime<chrono::Utc>>() {
                        Ok(t) => t.format("%Y-%m-%d %H:%M UTC").to_string(),
                        Err(_) => g.to_string(),
                    })
                    .unwrap_or_else(|| "?".into()),
                months(s.releases),
                s.newest
                    .map(|n| format!(", newest {n}"))
                    .unwrap_or_default()
            ),
            None if r.exists => "none (built by local imports)".into(),
            None => "none".into(),
        },
    ));
    rows.push((
        "Mirror",
        match &r.remote {
            None => "not checked".into(),
            Some(m) => {
                let index = match (m.releases, m.newest) {
                    (Some(n), Some(newest)) => format!(
                        "{}, newest {newest}{}",
                        months(n),
                        m.index_generated_at
                            .as_deref()
                            .map(|g| format!(" (index of {g})"))
                            .unwrap_or_default()
                    ),
                    _ => "index.json unreadable".into(),
                };
                let snap = match &m.snapshot {
                    Some(s) => format!(
                        "snapshot of {} ({}, newest {}, {})",
                        s.generated_at.format("%Y-%m-%d %H:%M UTC"),
                        months(s.releases),
                        s.newest,
                        size(s.bytes)
                    ),
                    None => format!("no snapshot for schema {}", r.supported_schema_version),
                };
                format!("{index}; {snap}")
            }
        },
    ));
    let missing = if r.missing.is_empty() {
        String::new()
    } else {
        format!(" (missing {})", dates(&r.missing))
    };
    rows.push((
        "Sync",
        match r.sync {
            None => "unknown (see warnings)".into(),
            Some(SyncNeeded::UpToDate) => "up to date".into(),
            Some(SyncNeeded::Install) => {
                format!("`idx ownership sync` would install the snapshot{missing}")
            }
            Some(SyncNeeded::Update) => format!(
                "`idx ownership sync` would replace the database with the snapshot{missing}"
            ),
            Some(SyncNeeded::Import) => {
                format!("`idx ownership sync` would import the monthly files{missing}")
            }
        },
    ));
    if !r.local_only.is_empty() {
        rows.push(("Local only", dates(&r.local_only)));
    }
    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    rows.iter()
        .map(|(k, v)| format!("{k:width$}  {v}\n"))
        .collect()
}

/// `ownership build-snapshot`.
pub fn built(m: &SnapshotManifest) -> String {
    let newest = m.releases.iter().map(|r| r.as_of).max();
    let oldest = m.releases.iter().map(|r| r.as_of).min();
    format!(
        "Built {} ({}, {} SQLite; {}, {} to {}) and {}.\n",
        m.file,
        size(m.bytes),
        size(m.sqlite_bytes),
        months(m.releases.len() as u64),
        oldest.map(|d| d.to_string()).unwrap_or_default(),
        newest.map(|d| d.to_string()).unwrap_or_default(),
        crate::ownership::snapshot::manifest_name(m.db_schema_version)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MirrorState;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn summary() -> SnapshotSummary {
        SnapshotSummary {
            file: "ownership-v2-2026-09-30-0123456789ab.sqlite.gz".into(),
            generated_at: "2026-11-06T01:17:40Z".parse().unwrap(),
            db_schema_version: 2,
            oldest: d("2026-02-27"),
            newest: d("2026-09-30"),
            releases: 8,
            bytes: 6_890_866,
        }
    }

    #[test]
    fn sync_sentences() {
        let mut r = SyncReport {
            action: SyncAction::Updated,
            database: "/data/o.sqlite".into(),
            snapshot: Some(summary()),
            installed: vec![d("2026-09-30")],
            imported: vec![d("2026-10-30")],
            kept: vec![],
            dropped: vec![d("2026-01-30")],
            backup: Some("/data/o.sqlite.bak".into()),
            releases: 9,
            newest: Some(d("2026-10-30")),
        };
        let t = sync(&r);
        assert!(t.starts_with("Replaced /data/o.sqlite with the snapshot of 2026-11-06 01:17 UTC (8 months, 2026-02-27 to 2026-09-30, 6.9 MB); the previous database is at /data/o.sqlite.bak."), "{t}");
        assert!(t.contains(
            "Imported 2026-10-30 from the mirror's monthly files (newer than the snapshot)."
        ));
        assert!(t.contains("Dropped 2026-01-30"));
        assert!(t.ends_with("9 months, newest 2026-10-30.\n"));
        r.action = SyncAction::UpToDate;
        r.imported.clear();
        r.dropped.clear();
        r.kept = vec![d("2026-10-30")];
        let t = sync(&r);
        assert!(t.starts_with("/data/o.sqlite is up to date."), "{t}");
        assert!(t.contains("Kept 2026-10-30"));
    }

    #[test]
    fn status_lines() {
        let mut r = StatusReport {
            database: "/data/o.sqlite".into(),
            exists: false,
            bytes: None,
            db_schema_version: None,
            supported_schema_version: 2,
            releases: 0,
            oldest: None,
            newest: None,
            installed_snapshot: None,
            remote: None,
            missing: vec![],
            local_only: vec![],
            sync: None,
        };
        let t = status(&r);
        assert!(
            t.contains("Database  /data/o.sqlite (not created yet)"),
            "{t}"
        );
        assert!(t.contains("Mirror    not checked"));
        r.remote = Some(MirrorState {
            mirror: "https://m".into(),
            index_generated_at: Some("2026-10-17T08:49:52Z".into()),
            releases: Some(8),
            newest: Some(d("2026-09-30")),
            snapshot: Some(summary()),
        });
        r.sync = Some(SyncNeeded::Install);
        r.missing = vec![d("2026-09-30")];
        let t = status(&r);
        assert!(t.contains("8 months, newest 2026-09-30 (index of 2026-10-17T08:49:52Z); snapshot of 2026-11-06 01:17 UTC (8 months, newest 2026-09-30, 6.9 MB)"), "{t}");
        assert!(t.contains("would install the snapshot (missing 2026-09-30)"));
    }
}
