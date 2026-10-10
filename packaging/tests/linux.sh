#!/usr/bin/env bash
# Exercise staging/control/portable payloads without rebuilding the application.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nntpbrowser-linux-test.XXXXXXXX")"
trap 'rm -rf -- "$TEST_ROOT"' EXIT
mkdir -p "$TEST_ROOT/"{scripts,packaging,docs,bin,crates/app}
cp "$ROOT_DIR/scripts/"{set-rust-version.sh,version.py,collect-notices.py,notice_exceptions.py} "$TEST_ROOT/scripts/"
cp "$ROOT_DIR/packaging/"{build-linux.sh,nntpbrowser.desktop,nntpbrowser.svg,RUNTIME.md} "$TEST_ROOT/packaging/"
cp "$ROOT_DIR/docs/dependencies.md" "$TEST_ROOT/docs/"
cp "$ROOT_DIR/LICENSE" "$TEST_ROOT/"
cat > "$TEST_ROOT/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/app"]
[workspace.package]
version = "0.1.0"
EOF
cat > "$TEST_ROOT/crates/app/Cargo.toml" <<'EOF'
[package]
name = "app"
version.workspace = true
EOF
cat > "$TEST_ROOT/Cargo.lock" <<'EOF'
version = 4
[[package]]
name = "app"
version = "0.1.0"
EOF
cat > "$TEST_ROOT/bin/cargo" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
case "$1" in
  build)
    [[ "$*" == 'build --locked --release -p app --bin nntpbrowser' ]]
    mkdir -p "$CARGO_TARGET_DIR/release"
    cp /bin/true "$CARGO_TARGET_DIR/release/nntpbrowser"
    ;;
  metadata)
    printf '{"packages":[{"id":"app","name":"app","version":"3.4.5","manifest_path":"%s/crates/app/Cargo.toml"}],"workspace_members":["app"],"resolve":{"nodes":[{"id":"app","deps":[]}]}}\n' "$PWD"
    ;;
  *) exit 3 ;;
esac
EOF
cat > "$TEST_ROOT/packaging/build-linux-extras.sh" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" > "$(dirname "$0")/extras-args"
EOF
chmod +x "$TEST_ROOT/bin/cargo" "$TEST_ROOT/packaging/build-linux-extras.sh"
export PATH="$TEST_ROOT/bin:$PATH" CARGO_TARGET_DIR="$TEST_ROOT/build-output"
ARCH="$(dpkg --print-architecture)"
bash "$TEST_ROOT/packaging/build-linux.sh" 3.4.5 "$ARCH" > "$TEST_ROOT/build.log" 2>&1
[[ "$(cat "$TEST_ROOT/packaging/extras-args")" == "3.4.5 $ARCH" ]]
DEB="$TEST_ROOT/dist/nntpbrowser_3.4.5_${ARCH}.deb"
TAR="$TEST_ROOT/dist/nntpbrowser_3.4.5_linux_${ARCH}.tar.gz"
[[ "$(dpkg-deb -f "$DEB" Version)" == 3.4.5 ]]
[[ "$(dpkg-deb -f "$DEB" Architecture)" == "$ARCH" ]]
[[ "$(dpkg-deb -f "$DEB" Depends)" == *libxkbcommon0* ]]
[[ "$(dpkg-deb -f "$DEB" Recommends)" == *ffmpeg* ]]
mkdir "$TEST_ROOT/extracted"
tar -xzf "$TAR" -C "$TEST_ROOT/extracted"
PORTABLE="$TEST_ROOT/extracted/nntpbrowser_3.4.5_linux_$ARCH"
[[ -x "$PORTABLE/bin/nntpbrowser" ]]
[[ -f "$PORTABLE/share/applications/nntpbrowser.desktop" ]]
[[ -f "$PORTABLE/share/icons/hicolor/scalable/apps/nntpbrowser.svg" ]]
[[ -f "$PORTABLE/share/doc/nntpbrowser/third-party/INDEX.md" ]]
cmp "$ROOT_DIR/LICENSE" "$PORTABLE/share/doc/nntpbrowser/LICENSE"
[[ ! -e "$PORTABLE/DEBIAN" ]]
cp "$TEST_ROOT/Cargo.toml" "$TEST_ROOT/before"
for args in 'invalid amd64' '3.4.5 mips' '01.2.3 amd64'; do
  read -r version arch <<< "$args"
  if bash "$TEST_ROOT/packaging/build-linux.sh" "$version" "$arch" > /dev/null 2>&1; then
    echo "invalid package arguments accepted" >&2; exit 1
  fi
  cmp "$TEST_ROOT/before" "$TEST_ROOT/Cargo.toml"
done
echo "Linux packaging payload and validation tests passed"
