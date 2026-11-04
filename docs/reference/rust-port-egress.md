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
`url` supplies URL/origin handling with `idna_adapter ~1.0` in ASCII-only mode
(no ICU4X dependencies), `base64` supplies proxy authentication,
and native/Mozilla roots supply the existing Python trust choices. Dependencies
are locked in the root Cargo.lock; licenses are in THIRD_PARTY_LICENSES.md.
No rates, sync, BTCPay, AI or update action is enabled by this foundation.

### Authority and policy

`Endpoint` accepts a validated ASCII host/port (including Python-compatible
underscores for Docker/service names), never a URL. HTTP URLs are parsed
separately with reviewed ASCII A-label hosts and no implicit IDNA conversion.
Userinfo, fragments, control characters and non-HTTP schemes are refused. `Request` binds subsystem, endpoint, explicit
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
All paths check the current injected offline state, consent
revocation and scope before DNS, each address connect and every physical write,
including proxy negotiation, TLS handshake, TLS alerts and post-handshake
records. TLS receives only the private guarded `Transport`. No client sees a
raw socket, descriptor, socket clone, unguarded connector or TLS stream field.
Offline blocks all routes except an explicitly local AI provider at loopback,
matching Python; an unavailable preference fails closed for every route.
The preference reader must fail closed; this crate does not select a data root
or implement storage policy. `Gate::new` retains fresh reads. Storage integrations
may use `OfflineCache` / `Gate::with_offline_cache` only with an authoritative,
non-reused cross-process preference generation. Every check reads that current
revision, cache misses verify the revision before and after loading, and missing
or racing revisions fail closed. Explicit `invalidate()` clears a snapshot.
A TTL, file timestamp alone, delayed file watcher or in-process-only generation
cannot prove freshness against another CLI process: retain fresh reads until
the storage slice supplies the publication/invalidation protocol. No production
preference cache is installed automatically.

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
  paths and certificates. Backend roots additionally accept a byte-identical
  pinned end-entity certificate, including LND/OpenSSL CA:TRUE shapes. The pin
  path checks validity and server purpose, verifies SAN identity with webpki,
  and uses Python/OpenSSL-style CN fallback only for DNS names without DNS SANs;
  IP identities always require an IP SAN. Other certificates defer to webpki's
  chain verifier. Both paths always verify TLS 1.2/1.3 handshake signatures.
  Operator/host stores do not acquire the backend pin exception.
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
use std::time::{Duration, Instant};
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
    .send(route, HttpOptions {
        deadline: Some(Instant::now() + Duration::from_secs(30)),
        max_response_bytes: 2 * 1024 * 1024,
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
optional monotonic overall deadline also bounds connect/handshake, reads/writes
and redirects. Future service slices must set one appropriate to their action;
`None` retains Python's per-operation behavior. The system `ToSocketAddrs`
resolver, like Python's blocking system resolver, cannot be interrupted with this API: it is checked before
entry and before subsequent connect, and elapsed resolution consumes the
connect budget. There is no resolver worker that can outlive the action.

Responses, including errors, default to a 16 MiB body limit with a hard 64 MiB
cap; updates can request their Python 2 MiB bound. Bitcoin Core `listsinceblock`
can exceed 16 MiB: its service slice must raise the limit to 64 MiB or page the
acquisition. Python's generic backend, rates, BTCPay and AI buffered reads
currently have no universal body bound; this is an intentional bounded-memory
tightening. Headers and trailers are separately bounded to 64 KiB, each chunk-size line
to 4 KiB, headers to 100, interim responses to eight. Chunk framing consumes
the response byte budget and reads retain the action deadline. Fixed, chunked
and EOF framing are supported. Identical duplicate Content-Length values are
accepted like Python; conflicting values, ambiguous framing, truncation and
unsupported transfer encodings fail closed. Field values decode as latin-1,
matching `http.client`. Host omits the scheme's default port; requests default
to `Accept-Encoding: identity`, and non-identity Content-Encoding responses
are refused. No implicit content decompression occurs. Close-delimited TLS
bodies accept a missing close_notify, matching Python's `suppress_ragged_eofs`;
fixed-length and chunked bodies still refuse premature EOF. The client buffers
a bounded body; service streaming adapters can build on the guarded connection in their own slice.

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
traffic; plaintext counts are application bytes. SOCKS and CONNECT setup record
`ProxyHandshake` against the proxy (CONNECT carries the closed method label);
payload/TLS writes record the logical destination, with the HTTP method on
application writes. Headers, including Proxy-Authorization, never enter records.
Nothing automatically exports the ledger, installs a TLS key logger or writes logs to disk.

`test-observer` (also active for unit tests) retains R0's strict/loopback modes,
independent bounded attempt buffer, uncatchable-by-Result panic marker and exit
78 for guard-incapable armed builds. Product policy refusals leave attempts
empty. The observer is a test boundary, not interception of foreign SDK/child
sockets. The route-less CI namespace required by plan 21 remains a second
layer; this change adds no CI or service enablement.

`crates/net/tests/chokepoint.rs` scans every file under each Rust package,
including non-`.rs` include/module targets and nested build/dist/vendor trees;
symlinks and special files fail closed. Only exact generated checkout roots
(`target`, Tauri target/node_modules/dist, `.git`, `.venv`) are excluded. The net
crate's sources remain the reviewed connector exception; its manifest is checked.
The scanner detects socket/DNS/HTTP/TLS clients, libc glob/grouped imports and
syscalls, nix sockets, Windows WinHttp/WinInet and NSURLSession, Python module
imports, and child constructors. Existing sidecar/native/test child launches
are allowlisted by exact file, enclosing function, call line and occurrence in
`reviewed-process-calls.txt`, never by entire file.

`toml_edit` (dev only, reusing the lockfile's existing parser) handles Cargo
manifests and `.cargo/config{,.toml}`. Patch/replace/source tables, dependency
aliases, alternate registries/git sources, unknown dependencies and unreviewed
explicit or forwarded `dep/feature` / `dep?/feature` activations are refused.
`reviewed-dependencies.txt` records allowed features per dependency, including
Tokio's process/io/time-only direct surface and nix's user/fs-only surface.
Offline Cargo metadata independently checks workspace dependency declarations;
Tauri declarations and all feature combinations are checked by the manifest
walk. Tests inject each reported evasion through the real file/config walker.
This guard is not an OS sandbox, macro expander, toolchain security boundary or
transitive native SDK audit. Owned loopback fakes stay in kassiber-net.

Run the requested root checks with `mise exec rust@1.99.1 -- …`:
`cargo fmt --all --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`,
`cargo test --workspace --locked`,
`cargo test -p kassiber-net --features test-observer --locked`,
`cargo test -p kassiber-net --test chokepoint --locked`, and
`python -m tests.rust_port.classify_tests check`.
The scanner uses the reviewed TOML/JSON dev parsers and runs with Cargo's offline
cache: `cargo test -p kassiber-net --test chokepoint --offline --locked`.

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

HTTP host configuration must use ASCII A-labels. A socket-free Python experiment
with `urllib` found that non-latin-1 hostnames raise `UnicodeEncodeError` before
connect, while latin-1 names can reach DNS with a latin-1 Host field. Python does
not uniformly normalize IDNs. Rust therefore enforces the requested existing
Endpoint A-label boundary for URLs too (including percent-encoded Unicode hosts),
rather than silently granting consent to a converted host. Unicode URL paths
remain supported. Underscores remain valid, matching Python service hostnames.

### Per-operation transport cost

A Connection read checks policy once, including reads satisfied by rustls's
plaintext buffer; nested transport reads reuse that operation and still check
revocation. Every physical write, including TLS writes caused by a read, checks
current policy again. There is no cached write grant. Read/write socket timeouts
are updated separately and only when the effective timeout changes; active
overall deadlines still shrink before blocking I/O. The revision cache above
avoids repeated preference parsing only where freshness can be proved.
CONNECT replies read in 4 KiB blocks and retain any bytes beyond the header for
the guarded TLS reader, with the existing header cap and connection deadline.

## Rates service paths (R5)

`crates/rates/src/providers.rs` is the only rates network adapter. It builds
`kassiber-net::HttpRequest` GETs under the `pricing` subsystem. Constructing a
provider, opening a book, summary/coverage, cache lookup, manual rates and
local/bundled CSV ingestion create no connection. No timer is installed.

`FetchConsent` binds the database-instance identity, exact operation (`Sync`,
`Rebuild`, `Latest`, or `EnabledRefresh`), source and normalized pair. An
explicit all-pairs action may cover both supported pairs. CLI sync/rebuild mint
only the matching action; cached `rates latest` never fetches. Desktop latest
first requires the active profile's market-rate opt-in. Enabled-refresh grants
also bind that profile and recheck its feature before each request. Saving a
provider/backend does not mint a grant. Wrong-action, wrong-pair or cross-book
grants fail before transport. The existing gate rechecks process offline state
and the persisted preference before connection/writes, including loopback.

| Provider path | Action | Native no-traffic-before-consent regression |
| --- | --- | --- |
| Coinbase `/products/{pair}/candles`, recent latest window | desktop latest or enabled refresh | `coinbase_latest_no_traffic_before_exact_consent` |
| Coinbase transaction-needed UTC blocks | sync/rebuild | `coinbase_transaction_need_no_traffic_before_consent_and_incremental_cache` |
| Coinbase continuous day windows when idle warming is requested | explicit sync/rebuild | `coinbase_idle_continuous_windows_require_consent` |
| CoinGecko `/api/v3/coins/bitcoin/market_chart` (latest and historical) | latest, sync/rebuild or enabled refresh | `coingecko_sync_and_latest_no_traffic_before_consent` |
| Selected mempool backend `/api/v1/prices` | latest, idle sync/rebuild or enabled refresh | `mempool_latest_idle_and_historical_no_traffic_before_consent` |
| Selected mempool backend `/api/v1/historical-price?currency=…&timestamp=…` | transaction-needed sync/rebuild | `mempool_latest_idle_and_historical_no_traffic_before_consent` |
| Every provider's rebuild path | rebuild grant only | `every_provider_rebuild_requires_its_own_action_grant` |
| Service method selection and background idle warming | matching method; enabled Coinbase/mempool refresh does not warm an idle cache | `action_grants_cannot_relabel_fetches_or_warm_background_cache` |

All tests use owned loopback fakes in `kassiber-net::test_support`, compiled
only with its explicit test-support feature. That feature does not enable the
process observer in normal binaries. `every_live_path_offline_blocks_before_traffic`
pins Python's `network_egress_disabled` code for every live provider's latest
and sync paths. `background_grant_rechecks_feature_and_book_identity` pins
feature revocation and cross-book refusal. `desktop_reads_are_local_and_live_disabled_without_feature`
pins inert display reads and disabled desktop fetches; native daemon oracle
tests also compare those records with Python.

Only mempool inherits the explicitly selected backend proxy; Coinbase and
CoinGecko have no provider proxy setting or environment proxy discovery.
Credentials are percent-decoded as urllib does, SOCKS sends remote target DNS,
and no alternate proxy port is tried. Default/custom host TLS, redirects,
bounded responses, ledger redaction and offline enforcement remain owned by
the shared client. Retry remains bounded to three attempts and 20 cumulative
seconds; HTTP-date Retry-After and complete error-envelope parity are open
rates gates. The preference reader also enforces `crates/paths` private-file
ownership/mode checks, a stricter fail-closed rule than Python's reader.

Desktop rebuild is unported and returns `not_implemented` before cache deletion,
transaction repricing or network work. Background scheduling and AI projections
remain unported/Python respectively. This is a partial service checkpoint, not
permission to enable automatic traffic.

### Combined R5 scanner review

The combined stack retains the reviewed client's dependency and feature checks.
Rates reuse the client's pure `HttpUrl` parser; the direct `url` dependency stays
inside `kassiber-net`. The explicit allowlist includes the service crates,
local archive/encryption and descriptor dependencies already pinned by their
slices, the rates inflater's `rust_backend`, and test-only provider fixtures.
Reviewed child calls include the setup partition export's existing
`age`/`rage` recipient-encryption invocation, bound to its exact file, function
and call line. Attachment URL validation's exact IPv6 literal parse is pure;
its scanner exemption does not permit address resolution, imports of network
modules, or socket creation. The regression keeps rejecting a socket call next
to that parse. The combined daemon advertises all registered kinds in Python's
order, with test-only kinds appended after the product kinds.

R9 also admits the existing local journal and transfer harness features. Privacy
hygiene imports only the three IP-literal value types for classification, without
DNS or sockets; the scanner recognizes that exact import and still rejects a
connector beside it. Its frozen Python differential test has one reviewed child
call bound to `privacy_reports.rs`, `reference`, and the exact launch line.

## Chain client paths (R5 partial)

`crates/chain` creates only `kassiber-net` routes under `sync`. Its HTTP clients
use explicit backend proxy/trust snapshots, no ambient proxies or redirects,
30-second default operation timeouts, a caller-supplied overall deadline and
bounded 429/503 retry (three attempts, eight host slots, 20 seconds total sleep).
Raw Electrum uses the same gate and explicit SOCKS route, including `ssl://`,
`tcp://` and Liquid endpoints; HTTP proxies remain refused for raw streams.
Configured proxies never fall back to direct. TLS/custom/self-signed policy
continues through the reviewed pinned verifier; no new TLS connector exists.
`Connection::set_deadline` now exposes the existing internal deadline control
for request-scoped Electrum reads/writes, including TLS-buffered plaintext.

HTTP bodies retain 16 MiB by default; Core `listsinceblock` selects 64 MiB.
Typed Core HTTP errors also enforce Python's 65,536-byte decoding cap and ID
binding. Electrum limits each batch's response bytes (including unsolicited
messages) to 16 MiB, and checks the overall deadline while ignoring unrelated
IDs. Core/Electrum methods that sign, spend or broadcast are not exposed.

| Egress path | Consent owner | No-traffic regression |
| --- | --- | --- |
| Esplora/mempool Bitcoin/Liquid GET JSON/text | caller supplies exact route consent and deadline | `every_client_refuses_without_consent_and_offline_before_any_attempt` (HTTP/HTTPS and DNS/loopback) |
| Core read-only JSON-RPC POST | same; cookie reads occur after authorization | same; `watch_only_methods_and_expired_deadlines_refuse_before_connection` |
| Electrum connect/version and call/batch TCP/TLS | explicit authorized connection; gate rechecks every write | same; `electrum_policy_change_stops_existing_session_and_write` |
| HTTP, Electrum, Core and BTCPay daemon tests | `ProbeConsent`, exact kind/arguments/book | `every_probe_requires_exact_action_arguments_and_book_and_refuses_offline`; hybrid `test_offline_probes_cannot_touch_loopback` |
| Core detection candidates | explicit detect action, local candidates only | same grant test; `detect_candidates_ignore_remote_configuration_and_refs_exclude_secrets`; hybrid detection fixture |

The hybrid offline regression supplies a configured local Core candidate and
both HTTP and Electrum loopback fakes. All five native probe kinds refuse or
return an empty discovery result with no fake-server hits. Unrepresentable
probe deadlines fail before connection (`oversized_probe_timeouts_refuse_before_connection`);
untrusted Retry-After calendars/offsets cannot overflow or panic the parser.

The real-store boundary reads the shared setup offline preference afresh through
its `Gate`; invalid/unreadable preferences fail closed. Launch, display, backend
resolution and constructors do not probe. Test-only owned HTTP/Electrum fakes
remain in `crates/net/src/test_support.rs`; the direct client differential
example is not registered as a product or AI command. HTTP responses now expose
the bounded, untrusted reason phrase for explicit probe output, never transport
errors or the ledger. No disk logging or additional network permission is added.

`ui.backends.lightning.test` is native (see
[desktop settings](rust-shell.md#desktop-settings-and-sync-status-r5)): it
builds the LND/CLN report client for the one named backend, grants it for that
request, and keeps only the node alias, network, height and counts.
Outstanding: complete cross-language TLS/proxy/failure tapes and full wallet
sync. The
existing net suite exercises pinned LND/self-signed certificates and SOCKS;
that inherited proof is not claimed as new end-to-end Lightning migration.
Native HTTP probing currently buffers the bounded response before sampling
4096 bytes, while Python reads only that prefix; streaming/oversize probe
behavior is an explicit remaining parity gap.

## Bitcoin compatibility discovery (R5 preparation only)

`crates/wallets/src/scan.rs` is a preparation helper with no CLI/daemon/AI
registration and no database writes. Constructing an action, loading its
descriptor or reading its saved checkpoint opens no connection. Explicit
`DiscoveryConsent` binds one immutable action and is single-use and revocable;
the action includes the backend route/trust/proxy, descriptor, checkpoint and
database/profile/wallet/environment/source scope. A different action or changed
current scope refuses before network access. Saving a backend or pending job
never grants consent. There is no timer, pool or background scan.

| Preparation path | Exact authority | Regression |
| --- | --- | --- |
| Esplora/mempool GET `/scripthash/{reversed_sha256}` with at most eight workers | explicit discovery grant, rechecked by the shared client before connect/write | `every_discovery_path_requires_exact_action_consent_and_current_scope_before_traffic` |
| Compatibility Electrum TCP/TLS connect/version and `blockchain.scripthash.subscribe` batches | same grant and request deadline; no ordinary BDK route substitution | same regression; `electrum_status_batches_preserve_cached_non_null_and_requery_null` |
| Cached/fixed discovery, including loopback | offline/unavailable policy refuses even without a needed probe | `offline_refuses_both_paths_including_cached_and_fixed_scans` |
| Already transmitted discovery | cancellation prevents later batches; in-flight I/O remains deadline-bounded | `cancelled_discovery_returns_no_partial_result_and_stops_before_the_next_batch`; `revocation_between_http_batches_prevents_further_requests` |
| Per-backend explicit Tor/SOCKS proxy | inherited route only, with no direct retry | `proxy_failure_never_retries_the_destination_directly` |

The no-consent test covers HTTP/HTTPS and TCP/TLS, loopback and an unresolved
non-loopback name, with an empty independent attempt buffer. The Python
differential driver also asserts no fake-server hits without consent and while
offline for each protocol. Owned loopback fakes stay in the existing net test
support. HTTP auth, custom TLS and remote-DNS SOCKS inherit the reviewed chain
client implementation; this checkpoint adds no full TLS/proxy differential
claim. Cooperative cancellation does not interrupt a blocking socket operation.

Targets and cached history hints grant no ownership, custody, accounting or
absence/retraction authority. Product integration must still perform the local
book-network and source-overlap preflight and revalidate scope before atomic
publication. No transaction fetching or persistence, graph follow-up, freshness
job or new product egress is enabled here.

## Bitcoin protocol acquisition (R5 preparation only)

`crates/wallets/src/acquisition.rs` consumes a fixed target list and backend
snapshot, bound to database/profile/wallet/environment/source scope. Creating
the action opens no connection. Only `AcquisitionConsent::explicit_trigger`
grants that exact action; discovery consent cannot authorize acquisition.
Grants are revocable and single-use. No product command, daemon kind, AI tool,
timer or background worker calls this helper yet. The future coordinator must
perform book-network/source-overlap preflight before acquisition and revalidate
scope before atomic publication.

| New acquisition path | Exact authority and regression |
| --- | --- |
| Esplora statistics before/after history, `/scripthash/{hash}/txs/chain[/{txid}]`, `/txs/mempool`, `/utxo`, `/blocks/tip/height` | Exact acquisition action; `acquisition_every_path_requires_current_scope_and_exact_single_use_consent` and `acquisition_offline_cancellation_and_deadlines_precede_all_protocol_paths` |
| Electrum version/connect, subscriptions, histories, listunspent, raw transactions and distinct prevouts, block headers and tip | Same two regressions for TCP/TLS, loopback and unresolved hostnames; the protocol remains the gated chain client's fixed watch-only allowlist |
| Saved target lists, including empty lists | Still require consent and current online policy; no stored checkpoint or target is a grant |
| Proxy routing | Existing route/trust/auth only; `acquisition_proxy_failure_has_no_direct_fallback` proves no direct destination hit after proxy failure |
| Cancellation/revocation | Checked between pages/protocol batches and before return; `acquisition_cancellation_revocation_and_replay_return_no_partial_batch` proves no partial result. In-flight I/O remains deadline-bounded |

`tests/test_rust_wallet_acquisition.py` separately proves zero fixture requests
before consent and offline over both protocols. The result preserves raw
protocol data, including pre/post Esplora statistics and Electrum raw headers;
it does not assert completeness, canonical-chain membership, absence,
ownership, custody or tax facts. No database/checkpoint is written. Native
transaction decoding validates txid and header length with fixed diagnostics;
these stricter malformed-input refusals are preparation behavior, not a claim
of full Python sync-error parity. TLS/custom trust/auth inherit the existing
chain client; this adds no new full TLS/proxy differential evidence.

The shared backend parser now also rejects non-string configured proxy values
instead of treating them as absent and selecting a direct route.
`acquisition_invalid_proxy_types_refuse_instead_of_selecting_direct` covers both
proxy keys, nested/top-level configuration and both protocols before any I/O;
the `acquisition-proxy-type` mutation restores the bug and is killed.

## Bitcoin observer state/reuse (R5, inert partial step 1)

`crates/wallets/src/observer_state.rs` and `crates/store/src/observer.rs` add only
local preparation reads and pure commitment/aggregate transformations. They
construct no backend client, accept no network grant, create no dependency
wallet/sidecar, start no job and perform no durable write. Loading a saved
observer or a current transaction proof cannot grant acquisition, discovery,
canonical-chain, retraction, ownership or tax authority. The future coordinator
must still obtain exact-action consent through the existing gate before any
protocol operation, even on a restart with reusable records.

`observer_state_load_and_reuse_are_inert_without_consent_online_and_offline`
checks both policy states, no supplied consent, empty independent attempt and
ledger buffers, and unchanged database write counts. The subprocess regression
`test_state_restart_reads_match_python_preserve_complete_book_without_consent`
runs under the strict egress observer, including offline mode, and compares the
complete pre-existing book before/after repeated reads. The existing chokepoint
scanner remains the enforcement against an unreviewed connector/dependency.
There is no new egress path or proxy fallback; Esplora/Electrum product sync
remains unregistered. Inherited loopback transport tests require execution
outside this run's socket-restricted sandbox.

The state normalizer's additional duplicate-list, byte-separated-hex and
descriptor-network cases run through the same inert regression online and
offline, without consent and with empty ledger/attempt buffers. Wrong-network
keys and malformed hex refuse locally; no descriptor check probes a backend.
This remains representation/read verification, not a native sync path. The
offline registry lacks both `bdk_wallet` and `bdk_electrum`; neither dependency
nor an alternate transport was fetched to bypass that boundary.

## Native Bitcoin BDK execution and compatibility caches (R5 step 1)

BDK's native transport crates are deliberately absent. `bdk_wallet` 3.0.0,
`bdk_chain` 0.23.3 and `bdk_core` 0.6.3 provide pure in-memory state, indexing,
updates and merge semantics under `std`; no file-store or network feature is
enabled. Cached connector source is used only to implement protocol translation.
`bdk_wallet_uses_pinned_pure_state_without_transport_or_file_store` verifies the
resolved wallet dependency tree, including the exact pins and the absence of
BDK connectors and alternate HTTP clients. Source/dependency chokepoint checks
continue to reject independent connectors outside `kassiber-net`.

| Preparation path | Consent and route | Named regressions |
| --- | --- | --- |
| Native Electrum BDK full/revealed-script execution, histories/raw txs/parents/headers/Merkle proofs | `ExecutionConsent` binds one immutable scope, public source, prior state and backend; single-use; all calls through existing `ElectrumClient` and `kassiber-net` | `bdk_requires_exact_consent_and_online_even_for_loopback`; differential `test_cached_acquisition_and_native_bdk_require_consent_and_online` |
| Prepared native result | cancellation/deadline and offline checked again before return; no persistence | `bdk_offline_toggle_before_return_discards_prepared_state` |
| Cached Esplora and compatibility Electrum | existing acquisition consent also binds the immutable checkpoint/history snapshot and explicit book network; cache hits do not grant consent | `compatibility_cached_paths_require_bound_consent_and_online`; same differential no-traffic regression |
| Native BDK explicit proxy | same selected route and TLS policy; no SDK socket constructor or direct retry | `bdk_proxy_failure_never_connects_directly_to_the_backend` |

The native execution snapshots the tip/suffix before histories and uses
request-local caches. Disconnected or behind prior state currently refuses;
canonical recovery and authoritative retractions are the next numbered step.
Python's Esplora route stays compatibility. Native cache preparation preserves
complete/source-bound reuse and incomplete positive memberships, but confers no
absence, ownership, custody or tax authority. No new product command, background
feature or AI projection is enabled. No new diagnostic/disk logging is added.
See the [native execution checkpoint](rust-shell.md#native-bdk-execution-and-compatibility-caches-r5-step-1)
for verification, limitations and remaining registration/persistence work.

## Bitcoin canonical authority preparation (R5 step 2)

The [step-2 authority checkpoint](rust-shell.md#bitcoin-canonical-authority-preparation-r5-step-2)
supersedes the temporary disconnected-tip refusal above. Tip preflight, stale-
anchor rebuild and point-of-agreement recovery use the same request's existing
`ElectrumClient`; they introduce no connector, background work, provider probe,
DNS shortcut or direct proxy fallback. The immutable action also binds forced
refresh policy, prior canonical membership and the identity-local checkpoint.
`bdk_rebuild_paths_require_exact_consent_online_scope_and_return_fence` pins
consent/scope/offline before entry and offline/cancellation after preparation
for reorg and forced rebuilds. The disposable-loopback oracle
`test_native_rebuild_preflight_and_consent_fences_match_python` proves no
requests without consent or while offline and refusal before histories when
the backend is behind.

Esplora's source-bound membership calculation and forced checkpoint extraction
are local. Complete current histories can now prepare absence candidates, while
partial, uncovered and page-limited observations retain their positives.
`test_esplora_absence_sequences_match_python_source_bound_completeness` checks
saved-baseline no-traffic behavior online without consent and offline, together
with source changes and forced refresh. No durable observation is deleted at
this checkpoint; the future atomic coordinator must still fence scope/epoch
and compose every live observer before applying a withdrawal.

The translator's preflight/header/cache distinction has a direct regression
(`bdk_preflight_does_not_seed_the_scan_header_cache`) and an actual chain change
between preflight and suffix acquisition in the Python/native oracle. All
network operations still pass through `Wire`, rechecking cancellation/deadlines,
and the prepared result remains subject to the return-time offline/consent
fence. Product sync paths, automatic freshness and AI exposure remain disabled.

## Bitcoin normalized record composition (R5 continuation)

The [record checkpoint](rust-shell.md#bitcoin-normalized-record-composition-r5-continuation)
adds no egress path. `bitcoin_records`, `bdk::projection` and `composition` are
pure preparation code; they have no client, connection, database handle, path
or background worker. The bounded `records_fixture` stdin/stdout driver is
test-harness-only, accepts no endpoint or filesystem dispatch, and has no
product or AI registration.

`test_normalized_records_and_composition_match_live_python_without_consent`
compares the complete pure projection online without consent and with the
product offline switch enabled, under the subprocess no-egress guard. The
existing native BDK driver projects records only after the consent-bound scan
returns successfully. `test_cached_acquisition_and_native_bdk_require_consent_and_online`
and `test_native_rebuild_preflight_and_consent_fences_match_python` still prove
zero requests for missing consent and offline, including saved state, forced
refresh and loopback. The inherited proxy-failure, scope, cancellation and
return-fence tests remain required and unchanged.

Electrum decoder serialization fixes occur after acquisition. No fallback
connection, DNS lookup or proxy shortcut is introduced. Complete record JSON
parity grants no durable application authority: the future coordinator must
still bind scope/epoch and apply every state group in one wallet savepoint.

## Bitcoin native fetch assembly (R5)

`wallets::fetch::Fetch` captures local single-wallet scope, observer epochs,
freshness and closed reuse evidence without transport or writes. Its exact
action grant wraps one shared `kassiber-net::Consent`; the discovery,
acquisition and BDK stages inherit that same revocable transport grant.
Configuration, display, construction and saved observer state grant nothing.
The assembler introduces no connector and no BDK networking crate. Esplora's
tip read uses the existing consent-bound HTTP client after history and before
UTXOs, preserving the Python depth-reference ordering.

The native fetch refusal tests prove no DNS, connection or HTTP metadata for
missing/foreign/revoked consent, changed scope, cancellation and offline mode.
Unsupported source/backend shapes, additional profile wallets and active
transactions refuse locally. The test-only publication driver independently
prepares and publishes against owned loopback fixtures under the subprocess
guard. CLI, daemon and AI sync routing remain unregistered; this checkpoint
does not enable automatic freshness. See the
[fetch handoff](rust-shell.md#bitcoin-native-fetch-assembly-r5).

## Bitcoin product sync and freshness (R5, no-refresh)

The [product checkpoint](rust-shell.md#bitcoin-wallet-sync-product-routes-r5-no-refresh)
supersedes the unregistered daemon status of the earlier checkpoints. The three
native product run kinds admit explicit journal opt-outs and supported Bitcoin
Esplora/Electrum sources. Admission and the guarded hybrid selector initiate no
transport; the selector never opens or migrates a database. Saved backends and
jobs, launch, status and idle confer no sync or graph authority.

Only the exact product invocation creates its once-only request consent.
Foreground acquisition still uses the captured `kassiber-net` gate and existing
clients, with current offline/no-egress, scope, replay, cancellation and
revocation fences. There is no BDK networking dependency or alternate-backend
fallback. After successful terminal serialization, write and flush, private
successful-job grants authorize bounded graph follow-up on the captured
backend. The worker owns its connection and rechecks authority between batches
and before cache writes. Source pause/cancel, restore and authenticated rotation
retain the same stopping and connection-retirement barriers.

`test_product_consent_fences_before_transport` and
`test_product_source_controls_remain_available_during_fetch` drive the registered
product paths. The complete product oracle records per-invocation hybrid
decisions and rejects any served-shape reference fallback. Default refresh
remains dependent on native R4-13 journal processing; no Python journal bridge,
new AI allowlist entry or disk logging is introduced.
## CLN profitability subprocess acquisition (R5 preparation)

`kassiber_chain::lightning::ClnReportClient` serves the profitability read/CSV
routes through the shared book service described below.
Its `kassiber-net` request binds the book, profile, connection, window, export
path and backend configuration, including the executable and every CLN option.
Construction is inert. The gate checks current consent, offline/unavailable
policy and `KASSIBER_NO_EGRESS` before launching `lightning-cli`, rechecks while
the child runs, and kills/waits on refusal or timeout. Only nine read-only
`ClnMethod` variants are accepted; no arbitrary method, shell or argument-list
API is exposed. The helper is not an AI tool.

The process owns its CLN IPC/commando transport. The parent does not implement
or claim to observe the child's DNS, proxy or TLS. Unlike Python's CLN path,
the native launch is covered by the product no-egress switch. RAM observation
uses `operation=Process`, `scheme=cln`, the fixed `lightning-cli` label and port
1 as a process marker; these are not a fabricated TCP destination. Paths,
peer ids, runes, stdout and stderr never enter those records or error details.
Captured stdout is bounded and parsed inside the client; stderr is discarded.

The test observer blocks unapproved subprocesses even in its loopback mode.
Only a gate-local executable registered by a test-support build can run as a
fixture; strict mode blocks it too. The CLN fixture executable contacts only
its disposable loopback fake node. Process comparisons exercise actual child
arguments and acquired projections against unchanged Python. Policy refusal
processes prove zero node hits, process attempts and ledger rows before exact
action consent. This is acquisition evidence, not public CLI/daemon acceptance.

## Lightning profitability product routes (R5)

`reports lightning-profitability`, `reports export-lightning-profitability-csv`
and `ui.reports.lightning_profitability` now use the native shared book service.
Only invocation of one of these exact actions creates its request-local grant.
Loading the book, listing wallets/backends, status, route registration and saved
configuration create no grant and perform no node request or subprocess launch.
The grant binds the operation, book/profile/connection, window, export path and
effective private backend configuration. Scope/config changes are checked before
each resource and before publication, and completion/failure revokes the grant.
Offline and `KASSIBER_NO_EGRESS` refuse loopback and CLN processes too.

CLN retains the subprocess visibility limit above. LND uses the reviewed HTTP,
TLS and explicit proxy routes without redirects or direct retry around a proxy.
Only curated open channels and routing aggregates reach the report renderer;
raw snapshots, credentials, invoice/payment data and node bodies are not returned
or persisted. The full operator report/CSV is not an AI or diagnostics projection.
The desktop backend test reuses these clients with its own request-local grant;
the AI node-snapshot routes remain unported. The [service checkpoint](rust-shell.md#lightning-profitability-product-wiring-r5)
records actual wire/CSV/database and served-by verification.

## Chain-analysis acquisition apply (R5)

`analysis_ops::acquisition::apply` ports `apply_acquisition` for Esplora,
Bitcoin Core RPC and Electrum (Bitcoin and Liquid transaction bytes). The
reviewed plan is recomputed from the saved backend row, book and local index;
only an exact canonical match mints `Consent::explicit_trigger` for the
backend's `kassiber_chain::Backend` request (TLS trust and explicit proxy only).
A budget mirrors Python: `6 + 2 × max_transactions` requests, 45 seconds,
8 MiB per response and 64 MiB in total, and checks `KASSIBER_NO_EGRESS` and the
offline switch before every request and the Electrum handshake. HTTP sends no
retries and refuses redirects; Core's typed 400/404/500 envelopes are read,
other statuses become `history_unavailable` without server prose. Responses are
sanitized to public legs (sequence kept; scripts, witnesses and preimages from
inputs dropped), Bitcoin feature snapshots are extracted before discard, and the
grant is revoked before the atomic local save, which revalidates the plan.

`tests/test_rust_analysis_acquisition_apply.py` serves synthetic transactions
from loopback Esplora/Core and Electrum fakes and compares native and Python
request sequences, CLI/daemon bytes and all 112 tables across ten traversal
cases, plus offline, `KASSIBER_NO_EGRESS`, stale-plan, genesis and spender-index
refusals; refused, stale and offline actions send nothing. Rust unit tests with
the `kassiber-net` loopback fake pin plan binding, offline refusal, depth and
transaction limits, spender corroboration and witness discard; six named
mutations are killed. Known divergence, retained as a Python reference repro in
`tests/rust_port/repro_acquisition_electrum_truncated.py`: truncated Electrum
transaction bytes raise a `RuntimeError` in embit, which escapes Python's
frontier handling and aborts the whole acquisition; the native port records a
`transaction_unavailable` frontier instead.

## Chain-analysis source runner (R5)

`analysis_ops::acquisition::runner` ports `chain_analysis_backfill.run` and
`run_due`; `analysis_ops::worker_tick` and the daemon's `watches` thread port
`daemon_chain_analysis_watches`. The worker starts after a request only for an
unlocked keyed book, opens its own connection fenced to the database identity,
and runs at most one due grant per profile and pass, then due watches. Offline
mode pauses acquisition; local watch evaluation continues. A pass leases the
grant for 90 seconds; every request settles the previous reservation, checks
`KASSIBER_NO_EGRESS` and offline mode, revalidates the leased grant and reserves
8 MiB of lifetime quota before transport, through a `narrowly_scoped` grant for
the backend's exact route. Block ranges are parsed with the consensus decoder,
checked against the header hash and merkle commitment, and stored as sanitized
reference assertions; a changed cursor block withholds the source and rewinds
one block per pass. The worker emits only the generic
`ui.chain_analysis.watches.changed` event; inbox identities never leave the
book. Lock, restore, passphrase rotation and shutdown stop it, waiting up to 10
seconds for a bounded read.

`tests/test_rust_analysis_watch_worker.py` runs both daemons against a loopback
Bitcoin Core fake and compares events, request sequences and all 112 tables for
watch evaluation, subject and block-range runs, and a reorg with resume. Rust
unit tests with the `kassiber-net` loopback fake pin lease exclusion, quota
reservation and refund, the lifetime quota pause, lease release and the offline
pause; five named mutations are killed. Malformed block bytes that make embit
raise outside Python's handled exception types abort the Python pass; the native
runner records `invalid_observation` instead.
