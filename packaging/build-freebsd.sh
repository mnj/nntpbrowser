#!/usr/bin/env bash
# Native FreeBSD archive; CARGO_TARGET_DIR may live outside the VM checkout.
set -euo pipefail
[[ $# -eq 1 ]] || { echo "usage: $0 <stable-semver>" >&2; exit 2; }
VERSION="$1"
[[ "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || { echo "invalid stable SemVer" >&2; exit 2; }
[[ "$(uname -s)" == FreeBSD && "$(uname -m)" == amd64 ]] || { echo "native FreeBSD amd64 host required" >&2; exit 2; }
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
for tool in cargo rustc python3 tar; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 1; }
done
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/nntpbrowser-freebsd.XXXXXXXX")"
trap 'rm -rf -- "$STAGING"' EXIT
cd "$ROOT_DIR"
bash scripts/set-rust-version.sh "$VERSION"
cargo build --locked --release -p app --bin nntpbrowser
HOST="$(rustc -vV | sed -n 's/^host: //p')"
[[ "$HOST" == x86_64-unknown-freebsd ]] || { echo "unexpected Rust host: $HOST" >&2; exit 2; }
cargo metadata --locked --format-version 1 --filter-platform "$HOST" > "$STAGING/metadata.json"
TARGET_DIR="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["target_directory"])' "$STAGING/metadata.json")"
PACKAGE="nntpbrowser_${VERSION}_freebsd_x86_64"
PAYLOAD="$STAGING/$PACKAGE"
DOCS="$PAYLOAD/share/doc/nntpbrowser"
mkdir -p "$PAYLOAD/bin" "$PAYLOAD/share/applications" "$PAYLOAD/share/icons/hicolor/scalable/apps" "$DOCS"
install -m 0755 "$TARGET_DIR/release/nntpbrowser" "$PAYLOAD/bin/nntpbrowser"
install -m 0644 packaging/nntpbrowser.desktop "$PAYLOAD/share/applications/"
install -m 0644 packaging/nntpbrowser.svg "$PAYLOAD/share/icons/hicolor/scalable/apps/"
cp LICENSE docs/dependencies.md packaging/RUNTIME.md "$DOCS/"
python3 scripts/collect-notices.py "$STAGING/metadata.json" "$DOCS/third-party"
"$PAYLOAD/bin/nntpbrowser" --version
cat > "$PAYLOAD/README.txt" <<'TEXT'
NNTPBrowser for FreeBSD 14.3+ amd64. Run bin/nntpbrowser.
GUI runtime packages: libX11 libXcursor libXrandr libXi libxcb libxkbcommon
wayland mesa-libs. Install ffmpeg for video previews, and unrar or unar
for extraction. See share/doc/nntpbrowser/RUNTIME.md for optional tools.
TEXT
mkdir -p "$ROOT_DIR/dist"
tar -C "$STAGING" -czf "$STAGING/$PACKAGE.tar.gz" "$PACKAGE"
mv "$STAGING/$PACKAGE.tar.gz" "$ROOT_DIR/dist/"
printf 'Built dist/%s.tar.gz\n' "$PACKAGE"
