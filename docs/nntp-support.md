# NNTP newsreader support

Checked 2026-10-10. NNTPBrowser exposes the core RFC 3977 newsreader command set, modern TLS authentication, standard compression and tested provider compatibility. Posting, injection and feeds are outside its newsreader scope. This is a source and transcript review, not an exhaustive RFC conformance certification.

## Standards baseline

The core protocol is [RFC 3977](https://www.rfc-editor.org/info/rfc3977/), with additional LIST facilities in [RFC 6048](https://www.ietf.org/rfc/rfc6048.html). Authentication uses [RFC 4643](https://www.rfc-editor.org/rfc/rfc4643.html); the implemented SASL mechanisms are [PLAIN / RFC 4616](https://www.rfc-editor.org/rfc/rfc4616.html) and [SCRAM-SHA-256 / RFC 7677](https://www.rfc-editor.org/rfc/rfc7677.html), using the exchange defined by [RFC 5802](https://www.rfc-editor.org/rfc/rfc5802.html). The [IANA NNTP registry](https://www.iana.org/assignments/nntp-parameters/) lists registered capabilities and compression algorithms.

TLS uses [RFC 4642 STARTTLS](https://www.rfc-editor.org/info/rfc4642/) or implicit TLS, preferred by its [RFC 8143 update](https://www.rfc-editor.org/rfc/rfc8143.html). rustls supports TLS 1.2 and 1.3 with safe default versions, consistent with the minimum versions in [BCP 195 / RFC 9325](https://www.rfc-editor.org/rfc/rfc9325.html). Connection compression uses [RFC 8054 DEFLATE](https://www.rfc-editor.org/rfc/rfc8054.html).

## Implementation coverage

The public protocol APIs live in [connection](../crates/nntp/src/conn.rs), [reader operations](../crates/nntp/src/conn/reader.rs), [authentication](../crates/nntp/src/conn/auth.rs), [TLS upgrade](../crates/nntp/src/conn/tls.rs), [overview negotiation](../crates/nntp/src/conn/overview.rs) and [pool](../crates/nntp/src/pool.rs). Protocol APIs are broader than the commands the binary-focused GUI needs today.

| Area | Implemented behavior | Limits |
| --- | --- | --- |
| Session | Greeting, CAPABILITIES and extension keyword, MODE READER, DATE, HELP, QUIT with response validation; fresh capabilities after TLS, authentication and successful mode changes | MODE READER is rejected after TLS. |
| Groups | GROUP, streaming LIST ACTIVE and sparse LISTGROUP, NEWGROUPS, NEWNEWS with validated dates and optional GMT | ACTIVE range counts are estimates. Servers may deny or omit optional queries. |
| LIST | Bare LIST; typed ACTIVE.TIMES, NEWSGROUPS, DISTRIB.PATS, HEADERS and OVERVIEW.FMT; generic keyword/arguments interface preserves additional facilities | Additional LIST extensions return raw lines rather than each having a separate typed wrapper. |
| Overview and fields | OVER current article, number, open/closed range and message-id; lossless OVER/HDR byte APIs retain legacy octets and extra/empty columns; HDR with XHDR fallback | Structured OverviewLine retains standard fields only. Display String APIs replace invalid UTF-8 octets; callers can request lossless bytes. |
| Articles and navigation | ARTICLE with identity and bounded bytes; HEAD/BODY/STAT by number, message-id and current article; NEXT/LAST identities | Oversized bounded ARTICLE/BODY requires discarding the connection. Unbounded calls remain available for explicit downloads. |
| Authentication | Advertised SCRAM-SHA-256 preferred, then advertised PLAIN, then USER/PASS; verified server proof; initial-response and challenge framing; cancellation; fresh capabilities | No SCRAM-SHA-256-PLUS/channel binding, OAuth, GSSAPI, legacy DIGEST-MD5 or SASL security layer. An authentication rejection or bad proof does not fall back to a weaker mechanism. |
| TLS | Implicit TLS and required STARTTLS; hostname/certificate validation and public roots; explicit client-config API supports custom roots and client certificates | Custom TLS policies are a protocol API, not a GUI trust-store editor. Explicit plain mode allows plaintext credentials. |
| Standard compression | Bidirectional COMPRESS DEFLATE, raw stream and sync flush; current capabilities checked before enabling; 403/502/503 decline falls through on the existing session | DEFLATE is the implemented standard algorithm. Compression starts after authentication. |
| Provider compatibility | XOVER, XZVER yEnc/compressed and binary-zlib variants, XFEATURE COMPRESS GZIP variants | These are compatibility extensions, not additional standardized compression capabilities. |
| Pipelining | FIFO BODY windows with per-server connection/depth limits, bounded-response variants and retained earlier successes if later responses/reconnect fail | GROUP, capabilities, authentication, STARTTLS and COMPRESS are never pipelined. |
| Posting and feeds | Outside newsreader scope | POST/IHAVE and [RFC 4644 streaming feeds](https://www.rfc-editor.org/rfc/rfc4644.html) are not implemented. |

## Framing and security checks

Core command lines are checked before compression or network writes, including the 512-octet CRLF-inclusive line limit and 497-octet argument limit. Whole BODY batches are validated before sending any item. AUTHINFO SASL and continuation lines use a separate bounded 32 KiB limit because RFC 4643 permits these exchanges to exceed the core line limit. Credentials are bounded at 4096 octets per value. SCRAM uses ring cryptography, checks nonce extension and server proof, applies SASLprep, rejects unsupported mandatory extensions and bounds server-selected PBKDF2 work to 1,000,000 rounds.

Required STARTTLS fails closed if absent, refused or its TLS handshake fails. No credentials precede the upgrade. Buffered unexpected plaintext after the 382 response is rejected. TLS resets the previous capability, overview-format and compression-strategy snapshot, with no second greeting. Certificate validation uses the configured hostname, including custom-port configurations.

Status parsing checks ASCII code/separator boundaries before slicing UTF-8, avoiding malformed-response panics. Bounded BODY preserves CRLF and empty lines exactly like pipelined BODY. Dot stuffing is removed once. State errors distinguish missing group, absent current article, absent previous/next article and permission denial.

## Evidence and remaining limits

Local TCP/TLS fixtures cover reader commands and selection errors; sparse article ranges; extension LIST queries; HDR/XHDR, overview optional fields and invalid-UTF-8 byte preservation; dot stuffing; FIFO pipelines, partial successes and retry; compression variants; rejected commands sending no bytes; STARTTLS credential ordering and capability reset; TLS certificate/name rejection; long PLAIN framing and cancellation; both SCRAM server-proof endings and tampered-proof rejection. SCRAM calculation also matches the RFC 7677 published vector. Fixtures do not use user credentials or live providers.

The `over_bytes` and `hdr_bytes` APIs preserve original overview/header octets. Their display-oriented string alternatives use replacement for invalid UTF-8. The core argument validators cover tokens, article identities/ranges, dates and control characters, but there is no exhaustive command/error/state conformance matrix. All possible advertised SASL mechanisms are intentionally not implemented. Server extensions and provider-specific framing cannot be described as universally supported.

Product wording: **“NNTP newsreader with implicit TLS or STARTTLS, SCRAM-SHA-256/PLAIN/USER-PASS authentication, overview compression and tested provider compatibility.”** Avoid “fully RFC compliant” or “supports every extension.”
