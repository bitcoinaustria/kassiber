# Credential echoes

A server, proxy or tool that Kassiber talks to can repeat a credential it
received: an error body that quotes the request's `Authorization` header, a
JSON `message` naming the API key, `lightning-cli` printing its own arguments.
Pattern redaction (`redact_secret_text`) cannot recognize a bare API key,
macaroon, rune or password, so every integration that sends a credential also
withholds it, by value, from any text it gets back. No credential may reach a
CLI or daemon envelope, the RAM log ring, the diagnostics report, an AI tool
result or the book. See [privacy and security](privacy-and-security.md) and
[logging](logging.md) for those surfaces.

## The scrub

The crate `kassiber-credentials` implements the scrub. Its tests reproduce,
byte for byte, the outputs of the Python reference it was ported from, frozen
in `crates/credentials/tests/data/vectors.json`:

- **What is withheld.** Each credential the caller sent, as sent,
  percent-encoded (`quote` and `quote_plus`), JSON-escaped (with and without
  `\/`), and for a hex credential such as an LND macaroon in either letter
  case. Spellings are replaced longest first, ties by text, with `<redacted>`.
- **What a request carries.** `request_credentials` lists the URL's
  userinfo (password, `user:password`, its Basic base64, or a whole userinfo
  without a password, which is a token in the user position) and query values
  whose key names a secret, key, auth, signature or access token; the
  credential headers (`Authorization` and `Proxy-Authorization` decoded as
  Basic, plus cookies, macaroons, API-key and rune headers); and the proxy's
  userinfo. `backend_credentials` lists the same for a saved backend: token,
  auth header, RPC password with `user:password` and Basic forms, URL and
  proxy userinfo. A user name alone is not withheld: it is an identifier, and
  withholding a short one would garble the message.
- **When.** Before the text is cut short (an error keeps 200 characters, an
  AI body preview 512, CLN stderr the last 800), so a cut cannot keep a
  credential's prefix, and before pattern redaction and operational
  pseudonymization. The BTCPay client also scrubs every string key and
  leaf of a decoded error body (eight levels deep) before the error leaves it
  (`crates/chain/src/btcpay/client.rs`).

## Inventory

"Before" is the Python reference before the echo fix (round 8 already covered
exchange keys). "Tests" names the regression tests that failed on the old
code: `test_credential_echo.py` ran in process against a loopback server that
repeats every credential the request carried, and `test_credential_echo_cli.py`
ran the same checks through the command line. Both went with Python
([plan 22](../plan/22-python-removal.md)). The command-line cases are recorded
sessions that `scripts/replay-sessions.sh --only test_credential_echo_cli`
replays against the binary; the crate tests named under
[per integration](#per-integration) pin the rules one at a time.

| Integration | Credentials sent | Error paths | Where the text landed | Before | Now | Tests |
| --- | --- | --- | --- | --- | --- | --- |
| Kraken, Coinbase, Binance | API key and secret; proxy userinfo | non-2xx body (`HTTP n from backend`), Kraken `kraken_errors` | envelope, ring, diagnostics, AI tool result | key and secret withheld (round 8); proxy password echoed by an HTTP proxy | proxy userinfo withheld too | `ExchangeCredentialEchoTest` (test_exchange_importers), `ProxyCredentialEchoTest`, CLI `test_kraken_sync` |
| BTCPay Greenfield | API key (`token`); proxy userinfo | error body `message`, `code`, `missingPermission`; retry text; unreachable-host reason | envelope, ring, diagnostics, AI tool result; **book**: `btcpay_store_sync_states.last_error_message`, `freshness_jobs.error_json`, `freshness_source_states.last_error_message` | echoed and stored | decoded body scrubbed; store-sync handler scrubs again before storing | `BtcpayCredentialEchoTest` (incl. a freshness job writing the book), `ProxyCredentialEchoTest`, CLI `test_btcpay_store_sync` |
| LND REST | macaroon (hex header); proxy userinfo | HTTP error body (first 200 chars), non-JSON `response_preview`, reason | envelope, ring, diagnostics, AI tool result, freshness job | echoed, no scrub at all | withheld in either hex case | `LndCredentialEchoTest`, CLI `test_lnd_wallet_sync` |
| Core Lightning (`lightning-cli`) | commando rune (argument) | `details.stderr` tail | envelope, ring, diagnostics, AI tool result; `lightning_node_syncs.error_json` keeps only the fixed message | echoed | withheld; since round 7, commando is refused before `lightning-cli` starts (the rune would be in argv), so no rune is sent | `ClnCredentialEchoTest`, CLI `test_core_lightning_wallet_sync` |
| Esplora (HTTP) | bearer token or auth header; key-like URL query values; proxy userinfo | non-2xx body | envelope, ring, diagnostics, AI tool result, freshness job | pattern redaction only (a bare token passed) | every credential the request carried withheld | `ChainBackendCredentialEchoTest`, CLI `test_esplora_wallet_sync` |
| Bitcoin Core RPC | `user:password` (Basic) or cookie; proxy userinfo | untyped non-2xx body; rescan failure text; typed 400/404/500 RPC errors | as Esplora | untyped body and rescan text echoed; typed errors already kept no server text | withheld; typed errors still keep none | `ChainBackendCredentialEchoTest` (incl. cookie, typed errors, rescan), CLI `test_bitcoin_core_wallet_sync` |
| Electrum | none reaches the server (proxy userinfo goes to the SOCKS proxy only) | `Electrum call … failed`, non-JSON preview, unparseable socket URL | envelope, ring, diagnostics | could not echo a credential; a socket URL that did not parse was quoted with its userinfo (and urllib's port error quoted part of it) | backend credentials withheld as a backstop; URL shown redacted | `ChainBackendCredentialEchoTest` |
| BDK / LWK native observers | none: a backend with a token, auth header or proxy keeps the compatibility HTTP transport | n/a | n/a | cannot echo | cannot echo | `NoEchoPathTest.test_credentialed_esplora_backends_never_reach_the_bdk_observer` |
| Chain-analysis acquisition | Esplora/Core credentials | categorical errors only | envelope | cannot echo (no server prose kept) | cannot echo | `NoEchoPathTest.test_chain_acquisition_errors_are_categorical` |
| OpenAI-compatible AI providers (chat, models, connection test, document extraction) | API key; base-URL userinfo or key query | HTTP body preview; error object in a 200 answer or stream (`body`, `provider_code`); network reason | envelope, ring, diagnostics; `ai_provider_checks.error_json` keeps code and fixed message only | only `sk-` keys recognized; others echoed, and lowercased into `provider_code` | withheld before the cut and before lowercasing | `AiProviderCredentialEchoTest`, CLI `test_ai_provider_test` |
| AI broker (Claude Code, Codex CLIs) | none: the CLI holds its own credential | n/a | n/a | cannot echo | cannot echo | — (Kassiber holds no credential to withhold) |
| Rate providers (CoinGecko, Coinbase) | none; proxy userinfo | non-2xx body | envelope, ring, diagnostics | an HTTP proxy's echo of its credentials passed | proxy userinfo withheld | `ProxyCredentialEchoTest.test_a_rate_source_proxy_error_withholds_proxy_credentials` |
| Proxy and Tor settings | SOCKS5 or HTTP proxy `user:password` | SOCKS replies (binary codes, no text); HTTP proxy 407/502 bodies; malformed proxy setting; unparseable port | envelope, ring, diagnostics | malformed setting quoted verbatim; urllib's port error quoted part of the userinfo | setting shown redacted; parse errors withheld | `ProxyCredentialEchoTest` |
| Update check (GitHub) | none | categorical error | envelope | cannot echo | cannot echo | `NoEchoPathTest.test_update_check_errors_keep_no_response_text_and_send_no_credential` |

## Per integration

Every integration uses the `kassiber-credentials` scrub with the credential
lists above:

- **AI providers** (chat, streams, model lists, connection tests, document
  extraction): API key and base-URL credentials withheld from body previews,
  error objects (before lowercasing the code) and network reasons.
  `provider_errors_never_repeat_the_api_key` and the transport error unit
  tests pin it in `kassiber-ai`; the recorded `test_rust_credential_echo`
  sessions replay `ai providers test` byte for byte.
- **Esplora and Bitcoin Core HTTP** (`kassiber-chain`): every credential the
  request carried withheld from a backend error body before the cut. Core's
  typed RPC errors keep no server text.
- **BTCPay store-list check**: the decoded error body is withheld.
- **Electrum**: backend credentials withheld from error messages.
- **LND, Core Lightning, rate providers**: the clients keep no server text in
  their errors (no body, no `stderr`), so they cannot echo; their messages are
  shorter than the Python reference's.
- **Exchange syncs** (`crates/wallets/src/file_import/exchange_api`): the
  HTTP client withholds the API key and secret, and Kraken's error items are
  scrubbed one by one (`kraken_errors_keep_permission_codes_and_withhold_credentials`).
- **Core Lightning commando** (round 7): `lightning-cli` takes a rune only in
  argv, readable by every local user, so the report client refuses a
  commando configuration before any request, attempt or process; `ClnConfig` no longer carries a peer or rune.
  `cln_report_commando_configuration_is_refused_inert_and_secret_free`
  pins it; mutation entries `chain-cln-report-commando-{refused,peer,token,url}`
  and `chain-cln-report-argv-secret-free` are killed by that test and the
  argv pins.
- **BTCPay read-only keys** (round 7): a missing permission scoped to the
  route's store (`btcpay.store.canmodifystoresettings:<storeId>`) degrades
  label enrichment like the bare form; another store's scope still fails
  (`crates/wallets/src/sync/btcpay.rs`).
- **Stored error text**: the store's migrating open runs the same
  `stored-error-credentials-v1` migration.

## Existing books

Opening a book runs `stored-error-credentials-v1` once
(`kassiber_store::stored_errors`, ported from the Python reference's
`_migrate_stored_error_credentials` with the same rows, bytes and audit). It withholds every credential the book holds (backend
tokens, auth headers, RPC and proxy passwords, AI provider keys) from
`freshness_jobs.error_json`, `freshness_source_states.last_error_message`,
`btcpay_store_sync_states.last_error_message`, `lightning_node_syncs.error_json`
and `ai_provider_checks.error_json`. JSON columns are scrubbed decoded and
rewritten compactly with sorted keys; the audit records how many values
changed. Tests: the store's `stored_errors` unit tests.

The migration can only recognize credentials the book still holds. A
credential that was deleted, rotated, or set only in a `backends.env` file
before the upgrade stays in old error text.

## What to rotate

Rotation is only needed where a credential reached someone who should not
have it. The server that echoed a credential already had it; the risk is the
copies on Kassiber's side.

- **You shared Kassiber output** (a CLI error, a diagnostics report, a log
  export, a bug report, an AI chat with a remote provider) produced after a
  failing BTCPay, LND, Core Lightning, Esplora, Bitcoin Core, exchange or AI
  provider request: rotate that backend's credential (BTCPay API key, LND
  macaroon, CLN rune, RPC password or cookie, exchange key and secret, AI
  key) and any proxy password configured for it.
- **An AI provider read a failed tool result** for one of those backends (the
  in-product assistant can see tool errors): treat that provider as having
  seen the backend's credential and rotate it, unless the provider is local.
- **A book or backup left your control** after a failed BTCPay sync: rotate
  the BTCPay API key. Before this release that sync stored the server's error
  text in the book.
- **You deleted or rotated a backend, or keep it only in `backends.env`**:
  the migration cannot recognize that credential in old error text. If the
  old credential is still valid on the server, revoke it.
