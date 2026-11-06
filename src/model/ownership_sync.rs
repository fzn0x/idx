//! The prebuilt ownership snapshot: the manifest CI publishes on the mirror
//! (`snapshot-v{V}.json`, `idx.ownership.snapshot.v1`) and the reports of
//! `idx ownership sync` (`idx.ownership.snapshot-sync.v1`) and
//! `idx ownership status` (`idx.ownership.status.v1`). These structs are the
//! JSON contract; `ownership::snapshot` builds, checks and installs them.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// `snapshot-v{V}.json` on the mirror: what the published SQLite file holds
/// and how to check it. One manifest per database schema version `V`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotManifest {
    /// Always `idx.ownership.snapshot.v1`.
    pub schema: String,
    /// When the snapshot was built (UTC).
    pub generated_at: DateTime<Utc>,
    /// The idx that built it, e.g. `idx 0.8.2`.
    pub built_by: String,
    /// `PRAGMA user_version` of the SQLite file (the `V` in the manifest's name).
    pub db_schema_version: i64,
    /// Asset name on the mirror release, `ownership-v{V}-{newest as_of}-{first 12 hex of sha256}.sqlite.gz`.
    pub file: String,
    /// Always `gzip`.
    pub compression: String,
    /// Size of the `.sqlite.gz` file in bytes.
    pub bytes: u64,
    /// SHA-256 of the `.sqlite.gz` file, lower-case hex.
    pub sha256: String,
    /// Size of the decompressed SQLite file in bytes.
    pub sqlite_bytes: u64,
    /// SHA-256 of the decompressed SQLite file.
    pub sqlite_sha256: String,
    /// SHA-256 of a canonical dump of every table (import times left out):
    /// equal for two builds of the same files by the same parser, so CI
    /// republishes only when the data or the schema changed.
    pub content_sha256: String,
    /// Rows per table in the SQLite file.
    pub tables: BTreeMap<String, u64>,
    /// The monthly releases inside, newest first.
    pub releases: Vec<SnapshotRelease>,
}

/// One monthly release inside a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotRelease {
    pub as_of: NaiveDate,
    /// `xlsx` or `pdf`.
    pub format: String,
    /// SHA-256 of the IDX file the release was imported from (as in the
    /// mirror's `index.json`).
    pub sha256: String,
    /// The file's address on idx.co.id, when the builder knew it.
    pub source_url: Option<String>,
    /// Register lines stored (`releases.row_count`).
    pub rows: u64,
    /// Holder positions stored (rows of `holdings` for this release).
    pub holdings: u64,
    pub issuers: u64,
    pub entities: u64,
}

/// The snapshot a sync or status compared against, in short.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotSummary {
    pub file: String,
    pub generated_at: DateTime<Utc>,
    pub db_schema_version: i64,
    /// Oldest and newest release inside.
    pub oldest: NaiveDate,
    pub newest: NaiveDate,
    pub releases: u64,
    /// Download size of the `.sqlite.gz` file.
    pub bytes: u64,
}

/// What `idx ownership sync` did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncAction {
    /// The database was empty or missing; the snapshot was installed.
    Installed,
    /// The database was replaced by the snapshot (a `.bak` copy is kept).
    Updated,
    /// The database was kept and the missing months were imported from the
    /// mirror's monthly files.
    Imported,
    /// Nothing to do.
    UpToDate,
}

/// `idx ownership sync`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SyncReport {
    pub action: SyncAction,
    /// The local database file.
    pub database: String,
    /// The mirror's snapshot for this idx's schema version, `null` when the
    /// mirror has none.
    pub snapshot: Option<SnapshotSummary>,
    /// Months that came from the snapshot (installed or updated).
    pub installed: Vec<NaiveDate>,
    /// Months imported from the mirror's monthly files.
    pub imported: Vec<NaiveDate>,
    /// Months only the local database has (or re-imported locally with
    /// another file), kept because `--force` was not given.
    pub kept: Vec<NaiveDate>,
    /// Months the local database had and the snapshot replaced or dropped
    /// with `--force` (still in the `.bak` copy).
    pub dropped: Vec<NaiveDate>,
    /// Copy of the previous database, written before it was replaced.
    pub backup: Option<String>,
    /// Releases in the database after the sync.
    pub releases: u64,
    /// Newest release after the sync.
    pub newest: Option<NaiveDate>,
}

/// The snapshot a database was installed from (its `snapshot_info` table).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct InstalledSnapshot {
    pub generated_at: Option<String>,
    pub built_by: Option<String>,
    pub content_sha256: Option<String>,
    /// Newest release the snapshot held.
    pub newest: Option<NaiveDate>,
    /// Releases the snapshot held.
    pub releases: u64,
}

/// What the mirror holds, as `idx ownership status` read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MirrorState {
    /// Mirror base URL (`IDX_OWNERSHIP_MIRROR` or the GitHub release).
    pub mirror: String,
    /// `generated_at` of `index.json`; this and the next two are `null` when
    /// `index.json` could not be read (see `warnings`).
    pub index_generated_at: Option<String>,
    /// Monthly files listed in `index.json`.
    pub releases: Option<u64>,
    pub newest: Option<NaiveDate>,
    /// The snapshot for this idx's schema version, `null` when the mirror
    /// has none or it could not be read.
    pub snapshot: Option<SnapshotSummary>,
}

/// What `idx ownership sync` would do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncNeeded {
    /// Install the snapshot into an empty database.
    Install,
    /// Replace the database with a newer snapshot.
    Update,
    /// Keep the database and import missing months from the monthly files.
    Import,
    UpToDate,
}

/// `idx ownership status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StatusReport {
    /// The local database file.
    pub database: String,
    /// Whether the file exists (`status` never creates it).
    pub exists: bool,
    /// File size, `null` when it does not exist.
    pub bytes: Option<u64>,
    /// `PRAGMA user_version` of the file, `null` when it does not exist.
    pub db_schema_version: Option<i64>,
    /// The schema version this idx writes and installs.
    pub supported_schema_version: i64,
    /// Imported releases.
    pub releases: u64,
    pub oldest: Option<NaiveDate>,
    pub newest: Option<NaiveDate>,
    /// The snapshot the database was installed from, `null` when it was
    /// built by local imports.
    pub installed_snapshot: Option<InstalledSnapshot>,
    /// The mirror, `null` offline, with `IDX_OWNERSHIP_MIRROR=off` or when
    /// it could not be reached (see `warnings`).
    pub remote: Option<MirrorState>,
    /// Months on the mirror the local database lacks.
    pub missing: Vec<NaiveDate>,
    /// Months only the local database has (`[]` without `remote`).
    pub local_only: Vec<NaiveDate>,
    /// What `idx ownership sync` would do; `null` without `remote`.
    pub sync: Option<SyncNeeded>,
}
