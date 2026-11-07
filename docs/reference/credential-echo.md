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

`kassiber/redaction.py` and the Rust crate `kassiber-credentials` implement
the same scrub, byte for byte (`crates/credentials/tests/data/vectors.json`
holds the Python outputs the Rust tests reproduce):

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
  pseudonymization. Where a caller still holds the backend when an error
  leaves it (the BTCPay store sync), it scrubs the whole error again with
  `withhold_error_credentials`.

## Inventory

"Before" is the Python reference before this change (round 8 already covered
exchange keys). Each echo path has a regression test that fails on the old
code; "Tests" names them (`test_credential_echo.py` in process, with a
loopback server that repeats every credential the request carried, and
`test_credential_echo_cli.py` through the command line, so the same checks run
against the Rust binary through `KASSIBER_TEST_CLI`).

| Integration | Credentials sent | Error paths | Where the text landed | Before | Now | Tests |
| --- | --- | --- | --- | --- | --- | --- |
| Kraken, Coinbase, Binance | API key and secret; proxy userinfo | non-2xx body (`HTTP n from backend`), Kraken `kraken_errors` | envelope, ring, diagnostics, AI tool result | key and secret withheld (round 8); proxy password echoed by an HTTP proxy | proxy userinfo withheld too | `ExchangeCredentialEchoTest` (test_exchange_importers), `ProxyCredentialEchoTest`, CLI `test_kraken_sync` |
| BTCPay Greenfield | API key (`token`); proxy userinfo | error body `message`, `code`, `missingPermission`; retry text; unreachable-host reason | envelope, ring, diagnostics, AI tool result; **book**: `btcpay_store_sync_states.last_error_message`, `freshness_jobs.error_json`, `freshness_source_states.last_error_message` | echoed and stored | decoded body scrubbed; store-sync handler scrubs again before storing | `BtcpayCredentialEchoTest` (incl. a freshness job writing the book), `ProxyCredentialEchoTest`, CLI `test_btcpay_store_sync` |
| LND REST | macaroon (hex header); proxy userinfo | HTTP error body (first 200 chars), non-JSON `response_preview`, reason | envelope, ring, diagnostics, AI tool result, freshness job | echoed, no scrub at all | withheld in either hex case | `LndCredentialEchoTest`, CLI `test_lnd_wallet_sync` |
| Core Lightning (`lightning-cli`) | commando rune (argument) | `details.stderr` tail | envelope, ring, diagnostics, AI tool result; `lightning_node_syncs.error_json` keeps only the fixed message | echoed | withheld | `ClnCredentialEchoTest`, CLI `test_core_lightning_wallet_sync` |
| Esplora (HTTP) | bearer token or auth header; key-like URL query values; proxy userinfo | non-2xx body | envelope, ring, diagnostics, AI tool result, freshness job | pattern redaction only (a bare token passed) | every credential the request carried withheld | `ChainBackendCredentialEchoTest`, CLI `test_esplora_wallet_sync` |
| Bitcoin Core RPC | `user:password` (Basic) or cookie; proxy userinfo | untyped non-2xx body; rescan failure text; typed 400/404/500 RPC errors | as Esplora | untyped body and rescan text echoed; typed errors already kept no server text | withheld; typed errors still keep none | `ChainBackendCredentialEchoTest` (incl. cookie, typed errors, rescan), CLI `test_bitcoin_core_wallet_sync` |
| Electrum | none reaches the server (proxy userinfo goes to the SOCKS proxy only) | `Electrum call … failed`, non-JSON preview, unparseable socket URL | envelope, ring, diagnostics | could not echo a credential; a socket URL that did not parse was quoted with its userinfo (and urllib's port error quoted part of it) | backend credentials withheld as a backstop; URL shown redacted | `ChainBackendCredentialEchoTest` |
| BDK / LWK native observers | none: a backend with a token, auth header or proxy keeps the Python transport | n/a | n/a | cannot echo | cannot echo | `NoEchoPathTest.test_credentialed_esplora_backends_never_reach_the_bdk_observer` |
| Chain-analysis acquisition | Esplora/Core credentials | categorical errors only | envelope | cannot echo (no server prose kept) | cannot echo | `NoEchoPathTest.test_chain_acquisition_errors_are_categorical` |
| OpenAI-compatible AI providers (chat, models, connection test, document extraction) | API key; base-URL userinfo or key query | HTTP body preview; error object in a 200 answer or stream (`body`, `provider_code`); network reason | envelope, ring, diagnostics; `ai_provider_checks.error_json` keeps code and fixed message only | only `sk-` keys recognized; others echoed, and lowercased into `provider_code` | withheld before the cut and before lowercasing | `AiProviderCredentialEchoTest`, CLI `test_ai_provider_test` |
| AI broker (Claude Code, Codex CLIs) | none: the CLI holds its own credential | n/a | n/a | cannot echo | cannot echo | — (Kassiber holds no credential to withhold) |
| Rate providers (CoinGecko, Coinbase) | none; proxy userinfo | non-2xx body | envelope, ring, diagnostics | an HTTP proxy's echo of its credentials passed | proxy userinfo withheld | `ProxyCredentialEchoTest.test_a_rate_source_proxy_error_withholds_proxy_credentials` |
| Proxy and Tor settings | SOCKS5 or HTTP proxy `user:password` | SOCKS replies (binary codes, no text); HTTP proxy 407/502 bodies; malformed proxy setting; unparseable port | envelope, ring, diagnostics | malformed setting quoted verbatim; urllib's port error quoted part of the userinfo | setting shown redacted; parse errors withheld | `ProxyCredentialEchoTest` |
| Update check (GitHub) | none | categorical error | envelope | cannot echo | cannot echo | `NoEchoPathTest.test_update_check_errors_keep_no_response_text_and_send_no_credential` |

## Existing books

Opening a book runs `stored-error-credentials-v1` once (Python
`_migrate_stored_error_credentials`, Rust `kassiber_store::stored_errors`, same
rows, bytes and audit). It withholds every credential the book holds (backend
tokens, auth headers, RPC and proxy passwords, AI provider keys) from
`freshness_jobs.error_json`, `freshness_source_states.last_error_message`,
`btcpay_store_sync_states.last_error_message`, `lightning_node_syncs.error_json`
and `ai_provider_checks.error_json`. JSON columns are scrubbed decoded and
rewritten compactly with sorted keys; the audit records how many values
changed. Tests: `StoredErrorMigrationTest`, the store's `stored_errors` unit
tests and `test_stored_error_credential_scrub_matches_python`.

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
