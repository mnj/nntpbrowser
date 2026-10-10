# Releases and GitHub Pages

The publishing setup follows the rMail repository: a successful current `main`
build publishes a numbered release and a rolling `latest` release. GitHub Pages
deploys the static project site in `site/` through GitHub Actions.

## Release versions

The first release uses the workspace version in `Cargo.toml`, currently `0.1.0`.
Later releases increment the highest stable `vX.Y.Z` tag. A manually increased
workspace version ahead of that tag acts as a floor and is released unchanged.
The build stamps the selected version into the workspace manifest and lockfile
inside the runner; it does not commit version changes back to `main`.

Merged pull requests can control the increment with `release:major`,
`release:minor` or `release:patch`. Without a label, a `feat:` title increments
the minor version, a breaking Conventional Commit title increments the major
version, and other titles increment the patch version. Direct pushes use the
commit title. `release:skip` or `[skip release]` in the pushed commit message
skips publication while CI still runs. The Release workflow also supports a
manual choice of increment on `main`.

CI runs formatting, all-feature workspace tests, strict Clippy and the normal
application build, plus release-script and site checks. Release jobs build Linux amd64 and arm64,
Windows x64/x86/arm64, universal macOS, and FreeBSD x86_64 packages. Linux uses
Ubuntu 22.04 for its glibc baseline. All required package jobs must succeed
before publication. Old runs cannot publish over a newer `main` commit.

## Downloads

The native package formats follow [PhotoCraft](https://github.com/storytold/photocraft/releases).
Each numbered release supplies these packages and `SHA256SUMS`:

- `nntpbrowser_X.Y.Z_amd64.deb` and `nntpbrowser_X.Y.Z_arm64.deb`
- Linux amd64/arm64 portable `.tar.gz`, `.AppImage` with `.zsync`, `.rpm`, and
  `.flatpak` packages named `nntpbrowser_X.Y.Z_linux_ARCH.EXT`
- Windows x64/x86/arm64 `.msi` installers and `_portable.zip` archives named
  `nntpbrowser_X.Y.Z_windows_ARCH...`
- `nntpbrowser_X.Y.Z_macos_universal.dmg` and
  `nntpbrowser-cli_X.Y.Z_macos_universal.zip`
- `nntpbrowser_X.Y.Z_freebsd_x86_64.tar.gz`

Numbered releases retain their published assets. The rolling `latest` tag moves
to the newest successfully published commit and replaces its assets using
`latest` in place of `X.Y.Z`, providing stable links for the project site.
Publication retries preserve numbered assets and can finish a failed rolling
release update. Packages include the app license, third-party notices and
covered-source materials; see [the dependency inventory](dependencies.md).

Debian/RPM packages install the binary and desktop launcher. Portable archives
include the binary, launcher where applicable, icon and documentation. Check the archive's runtime notes for
graphics libraries and optional external extraction/video tools. Packages do
not configure servers or touch personal caches. Use `nntpbrowser --version` to
inspect the embedded release version. Windows installers are unsigned; the
macOS app uses an ad-hoc signature and is not notarized. Developer certificates
are not required by these workflows.

A browser/WebAssembly bundle is outside this native release setup: NNTPBrowser
uses direct NNTP sockets and local filesystem storage, which would need a
browser-compatible backend.

## Pages

The site is published at <https://mnj.github.io/nntpbrowser/>. Its workflow
validates local links and assets, uploads only `site/`, and deploys through the
`github-pages` environment. Repository Settings → Pages must use **GitHub
Actions** as its publishing source. Site or workflow changes on `main` trigger
deployment; manual deployment uses the current `main` site.

## Local validation

```sh
bash scripts/tests/version.sh
python3 scripts/check-site.py
python3 scripts/tests/notices.py
bash packaging/tests/linux.sh
cargo fmt --all -- --check
cargo test --workspace --all-features --locked
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo build --locked -p app --bin nntpbrowser
```

Build a Linux release on its native architecture with
`packaging/build-linux.sh 0.1.0 amd64` (or `arm64`). This stamps the checkout's
version and writes packages to ignored `dist/`; use a disposable checkout when
testing a different version. The build also needs the packaging tools listed in
the workflow; run `bash packaging/tests/artifacts-linux.sh VERSION ARCH` to
validate the real package contents and headless launchers. The workflow checks themselves can be validated
with `actionlint`.

Windows packages include `nntpbrowser.exe` for desktop launch and
`nntpbrowser-cli.exe` for terminal commands. Both share the same dispatcher.
Other platforms use `nntpbrowser` for both modes.
