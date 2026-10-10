#!/usr/bin/env bash
# Repackage the Debian payload without rebuilding Rust.
set -euo pipefail
[[ $# -eq 2 ]] || { echo "usage: $0 <stable-semver> <amd64|arm64>" >&2; exit 2; }
VERSION="$1"; ARCH="$2"
[[ "$VERSION" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || exit 2
case "$ARCH" in
  amd64) NATIVE=x86_64; TOOL_SHA=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0; RUNTIME_SHA=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d ;;
  arm64) NATIVE=aarch64; TOOL_SHA=f0837e7448a0c1e4e650a93bb3e85802546e60654ef287576f46c71c126a9158; RUNTIME_SHA=00cbdfcf917cc6c0ff6d3347d59e0ca1f7f45a6df1a428a0d6d8a78664d87444 ;;
  *) echo "unsupported architecture" >&2; exit 2 ;;
esac
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
for tool in dpkg-deb rpmbuild curl sha256sum zsyncmake python3; do
  command -v "$tool" >/dev/null || { echo "$tool is required" >&2; exit 1; }
done
[[ "$(uname -m)" == "$NATIVE" ]] || { echo "native architecture mismatch" >&2; exit 2; }
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/nntpbrowser-formats.XXXXXXXX")"
trap 'rm -rf -- "$STAGING"' EXIT
PAYLOAD="$STAGING/payload"
dpkg-deb -x "$ROOT_DIR/dist/nntpbrowser_${VERSION}_${ARCH}.deb" "$PAYLOAD"
mkdir -p "$STAGING/rpm/"{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
cat > "$STAGING/rpm/SPECS/nntpbrowser.spec" <<EOF
%define debug_package %{nil}
%define _build_id_links none
Name: nntpbrowser
Version: $VERSION
Release: 1
Summary: Native Usenet newsreader
License: MIT
URL: https://github.com/mnj/nntpbrowser
Requires: libX11.so.6()(64bit), libxkbcommon.so.0()(64bit), libwayland-client.so.0()(64bit), libGL.so.1()(64bit), libEGL.so.1()(64bit)
%description
Browse articles and media and download multipart files with PAR2 recovery.
Third-party component terms and covered source accompany the application.
%install
mkdir -p "%{buildroot}"
cp -a "$PAYLOAD/usr" "%{buildroot}/"
%files
/usr/bin/nntpbrowser
/usr/share/applications/nntpbrowser.desktop
/usr/share/icons/hicolor/scalable/apps/nntpbrowser.svg
%doc /usr/share/doc/nntpbrowser
EOF
rpmbuild --define "_topdir $STAGING/rpm" --target "$NATIVE" -bb "$STAGING/rpm/SPECS/nntpbrowser.spec"
RPM="$STAGING/rpm/RPMS/$NATIVE/nntpbrowser-$VERSION-1.$NATIVE.rpm"
[[ -s "$RPM" ]] || { echo "RPM output missing" >&2; exit 1; }

APPDIR="$STAGING/NNTPBrowser.AppDir"
mkdir -p "$APPDIR"
cp -a "$PAYLOAD/usr" "$APPDIR/"
cp "$ROOT_DIR/packaging/nntpbrowser.desktop" "$APPDIR/"
cp "$ROOT_DIR/packaging/nntpbrowser.svg" "$APPDIR/"
ln -s nntpbrowser.svg "$APPDIR/.DirIcon"
cat > "$APPDIR/AppRun" <<'EOF'
#!/bin/sh
set -eu
APPDIR="${APPDIR:-$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)}"
exec "$APPDIR/usr/bin/nntpbrowser" "$@"
EOF
chmod 0755 "$APPDIR/AppRun"
python3 "$ROOT_DIR/scripts/collect-appimage-runtime.py" "$APPDIR/usr/share/doc/nntpbrowser/third-party/appimage-runtime"
curl -fL --retry 3 --proto '=https' -o "$STAGING/appimagetool" "https://github.com/AppImage/appimagetool/releases/download/1.9.1/appimagetool-$NATIVE.AppImage"
curl -fL --retry 3 --proto '=https' -o "$STAGING/runtime" "https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-$NATIVE"
printf '%s  %s\n%s  %s\n' "$TOOL_SHA" "$STAGING/appimagetool" "$RUNTIME_SHA" "$STAGING/runtime" | sha256sum -c -
chmod 0755 "$STAGING/appimagetool"
REPOSITORY="${GITHUB_REPOSITORY:-mnj/nntpbrowser}"
[[ "$REPOSITORY" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo "invalid release repository" >&2; exit 2; }
APPIMAGE="nntpbrowser_${VERSION}_linux_${ARCH}.AppImage"
UPDATE="gh-releases-zsync|${REPOSITORY%/*}|${REPOSITORY#*/}|latest|nntpbrowser_*_linux_${ARCH}.AppImage.zsync"
(cd "$STAGING" && ARCH="$NATIVE" VERSION="$VERSION" APPIMAGE_EXTRACT_AND_RUN=1 "$STAGING/appimagetool" \
  --runtime-file "$STAGING/runtime" --updateinformation "$UPDATE" "$APPDIR" "$STAGING/$APPIMAGE")
[[ -s "$STAGING/$APPIMAGE" ]] || { echo "AppImage missing" >&2; exit 1; }
chmod 0755 "$STAGING/$APPIMAGE"
# Rolling assets are renamed. Point the zsync payload at its immutable release
# URL so it remains valid when the .zsync itself is served from latest.
zsyncmake -u "https://github.com/$REPOSITORY/releases/download/v$VERSION/$APPIMAGE" \
  -o "$STAGING/$APPIMAGE.zsync" "$STAGING/$APPIMAGE"
[[ -s "$STAGING/$APPIMAGE.zsync" ]] || { echo "zsync missing" >&2; exit 1; }
mv -- "$RPM" "$ROOT_DIR/dist/nntpbrowser_${VERSION}_linux_${ARCH}.rpm"
mv -- "$STAGING/$APPIMAGE" "$STAGING/$APPIMAGE.zsync" "$ROOT_DIR/dist/"
printf 'Built RPM, AppImage and zsync for %s\n' "$ARCH"
