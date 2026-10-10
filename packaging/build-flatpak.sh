#!/usr/bin/env bash
# Package the already-built native Linux tarball inside Freedesktop 26.08.
set -euo pipefail
[[ $# -eq 2 ]] || { echo "usage: $0 <stable-semver> <amd64|arm64>" >&2; exit 2; }
VERSION="$1"; ARCH="$2"
[[ "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || exit 2
case "$ARCH" in amd64) NATIVE=x86_64 ;; arm64) NATIVE=aarch64 ;; *) echo "unsupported architecture" >&2; exit 2 ;; esac
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# flatpak-builder strips/debug-compresses ELF files with host elfutils.
for tool in flatpak flatpak-builder python3 eu-strip eu-elfcompress; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 1; }
done
[[ "$(uname -m)" == "$NATIVE" ]] || { echo "native architecture mismatch" >&2; exit 2; }
TARBALL="$ROOT_DIR/dist/nntpbrowser_${VERSION}_linux_${ARCH}.tar.gz"
[[ -s "$TARBALL" ]] || { echo "Linux tarball missing: $TARBALL" >&2; exit 1; }
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/nntpbrowser-flatpak.XXXXXXXX")"
trap 'rm -rf -- "$STAGING"' EXIT
APP_ID=io.github.mnj.NNTPBrowser
flatpak remote-add --user --if-not-exists flathub https://flathub.org/repo/flathub.flatpakrepo
flatpak install --user --noninteractive --arch="$NATIVE" flathub org.freedesktop.Platform//26.08 org.freedesktop.Sdk//26.08
python3 - "$STAGING/manifest.json" "$TARBALL" "$APP_ID" <<'PY'
import json, sys
manifest = {
    "app-id": sys.argv[3], "runtime": "org.freedesktop.Platform",
    "runtime-version": "26.08", "sdk": "org.freedesktop.Sdk", "command": "nntpbrowser",
    "rename-desktop-file": "nntpbrowser.desktop", "rename-icon": "nntpbrowser",
    "finish-args": ["--share=network", "--share=ipc", "--socket=wayland",
                    "--socket=fallback-x11", "--device=dri", "--filesystem=xdg-download"],
    # Flatpak's private XDG data/config/cache dirs persist automatically. Share
    # Downloads explicitly for user-chosen output, without granting the home.
    "modules": [{"name": "nntpbrowser", "buildsystem": "simple",
                 "build-commands": ["cp -a bin /app/", "cp -a share /app/"],
                 "sources": [{"type": "archive", "path": sys.argv[2]}]}]
}
with open(sys.argv[1], "w") as output:
    json.dump(manifest, output, indent=2)
PY
flatpak-builder --user --force-clean --arch="$NATIVE" --default-branch=stable --state-dir="$STAGING/state" \
  --repo="$STAGING/repo" "$STAGING/build" "$STAGING/manifest.json"
OUTPUT="nntpbrowser_${VERSION}_linux_${ARCH}.flatpak"
flatpak build-bundle --arch="$NATIVE" --runtime-repo=https://flathub.org/repo/flathub.flatpakrepo \
  "$STAGING/repo" "$STAGING/$OUTPUT" "$APP_ID" stable
[[ -s "$STAGING/$OUTPUT" ]] || { echo "Flatpak output missing" >&2; exit 1; }
mv -- "$STAGING/$OUTPUT" "$ROOT_DIR/dist/$OUTPUT"
printf 'Built dist/%s\n' "$OUTPUT"
