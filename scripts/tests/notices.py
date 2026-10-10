#!/usr/bin/env python3
"""Check graph selection, nested licenses, font terms and covered sources."""
import json
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile

script = Path(__file__).resolve().parents[1] / "collect-notices.py"
sys.path.insert(0, str(script.parent))
from notice_exceptions import declaration_texts, DISPATCH_REVISION
spec = importlib.util.spec_from_file_location("notice_collector", script)
collector = importlib.util.module_from_spec(spec)
spec.loader.exec_module(collector)
with tempfile.TemporaryDirectory(prefix="nntpbrowser-notices-test-") as temporary:
    root = Path(temporary)
    packages = []
    for name in ("app", "native", "fonts", "option-ext", "dev-only"):
        folder = root / name
        folder.mkdir()
        (folder / "Cargo.toml").write_text("fixture")
        (folder / "LICENSE").write_text(f"{name} terms")
        packages.append({"id": name, "name": name, "version": "0.2.0",
                         "manifest_path": str(folder / "Cargo.toml"),
                         "license": "MPL-2.0" if name == "option-ext" else "MIT OR Apache-2.0",
                         "source": "registry+https://github.com/rust-lang/crates.io-index"})
    (root / "app/assets").mkdir()
    for family in ("SourceSans3", "JetBrainsMono"):
        (root / "app/assets" / f"{family}-Regular.ttf").write_bytes(b"fixture font")
        (root / "app/assets" / f"OFL-{family}.txt").write_text(f"{family} exact OFL terms")
    (root / "native/nested").mkdir()
    (root / "native/nested/LICENSE-BSD").write_text("native BSD terms")
    (root / "native/COPYRIGHT").write_text("native copyright")
    (root / "fonts/fonts").mkdir()
    for name in ("OFL.txt", "UFL.txt", "Hack-Regular.txt", "emoji-icon-font-mit-license.txt"):
        (root / "fonts/fonts" / name).write_text(name)
    (root / "option-ext/lib.rs").write_text("exact MPL source")
    nodes = [{"id": package["id"], "deps": []} for package in packages]
    nodes[0]["deps"] = [{"pkg": name, "dep_kinds": [{"kind": kind}]} for name, kind in
                         (("native", "build"), ("fonts", None), ("option-ext", None), ("dev-only", "dev"))]
    metadata = root / "metadata.json"
    metadata.write_text(json.dumps({"packages": packages, "workspace_members": ["app"],
                                    "resolve": {"nodes": nodes}}))
    output = root / "notices"
    subprocess.run([sys.executable, str(script), str(metadata), str(output)], check=True)
    assert not (output / "licenses/dev-only-0.2.0").exists()
    for family in ("SourceSans3", "JetBrainsMono"):
        assert (output / "licenses/app-fonts" / f"OFL-{family}.txt").read_text() == f"{family} exact OFL terms"
    assert (output / "licenses/native-0.2.0/nested/LICENSE-BSD").read_text() == "native BSD terms"
    assert (output / "licenses/native-0.2.0/COPYRIGHT").exists()
    for name in ("OFL.txt", "UFL.txt", "Hack-Regular.txt", "emoji-icon-font-mit-license.txt"):
        assert (output / "licenses/fonts-0.2.0/fonts" / name).exists()
    assert (output / "covered-source/option-ext-0.2.0/lib.rs").read_text() == "exact MPL source"
    index = (output / "INDEX.md").read_text()
    assert "MPL-2.0" in index and "Selected terms: MIT" in index
    assert "https://crates.io/crates/option-ext/0.2.0" in index
    # The legacy exception requires the exact published declaration and a
    # byte-identical immutable upstream manifest, not just a claimed MIT label.
    dispatch = root / "dispatch"
    dispatch.mkdir()
    manifest = '''[package]
name = "dispatch"
version = "0.2.0"
authors = ["Steven Sheldon"]
edition = "2018"

description = "Rust wrapper for Apple's Grand Central Dispatch."
keywords = ["gcd", "objective-c", "osx", "ios"]
readme = "README.md"
repository = "http://github.com/SSheldon/rust-dispatch"
documentation = "http://ssheldon.github.io/rust-objc/dispatch/"
license = "MIT"

exclude = [
  ".gitignore",
  ".travis.yml",
  "travis_install.sh",
  "travis_test.sh",
  "tests-ios/**",
]
'''.encode()
    (dispatch / "Cargo.toml.orig").write_bytes(manifest)
    package = {"name": "dispatch", "version": "0.2.0", "license": "MIT",
               "repository": "http://github.com/SSheldon/rust-dispatch"}
    def pinned_manifest(url):
        assert url == f"https://raw.githubusercontent.com/SSheldon/rust-dispatch/{DISPATCH_REVISION}/Cargo.toml"
        return manifest
    texts = declaration_texts(package, dispatch, DISPATCH_REVISION, pinned_manifest)
    by_name = {name: data for name, _, data in texts}
    assert by_name['Cargo.toml.orig'] == manifest
    assert b'Steven Sheldon' in by_name['NOTICE-EXCEPTION.txt']
    assert b'not a license' in by_name['NOTICE-EXCEPTION.txt']
    assert b'Copyright <YEAR> <COPYRIGHT HOLDER>' in by_name['LICENSE-MIT-STANDARD.txt']
    for field, value in (("name", "other"), ("version", "0.2.1"),
                         ("license", "Apache-2.0"), ("repository", "https://github.com/other/repo")):
        assert declaration_texts(dict(package, **{field: value}), dispatch,
                                 DISPATCH_REVISION, pinned_manifest) is None
    assert declaration_texts(package, dispatch, '0' * 40, pinned_manifest) is None
    for local, remote in ((manifest + b'changed', manifest), (manifest, b'changed upstream')):
        (dispatch / "Cargo.toml.orig").write_bytes(local)
        try:
            declaration_texts(package, dispatch, DISPATCH_REVISION, lambda _: remote)
        except ValueError:
            pass
        else:
            raise AssertionError('mismatched pinned declaration accepted')
    workspace = root / 'upstream-workspace'
    workspace.mkdir()
    (workspace / '.cargo_vcs_info.json').write_text(json.dumps({'git': {'sha1': 'a' * 40}}))
    api_url = 'https://api.github.com/repos/owner/repo/contents?ref=' + 'a' * 40
    raw_url = 'https://raw.githubusercontent.com/owner/repo/' + 'a' * 40 + '/LICENSE'
    def pinned_notice(url):
        if url == api_url:
            return json.dumps([{'type': 'file', 'name': 'LICENSE', 'path': 'LICENSE',
                                'download_url': raw_url}]).encode()
        assert url == raw_url
        return b'upstream terms'
    collector.download = pinned_notice
    for repository in ('http://github.com/owner/repo', 'https://github.com/owner/repo.git',
                       'https://github.com/owner/repo/tree/main/crates/component'):
        collector.UPSTREAM_CACHE.clear()
        texts = collector.repository_texts(dict(package, name='component', repository=repository), workspace)
        assert texts == [('LICENSE', raw_url, b'upstream terms')]
print("notice collection tests passed")
