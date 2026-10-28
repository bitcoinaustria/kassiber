# Rust port: connection inventory and R0 gate

Audited against Python commit `8709d430`, before porting transports. This is
an implementation inventory, not additional network permission. The owning
contracts are [privacy and security](privacy-and-security.md#external-requests-complete-list)
and [plan 21, verification 6](../plan/21-rust-backend.md#verification).
The policy implementation in this checkout is `kassiber/egress_policy.py`;
there is no `kassiber/core/egress_policy.py`.

## Present Python boundaries

In the tables, **shared gate** means
`kassiber/proxy.py:19` → `kassiber/egress_policy.py:133`:
`KASSIBER_NO_EGRESS` (`1/true/yes/on`) and the persisted offline preference,
both destination-blind, including loopback. A missing preference is online;
an existing unreadable, symlinked or invalid preference is offline.
**Online gate** means `kassiber/egress_policy.py:156`: offline blocks, with an
exception only for an explicitly local AI provider at a loopback URL;
`KASSIBER_NO_EGRESS` does not cover that gate. Transport helpers do not themselves
authenticate a click or a per-book permission: their callers own that consent.

**Backend HTTP policy** means explicit per-backend proxy only, no OS/environment
proxy discovery or `NO_PROXY` bypass, redirects refused by default, HTTPS proxies
refused before connection, and `.onion` refused without a proxy. SOCKS5 uses
proxy-side target DNS, including the `socks5` spelling. HTTP proxy CONNECT
preserves target TLS and separates proxy authentication from target credentials.
Default TLS is Python/OpenSSL host trust, including an operator's `SSL_CERT_FILE`;
backends may supply a certificate or explicitly disable verification where
their adapter exposes that choice. No alternate Tor port is probed.

| Connection constructor / caller (file:line) | Current authority and kill switches | Ledger subsystem and route / trust |
| --- | --- | --- |
| `kassiber/proxy.py:280`, `:337`, `:348`, `:349`: urllib opener; `:510`: `ProxyUrlOpener.open` | Shared gate before opener creation; explicit sync/check/refresh or enabled source feature at callers | `sync`, or `pricing` when source label contains rate/price; `http.request`, method and estimated outbound application bytes. Backend HTTP policy. The optional `follow_redirects=True` escape exists; production callers retain the false default. |
| `kassiber/proxy.py:215`, `:221`: SOCKS socket; `:411`, `:420`, `:429`: `http.client` subclasses and TLS wrapper | Shared gate before proxy DNS/connect, also reached through shared HTTP | HTTP records logical target before SOCKS construction. Direct `connect_via_socks5` has no independent ledger entry. Only proxy host resolves locally; IPv4/IPv6 target literals use SOCKS address types and domain targets are sent verbatim as IDNA. TLS wraps the tunnel with target SNI and selected context. |
| `kassiber/core/sync_backends.py:125`, `:164`, `:211`: JSON/text GET and JSON POST; Core RPC, Bitcoin/Liquid explorer and server-assisted Silent Payments use these | Shared gate for every HTTP attempt/retry; explicit sync/check or matching automatic-source grant | `sync`; backend HTTP policy. `_backend_ssl_context` supplies backend certificate/insecure policy. Silent Payments local scanner files are local reads, not child execution. |
| `kassiber/core/sync_backends.py:282`, `:305`, `:338`, `:347`: Electrum TCP connection and TLS; `:407`, `:464`: writes | Shared gate before connect and every call/batch write; explicit sync/check or source-class feature | `sync`, `socket.connect` and `socket.write` (RPC method label and byte count). Direct or explicit SOCKS, never HTTP proxy for raw Electrum. `.onion` requires proxy. TLS defaults to verified host roots, supports backend certificate and explicit insecure mode. |
| `kassiber/core/chain_analysis_acquisition.py:163`, `:216`: bounded Electrum writes / HTTP; `kassiber/daemon.py:13997`: explicit HTTP connection probe | Acquisition requires reviewed plan and explicit apply/once-only AI consent or active recurring grant; rechecks shared gate per request. Probe requires explicit check or the enabled per-book connection feature | `sync`; existing backend route and trust, HTTP redirects refused. Acquisition plans, graph reads and status do no network work. Graph follow-ups reuse the selected backend under a successful-sync grant, never another server. |
| `kassiber/core/rates.py:180`, `:210`: mempool, Coinbase and CoinGecko HTTP | Shared gate; explicit rates sync, or narrowly enabled per-book live-rate feature for live desktop/background refresh; cache/display reads do not grant access | `pricing`; backend HTTP policy. Only mempool inherits its selected backend proxy; standalone providers have no per-provider proxy setting. |
| `kassiber/btcpay/client.py:246`, `:291`: Greenfield opener | Shared gate through proxy opener; explicit inspect/setup/key check/sync or enabled source feature | `sync`; backend HTTP policy, verified TLS, GET-only client. Browser key URL construction does not fetch. |
| `kassiber/core/exchange_imports.py:552`, `:559`: authenticated Kraken/Coinbase/Binance opener | Shared gate through production proxy opener; explicit configured exchange import/sync | `sync`; backend HTTP policy. Injectable fake openers are a testing seam, not another production connection constructor. |
| `kassiber/core/lightning/lnd.py:218`: LND REST | Shared gate; explicit sync/check/report tool consent or matching enabled refresh source | `sync`; explicit backend proxy, no redirects, macaroon stays at selected endpoint. Adapter context supplies default/custom CA or explicit insecure selection. Construction alone does not probe. |
| `kassiber/core/chain_observer/bdk.py:276`, `:285`, `:294`: native BDK Electrum | Direct shared product policy check before construction; explicit sync and observer routing preflight | `sync`, one `bdk.electrum.connect` row, zero estimated bytes; SDK owns later DNS/sockets/TLS. Capability matrix below. |
| `kassiber/core/chain_observer/lwk.py:373`, `:383`, `:393`: native LWK Electrum | Direct shared product policy check before construction; explicit sync and observer routing preflight | `sync`, one `lwk.electrum.connect` row, zero estimated bytes; SDK owns later DNS/sockets/TLS. Capability matrix below. |
| `kassiber/ai/client.py:1767`, `:1778`, `:1790`, `:1794`: HTTP AI opener (models, chat, document/OCR inference, fixed connection test) | Online gate; caller checks provider enabled state and remote/TEE acknowledgement, then explicit check/models/chat/import action. Only provider marked local at loopback can work offline | `ai`, `http.request`. Ambient proxies disabled; redirects allowed only within configured scheme/host/port (`:67`), unlike backend HTTP's no-redirect rule. Verified default Python TLS. No backend proxy inheritance. Outside product environment kill switch. |
| `kassiber/update_check.py:82`, `:299`, `:326`: GitHub update opener / fetch | Online gate plus owner-only app-wide consent (`:251`), explicit CLI update or permission-enabled desktop check; `KASSIBER_DISABLE_UPDATE_CHECK` override. Ordinary CLI banner only reads cache | **No Python ledger row**. Ambient proxies disabled, redirects refused, verified default TLS, fixed GitHub endpoints and bounded response. Outside `KASSIBER_NO_EGRESS`. Desktop launches a separate check process, so its parent ledger cannot see it either. |

### DNS and children

There are no direct production `getaddrinfo`/`gethostbyname`/`getnameinfo`
call sites in `kassiber/`. DNS happens implicitly in direct urllib/http.client,
`socket.create_connection`, and native SDK/child code. SOCKS resolves only the
proxy locally and sends target names to that proxy. Direct `.onion` refusal is
therefore required before entering any resolver. TLS wrappers above operate on
existing sockets; they still send handshake bytes and must retain the grant.

| Child or local IPC boundary (file:line) | Authority, routing and present observation |
| --- | --- |
| `kassiber/core/lightning/cln.py:297`, `:303`: `lightning-cli` | Explicit observation/check/report or enabled source feature; online gate refuses offline before spawn. **Outside `KASSIBER_NO_EGRESS`, Python socket patch and ledger.** Child selects its configured RPC transport (normally local CLN IPC); parent does not enforce proxy/TLS or fabricate a destination row. |
| `kassiber/ai/broker_client.py:177`, `:189`: Node provider broker; online checks at `:335`, `:492` | Explicit provider check/models/chat, enabled and acknowledged provider at callers; online gate before spawn/use, including model/status calls. **Outside `KASSIBER_NO_EGRESS`, Python socket patch and ledger.** Node and provider CLIs own DNS/proxy/TLS; per-backend proxies do not apply. Claude Fast can additionally contact `api.anthropic.com` despite its inference base URL. |
| Provider-broker transitive children: `ui-tauri/provider-broker/src/executables.ts:75`, `codex.ts:101`, `claude.ts:334`, `opencode.ts:64`, `:138`, `acp.ts:323` | Executable/version/auth/model probes and provider runtimes can themselves connect, including authenticated status refresh. Scope is the selected runtime(s), not every configured provider. Same child/ledger/guard gap as Node. This TypeScript broker stays after Python retirement. |
| `kassiber/cli/chat.py:98`, `kassiber/operator/client.py:131`, `kassiber/operator/runner.py:90`: Kassiber daemon/broker/CLI children | Explicit command or broker operation; children retain their own transport and action gates. Python test hooks reach Python children through inherited environment/PYTHONPATH; packaged or Rust children need their own guard. Runner propagates a caller's product kill switch. No parent ledger row for child activity. |
| `kassiber/operator/protocol.py:179`, `:194`, `:293`, `:296`, `:400`, `:403`; Windows pipe at `:282` | Bind/accept/connect/stale-endpoint probe on local Unix sockets, or SID-restricted Windows named pipe with remote clients rejected. Local authenticated IPC, no IP resolver/transport, no network ledger; network switches and tuple-address socket guard do not govern these. |
| `kassiber/release_verification.py:742`: `ssh-keygen -Y verify`; `kassiber/operator/peers.py:78`: `/bin/ps` | Local signature verification / peer process inspection; no network destination, proxy, TLS or ledger row. `ssh-keygen` here does not use SSH transport or contact a host. |
| `kassiber/operator/native_auth.py:339`, `:373`, `:443`: codesign inspection/verification and signed native-auth helper | Local code identity, Keychain/Touch ID and pipe-gated helper operations, not a provider probe; no parent network ledger/gate. These are native children, so the Python socket patch is not proof of their behavior. |
| `kassiber/core/document_import.py:475`, `:599`: `pdfinfo`, `pdftoppm`; `kassiber/backup/age_cli.py:94`: optional age/rage binary | Explicit document/backup operation; intended local processing, no parent transport gate or ledger. OCR inference itself uses the AI path above. Native executables (and any age plugin) are not constrained by Python's socket patch; namespace isolation must cover them too. |

External links launch in the system browser via the desktop, not a Python
HTTP client; they are never prefetched. SQLite, SQLCipher, keyring/D-Bus and
local files are storage/OS services, not additional configured IP transports.

## Native SDK capability and routing matrix

`bdk_compatibility_reason` (`kassiber/core/chain_observer/bdk.py:116`) and
`lwk_compatibility_reason` (`kassiber/core/chain_observer/lwk.py:33`) select
the named compatibility route **before** native construction. A failure of an
already selected native route does not silently retry with another transport.

| Configuration | BDK 3.0.0 | LWK 0.18.0 |
| --- | --- | --- |
| Supported descriptor, matching chain, direct verified Electrum | Native Bitcoin, default host/domain validation; configurable timeout | Native Liquid, explicit endpoint/TLS/domain-validation arguments; default 30-second timeout |
| Address list, wrong chain, missing dependency, unsupported backend kind | Named compatibility (`address_list`, `non_bitcoin_chain`, `dependency_unavailable`, `backend_kind`) | Named compatibility (`address_list`, `non_liquid_chain`, `dependency_unavailable`, `backend_kind`) |
| Silent Payments | Named `silent_payment` compatibility | No Bitcoin/SP native route |
| Any proxy or `.onion`, including Electrum | `proxy_transport`: binding's SOCKS spelling cannot guarantee Kassiber's remote DNS semantics; direct native constructor also refuses | `proxy_transport`: binding cannot preserve explicit remote-DNS proxy route |
| Esplora HTTP or HTTPS, even direct without credentials | Always `http_route_policy` after more specific reasons: cannot disable redirects/ambient proxies | Always `http_route_policy`: same limitation; direct native client constructor refuses HTTP too |
| Esplora custom CA / insecure / explicit operator CA roots | `custom_ca` / `insecure_tls` / `system_ca_trust`; native WebPKI roots cannot honor Python/OpenSSL selection | Same reasons and restriction |
| Auto-discovered host CA bundle | Does not by itself force custom trust; Esplora still `http_route_policy` | Same |
| Electrum custom certificate | `custom_ca`, no custom trust-store hook | `custom_ca`, no custom trust-store hook |
| Electrum explicit insecure | Native can pass `validate_domain=False`; not treated as a boolean string | `insecure_tls`: pinned verifier advertises no signature schemes and cannot complete that handshake |
| Custom HTTP auth / nondefault timeout | Esplora `custom_http_auth` / `custom_timeout` before HTTP-route reason; Electrum timeout supported | HTTP stays compatibility; Electrum nondefault/invalid timeout is `custom_timeout` |
| Descriptor shape not representable by native wallet | BDK branch/identity preparation refuses unsupported shape | `noncanonical_multipath`, `separate_change_descriptor`, `descriptor_unsupported`; equivalent canonical change descriptors may be normalized |

`kassiber/core/chain_observer/host_trust.py:18` recognizes only
`KASSIBER_HOST_CA_BUNDLE=explicit` as operator-selected trust. The frozen-build
auto-discovery value `1` does not confer that meaning. Native scans record client
construction, not each request, and only recheck offline at a scan boundary.

## What the current tests actually prove

- `tests/test_no_egress_guard.py`: DNS APIs, TCP connect, UDP sendto/sendmsg,
  loopback allowance, restoration and inherited Python-child installation;
  a broken child hook terminates before user code. `tests/integration/env.py:52`
  patches Python sockets; `EgressBlocked` derives from `BaseException` so ordinary
  product error wrapping cannot hide a trip.
- `tests/conftest.py:115` arms `KASSIBER_TEST_NO_EGRESS=1` outside integration
  runs and isolates the offline preference. It deliberately permits loopback
  fakes. `tests/_child_hooks/sitecustomize.py:36` treats any nonempty test value
  (or fallback product value) as armed; `strict` blocks loopback and installation
  failure requests exit 78. The in-process `env_flag` recognizes truthy strings
  instead; conftest installs it explicitly. The two semantics are not identical.
- `tests/test_daemon_smoke.py::DaemonSmokeTest::test_daemon_launch_makes_no_connection_at_all`
  (method at line 1722) exercises launch through daemon-ready
  under the strict child guard. This is stronger than the suite's default
  non-loopback guard, but cannot intercept native SDKs or non-Python children.
- `tests/test_proxy.py::test_process_override_blocks_compatibility_transport_before_dns`
  proves shared HTTP, SOCKS and Electrum refusal before opener/DNS/connect.
  Its explicit-route, HTTPS-proxy, redirect and CONNECT/TLS tests exercise the
  actual urllib handler chain. `tests/test_egress_policy.py` proves offline
  refusal of shared transport, ongoing Electrum writes, AI HTTP/broker, updates,
  CLN and background acquisition/refresh; the local AI exception is explicit.
- `tests/test_bdk_observer.py` and `tests/test_lwk_observer.py` pin capability
  routing, trust handling and refusal before native constructor under
  `KASSIBER_NO_EGRESS`. These are explicit boundary assertions, not interception
  of every SDK request by the Python guard.
- `tests/test_ai_core_provider_settings.py` pins cached display reads, enabled
  and acknowledged provider checks and selected-runtime scope;
  `tests/test_ai_core_api_format.py` pins refusal of unacknowledged form tests;
  `tests/test_ai_core.py` / `test_ai_core_http_transport.py` pin HTTP behavior.
  `tests/test_update_check.py` proves default transport inert before canonical
  consent, no ambient proxy, no redirects and cache-only ordinary commands.
- `tests/test_lnd_adapter.py::LndRestTransportTest::test_client_construction_does_not_probe_the_configured_node`
  (method at line 526) pins no LND construction probe;
  its transport tests and `tests/test_proxy.py` cover shared routing.
  BTCPay/exchange/rates inherit that shared boundary; their adapter suites are
  not independent strict-loopback consent proofs for every public action.
- `tests/test_chain_analysis_consent.py` runs real AI tool/once-only consent
  logic under strict DNS/socket refusal for approval, denial, cancellation,
  timeout and stale plans. Persisted work is not renewed network consent.
  `tests/test_egress_ledger.py` checks bounded metadata/redaction, not complete
  socket coverage.

Remaining Python gaps are the per-surface consent ownership at callers,
native/child guard blind spots, child and update ledger omissions, SDK
construction-only accounting, and default loopback permission. A source-code
inventory or a green socket-patched run alone is not an OS-level egress proof.

## Rust gate and HTTPS/HTTP foundation

`crates/net` (`kassiber-net`) owns every Rust backend IP transport. It uses a
blocking guarded connector, rustls with the ring provider, and httparse for
HTTP/1.1. The standard library has no TLS or HTTP parser; these libraries
provide protocol and cryptographic implementation without owning a connector.
`url` supplies URL/IDNA/origin handling, `base64` supplies proxy authentication,
and native/Mozilla roots supply the existing Python trust choices. Dependencies
are locked in the root Cargo.lock; licenses are in THIRD_PARTY_LICENSES.md.
No rates, sync, BTCPay, AI or update action is enabled by this foundation.

### Authority and policy

`Endpoint` accepts a validated ASCII host/port, never a URL. HTTP URLs are
parsed separately (including IDNA), with userinfo, fragments, control characters
and non-HTTP schemes refused. `Request` binds subsystem, endpoint, explicit
proxy and the actual TLS trust snapshot. `Consent::explicit_trigger` /
`narrowly_scoped` attest a verified action or enabled feature for that exact
request. `Gate::authorize` consumes request and proof into a private `Route`;
only `Route::connect` or an HTTP adapter consuming that route constructs a
`Connection`. The adapter verifies its URL's host, port and HTTP/HTTPS choice
against the route before I/O. A URL, trust setting, saved provider, client
construction or display read grants no authority.

The application must mint proof at its trusted consent boundary, bind it to
its book/input versions and action, and revoke clones on cancellation, feature
disable, lock/switch or stale inputs. This crate cannot authenticate a click or
infer a book revision. Consent is an ephemeral action attestation, not a
persisted job or a credential. A route cannot be publicly cloned or retargeted.

The shared Sync (including LND) and Pricing gate checks `KASSIBER_NO_EGRESS`.
AI and Update match Python's `require_online` and remain outside that override.
All paths check a fresh injected offline reader, consent
revocation and scope before DNS, each address connect and every physical write,
including proxy negotiation, TLS handshake, TLS alerts and post-handshake
records. TLS receives only the private guarded `Transport`. No client sees a
raw socket, descriptor, socket clone, unguarded connector or TLS stream field.
Offline blocks all routes except an explicitly local AI provider at loopback,
matching Python; an unavailable preference fails closed for every route.
The preference reader must fail closed; this crate does not select a data root
or implement storage policy.

### Trust and proxies

Resolve trust before minting consent:

- `TlsTrust::python_default()` mirrors the frozen Python CA bootstrap: honor
  nonempty `SSL_CERT_FILE` with no fallback if invalid; otherwise select the
  first loadable Linux bundle from the same four paths; otherwise use embedded
  Mozilla roots (`BundledRoots`). Other frozen platforms use the bundled roots.
  This preserves the host/override/bundled selection intent of
  `tests/test_pyinstaller_entry.py`; Mozilla roots are maintained by
  webpki-roots rather than the separately versioned certifi package.
- `TlsTrust::platform_roots()` explicitly selects the native platform store
  for source/non-frozen behavior. Empty/error stores refuse. Native root
  discovery is local; rustls never fetches AIA, OCSP or CA material.
- `ca_file`, `backend_certificate` and `backend_pem` validate and retain an
  immutable DER snapshot (file/PEM input bounded to 8 MiB). Operator/backend
  roots replace default roots, matching `ssl.create_default_context(cafile=…)`.
  File replacement after consent cannot silently replace reviewed roots.
  Root source and contents participate in consent equality; diagnostics omit
  paths and certificates.
- `ExplicitInsecure` is refused by `Route::check` outside Sync (LND also uses
  Sync). It is a separately consent-bound backend setting, never a
  retry after verification failure. It skips certificate/name checks while
  retaining cryptographic handshake signature verification. `Plaintext` is
  explicit and cannot serve an HTTPS URL. TLS has no client keys, key logger,
  early data or cross-route session cache. HTTP negotiates only HTTP/1.1; raw
  TLS streams do not advertise HTTP ALPN.

`Proxy::new` accepts explicit `socks5`, `socks5h` and `http`. Both SOCKS
spellings use remote target DNS, including onion domains; IPv4/IPv6 literals
use SOCKS address types. Only the configured proxy resolves locally. Optional
SOCKS username/password authentication retains Python's nonempty 255-byte
bounds. HTTP proxies use absolute-form requests for plaintext HTTP and CONNECT
for TLS; proxy Basic authentication stays separate from target Authorization
and cookies, and target SNI/trust apply inside the tunnel. Raw backend routes,
including TLS, refuse HTTP proxies before DNS/connect,
matching `_connect_backend_socket` / `_connect_via_socks5`; CONNECT belongs only
to the HTTP adapter.

HTTPS proxies and unsupported schemes refuse before I/O. Onion targets without
a proxy refuse before resolution. Ambient/OS proxies and `NO_PROXY` are never
read. Failed proxy, trust, TLS and handshake attempts never fall back to direct,
plaintext, alternate roots or another Tor port. Credentials are supplied
separately from endpoint metadata and are redacted from Debug.

### Client API

Blocking I/O fits the existing gate and the binary's worker-thread model:
there is no executor, connection pool, idle task, preconnect, retry, cookie jar
or background discovery. Async callers must dispatch it on a blocking worker.
`Connection` implements guarded `Read`/`Write` for raw TLS protocols and future
WebSocket codecs. `HttpRequest::allow_upgrade` permits a 101 response;
`HttpResponse::into_upgrade` preserves buffered bytes and returns a guarded
`Upgrade` stream. The future WebSocket adapter must validate the WebSocket
handshake, including Sec-WebSocket-Accept, before interpreting frames. This
foundation does not implement WebSocket framing or AI SSE streaming.

```rust,no_run
use kassiber_net::{Consent, Endpoint, Gate, HttpOptions, HttpRequest,
    Ledger, Offline, Request, Subsystem, TlsTrust};
# fn main() -> Result<(), Box<dyn std::error::Error>> {
// Supply the application's fresh, fail-closed preference reader here.
let gate = Gate::new(|| Offline::Online, Ledger::default());
let request = Request::new(Subsystem::Pricing,
    Endpoint::new("rates.example", 443)?, TlsTrust::python_default()?);
// Mint only after verifying this exact action's consent/book/input scope.
let proof = Consent::explicit_trigger(&request);
let route = gate.authorize(request, Some(proof))?;
let response = HttpRequest::new("GET", "https://rates.example/prices")?
    .header("User-Agent", "kassiber/<product-version>")?
    .send(route, HttpOptions { max_response_bytes: 2 * 1024 * 1024,
        ..HttpOptions::default() })?;
// The owning service validates status and interprets the bounded body.
# let _ = response;
# Ok(()) }
```

Callers supply the existing product User-Agent and service headers. Header names
and values are validated; Host, framing, connection and proxy-auth headers are
adapter-owned. Requests, responses and upgraded streams have no Debug
projection. Errors contain fixed diagnostics and never include URLs, peer
headers, reason phrases, response bodies, credentials or TLS peer details.
`Error::app_error()` supplies the shared Python policy envelope codes/messages
(`network_egress_disabled`, `network_proxy_unsupported`,
`network_proxy_required`, `backend_tls_config_invalid`, `backend_timeout`).
Service-specific AI/BTCPay/update status/body mapping and rate-limit retry policy
remain with those future slices; raw HTTP status/headers/body are returned so
those mappings do not depend on a generic HTTP exception or leak peer text.

`HttpOptions` defaults to Python's 30-second blocking-operation timeout. Its
optional monotonic deadline also bounds connect/handshake, reads/writes and
redirects. The system `ToSocketAddrs` resolver, like Python's blocking
system resolver, cannot be interrupted with this API: it is checked before
entry and before subsequent connect, and elapsed resolution consumes the
connect budget. There is no resolver worker that can outlive the action.

Responses, including errors, default to a 16 MiB body limit with a hard 64 MiB
cap; updates can request their Python 2 MiB bound. Python's generic backend,
rates, BTCPay and AI buffered reads currently have no universal body bound;
this is an intentional bounded-memory tightening. Headers and chunk metadata
are each bounded to 64 KiB, headers to 100, interim responses to eight. Fixed,
chunked and EOF framing are supported; conflicting/duplicate framing,
truncation and unsupported transfer encodings fail closed. No implicit content
decompression occurs. The client buffers a bounded body; service streaming
adapters can build on the guarded connection in their own slice.

Redirects default to `RedirectPolicy::Refuse`, matching backend/update policy.
AI callers may explicitly select `SameOrigin`: the exact scheme, normalized host
and effective port must remain equal. Every hop keeps the original proxy,
trust, consent and policy rechecks. Like urllib, POST becomes GET on
301/302/303 with body/content headers removed; POST 307/308 and other mutating
verbs are refused. urllib's four-visits-per-target and ten-distinct-target limits are retained. No authority is inferred from a
Location header, and no off-origin host is resolved or connected.

### Observation and verification

The RAM ledger retains the Python metadata fields with 5,000-record / 2 MiB
limits, monotonic IDs and eviction gaps; snapshots cap at 2,000 rows. Closed
labels and validated endpoints exclude paths, queries, bodies and headers.
TLS write counts are physical encrypted record bytes, including handshake
traffic; plaintext counts are application bytes. SOCKS setup records the proxy;
payload/TLS writes record the logical destination. Nothing automatically
exports the ledger, installs a TLS key logger or writes logs to disk.

`test-observer` (also active for unit tests) retains R0's strict/loopback modes,
independent bounded attempt buffer, uncatchable-by-Result panic marker and exit
78 for guard-incapable armed builds. Product policy refusals leave attempts
empty. The observer is a test boundary, not interception of foreign SDK/child
sockets. The route-less CI namespace required by plan 21 remains a second
layer; this change adds no CI or service enablement.

`crates/net/tests/chokepoint.rs` scans other Rust sources/manifests for
network types, DNS, HTTP/TLS/WebSocket clients, std::net imports (including
aliases/grouped/comment-separated forms) and Tokio net/full features. A reviewed
direct-dependency allowlist also refuses unfamiliar crates, aliases and new
subtable dependency declarations until reviewed. Only kassiber-net is exempt;
owned loopback fakes stay inside its tests. Discrepancy tests inject both a
source file and an unfamiliar/aliased dependency through the real tree walk.
This conservative source/dependency regression guard is not a capability
sandbox, macro expander, FFI audit or audit of transitive native SDKs.

Run the requested root checks with `mise exec rust@1.99.1 uv@latest -- …`:
`cargo fmt --all --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked`,
`cargo test -p kassiber-net --features test-observer --locked`,
`cargo test -p kassiber-net --test chokepoint --locked`, and
`python -m tests.rust_port.classify_tests check`.
The std-only scanner can also run when dependency archives are unavailable:

```sh
CARGO_MANIFEST_DIR="$PWD/crates/net" mise exec rust@1.99.1 uv@latest -- \
  rustc --edition 2021 --test crates/net/tests/chokepoint.rs -o /tmp/kassiber-net-scanner
/tmp/kassiber-net-scanner
```

Regressions cover every subsystem across direct/TLS/SOCKS/HTTP proxy paths with
no consent, offline state and policy changes after authorization; scope changes;
root selection and immutable CA snapshots; authenticated proxy routes and onion
remote DNS; target SNI, invalid roots and wrong certificate names; bounded HTTP
framing; default and off-origin redirect refusal; same-origin POST redirects;
revoked redirects and upgraded TLS writes; secret-free errors and existing
bounded ledger/strict-observer/child-process contracts. Real transport tests use
only owned ephemeral loopback servers and checked-in synthetic certificates.
The public test server key is test-only material, never a wallet/spending key.

### Python policy parity decisions

Plan 21 retains Python behavior: offline permits an explicitly local AI provider
at a loopback IP or `localhost`, with exact-action consent still required; remote
AI and every other subsystem remain blocked. AI and updates do not consult
`KASSIBER_NO_EGRESS`, matching `require_online`; the test observer remains an
independent boundary. A configured loopback gateway is not local AI authority.
