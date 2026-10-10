# Dependencies and distribution notices

NNTPBrowser's own code is licensed under [MIT](../LICENSE). Its dependencies,
embedded fonts and trust-store data retain their respective licenses. An MIT
application does not make every component MIT-licensed.

This inventory was reviewed on **2026-10-10** against `Cargo.lock`, resolved
Cargo metadata, local registry manifests and license files, selected bundled
native sources, and the embedded fonts' metadata. It records the evidence and
distribution requirements; it is not legal certification or a generated release
notice bundle.

## Scope and feature choices

The feature cleanup reduced the all-platform lockfile from **508 to 500 external
packages**, without adding packages or changing retained package versions. It
removed `aws-lc-rs`, `aws-lc-sys`, `cmake`, `dunce`, `fs_extra`, `iana-time-zone`,
`iana-time-zone-haiku` and `zopfli`. These totals exclude the four workspace
packages and include platform-specific and development dependencies. They are
not counts of libraries shipped in one executable.

At the feature-cleanup review, the Linux application dependency graph contained **347 external
packages** when traversing normal and build edges with default features, or
**355** with all application features. Build tools and procedural macros do not
all become executable code; generated bindings, embedded data and statically
compiled native libraries still need consideration.

The retained libraries provide established implementations:

| Area | Current choice |
| --- | --- |
| TLS and authentication crypto | `rustls`/`tokio-rustls` with Ring, TLS 1.2 and logging retained; the unused default AWS-LC provider is disabled. |
| ZIP extraction | `zip` with `deflate-flate2` and `flate2`; the extra Zopfli compression dependency is removed. |
| 7z extraction | `sevenz-rust2` retains `aes256`, `bzip2`, `ppmd` and `util`; unused compression features are disabled. |
| Dates | `chrono` retains `std`; unused local-clock/time-zone discovery is disabled. |
| Platform application paths | `directories` remains in the app, preserving platform conventions. Its unused engine dependency is removed. |
| Storage | `rusqlite` with bundled SQLite and `zstd` remain. |
| Images and UI | `image` retains JPEG, PNG, GIF and WebP; the egui/eframe UI stack remains. |

Unused direct dependencies on engine `directories`, NNTP `rustls-pki-types`,
the app fixture's optional `store` edge and workspace `bytes` were removed.
Some of those packages remain as required transitive dependencies. This cleanup
does not replace maintained libraries with bespoke crypto, archive or platform
code. Possible future substitutions, such as replacing a channel library with
standard-library channels, require their own behavior review.

## Licenses needing particular attention

The current Cargo package license expressions provide no mandatory GPL or LGPL
selection. This statement excludes the separately packaged AppImage runtime,
which statically links LGPL libfuse; its notices and corresponding source/build
materials must accompany that format. `self_cell 1.3.0` offers Apache-2.0 as an alternative to GPL-2.0;
the UEFI-specific `r-efi` packages offer MIT/Apache-2.0 alternatives to LGPL;
vendored Zstandard permits BSD as an alternative to GPL. Select and record the
permissive alternative rather than interpreting an `OR` expression as requiring
both licenses. Conversely, `AND` expressions require all applicable terms.

| Component | Distribution considerations |
| --- | --- |
| `option-ext 0.2.0` through `directories → dirs-sys` | MPL-2.0 is the one MPL-only package. Binary recipients must be told how to obtain its covered source; that source and any modifications to covered files remain MPL. Separate application files may remain MIT. See [Mozilla's MPL guidance](https://www.mozilla.org/en-US/MPL/2.0/FAQ/) and [sections 3.1–3.3](https://www.mozilla.org/en-US/MPL/2.0/). |
| Application assets: Source Sans 3 and JetBrains Mono | These fonts are embedded directly by the UI. Preserve their SIL OFL 1.1 texts and naming restrictions in `crates/app/assets/OFL-*.txt`; the package notice collector includes both alongside Cargo font notices. |
| `epaint_default_fonts 0.36.2` | Embedded fonts have additional terms: Noto Emoji uses [SIL OFL 1.1](https://openfontlicense.org/open-font-license-official-text/); Ubuntu Light uses the [Ubuntu Font Licence](https://canonical.com/legal/font-licence); Hack includes both MIT and Bitstream Vera notices; emoji-icon-font is MIT. Preserve their copyright/license notices and font-specific naming restrictions. Font licenses are not replaced by the application's MIT license. |
| `webpki-roots 1.0.9` | Its embedded trust-anchor data uses [CDLA-Permissive-2.0](https://cdla.dev/permissive-2-0/), requiring the agreement text when sharing the data. It is not MPL in this resolved version. |
| `ring 0.17.14` | Apache-2.0 and ISC apply, with additional included origins. Preserve Ring's comprehensive license files, including BoringSSL, Fiat and once_cell notices. |
| macOS `dispatch 0.2.0` | Its [original manifest at revision `82d6c7a`](https://github.com/SSheldon/rust-dispatch/blob/82d6c7a5b75dc0c71c3f46f87bb6c16a476f7748/Cargo.toml) declares MIT and author Steven Sheldon, but the published crate and pinned repository omit a separate license file. The collector preserves that byte-identical declaration, attribution and explicit explanation; it includes the [canonical MIT template](https://opensource.org/license/mit) separately without presenting it as an upstream-supplied license or inventing a copyright year. This exception requires the exact package/version/repository/revision and manifest SHA256 `05c60b829931ef7f1f130735843c11f5cf9cc69546eca5fc941d014781c68e31`; it does not infer licenses for other packages. |
| `zstd-safe`/`zstd-sys` and bundled Zstandard | Preserve the distinct wrapper, generated-binding and native-library BSD notices. The native source's GPL alternative does not require choosing GPL. |
| `libsqlite3-sys 0.30.1` | The Rust wrapper is MIT; the bundled SQLite core is [public domain](https://sqlite.org/copyright.html). SQLCipher source present in the crate is not enabled by this application's `bundled` feature. |
| Apache-only, BSD, ISC, Zlib, Unicode and bzip2 components | Retain the applicable license, copyright, attribution and disclaimer terms. [Apache-2.0](https://www.apache.org/licenses/LICENSE-2.0) also requires applicable upstream NOTICE content and notices identifying modified files. Unicode data/software notices remain relevant even inside a dual-licensed wrapper. |
| Boost-licensed `xxhash-rust` and platform crates | Boost's source-notice requirement has an exception for solely machine-executable object code. A source distribution still retains the notices. |

Other notable scopes include test-only `rcgen` and its certificate-generation
dependencies, Windows clipboard/socket dependencies, macOS framework bindings
and WASM/UEFI entries. Audit the selected target/features when packaging; the
presence of a platform crate in `Cargo.lock` does not mean it is in the Linux
binary.

## Release materials

The later PAR2 implementation expanded the default Linux normal/build graph;
release notice generation has been verified for its **360 external packages**.

Native packaging invokes `scripts/collect-notices.py` against Cargo metadata
filtered for the actual build target. It copies upstream license, attribution,
font and native-source notices into a `third-party` directory, records selected
license alternatives and versioned sources in its index, and includes the exact
MPL-covered `option-ext` source. macOS universal packages include both target
graphs. Missing required upstream texts fail packaging, apart from the explicitly
guarded and documented legacy `dispatch` declaration above. Some workspace crates
omit root license files in their registry tarball; the collector retrieves
those texts from the crate's pinned upstream Git commit and records provenance.
The repository's MIT `LICENSE` alone is not the distribution bundle.

AppImage also includes its separately pinned runtime's notices and corresponding
source/build materials. Recheck these materials when upgrading the runtime.
See [release packaging](releases.md) for formats and build commands.

Useful primary files in the resolved Cargo registry sources include:

- `epaint_default_fonts-0.36.2/fonts/{OFL.txt,UFL.txt,Hack-Regular.txt,emoji-icon-font-mit-license.txt}` and the font metadata. Noto Emoji identifies ©2013 Google; Ubuntu Light identifies ©2011 Canonical.
- `ring-0.17.14/{LICENSE,LICENSE-BoringSSL,LICENSE-other-bits,third_party/fiat/LICENSE}` and `src/polyfill/once_cell/LICENSE-*`.
- `zstd-sys-2.1.1+zstd.1.5.7/{LICENSE,LICENSE.BSD-3-Clause,zstd/LICENSE}`.
- `option-ext-0.2.0/LICENSE.txt`, `webpki-roots-1.0.9/LICENSE`, ICU `LICENSE` files and `libbz2-rs-sys`'s `LICENSE`.
- `cfg_aliases-0.2.2/NOTICES.md` for build tooling and `uds_windows-1.2.1/THIRDPARTYNOTICES` for Windows.

The optional `ffmpeg`, `unrar` and `unar` tools are invoked as external programs;
they are not Cargo dependencies or bundled by this repository. Bundling those
executables requires a separate review of their exact builds and distribution
terms. In particular, FFmpeg's LGPL/GPL selection depends on its enabled parts:
see [FFmpeg's licensing guidance](https://ffmpeg.org/legal.html). System runtime
libraries and graphics drivers are also outside the Cargo package inventory.

## Reproducing the inventory

```sh
cargo metadata --offline --format-version 1 --all-features
cargo tree --offline -p app --target x86_64-unknown-linux-gnu --edges normal,build
cargo tree --offline -p app --target x86_64-unknown-linux-gnu --edges normal,build --all-features
```

Offline metadata requires the relevant platform sources to have been fetched.
Inspect the resolved versions' manifest `license`/`license-file` fields and
actual license, notice, native-source and embedded-asset files. Recheck the
inventory when dependencies, features, target or release packaging change.
