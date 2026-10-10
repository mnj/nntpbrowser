# NNTPBrowser

Cross-platform binary Usenet reader. Rust, native egui, our own NNTP stack. Browse binary newsgroups, download files, and scroll a thumbnail grid of articles. Several servers, one catalog.

The project license is MIT. Every crate we link must be MIT, Apache-2.0, BSD, ISC, Zlib, or a public-domain equivalent. GPL, LGPL, and AGPL are out, including `par2-rs`. An optional external `unrar` or `unar` binary is spawned, not linked, and is not shipped.

This file is the product plan as agreed so far. The first UI sketch (server fields on the main window, a dotted tree of every group) was rejected and is replaced by the UI section below.

## Implementation status (2026-10-10)

- `crates/nntp` — streaming `LIST ACTIVE`, compressed overview fetching, TLS/auth, pool/failover. Connection code is split into transport and overview modules. Retry after a partial group-list callback is disabled, and reconnect restores the active group.
- `crates/store` — compressed OSF1 headers plus SQLite metadata/queue. Catalog and header storage are split by responsibility. Compaction streams bounded blocks and preserves gaps. A durable per-group journal makes publication across OSF and SQLite recoverable and idempotent.
- `crates/engine` — modular commands/events, server discovery, header synchronization, queue, downloads and previews. Latest-N/all-retained header fetch fills missing spans without reimporting existing records. Bulk downloads use one sequential header scan and a SQLite temporary selection table.
- `crates/app` — rebuilt native article/gallery workspace, collapsible subscribed-group sidebar, streaming subscription dialog, settings, downloads/library, and a headless CLI. UI modules separate events/actions, navigation, sidebar, feed, table, details, gallery, subscriptions, settings, downloads, textures and library.

User-facing terminology is **articles** and **files**, never releases. Multipart headers are grouped internally to assemble files; `post` and grouping-key names remain internal storage terms.

No-argument startup launches the GUI. Subcommands dispatch before initializing a window. An optional `ui-preview` feature supplies isolated offline fixtures for native layout review; see README.md.

The OSF1 layout and farm order remain stable. SQLite schema v2 migrates existing v1 catalogs transactionally, preserving credentials while adding explicit STARTTLS configuration. The custom header cache is retained; SQLite continues to own searchable metadata and download state. `crates/store/examples/storage_bench.rs` provides a reproducible synthetic comparison against storing individual overview rows in SQLite. Compressed SQLite BLOB blocks have not been benchmarked.

## Decisions

**NNTP is ours.** Our async client exposes the RFC 3977 newsreader command set: article retrieval/current selection and navigation, group discovery, overview/header queries, LIST facilities and session commands. It supports implicit TLS or required STARTTLS, advertised SCRAM-SHA-256/PLAIN with USER/PASS fallback, and RFC 8054 COMPRESS DEFLATE. Provider-specific compressed-header commands (`XZVER`, `XFEATURE COMPRESS GZIP`) remain supported. Posting/feed commands and arbitrary SASL mechanisms are outside the agreed newsreader scope. This is tested feature coverage without a claim of exhaustive RFC conformance; see [the support matrix](docs/nntp-support.md). No third-party NNTP protocol stack is linked. The protocol crate has no UI and no database.

**yEnc, NZB, PAR2, egui are libraries.**

- yEnc on the wire inside `XZVER` is decoded in the nntp crate (`crc32fast`). Article bodies use `yenc-simd` (MIT, scalar fallback) in the engine only, behind our own type so the crate can be swapped. The nntp crate does not depend on `bytes` or `yenc-simd`.
- NZB: parsed with `quick-xml` in our code. The schema is small, and the published NZB stacks drag in someone else's NNTP client.
- PAR2: `rust-par2` 0.1.3 only (MIT OR Apache-2.0). Real API: `parse(path) -> Par2FileSet`, `verify(&file_set, dir) -> VerifyResult`, `repair(&file_set, dir) -> Result<RepairResult, RepairError>`. `RepairResult` is `{ success, blocks_repaired, files_repaired, message }`. There is no `files_failed` and no create API, so there is no PAR2 fixture round-trip. We do not link a GPL PAR2 library.
- Compression: `flate2` with the pure-Rust backend (MIT OR Apache-2.0) for raw DEFLATE, zlib, and gzip. `lzma-rs` 0.3 for an xz payload. No `liblzma` or `libz` system dependency. `flate2`'s rust backend has no `Decompress::new_gzip`; gzip uses `flate2::write::GzDecoder`.
- Header blocks: `zstd` 0.13 at level 3, `xxhash-rust` 0.8 `xxh3`.
- Archives: `zip` 2.4.2 (`ZipArchive::by_index_decrypt(index, password: &[u8])`; default features off, `deflate-flate2` and `flate2` enable decoding without Zopfli encoding) and `sevenz-rust2` 0.20.2 (default features off, `aes256`, `bzip2`, `ppmd`, `util` preserve extraction formats and passwords without compression APIs).
- UI: `eframe` / `egui` 0.36. Desktop Linux, Windows, macOS from one window. Not a web UI. egui 0.36 names that matter: the app method is `ui`, not `update`; `SidePanel` is `Panel`; `RichText` is not a widget; style is `style_mut_of(Theme::Dark)`; there is no `SelectableLabel` (use `Button`); `allocate_ui` inherits the parent layout, so a horizontal parent will lay form fields out in one row and clip them.

**Storage is hybrid.** Keep compressed overview blocks in the existing OSF cache and transactional metadata in SQLite. Per-header SQLite rows with indexes were larger and slower in the local synthetic benchmark; compressed SQLite BLOB blocks remain a viable alternative that has not been benchmarked.

Overview is a firehose. One refresh of a busy binary group is hundreds of thousands of tab-separated lines, and a retained group can be millions. We scan each article once, group it, and almost never look it up by SQL. The benchmark compares compressed OSF blocks against per-header SQLite rows, with and without a message-ID index; it does not establish an advantage over compressed blocks inside SQLite.

Servers, subscriptions, grouped articles, download jobs and queued segment status remain in SQLite. The OSF manifest owns the durable header watermark and stored spans; the subscription watermark in SQLite mirrors that progress.

Payloads are files. Header OSF blocks use zstd level 3. Selected article-body cache writes use fast zstd level 1 with checksums, a 4 MiB decompressed-body cap and 4 MiB decoder-window limit; corrupt or truncated entries are cache misses and can be fetched again. Legacy raw `.body` cache files remain readable and migrate to zstd lazily when used; failed migration preserves the cache hit. SQLite stays an ordinary live database for transactions and indexed queries. Explicitly saved JPEG/PNG/video/archive payloads keep their native formats; saved-download thumbnails remain JPEG. Gallery media and decoded previews stay in RAM until Save. Older `media-cache` jobs are cancelled on startup if unfinished; completed legacy files remain until explicitly removed in Downloads.

**Reader, not a poster.** No POST/IHAVE. Article payloads decode yEnc, classic uuencode and POSIX begin-base64, including ordered line-boundary multipart UU fragments. UU mode bits are validated but never applied. Other encodings such as BinHex, xxencode and arbitrary MIME attachments are not implemented. No full-text search of every historical header. Search runs over grouped articles already in SQLite (`title LIKE`, with `ESCAPE '\'`). Local search only, not a Usenet indexer.

**Passwords** sit in the SQLite `server` row. No OS keyring in this product. The UI and the CLI never print a password. A failed `PASS` is `Error::Auth`; the server's text is discarded. Panic and log lines must not `Debug`-print a `ServerRow`.

**One engine, two fronts.** The window and the CLI send the same `Command` values to the same `Handle` and the same data directory. If a click can do it, a command can do it. No behavior that exists only behind a widget.

## Layout

```
NNTPBrowser/
  LICENSE                 # MIT
  Cargo.toml              # workspace, edition 2021, publish = false, resolver 2
  plan.md
  crates/nntp/            # protocol, pool, failover, compression
  crates/store/           # OSF1 header files + SQLite catalog
  crates/engine/          # sync, grouping, download, yenc, nzb, par2, thumbs
  crates/app/             # eframe UI + headless CLI
    assets/               # OFL Source Sans 3 (Regular, Semibold) and JetBrains Mono Regular
```

Tokio runtime lives on a background thread. The UI thread only reads snapshots and sends commands (`crossbeam-channel` into the runtime, `tokio::sync::mpsc` unbounded for engine events back). `Handle::poll` is `try_recv` under a mutex. The egui mutex is never held across an await.

Worker loop: drain deferred work and `cmd_rx` (a command may await a full header sync), then schedule or collect concurrent download windows. The worker owns catalog updates and decoded file writes; network tasks return BODY results. With no work, sleep 40 ms. During transfers, poll completions and controls in bounded intervals. The UI requests a repaint after 80 ms. `#[tokio::test]` is current-thread, so a fake NNTP listener dies if the test blocks on `std::thread::sleep`. Waits are `tokio::time::sleep`. A test waiter keeps unmatched events in an inbox. Draining until the first match drops later events from the same `poll()` batch.

Data directory: `directories::ProjectDirs::from("org", "nntpbrowser", "NNTPBrowser").data_dir()`, else `nntpbrowser-data`. The CLI and the GUI share it.

Workspace deps: tokio 1 (`rt-multi-thread`, `macros`, `io-util`, `net`, `time`, `sync`), thiserror 2, serde 1 with derive, serde_json 1, flate2 rust backend, chrono 0.4 with default features off plus `std`. `rusqlite` 0.32 bundled. TLS uses only the Ring provider; `tokio-rustls` defaults are disabled with `ring`, `tls12`, and `logging` retained. See [dependency review](docs/dependencies.md) for feature choices and third-party license obligations.

## NNTP

One connection is a state machine: implicit TLS before greeting, or greeting/capabilities followed by required STARTTLS; fresh encrypted capabilities; advertised SASL or USER/PASS authentication; fresh authenticated capabilities; then reader commands. Auth finishes before `COMPRESS`. STARTTLS never falls back to plaintext and never sends credentials before the upgrade.

Transport is `tokio` TCP plus `tokio-rustls` 0.26 (`rustls` 0.23, ring, `webpki-roots`). Implicit TLS on 563 is the default (`TlsMode::Implicit`). Plain mode is explicit (`TlsMode::Plain`); required STARTTLS is `TlsMode::StartTls` and works on any configured port. The public `connect_with_tls_config` API supports explicit roots/client certificates; ordinary connections validate against public roots. `TCP_NODELAY` is on. Endpoint timeout is 8 s. Reconnect is about 3 attempts, about 50 ms apart. Re-auth on every new connection.

Codec:

- Read `\r\n` lines. A response is a 3-digit code with optional space-separated text; malformed separators return a protocol error without slicing through UTF-8 characters.
- Core outgoing commands are at most 512 octets including CRLF, with arguments at most 497 octets, checked before compression or network writes. A pipeline validates the whole batch before submitting any command. Core token/range/date grammar is validated, but the command/state matrix is not exhaustive. RFC 4643 SASL exchanges use a separate bounded 32 KiB line limit.
- Multiline only for known codes: 100, 101, 215, 220, 221, 222, 224, 225, 230, 231. Not a blanket 1xx rule. `DATE` is 111 and single-line. `STAT` is 223. `GROUP` is 211. Auth uses 281/283 success, 381 USER/PASS continuation, 383 SASL continuation and error replies. `COMPRESS` success is 206. `XFEATURE` success is 290. `XZVER` success is 224.
- Data ends at a line that is a single `.`. A line that starts with `..` loses one dot.
- Bounded and pipelined BODY return the same dot-unstuffed CRLF representation, including the final line ending and empty lines. Oversize bounded responses require discarding that connection.
- Overview and `LIST ACTIVE` are streamed. Compressed `XZVER` responses remain buffered, so the engine caps overview command ranges at 20,000 article numbers.
- Article numbers are `u64`. Commands print decimal. SQLite stores article numbers as `i64`.
- `LIST ACTIVE` lines are `name high low status` (high, then low).

Capabilities are refreshed after TLS, authentication and successful MODE READER. An unavailable CAPABILITIES response clears old capability state. Remembered DEFLATE strategies require the current session to advertise COMPRESS DEFLATE before enabling compression. Servers advertising `XOVER` without `OVER` use `XOVER`; older servers returning 500/501 to `OVER` also fall back to `XOVER`. A zero estimated `GROUP` count does not hide a valid low–high range.

`MODE READER` is sent only when the capability label is exactly `MODE-READER`. The label `READER` alone does not mean that. It is forbidden once TLS is established; the explicit protocol API enforces this too.

`LIST OVERVIEW.FMT` is read once per connection and maps OVER columns. Structured overview reads the standard seven fields (subject, from, date, message-id, references, bytes, lines). The raw-field API retains extra and empty columns. References remain available at the protocol layer but the binary header store drops them. Lossless `over_bytes`/`hdr_bytes` APIs preserve legacy octets; display string APIs replace invalid UTF-8.

Status mapping: 411 `NoSuchGroup`; 412 `NoGroupSelected`; 420 `NoCurrentArticle`; 421/422 `NoPreviousArticle`/`NoNextArticle`; 423/430 `Missing`; 480/481/482 `Auth`; 500/501 `Unsupported`; 502 `PermissionDenied`; anything else `Protocol`.

Pipelining: the pool default depth is 8, per server. Responses are read in send order, with a bounded-response variant for media caching. `GROUP`, auth, capabilities, and `COMPRESS` are never pipelined. Header sync uses one connection and one bounded `OVER` / `XZVER` range at a time. Downloads run concurrent FIFO windows: up to 64 windows globally, bounded by enabled servers' configured connection counts. Window depth respects configured pipeline depth and an estimated 8 MiB window budget; small queues spread across connections before increasing depth. The byte estimate is a scheduling heuristic, not a hard allocation limit for unrestricted manual downloads. Automatic media previews use a separate RAM-only path whose concurrency follows the enabled servers' connection limits. Visible files and multipart windows fetch concurrently through the shared pool; there is no separate four-preview ceiling. A shared budget bounds successful encoded BODY bytes for each preview to twice its file limit plus 64 KiB, including concurrent readers. Previews never create download jobs or saved files. A fake server that withholds replies until three connections each submit two BODY commands verifies real concurrency and pipelining. Catalog/file writes remain on the worker, with generation checks rejecting stale results after pause/resume.

Pool, per server:

- 1 logical header connection, behind a tokio mutex.
- Idle plain connections can be reused for header or BODY work; BODY connections stay uncompressed.
- A shared semaphore of `max_connections` (default 8) covers header and BODY operations. Idle header sockets are reused or dropped before opening BODY sockets, avoiding an additional reserved socket beyond the configured count.
- Reconnect on dropped sockets and on protocol 400, 401, and 503.
- The pool retries `Io`, `Closed`, `Timeout`, and protocol 400/401/503 once. `430` is not retried inside one pool. Only `Error::Missing` continues to the next farm server. Auth failure does not fall through.

Farm order: stable sort by `(fill, priority)`, then id. A lower priority number is better. Non-fill servers come first. A fill server is used only after every non-fill server returned `Missing` for that message-id. The engine does not call `Farm::new` (that would drop live connections). `Worker::body_from_farm` uses the same order.

Header compression preference, never stacked, and never on a download connection:

1. `XZVER` when the server accepts it.
2. Else `COMPRESS DEFLATE` when `caps.compress_deflate`, then `OVER` or `XOVER`.
3. Else `XFEATURE COMPRESS GZIP`, then `OVER` or `XOVER`.
4. Else plain `OVER` or `XOVER`.

`HeaderStrategy` is `Unknown`, `Xzver`, `CompressDeflate`, `XfeatureGzip`, `Plain`, with `as_str` values `unknown`, `xzver`, `compress-deflate`, `xfeature-gzip`, `plain`. The server row shows which one was negotiated. A `500` on a probe is remembered for that server so later syncs skip it.

Errors the UI can show, after scrubbing passwords: auth failed, TLS failed, group does not exist, article missing on every server, overview unsupported, compression failed, timeout.

Tests use local TCP and certificate-backed TLS peers. They cover the core reader operations and state errors, optional fields, authentication, STARTTLS ordering/reset, certificate/name rejection, RFC SCRAM proofs, FIFO/partial pipelines and compression variants. No user credentials or live provider are used in these tests.

## Compressed headers

"Compressed headers" is two different mechanisms. RFC 8054 compresses the whole connection. The command newsreaders mean is `XZVER`, which is not an RFC. Commercial farms implement `XZVER`. INN-style servers advertise `COMPRESS DEFLATE`. We speak both, and we never stack them on one connection. Download connections stay uncompressed. yEnc bodies are already high-entropy, and deflating them costs CPU for almost no bytes (RFC 8054 section 3).

**RFC 8054 `COMPRESS DEFLATE`.** Capability line is `COMPRESS` plus a list of algorithms. We use `DEFLATE` and ignore names we do not know. Order on the connection is configured TLS, then `AUTHINFO`, then `COMPRESS`. The RFC forbids `AUTHINFO` after compression. `COMPRESS` is not pipelined. On `206 Compression active`, the very next byte in both directions is raw DEFLATE (RFC 1951): zlib header off, no gzip header. `403`, `502`, and `503` leave the connection uncompressed and negotiation tries the remaining strategies without reconnecting. Each direction has one deflate context for the life of the socket. The client sync-flushes after each command so the server can decode it. A full flush is not used. That would drop the dictionary that makes overview text small. Line splitting and dot-stuffing run on the inflated bytes. A compressor or inflater error closes the connection. There is no resync.

**`XZVER` (de facto, not an RFC).** Same article range syntax as `OVER`. Success is `224`. Eweka sends a binary zlib stream immediately after the status, with the dot inside the inflated data. Read through the compression stream end, preserving subsequent response bytes; waiting for a wire dot causes a timeout. Other providers send one yEnc document (`=ybegin` … `=yend`), then the usual lone dot. `size=` is often `-1` and is ignored. The yEnc CRC is checked unless it is absent or `FFFFFFFF`. The decoded bytes are the compressed overview, picked by `inflate_auto`:

- `1F 8B` is gzip.
- `FD 37 7A 58 5A 00` is xz (`lzma-rs`).
- A zlib header (`0x78` and `(CMF * 256 + FLG) % 31 == 0`) is zlib's deflate with an Adler-32 trailer. This is the common Astraweb case.
- Anything else is raw deflate.

Inflated output is normal overview lines, `\n` or `\r\n`, and goes through the same field parser as `OVER`. `XZHDR` is the same wrapping for `HDR`. We do not need it until something wants one header field in bulk.

**`XFEATURE COMPRESS GZIP`.** Sent only when `XZVER` and RFC 8054 are both unavailable. After `290`, detect either a continuous gzip stream or plain statuses followed by independent zlib overview bodies. Eweka uses the latter despite the command name; its inflated body can omit the terminating dot. Client commands stay plain text. End binary bodies at the compression stream boundary.

A live provider check fetched 20,010 headers into an isolated cache through plain `XOVER`, compressed `XZVER` and `XFEATURE`, including restart/backfill. Automated tests retain provider-shaped transcripts and older compression variants. `NNTPBROWSER_PROTOCOL_TRACE=1` logs command labels and status codes without authentication arguments or response content.

## Header store (OSF1)

```
{data_dir}/headers/{server_id}/{group_slug}/
  manifest.json
  r_{low}_{high}.osf
```

`manifest.json` is the list of immutable range files plus the watermark (highest article number fully stored). It is replaced by write-temp-and-rename.

An `.osf` file:

- Magic `b"OSF1"`, version `u16` = 1, `article_low` `u64`, `article_high` `u64`, block count, index offset. `HEADER_LEN` is `usize` 34.
- Blocks of records, each zstd at level 3, default 256 KiB uncompressed. Each block stores uncompressed length, compressed length, and an xxh3-64 of the uncompressed bytes.
- Trailing index: for each block, first article number and file offset. Range lookup binary-searches the index and decodes one block. A full group build reads straight through.

Record, little-endian, length-prefixed UTF-8: article number `u64`, date unix `i64` (`0` if the overview date did not parse; the raw date is kept), bytes `u64`, lines `u32`, message-id, subject, from.

No message-id hash index. NZB downloads already have ids. "Have we seen this article" is answered from SQLite. A bad block raises an error; automatic corruption repair/refetch is not implemented. Overlapping ranges are `Error::Header`. Out-of-order records are `Error::Order`.

Group slug: ASCII alphanumeric, dot, hyphen, underscore, or plus, and the name is not empty, `.`, `..`, or a string containing `..`. Anything else is `g_` plus the full 16 hex digits of xxh3-64 of the name.

Unix directories are `0700`, files `0600`. `Catalog::open` chmods a parent only when that call created the parent. Chmod of an existing directory (including `/tmp`) is `EPERM` and must not be attempted.

Compaction: more than 16 contiguous range files are merged with bounded block memory and the manifest is swapped. Historical gaps remain separate spans so backfill cannot skip unstored articles. Damaged input prevents publication of a compacted replacement. Orphan files are swept at start.

Sync:

1. `GROUP` to read the server high water.
2. Fetch `(watermark + 1)-high` with the compression choice above. Lines stream into one new `.osf` and into the grouper. The `.osf` file stores parsed records in zstd level-3 blocks; this storage compression is independent of wire compression.
3. Write a durable `pending_catalog.json` containing absolute grouped-post/file snapshots before publishing the header span. After OSF publication, apply the snapshots to SQLite and remove the journal. Startup replays published journals idempotently, or discards journals for unpublished spans. A pre-manifest crash leaves an orphan file, which startup sweeps.

`sync_group` removes the server's `Pool` from the map before `fetch_overview`, so the progress callback can `flush_sync`, then puts the pool back. Flush counts `records.len()` before `flush_sync` clears them. A missing overview is an empty group, not an error. `Catalog::merge_posts` accumulates file size and `expected_segments` across sync batches. `upsert_posts` still replaces the file set for a post and must keep that contract.

## SQLite catalog

`rusqlite` 0.32, bundled SQLite, WAL (turned on through `query_row`), `foreign_keys` ON, `user_version` 3. Transactional migrations add STARTTLS configuration to version 1 and durable automatic-recovery metadata to version 2. The database file is `0600`. One connection, owned by the engine.

Tables:

- `server` — name, host, port, tls, user, password, priority, max_connections, pipeline_depth, fill, enabled, header strategy once negotiated.
- `group_sub` — server, name, low, high, watermark, subscribed, last_refresh.
- `post` — group, grouping key, display title, poster, posted_at, total bytes, file count, expected segments, flags.
- `file` — post, filename, size, expected parts, kind (`data`, `par2`, `nfo`, `image`, `archive`). `FileKind` is not re-exported from the engine. Callers use `file.kind.as_str()`.
- `job` — post or NZB name, dest directory, state, bytes done, error, optional password.
- `segment` — job file, part number, message-id, byte offset and size from yEnc (`begin` / `end`; a queued post starts at `0, 0`), article number if we have one, state, crc ok, server that supplied it.
- `thumb` — file id or post id, relative path, pixel size.

Flags: bit0 incomplete, bit1 has_image, bit2 has_par2.

Job states: `queued`, `fetching`, `recovery_headers`, `recovery_fetching`, `decoding`, `repair`, `extract`, `paused`, `failed`, `done`, `cancelled`.

Segment states: `pending`, `fetching`, `written`, `missing`, `failed`.

`list_groups` returns existing group rows for a server. Unsubscribe removes the group database row and cascades its post/file/thumbnail metadata, removes its derived header/body/thumbnail cache files, and invalidates pending previews and decodes. Download jobs detach from the removed post via their foreign key; explicit saved/downloaded files and job history remain. A later subscription starts with an empty cache and watermark.

Segment rows are created when a download is queued, not for every overview line. Posts and files are created for every grouped article in a subscribed group, so the grid can scroll the whole group without opening `.osf` files.

Writes during sync are batched transactions. The UI pages with a keyset on `(posted_at, id)`, never `OFFSET`.

Post count from a `LIST ACTIVE` line: if `low == 0` and `high == 0` then 0; else if `high >= low` then `high - low + 1`; else 0. Empty groups often have `high < low`. Status character: `y` open for posting, `n` no posting, `m` moderated.

## Grouping

Pure function from one overview record to a group key. Fixture-tested on:

- `Name [01/42] - "file.part01.rar" yEnc (1/50)`
- `"file.r00" yEnc (01/99)`
- subjects with no quotes, only a `(n/m)` counter
- a single-part image with no counter

Rules:

- Quoted filename, else the last filename-like token.
- `[n/m]` is the file index (last match). `(n/m)` is the segment (last match). Strip `yEnc (n/m)`, `[n/m]`, and `(n/m)` from the title.
- Group key is the lowercased, collapsed title, a newline, and the lowercased poster.
- Filename is the file key inside the post. Part number is the segment.
- `FileKind`: `Par2`, `Nfo`, `Image`, `Video`, `Archive` (rar, zip, 7z, or `.r` plus digits), otherwise `Data`. Video extensions are case-insensitive; legacy data-kind video files are normalized on read.
- The raw subject stays in the `.osf` record so a better parser can rebuild SQLite without re-downloading.

Re-parse is a catalog rebuild from OSF1, not a new server fetch. Queueing a post scans OSF1, matches `parse_subject` group keys, and builds `NewSegment { begin: 0, end: 0 }`.

## Download path

Same job model for a selected article and an imported NZB.

1. Expand the post's files, or the NZB's `<file>` / `<segment>` elements, into `segment` rows. NZB bytes and message-ids come from the file. Group is a hint for servers that want one. Destination is `data/downloads/` plus a sanitized title. NZB import takes a path string. No native file dialog (`rfd` / gtk).
2. The scheduler assigns FIFO BODY windows to pooled connections concurrently, respecting each server connection/pipeline limit and a global 64-window bound. Window depth adapts to pending segments and an 8 MiB size estimate. This estimate is not a hard memory limit for manual downloads.
3. `BODY <message-id>`. The body is scanned for `=ybegin` / `=ypart` / `=yend`. `yenc-simd` decodes the payload. We check size and the part CRC when `=yend` has one.
4. `=ypart begin=` / `end=` is a byte range. The decoder writes that range into the destination file (preallocated to the yEnc size). Parts may finish out of order. Offset-free uuencode fragments are staged separately by segment ID and assembled in article-part order once all expected fragments arrive. Streamed UU decoding uses an 8 GiB output cap and a 1 MiB line cap, validates the filename against the subject, and atomically publishes the final file only after a valid complete trailer. Staging remains through retry/repair until the job is durably Done, then owner-checked cleanup is best effort; progress recomputation is idempotent. UU has no embedded checksum: decoded CRC32 is computed, not verified against one.
5. Missing parts stay holes. Completion checks catalog part totals, unique contiguous part numbers, persisted written byte ranges and actual file length. Failed/missing yEnc bodies and files with missing cached headers enter automatic recovery, described below. A usable set must explicitly protect every unresolved filename, have valid source metadata and safe regular-file targets, and pass full verification after repair. An unrelated clean set cannot certify an incomplete download. Incomplete sequential UU streams still fail before PAR2, since they cannot safely be assembled with unknown byte offsets. Unavailable auxiliary PAR2 files can be ignored only when a usable local set verifies every payload file; parity-only downloads still require the requested repair files. Multiple independent recovery sets are not combined to cover one job.
6. After repair, extract. `zip` and `sevenz-rust2` in-process. RAR only when `unrar` or `unar` is on `PATH`. If it is not, the job completes and the UI says the archive is still packed. One password, from NZB meta or the job row. No guessing.

Finished images and videos emit cached-media paths and thumbnail events. Loading a cached article page after restart restores both paths without network downloads.

Pause and cancel are polled while network windows run. Already submitted requests may finish; generation checks discard their results after pause/cancel and prevent stale writes after resume. Resume queues failed/missing parts and preserves written parts. Restart reloads queued work from SQLite.

### Automatic PAR2 recovery

The download strategy follows [SABnzbd's on-demand repair model](https://sabnzbd.org/wiki/faq): keep the small index, defer large volumes, verify, then fetch recovery blocks as needed. Explicit PAR2-only downloads still fetch the files the user requested.

Source identity, completed header-search ranges, recovery-set identity and deferred/requested/exhausted candidates belong to the job in SQLite. Queue creation stores this information atomically, including bulk downloads and NZB imports. Activating a complete candidate atomically adds its segments without replacing written parts. Restart preserves candidates and completed search ranges. Removing a job clears its recovery metadata; deleting its source group/server disables further discovery without retargeting reused IDs.

Recovery searches cached overview records first. Missing headers can be looked up in the original newsgroup around the job's known article numbers, expanding nearby ranges up to one million article-number positions in 5,000-position requests. Successful empty searches count toward the durable budget; interrupted requests remain retryable. Matching uses the original posting identity, poster and exact file/PAR2 stem, with a seven-day posting-time bound when the original date is known. Partial recovery-volume headers are retained but not activated until complete. Found records are job recovery data, not partial publications into the shared OSF header cache.

Newly found payload segments use the normal downloader. If files remain incomplete, inspection determines needed and available recovery blocks; the planner activates matching volumes incrementally and verifies their actual recovery-set identity. Filename block counts are hints, not integrity evidence. Recovery downloads share the normal server connection/pipeline limits and fill-server fallback. The planner skips ranges already fully covered by structurally checked matching recovery packets. A newer matching volume can replace overlapping local volumes; replaced files remain inside the job folder under `.recovery-retired/`, outside the repair library's scan, to avoid duplicate recovery exponents. Forced retries keep alternatives eligible after a repair failure. Packet structure and filename block counts do not prove recovery content is usable; final payload verification remains authoritative. A finite exhausted search/candidate set fails clearly instead of looping or completing damaged files.

The queue exposes `recovery_headers` and `recovery_fetching` before `repair`. Pause/cancel interrupt network waits and preserve committed candidate/search progress. Generation checks prevent stale completion after a control change; an already-running blocking repair must settle before another repair or folder removal can write the same files.

## UI

Native egui. Dark theme set in code. Bundled OFL fonts: Source Sans 3 for UI text, JetBrains Mono for ids. Widget corner radius 10. Default item spacing `(12, 10)`, button padding `(14, 8)`. Dense browser rows locally use button padding `(10, 5)` and item spacing `(8, 4)`. The Options dialog stays airy and vertical.

Tokens:

| Token | Hex | Use |
| --- | --- | --- |
| bg | `#12141a` | window background |
| surface | `#1c1f27` | panels |
| raised | `#262a34` | cards, fields |
| hairline | `#343846` | strokes |
| text | `#e8eaef` | primary text |
| dim | `#9aa3b2` | secondary text |
| accent | `#3dbea5` | primary button, active nav, progress |
| warn | `#e6b35a` | warnings |
| danger | `#e15d5d` | destructive actions |
| ink | `#12141a` | text on accent |
| select | `#17332e` | selected row |
| hover | `#222630` | hover row |

Viewport default 1280×820, minimum 1040×680, title `NNTPBrowser`.

The look to hit is a dense modern reader: a Newzbin-style article table, a Newshosting / Easynews thumbnail gallery, checkboxes on rows and tiles, a lightbox. Flat, dark, generous type. Not a Windows XP tree, and not stock egui.

### Main window

The main window browses groups. It does not host the add-server form.

App bar: Articles and Gallery tabs, the active newsgroup name, a separate Downloads action, and Settings. Newsgroup selection stays in the left sidebar; the main pane does not repeat the group name or view tabs. Its controls include Filter, search, Download selected and Download range for newest-N/all cached search matches. One Get article headers button uses the selected Latest, New only or All available scope. Latest uses a header-count dropdown with 10,000, 25,000, 50,000, 100,000, 250,000, 500,000, 1 million, 2.5 million, 5 million and 10 million presets; the default is 10,000.

When selecting a different group, the worker checks its stored header spans. With no cached headers, it fetches the latest 10,000 automatically, independently of the manual count preset. Cached groups need no network request. Search/date filtering and pagination never trigger this check. Automatic fetches use the normal header progress/error flow and reload the active search/date query on completion.

Sidebar: subscribed groups only, as a tree with one level. Each server is a `CollapsingHeader`. Its children are the `group_sub` rows with `subscribed == true`, shown as the full group name, flat and left-aligned with consistent padding inside a full-width selectable row. Do not nest on every dot. No dotted hierarchy is used. A row click loads posts (`LoadPosts`). The context menu is Get article headers, Unsubscribe, and Copy name. Subscribe to groups on each server opens the group-list dialog. No wildmat field or redundant add-server button appears in the tree. Server creation lives in Settings. A drawn panel-toggle control collapses the sidebar to a 60 px rail with the restore button and subscribed count. The server header shows how many groups are subscribed; an empty sidebar directs users to Settings.

Status bar: the latest scrubbed notice, a combined received NNTP speed across every connection, open sockets versus enabled-server limits, busy connections, this process's resident RAM and CPU usage, and Errors (N). Process statistics are sampled off the UI thread once a second; CPU counts one fully busy core as 100% and may exceed 100% across multiple cores. Live download progress or header-fetch context appears when relevant. Repeated idle, active and failed counters are omitted. NNTP speed is measured before decompression and excludes TLS framing; a five-second trailing average smooths the 250 ms updates, using the full window even during startup. Download row rates use the same bounded estimator and expire when idle without another job update. Hovering Speed shows humanized session received/sent network totals and received data consumed after NNTP decompression. Counters include setup/authentication and partial traffic, retain totals after pool replacement/closure, and exclude TLS overhead and attachment decoding. Hovering Connections shows a scrollable list of live sockets grouped by server identity, including setup/authentication, pending network reads/writes, recent sending/receiving, five-second receive/send rates and individual network byte totals. Connection IDs remain stable within a pool and are never reused; cancellation, failure and closure remove the corresponding rows without losing session totals. Sampling retains counters and state only, not socket or protocol contents. Sockets open on demand. Open counts idle reusable sockets too; busy is the peak activity within the most recent sampling interval (normally 250 ms), preserving short transfers which finish between samples. Errors open a bounded, timestamped history dialog with selectable text, copy and clear actions.

The 48 px status bar uses fixed-width CPU, RAM, Speed, Connections, Busy and Errors segments in one row with inline labels, right-aligned monospaced values and subtle separators. Changing values or units leaves neighboring statistics in place. Header progress lives on the left with received/cached counts, an inline checked-range progress bar and a 32 px cancel icon; the main pane retains only fetch controls. Cancellation interrupts connect/GROUP/OVER/XZVER waits, drops unread sockets, keeps committed header batches and discards the uncommitted tail. Fetching again requests missing ranges. Progress and cancellation remain available when switching groups; cancellation does not enter error history.

The Filter menu offers All articles, Images and videos, Images, Several files, Repair files (PAR2), and Missing file parts. Cached count and pagination labels distinguish local rows from provider headers.

**Articles.** Top pane is a virtualized table of articles (`ScrollArea::show_rows`). Columns: date, title, size, files, parts, poster, status. A checkbox selects. Ctrl toggles, shift extends from an anchor, double-click queues the download. A drag splitter (default about 56% to the table) separates the table from the selected article. Selecting an article fetches its body into memory. Plain text fills the details pane; binary articles show a media preview and file list. Eligible media previews use complete in-memory file bytes, with a separate explicit Save action. Unknown part counts are labeled explicitly.

**Gallery.** A scrollable thumbnail grid of the same group. A Gallery-only Tile size slider chooses a preferred width from 140–360 px, defaulting to 220 px, and persists it in preferences. Column count, image height and virtual row pitch adapt to the pane and selected size. Resizing retains completed RAM previews and recomputes viewport requests. Each tile has a checkbox, size, and a UTC stamp `YYYY-MM-DD HH:MM` (unix time `<= 0` renders as an em dash). Double-click opens a lightbox with previous and next. Visible media tiles fetch eligible files smaller than the configured limit (default 10 MiB) into RAM, including multipart images and videos with complete file headers. Previewing never creates download jobs, saved files, thumbnails on disk, or completed-file entries. Unknown-size or larger files require an explicit normal download. Save writes the selected previewed media file; the regular article download actions retain their whole-article behavior. All dates, Today, Last 7 days, Last 30 days and a custom UTC date range filter the whole cached group in SQL before pagination. Custom end dates include the final second of the day; invalid dates leave the previous valid filter intact.

**Downloads.** Reachable from the toolbar. Transfers lists queued, paused, failed and cancelled jobs with progress, speed, pause/resume/cancel and retry. Completed combines saved files and finished downloads, with their folder path, file count, expandable thumbnails and an image viewer. NZB import opens a compact path field. Each row uses drawn icon buttons with named tooltips for transfer actions, opening or copying the folder path, removing a terminal job from the list while keeping its files, and deleting its dedicated folder. Remove and delete remain disabled until the transfer is terminal; deletion opens a confirmation showing the name, path and indexed file count. The confirmation rejects changed job identities; the engine rejects active jobs and unsafe or shared output folders.

**Completed files.** The Downloads page includes finished files as a thumbnail grid. Click an image for a large overlay. Video and unknown types open with the system opener (`xdg-open`, `open`, or `cmd start`). Tiles that are still packed archives say so. Video playback uses the system player. Optional installed ffmpeg creates bounded video poster thumbnails with a timeout; absent or failed helpers retain a video icon.

**Previews while scrolling.** Preview requests use a separate bounded network path with generation-scoped readiness, progress and failure events. They do not enter the regular download queue. The UI retains original encoded bytes in a 64 MiB LRU and decodes bounded RGBA away from its thread. Texture retention is capped at approximately 200, with only two full-image textures; full-image views use a maximum 2048 px edge. Video posters use an optional memory pipe to ffmpeg; no temporary media file is written, so seek-dependent formats can fall back to a video icon. Failed requests are suppressed until file metadata changes, preventing scroll retry loops. Gallery requests only tiles intersecting the current viewport. Scrolling cancels unfinished offscreen requests and frees their connection slots; completed previews remain in the RAM cache for reuse. The open viewer takes priority over the underlying grid. Requests enter as lightweight intents, so discarded offscreen work does not scan stored headers. During header synchronization, preview controls and dispatch run between 20,000-article-number windows; an in-progress network response must finish, time out or be cancelled before new previews dispatch. Header waits poll controls every 25 ms. Headers and previews retain the same server pool and connection budget. Changing context advances the preview generation and rejects late results. Explicit Save writes the selected file and reports its saved path or failure.

Preparing ready previews batches only their requested filenames/group keys into one streamed header scan per group, avoiding a whole-group scan for every visible tile. After preparation, controls are checked again before dispatch so obsolete work cannot start a network request. Multipart parts spread across connections before increasing FIFO pipeline depth. The first original part identifies encoding; out-of-order UU results retain their part order. Each preview shares one cumulative encoded-body budget across concurrent connections and fill servers. Failed or cancelled partial responses refund their reservations; successful responses remain charged until that preview operation ends. The completed-preview cache budget does not include transient network buffers, decoded images or textures.

Startup sends `ShowGroups` for each new server id. `GroupRows` includes the server id, so empty replies remain attributable. Article responses include group, search and pagination cursor; stale replies cannot replace the active view. Header fetching reports received/stored counts, range and completion separately from article-page loading.

### Settings

Settings is a centered, resizable dialog, reviewed at both 1040×680 and 1280×820. Global sections are Appearance, Network, Storage and Servers, with preferences persisted to `preferences.json`. Appearance controls interface scale and default view, sidebar and automatic previews. Network controls preview file limit (1–10 MiB), encoded-byte cache budget (8–64 MiB) and NNTP connection timeout (2–120 seconds). Preview connections use each server's configured limit; old `preview_concurrency` preferences are ignored. Storage accepts absolute download and data directories and shows the current paths. New saves/downloads use the download directory; the data directory changes after restart, preserving existing data in its original folder.

In Servers, a server card list and Add server action occupy the left rail. The right pane has Connection, Account and Downloads tabs with aligned, bounded fields and a fixed Save / Test / Delete footer. Narrow windows use a compact server picker. Host, port, security (implicit TLS, STARTTLS or plain) and enabled belong to Connection; user/password to Account; priority, connection limit, pipeline depth and fill-server behavior to Downloads. Password uses `TextEdit::password(true)`; header compression is a negotiated read-only value. Saving enabled state is atomic, deleting asks for confirmation, and connection testing streams the group list through the shared pool, reports success or failure, and leaves the group picker unchanged.

### Subscribe dialog

Subscribing opens a dialog with a flat list of that server's groups. It is not a tree.

- The dialog starts with a large viewport, reserves it while loading, and remains resizable.
- Server picker: `ComboBox::from_id_salt`. Opening the dialog sends `Command::ListActive { server_id, wildmat: "*" }`.
- While the list is downloading, an animated `ProgressBar` (`animate(true)`). `LIST ACTIVE` has no total until the lone dot, so the bar is indeterminate until `done`, then `1.0`. Next to it: groups received, sum of article counts, elapsed seconds, rate, and `host:port` with TLS or plain. A notice that starts with `group list:` clears the loading flag.
- When finished: a summary line (N groups, M articles, K already subscribed) and a virtualized list. Columns: checkbox (subscribe or unsubscribe), name (truncated), `posts()`, posting status (`y` open, `n` no posting, `m` moderated). Article-number low/high watermarks remain protocol data and are not displayed as a column.
- A local filter box. Build a `Vec` of indexes once per filter or data change. Every column header toggles ascending/descending sorting of those indexes; stable server/group identities preserve checkbox actions. Names use small capped dot-depth indentation with a muted namespace and brighter final component, retaining full-name sorting and hover text. The chosen style combines comparison options 2 and 3; indentation never reorders rows. The Compare layouts action remains a separate proof-of-concept dialog with eight selectable designs, taking a bounded, read-only snapshot of the loaded server groups matching the current filter. Show the source and preview count; use sample groups when no loaded list is available and in offline fixtures. Name/Articles sort controls illustrate global sorting versus sorting within folders/sections. Preview choices do not write settings or subscriptions. Do not clone or sort the group list every frame, and do not build a widget per group. `show_rows`.
- Sort and dedupe once, when the download finishes. Case-insensitive name, then exact name. Drop empty names and names that contain whitespace. Do not sort on every batch.
- An optional exact-name line at the bottom, for a group the filter is aimed at.
- Checkbox on sends `Subscribe`. Checkbox off sends `Unsubscribe`.

`LIST ACTIVE` streams to drive progress during the download.

- `Connection::list_active_each` writes the command, reads the status, then reads lines, unstuffs, and parses with the existing `parse_group_info`. One junk line is skipped. A hard failure still fails the call.
- The pool retries only when zero groups were emitted and the error is retryable. A partial callback is not retried, or the dialog would show duplicates.
- The engine batches about 400 to 500 groups into `Event::GroupSample`. The event includes `received: u64` and `done: bool`. An empty batch with `done: false` is sent at the start so the dialog clears and shows the bar immediately. The UI appends batches and adds up `posts()`. On `done`, it sorts and dedupes.
- `test_server` keeps not sending `GroupSample`.

## Headless CLI

No arguments launches the GUI. Any subcommand stays in the terminal, uses `Handle` and the default data directory, and never opens a window. The parser lives in `crates/app`, is `pub use`d from the library, and `main` dispatches before `eframe`. Unit-test the parser only. Do not require a live server for those tests.

Wait for the startup `Servers` event, send the command, then poll until the completion event or a timeout. `groups` and `sync` use a long timeout (120–180 s). Print a progress line as each `GroupSample` batch arrives. Exit 1 on timeout or on a failure notice. Never print a password. `server-add` learns the new id the same way the dialog does: the maximum id on the following `Servers` event. `NNTP_PASSWORD` supplies the password when `--password` is omitted.

Commands, one per engine `Command`:

- `help`
- `servers`
- `server-add --name --host [--port 563] [--plain] [--user] [--password] [--priority 1] [--connections 8] [--pipeline 8] [--fill] [--disabled]`
- `server-test ID`
- `server-delete ID`
- `groups SERVER_ID [--wildmat *]`
- `subscribe SERVER_ID GROUP`
- `unsubscribe SERVER_ID GROUP`
- `sync GROUP_ID [--limit COUNT | --all]`
- `posts GROUP_ID [--search TEXT]`
- `open POST_ID`
- `queue POST_ID`
- `jobs`
- `pause JOB_ID`
- `resume JOB_ID`
- `cancel JOB_ID`
- `nzb PATH`
- `preview POST_ID`

Engine commands those map onto: `Refresh`, `SaveServer`, `UpdateServer { id, server, enabled }`, `DeleteServer`, `TestServer`, `Subscribe`, `Unsubscribe`, `ListActive { server_id, wildmat }`, `SelectGroup`, `ShowGroups`, `SyncGroup`, `LoadPosts { group_id, after, search }`, `OpenPost`, `QueuePost`, `ImportNzb { xml, dest }`, `Pause`, `Resume`, `Cancel`, `Preview`.

Events include `Servers`, `GroupRows { server_id, groups }`, `PostsPage { group_id, after, search, posts }`, `PostsFailed`, `HeaderFetchFinished`, `Files { post_id, files }`, `Jobs`, `GroupSample`, `Notice`, `Thumb`, `MediaCached`, `MediaCacheFailed`, `ArticleContent` and `ArticleReadFailed`. Group/search/cursor-scoped responses prevent stale views. Legacy `Groups`/`Posts` variants remain for compatibility.

## Known limits

These are accepted. Do not expand them unless asked.

- At most 64 active download windows globally; per-server connection limits also apply. Pipeline depth is a ceiling, reduced by pending work and estimated size.
- Automatic media previews require known file sizes below 10 MiB and complete headers. They use bounded requests and a 64 MiB encoded-byte memory cache, without creating jobs or files. Save and normal article download actions are explicit disk-writing operations.
- RAR needs `unrar` or `unar` on `PATH`.
- No PAR2 creation. Verification and recovery use fixed binary fixtures, rather than a create/repair round trip.
- Video playback uses the system player; video poster thumbnails require an optional installed ffmpeg.
- No OS keyring.
- No BinHex, xxencode or arbitrary MIME attachment decoding.
- No historical header full-text index.
- No recombination of a article posted across groups under unrelated subjects.
- No `XZHDR` until a bulk single-field fetch is actually needed.
- No posting.

## Verification

`cargo test` is the check. The app is native egui, so a browser pass does not apply. Required checks: `cargo fmt` on the crates that changed, `cargo test -p app --lib`, `cargo test -p engine --lib` if the `GroupSample` shape changes, and `cargo build -p app --bin nntpbrowser`. The engine integration test must expect `ListedGroup` batches, not the old "in the sample" notice and not `GroupSample { names }`. A bad-password server stays enabled and is tried first, because auth does not fail over. Tests that queue a post disable that server with `UpdateServer` first, and they clone the row before mutating it.

The CLI is how a real server is exercised without the window: add, test, list groups, subscribe, sync, list posts, queue, and watch jobs.

The main browse view has one **Get article headers** action. Its scope selector offers Latest N, New only, or All available; empty views refer to that control rather than adding another sync button. Header progress measures checked article-number slots in missing ranges, excludes cached spans and accounts for sparse responses. Received/cached counts remain separate. Downloads show transfer percentages and stage-specific processing progress, with aggregate progress visible while browsing.

## Distribution

See [release automation and native packages](docs/releases.md) for CI, automatic
versioning, GitHub Pages and platform-specific distribution formats.
