#!/usr/bin/env bash
# Smoke-check real release payloads; never launch the GUI or contact a provider.
set -euo pipefail
[[ $# -eq 2 ]] || { echo "usage: $0 <stable-semver> <amd64|arm64>" >&2; exit 2; }
VERSION="$1"; ARCH="$2"
[[ "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || exit 2
case "$ARCH" in amd64) NATIVE=x86_64 ;; arm64) NATIVE=aarch64 ;; *) exit 2 ;; esac
[[ "$(uname -m)" == "$NATIVE" ]] || { echo "native architecture mismatch" >&2; exit 2; }
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DIST="${DIST_DIR:-$ROOT_DIR/dist}"
for tool in dpkg-deb rpm rpm2cpio tar python3; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 1; }
done
DEB="$DIST/nntpbrowser_${VERSION}_${ARCH}.deb"
TARBALL="$DIST/nntpbrowser_${VERSION}_linux_${ARCH}.tar.gz"
RPM="$DIST/nntpbrowser_${VERSION}_linux_${ARCH}.rpm"
APPIMAGE="$DIST/nntpbrowser_${VERSION}_linux_${ARCH}.AppImage"
ZSYNC="$APPIMAGE.zsync"
for file in "$DEB" "$TARBALL" "$RPM" "$APPIMAGE" "$ZSYNC"; do
  [[ -s "$file" ]] || { echo "missing release asset: $file" >&2; exit 1; }
done
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/nntpbrowser-artifact-test.XXXXXXXX")"
trap 'rm -rf -- "$STAGING"' EXIT
[[ "$(dpkg-deb -f "$DEB" Package)" == nntpbrowser ]]
[[ "$(dpkg-deb -f "$DEB" Version)" == "$VERSION" ]]
[[ "$(dpkg-deb -f "$DEB" Architecture)" == "$ARCH" ]]
[[ "$(dpkg-deb -f "$DEB" Depends)" == *libxkbcommon0* ]]
[[ "$(dpkg-deb -f "$DEB" Depends)" == *libc6* ]]
[[ "$(dpkg-deb -f "$DEB" Recommends)" == *ffmpeg* ]]
dpkg-deb -x "$DEB" "$STAGING/debian"
# Check the portable archive before extraction; keep paths inside staging and
# bound decompressed members to 512MiB/20,000 entries.
python3 - "$TARBALL" <<'PY'
from pathlib import PurePosixPath
import sys, tarfile
with tarfile.open(sys.argv[1]) as archive:
    total = count = 0
    for member in archive:
        path = PurePosixPath(member.name)
        assert not path.is_absolute() and ".." not in path.parts, member.name
        assert member.isfile() or member.isdir(), member.name
        total += member.size
        count += 1
        assert total <= 512 * 1024 * 1024 and count <= 20000, "excessive archive"
PY
mkdir "$STAGING/portable"
tar -xzf "$TARBALL" -C "$STAGING/portable"
PORTABLE="$STAGING/portable/nntpbrowser_${VERSION}_linux_${ARCH}"
EXPECTED="NNTPBrowser $VERSION"
[[ "$("$STAGING/debian/usr/bin/nntpbrowser" --version)" == "$EXPECTED" ]]
[[ "$("$PORTABLE/bin/nntpbrowser" --version)" == "$EXPECTED" ]]
cmp "$STAGING/debian/usr/bin/nntpbrowser" "$PORTABLE/bin/nntpbrowser"
for prefix in "$STAGING/debian/usr" "$PORTABLE"; do
  [[ -s "$prefix/share/applications/nntpbrowser.desktop" ]]
  [[ -s "$prefix/share/icons/hicolor/scalable/apps/nntpbrowser.svg" ]]
  [[ -s "$prefix/share/doc/nntpbrowser/LICENSE" ]]
  [[ -s "$prefix/share/doc/nntpbrowser/dependencies.md" ]]
  [[ -s "$prefix/share/doc/nntpbrowser/RUNTIME.md" ]]
done
cmp "$ROOT_DIR/LICENSE" "$PORTABLE/share/doc/nntpbrowser/LICENSE"
python3 - "$PORTABLE/share/doc/nntpbrowser/third-party" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
assert root.joinpath("INDEX.md").is_file()
for filename in ("OFL-SourceSans3.txt", "OFL-JetBrainsMono.txt"):
    assert root.joinpath("licenses/app-fonts", filename).is_file(), filename
assert any(root.glob("covered-source/option-ext-*/src/lib.rs")), "MPL source missing"
for filename in ("OFL.txt", "UFL.txt", "Hack-Regular.txt", "emoji-icon-font-mit-license.txt"):
    assert any(root.glob(f"licenses/epaint_default_fonts-*/fonts/{filename}")), filename
assert any(root.glob("licenses/ring-*/LICENSE-BoringSSL")), "native Ring terms missing"
assert any(root.glob("licenses/zstd-sys-*/zstd/LICENSE")), "native Zstandard terms missing"
PY
[[ "$(rpm -qp --qf '%{NAME}' "$RPM")" == nntpbrowser ]]
[[ "$(rpm -qp --qf '%{VERSION}' "$RPM")" == "$VERSION" ]]
[[ "$(rpm -qp --qf '%{ARCH}' "$RPM")" == "$NATIVE" ]]
rpm --checksig "$RPM"
# Check the payload without installing the RPM. rpmbuild may strip its binary,
# so verify that actual executable rather than assuming byte-identical output.
python3 - "$RPM" "$STAGING/rpm-nntpbrowser" <<'PY'
from pathlib import PurePosixPath
import stat, subprocess, sys
process = subprocess.Popen(["rpm2cpio", sys.argv[1]], stdout=subprocess.PIPE)
stream = process.stdout
total = 0

def read(count):
    global total
    total += count
    assert total <= 512 * 1024 * 1024, "excessive RPM payload"
    result = bytearray()
    while len(result) < count:
        chunk = stream.read(count - len(result))
        assert chunk, "truncated RPM payload"
        result.extend(chunk)
    return bytes(result)

found = False
try:
    for _ in range(20000):
        header = read(110)
        assert header[:6] in (b"070701", b"070702"), "unexpected CPIO format"
        fields = [int(header[6 + index * 8:14 + index * 8], 16) for index in range(13)]
        size, namesize = fields[6], fields[11]
        assert 0 < namesize <= 65536 and size <= 128 * 1024 * 1024
        name = read(namesize)
        assert name[-1:] == b"\0"
        name = name[:-1].decode("utf-8")
        read(-(110 + namesize) % 4)
        if name == "TRAILER!!!":
            break
        path = PurePosixPath(name)
        assert not path.is_absolute() and ".." not in path.parts, name
        output = None
        if name.removeprefix("./") == "usr/bin/nntpbrowser":
            assert not found and stat.S_ISREG(fields[1])
            output = open(sys.argv[2], "wb")
            found = True
        try:
            remaining = size
            while remaining:
                chunk = read(min(1024 * 1024, remaining))
                if output is not None:
                    output.write(chunk)
                remaining -= len(chunk)
        finally:
            if output is not None:
                output.close()
        read(-size % 4)
    else:
        raise AssertionError("excessive RPM entries")
    assert found, "RPM executable missing"
    assert len(stream.read(1024)) <= 512, "unexpected trailing RPM data"
    assert process.wait() == 0, "rpm2cpio failed"
finally:
    if process.poll() is None:
        process.kill()
        process.wait()
PY
chmod 0755 "$STAGING/rpm-nntpbrowser"
[[ "$("$STAGING/rpm-nntpbrowser" --version)" == "$EXPECTED" ]]
rpm -qp --qf '[%{FILENAMES}\t%{FILEDIGESTS}\n]' "$RPM" > "$STAGING/rpm-files.tsv"
ALGORITHM="$(rpm -qp --qf '%{FILEDIGESTALGO}' "$RPM")"
python3 - "$STAGING/rpm-files.tsv" "$ALGORITHM" "$STAGING/rpm-nntpbrowser" <<'PY'
import hashlib
from pathlib import Path
import sys
files = dict(line.split("\t", 1) for line in Path(sys.argv[1]).read_text().splitlines())
algorithm = {"1": "md5", "8": "sha256", "9": "sha384", "10": "sha512"}[sys.argv[2]]
hasher = hashlib.new(algorithm)
with open(sys.argv[3], "rb") as binary:
    while chunk := binary.read(1024 * 1024):
        hasher.update(chunk)
digest = hasher.hexdigest()
assert files["/usr/bin/nntpbrowser"] == digest, "RPM binary differs"
for path in ("/usr/share/applications/nntpbrowser.desktop", "/usr/share/icons/hicolor/scalable/apps/nntpbrowser.svg",
             "/usr/share/doc/nntpbrowser/LICENSE", "/usr/share/doc/nntpbrowser/third-party/INDEX.md"):
    assert path in files, path
assert any("/covered-source/option-ext-" in path and path.endswith("/src/lib.rs") for path in files)
PY
mkdir "$STAGING/appimage"
(cd "$STAGING/appimage" && "$APPIMAGE" --appimage-extract > "$STAGING/appimage-extract.log")
APPDIR="$STAGING/appimage/squashfs-root"
[[ "$(APPDIR="$APPDIR" "$APPDIR/AppRun" --version)" == "$EXPECTED" ]]
cmp "$STAGING/debian/usr/bin/nntpbrowser" "$APPDIR/usr/bin/nntpbrowser"
[[ -s "$APPDIR/nntpbrowser.desktop" && -s "$APPDIR/nntpbrowser.svg" ]]
for file in runtime-LICENSE libfuse-LGPL2.txt libfuse-LICENSE type2-runtime-source.tar.gz fuse-3.15.0.tar.xz SOURCES.json; do
  [[ -s "$APPDIR/usr/share/doc/nntpbrowser/third-party/appimage-runtime/$file" ]]
done
REPOSITORY="${GITHUB_REPOSITORY:-mnj/nntpbrowser}"
python3 - "$ZSYNC" "$APPIMAGE" "$VERSION" "$REPOSITORY" <<'PY'
import hashlib
from pathlib import Path
import sys
zsync, image, version, repository = map(str, sys.argv[1:])
with open(zsync, "rb") as stream:
    header = stream.read(65536).split(b"\n\n", 1)[0].decode("ascii")
fields = dict(line.split(": ", 1) for line in header.splitlines()
              if ": " in line)
expected = f"https://github.com/{repository}/releases/download/v{version}/{Path(image).name}"
assert fields["URL"] == expected, fields["URL"]
assert int(fields["Length"]) == Path(image).stat().st_size
hasher = hashlib.sha1()
with open(image, "rb") as binary:
    while chunk := binary.read(1024 * 1024):
        hasher.update(chunk)
assert fields["SHA-1"] == hasher.hexdigest()
PY
[[ "$("$APPIMAGE" --appimage-updateinformation)" == "gh-releases-zsync|${REPOSITORY%/*}|${REPOSITORY#*/}|latest|nntpbrowser_*_linux_${ARCH}.AppImage.zsync" ]]
printf 'Real Linux %s artifacts passed version, payload, notices and update checks\n' "$ARCH"
