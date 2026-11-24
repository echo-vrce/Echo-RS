"""Release notes and CHANGELOG.md for the release workflow (.github/workflows/release.yml).

    python tools/release_notes.py version [Cargo.toml | -]
        The package version in Cargo.toml (or stdin with `-`).

    python tools/release_notes.py notes VERSION --out notes.md [--update]
        The notes for VERSION. If CHANGELOG.md already has a `## VERSION`
        section (written by hand), that section is the notes. Otherwise the
        section is made from the commits since the previous release tag
        (`v*`, reachable from HEAD): each commit's subject, with the `- `
        lines of its message under it. With --update the new section is
        written into CHANGELOG.md, above the newest one.

Uses only git and the standard library.
"""

from __future__ import annotations

import argparse
import datetime
import re
import subprocess
import sys

DOWNLOADS = """### Downloads
- `echo-rust-server-windows.zip`: `echo-rust-server.exe` and `echo-server-launcher.exe` (no Visual C++ runtime needed)
- `echo-rust-server-linux.zip`: `echo-rust-server` and `echo-server-launcher` (x86_64, glibc 2.31 or newer)
"""

# Message lines that are not changes.
TRAILER = re.compile(r"^(co-authored-by|signed-off-by|reviewed-by|🤖 generated with)\b", re.I)
# A subject's own version prefix ("v0.2.7: ...").
VERSION_PREFIX = re.compile(r"^v?\d+\.\d+\.\d+\s*[:\-]\s*")


def git(*args: str) -> str:
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True, encoding="utf-8").stdout


def cargo_version(text: str) -> str:
    in_package = False
    for line in text.splitlines():
        line = line.strip()
        if line.startswith("["):
            in_package = line == "[package]"
        elif in_package:
            m = re.match(r'version\s*=\s*"([^"]+)"', line)
            if m:
                return m.group(1)
    sys.exit("no [package] version in Cargo.toml")


def version_key(v: str) -> tuple[int, ...]:
    return tuple(int(x) for x in re.findall(r"\d+", v)[:3])


def previous_tag(version: str) -> str | None:
    """The highest `v*` tag reachable from HEAD below this version (by
    number: several tags can share one commit, so dates don't order them)."""
    tags = [t for t in git("tag", "--merged", "HEAD", "--list", "v[0-9]*").split() if re.fullmatch(r"v\d+\.\d+\.\d+", t)]
    older = [t for t in tags if version_key(t) < version_key(version)]
    return max(older, key=version_key) if older else None


def existing_section(changelog: str, version: str) -> str | None:
    m = re.search(rf"^## {re.escape(version)}\b.*?$(.*?)(?=^## |\Z)", changelog, re.M | re.S)
    return m.group(1).strip("\n") if m else None


def commit_lines(since: str | None) -> list[str]:
    rng = f"{since}..HEAD" if since else "HEAD"
    log = git("log", rng, "--no-merges", "--format=%s%x1f%b%x1e")
    out: list[str] = []
    for entry in log.split("\x1e"):
        entry = entry.strip("\n")
        if not entry:
            continue
        subject, _, body = entry.partition("\x1f")
        subject = subject.strip()
        if "[skip ci]" in subject or subject.lower().startswith("chore(release)"):
            continue
        subject = VERSION_PREFIX.sub("", subject)
        # A message with no blank line after its title: git folds its "- "
        # lines into the subject.
        subject, *folded = re.split(r" - (?=\S)", subject)
        if not subject:
            continue
        out.append(f"- {subject[0].upper()}{subject[1:]}")
        out.extend(f"  - {f}" for f in folded)
        bullet = False
        for line in body.splitlines():
            if TRAILER.match(line.strip()):
                continue
            if line.startswith("- "):
                out.append("  " + line)
                bullet = True
            elif bullet and line.startswith("  ") and line.strip():
                out.append("  " + line)
            else:
                bullet = False
    return out


def notes(version: str, out_path: str, update: bool) -> None:
    try:
        changelog = open("CHANGELOG.md", encoding="utf-8").read()
    except FileNotFoundError:
        changelog = "# Changelog\n"
    body = existing_section(changelog, version)
    if body is None:
        since = previous_tag(version)
        lines = commit_lines(since)
        if not lines:
            lines = ["- Version bump."]
        body = "### Changes\n" + "\n".join(lines)
        if update:
            today = datetime.date.today().isoformat()
            section = f"## {version} ({today})\n\n{body}\n\n"
            m = re.search(r"^## ", changelog, re.M)
            changelog = changelog[: m.start()] + section + changelog[m.start():] if m else changelog.rstrip("\n") + "\n\n" + section
            open("CHANGELOG.md", "w", encoding="utf-8", newline="\n").write(changelog)
        print(f"notes for {version}: made from the commits since {since or 'the start'}", file=sys.stderr)
    else:
        print(f"notes for {version}: CHANGELOG.md's own section", file=sys.stderr)
    with open(out_path, "w", encoding="utf-8", newline="\n") as f:
        f.write(body.strip("\n") + "\n\n" + DOWNLOADS)


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    v = sub.add_parser("version")
    v.add_argument("file", nargs="?", default="Cargo.toml")
    n = sub.add_parser("notes")
    n.add_argument("version")
    n.add_argument("--out", required=True)
    n.add_argument("--update", action="store_true")
    a = ap.parse_args()
    if a.cmd == "version":
        text = sys.stdin.read() if a.file == "-" else open(a.file, encoding="utf-8").read()
        print(cargo_version(text))
    else:
        notes(a.version, a.out, a.update)


if __name__ == "__main__":
    main()
