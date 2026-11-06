#!/usr/bin/env python3
"""Mirror IDX's monthly "Pemegang Saham di Atas 1%" register to a GitHub release.

Run by .github/workflows/ownership.yml three times a day; safe to run by hand.

1. Fetch the IDX "Data Kepemilikan Saham" page through a browser-impersonating
   client (curl_cffi). idx.co.id answers 403 to ordinary HTTP clients and
   blocks cloud networks outright (GitHub-hosted runners, Cloudflare Workers),
   so this only succeeds from an ordinary connection: a self-hosted runner or
   a maintainer's machine. From a blocked network the run still rebuilds the
   parts from the files already mirrored (so a newer idx parser reaches the
   web terminal), and fails once the newest register is STALE_AFTER_DAYS old.
2. Let `idx ownership discover` parse the saved page, so the CLI and the mirror
   share one parser.
3. Reuse every file already on the mirror release; download only new or
   re-published files from idx.co.id and check they really are XLSX/PDF.
4. Import all files, oldest first, into a fresh SQLite with
   `idx ownership import --file`. Any parse failure fails the run, so a broken
   file never reaches the mirror.
5. Export each release as JSON parts (what the web terminal loads into D1, one
   part per minute): one row per holder and issuer, the holder's register
   lines summed by idx (`accounts`). Write index.json with each release's
   register `lines`, `positions`, `other_date_rows` and `controller_column`;
   `rows` stays the exported row count the Worker checks. Upload only what
   changed, index.json last, and remove assets the new index no longer
   references.

The release is a prerelease so it never becomes the repository's "latest"
release (install.sh resolves that). Requires the gh CLI with GH_TOKEN,
curl_cffi, and a built idx binary.
"""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import os
import shutil
import sqlite3
import subprocess
import sys
import time
from pathlib import Path

PAGE_URL = "https://www.idx.co.id/id/perusahaan-tercatat/data-kepemilikan-saham/"
SCHEMA = "idx.ownership.mirror.v1"
IMPERSONATE = ["chrome", "chrome131", "edge", "safari"]
STALE_AFTER_DAYS = 45
NOTES = """Automatic mirror of the KSEI "Pemegang Saham di Atas 1%" register published monthly by \
the Indonesia Stock Exchange at https://www.idx.co.id/id/perusahaan-tercatat/data-kepemilikan-saham/ .

Files are byte-for-byte copies of the IDX workbooks; `index.json` lists each release with its \
original IDX URL and SHA-256, and the `*.partNN.json` files are the parsed rows the web terminal \
at https://ihsg.midoriresearch.com loads. Refreshed by .github/workflows/ownership.yml. \
`idx ownership import --latest` uses this mirror when idx.co.id refuses the request.

Source: Indonesia Stock Exchange (IDX) / KSEI. Holders below 1% are not published."""


def log(msg: str) -> None:
    print(msg, flush=True)


def warn(msg: str) -> None:
    # GitHub Actions annotation when running in CI, plain text otherwise.
    print(f"::warning::{msg}" if os.environ.get("GITHUB_ACTIONS") else f"warning: {msg}", flush=True)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def gh(*args: str, check: bool = True, capture: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(["gh", *args], check=check, capture_output=capture, text=True)


# ---- IDX access ---------------------------------------------------------------


def fetch(url: str, referer: str | None = None, timeout: int = 90) -> bytes:
    from curl_cffi import requests  # imported lazily so --help works without it

    last = None
    for attempt, target in enumerate(IMPERSONATE * 2):
        try:
            headers = {"Referer": referer} if referer else {}
            r = requests.get(url, impersonate=target, timeout=timeout, headers=headers)
            body = r.content
            challenged = b"Just a moment" in body[:4096] or b"cf-chl" in body[:4096]
            if r.status_code == 200 and not challenged:
                return body
            last = f"HTTP {r.status_code}{' (challenge page)' if challenged else ''} as {target}"
        except Exception as e:  # network errors, TLS resets
            last = f"{type(e).__name__}: {e} as {target}"
        time.sleep(min(2 + attempt * 2, 10))
    raise RuntimeError(f"{url}: {last}")


def discover(idx: str, page: Path, work: Path) -> list[dict]:
    env = dict(os.environ)
    env["IDX_OWNERSHIP_PAGE_URL"] = page.resolve().as_uri()
    env["IDX_OWNERSHIP_MIRROR"] = "off"
    env["IDX_OWNERSHIP_DB"] = str(work / "discover.sqlite")
    out = subprocess.run([idx, "-o", "json", "ownership", "discover"], env=env, capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(f"idx ownership discover failed: {out.stderr.strip()}")
    return json.loads(out.stdout)["data"]


def looks_valid(name: str, data: bytes) -> bool:
    if name.endswith(".xlsx"):
        return data[:4] == b"PK\x03\x04"
    if name.endswith(".pdf"):
        return data[:5] == b"%PDF-"
    return False


# ---- mirror release -------------------------------------------------------------


def release_assets(repo: str, tag: str) -> dict[str, int] | None:
    r = gh("release", "view", tag, "-R", repo, "--json", "assets", check=False)
    if r.returncode != 0:
        return None
    return {a["name"]: a.get("size", 0) for a in json.loads(r.stdout)["assets"]}


def download_asset(repo: str, tag: str, name: str, dest: Path) -> Path:
    dest.mkdir(parents=True, exist_ok=True)
    gh("release", "download", tag, "-R", repo, "-p", name, "-D", str(dest), "--clobber")
    return dest / name


# ---- export -----------------------------------------------------------------------

PART_FIELDS = (
    "symbol", "issuer_name", "entity_key", "holder", "holder_raw", "investor_type", "local_foreign",
    "nationality", "domicile", "shares_scripless", "shares_scrip", "shares_total", "pct",
    "accounts", "controller_or_affiliate",
)


def export_parts(db: Path, out: Path, part_rows: int) -> dict[str, dict]:
    """Writes `ksei-above1-<as_of>.partNN.json` files; returns per-as_of stats.

    Reads the positions table of a database written by idx 0.8.2 or later:
    one row per (symbol, entity), so a holder's accounts never straddle two
    parts and D1's (as_of, symbol, entity_key) key cannot collapse them.
    """
    con = sqlite3.connect(db)
    stats: dict[str, dict] = {}
    releases = con.execute(
        "SELECT id, as_of, row_count, positions, other_date_rows, controller_column FROM releases ORDER BY as_of"
    ).fetchall()
    for rid, as_of, lines, positions, other_date_rows, controller_column in releases:
        rows = con.execute(
            """SELECT h.symbol, h.issuer_name, e.entity_key, e.display_name, h.holder_raw, h.investor_type,
                      h.local_foreign, h.nationality, h.domicile, h.shares_scripless, h.shares_scrip,
                      h.shares_total, h.pct, h.accounts, h.controller_or_affiliate
               FROM holdings h JOIN entities e ON e.id = h.entity_id
               WHERE h.release_id = ? ORDER BY h.symbol, e.entity_key""",
            (rid,),
        ).fetchall()
        issuers = len({r[0] for r in rows})
        entities = len({r[2] for r in rows})
        parts = []
        for k in range(0, len(rows), part_rows):
            chunk = [dict(zip(PART_FIELDS, r)) for r in rows[k : k + part_rows]]
            name = f"ksei-above1-{as_of}.part{k // part_rows + 1:02d}.json"
            data = json.dumps(chunk, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
            (out / name).write_bytes(data)
            parts.append({"file": name, "sha256": sha256(data), "rows": len(chunk)})
        stats[as_of] = {
            "rows": len(rows), "issuers": issuers, "entities": entities, "parts": parts,
            "lines": lines, "positions": positions, "other_date_rows": other_date_rows,
            "controller_column": None if controller_column is None else bool(controller_column),
        }
    con.close()
    return stats


def comparable(index: dict | None) -> str:
    if not index:
        return ""
    return json.dumps({k: v for k, v in index.items() if k not in ("generated_at", "page_fetched_at")}, sort_keys=True)


# ---- main -----------------------------------------------------------------------------


def main() -> int:
    ap = argparse.ArgumentParser(description=(__doc__ or "").split("\n\n")[0])
    ap.add_argument("--idx", required=True, help="path to the idx binary")
    ap.add_argument("--repo", required=True, help="owner/name of the GitHub repository")
    ap.add_argument("--tag", default="ownership-data")
    ap.add_argument("--work", default="build/ownership")
    ap.add_argument("--part-rows", type=int, default=1000)
    ap.add_argument("--dry-run", action="store_true", help="build everything, upload nothing")
    a = ap.parse_args()
    a.idx = str(Path(a.idx).resolve())

    work = Path(a.work)
    if work.exists():
        shutil.rmtree(work)
    raw_dir, out_dir, prev_dir = work / "raw", work / "out", work / "prev"
    for d in (raw_dir, out_dir, prev_dir):
        d.mkdir(parents=True)

    assets = release_assets(a.repo, a.tag)
    prev: dict | None = None
    if assets and "index.json" in assets:
        loaded: dict = json.loads(download_asset(a.repo, a.tag, "index.json", prev_dir).read_text("utf-8"))
        log(f"mirror has {len(loaded['releases'])} releases")
        prev = loaded
    prev_by_date = {r["as_of"]: r for r in (prev or {}).get("releases", [])}

    # 1-2. Discover on idx.co.id.
    page_ok = False
    unreachable = ""
    discovered: list[dict] = []
    try:
        page = work / "page.html"
        page.write_bytes(fetch(PAGE_URL))
        discovered = discover(a.idx, page, work)
        page_ok = True
        log(f"idx.co.id lists {len(discovered)} releases: {', '.join(d['as_of'] for d in discovered)}")
    except Exception as e:
        if not prev:
            print(f"error: idx.co.id unreachable and no mirror to fall back on: {e}", file=sys.stderr)
            return 1
        # Keep going with the files already mirrored: the parts are rebuilt with
        # this idx, so a parser fix reaches the web terminal without idx.co.id.
        unreachable = str(e)
        warn(f"idx.co.id unreachable from this runner ({e}); rebuilding the parts from the mirrored files only")

    # 3. Gather files.
    wanted: dict[str, dict] = {}
    for d in discovered:
        wanted[d["as_of"]] = {"as_of": d["as_of"], "title": d["title"], "format": d["format"], "source_url": d["url"]}
    for as_of, r in prev_by_date.items():  # keep months IDX has dropped from its page
        wanted.setdefault(as_of, {k: r.get(k) for k in ("as_of", "title", "format", "source_url")})

    files: dict[str, Path] = {}
    for as_of, w in sorted(wanted.items()):
        name = f"ksei-above1-{as_of}.{w['format']}"
        p = prev_by_date.get(as_of)
        if p and p.get("source_url") == w["source_url"] and assets and name in assets:
            path = download_asset(a.repo, a.tag, name, raw_dir)
            if sha256(path.read_bytes()) != p["sha256"]:
                print(f"error: mirror asset {name} does not match its recorded SHA-256", file=sys.stderr)
                return 1
        else:
            if not page_ok:
                continue
            try:
                data = fetch(w["source_url"], referer=PAGE_URL, timeout=180)
            except Exception as e:
                warn(f"{as_of}: download failed ({e}); skipped this run")
                continue
            if not looks_valid(name, data):
                warn(f"{as_of}: {w['source_url']} did not return a {w['format']} file; skipped")
                continue
            path = raw_dir / name
            path.write_bytes(data)
            log(f"{as_of}: downloaded {len(data):,} bytes from idx.co.id")
        files[as_of] = path

    # 4. Import into a fresh database.
    db = work / "ownership.sqlite"
    env = dict(os.environ, IDX_OWNERSHIP_DB=str(db), IDX_OWNERSHIP_MIRROR="off")
    imported: dict[str, dict] = {}
    for as_of, path in sorted(files.items()):
        r = subprocess.run([a.idx, "-o", "json", "ownership", "import", "--file", str(path)], env=env, capture_output=True, text=True)
        if r.returncode != 0:
            print(f"error: {path.name} did not import: {r.stderr.strip()}", file=sys.stderr)
            return 1
        rep = json.loads(r.stdout)["data"][0]["release"]
        if rep["as_of"] != as_of:
            print(f"error: {path.name} contains as-of {rep['as_of']}, expected {as_of}", file=sys.stderr)
            return 1
        imported[as_of] = rep
        log(f"{as_of}: {rep['rows']:,} lines, {rep.get('positions') or 0:,} positions, {rep['issuers']} issuers, {rep['entities']:,} holders")

    # 5. Export and index.
    stats = export_parts(db, out_dir, a.part_rows)
    now = dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")
    releases = []
    for as_of in sorted(imported, reverse=True):
        w, path, st = wanted[as_of], files[as_of], stats[as_of]
        shutil.copy2(path, out_dir / path.name)
        data = path.read_bytes()
        releases.append({
            "as_of": as_of, "title": w["title"], "format": w["format"], "source_url": w["source_url"],
            "file": path.name, "sha256": sha256(data), "bytes": len(data),
            "rows": st["rows"], "issuers": st["issuers"], "entities": st["entities"],
            "lines": st["lines"], "positions": st["positions"], "other_date_rows": st["other_date_rows"],
            "controller_column": st["controller_column"], "parts": st["parts"],
        })
    index = {
        "schema": SCHEMA, "generated_at": now, "source_page": PAGE_URL,
        "page_fetched_at": now if page_ok else (prev or {}).get("page_fetched_at"),
        "releases": releases,
    }
    (out_dir / "index.json").write_text(json.dumps(index, ensure_ascii=False, indent=2) + "\n", "utf-8")

    summary = ["| as of | lines | rows | issuers | holders | parts | sha256 |", "| --- | ---: | ---: | ---: | ---: | ---: | --- |"]
    summary += [f"| {r['as_of']} | {r['lines'] or 0:,} | {r['rows']:,} | {r['issuers']} | {r['entities']:,} | {len(r['parts'])} | `{r['sha256'][:12]}` |" for r in releases]
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as f:
            f.write(f"### KSEI >1% register mirror\n\nidx.co.id reachable: {'yes' if page_ok else 'no'}\n\n" + "\n".join(summary) + "\n")
    log("\n".join(summary))

    if comparable(prev) == comparable(index):
        log("mirror already up to date; nothing to upload")
        return stale_exit(unreachable, releases)
    if a.dry_run:
        log(f"dry run: would upload to {a.repo}@{a.tag}: {', '.join(sorted(p.name for p in out_dir.iterdir()))}")
        return stale_exit(unreachable, releases)

    if assets is None:
        gh("release", "create", a.tag, "-R", a.repo, "--prerelease", "--latest=false",
           "--title", "Ownership data: KSEI >1% register mirror", "--notes", NOTES)
        assets = {}
    prev_sha = {r["file"]: r["sha256"] for r in (prev or {}).get("releases", [])}
    prev_sha.update({p["file"]: p["sha256"] for r in (prev or {}).get("releases", []) for p in r["parts"]})
    upload = []
    for p in sorted(out_dir.iterdir()):
        if p.name == "index.json":
            continue
        if p.name not in assets or prev_sha.get(p.name) != sha256(p.read_bytes()):
            upload.append(str(p))
    for i in range(0, len(upload), 20):
        gh("release", "upload", a.tag, "-R", a.repo, "--clobber", *upload[i : i + 20])
    gh("release", "upload", a.tag, "-R", a.repo, "--clobber", str(out_dir / "index.json"))
    keep = {p.name for p in out_dir.iterdir()}
    for stale in sorted(set(assets) - keep):
        gh("release", "delete-asset", a.tag, stale, "-R", a.repo, "-y")
    log(f"uploaded {len(upload)} files and index.json; removed {len(set(assets) - keep)} stale assets")
    return stale_exit(unreachable, releases)


def stale_exit(unreachable: str, releases: list[dict]) -> int:
    """1 when idx.co.id was unreachable and the newest register is too old, else 0."""
    if not unreachable or not releases:
        return 0
    newest = max(r["as_of"] for r in releases)
    age = (dt.date.today() - dt.date.fromisoformat(newest)).days
    if age > STALE_AFTER_DAYS:
        # IDX publishes each month-end register within a few weeks; past this
        # the mirror is genuinely behind, so make the run fail visibly.
        print(f"error: idx.co.id unreachable ({unreachable}) and the newest mirrored register ({newest}) is {age} days old. "
              "Run scripts/ownership_mirror.py from a connection idx.co.id accepts (see docs/ownership.md).", file=sys.stderr)
        return 1
    warn(f"idx.co.id unreachable from this runner; newest register {newest} ({age} days old)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
