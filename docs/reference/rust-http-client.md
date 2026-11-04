# Rust HTTP client: evaluation

Plan 21's dependency table names "`reqwest` or `ureq`" for the transport that
replaces `urllib`, the SOCKS5 client and the Electrum client. `kassiber-net`
instead carries a small HTTP/1.1 client (`crates/net/src/http.rs`, 913 lines
with tests) over its own guarded socket, `rustls` and `httparse`. This page
records the 2026-11-04 evaluation of `ureq` 3.5.1 and `hyper` 1.12.0 against
that client. `reqwest` was not evaluated separately: it is built on `hyper`
and `tokio`, adds connection pooling, and reads proxies from the environment
by default, so it inherits every finding below for `hyper` plus those for
`ureq`'s defaults.

**Decision: keep the current client.** Neither library meets every
requirement while putting less of our own code in the security-critical
path. Revisit if `ureq` stabilises its transport API (see below) or if the
backend moves to async I/O for other reasons.

## Method

The candidates were fetched once (the single approved online `cargo fetch`
of the dependency round) and resolved in a throwaway workspace member with
default features off: `ureq` with `rustls-no-provider` and `socks-proxy`,
`hyper` with `client` and `http1`. Sources were read from the local registry.
Nothing was added to the workspace.

## Requirements

| Requirement | Current `kassiber-net` | `ureq` 3.5.1 | `hyper` 1.12.0 (client, http1) |
| --- | --- | --- | --- |
| No proxy from the environment | Never reads it; `Proxy` comes only from saved settings | **Fails by default**: `Config::default()` sets `proxy: Proxy::try_from_env()`; must be overridden on every agent | Has no proxy support at all; Kassiber would keep its own |
| No redirects | `RedirectPolicy::Refuse` by default; AI uses exact-origin with urllib's limits | Default `max_redirects: 10`; settable to 0, but same-origin-only with urllib's POST rules would need our own loop anyway | None; our redirect loop stays |
| SOCKS5 with remote DNS over Kassiber's socket | `socks.rs` (127 lines) runs inside the guarded `Transport`; target names never reach the resolver | `socks-proxy` dials its own `TcpStream` (`unversioned/transport/tcp.rs`); remote DNS exists, but over a socket `kassiber-net` did not open | None; ours stays |
| TLS trust exactly as now (bundled roots, native roots, pinned, explicit insecure) | `tls.rs` builds the rustls config per route | Possible only with `rustls-no-provider` and a hand-built `ClientConfig` passed through its non-semver TLS config | None (bring-your-own IO); ours stays |
| Bounded responses and timeouts | 64 MiB hard cap, per-call cap, header/count limits, per-operation timeout plus whole-acquisition deadline | Body limits via `read_to_vec`/`with_config().limit`; per-phase timeouts. Workable | No body limit; we would wrap `Incoming` with our own counter. Timeouts need our IO wrapper |
| HTTP/1.1 chunked; keep-alive | Chunked (with trailer and chunk-count limits) and `Content-Length`; one request per connection, `Connection: close` by design, because consent and the ledger bind to a connection | Pools by default (`max_idle_connections: 10`, 15 s idle age); must be disabled. Chunked via `ureq-proto` | Chunked framing is what it would replace; per-connection use is natural |
| Cancellation | Consent revocation and the offline/no-egress switches are rechecked before every connect, DNS lookup, TLS record and write (`Route::check`) | Only through timeouts, unless a custom `Transport` re-implements our checks | Driven by our IO, so our checks survive, but the request future and the connection future need an executor |
| Egress ledger and consent check on every request | Every DNS, connect, proxy handshake and write records into `Ledger` after `Route::check` | Only with a custom `Connector`/`Resolver` (`Agent::with_parts`), both under `ureq::unversioned`, which the crate exempts from semver | Preserved, because hyper writes through our `Connection` |
| No socket outside `kassiber-net` | `Route::connect_inner` is the only `TcpStream::connect_timeout` call; the chokepoint scanner enforces it | **Fails**: the crate ships `DefaultConnector` (`TcpStream::connect*`) and public one-liners (`ureq::get`, `Agent::new_with_defaults`) that dial directly with the environment proxy; any crate holding the dependency can call them. It also trips `other_crates_cannot_open_connections_or_depend_on_network_clients` and the explicit `ureq` ban in `crates/net/tests/dependencies.rs` | Opens no sockets |

## What a switch would remove

- **`ureq`**: request encoding, response framing and parts of the redirect
  loop in `http.rs`. To meet the table we would still write a custom `Connector`
  (wrapping `Route::connect_inner`), a `Resolver` that refuses, a rustls
  config bridge, and agent settings that undo three unsafe defaults (proxy,
  redirects, pooling), against an API the crate reserves the right to break
  in minor releases. The socket-opening code we must never call would still
  remain one call away for any crate that depends on it. Net effect: about as much of our own code, plus a
  second HTTP engine and its defaults in the security path.
- **`hyper`**: request encoding and chunked/`Content-Length` framing in
  `http.rs`, which already parse heads with `httparse` (as `hyper` does). It adds
  `tokio` (`sync`), `futures`, `http`, `http-body`, `bytes`, `want` and
  `atomic-waker`, plus an async bridge: the blocking `Connection` must
  implement `hyper::rt::Read`/`Write`, and a local executor must drive the
  connection and request futures together. Body limits, deadlines, redirects,
  SOCKS, TLS, upgrades and the ledger all stay ours. Net effect: a small cut in
  framing code for a larger and less familiar surface in the same path.

The current framing code is exercised by `framing_and_limits_fail_closed_for_success_and_error_responses`,
the redirect tests and `crates/net/src/adapter_tests.rs`; the strict socket
guard and the no-egress regressions cover the whole client.

## When to revisit

- `ureq` moves `Transport`/`Connector`/`Resolver` out of `unversioned`, offers
  a build without its TCP connector, and stops reading proxies from the
  environment by default.
- The backend adopts async I/O elsewhere (for example a native Electrum
  multiplexer), at which point `hyper` over the guarded stream costs less.
