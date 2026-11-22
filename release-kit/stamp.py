#!/usr/bin/env python3
"""Stamp an install script of the release kit for one app and one channel.

The scripts in release-kit/scripts carry a block between the lines `release-kit:stamp:begin` and
`release-kit:stamp:end` that holds empty values (`$StampRepo = ''` in install.ps1,
`STAMP_REPO=""` in install.sh). The publish action fills them for the release it attaches the
script to, so a script downloaded from a release knows its repository, its app and its channel
(that release, or the rolling build `latest`) and runs without parameters.

    stamp.py SOURCE OUT --repo OWNER/NAME --channel latest|nightly|stable|vX.Y.Z --app NAME --kind KIND
             --program NAME [--release-asset NAME] [--rolling-asset NAME]

A value may hold letters, digits and `._-/{}` only: it ends up in a URL, a file name and a line of
a script, and nothing in it may need quoting.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

BEGIN = "release-kit:stamp:begin"
END = "release-kit:stamp:end"

# The kinds each script can install.
KINDS = {
    ".ps1": ("hub", "nsis", "inno"),
    ".sh": ("hub", "appimage"),
}

# Option name -> variable of the script (PowerShell, sh).
FIELDS = {
    "repo": ("StampRepo", "STAMP_REPO"),
    "channel": ("StampChannel", "STAMP_CHANNEL"),
    "app": ("StampApp", "STAMP_APP"),
    "kind": ("StampKind", "STAMP_KIND"),
    "program": ("StampProgram", "STAMP_PROGRAM"),
    "release_asset": ("StampReleaseAsset", "STAMP_RELEASE_ASSET"),
    "rolling_asset": ("StampRollingAsset", "STAMP_ROLLING_ASSET"),
}

ROLLING = ("latest", "nightly", "stable")
PLAIN = re.compile(r"^[A-Za-z0-9._{}/-]*$")
REPO = re.compile(r"^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$")
CHANNEL = re.compile(r"^(latest|nightly|stable|v[0-9]+\.[0-9]+\.[0-9]+[A-Za-z0-9.-]*)$")
WORD = re.compile(r"^[A-Za-z0-9._-]+$")


class StampError(Exception):
    pass


def check(values: dict[str, str], suffix: str) -> None:
    """Refuse a value that is not what its field may hold."""
    for name, value in values.items():
        if not PLAIN.fullmatch(value):
            raise StampError(f"--{name.replace('_', '-')} has a character that is not allowed: {value!r}")
    if not REPO.fullmatch(values["repo"]):
        raise StampError("--repo must be OWNER/NAME")
    if not CHANNEL.fullmatch(values["channel"]):
        raise StampError("--channel must be latest, nightly, stable or vX.Y.Z")
    if not WORD.fullmatch(values["app"]):
        raise StampError("--app must be a plain word")
    if values["kind"] not in KINDS[suffix]:
        raise StampError(f"--kind of a {suffix} script is one of {', '.join(KINDS[suffix])}")
    # A hub archive and an AppImage need the name of the program; a setup names itself.
    needs_program = values["kind"] in ("hub", "appimage")
    if (needs_program or values["program"]) and not WORD.fullmatch(values["program"]):
        raise StampError("--program must be a plain file name")
    asset = values["rolling_asset"] if values["channel"] in ROLLING else values["release_asset"]
    if asset == "":
        which = "--rolling-asset" if values["channel"] in ROLLING else "--release-asset"
        raise StampError(f"{which} is needed for the channel {values['channel']}")
    for name in ("release_asset", "rolling_asset"):
        if "/" in values[name]:
            raise StampError(f"--{name.replace('_', '-')} is a file name, not a path")


def stamp(text: str, suffix: str, values: dict[str, str]) -> str:
    """Return the script with the stamp block filled; every field must be there exactly once."""
    if suffix not in KINDS:
        raise StampError(f"cannot stamp a {suffix or 'nameless'} file (only .ps1 and .sh)")
    check(values, suffix)
    lines = text.splitlines(keepends=True)
    try:
        start = next(i for i, line in enumerate(lines) if BEGIN in line)
        end = next(i for i, line in enumerate(lines) if END in line and i > start)
    except StopIteration:
        raise StampError("the script has no stamp block") from None
    column = 0 if suffix == ".ps1" else 1
    seen: set[str] = set()
    for i in range(start + 1, end):
        for field, names in FIELDS.items():
            var = names[column]
            if suffix == ".ps1":
                pattern = re.compile(r"^(\s*\$" + var + r" = )''(\r?\n?)$")
                quote = "'"
            else:
                pattern = re.compile(r"^(\s*" + var + r'=)""(\r?\n?)$')
                quote = '"'
            match = pattern.match(lines[i])
            if match:
                if field in seen:
                    raise StampError(f"{var} is twice in the stamp block")
                seen.add(field)
                lines[i] = f"{match.group(1)}{quote}{values[field]}{quote}{match.group(2)}"
    missing = [FIELDS[f][column] for f in FIELDS if f not in seen]
    if missing:
        raise StampError(f"the stamp block has no empty {', '.join(missing)} (stamped already?)")
    return "".join(lines)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("source", type=Path)
    parser.add_argument("out", type=Path)
    parser.add_argument("--repo", required=True)
    parser.add_argument("--channel", required=True)
    parser.add_argument("--app", required=True)
    parser.add_argument("--kind", required=True)
    parser.add_argument("--program", required=True)
    parser.add_argument("--release-asset", default="")
    parser.add_argument("--rolling-asset", default="")
    args = parser.parse_args(argv)
    values = {field: getattr(args, field) for field in FIELDS}
    try:
        # Bytes in, bytes out: the line endings and the encoding of the script stay as they are.
        text = args.source.read_bytes().decode("utf-8")
        out = stamp(text, args.source.suffix, values)
    except (StampError, OSError, UnicodeDecodeError) as err:
        print(f"stamp.py: {err}", file=sys.stderr)
        return 1
    args.out.write_bytes(out.encode("utf-8"))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
