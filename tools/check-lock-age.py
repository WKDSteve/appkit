#!/usr/bin/env python3
"""Fail when a locked crate version is younger than N days.

A freshly published version is the window in which a hijacked crate does
its harm (concept risk T7: a bad release was online for 86 minutes). Cargo
has no "minimum release age" setting, so this reads Cargo.lock and asks
crates.io when each registry version was published.

    python tools/check-lock-age.py [--min-days 7] [--lock Cargo.lock]

Exit 0: every version is old enough. Exit 1: the young ones are listed;
take an older version with `cargo update -p <crate> --precise <version>`.
Exit 2: the check itself could not run (network, API); that is not a pass.

crates.io asks for one request per second and an identifying User-Agent. When a publication date has been read
once, it is kept in a cache file (it never changes): `--cache`, else `$LOCK_AGE_CACHE`, else on a runner
`$RUNNER_WORKSPACE/lock-age-cache.json` (the repository's folder of the runner, beside the checkout, which
actions/checkout empties with `git clean -ffdx`, `target/` included), else `target/lock-age-cache.json` next to the
lock file. A run then only asks for the versions it has not seen, which keeps a runner below the rate limit of
crates.io.

The cache belongs to one workspace, not to the account: a file every job of every repository on a self-hosted runner
can write (`~/.cache`) would let the build script of any dependency anywhere enter an old date for a young version and
open this gate. What is left: a build script of this very repository can still write its cache; a hosted runner
starts without it. A date without a time zone is no date this tool wrote: it is asked again.

A security fix can be younger than the week: the advisory (cargo audit, cargo deny) fails on the old version, this
check on the new one. `lock-age-allow.json` beside the lock file (`--allow` names another) lets such a version through
for a few days, each entry with its reason and an end:

    [{"crate": "url", "version": "2.5.9", "advisory": "RUSTSEC-2026-0592",
      "reason": "fixes the advisory; no older fixed version", "until": "2026-11-27"}]

An entry holds up to and including its day `until` and is printed on every run. Once that day is over, the entry fails
the check (exit 1) until it is taken out, whether or not the version is still young: an allowance does not stay behind
unnoticed. A file that is not such a list fails with exit 2.
"""

import argparse
import json
import os
import re
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone

USER_AGENT = "appkit-lock-age-check (https://github.com/WKDSteve/appkit)"


def locked_registry_packages(lock_text):
    """Yield (name, version) of every package that comes from a registry."""
    for block in lock_text.split("[[package]]")[1:]:
        name = re.search(r'^name = "([^"]+)"', block, re.M)
        version = re.search(r'^version = "([^"]+)"', block, re.M)
        source = re.search(r'^source = "registry\+', block, re.M)
        if name and version and source:
            yield name.group(1), version.group(1)


# Answers that mean "ask again later": 429 (too many requests) and 503 (service unavailable).
RETRY_CODES = (429, 503)
# Waits after such an answer, in seconds, when crates.io names none: about four minutes in all.
# A run asks for every crate of the lock file, one a second, and crates.io can still ask for a pause minutes into it.
BACKOFF = (10, 20, 40, 60, 60, 60)
# The longest pause taken from a Retry-After header.
MAX_RETRY_AFTER = 120


def retry_after(err, default):
    """The pause crates.io asks for (Retry-After, in seconds) within MAX_RETRY_AFTER, else `default`."""
    value = (err.headers or {}).get("Retry-After") if hasattr(err, "headers") else None
    try:
        seconds = int(str(value).strip())
    except (TypeError, ValueError):
        return default
    return max(1, min(seconds, MAX_RETRY_AFTER))


def published(name, version, sleep=time.sleep):
    """The moment a version was published (UTC), from the crates.io API. An answer 429 is waited out (its
    Retry-After, else BACKOFF) and asked again, a 503 too; anything else, or 429/503 after the last pause, is raised."""
    url = f"https://crates.io/api/v1/crates/{name}/{version}"
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
    for attempt in range(len(BACKOFF) + 1):
        try:
            with urllib.request.urlopen(request, timeout=20) as response:
                text = json.load(response)["version"]["created_at"]
            return datetime.fromisoformat(text.replace("Z", "+00:00"))
        except urllib.error.HTTPError as err:
            if err.code in RETRY_CODES and attempt < len(BACKOFF):
                sleep(retry_after(err, BACKOFF[attempt]))
                continue
            raise
    raise RuntimeError("unreachable")


def default_cache(lock="Cargo.lock"):
    """Where the publication dates are kept between runs: `$LOCK_AGE_CACHE`, else the runner's folder of this
    repository (`$RUNNER_WORKSPACE`, it outlives the checkout), else `target/` of the lock's workspace."""
    if os.environ.get("LOCK_AGE_CACHE"):
        return os.environ["LOCK_AGE_CACHE"]
    if os.environ.get("RUNNER_WORKSPACE"):
        return os.path.join(os.environ["RUNNER_WORKSPACE"], "lock-age-cache.json")
    return os.path.join(os.path.dirname(os.path.abspath(lock)), "target", "lock-age-cache.json")


def load_cache(path):
    """The cache as {"name version": "ISO time"}; empty when missing or unreadable (it is only a cache)."""
    try:
        with open(path, encoding="utf-8") as handle:
            data = json.load(handle)
        return {k: v for k, v in data.items() if isinstance(k, str) and isinstance(v, str)}
    except (OSError, ValueError, AttributeError):
        return {}


def save_cache(path, cache):
    """Writes the cache through a temporary file, so a run that is cut off leaves the old one."""
    try:
        os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
        temp = f"{path}.{os.getpid()}.tmp"
        with open(temp, "w", encoding="utf-8") as handle:
            json.dump(cache, handle, sort_keys=True)
        os.replace(temp, path)
    except OSError as err:
        print(f"cache not written ({path}): {err}", file=sys.stderr)


def published_cached(name, version, cache, ask=None):
    """The publication date from the cache, or asked (and then kept)."""
    key = f"{name} {version}"
    if key in cache:
        try:
            moment = datetime.fromisoformat(cache[key])
            # a date without a time zone was not written here (and cannot be compared with now): ask again
            if moment.tzinfo is not None:
                return moment, False
        except ValueError:
            pass
    moment = (ask or published)(name, version)
    cache[key] = moment.isoformat()
    return moment, True


def too_young(age, min_days):
    """Is a version of this age (the time since it was published) younger than `min_days` days?

    Elapsed time, not calendar days: a version published late on one day is not a day older at midnight.
    """
    return age < timedelta(days=min_days)


ADVISORY = re.compile(r"^RUSTSEC-\d{4}-\d{4}$")
DAY = re.compile(r"^\d{4}-\d{2}-\d{2}$")
ALLOW_FIELDS = ("crate", "version", "advisory", "reason", "until")


def load_allow(path):
    """The entries of the allow-list of security fixes, or [] without the file. A file that is not a list of entries
    with every field (`crate`, `version`, an advisory id `RUSTSEC-YYYY-NNNN`, a `reason`, an `until` day) raises
    ValueError: a broken list lets nothing through by mistake."""
    if not os.path.exists(path):
        return []
    with open(path, encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, list):
        raise ValueError("not a list of entries")
    entries = []
    for i, entry in enumerate(data):
        if not isinstance(entry, dict) or not all(isinstance(entry.get(k), str) and entry[k].strip() for k in ALLOW_FIELDS):
            raise ValueError(f"entry {i}: needs {', '.join(ALLOW_FIELDS)}")
        if not ADVISORY.match(entry["advisory"]):
            raise ValueError(f"entry {i}: advisory {entry['advisory']!r} is no RUSTSEC id")
        if not DAY.match(entry["until"]):
            raise ValueError(f"entry {i}: until {entry['until']!r} is no day YYYY-MM-DD")
        datetime.strptime(entry["until"], "%Y-%m-%d")
        entries.append(entry)
    return entries


def split_allow(entries, today):
    """The entries that hold on `today` (a date; up to and including `until`) as {(crate, version): entry}, and those
    whose day is over."""
    active, expired = {}, []
    for entry in entries:
        if today.isoformat() <= entry["until"]:
            active[(entry["crate"], entry["version"])] = entry
        else:
            expired.append(entry)
    return active, expired


def default_allow(lock):
    """The allow-list beside the lock file."""
    return os.path.join(os.path.dirname(os.path.abspath(lock)), "lock-age-allow.json")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--min-days", type=int, default=7)
    parser.add_argument("--lock", default="Cargo.lock")
    parser.add_argument("--cache", default=None, help="file of known publication dates")
    parser.add_argument("--allow", default=None, help="allow-list of security fixes (default: beside the lock file)")
    args = parser.parse_args()
    cache_path = args.cache or default_cache(args.lock)
    cache = load_cache(cache_path)
    allow_path = args.allow or default_allow(args.lock)
    try:
        allowed, expired = split_allow(load_allow(allow_path), datetime.now(timezone.utc).date())
    except (OSError, ValueError) as err:
        print(f"cannot read the allow-list {allow_path}: {err}", file=sys.stderr)
        return 2

    with open(args.lock, encoding="utf-8") as handle:
        packages = sorted(set(locked_registry_packages(handle.read())))
    now = datetime.now(timezone.utc)

    young = []
    let_through = []
    asked = 0
    for name, version in packages:
        try:
            moment, fresh = published_cached(name, version, cache)
        except Exception as err:  # noqa: BLE001 - any failure means "not checked"
            save_cache(cache_path, cache)
            print(f"cannot check {name} {version}: {err}", file=sys.stderr)
            return 2
        age = now - moment
        if too_young(age, args.min_days):
            entry = allowed.get((name, version))
            if entry is not None:
                let_through.append((age.total_seconds() / 86400, name, version, entry))
            else:
                young.append((age.total_seconds() / 86400, name, version))
        if fresh:
            asked += 1
            time.sleep(1.05)
    save_cache(cache_path, cache)

    print(f"{len(packages)} registry crates checked ({asked} asked, the rest from the cache), minimum age {args.min_days} days")
    for age, name, version in sorted(young):
        print(f"  too young: {name} {version} ({age:.1f} days)")
    for age, name, version, entry in sorted(let_through, key=lambda x: x[:3]):
        print(
            f"  allowed while young: {name} {version} ({age:.1f} days), {entry['advisory']} until {entry['until']}:"
            f" {entry['reason']}"
        )
    for entry in expired:
        print(
            f"  expired allowance: {entry['crate']} {entry['version']} ({entry['advisory']}, until {entry['until']}):"
            f" take it out of {allow_path}"
        )
    return 1 if young or expired else 0


if __name__ == "__main__":
    sys.exit(main())
