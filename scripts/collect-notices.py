#!/usr/bin/env python3
"""Collect upstream texts for the target's normal/build application graph."""
import json
import os
from pathlib import Path
import re
import shutil
import sys
from urllib.parse import quote, urlparse
from urllib.request import HTTPRedirectHandler, Request, build_opener
from notice_exceptions import declaration_texts

LICENSE_NAME = re.compile(r"(?i)^(licen[cs]e|copying|copyright|notice|thirdpartynotice)")
UPSTREAM_CACHE = {}


def download(url):
    class SafeRedirect(HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
            if redirected is not None and urlparse(newurl).hostname != "api.github.com":
                redirected.remove_header("Authorization")
            return redirected

    headers = {"User-Agent": "NNTPBrowser-release-notices"}
    if urlparse(url).hostname == "api.github.com" and os.environ.get("GH_TOKEN"):
        headers["Authorization"] = "Bearer " + os.environ["GH_TOKEN"]
    request = Request(url, headers=headers)
    with build_opener(SafeRedirect).open(request, timeout=30) as response:
        data = response.read(1024 * 1024 + 1)
    if len(data) > 1024 * 1024:
        raise ValueError(f"upstream notice exceeds 1MiB: {url}")
    return data


def repository_texts(package, folder):
    """Published workspace crates sometimes omit their repository-root texts."""
    repository = package.get("repository") or ""
    # Legacy manifests used HTTP URLs; notices are always fetched over HTTPS.
    match = re.match(r"https?://github\.com/([A-Za-z0-9_.-]+)/([A-Za-z0-9_.-]+)(?:/|$)", repository)
    vcs = folder / ".cargo_vcs_info.json"
    if not match or not vcs.is_file():
        raise ValueError(f"missing local licenses and pinned GitHub provenance: {package['name']}")
    owner, repo = match.groups()
    repo = repo.removesuffix(".git")
    revision = json.loads(vcs.read_text())["git"]["sha1"]
    if not re.fullmatch(r"[0-9a-f]{40}", revision):
        raise ValueError("invalid upstream commit identity")
    exception = declaration_texts(package, folder, revision, download)
    if exception is not None:
        return exception
    key = (owner, repo, revision)
    if key not in UPSTREAM_CACHE:
        url = f"https://api.github.com/repos/{owner}/{repo}/contents?ref={revision}"
        entries = json.loads(download(url))
        texts = []
        for entry in entries[:]:
            if entry["type"] == "dir" and LICENSE_NAME.match(entry["name"]):
                nested = json.loads(download(entry["url"]))
                if len(nested) > 100:
                    raise ValueError("excessive upstream license directory")
                entries.extend(nested)
        for entry in entries:
            if entry["type"] == "file" and (LICENSE_NAME.match(entry["name"])
                                            or LICENSE_NAME.match(entry["path"].split("/")[0])):
                source = entry["download_url"]
                if not source.startswith("https://raw.githubusercontent.com/"):
                    raise ValueError("unexpected upstream notice download host")
                texts.append((entry["path"], source, download(source)))
        if not any(re.match(r"(?i)^(licen[cs]e|copying)", name) for name, _, _ in texts):
            raise ValueError(f"no pinned repository-root license texts: {url}")
        UPSTREAM_CACHE[key] = texts
    return UPSTREAM_CACHE[key]


def chosen_license(expression):
    # Preserve every AND term. Plain OR alternatives use a permissive choice;
    # nested/native/font terms still retain their actual upstream notices.
    if " AND " not in expression:
        alternatives = [part.strip(" ()") for part in expression.split(" OR ")]
        for choice in ("MIT", "Apache-2.0", "BSD-3-Clause", "BSD-2-Clause", "ISC", "Zlib"):
            if choice in alternatives:
                return choice
    else:
        return re.sub(r"\(([^()]*)\)", lambda match: chosen_license(match.group(1)), expression)
    return expression


def main(metadata_path, destination):
    metadata = json.loads(Path(metadata_path).read_text())
    packages = {package["id"]: package for package in metadata["packages"]}
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    roots = [package["id"] for package in packages.values()
             if package["name"] == "app" and package["id"] in metadata["workspace_members"]]
    if len(roots) != 1:
        raise ValueError("expected one application workspace package")
    selected = set()
    todo = roots[:]
    while todo:
        current = todo.pop()
        if current in selected:
            continue
        selected.add(current)
        for dependency in nodes[current]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                todo.append(dependency["pkg"])
    output = Path(destination)
    output.mkdir(parents=True, exist_ok=False)
    lines = ["# Third-party release notices", "",
             "Generated from the locked target's normal/build dependency graph.",
             "Build-only packages are included conservatively. Individual upstream",
             "texts and embedded/native asset terms remain authoritative; this is",
             "an inventory, not legal certification. Unmodified upstream sources",
             "are used. Plain OR expressions select the permissive alternative",
             "shown below; AND terms and additional font/native terms still apply.", "",
             "MPL-covered option-ext source is included in covered-source/.", ""]
    # The UI embeds repository fonts in addition to epaint's crate fonts.
    assets = Path(packages[roots[0]]["manifest_path"]).parent / "assets"
    fonts = list(assets.glob("*.ttf"))
    if fonts:
        font_notices = list(assets.glob("OFL-*.txt"))
        if not font_notices:
            raise ValueError("embedded application fonts have no OFL notices")
        target = output / "licenses" / "app-fonts"
        target.mkdir(parents=True)
        for notice in font_notices:
            shutil.copyfile(notice, target / notice.name)
        lines.extend(["## Application fonts", "Embedded Source Sans 3 and JetBrains Mono.",
                      "License: SIL OFL 1.1; retain the copied copyright and naming terms.",
                      "Notices: licenses/app-fonts/", ""])
    count = 0
    for package in sorted((packages[id] for id in selected), key=lambda package: (package["name"], package["version"])):
        if package["id"] in metadata["workspace_members"]:
            continue
        name, version = package["name"], package["version"]
        folder = Path(package["manifest_path"]).parent
        target = output / "licenses" / f"{name}-{version}"
        expression = package.get("license") or "See upstream license-file"
        copied = set()
        explicit = package.get("license_file")
        files = [file for file in folder.rglob("*") if file.is_file() and
                 (LICENSE_NAME.match(file.name)
                  or file.name in ("OFL.txt", "UFL.txt", "Hack-Regular.txt", "emoji-icon-font-mit-license.txt"))]
        if explicit:
            file = folder / explicit
            if not file.is_file():
                raise ValueError(f"missing explicit license file: {file}")
            files.append(file)
        for file in files:
            relative = file.relative_to(folder)
            if relative in copied:
                continue
            copied.add(relative)
            destination = target / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(file, destination)
        if not explicit and not any(len(path.parts) == 1 and re.match(r"(?i)^(licen[cs]e|copying)", path.name)
                                    for path in copied):
            provenance = {}
            for filename, source, data in repository_texts(package, folder):
                destination = target / filename
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(data)
                copied.add(Path(filename))
                provenance[filename] = source
            (target / "UPSTREAM-SOURCES.json").write_text(json.dumps(provenance, indent=2) + "\n")
        if not copied:
            raise ValueError(f"no upstream license texts found for {name} {version}")
        url = f"https://crates.io/crates/{quote(name)}/{quote(version)}"
        if package.get("source", "").startswith("git+"):
            url = package["source"]
        lines.extend([f"## {name} {version}", f"License expression: {expression}",
                      f"Selected terms: {chosen_license(expression)}", f"Source: {url}",
                      f"Notices: licenses/{name}-{version}/", ""])
        if name == "option-ext":
            shutil.copytree(folder, output / "covered-source" / f"{name}-{version}")
        count += 1
    (output / "INDEX.md").write_text("\n".join(lines))
    print(f"collected notices for {count} target dependencies")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 3:
            raise ValueError("usage: collect-notices.py <cargo-metadata.json> <new-output-directory>")
        main(*sys.argv[1:])
    except (ValueError, OSError, KeyError) as error:
        print(str(error), file=sys.stderr)
        sys.exit(2)
