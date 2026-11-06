//! Content-addressed file cache.
//!
//! Layout: `<cache_dir>/<first two hex of sha256(key)>/<sha256(key)>.json`.
//! Each file stores the raw provider body plus when it was fetched. Freshness
//! is decided by the *caller's* TTL at read time, so changing a TTL in config
//! takes effect immediately without clearing anything.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stored {
    pub key: String,
    pub fetched_at: DateTime<Utc>,
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone)]
pub struct Hit {
    pub stored: Stored,
    pub fresh: bool,
    pub age: Duration,
}

#[derive(Debug, Clone)]
pub struct Cache {
    dir: PathBuf,
}

impl Cache {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Cache { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, key: &str) -> PathBuf {
        let hash = hex::encode(Sha256::digest(key.as_bytes()));
        self.dir.join(&hash[..2]).join(format!("{hash}.json"))
    }

    /// Returns whatever is stored for `key`, flagged fresh or stale against `ttl`.
    pub fn get(&self, key: &str, ttl: Duration) -> Option<Hit> {
        let path = self.path_for(key);
        let raw = fs::read_to_string(&path).ok()?;
        let stored: Stored = match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(_) => {
                // Corrupt entry: drop it rather than fail forever.
                let _ = fs::remove_file(&path);
                return None;
            }
        };
        let age = (Utc::now() - stored.fetched_at)
            .to_std()
            .unwrap_or_default();
        Some(Hit {
            fresh: age <= ttl,
            age,
            stored,
        })
    }

    pub fn put(&self, key: &str, status: u16, body: &str) -> Result<()> {
        self.put_at(key, status, body, Utc::now())
    }

    /// Like [`put`](Self::put) with an explicit fetch time, so tests can
    /// seed a copy that is already older than its TTL.
    pub fn put_at(
        &self,
        key: &str,
        status: u16,
        body: &str,
        fetched_at: DateTime<Utc>,
    ) -> Result<()> {
        let stored = Stored {
            key: key.to_string(),
            fetched_at,
            status,
            body: body.to_string(),
        };
        self.write(key, &stored)
    }

    /// Typed convenience for small structured state (sessions, tokens).
    /// [`put`](Self::put) as if fetched at `fetched_at`, for tests of cache
    /// lifetimes.
    #[cfg(test)]
    pub fn put_fetched_at(
        &self,
        key: &str,
        status: u16,
        body: &str,
        fetched_at: DateTime<Utc>,
    ) -> Result<()> {
        let stored = Stored {
            key: key.to_string(),
            fetched_at,
            status,
            body: body.to_string(),
        };
        self.write(key, &stored)
    }

    pub fn get_json<T: DeserializeOwned>(&self, key: &str, ttl: Duration) -> Option<T> {
        let hit = self.get(key, ttl)?;
        if !hit.fresh {
            return None;
        }
        serde_json::from_str(&hit.stored.body).ok()
    }

    pub fn put_json<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let body = serde_json::to_string(value)?;
        self.put(key, 200, &body)
    }

    fn write(&self, key: &str, stored: &Stored) -> Result<()> {
        let path = self.path_for(key);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        fs::write(&tmp, serde_json::to_vec(stored)?)?;
        // rename is atomic on the same filesystem; fall back to copy on Windows quirks.
        if fs::rename(&tmp, &path).is_err() {
            fs::copy(&tmp, &path)?;
            let _ = fs::remove_file(&tmp);
        }
        Ok(())
    }

    /// Removes every cached file. Returns the number of files deleted.
    pub fn clear(&self) -> Result<usize> {
        let mut n = 0;
        if !self.dir.exists() {
            return Ok(0);
        }
        for shard in fs::read_dir(&self.dir)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for f in fs::read_dir(shard.path())? {
                let f = f?;
                if f.path().extension().is_some_and(|e| e == "json") {
                    fs::remove_file(f.path())?;
                    n += 1;
                }
            }
            let _ = fs::remove_dir(shard.path());
        }
        Ok(n)
    }

    /// (files, total bytes)
    pub fn stats(&self) -> Result<(usize, u64)> {
        let mut files = 0;
        let mut bytes = 0;
        if !self.dir.exists() {
            return Ok((0, 0));
        }
        for shard in fs::read_dir(&self.dir)? {
            let shard = shard?;
            if !shard.file_type()?.is_dir() {
                continue;
            }
            for f in fs::read_dir(shard.path())? {
                let f = f?;
                if f.path().extension().is_some_and(|e| e == "json") {
                    files += 1;
                    bytes += f.metadata()?.len();
                }
            }
        }
        Ok((files, bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_freshness() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        assert!(cache.get("k", Duration::from_secs(10)).is_none());
        cache.put("k", 200, "body").unwrap();
        let hit = cache.get("k", Duration::from_secs(10)).unwrap();
        assert!(hit.fresh);
        assert_eq!(hit.stored.body, "body");
        assert_eq!(cache.stats().unwrap().0, 1);
        assert_eq!(cache.clear().unwrap(), 1);
        assert!(cache.get("k", Duration::from_secs(10)).is_none());
    }

    #[test]
    fn typed_json() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct S {
            a: u8,
        }
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        cache.put_json("s", &S { a: 7 }).unwrap();
        assert_eq!(
            cache.get_json::<S>("s", Duration::from_secs(60)),
            Some(S { a: 7 })
        );
    }

    #[test]
    fn corrupt_entry_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let cache = Cache::new(dir.path());
        let p = cache.path_for("bad");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, "not json").unwrap();
        assert!(cache.get("bad", Duration::from_secs(1)).is_none());
        assert!(!p.exists());
    }
}
