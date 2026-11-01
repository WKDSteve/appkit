#!/usr/bin/env python3
"""Write the `version.json` that goes on every build of the release kit.

A hub that lists apps (hub-desktop) shows "update available" by comparing the version of the installed
app with the newest build. It reads this one small file, from the same place for every app
(`releases/download/<channel>/version.json`), instead of the release API of each app.

    version_json.py OUT --app NAME --channel latest|vX.Y.Z [--version V] [--commit SHA] [--date ISO]

The document (schema `release-kit-version/1`, a later minor version only adds keys):

    {"schema": "release-kit-version/1", "app": "demo", "channel": "latest", "version": "1.2.3",
     "commit": "<40 hex>", "date": "2026-11-01T12:00:00Z"}

`version` is the version the program reports about itself (`null` when a rolling build is not told it),
`commit` is the commit of the files (`null` for a release, whose tag the kit does not resolve), `date` is
when the build was published (UTC). A release carries the version of its tag; a different `--version` is
refused, so the file can never say something else than the tag.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

SCHEMA = "release-kit-version/1"

WORD = re.compile(r"^[A-Za-z0-9._-]+$")
CHANNEL = re.compile(r"^(latest|v([0-9]+\.[0-9]+\.[0-9]+[A-Za-z0-9.-]*))$")
VERSION = re.compile(r"^[0-9]+\.[0-9]+\.[0-9]+[A-Za-z0-9.+-]*$")
COMMIT = re.compile(r"^[0-9a-f]{40}$")
DATE = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z$")


class VersionError(Exception):
    pass


def make(app: str, channel: str, version: str = "", commit: str = "", date: str = "") -> dict[str, object]:
    """The document for one build; every value is checked, none is trusted."""
    if not WORD.fullmatch(app):
        raise VersionError("--app must be a plain word")
    tag = CHANNEL.fullmatch(channel)
    if not tag:
        raise VersionError("--channel must be latest or vX.Y.Z")
    if version and not VERSION.fullmatch(version):
        raise VersionError(f"--version must be X.Y.Z, got {version!r}")
    if tag.group(2):
        if version and version != tag.group(2):
            raise VersionError(f"--version {version} is not the version of the tag {channel}")
        version = tag.group(2)
    if commit and not COMMIT.fullmatch(commit):
        raise VersionError("--commit must be a full commit id")
    date = date or datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
    if not DATE.fullmatch(date):
        raise VersionError("--date must be YYYY-MM-DDTHH:MM:SSZ")
    return {
        "schema": SCHEMA,
        "app": app,
        "channel": channel,
        "version": version or None,
        "commit": commit or None,
        "date": date,
    }


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("out", type=Path)
    parser.add_argument("--app", required=True)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--version", default="")
    parser.add_argument("--commit", default="")
    parser.add_argument("--date", default="")
    args = parser.parse_args(argv)
    try:
        doc = make(args.app, args.channel, args.version, args.commit, args.date)
    except VersionError as err:
        print(f"version_json.py: {err}", file=sys.stderr)
        return 1
    args.out.write_bytes((json.dumps(doc, indent=2) + "\n").encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
