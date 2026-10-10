# Repository guidance

NNTPBrowser is a native Rust/egui newsreader. Read `plan.md` for architecture and `docs/nntp-support.md` for protocol coverage. Keep those documents aligned with implemented, tested behavior.

## Structure and ownership

- `crates/nntp`: NNTP commands, session state, TLS/authentication, compression, connection pools.
- `crates/store`: SQLite catalog and immutable compressed OSF header cache.
- `crates/engine`: background runtime, header fetching, multipart assembly, downloads and temporary media previews.
- `crates/app`: native interface and headless CLI. Keep rendering modules focused on a single pane or dialog.

Split growing Rust modules by responsibility. Coordinate file ownership when using parallel agents, since they share the working tree. Do not use an egui/browser UI thread for network access or image/video decoding.

## Product behavior

- Browse newsgroups per server; use Articles for the list/body split and Gallery for media previews. Left-align tree row names with consistent indentation, while keeping the whole row selectable.
- Gallery previews are temporary RAM data. Browsing must not create download jobs, saved media or thumbnails on disk. Saving is explicit and appears in Downloads → Completed. Preserve bounded byte caches and off-thread decoding.
- Request Gallery previews for visible tiles only. Cancel unfinished offscreen requests when scrolling; retain completed memory previews for reuse. An open media viewer takes request priority.
- Gallery concurrency follows configured server connection limits, with no separate four-preview ceiling. Batch header lookups for visible files and fetch multipart windows concurrently. Preserve ordered UU assembly and a shared encoded-byte bound across each preview’s readers.
- The top bar contains Articles and Gallery plus the active group name; selection stays in the sidebar. Downloads combines transfers and saved files. Folder deletion requires a concrete confirmation and backend ownership/shared-path checks.
- Header fetching has one primary action with Latest/New/All scopes. Keep received/stored counts separate from scanned range progress.
- Manual downloads use automatic PAR2 recovery: retain small indexes, defer auxiliary volumes, discover matching missing headers within bounded ranges, and fetch enough validated repair blocks. Keep source identity, candidates and completed searches durable per job. Explicit PAR2-only downloads retain their requested files. Recovery never publishes partial group-cache spans, retargets deleted source IDs, or marks unresolved payload complete. Show recovery stages and preserve pause/cancel generation checks.
- Selecting a group without cached header spans fetches its latest 10,000 headers automatically. Check the header store in the worker; empty filtered results and pagination must never trigger this fetch. Preserve the active search/date query after completion.
- Date filtering searches the cached group before pagination. Group-list sorting must preserve stable row identities and subscription bindings. Use subtle capped indentation with muted dotted namespaces and a bright final component in the same normal font size; preserve global sorting, aligned numeric columns and full names on hover. Cache name layout metadata when the picker indexes rebuild. The separate group-layout comparison is a proof of concept using a bounded snapshot of loaded filtered groups, with sample data as a fallback. It must not change subscriptions, network state or saved settings.
- Unsubscribe deletes the group database row and its derived header/body/thumbnail caches. Preserve explicit downloads and their job history. Invalidate pending previews and decodes before deleted IDs can be reused.
- Server connection limits include header and body sockets. Pipeline replies remain ordered. Pause/cancel and context changes must discard stale asynchronous results.
- Errors remain inspectable through history. Scrub credentials before recording or displaying diagnostics.
- Network speed uses a bounded five-second trailing average; socket and activity samples remain frequent. The Speed tooltip shows humanized session received/sent network totals and consumed NNTP data after decompression; retain final counters after pool closure. The Connections tooltip shows live per-socket setup/wait/traffic states, send/receive rates and byte totals; remove closed socket rows without retaining socket handles or protocol contents. Network status distinguishes open sockets from busy connections and configured limits. Capture short request bursts between samples; idle reusable sockets are still open.
- Keep the status bar in one row with inline labels, values and task progress. Keep each statistic at a fixed width so changing values and units do not move neighboring statistics. Show header progress and cancellation in the footer; cancellation must interrupt network waits, preserve committed batches and leave unfinished ranges fetchable.
- Gallery tile size is adjustable and persisted. Keep virtual row pitch and visible-tile requests aligned with the selected size, and retain completed RAM previews during resizing.
- Show this process’s resident RAM and CPU usage in the status bar. Sample off the UI thread; CPU percentages count one fully busy core as 100%.
- Settings separate application appearance/network preferences from server-specific connection/account/download configuration.

## Storage and protocol

Retain SQLite for transactional metadata and compressed OSF files for overview records unless evidence supports a change. Preserve migration compatibility, the exclusive data-directory lock, crash recovery journals and historical gaps. Do not silently skip corrupted header blocks.

Header blocks use zstd level 3; selected article bodies use checksummed zstd level 1 with bounded decompression and lazy legacy-cache migration. Saved media retains its native format.

The target is complete newsreader protocol support; posting and server-to-server feeds are outside scope. Refer to primary RFCs when extending NNTP. Add transcript tests for response framing, capabilities and state transitions; document implemented coverage separately from exhaustive conformance claims. Never weaken TLS verification or fall back to plaintext credentials when configured TLS fails.

## Verification

For changes affecting behavior, run appropriate tests, then the workspace checks before committing:

```sh
cargo fmt --all -- --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo build -p app --bin nntpbrowser
```

Protocol tests use local fake servers; TLS tests use local test certificates. Use isolated temporary data for integration and native UI preview fixtures. Do not use a provider account or change the user's live cache as part of automated checks. Verify layout changes with the `ui-preview` example at 1040x680 and 1280x820; screenshots belong in `docs/` when useful.

Do not commit provider credentials, personal downloads, caches, temporary logs or build output. Commit requested changes only after the normal binary builds and relevant checks pass.

## Publishing

`site/` is the static GitHub Pages project site. Keep its feature claims aligned
with the app, use project-relative asset links, and run `python3 scripts/check-site.py`
after changes. Release/CI/Pages workflows live in `.github/workflows/`; release
versioning and native packaging are documented in `docs/releases.md`.
Validate script changes with `bash scripts/tests/version.sh` and workflows with
`actionlint`. Preserve numbered release assets, rolling download names, locked
dependency versions and distributed third-party notices/covered source.
