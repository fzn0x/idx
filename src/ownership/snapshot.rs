//! The prebuilt SQLite snapshot of the register, published on the mirror.
//!
//! CI (`scripts/ownership_mirror.py`) imports every mirrored month into a
//! fresh database and runs the hidden `idx ownership build-snapshot`
//! ([`build`]), which writes two assets for the `ownership-data` release:
//!
//! - `ownership-v{V}-{newest as_of}-{sha12}.sqlite.gz`: the database,
//!   vacuumed and gzip-compressed, named after its own SHA-256 so it is never
//!   overwritten;
//! - `snapshot-v{V}.json` (`idx.ownership.snapshot.v1`): sizes and SHA-256 of
//!   both forms, a content digest, the rows per table and every release
//!   inside. CI uploads it last, after checking the uploaded file.
//!
//! `V` is the database schema version (`PRAGMA user_version`). An idx reads
//! only the manifest of its own version; older manifests stay on the release,
//! so an older idx keeps installing a valid (older) snapshot and imports the
//! newer months from the raw files.
//!
//! `idx ownership sync` ([`sync`]) installs the snapshot when the local
//! database is empty, older or a subset of it, and otherwise imports only the
//! missing months from the raw files. It never drops data only the local
//! database has without `force` ([`plan`] decides). The swap goes through
//! SQLite's online backup API: one write transaction into the live file, so
//! a reader (an `idx mcp` call) keeps its view until it finishes and nothing
//! is renamed under it. Syncs of one database run one at a time
//! ([`lock_path`]). `idx ownership status` ([`status`]) reports both sides
//! without writing anything.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::{DateTime, NaiveDate, SubsecRound, Utc};
use rusqlite::backup::{Backup, StepResult};
use rusqlite::types::ValueRef;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::error::{IdxError, Result};
use crate::model::{
    InstalledSnapshot, MirrorState, SnapshotManifest, SnapshotRelease, SnapshotSummary,
    StatusReport, SyncAction, SyncNeeded, SyncReport,
};
use crate::providers::Sourced;

use super::mirror::{self, MirrorIndex};
use super::{sha256_hex, OwnershipDb, SCHEMA_VERSION};

/// `schema` of the manifest.
pub const MANIFEST_SCHEMA: &str = "idx.ownership.snapshot.v1";
/// Largest manifest a client reads.
pub const MAX_MANIFEST_BYTES: u64 = 1 << 20;
/// Largest `.sqlite.gz` a client downloads.
pub const MAX_BYTES: u64 = 256 << 20;
/// Largest decompressed SQLite file a client writes.
pub const MAX_SQLITE_BYTES: u64 = 1 << 30;
/// Page size of every snapshot: the backup API cannot change the page size
/// of a database in WAL mode, and idx creates its files with this one.
pub const PAGE_SIZE: i64 = 4096;
/// Shortest network timeout `sync` and `status` use, whatever `--timeout` says.
pub const MIN_TIMEOUT: Duration = Duration::from_secs(120);
/// Table the builder adds to the snapshot; `OwnershipDb` never reads it.
const INFO_TABLE: &str = "snapshot_info";
/// Its definition, the one schema object a snapshot has beyond this idx's.
const INFO_DDL: &str = "CREATE TABLE snapshot_info (key TEXT PRIMARY KEY, value TEXT NOT NULL)";
const SNAPSHOT_PROVIDER: &str = "github-mirror";

/// `snapshot-v{V}.json`.
pub fn manifest_name(v: i64) -> String {
    format!("snapshot-v{v}.json")
}

/// `ownership-v{V}-{newest}-{first 12 hex of sha256}.sqlite.gz`.
pub fn file_name(v: i64, newest: NaiveDate, sha256: &str) -> String {
    format!(
        "ownership-v{v}-{newest}-{}.sqlite.gz",
        &sha256[..12.min(sha256.len())]
    )
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn bad_manifest(msg: impl std::fmt::Display) -> IdxError {
    IdxError::provider("mirror", format!("invalid snapshot manifest: {msg}"))
}

/// Parses `snapshot-v{v}.json` and checks every field a client relies on:
/// schema, version, file name, sizes, hashes and the release list.
pub fn parse_manifest(bytes: &[u8], v: i64) -> Result<SnapshotManifest> {
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(bad_manifest(format!(
            "{} bytes, more than {MAX_MANIFEST_BYTES}",
            bytes.len()
        )));
    }
    let m: SnapshotManifest = serde_json::from_slice(bytes).map_err(bad_manifest)?;
    if m.schema != MANIFEST_SCHEMA {
        return Err(IdxError::provider(
            "mirror",
            format!("unsupported snapshot schema {:?}; upgrade idx", m.schema),
        ));
    }
    if m.db_schema_version != v {
        return Err(bad_manifest(format!(
            "{} says db_schema_version {}",
            manifest_name(v),
            m.db_schema_version
        )));
    }
    if m.compression != "gzip" {
        return Err(bad_manifest(format!(
            "compression {:?} (only gzip)",
            m.compression
        )));
    }
    if m.bytes == 0 || m.bytes > MAX_BYTES {
        return Err(bad_manifest(format!(
            "bytes {} outside 1..={MAX_BYTES}",
            m.bytes
        )));
    }
    if m.sqlite_bytes == 0 || m.sqlite_bytes > MAX_SQLITE_BYTES {
        return Err(bad_manifest(format!(
            "sqlite_bytes {} outside 1..={MAX_SQLITE_BYTES}",
            m.sqlite_bytes
        )));
    }
    for (name, value) in [
        ("sha256", &m.sha256),
        ("sqlite_sha256", &m.sqlite_sha256),
        ("content_sha256", &m.content_sha256),
    ] {
        if !is_hex64(value) {
            return Err(bad_manifest(format!(
                "{name} is not 64 lower-case hex digits"
            )));
        }
    }
    if m.releases.is_empty() {
        return Err(bad_manifest("no releases"));
    }
    let mut dates = BTreeSet::new();
    for r in &m.releases {
        if !dates.insert(r.as_of) {
            return Err(bad_manifest(format!("release {} listed twice", r.as_of)));
        }
        if !is_hex64(&r.sha256) {
            return Err(bad_manifest(format!(
                "release {}: sha256 is not 64 lower-case hex digits",
                r.as_of
            )));
        }
    }
    let newest = *dates.last().expect("not empty");
    let expected = file_name(v, newest, &m.sha256);
    if m.file != expected {
        // Also refuses any path: the file must be a bare asset name.
        return Err(bad_manifest(format!(
            "file {:?}, expected {expected:?}",
            m.file
        )));
    }
    Ok(m)
}

/// The short form reports carry.
pub fn summary(m: &SnapshotManifest) -> SnapshotSummary {
    let oldest = m.releases.iter().map(|r| r.as_of).min();
    let newest = m.releases.iter().map(|r| r.as_of).max();
    SnapshotSummary {
        file: m.file.clone(),
        generated_at: m.generated_at,
        db_schema_version: m.db_schema_version,
        oldest: oldest.unwrap_or_default(),
        newest: newest.unwrap_or_default(),
        releases: m.releases.len() as u64,
        bytes: m.bytes,
    }
}

fn db_err(path: &Path) -> impl Fn(rusqlite::Error) -> IdxError + '_ {
    move |e| IdxError::Other(format!("ownership database {}: {e}", path.display()))
}

fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

/// Tables that hold data, sorted: everything but SQLite's own and
/// `snapshot_info`.
fn user_tables(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut st = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type = 'table'
         AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' AND name <> ?1 ORDER BY name",
    )?;
    let names = st
        .query_map([INFO_TABLE], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(names)
}

fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n > 0)
}

fn table_counts(conn: &Connection) -> rusqlite::Result<BTreeMap<String, u64>> {
    let mut out = BTreeMap::new();
    for t in user_tables(conn)? {
        let n: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM {}", quote_ident(&t)),
            [],
            |r| r.get(0),
        )?;
        out.insert(t, n as u64);
    }
    Ok(out)
}

fn render(v: ValueRef<'_>, out: &mut String) {
    match v {
        ValueRef::Null => out.push('N'),
        ValueRef::Integer(i) => out.push_str(&format!("i{i}")),
        ValueRef::Real(f) => out.push_str(&format!("r{f:?}")),
        ValueRef::Text(t) => {
            out.push('t');
            for c in String::from_utf8_lossy(t).chars() {
                match c {
                    '\\' => out.push_str("\\\\"),
                    '|' => out.push_str("\\|"),
                    '\n' => out.push_str("\\n"),
                    '\r' => out.push_str("\\r"),
                    c => out.push(c),
                }
            }
        }
        ValueRef::Blob(b) => {
            out.push('b');
            out.push_str(&hex::encode(b));
        }
    }
}

/// SHA-256 over a canonical dump: the schema version, then for each table
/// (by name) its columns and every row ordered by all of them, values typed
/// and escaped. Import times (`imported_at`) are left out, so two builds of
/// the same files by the same idx give the same digest; it changes with the
/// data, the parser or the schema.
pub fn content_digest(conn: &Connection) -> rusqlite::Result<String> {
    let mut h = Sha256::new();
    let v: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    h.update(format!("db_schema_version={v}\n"));
    for table in user_tables(conn)? {
        let cols: Vec<String> = {
            let mut st = conn.prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")?;
            let c = st
                .query_map([&table], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            c.into_iter().filter(|c| c != "imported_at").collect()
        };
        h.update(format!("#{table}|{}\n", cols.join("|")));
        if cols.is_empty() {
            continue;
        }
        let sql = format!(
            "SELECT {} FROM {} ORDER BY {}",
            cols.iter()
                .map(|c| quote_ident(c))
                .collect::<Vec<_>>()
                .join(", "),
            quote_ident(&table),
            (1..=cols.len())
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let mut st = conn.prepare(&sql)?;
        let mut rows = st.query([])?;
        let mut line = String::new();
        while let Some(row) = rows.next()? {
            line.clear();
            line.push_str(&table);
            for i in 0..cols.len() {
                line.push('|');
                render(row.get_ref(i)?, &mut line);
            }
            line.push('\n');
            h.update(line.as_bytes());
        }
    }
    Ok(hex::encode(h.finalize()))
}

// ---- local state ----------------------------------------------------------

/// The `snapshot_info` table of a database installed from a snapshot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SnapshotInfo {
    pub generated_at: Option<String>,
    pub built_by: Option<String>,
    pub content_sha256: Option<String>,
    /// The releases the snapshot held: as_of → SHA-256 of the IDX file.
    pub releases: BTreeMap<NaiveDate, String>,
}

/// What [`inspect`] read from the local database, without creating it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocalState {
    pub exists: bool,
    pub bytes: Option<u64>,
    pub user_version: Option<i64>,
    /// as_of → SHA-256 of the file each release was imported from.
    pub months: BTreeMap<NaiveDate, String>,
    /// Rows per table.
    pub tables: BTreeMap<String, u64>,
    pub info: Option<SnapshotInfo>,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct InfoRelease {
    as_of: NaiveDate,
    sha256: String,
}

fn read_info(conn: &Connection) -> rusqlite::Result<Option<SnapshotInfo>> {
    if !table_exists(conn, INFO_TABLE)? {
        return Ok(None);
    }
    let get = |key: &str| -> rusqlite::Result<Option<String>> {
        conn.query_row(
            &format!("SELECT value FROM {INFO_TABLE} WHERE key = ?1"),
            [key],
            |r| r.get(0),
        )
        .optional()
    };
    let releases: Vec<InfoRelease> = get("releases")?
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    Ok(Some(SnapshotInfo {
        generated_at: get("generated_at")?,
        built_by: get("built_by")?,
        content_sha256: get("content_sha256")?,
        releases: releases
            .into_iter()
            .map(|r| (r.as_of, r.sha256.to_ascii_lowercase()))
            .collect(),
    }))
}

/// Reads the local database without creating or changing it: a missing
/// file is `exists: false`. A file SQLite cannot read is an error.
pub fn inspect(path: &Path) -> Result<LocalState> {
    let Ok(meta) = std::fs::metadata(path) else {
        return Ok(LocalState::default());
    };
    let err = db_err(path);
    // Read-write without CREATE: never creates the file, and works on a
    // WAL database whose -shm file is gone. Nothing is written.
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(&err)?;
    conn.execute_batch("PRAGMA query_only = ON;")
        .map_err(&err)?;
    let user_version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(&err)?;
    let tables = table_counts(&conn).map_err(&err)?;
    let mut months = BTreeMap::new();
    if tables.contains_key("releases") {
        let mut st = conn
            .prepare("SELECT as_of, sha256 FROM releases")
            .map_err(&err)?;
        let rows = st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(&err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(&err)?;
        for (d, sha) in rows {
            if let Ok(d) = NaiveDate::parse_from_str(&d, "%Y-%m-%d") {
                months.insert(d, sha.to_ascii_lowercase());
            }
        }
    }
    let info = read_info(&conn).map_err(&err)?;
    Ok(LocalState {
        exists: true,
        bytes: Some(meta.len()),
        user_version: Some(user_version),
        months,
        tables,
        info,
    })
}

// ---- the decision ---------------------------------------------------------

/// What the local database has that the snapshot would not bring back.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Divergence {
    /// Months only the local database has (and the mirror's monthly files
    /// do not list with the same SHA-256).
    pub local_only: Vec<NaiveDate>,
    /// Months re-imported locally, after a snapshot install, from another
    /// file than both the installed and the new snapshot.
    pub modified: Vec<NaiveDate>,
    /// Tables with rows locally that the snapshot has none of (data from
    /// other imports or hand-made rules), with their row counts.
    pub tables: Vec<(String, u64)>,
}

impl Divergence {
    pub fn is_empty(&self) -> bool {
        self.local_only.is_empty() && self.modified.is_empty() && self.tables.is_empty()
    }

    /// Local-only and modified months, sorted.
    pub fn months(&self) -> Vec<NaiveDate> {
        let mut v: Vec<NaiveDate> = self
            .local_only
            .iter()
            .chain(&self.modified)
            .copied()
            .collect();
        v.sort();
        v.dedup();
        v
    }

    fn describe(&self) -> String {
        let mut parts = Vec::new();
        if !self.local_only.is_empty() {
            parts.push(format!(
                "months the snapshot lacks ({})",
                join_dates(&self.local_only)
            ));
        }
        if !self.modified.is_empty() {
            parts.push(format!(
                "months re-imported from another file ({})",
                join_dates(&self.modified)
            ));
        }
        if !self.tables.is_empty() {
            let t: Vec<String> = self
                .tables
                .iter()
                .map(|(name, n)| format!("{name}: {n} rows"))
                .collect();
            parts.push(format!(
                "data the snapshot does not carry ({})",
                t.join(", ")
            ));
        }
        parts.join(", ")
    }
}

/// What a sync does, decided from the local state, the mirror's snapshot
/// and its monthly index (as_of → SHA-256). First match wins:
///
/// 1. the file is from a newer idx and `force` is off: refuse;
/// 2. the mirror has no snapshot for this schema: import the raw files;
/// 3. the database is missing or empty: install;
/// 4. it has months or data the snapshot lacks: keep it and import missing
///    months, or with `force` replace it (a `.bak` copy is kept);
/// 5. it was installed from this very snapshot and still holds its months:
///    up to date;
/// 6. otherwise (more months on the mirror, a re-published file, a new
///    parser or schema, a database built by local imports): replace it.
///
/// Months on neither the snapshot nor the index are local-only; a month the
/// index lists with the same SHA-256 is not (after the swap it is imported
/// again from the raw file). Files are compared by the SHA-256 of the IDX
/// file, never the SQLite files themselves (idx rewrites their header).
#[derive(Debug, Clone, PartialEq)]
pub enum Plan {
    NewerSchema(i64),
    NoSnapshot,
    Install,
    Keep(Divergence),
    Replace {
        /// What `force` drops (empty without force).
        divergence: Divergence,
        /// Months the local database has from another file than the snapshot.
        differed: Vec<NaiveDate>,
    },
    UpToDate,
}

pub fn plan(
    local: &LocalState,
    snapshot: Option<&SnapshotManifest>,
    index: &BTreeMap<NaiveDate, String>,
    force: bool,
) -> Plan {
    if let Some(v) = local.user_version.filter(|v| *v > SCHEMA_VERSION) {
        if !force {
            return Plan::NewerSchema(v);
        }
    }
    let Some(snap) = snapshot else {
        return Plan::NoSnapshot;
    };
    let s: BTreeMap<NaiveDate, &str> = snap
        .releases
        .iter()
        .map(|r| (r.as_of, r.sha256.as_str()))
        .collect();
    let empty_info = SnapshotInfo::default();
    let r = &local.info.as_ref().unwrap_or(&empty_info).releases;

    let mut div = Divergence::default();
    for (d, sha) in &local.months {
        match s.get(d) {
            None => {
                if index.get(d).map(String::as_str) != Some(sha.as_str()) {
                    div.local_only.push(*d);
                }
            }
            Some(snap_sha) => {
                if let Some(installed) = r.get(d) {
                    if installed != sha && *snap_sha != sha.as_str() {
                        div.modified.push(*d);
                    }
                }
            }
        }
    }
    for (t, n) in &local.tables {
        if *n > 0 && snap.tables.get(t).copied().unwrap_or(0) == 0 {
            div.tables.push((t.clone(), *n));
        }
    }

    if local.months.is_empty() && div.tables.is_empty() {
        return Plan::Install;
    }
    if !div.is_empty() {
        if !force {
            return Plan::Keep(div);
        }
        let differed = differed(local, &s, &div);
        return Plan::Replace {
            divergence: div,
            differed,
        };
    }
    let same_content = local
        .info
        .as_ref()
        .and_then(|i| i.content_sha256.as_deref())
        == Some(snap.content_sha256.as_str());
    let holds_all = s
        .iter()
        .all(|(d, sha)| local.months.get(d).map(String::as_str) == Some(*sha));
    if same_content && local.user_version == Some(SCHEMA_VERSION) && holds_all {
        return Plan::UpToDate;
    }
    Plan::Replace {
        differed: differed(local, &s, &div),
        divergence: div,
    }
}

/// Local months the snapshot also has, from another file (not counting
/// the ones `div` already names).
fn differed(local: &LocalState, s: &BTreeMap<NaiveDate, &str>, div: &Divergence) -> Vec<NaiveDate> {
    local
        .months
        .iter()
        .filter(|(d, sha)| {
            s.get(d).is_some_and(|snap| *snap != sha.as_str()) && !div.modified.contains(d)
        })
        .map(|(d, _)| *d)
        .collect()
}

fn join_dates(d: &[NaiveDate]) -> String {
    d.iter()
        .map(|d| d.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

// ---- mirror access --------------------------------------------------------

/// The mirror `sync` and `status` read, or why there is none: offline, or
/// `IDX_OWNERSHIP_MIRROR=off`.
pub fn mirror_base(offline: bool) -> std::result::Result<String, IdxError> {
    if offline {
        return Err(IdxError::OfflineMiss(
            "ownership sync needs the network (the GitHub mirror)".into(),
        ));
    }
    mirror::base().ok_or_else(|| {
        IdxError::Config(
            "IDX_OWNERSHIP_MIRROR=off: there is no mirror to sync from; use `idx ownership import --all`"
                .into(),
        )
    })
}

/// The mirror `status` reads, or `None` with the warning that says why not.
pub fn status_mirror(offline: bool) -> (Option<String>, Option<String>) {
    match mirror_base(offline) {
        Ok(base) => (Some(base), None),
        Err(IdxError::OfflineMiss(_)) => (
            None,
            Some("offline: the mirror was not checked".to_string()),
        ),
        Err(_) => (
            None,
            Some("IDX_OWNERSHIP_MIRROR=off: the mirror was not checked".to_string()),
        ),
    }
}

/// `{base}/snapshot-v{V}.json` for this idx's schema, `None` when the
/// mirror has none (404 or a missing file).
pub async fn fetch_manifest(base: &str, timeout: Duration) -> Result<Option<SnapshotManifest>> {
    let url = format!("{base}/{}", manifest_name(SCHEMA_VERSION));
    match mirror::get_limited(&url, timeout, Some(MAX_MANIFEST_BYTES)).await {
        Ok(bytes) => parse_manifest(&bytes, SCHEMA_VERSION).map(Some),
        Err(IdxError::NotFound(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Downloads the snapshot file the manifest names, from the same mirror,
/// and checks its size and SHA-256.
pub async fn download(base: &str, m: &SnapshotManifest, timeout: Duration) -> Result<Vec<u8>> {
    let url = format!("{base}/{}", m.file);
    let bytes = match mirror::get_limited(&url, timeout, Some(m.bytes)).await {
        Ok(b) => b,
        Err(IdxError::NotFound(_)) => {
            return Err(IdxError::provider(
                "mirror",
                format!(
                    "{} names {}, which is not on the mirror; try again later",
                    manifest_name(m.db_schema_version),
                    m.file
                ),
            ))
        }
        Err(e) => return Err(e),
    };
    if bytes.len() as u64 != m.bytes {
        return Err(IdxError::provider(
            "mirror",
            format!(
                "{}: {} bytes, the manifest says {}",
                m.file,
                bytes.len(),
                m.bytes
            ),
        ));
    }
    let actual = sha256_hex(&bytes);
    if actual != m.sha256 {
        return Err(IdxError::provider(
            "mirror",
            format!(
                "{}: SHA-256 {actual} does not match the manifest ({})",
                m.file, m.sha256
            ),
        ));
    }
    Ok(bytes)
}

// ---- install --------------------------------------------------------------

/// A file deleted (with any SQLite sidecars) when dropped.
struct TempFile(PathBuf);

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        for suffix in ["-journal", "-wal", "-shm"] {
            let _ = std::fs::remove_file(sidecar(&self.0, suffix));
        }
    }
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

/// `{dir}/.{stem}-{pid}-{n}.tmp`, unique per call: two installs in one
/// process (parallel calls of `idx mcp`) never share a file.
fn unique_tmp(dir: &Path, stem: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    dir.join(format!(".{stem}-{}-{n}.tmp", std::process::id()))
}

/// `<database>.bak`: the copy `sync` keeps of a database it replaces.
pub fn backup_path(db_path: &Path) -> PathBuf {
    sidecar(db_path, ".bak")
}

/// `<database>.sync.lock`: the empty SQLite file a sync holds an exclusive
/// lock on, so syncs from several idx processes run one at a time. It stays
/// in place; the operating system drops the lock when a process ends.
pub fn lock_path(db_path: &Path) -> PathBuf {
    sidecar(db_path, ".sync.lock")
}

/// Serializes syncs within this process (parallel `idx mcp` calls).
static SYNCING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Held for a whole sync. Fields drop in order: the file lock first, then
/// the process-wide mutex.
struct SyncLock {
    _file: Connection,
    _process: tokio::sync::MutexGuard<'static, ()>,
}

fn is_busy(e: &rusqlite::Error) -> bool {
    matches!(
        e.sqlite_error_code(),
        Some(rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked)
    )
}

/// Waits (up to `wait`) until no other sync of `db_path` runs, in this
/// process or another one.
async fn lock_sync(db_path: &Path, wait: Duration) -> Result<SyncLock> {
    let process = SYNCING.lock().await;
    let path = lock_path(db_path);
    if let Some(dir) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    let err = db_err(&path);
    let file = Connection::open(&path).map_err(&err)?;
    file.busy_timeout(Duration::ZERO).map_err(&err)?;
    let started = Instant::now();
    loop {
        match file.execute_batch("BEGIN EXCLUSIVE") {
            Ok(()) => {
                return Ok(SyncLock {
                    _file: file,
                    _process: process,
                })
            }
            Err(e) if is_busy(&e) && started.elapsed() < wait => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Err(e) if is_busy(&e) => {
                return Err(IdxError::Other(format!(
                    "another idx is syncing the ownership database {} ({} is locked); try again when it finishes",
                    db_path.display(),
                    path.display()
                )))
            }
            Err(e) => return Err(err(e)),
        }
    }
}

fn invalid(m: &SnapshotManifest, msg: impl std::fmt::Display) -> IdxError {
    IdxError::provider(
        "mirror",
        format!("snapshot {} failed validation: {msg}", m.file),
    )
}

fn decompress(gz: &[u8], dest: &Path, m: &SnapshotManifest) -> Result<()> {
    let mut reader = flate2::read::GzDecoder::new(gz).take(m.sqlite_bytes + 1);
    let mut out = std::fs::File::create(dest)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| invalid(m, format!("not a valid gzip stream ({e})")))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buf[..n]);
        out.write_all(&buf[..n])?;
    }
    out.sync_all()?;
    if total != m.sqlite_bytes {
        return Err(invalid(
            m,
            format!(
                "decompresses to {total} bytes{}, the manifest says {}",
                if total > m.sqlite_bytes {
                    " or more"
                } else {
                    ""
                },
                m.sqlite_bytes
            ),
        ));
    }
    let actual = hex::encode(hasher.finalize());
    if actual != m.sqlite_sha256 {
        return Err(invalid(
            m,
            format!(
                "SQLite SHA-256 {actual} does not match the manifest ({})",
                m.sqlite_sha256
            ),
        ));
    }
    Ok(())
}

/// One schema object: type, name, table, SQL.
type SchemaRow = (String, String, String, Option<String>);

fn schema_rows(conn: &Connection) -> rusqlite::Result<Vec<SchemaRow>> {
    let mut st =
        conn.prepare("SELECT type, name, tbl_name, sql FROM sqlite_master ORDER BY type, name")?;
    let rows = st
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// The schema a snapshot must have: what this idx creates plus
/// `snapshot_info`, nothing else.
fn expected_schema() -> Result<Vec<SchemaRow>> {
    let db = OwnershipDb::in_memory()?;
    let conn = db.connection();
    let err = |e: rusqlite::Error| IdxError::Other(format!("ownership schema: {e}"));
    conn.execute_batch(INFO_DDL).map_err(err)?;
    schema_rows(conn).map_err(err)
}

/// Checks a decompressed snapshot against its manifest, read-only: SQLite's
/// own integrity check, the schema version, page size and journal mode, the
/// schema itself (exactly this idx's tables and indexes: no views, no
/// triggers, nothing that would run in the user's database later), the
/// releases (date, format, file SHA-256, lines), the positions stored per
/// release, the rows per table and the content digest it recorded.
pub fn validate(path: &Path, m: &SnapshotManifest) -> Result<()> {
    let mut header = [0u8; 100];
    std::fs::File::open(path)?
        .read_exact(&mut header)
        .map_err(|e| invalid(m, format!("no SQLite header ({e})")))?;
    if &header[..16] != b"SQLite format 3\0" {
        return Err(invalid(m, "not an SQLite file"));
    }
    if header[18] != 1 || header[19] != 1 {
        return Err(invalid(m, "not in rollback-journal mode"));
    }
    let bad = |e: rusqlite::Error| invalid(m, e);
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(bad)?;
    conn.execute_batch("PRAGMA trusted_schema = OFF;")
        .map_err(bad)?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(bad)?;
    if check != "ok" {
        return Err(invalid(m, format!("quick_check: {check}")));
    }
    let v: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .map_err(bad)?;
    if v != m.db_schema_version {
        return Err(invalid(
            m,
            format!(
                "schema version {v}, the manifest says {}",
                m.db_schema_version
            ),
        ));
    }
    let page: i64 = conn
        .pragma_query_value(None, "page_size", |r| r.get(0))
        .map_err(bad)?;
    if page != PAGE_SIZE {
        return Err(invalid(
            m,
            format!("page size {page}, expected {PAGE_SIZE}"),
        ));
    }
    let schema = schema_rows(&conn).map_err(bad)?;
    if let Some((kind, name, ..)) = schema.iter().find(|(t, ..)| t == "trigger" || t == "view") {
        return Err(invalid(m, format!("it carries a {kind} ({name})")));
    }
    let expected = expected_schema()?;
    if schema != expected {
        let names = |rows: &[SchemaRow], other: &[SchemaRow]| -> Vec<String> {
            rows.iter()
                .filter(|r| !other.contains(r))
                .map(|(t, n, ..)| format!("{t} {n}"))
                .collect()
        };
        return Err(invalid(
            m,
            format!(
                "its schema differs from this idx's (unexpected or changed: [{}]; missing or changed: [{}])",
                names(&schema, &expected).join(", "),
                names(&expected, &schema).join(", ")
            ),
        ));
    }
    let tables = table_counts(&conn).map_err(bad)?;
    if tables != m.tables {
        return Err(invalid(
            m,
            format!(
                "rows per table {tables:?} differ from the manifest {:?}",
                m.tables
            ),
        ));
    }
    let mut st = conn
        .prepare(
            "SELECT r.as_of, r.format, r.sha256, r.row_count,
                    (SELECT COUNT(*) FROM holdings h WHERE h.release_id = r.id)
             FROM releases r ORDER BY r.as_of DESC",
        )
        .map_err(bad)?;
    let found = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?.to_ascii_lowercase(),
                r.get::<_, i64>(3)? as u64,
                r.get::<_, i64>(4)? as u64,
            ))
        })
        .map_err(bad)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(bad)?;
    let mut wanted: Vec<&SnapshotRelease> = m.releases.iter().collect();
    wanted.sort_by_key(|r| std::cmp::Reverse(r.as_of));
    let expected: Vec<(String, String, String, u64, u64)> = wanted
        .iter()
        .map(|r| {
            (
                r.as_of.to_string(),
                r.format.clone(),
                r.sha256.clone(),
                r.rows,
                r.holdings,
            )
        })
        .collect();
    if found != expected {
        return Err(invalid(
            m,
            "its releases (date, format, SHA-256, lines, positions) differ from the manifest",
        ));
    }
    let info = read_info(&conn).map_err(bad)?;
    if info.as_ref().and_then(|i| i.content_sha256.as_deref()) != Some(m.content_sha256.as_str()) {
        return Err(invalid(
            m,
            "its recorded content digest differs from the manifest",
        ));
    }
    Ok(())
}

/// Copies `src` over the database at `db_path` with SQLite's online backup
/// API (one write transaction; readers keep their view until they finish),
/// after copying the current database to `backup_to` the same way.
fn swap_in(src: &Path, db_path: &Path, backup_to: Option<&Path>) -> Result<()> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let err = db_err(db_path);
    let mut dest = Connection::open(db_path).map_err(&err)?;
    dest.busy_timeout(Duration::from_secs(10)).map_err(&err)?;
    if let Some(bak) = backup_to {
        let _ = std::fs::remove_file(bak);
        for suffix in ["-journal", "-wal", "-shm"] {
            let _ = std::fs::remove_file(sidecar(bak, suffix));
        }
        dest.backup(rusqlite::MAIN_DB, bak, None).map_err(|e| {
            IdxError::Other(format!(
                "could not copy the ownership database to {} before replacing it ({e}); nothing was changed",
                bak.display()
            ))
        })?;
    }
    let source = Connection::open_with_flags(
        src,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(&err)?;
    let backup = Backup::new(&source, &mut dest).map_err(&err)?;
    let mut waits = 0;
    loop {
        match backup.step(-1).map_err(&err)? {
            StepResult::Done => break,
            StepResult::Busy | StepResult::Locked => {
                waits += 1;
                if waits > 5 {
                    return Err(IdxError::Other(format!(
                        "the ownership database {} is busy (an import running?); try again",
                        db_path.display()
                    )));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Installs a downloaded snapshot (`gz`) as the database at `db_path`:
/// decompresses it beside the database, checks it ([`validate`]), copies the
/// current database to `<db>.bak` when `keep_backup`, swaps the snapshot in
/// and opens the result. The database is untouched when any check fails.
/// Returns the backup path when one was written.
pub fn install(
    db_path: &Path,
    m: &SnapshotManifest,
    gz: &[u8],
    keep_backup: bool,
) -> Result<Option<PathBuf>> {
    let dir = db_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = db_path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "ownership.sqlite".into());
    let tmp = TempFile(unique_tmp(dir, &format!("{name}.snapshot")));
    decompress(gz, &tmp.0, m)?;
    validate(&tmp.0, m)?;
    let bak = keep_backup.then(|| backup_path(db_path));
    swap_in(&tmp.0, db_path, bak.as_deref())?;
    let db = OwnershipDb::open(db_path)?;
    let mut have: Vec<NaiveDate> = db.releases()?.iter().map(|r| r.as_of).collect();
    let mut want: Vec<NaiveDate> = m.releases.iter().map(|r| r.as_of).collect();
    have.sort();
    want.sort();
    if have != want {
        return Err(IdxError::Other(format!(
            "the installed ownership database lists {} releases, the snapshot {}; the previous database is at {}",
            have.len(),
            want.len(),
            bak.as_ref()
                .map(|b| b.display().to_string())
                .unwrap_or_else(|| "(none)".into())
        )));
    }
    Ok(bak)
}

// ---- sync and status ------------------------------------------------------

fn index_months(ix: &MirrorIndex) -> BTreeMap<NaiveDate, String> {
    ix.releases
        .iter()
        .map(|r| (r.as_of, r.sha256.to_ascii_lowercase()))
        .collect()
}

/// `idx ownership sync`: brings the database at `db_path` up to date from
/// the mirror at `base` (see [`plan`]), then imports the mirror's monthly
/// files the database still lacks (months newer than the snapshot, or all of
/// them when there is no snapshot or the database was kept). One sync of a
/// database runs at a time; a second one waits for it and then finds the
/// database up to date.
pub async fn sync(
    db_path: &Path,
    base: &str,
    force: bool,
    timeout: Duration,
) -> Result<Sourced<SyncReport>> {
    let timeout = timeout.max(MIN_TIMEOUT);
    let _lock = lock_sync(db_path, timeout).await?;
    let mut warnings = Vec::new();
    let manifest = fetch_manifest(base, timeout).await?;
    let index = match mirror::index(base, timeout).await {
        Ok(ix) => Some(ix),
        Err(e) if manifest.is_some() => {
            warnings.push(format!(
                "could not read the mirror's index.json ({e}); months newer than the snapshot were not checked"
            ));
            None
        }
        Err(e) => return Err(e),
    };
    let listed = index.as_ref().map(index_months).unwrap_or_default();
    let local = inspect(db_path)?;
    let decision = plan(&local, manifest.as_ref(), &listed, force);

    let mut report = SyncReport {
        action: SyncAction::UpToDate,
        database: db_path.display().to_string(),
        snapshot: manifest.as_ref().map(summary),
        installed: vec![],
        imported: vec![],
        kept: vec![],
        dropped: vec![],
        backup: None,
        releases: 0,
        newest: None,
    };
    let mut have: BTreeSet<NaiveDate> = local.months.keys().copied().collect();
    match &decision {
        Plan::NewerSchema(v) => {
            return Err(IdxError::Config(format!(
                "the ownership database {} was written by a newer idx (schema {v}; this idx installs {SCHEMA_VERSION}); upgrade idx, or pass --force to replace it with the snapshot (a .bak copy is kept)",
                db_path.display()
            )))
        }
        Plan::NoSnapshot => warnings.push(format!(
            "the mirror has no snapshot for database schema {SCHEMA_VERSION} ({}); sync reads its monthly files instead",
            manifest_name(SCHEMA_VERSION)
        )),
        Plan::Install | Plan::Replace { .. } => {
            let m = manifest.as_ref().expect("a plan to install has a manifest");
            let replace = !matches!(decision, Plan::Install);
            let gz = download(base, m, timeout).await?;
            let bak = install(db_path, m, &gz, replace)?;
            report.action = if replace {
                SyncAction::Updated
            } else {
                SyncAction::Installed
            };
            report.installed = {
                let mut d: Vec<NaiveDate> = m.releases.iter().map(|r| r.as_of).collect();
                d.sort();
                d
            };
            have = report.installed.iter().copied().collect();
            report.backup = bak.as_ref().map(|b| b.display().to_string());
            if let Plan::Replace {
                divergence,
                differed,
            } = &decision
            {
                if !divergence.is_empty() {
                    report.dropped = divergence.months();
                    warnings.push(format!(
                        "--force replaced a database holding {}",
                        divergence.describe()
                    ));
                }
                if !differed.is_empty() {
                    warnings.push(format!(
                        "the snapshot's copy of {} came from another IDX file than the local one",
                        join_dates(differed)
                    ));
                }
                if let Some(b) = &report.backup {
                    warnings.push(format!("the previous database is kept at {b}"));
                }
            }
        }
        Plan::Keep(div) => {
            report.kept = div.months();
            warnings.push(format!(
                "kept the local database: it holds {}; `idx ownership sync --force` replaces it with the snapshot (a .bak copy is kept)",
                div.describe()
            ));
        }
        Plan::UpToDate => {}
    }

    if let Some(ix) = &index {
        let files: Vec<_> = mirror::to_discovered(base, ix)
            .into_iter()
            .filter(|f| !have.contains(&f.as_of))
            .collect();
        if !files.is_empty() {
            let mut db = OwnershipDb::open(db_path)?;
            let (reports, notes) =
                super::import_discovered(&mut db, &files, false, false, timeout).await?;
            let mut imported: Vec<NaiveDate> = reports.iter().map(|r| r.release.as_of).collect();
            imported.sort();
            report.imported = imported;
            warnings.extend(notes);
        }
    }
    if report.action == SyncAction::UpToDate && !report.imported.is_empty() {
        report.action = SyncAction::Imported;
    }
    let after = inspect(db_path)?;
    report.releases = after.months.len() as u64;
    report.newest = after.months.keys().next_back().copied();
    let mut s = Sourced::fresh(report, SNAPSHOT_PROVIDER);
    s.warnings = warnings;
    Ok(s)
}

fn needed(decision: &Plan, missing_from_index: bool) -> Option<SyncNeeded> {
    match decision {
        Plan::NewerSchema(_) => None,
        Plan::Install => Some(SyncNeeded::Install),
        Plan::Replace { .. } => Some(SyncNeeded::Update),
        Plan::NoSnapshot | Plan::Keep(_) | Plan::UpToDate => Some(if missing_from_index {
            SyncNeeded::Import
        } else {
            SyncNeeded::UpToDate
        }),
    }
}

/// `idx ownership status`: the local database (never created) and, with a
/// mirror, what it holds and what `sync` would do. A mirror that cannot be
/// read is a warning, not an error.
pub async fn status(
    db_path: &Path,
    base: Option<&str>,
    timeout: Duration,
) -> Result<Sourced<StatusReport>> {
    let local = inspect(db_path)?;
    let mut warnings = Vec::new();
    if let Some(v) = local.user_version.filter(|v| *v > SCHEMA_VERSION) {
        warnings.push(format!(
            "the database was written by a newer idx (schema {v}; this idx knows {SCHEMA_VERSION}); upgrade idx"
        ));
    }
    let mut report = StatusReport {
        database: db_path.display().to_string(),
        exists: local.exists,
        bytes: local.bytes,
        db_schema_version: local.user_version,
        supported_schema_version: SCHEMA_VERSION,
        releases: local.months.len() as u64,
        oldest: local.months.keys().next().copied(),
        newest: local.months.keys().next_back().copied(),
        installed_snapshot: local.info.as_ref().map(|i| InstalledSnapshot {
            generated_at: i.generated_at.clone(),
            built_by: i.built_by.clone(),
            content_sha256: i.content_sha256.clone(),
            newest: i.releases.keys().next_back().copied(),
            releases: i.releases.len() as u64,
        }),
        remote: None,
        missing: vec![],
        local_only: vec![],
        sync: None,
    };
    if let Some(base) = base {
        let timeout = timeout.max(MIN_TIMEOUT);
        let manifest = fetch_manifest(base, timeout).await;
        let index = mirror::index(base, timeout).await;
        if let Err(e) = &manifest {
            warnings.push(format!(
                "could not read the mirror's {} ({e})",
                manifest_name(SCHEMA_VERSION)
            ));
        }
        if let Err(e) = &index {
            warnings.push(format!("could not read the mirror's index.json ({e})"));
        }
        if manifest.is_ok() || index.is_ok() {
            let snapshot = manifest.as_ref().ok().cloned().flatten();
            let listed = index.as_ref().map(index_months).unwrap_or_default();
            let mut on_mirror: BTreeSet<NaiveDate> = listed.keys().copied().collect();
            if let Some(m) = &snapshot {
                on_mirror.extend(m.releases.iter().map(|r| r.as_of));
            }
            report.missing = on_mirror
                .iter()
                .filter(|d| !local.months.contains_key(d))
                .copied()
                .collect();
            report.local_only = local
                .months
                .keys()
                .filter(|d| !on_mirror.contains(d))
                .copied()
                .collect();
            if let (Ok(m), Ok(_)) = (&manifest, &index) {
                let decision = plan(&local, m.as_ref(), &listed, false);
                let missing_from_index = listed.keys().any(|d| !local.months.contains_key(d));
                report.sync = needed(&decision, missing_from_index);
                if matches!(decision, Plan::NoSnapshot) {
                    warnings.push(format!(
                        "the mirror has no snapshot for database schema {SCHEMA_VERSION}; sync imports its monthly files"
                    ));
                }
            }
            let ix = index.as_ref().ok();
            report.remote = Some(MirrorState {
                mirror: base.to_string(),
                index_generated_at: ix.and_then(|i| i.generated_at.clone()),
                releases: ix.map(|i| i.releases.len() as u64),
                newest: ix.and_then(|i| i.releases.iter().map(|r| r.as_of).max()),
                snapshot: snapshot.as_ref().map(summary),
            });
        }
    }
    let mut s = Sourced::fresh(report, super::PROVIDER);
    s.warnings = warnings;
    Ok(s)
}

// ---- build ----------------------------------------------------------------

/// `idx ownership build-snapshot`: writes the snapshot of the database at
/// `db_path` into `out` (`ownership-v{V}-….sqlite.gz` and
/// `snapshot-v{V}.json`) and returns its manifest. `sources` (the mirror's
/// `index.json`) replaces each release's source path with its idx.co.id URL
/// when the file's SHA-256 matches; every `imported_at` becomes
/// `generated_at`. The database itself is not changed: one of another
/// schema version is refused, never migrated.
pub fn build(
    db_path: &Path,
    out: &Path,
    sources: Option<&MirrorIndex>,
    generated_at: DateTime<Utc>,
) -> Result<SnapshotManifest> {
    let local = inspect(db_path)?;
    if !local.exists {
        return Err(IdxError::Usage(format!(
            "no ownership database at {}; import the monthly files first",
            db_path.display()
        )));
    }
    if let Some(v) = local.user_version.filter(|v| *v != SCHEMA_VERSION) {
        return Err(IdxError::Usage(format!(
            "the ownership database {} has schema version {v} and this idx builds version {SCHEMA_VERSION}; build from a database this idx imported into (build-snapshot never migrates the file it reads)",
            db_path.display()
        )));
    }
    if local.months.is_empty() {
        return Err(IdxError::Usage(format!(
            "the ownership database {} has no releases; import the monthly files first",
            db_path.display()
        )));
    }
    let err = db_err(db_path);
    // Read-write without CREATE and query-only, as in `inspect`.
    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(&err)?;
    conn.execute_batch("PRAGMA query_only = ON;")
        .map_err(&err)?;
    conn.busy_timeout(Duration::from_secs(10)).map_err(&err)?;
    let legacy: Vec<NaiveDate> = {
        let mut st = conn
            .prepare("SELECT as_of FROM releases WHERE positions IS NULL ORDER BY as_of")
            .map_err(&err)?;
        let rows = st
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(&err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(&err)?;
        rows.iter()
            .filter_map(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
            .collect()
    };
    if !legacy.is_empty() {
        return Err(IdxError::Usage(format!(
            "releases {} were imported by idx 0.8.1 or older (no register lines); re-import them with `idx ownership import --all --force` first",
            join_dates(&legacy)
        )));
    }
    std::fs::create_dir_all(out)?;
    let generated_at = generated_at.trunc_subsecs(0);
    let stamp = generated_at.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let tmp = TempFile(unique_tmp(out, "build"));
    let _ = std::fs::remove_file(&tmp.0);
    // VACUUM INTO only reads this database (query_only refuses it anyway).
    conn.execute_batch("PRAGMA query_only = OFF;")
        .map_err(&err)?;
    conn.execute("VACUUM INTO ?1", [tmp.0.to_string_lossy()])
        .map_err(&err)?;
    drop(conn);
    let terr = db_err(&tmp.0);
    let (tables, releases, content) = {
        let conn = Connection::open(&tmp.0).map_err(&terr)?;
        conn.execute_batch("PRAGMA journal_mode=DELETE;")
            .map_err(&terr)?;
        if let Some(ix) = sources {
            for r in &ix.releases {
                if let Some(url) = &r.source_url {
                    conn.execute(
                        "UPDATE releases SET source = ?1 WHERE as_of = ?2 AND lower(sha256) = lower(?3)",
                        params![url, r.as_of.to_string(), r.sha256],
                    )
                    .map_err(&terr)?;
                }
            }
        }
        conn.execute("UPDATE releases SET imported_at = ?1", [&stamp])
            .map_err(&terr)?;
        let content = content_digest(&conn).map_err(&terr)?;
        let tables = table_counts(&conn).map_err(&terr)?;
        let releases = snapshot_releases(&conn).map_err(&terr)?;
        let listed: Vec<InfoRelease> = releases
            .iter()
            .map(|r| InfoRelease {
                as_of: r.as_of,
                sha256: r.sha256.clone(),
            })
            .collect();
        conn.execute_batch(&format!("DROP TABLE IF EXISTS {INFO_TABLE}; {INFO_DDL};"))
            .map_err(&terr)?;
        for (k, v) in [
            ("schema", MANIFEST_SCHEMA.to_string()),
            ("generated_at", stamp.clone()),
            ("built_by", built_by()),
            ("db_schema_version", SCHEMA_VERSION.to_string()),
            ("content_sha256", content.clone()),
            ("releases", serde_json::to_string(&listed)?),
        ] {
            conn.execute(
                &format!("INSERT INTO {INFO_TABLE} (key, value) VALUES (?1, ?2)"),
                params![k, v],
            )
            .map_err(&terr)?;
        }
        conn.execute_batch("VACUUM;").map_err(&terr)?;
        let page: i64 = conn
            .pragma_query_value(None, "page_size", |r| r.get(0))
            .map_err(&terr)?;
        if page != PAGE_SIZE {
            return Err(IdxError::Other(format!(
                "the ownership database uses {page}-byte pages; a snapshot needs {PAGE_SIZE}"
            )));
        }
        let v: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(&terr)?;
        if v != SCHEMA_VERSION {
            return Err(IdxError::Other(format!(
                "the snapshot copy has schema version {v}, expected {SCHEMA_VERSION}"
            )));
        }
        (tables, releases, content)
    };
    let sqlite = std::fs::read(&tmp.0)?;
    if sqlite.len() < 100 || sqlite[18] != 1 || sqlite[19] != 1 {
        return Err(IdxError::Other(
            "the snapshot copy is not in rollback-journal mode".into(),
        ));
    }
    let gz = gzip(&sqlite)?;
    let sha256 = sha256_hex(&gz);
    let newest = releases.first().map(|r| r.as_of).expect("releases checked");
    let manifest = SnapshotManifest {
        schema: MANIFEST_SCHEMA.to_string(),
        generated_at,
        built_by: built_by(),
        db_schema_version: SCHEMA_VERSION,
        file: file_name(SCHEMA_VERSION, newest, &sha256),
        compression: "gzip".into(),
        bytes: gz.len() as u64,
        sha256,
        sqlite_bytes: sqlite.len() as u64,
        sqlite_sha256: sha256_hex(&sqlite),
        content_sha256: content,
        tables,
        releases,
    };
    std::fs::write(out.join(&manifest.file), &gz)?;
    std::fs::write(
        out.join(manifest_name(SCHEMA_VERSION)),
        serde_json::to_string_pretty(&manifest)? + "\n",
    )?;
    Ok(manifest)
}

fn built_by() -> String {
    format!("idx {}", env!("CARGO_PKG_VERSION"))
}

fn gzip(bytes: &[u8]) -> Result<Vec<u8>> {
    // The default header has no file name and mtime 0, so the same SQLite
    // bytes always give the same file.
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(bytes)?;
    Ok(enc.finish()?)
}

fn snapshot_releases(conn: &Connection) -> rusqlite::Result<Vec<SnapshotRelease>> {
    let mut st = conn.prepare(
        "SELECT r.as_of, r.format, r.sha256, r.source, r.row_count,
                (SELECT COUNT(*) FROM holdings h WHERE h.release_id = r.id),
                (SELECT COUNT(DISTINCT symbol) FROM holdings h WHERE h.release_id = r.id),
                (SELECT COUNT(DISTINCT entity_id) FROM holdings h WHERE h.release_id = r.id)
         FROM releases r ORDER BY r.as_of DESC",
    )?;
    let rows = st
        .query_map([], |r| {
            let source: String = r.get(3)?;
            Ok(SnapshotRelease {
                as_of: NaiveDate::parse_from_str(&r.get::<_, String>(0)?, "%Y-%m-%d")
                    .unwrap_or_default(),
                format: r.get(1)?,
                sha256: r.get::<_, String>(2)?.to_ascii_lowercase(),
                source_url: (source.starts_with("https://") || source.starts_with("http://"))
                    .then_some(source),
                rows: r.get::<_, i64>(4)? as u64,
                holdings: r.get::<_, i64>(5)? as u64,
                issuers: r.get::<_, i64>(6)? as u64,
                entities: r.get::<_, i64>(7)? as u64,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ownership/snapshot_manifest_v2_constructed.json"
    );
    const JULY: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ownership/above1_20260731_synthetic.xlsx"
    );
    const AUGUST: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/ownership/above1_20260831_excerpt.xlsx"
    );

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn sha(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn fixture_value() -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(FIXTURE).unwrap()).unwrap()
    }

    #[test]
    fn the_manifest_fixture_parses() {
        let m = parse_manifest(&std::fs::read(FIXTURE).unwrap(), 2).unwrap();
        assert_eq!(m.schema, MANIFEST_SCHEMA);
        assert_eq!(m.db_schema_version, 2);
        assert_eq!(m.releases.len(), 2);
        assert_eq!(m.releases[0].as_of, d("2026-09-30"));
        assert_eq!(m.releases[0].holdings, 7151);
        assert_eq!(m.tables["holdings"], 57451);
        let s = summary(&m);
        assert_eq!(
            (s.oldest, s.newest, s.releases),
            (d("2026-08-31"), d("2026-09-30"), 2)
        );
        assert_eq!(manifest_name(2), "snapshot-v2.json");
        // The version comes from the name the client asked for.
        assert!(parse_manifest(&std::fs::read(FIXTURE).unwrap(), 3).is_err());
    }

    type Edit = Box<dyn Fn(&mut serde_json::Value)>;

    #[test]
    fn bad_manifests_are_refused() {
        let file = |name: &str| -> Edit {
            let name = name.to_string();
            Box::new(move |v| v["file"] = name.clone().into())
        };
        let cases: Vec<(&str, Edit)> = vec![
            (
                "schema",
                Box::new(|v| v["schema"] = "idx.ownership.snapshot.v9".into()),
            ),
            ("version", Box::new(|v| v["db_schema_version"] = 1.into())),
            ("compression", Box::new(|v| v["compression"] = "xz".into())),
            ("zero bytes", Box::new(|v| v["bytes"] = 0.into())),
            ("huge gz", Box::new(|v| v["bytes"] = (MAX_BYTES + 1).into())),
            (
                "huge sqlite",
                Box::new(|v| v["sqlite_bytes"] = (MAX_SQLITE_BYTES + 1).into()),
            ),
            (
                "upper-case hex",
                Box::new(|v| v["sqlite_sha256"] = sha('A').into()),
            ),
            (
                "short hex",
                Box::new(|v| v["content_sha256"] = "abc".into()),
            ),
            (
                "path",
                file("../ownership-v2-2026-09-30-3f9a2c7e5b1d.sqlite.gz"),
            ),
            (
                "subdir",
                file("x/ownership-v2-2026-09-30-3f9a2c7e5b1d.sqlite.gz"),
            ),
            (
                "other date",
                file("ownership-v2-2026-08-31-3f9a2c7e5b1d.sqlite.gz"),
            ),
            (
                "other hash",
                file("ownership-v2-2026-09-30-000000000000.sqlite.gz"),
            ),
            (
                "no releases",
                Box::new(|v| v["releases"] = serde_json::json!([])),
            ),
            (
                "duplicate",
                Box::new(|v| {
                    let first = v["releases"][0].clone();
                    v["releases"][1] = first;
                }),
            ),
            (
                "release hash",
                Box::new(|v| v["releases"][0]["sha256"] = "zz".into()),
            ),
            (
                "missing field",
                Box::new(|v| {
                    v.as_object_mut().unwrap().remove("tables");
                }),
            ),
        ];
        for (name, edit) in cases {
            let mut v = fixture_value();
            edit(&mut v);
            let bytes = serde_json::to_vec(&v).unwrap();
            let e = parse_manifest(&bytes, 2).unwrap_err();
            assert_eq!(e.exit_code(), 3, "{name}: {e}");
        }
        let mut big = std::fs::read(FIXTURE).unwrap();
        big.resize(MAX_MANIFEST_BYTES as usize + 1, b' ');
        assert!(parse_manifest(&big, 2).is_err());
    }

    fn manifest(months: &[(&str, char)], content: char) -> SnapshotManifest {
        let mut m = parse_manifest(&std::fs::read(FIXTURE).unwrap(), 2).unwrap();
        m.content_sha256 = sha(content);
        m.releases = months
            .iter()
            .map(|(date, c)| SnapshotRelease {
                as_of: d(date),
                format: "xlsx".into(),
                sha256: sha(*c),
                source_url: None,
                rows: 1,
                holdings: 1,
                issuers: 1,
                entities: 1,
            })
            .collect();
        m
    }

    type Months<'a> = &'a [(&'a str, char)];

    fn local(months: Months, info: Option<(char, Months)>) -> LocalState {
        let months: BTreeMap<NaiveDate, String> =
            months.iter().map(|(x, c)| (d(x), sha(*c))).collect();
        LocalState {
            exists: true,
            bytes: Some(1),
            user_version: Some(SCHEMA_VERSION),
            tables: [("releases".to_string(), months.len() as u64)].into(),
            months,
            info: info.map(|(content, rel)| SnapshotInfo {
                content_sha256: Some(sha(content)),
                releases: rel.iter().map(|(x, c)| (d(x), sha(*c))).collect(),
                ..SnapshotInfo::default()
            }),
        }
    }

    fn index(months: Months) -> BTreeMap<NaiveDate, String> {
        months.iter().map(|(x, c)| (d(x), sha(*c))).collect()
    }

    #[test]
    fn plan_covers_every_case() {
        let snap = manifest(&[("2026-07-31", 'a'), ("2026-08-31", 'b')], 'c');
        let ix = index(&[("2026-07-31", 'a'), ("2026-08-31", 'b')]);
        let installed: Months = &[("2026-07-31", 'a'), ("2026-08-31", 'b')];

        // A newer idx wrote the file: refuse, unless forced.
        let mut newer = local(installed, None);
        newer.user_version = Some(SCHEMA_VERSION + 1);
        assert_eq!(
            plan(&newer, Some(&snap), &ix, false),
            Plan::NewerSchema(SCHEMA_VERSION + 1)
        );
        assert!(matches!(
            plan(&newer, Some(&snap), &ix, true),
            Plan::Replace { .. }
        ));

        // No snapshot for this schema: raw imports.
        assert_eq!(plan(&local(&[], None), None, &ix, false), Plan::NoSnapshot);

        // Missing or empty database: install.
        assert_eq!(
            plan(&LocalState::default(), Some(&snap), &ix, false),
            Plan::Install
        );
        assert_eq!(
            plan(&local(&[], None), Some(&snap), &ix, false),
            Plan::Install
        );

        // Installed from this snapshot: up to date.
        let same = local(installed, Some(('c', installed)));
        assert_eq!(plan(&same, Some(&snap), &ix, false), Plan::UpToDate);

        // ... plus a month imported from the index after it (the lag step).
        let ix_more = index(&[
            ("2026-07-31", 'a'),
            ("2026-08-31", 'b'),
            ("2026-09-30", 'd'),
        ]);
        let lagged = local(
            &[
                ("2026-07-31", 'a'),
                ("2026-08-31", 'b'),
                ("2026-09-30", 'd'),
            ],
            Some(('c', installed)),
        );
        assert_eq!(plan(&lagged, Some(&snap), &ix_more, false), Plan::UpToDate);

        // A month neither the snapshot nor the index has: kept unless forced.
        let own = local(
            &[
                ("2026-07-31", 'a'),
                ("2026-08-31", 'b'),
                ("2026-09-30", 'e'),
            ],
            Some(('c', installed)),
        );
        let Plan::Keep(div) = plan(&own, Some(&snap), &ix_more, false) else {
            panic!("keep")
        };
        assert_eq!(div.local_only, vec![d("2026-09-30")]);
        assert!(div.modified.is_empty() && div.tables.is_empty());
        let Plan::Replace {
            divergence,
            differed,
        } = plan(&own, Some(&snap), &ix_more, true)
        else {
            panic!("replace")
        };
        assert_eq!(divergence.months(), vec![d("2026-09-30")]);
        assert!(differed.is_empty());

        // A month re-imported locally from another file after the install.
        let modified = local(
            &[("2026-07-31", 'a'), ("2026-08-31", 'f')],
            Some(('c', installed)),
        );
        let Plan::Keep(div) = plan(&modified, Some(&snap), &ix, false) else {
            panic!("keep")
        };
        assert_eq!(div.modified, vec![d("2026-08-31")]);
        assert!(matches!(
            plan(&modified, Some(&snap), &ix, true),
            Plan::Replace { .. }
        ));
        // ... unless the new snapshot carries that same file.
        let snap_f = manifest(&[("2026-07-31", 'a'), ("2026-08-31", 'f')], 'g');
        assert!(matches!(
            plan(&modified, Some(&snap_f), &ix, false),
            Plan::Replace { ref differed, .. } if differed.is_empty()
        ));

        // Rows in a table the snapshot leaves empty (hand-made rules, other imports).
        let mut rules = same.clone();
        rules.tables.insert("resolutions".into(), 3);
        let Plan::Keep(div) = plan(&rules, Some(&snap), &ix, false) else {
            panic!("keep")
        };
        assert_eq!(div.tables, vec![("resolutions".to_string(), 3)]);
        assert!(div.months().is_empty());
        let mut only_rules = local(&[], None);
        only_rules.tables.insert("resolutions".into(), 1);
        assert!(matches!(
            plan(&only_rules, Some(&snap), &ix, false),
            Plan::Keep(_)
        ));

        // A subset (the snapshot has more months), built by local imports.
        let subset = local(&[("2026-07-31", 'a')], None);
        assert_eq!(
            plan(&subset, Some(&snap), &ix, false),
            Plan::Replace {
                divergence: Divergence::default(),
                differed: vec![]
            }
        );
        // Same months, local imports: replaced (once; then it carries snapshot_info).
        assert!(matches!(
            plan(&local(installed, None), Some(&snap), &ix, false),
            Plan::Replace { .. }
        ));
        // A new build (parser fix): replaced.
        let rebuilt = manifest(&[("2026-07-31", 'a'), ("2026-08-31", 'b')], 'h');
        assert!(matches!(
            plan(&same, Some(&rebuilt), &ix, false),
            Plan::Replace { .. }
        ));
        // An older schema: replaced.
        let mut older = same.clone();
        older.user_version = Some(0);
        assert!(matches!(
            plan(&older, Some(&snap), &ix, false),
            Plan::Replace { .. }
        ));
        // A re-published IDX file: replaced, and named.
        let republished = manifest(&[("2026-07-31", 'a'), ("2026-08-31", 'i')], 'j');
        let Plan::Replace {
            differed,
            divergence,
        } = plan(&same, Some(&republished), &ix, false)
        else {
            panic!("replace")
        };
        assert_eq!(differed, vec![d("2026-08-31")]);
        assert!(divergence.is_empty());
    }

    fn imported_db(path: &Path, files: &[&str]) {
        let mut db = OwnershipDb::open(path).unwrap();
        for f in files {
            super::super::import_file(&mut db, Path::new(f)).unwrap();
        }
    }

    fn t0() -> DateTime<Utc> {
        "2026-11-06T00:00:00Z".parse().unwrap()
    }

    #[test]
    fn build_writes_a_checked_reproducible_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("a.sqlite");
        imported_db(&db, &[JULY, AUGUST]);
        let out = dir.path().join("out");
        let m = build(&db, &out, None, t0()).unwrap();
        assert_eq!(m.db_schema_version, SCHEMA_VERSION);
        assert_eq!(
            m.releases.iter().map(|r| r.as_of).collect::<Vec<_>>(),
            vec![d("2026-08-31"), d("2026-07-31")]
        );
        assert_eq!(
            m.file,
            file_name(SCHEMA_VERSION, d("2026-08-31"), &m.sha256)
        );
        assert!(m
            .file
            .starts_with(&format!("ownership-v{SCHEMA_VERSION}-2026-08-31-")));
        assert!(m
            .releases
            .iter()
            .all(|r| r.holdings > 0 && r.source_url.is_none()));
        assert_eq!(m.tables["releases"], 2);
        assert!(!m.tables.contains_key(INFO_TABLE));
        // The manifest on disk parses as a client reads it.
        let written = std::fs::read(out.join(manifest_name(SCHEMA_VERSION))).unwrap();
        assert_eq!(parse_manifest(&written, SCHEMA_VERSION).unwrap(), m);
        // The gzip decompresses to the recorded size and hash, and validates.
        let gz = std::fs::read(out.join(&m.file)).unwrap();
        assert_eq!(gz.len() as u64, m.bytes);
        assert_eq!(sha256_hex(&gz), m.sha256);
        let tmp = dir.path().join("check.sqlite");
        decompress(&gz, &tmp, &m).unwrap();
        validate(&tmp, &m).unwrap();
        let info = read_info(&Connection::open(&tmp).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            info.content_sha256.as_deref(),
            Some(m.content_sha256.as_str())
        );
        assert_eq!(info.generated_at.as_deref(), Some("2026-11-06T00:00:00Z"));
        assert_eq!(info.releases.len(), 2);
        // Nothing but the two assets is left in the output directory.
        let mut names: Vec<String> = std::fs::read_dir(&out)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec![m.file.clone(), manifest_name(SCHEMA_VERSION)]);

        // Another database from the same files, built later: same content digest.
        let db2 = dir.path().join("b.sqlite");
        imported_db(&db2, &[JULY, AUGUST]);
        let later: DateTime<Utc> = "2026-11-07T08:00:00Z".parse().unwrap();
        let m2 = build(&db2, &dir.path().join("out2"), None, later).unwrap();
        assert_eq!(m2.content_sha256, m.content_sha256);
        assert_ne!(m2.sqlite_sha256, m.sqlite_sha256, "generated_at is inside");
        // One changed holding changes it.
        Connection::open(&db2)
            .unwrap()
            .execute(
                "UPDATE holdings SET pct = pct + 0.01 WHERE rowid = (SELECT MIN(rowid) FROM holdings)",
                [],
            )
            .unwrap();
        let m3 = build(&db2, &dir.path().join("out3"), None, t0()).unwrap();
        assert_ne!(m3.content_sha256, m.content_sha256);

        // Sources: provenance from the mirror index when the SHA-256 matches.
        let aug_sha = m.releases[0].sha256.clone();
        let ix = mirror::parse_index(
            serde_json::json!({
                "schema": mirror::INDEX_SCHEMA,
                "releases": [
                    {"as_of": "2026-08-31", "format": "xlsx", "file": "a.xlsx", "sha256": aug_sha,
                     "source_url": "https://www.idx.co.id/Media/x/peng-2026-08.xlsx"},
                    {"as_of": "2026-07-31", "format": "xlsx", "file": "b.xlsx", "sha256": sha('0'),
                     "source_url": "https://www.idx.co.id/Media/y/other.xlsx"}
                ]
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        let m4 = build(&db, &dir.path().join("out4"), Some(&ix), t0()).unwrap();
        assert_eq!(
            m4.releases[0].source_url.as_deref(),
            Some("https://www.idx.co.id/Media/x/peng-2026-08.xlsx")
        );
        assert_eq!(m4.releases[1].source_url, None);

        // Nothing to build from.
        let none = dir.path().join("none.sqlite");
        assert_eq!(build(&none, &out, None, t0()).unwrap_err().exit_code(), 2);
        let empty = dir.path().join("empty.sqlite");
        OwnershipDb::open(&empty).unwrap();
        assert_eq!(build(&empty, &out, None, t0()).unwrap_err().exit_code(), 2);
    }

    #[test]
    fn install_swaps_under_a_reader_and_keeps_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.sqlite");
        imported_db(&src, &[JULY, AUGUST]);
        let m = build(&src, &dir.path().join("m"), None, t0()).unwrap();
        let gz = std::fs::read(dir.path().join("m").join(&m.file)).unwrap();

        let live = dir.path().join("live").join("ownership.sqlite");
        std::fs::create_dir_all(live.parent().unwrap()).unwrap();
        imported_db(&live, &[JULY]);
        let before = inspect(&live).unwrap();
        assert_eq!(before.months.len(), 1);
        assert!(before.info.is_none());

        // A reader in the middle of a transaction keeps its view.
        let reader = Connection::open(&live).unwrap();
        reader.execute_batch("BEGIN").unwrap();
        let count = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM releases", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(count(&reader), 1);
        let bak = install(&live, &m, &gz, true).unwrap().unwrap();
        assert_eq!(count(&reader), 1);
        reader.execute_batch("COMMIT").unwrap();
        assert_eq!(count(&reader), 2);
        drop(reader);

        let after = inspect(&live).unwrap();
        assert_eq!(after.months.len(), 2);
        assert_eq!(after.user_version, Some(SCHEMA_VERSION));
        assert_eq!(
            after.info.unwrap().content_sha256,
            Some(m.content_sha256.clone())
        );
        assert_eq!(bak, backup_path(&live));
        assert_eq!(
            inspect(&bak).unwrap().months.len(),
            1,
            "the .bak holds the old database"
        );
        // Queries work on the installed file, still in WAL mode.
        let db = OwnershipDb::open(&live).unwrap();
        assert_eq!(db.holders("AADI", None).unwrap().as_of, d("2026-08-31"));
        drop(db);
        let mode: String = Connection::open(&live)
            .unwrap()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");

        // Missing file: installed without a backup.
        let fresh = dir.path().join("fresh").join("o.sqlite");
        assert_eq!(install(&fresh, &m, &gz, false).unwrap(), None);
        assert_eq!(inspect(&fresh).unwrap().months.len(), 2);
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".snapshot-"))
            .collect()
    }

    #[test]
    fn a_bad_download_leaves_the_database_alone() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.sqlite");
        imported_db(&src, &[JULY, AUGUST]);
        let m = build(&src, &dir.path().join("m"), None, t0()).unwrap();
        let gz = std::fs::read(dir.path().join("m").join(&m.file)).unwrap();
        let live = dir.path().join("ownership.sqlite");
        imported_db(&live, &[JULY]);
        let before = inspect(&live).unwrap();

        let mut tampered = gz.clone();
        let mid = tampered.len() / 2;
        tampered[mid] ^= 0xff;
        let truncated = &gz[..gz.len() / 2];
        let mut wrong_count = m.clone();
        wrong_count.releases[0].holdings += 1;
        let mut wrong_tables = m.clone();
        wrong_tables.tables.insert("holdings".into(), 1);
        let mut wrong_content = m.clone();
        wrong_content.content_sha256 = sha('0');
        for (name, manifest, bytes) in [
            ("tampered", &m, tampered.as_slice()),
            ("truncated", &m, truncated),
            ("row count", &wrong_count, gz.as_slice()),
            ("tables", &wrong_tables, gz.as_slice()),
            ("content", &wrong_content, gz.as_slice()),
        ] {
            let e = install(&live, manifest, bytes, true).unwrap_err();
            assert_eq!(e.exit_code(), 3, "{name}: {e}");
            assert_eq!(inspect(&live).unwrap(), before, "{name}: database changed");
            assert!(leftovers(dir.path()).is_empty(), "{name}: temp file left");
            assert!(
                !backup_path(&live).exists(),
                "{name}: backup written before the checks"
            );
        }
    }

    #[test]
    fn inspect_never_creates_and_reports_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("o.sqlite");
        assert_eq!(inspect(&path).unwrap(), LocalState::default());
        assert!(!path.exists());
        std::fs::write(&path, [b'x'; 4096]).unwrap();
        assert_eq!(inspect(&path).unwrap_err().exit_code(), 1);
    }

    #[test]
    fn content_digest_types_and_escapes_values() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE t (a TEXT, b);
             INSERT INTO t VALUES ('x|y', 1), ('x', 'y|1');",
        )
        .unwrap();
        let one = content_digest(&conn).unwrap();
        conn.execute_batch("DELETE FROM t; INSERT INTO t VALUES ('x', 'y|1'), ('x|y', 1);")
            .unwrap();
        assert_eq!(
            content_digest(&conn).unwrap(),
            one,
            "row order does not matter"
        );
        conn.execute_batch("UPDATE t SET b = '1' WHERE a = 'x|y';")
            .unwrap();
        assert_ne!(
            content_digest(&conn).unwrap(),
            one,
            "text 1 differs from integer 1"
        );
    }

    // ---- sync against a file:// mirror ----------------------------------

    /// A `file://` URL for a directory, as `IDX_OWNERSHIP_MIRROR` takes it.
    fn file_url(dir: &Path) -> String {
        let p = dir.display().to_string().replace('\\', "/");
        if p.starts_with('/') {
            format!("file://{p}")
        } else {
            format!("file:///{p}")
        }
    }

    /// The monthly files of a mirror in `dir` and their index.json.
    fn write_index(dir: &Path, files: &[(&str, Vec<u8>)]) {
        let releases: Vec<serde_json::Value> = files
            .iter()
            .map(|(as_of, bytes)| {
                let file = format!("ksei-above1-{as_of}.xlsx");
                std::fs::write(dir.join(&file), bytes).unwrap();
                serde_json::json!({
                    "as_of": as_of, "format": "xlsx", "file": file, "sha256": sha256_hex(bytes),
                })
            })
            .collect();
        let index = serde_json::json!({"schema": mirror::INDEX_SCHEMA, "releases": releases});
        std::fs::write(dir.join("index.json"), index.to_string()).unwrap();
    }

    fn july_august() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("2026-07-31", std::fs::read(JULY).unwrap()),
            ("2026-08-31", std::fs::read(AUGUST).unwrap()),
        ]
    }

    /// A mirror under `root`: the snapshot of July and August and both as
    /// monthly files. Returns its directory and base URL.
    fn snapshot_mirror(root: &Path) -> (PathBuf, String) {
        let dir = root.join("mirror");
        std::fs::create_dir_all(&dir).unwrap();
        let src = root.join("mirror-src.sqlite");
        imported_db(&src, &[JULY, AUGUST]);
        build(&src, &dir, None, t0()).unwrap();
        write_index(&dir, &july_august());
        let base = file_url(&dir);
        (dir, base)
    }

    const T: Duration = Duration::from_secs(5);

    #[test]
    fn temp_files_are_unique_per_call() {
        let dir = Path::new("x");
        assert_ne!(unique_tmp(dir, "o.snapshot"), unique_tmp(dir, "o.snapshot"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn parallel_syncs_run_one_at_a_time() {
        let root = tempfile::tempdir().unwrap();
        let (_, base) = snapshot_mirror(root.path());
        let live = root.path().join("live").join("ownership.sqlite");
        let tasks: Vec<_> = (0..4)
            .map(|_| {
                let (live, base) = (live.clone(), base.clone());
                tokio::spawn(async move { sync(&live, &base, false, T).await })
            })
            .collect();
        let mut actions = Vec::new();
        for t in tasks {
            actions.push(t.await.unwrap().unwrap().data.action);
        }
        let count = |a: SyncAction| actions.iter().filter(|x| **x == a).count();
        assert_eq!(
            (count(SyncAction::Installed), count(SyncAction::UpToDate)),
            (1, 3),
            "{actions:?}"
        );
        assert!(!backup_path(&live).exists(), "nothing was replaced");
        assert!(leftovers(live.parent().unwrap()).is_empty());
        assert_eq!(inspect(&live).unwrap().months.len(), 2);
    }

    #[tokio::test]
    async fn a_sync_waits_for_one_in_another_process() {
        let dir = tempfile::tempdir().unwrap();
        let live = dir.path().join("o.sqlite");
        // Another idx process holding the lock, as SQLite sees it.
        let other = Connection::open(lock_path(&live)).unwrap();
        other.busy_timeout(Duration::ZERO).unwrap();
        other.execute_batch("BEGIN EXCLUSIVE").unwrap();
        let e = lock_sync(&live, Duration::from_millis(300))
            .await
            .err()
            .expect("locked");
        assert!(e.to_string().contains("another idx is syncing"), "{e}");
        other.execute_batch("COMMIT").unwrap();
        let held = lock_sync(&live, Duration::from_millis(300)).await.unwrap();
        assert!(other.execute_batch("BEGIN EXCLUSIVE").is_err());
        drop(held);
        other.execute_batch("BEGIN EXCLUSIVE; COMMIT").unwrap();
    }

    #[test]
    fn validate_refuses_schema_objects_idx_does_not_create() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src.sqlite");
        imported_db(&src, &[JULY]);
        let m = build(&src, &dir.path().join("m"), None, t0()).unwrap();
        let gz = std::fs::read(dir.path().join("m").join(&m.file)).unwrap();
        let clean = dir.path().join("clean.sqlite");
        decompress(&gz, &clean, &m).unwrap();
        validate(&clean, &m).unwrap();
        for (name, sql) in [
            (
                "trigger",
                "CREATE TRIGGER t AFTER INSERT ON releases BEGIN DELETE FROM holdings; END",
            ),
            (
                "trigger on snapshot_info",
                "CREATE TRIGGER t AFTER UPDATE ON snapshot_info BEGIN SELECT 1; END",
            ),
            ("view", "CREATE VIEW v AS SELECT * FROM releases"),
            ("index", "CREATE INDEX extra ON holdings (pct)"),
            ("column", "ALTER TABLE releases ADD COLUMN note TEXT"),
            ("table", "CREATE TABLE extra (x)"),
        ] {
            let copy = dir.path().join("copy.sqlite");
            std::fs::copy(&clean, &copy).unwrap();
            Connection::open(&copy).unwrap().execute_batch(sql).unwrap();
            let e = validate(&copy, &m).unwrap_err();
            assert_eq!(e.exit_code(), 3, "{name}: {e}");
            std::fs::remove_file(&copy).unwrap();
        }
    }

    #[test]
    fn build_never_migrates_the_database_it_reads() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old.sqlite");
        Connection::open(&old)
            .unwrap()
            .execute_batch(
                "CREATE TABLE releases (id INTEGER PRIMARY KEY, as_of TEXT NOT NULL, sha256 TEXT NOT NULL);
                 INSERT INTO releases (as_of, sha256) VALUES ('2026-07-31', 'ab');",
            )
            .unwrap();
        let before = std::fs::read(&old).unwrap();
        let out = dir.path().join("out");
        let e = build(&old, &out, None, t0()).unwrap_err();
        assert_eq!(e.exit_code(), 2, "{e}");
        assert!(e.to_string().contains("schema version 0"), "{e}");
        assert_eq!(std::fs::read(&old).unwrap(), before, "the file changed");
        assert!(!out.exists());
    }
}
