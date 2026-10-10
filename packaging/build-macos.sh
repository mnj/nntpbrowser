#!/usr/bin/env bash
# Universal native macOS application + the same binary for terminal use.
set -euo pipefail
[[ $# -eq 1 ]] || { echo "usage: $0 <stable-semver>" >&2; exit 2; }
VERSION="$1"
[[ "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || { echo "invalid stable SemVer" >&2; exit 2; }
[[ "$(uname -s)" == Darwin ]] || { echo "a macOS build host is required" >&2; exit 2; }
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
for tool in cargo rustc python3 lipo codesign hdiutil iconutil sips ditto plutil; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 1; }
done
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/nntpbrowser-macos.XXXXXXXX")"
trap 'rm -rf -- "$STAGING"' EXIT
cd "$ROOT_DIR"
"$ROOT_DIR/scripts/set-rust-version.sh" "$VERSION"
export MACOSX_DEPLOYMENT_TARGET=12.0
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  cargo build --locked --release --target "$target" -p app --bin nntpbrowser
  cargo metadata --locked --format-version 1 --filter-platform "$target" > "$STAGING/$target.json"
done
TARGET_DIR="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["target_directory"])' "$STAGING/aarch64-apple-darwin.json")"
APP="$STAGING/dmg/NNTPBrowser.app"
RESOURCES="$APP/Contents/Resources"
mkdir -p "$APP/Contents/MacOS" "$RESOURCES" "$STAGING/icons.iconset"
lipo -create "$TARGET_DIR/aarch64-apple-darwin/release/nntpbrowser" "$TARGET_DIR/x86_64-apple-darwin/release/nntpbrowser" -output "$APP/Contents/MacOS/nntpbrowser"
lipo "$APP/Contents/MacOS/nntpbrowser" -verify_arch arm64 x86_64
sed "s/VERSION_PLACEHOLDER/$VERSION/g" packaging/macos/Info.plist > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" packaging/macos/app-icon.png --out "$STAGING/icons.iconset/icon_${size}x${size}.png" >/dev/null
  doubled=$((size * 2))
  sips -z "$doubled" "$doubled" packaging/macos/app-icon.png --out "$STAGING/icons.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$STAGING/icons.iconset" -o "$RESOURCES/AppIcon.icns"
cp LICENSE docs/dependencies.md packaging/RUNTIME.md "$RESOURCES/"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  python3 scripts/collect-notices.py "$STAGING/$target.json" "$RESOURCES/third-party/$target"
done
# No Developer ID credentials are assumed; this is an ad-hoc signature, not
# notarization. Users may need macOS Privacy & Security → Open Anyway.
codesign --force --sign - "$APP/Contents/MacOS/nntpbrowser"
codesign --force --sign - "$APP"
codesign --verify --deep --strict "$APP"
"$APP/Contents/MacOS/nntpbrowser" --version
ln -s /Applications "$STAGING/dmg/Applications"
hdiutil create -volname "NNTPBrowser $VERSION" -srcfolder "$STAGING/dmg" -format UDZO "$STAGING/nntpbrowser_${VERSION}_macos_universal.dmg"
CLI_DIR="$STAGING/cli/nntpbrowser-cli_${VERSION}_macos_universal"
mkdir -p "$CLI_DIR/bin" "$CLI_DIR/share/doc/nntpbrowser"
cp "$APP/Contents/MacOS/nntpbrowser" "$CLI_DIR/bin/nntpbrowser"
cp -R "$RESOURCES/." "$CLI_DIR/share/doc/nntpbrowser/"
printf 'This is the same unified app/CLI binary. Run bin/nntpbrowser --help or --version.\n' > "$CLI_DIR/README.txt"
ditto -c -k --sequesterRsrc --keepParent "$CLI_DIR" "$STAGING/nntpbrowser-cli_${VERSION}_macos_universal.zip"
mkdir -p "$ROOT_DIR/dist"
mv "$STAGING/nntpbrowser_${VERSION}_macos_universal.dmg" "$ROOT_DIR/dist/"
mv "$STAGING/nntpbrowser-cli_${VERSION}_macos_universal.zip" "$ROOT_DIR/dist/"
printf 'Built universal macOS DMG and CLI ZIP in dist/\n'
