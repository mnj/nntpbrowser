#!/usr/bin/env bash
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nntpbrowser-version-test.XXXXXXXX")"
trap 'rm -rf -- "$TEST_ROOT"' EXIT
mkdir -p "$TEST_ROOT/scripts" "$TEST_ROOT/crates/example"
cp "$ROOT_DIR/scripts/next-version.sh" "$ROOT_DIR/scripts/set-rust-version.sh" "$ROOT_DIR/scripts/version.py" "$TEST_ROOT/scripts/"
cat > "$TEST_ROOT/Cargo.toml" <<'EOF'
[workspace]
members = ["crates/example"]
[workspace.package]
version = "0.1.0"
EOF
cat > "$TEST_ROOT/crates/example/Cargo.toml" <<'EOF'
[package]
name = "example"
version.workspace = true
EOF
cat > "$TEST_ROOT/Cargo.lock" <<'EOF'
version = 4

[[package]]
name = "example"
version = "0.1.0"
dependencies = ["example 9.9.9"]

[[package]]
name = "example"
version = "9.9.9"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "do-not-change"
EOF
git -C "$TEST_ROOT" init -q
git -C "$TEST_ROOT" -c user.name=Tests -c user.email=tests@example.invalid commit -q --allow-empty -m fixture
expect() {
  local expected="$1"; shift
  local actual
  actual="$(bash "$TEST_ROOT/scripts/next-version.sh" "$@")"
  [[ "$actual" == "$expected" ]] || { echo "expected $expected, received $actual" >&2; exit 1; }
}
reject() {
  if "$@" > "$TEST_ROOT/error.log" 2>&1; then
    echo "unexpected success: $*" >&2; exit 1
  fi
}
expect 0.1.0 auto 'feat!: initial release'
git -C "$TEST_ROOT" tag v0.1.0
git -C "$TEST_ROOT" tag v0.1.9
git -C "$TEST_ROOT" tag v0.1.10
git -C "$TEST_ROOT" tag v9.0.0-rc.1
git -C "$TEST_ROOT" tag v01.9.9
git -C "$TEST_ROOT" tag malformed
expect 0.1.11 patch
expect 0.2.0 minor
expect 1.0.0 major
expect 0.2.0 auto 'feat(gallery): thumbnails'
expect 1.0.0 auto 'fix(store)!: migration'
expect 1.0.0 auto 'refactor: BREAKING CHANGE'
expect 0.1.11 auto 'fix: reader'
expect 0.1.11 auto 'docs: help'
[[ "$(bash "$TEST_ROOT/scripts/next-version.sh" minor)" == "$(bash "$TEST_ROOT/scripts/next-version.sh" minor)" ]]
bash "$TEST_ROOT/scripts/set-rust-version.sh" 1.2.3 >/dev/null
expect 1.2.3 patch
cp "$TEST_ROOT/Cargo.toml" "$TEST_ROOT/manifest-before"
cp "$TEST_ROOT/Cargo.lock" "$TEST_ROOT/lock-before"
bash "$TEST_ROOT/scripts/set-rust-version.sh" 1.2.3 >/dev/null
cmp "$TEST_ROOT/manifest-before" "$TEST_ROOT/Cargo.toml"
cmp "$TEST_ROOT/lock-before" "$TEST_ROOT/Cargo.lock"
python3 - "$TEST_ROOT" <<'PY'
from pathlib import Path
import sys
root = Path(sys.argv[1])
lock = root.joinpath("Cargo.lock").read_text()
assert 'version = "1.2.3"' in lock
assert 'version = "9.9.9"\nsource = "registry+' in lock
assert 'checksum = "do-not-change"' in lock
assert 'dependencies = ["example 9.9.9"]' in lock
assert 'version.workspace = true' in root.joinpath("crates/example/Cargo.toml").read_text()
PY
for malformed in '' 1 1.2 01.2.3 1.2.03 1.2.3-rc.1 1.2.3+build '1.2.3/../../' '1.2.3 extra'; do
  reject bash "$TEST_ROOT/scripts/set-rust-version.sh" "$malformed"
  cmp "$TEST_ROOT/manifest-before" "$TEST_ROOT/Cargo.toml"
  cmp "$TEST_ROOT/lock-before" "$TEST_ROOT/Cargo.lock"
done
reject bash "$TEST_ROOT/scripts/next-version.sh" unknown
reject bash "$TEST_ROOT/scripts/next-version.sh" patch title extra
reject bash "$TEST_ROOT/scripts/set-rust-version.sh"
sed -i 's/1.2.3/not-a-version/' "$TEST_ROOT/Cargo.toml"
reject bash "$TEST_ROOT/scripts/next-version.sh" patch
cp "$TEST_ROOT/manifest-before" "$TEST_ROOT/Cargo.toml"
sed -i 's/name = "example"/name = "different"/' "$TEST_ROOT/Cargo.lock"
cp "$TEST_ROOT/Cargo.lock" "$TEST_ROOT/invalid-lock-before"
reject bash "$TEST_ROOT/scripts/set-rust-version.sh" 2.0.0
cmp "$TEST_ROOT/manifest-before" "$TEST_ROOT/Cargo.toml"
cmp "$TEST_ROOT/invalid-lock-before" "$TEST_ROOT/Cargo.lock"
echo "version planner and setter tests passed"
