#!/usr/bin/env bash
# Native Ubuntu 22.04 build; outputs both portable and Debian desktop packages.
set -euo pipefail
if [[ $# -ne 2 ]]; then
  echo "usage: $0 <stable-semver> <amd64|arm64>" >&2
  exit 2
fi
VERSION="$1"
ARCH="$2"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
[[ "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || { echo "invalid stable SemVer" >&2; exit 2; }
case "$ARCH" in amd64|arm64) ;; *) echo "unsupported architecture: $ARCH" >&2; exit 2 ;; esac
[[ "$(uname -s)" == Linux ]] || { echo "Linux native build required" >&2; exit 2; }
for tool in cargo rustc python3 dpkg dpkg-deb tar; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 1; }
done
[[ "$(dpkg --print-architecture)" == "$ARCH" ]] || { echo "requested architecture must match native host" >&2; exit 2; }
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/nntpbrowser-package.XXXXXXXX")"
cleanup() { [[ -n "$STAGING" && -d "$STAGING" ]] && rm -rf -- "$STAGING"; }
trap cleanup EXIT
cd "$ROOT_DIR"
"$ROOT_DIR/scripts/set-rust-version.sh" "$VERSION"
cargo build --locked --release -p app --bin nntpbrowser
TARGET_ROOT="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
HOST="$(rustc -vV | sed -n 's/^host: //p')"
[[ -n "$HOST" ]] || { echo "could not determine Rust target" >&2; exit 1; }
cargo metadata --locked --format-version 1 --filter-platform "$HOST" > "$STAGING/metadata.json"
PKG_ROOT="$STAGING/debian"
DOCS="$PKG_ROOT/usr/share/doc/nntpbrowser"
mkdir -p "$PKG_ROOT/DEBIAN" "$PKG_ROOT/usr/bin" "$PKG_ROOT/usr/share/applications" \
  "$PKG_ROOT/usr/share/icons/hicolor/scalable/apps" "$DOCS"
install -m 0755 "$TARGET_ROOT/release/nntpbrowser" "$PKG_ROOT/usr/bin/nntpbrowser"
install -m 0644 "$ROOT_DIR/packaging/nntpbrowser.desktop" "$PKG_ROOT/usr/share/applications/"
install -m 0644 "$ROOT_DIR/packaging/nntpbrowser.svg" "$PKG_ROOT/usr/share/icons/hicolor/scalable/apps/"
install -m 0644 "$ROOT_DIR/LICENSE" "$DOCS/LICENSE"
install -m 0644 "$ROOT_DIR/docs/dependencies.md" "$DOCS/dependencies.md"
install -m 0644 "$ROOT_DIR/packaging/RUNTIME.md" "$DOCS/RUNTIME.md"
python3 "$ROOT_DIR/scripts/collect-notices.py" "$STAGING/metadata.json" "$DOCS/third-party"

# Scan the actual executable. A minimal control file provides dpkg-shlibdeps
# its package context; dynamic GUI libraries are added explicitly below.
mkdir -p "$STAGING/shlibs/debian"
printf 'Source: nntpbrowser\nSection: net\nPriority: optional\nMaintainer: NNTPBrowser contributors\n\nPackage: nntpbrowser\nArchitecture: any\nDescription: Native Usenet newsreader\n' > "$STAGING/shlibs/debian/control"
DETECTED=""
if command -v dpkg-shlibdeps >/dev/null; then
  if SHLIBS="$(cd "$STAGING/shlibs" && dpkg-shlibdeps -O -e"$PKG_ROOT/usr/bin/nntpbrowser")"; then
    DETECTED="$(printf '%s\n' "$SHLIBS" | sed -n 's/^shlibs:Depends=//p')"
  else
    echo "dpkg-shlibdeps unavailable for host libraries; using detected glibc baseline" >&2
  fi
fi
if [[ -z "$DETECTED" ]]; then
  command -v readelf >/dev/null || { echo "readelf required when shlibdeps cannot resolve libraries" >&2; exit 1; }
  GLIBC="$(readelf --version-info "$PKG_ROOT/usr/bin/nntpbrowser" | sed -n 's/.*Name: GLIBC_\([0-9][0-9.]*\).*/\1/p' | sort -V | tail -n1)"
  [[ -n "$GLIBC" ]] || { echo "could not detect binary glibc requirement" >&2; exit 1; }
  DETECTED="libc6 (>= $GLIBC), libgcc-s1"
fi
cat > "$PKG_ROOT/DEBIAN/control" <<EOF
Package: nntpbrowser
Version: $VERSION
Section: net
Priority: optional
Architecture: $ARCH
Maintainer: NNTPBrowser contributors
Depends: $DETECTED, libx11-6, libxkbcommon0, libwayland-client0, libgl1, libegl1
Recommends: ffmpeg, unrar | unar
Description: Native Usenet newsreader
 Browse server newsgroups, read articles, preview media in memory,
 and download multipart files with automatic PAR2 recovery.
EOF
mkdir -p "$ROOT_DIR/dist"
DEB="nntpbrowser_${VERSION}_${ARCH}.deb"
TARBALL="nntpbrowser_${VERSION}_linux_${ARCH}.tar.gz"
dpkg-deb --root-owner-group --build "$PKG_ROOT" "$STAGING/$DEB"
PORTABLE="nntpbrowser_${VERSION}_linux_${ARCH}"
mkdir -p "$STAGING/portable/$PORTABLE"
cp -a "$PKG_ROOT/usr/." "$STAGING/portable/$PORTABLE/"
tar --owner=0 --group=0 -C "$STAGING/portable" -czf "$STAGING/$TARBALL" "$PORTABLE"
# Publish only completed artifacts; staging cleanup cannot remove output files.
mv -- "$STAGING/$DEB" "$ROOT_DIR/dist/$DEB"
mv -- "$STAGING/$TARBALL" "$ROOT_DIR/dist/$TARBALL"
printf 'Built dist/%s\nBuilt dist/%s\n' "$DEB" "$TARBALL"
"$ROOT_DIR/packaging/build-linux-extras.sh" "$VERSION" "$ARCH"
