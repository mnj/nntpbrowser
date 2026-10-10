#!/usr/bin/env bash
# Execute the manifest generator and bundle contract without downloading an SDK.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/nntpbrowser-flatpak-test.XXXXXXXX")"
trap 'rm -rf -- "$TEST_ROOT"' EXIT
mkdir -p "$TEST_ROOT/"{packaging,dist,bin,payload/nntpbrowser/bin}
cp "$ROOT_DIR/packaging/build-flatpak.sh" "$TEST_ROOT/packaging/"
case "$(uname -m)" in x86_64) ARCH=amd64 ;; aarch64) ARCH=arm64 ;; *) exit 2 ;; esac
printf 'fixture binary\n' > "$TEST_ROOT/payload/nntpbrowser/bin/nntpbrowser"
tar -czf "$TEST_ROOT/dist/nntpbrowser_3.4.5_linux_${ARCH}.tar.gz" -C "$TEST_ROOT/payload" nntpbrowser
cat > "$TEST_ROOT/bin/flatpak" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "$TEST_ROOT/flatpak-calls"
case "$1" in
  remote-add|install) ;;
  build-bundle)
    [[ "$*" == *'--runtime-repo=https://flathub.org/repo/flathub.flatpakrepo'* ]]
    [[ "${@: -2:1}" == io.github.mnj.NNTPBrowser && "${@: -1}" == stable ]]
    printf 'fixture bundle\n' > "${@: -3:1}"
    ;;
  *) exit 3 ;;
esac
EOF
cat > "$TEST_ROOT/bin/flatpak-builder" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
python3 - "${@: -1}" "$TEST_ROOT" "$@" <<'PY'
import json, pathlib, sys, tarfile
manifest_path = pathlib.Path(sys.argv[1])
arguments = sys.argv[3:]
assert '--default-branch=stable' in arguments
state_argument, = [arg for arg in arguments if arg.startswith('--state-dir=')]
repo_argument, = [arg for arg in arguments if arg.startswith('--repo=')]
assert pathlib.Path(state_argument.split('=', 1)[1]) == manifest_path.parent / 'state'
assert pathlib.Path(repo_argument.split('=', 1)[1]) == manifest_path.parent / 'repo'
assert pathlib.Path(arguments[-2]) == manifest_path.parent / 'build'
assert manifest_path.parent.name.startswith('nntpbrowser-flatpak.')
manifest = json.loads(manifest_path.read_text())
assert manifest['app-id'] == 'io.github.mnj.NNTPBrowser'
assert manifest['runtime'] == 'org.freedesktop.Platform'
assert manifest['sdk'] == 'org.freedesktop.Sdk'
assert manifest['runtime-version'] == '26.08'
assert manifest['command'] == 'nntpbrowser'
assert manifest['rename-desktop-file'] == 'nntpbrowser.desktop'
assert manifest['rename-icon'] == 'nntpbrowser'
assert set(manifest['finish-args']) == {
    '--share=network', '--share=ipc', '--socket=wayland',
    '--socket=fallback-x11', '--device=dri', '--filesystem=xdg-download'}
module, = manifest['modules']
assert module['buildsystem'] == 'simple'
assert module['build-commands'] == ['cp -a bin /app/', 'cp -a share /app/']
source, = module['sources']
assert source['type'] == 'archive'
archive = pathlib.Path(source['path'])
assert archive.parent == pathlib.Path(sys.argv[2]) / 'dist'
with tarfile.open(archive) as payload:
    assert 'nntpbrowser/bin/nntpbrowser' in payload.getnames()
pathlib.Path(sys.argv[2], 'manifest-checked').touch()
PY
EOF
chmod +x "$TEST_ROOT/bin/flatpak" "$TEST_ROOT/bin/flatpak-builder"
for tool in eu-strip eu-elfcompress; do
  printf '#!/usr/bin/env bash\nexit 0\n' > "$TEST_ROOT/bin/$tool"
  chmod +x "$TEST_ROOT/bin/$tool"
done
export TEST_ROOT PATH="$TEST_ROOT/bin:$PATH"
bash "$TEST_ROOT/packaging/build-flatpak.sh" 3.4.5 "$ARCH"
[[ -f "$TEST_ROOT/manifest-checked" ]]
[[ "$(cat "$TEST_ROOT/dist/nntpbrowser_3.4.5_linux_${ARCH}.flatpak")" == 'fixture bundle' ]]
[[ "$(wc -l < "$TEST_ROOT/flatpak-calls")" -eq 3 ]]
for args in 'invalid amd64' '3.4.5 mips' '01.2.3 amd64'; do
  read -r version arch <<< "$args"
  if bash "$TEST_ROOT/packaging/build-flatpak.sh" "$version" "$arch" > /dev/null 2>&1; then
    echo 'invalid Flatpak arguments accepted' >&2; exit 1
  fi
done
# Missing elfutils must fail before any runtime downloads or build operations.
mkdir "$TEST_ROOT/missing-tool-bin"
for tool in dirname flatpak flatpak-builder python3 eu-strip; do
  ln -s "$(command -v "$tool")" "$TEST_ROOT/missing-tool-bin/$tool"
done
if PATH="$TEST_ROOT/missing-tool-bin" /bin/bash "$TEST_ROOT/packaging/build-flatpak.sh" \
  3.4.5 "$ARCH" > "$TEST_ROOT/missing-tool.log" 2>&1; then
  echo 'missing elfutils accepted' >&2; exit 1
fi
[[ "$(cat "$TEST_ROOT/missing-tool.log")" == 'eu-elfcompress is required' ]]
[[ "$(wc -l < "$TEST_ROOT/flatpak-calls")" -eq 3 ]]
echo 'Flatpak manifest, bundle naming, permissions and validation tests passed'
