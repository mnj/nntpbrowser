#!/usr/bin/env python3
"""Stable release versions without modifying resolved dependency versions."""
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
STABLE = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\Z")


def version(text):
    match = STABLE.fullmatch(text)
    if not match or len(text) > 100:
        raise ValueError(f"invalid stable SemVer: {text!r}")
    return tuple(map(int, match.groups()))


def section(text, heading):
    match = re.search(r"(?ms)^\[" + re.escape(heading) + r"\]\s*\n(.*?)(?=^\[|\Z)", text)
    if not match:
        raise ValueError(f"missing [{heading}] section")
    return match


def string_field(text, name):
    match = re.search(r"(?m)^" + re.escape(name) + r'\s*=\s*"([^"\n]+)"', text)
    if not match:
        raise ValueError(f"missing string {name}")
    return match


def workspace():
    path = ROOT / "Cargo.toml"
    text = path.read_text()
    match = section(text, "workspace.package")
    field = string_field(match.group(1), "version")
    return path, text, match, field


def plan(args):
    if len(args) > 2:
        raise ValueError("usage: next-version.sh <auto|patch|minor|major> [title]")
    bump = args[0] if args else "auto"
    title = args[1] if len(args) > 1 else ""
    if bump not in ("auto", "patch", "minor", "major"):
        raise ValueError(f"unknown bump: {bump}")
    floor = version(workspace()[3].group(1))
    tags = subprocess.check_output(["git", "-C", str(ROOT), "tag", "-l", "v*"], text=True)
    stable = [version(tag[1:]) for tag in tags.splitlines() if STABLE.fullmatch(tag[1:])]
    latest = max(stable, default=None)
    if latest is None or floor > latest:
        result = floor
    else:
        if bump == "auto":
            if re.match(r"^[a-zA-Z]+(?:\([^)]*\))?!:", title) or "BREAKING CHANGE" in title:
                bump = "major"
            elif re.match(r"^feat(?:\([^)]*\))?:", title):
                bump = "minor"
            else:
                bump = "patch"
        major, minor, patch = latest
        result = {"major": (major + 1, 0, 0), "minor": (major, minor + 1, 0),
                  "patch": (major, minor, patch + 1)}[bump]
    print(".".join(map(str, result)))


def atomic_write(path, text):
    mode = path.stat().st_mode
    fd, name = tempfile.mkstemp(prefix=f".{path.name}.", dir=path.parent)
    try:
        with os.fdopen(fd, "w") as output:
            output.write(text)
        os.chmod(name, mode)
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def set_version(args):
    if len(args) != 1:
        raise ValueError("usage: set-rust-version.sh <stable-semver>")
    requested = args[0]
    version(requested)
    path, text, match, field = workspace()
    version(field.group(1))
    start = match.start(1) + field.start(1)
    end = match.start(1) + field.end(1)
    updated = text[:start] + requested + text[end:]
    members = re.search(r"(?s)\bmembers\s*=\s*\[([^]]*)\]", section(text, "workspace").group(1))
    if not members:
        raise ValueError("missing workspace members")
    names = set()
    for member in re.findall(r'"([^"\n]+)"', members.group(1)):
        manifests = sorted(ROOT.glob(f"{member}/Cargo.toml"))
        if not manifests:
            raise ValueError(f"missing workspace member: {member}")
        for manifest in manifests:
            package = section(manifest.read_text(), "package").group(1)
            if not re.search(r"(?m)^version\.workspace\s*=\s*true\s*$", package):
                raise ValueError(f"member must inherit workspace version: {manifest}")
            names.add(string_field(package, "name").group(1))
    lock_path = ROOT / "Cargo.lock"
    lock = lock_path.read_text()
    found = set()

    def replace(block):
        content = block.group(0)
        name = string_field(content, "name").group(1)
        if name not in names or re.search(r"(?m)^source\s*=", content):
            return content
        if name in found:
            raise ValueError(f"duplicate local lockfile package: {name}")
        found.add(name)
        current = string_field(content, "version")
        version(current.group(1))
        return content[:current.start(1)] + requested + content[current.end(1):]

    updated_lock = re.sub(r"(?ms)^\[\[package\]\]\n.*?(?=^\[\[package\]\]|\Z)", replace, lock)
    if found != names:
        raise ValueError(f"missing local lockfile packages: {sorted(names - found)}")
    # Validate both files before publishing either change. Per-file replacement
    # is atomic; a interrupted pair is refused by the subsequent locked build.
    atomic_write(path, updated)
    atomic_write(lock_path, updated_lock)
    print(f"set workspace and {len(names)} local lockfile packages to {requested}")


if __name__ == "__main__":
    try:
        if len(sys.argv) < 2 or sys.argv[1] not in ("plan", "set"):
            raise ValueError("expected plan or set")
        (plan if sys.argv[1] == "plan" else set_version)(sys.argv[2:])
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
