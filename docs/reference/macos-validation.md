# macOS validation of the Rust backend (2026-11-07)

The Mac-only gates of [plan 21](../plan/21-rust-backend.md): native SQLCipher
and Keychain interop, the signed sidecar identity the Touch ID helper checks,
macOS-specific test behaviour, and the R6-4 Touch ID port. Every run used
disposable books under `target/macval/` with
`KASSIBER_OFFLINE_PREFERENCE_FILE` set there. No real book,
`/Applications` app or existing Keychain item was read or changed. Nothing
was notarized, uploaded, pushed or installed.

## Machine and toolchain

```text
Hardware   Apple M5 Pro, 18 cores, 48 GiB, Touch ID with 1 enrolled finger
macOS      27.0.1 (build 26A434), arm64
Xcode      27.2 (27B52); CLT 27.2.0.0.1792721180
Rust       rustc 1.99.1 (58cf8f9c3 2026-10-13) from rustup's 1.99.1 toolchain
           (Homebrew's rustc 1.99.1 is also on PATH; mise is not installed)
Python     3.11.15 (uv-managed), uv 0.11.6, `uv sync --locked --python 3.11`
Node       v26.12.0; pnpm 10.33.0 via packageManager (global pnpm is 12.14.0)
Base       origin/t3code/rust-mac-validation at acd3f3e00
```

Pinned crates, wheels and npm packages were downloaded once for the builds
(owner-approved). Every Kassiber process stayed offline or loopback-only.

## Results

| Item | Result | Evidence | Code | Recommendation |
| --- | --- | --- | --- | --- |
| A1 Toolchain and build | PASS | `build-macos-arm64-rust-app.sh --with-python-app --no-backup` exited 0 in 227 s (Rust release `cargo build` 89 s). Rust sidecar 123,297,016 B arm64; `otool -L`: Security, CoreFoundation, libiconv, libSystem only. `kassiber-ui` 14,880,144 B. Apps 132 MB each, DMGs 60 MB (Python) / 47 MB (Rust). Daemon smoke serves 286 kinds. | `scripts/build-macos-arm64-rust-app.sh` | rustup's minimal 1.99.1 `rust-objcopy` cannot find `libLLVM.dylib`, so debug-info stripping fails with a warning (harmless: 90 KB). Debug builds warn `__eh_frame section too large`. I ran a copy of the script without the final `--version` probe of `/Applications/Kassiber Dev.app`. |
| A2 Test suites | FAIL (tests only; no product bug found) | Workspace: 2972 passed / 38 failed. 24 failures are macOS test-harness issues (see A2 notes), 14 would fail on Linux too. After the fixture fix: net 65/66, chain 38/40, wallets 389/389. `kassiber --features test-harness` 43/43. Python: preflight 57 passed, cli-smoke 74 passed, security-replication 968 passed / 1 failed / 160 skipped. | A2 notes | Commit `ec99b14d2` fixes the non-blocking `accept` fixtures. The rest is listed below. |
| A3 SQLCipher interop | PASS | Python-created (non-ASCII passphrase) ⇄ Rust-created books: each side writes, the other reads, and all 113 manifest tables dump byte-identically after every write. Both report `kdf_iter 256000`, `cipher_page_size 4096`, HMAC_SHA512/PBKDF2_HMAC_SHA512, no plaintext header. Opening with `kdf_iter 64000`, page 1024 or compatibility 3 fails. `test_rust_store_interop.py`: 116 passed, 3 failed (a test that hardcodes `"linux"`), 2 skipped. | `crates/store/src/lib.rs:828`, `encryption.rs:577`, `crates/store/Cargo.toml:25-30` | Linkage: Rust uses SQLCipher 4.14.0 + **CommonCrypto** (system Security.framework, `SQLCIPHER_CRYPTO_CC`), SQLite 3.51.3. Python's `sqlcipher3` 0.6.2 wheel bundles SQLCipher 4.12.0 + OpenSSL 3.6.0, SQLite 3.51.1. A set `OPENSSL_DIR` would make libsqlite3-sys link Homebrew `libcrypto`; the build script's `otool` guard refuses that. Fix the interop test's platform. |
| A4 Keychain formats | PASS (with prompts) | Test items `kassiber-mac-validation-*`, class `genp`, login keychain, `svce`/`acct` attributes, UTF-8 value. Python→Rust (ad-hoc) read: one Allow dialog, SHA-256 identical. Rust→Python read: one Allow dialog, identical. Two **different** Rust binaries signed with the same Developer ID identifier and team: the second read the first's item with **no** dialog. Deny → Rust adapter `secret_store_unavailable`. All test items deleted. | `crates/secrets/src/native.rs:153-155`, `kassiber/secrets/unlock_store.py:262-290` | Ship the Rust sidecar with the Python sidecar's identifier `kassiber-cli-aarch64-apple-darwin` and team. Items the signed Python sidecar wrote should then stay readable without a dialog (inferred from the signed-pair test; not run with a real Python sidecar). Ad-hoc dev builds prompt after every rebuild. |
| A5 Signing identity and layout | PASS (ad-hoc and Developer ID); not notarized | Ad-hoc: app `Identifier=kassiber_ui-0992cffe1a53d924`, sidecar `kassiber-529cd877e6f12e2d`, no team. The Rust check refuses the helper (`native_auth_unavailable`). Developer ID copy (sidecar `--identifier kassiber-cli-aarch64-apple-darwin`, app `at.bitcoinaustria.kassiber`, runtime, no timestamp): the helper **accepts** the Rust sidecar and **refuses** ad-hoc, wrong-identifier and out-of-bundle sidecars. Re-signing only the sidecar breaks the app seal, and the Rust check refuses that first. | `ui-tauri/src-tauri/src/lib.rs:4351-4445`, `crates/operator/src/native_auth.rs` | Layout matches what the helper checks: `Contents/Resources/binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin`. The helper does not require hardened runtime (a no-runtime sidecar was accepted); the release `verify_code` enforces it separately. `spctl` rejects the un-notarized copy, as expected. |
| A6 HTTPS/TLS and no egress | PASS | With the fixture fix, all six TLS loopback regressions pass; the only remaining net failure is the CLN 100 ms timing test. `python_default()`: `BundledRoots` without `SSL_CERT_FILE`, file roots with it, refusal with an invalid file (no fallback). `platform_roots()` loads the macOS store. The product sidecar on a fresh book (seeded Fulcrum default) opened **0** inet and **0** unix sockets through CLI `init`/`status`, the daemon's handshake, six status/display kinds and 45 s idle (`lsof` every 100 ms; positive control seen). | `crates/net/src/tls.rs:52-95` | `platform_roots()` also honours `SSL_CERT_FILE` (rustls-native-certs), so it is not always "the native store"; document it. |
| A7 Restore under `/var` | PASS | Raw `TMPDIR=/var/folders/…/T/` (→ `/private/var`): `backup export` + `import --install` for Py→Py, Py→Rust, Rust→Py, Rust→Rust. All install, tables identical, nothing left in TMPDIR. `test_rust_backup_install.py` + `test_backup_recovery.py`: 1391 passed, 4 failed (they also fail with a resolved TMPDIR; see A2). | `crates/backup/src/lib.rs:351`, `crates/store/src/restore.rs:70` | — |
| A8 Durability | REPORTED (gap) | Rust file writes (`durable::atomic_write`, restore staging and journal, encryption migration, credentials) call `File::sync_all`. On Apple targets std implements it as `fcntl(F_FULLFSYNC)`, including for directory syncs. SQLite in both runtimes: no `PRAGMA fullfsync`/`checkpoint_fullfsync` (no match in the repo), so commits use plain `fsync`. Python: every `os.fsync` is plain `fsync(2)`. | Rust: `crates/store/src/durable.rs:105-147`, `lib.rs:909-937`. Python: `kassiber/db.py:3465-3519`, `private_files.py:17-37` | Set `PRAGMA fullfsync=ON` (and `checkpoint_fullfsync=ON` for WAL books) on macOS in the Rust store. Use `fcntl(F_FULLFSYNC)` in Python's helpers while it still writes books. Fix the Rust and Python write paths that skip a file or directory sync (A8 notes). |
| A9 `/bin/ps` labels | PASS | `peers::tests` pass on macOS. For all 1356 live processes, Rust `process_label` and Python `peers.process_label` return identical labels (951 `None`, then `launchd_sim`, `codex`, `ChatGPT`, `Claude`, …). | `crates/operator/src/peers.rs:143-190` | — |
| B Touch ID (R6-4) | PASS (port, signed identity checks); BLOCKED (Touch ID prompt) | Native `operator touch-id status/enroll/forget` and `unlock --auth touch-id`. 19 native-auth unit tests, server and service tests, 7 byte-identical differential steps, 18/18 mutations killed. Signed run: status `available: true`; enroll reached the helper's data-protection store and failed there without a provisioning profile; unlock under a pty → `operator_native_auth_not_configured`; a shell-parented CLI refused. Rust and Python derive the same account for the test book. | `crates/operator/src/native_auth.rs`, `server.rs`, `service/mod.rs`, `crates/secrets/src/modes.rs`, `crates/cli/src/operator_cli.rs` | Rerun with the Developer ID provisioning profile to see the Touch ID prompt (steps in B notes). |

## A2 notes: failing tests

macOS-specific, test harness only:

- **Non-blocking `accept`:** on macOS an accepted socket inherits the
  listener's `O_NONBLOCK`; on Linux it doesn't. This failed 6 `kassiber-net`
  `adapter_tests`, 4 `kassiber-chain` and 7 `kassiber-wallets` tests through
  `adapter_tests::accept` and `test_support`'s `FakeProvider`/`FakeElectrum`.
  Fixed in `ec99b14d2`. Product listeners poll and then use blocking
  `accept` (`crates/operator/src/server.rs`, `runtime.rs:514`).
- **CLN process timeouts:** `process::tests::cln_process_timeout_…` and
  `lightning::cln::tests::cln_report_process_errors_…` give a freshly written
  `#!/usr/bin/python3` script 100 ms. Its first exec on macOS takes
  88–108 ms (later runs about 19 ms). Use about 2 s, or run the script once
  first.
- **`lightning::tests::lnd_report_python_vectors_…`:** with the fixture fix
  it still fails here, because a loopback `connect()` occasionally stalls
  past 2 s. Plain Python reproduces this; the firewall is set to "block all
  incoming". Recheck on a Mac with default firewall settings.
- **`operator/tests/runtime.rs` `production_environment_variation_…`:**
  `AF_UNIX path too long`. The socket path under `/private/var/folders/…` is
  about 109 bytes and macOS allows 104. The product socket
  (`~/.kassiber/run/operator-v1.sock`) is 38 bytes here. Use a `/tmp`-based
  tempdir.
- **`ipc::process::tests::detached_children_lead_a_new_session`:** macOS
  `ps` has no `sid` keyword. Use `pgid` or `getsid`.
- **`store::projects::tests::legacy_books_under_linked_parents_…`:**
  hardcodes the Linux `~/.local/share` state root.
- **`reports::export_range::tests::export_range_vectors`, case
  `europe/vienna`:** case-insensitive APFS resolves the zone. Python on this
  Mac also accepts it, so the runtimes agree here and the vector is
  Linux-only.
- **`test_rust_store_interop.py::test_runtime_paths_match_python_without_writes`
  (3 cases):** the probe is asked for `"linux"` paths. With the real platform
  all 4 cases match Python.

Branch drift, which would also fail on Linux:

- `source_funds_*` (10 tests in `crates/reports/tests`): stale
  `ambiguous_reference` vectors; regenerate them from current Python.
- `journals` `process_refresh` (2 tests): Rust quarantine-reason precedence
  is behind the regenerated vectors.
- `daemon::transfers::…desktop_component_review_caps…`: expects a `null`
  action to pass.
- `test_audit_package.py`: expects 8 schema migration audits; there are 9.
- `chokepoint`: `crates/ai/src/chat/files.rs:159` spawns
  `pdfinfo`/`pdftotext`/`pdftoppm` without a reviewed-process entry. Separately,
  the scanner reads the build artifact `ui-tauri/src-tauri/binaries/`; exclude it.
- `test_rust_backup_install.py::test_ordinary_cli_restart_recovers_…`
  (settings case, 4 tests): the Rust `status` reads `config/settings.json`
  before it recovers a pending restore and answers `unsafe_configuration`.
- `crates/operator/scripts/mutations.json`: entry
  `agent-access-keeps-other-field` no longer matches
  `crates/setup/src/agent_access.rs`.

Hazard: with `KASSIBER_RUST_BIN` set,
`tests/test_rust_secrets_lifecycle.py::test_unavailable_remember_and_authenticated_forget_match_without_native_items`
isolates the keyring only by pointing at a missing D-Bus socket. On macOS it
would write and delete real login-keychain items. CI and these runs leave
`KASSIBER_RUST_BIN` unset for that shard, so the file is skipped. Skip it on
macOS.

## A4 notes: the dialogs

The dialog asked whether the reading program may use the confidential
information stored in the named item. It offered *Always Allow*, *Deny* and
*Allow*, with a login-password field.

- A binary that didn't create an item gets one dialog per read with
  *Allow*.
- *Always Allow* adds the reader to the item's ACL. For ad-hoc binaries the
  ACL entry is the binary's cdhash, so every rebuild asks again. For
  Developer ID builds it is the designated requirement (identifier plus
  team), which survives updates.
- Rust updates an existing item in place and keeps the creator's ACL.
  Python's `keyring` deletes and re-adds, so the ACL resets to Python.
- One dialog opened behind other windows. One ad-hoc read succeeded after
  12 s although no dialog was noticed; that step is inconclusive.
- Python and Rust both treat a failed or denied remembered-unlock read as
  "no item": they prompt, or answer `interaction_required`.

## A8 notes: write paths without a full sync

- **Rust:** `layout.rs` `write_private_preference` has no directory sync.
  Backup export (`crates/backup/src/lib.rs:159`) has no directory sync.
  `crates/operator/src/backoff.rs:186` writes and renames without any
  `fsync`. Several report and export writers use plain `fs::write`.
- **Python:** the project catalog (`projects.py:314-326`) has no `fsync`.
  Legacy project staging copies have no `fsync`. The SQLCipher migration
  publish (`secrets/migration.py:309`) is a plain `rename` with no syncs.
  `settings.json` and `atomic_write_private` have no directory sync.

The Rust store is already stricter than Python on every book-critical path.
The remaining gap is SQLite's own commits.

## B notes: Touch ID hardware run

Setup:
- Built from `t3code/rust-touch-id` and copied the Rust app to
  `target/macval/session/`.
- Replaced its sidecar with the new release binary.
- Signed the sidecar:
  `codesign --force --sign <Developer ID> --timestamp=none --options runtime --identifier kassiber-cli-aarch64-apple-darwin`.
- Signed the app the same way with `--identifier at.bitcoinaustria.kassiber`.
  It got no entitlements and no profile: none was available.
- Throwaway book: `init`, then `secrets init` with a non-ASCII passphrase.
- No Kassiber app was running. The broker used the shared per-user endpoint
  and was stopped at the end.

Each step (`$APP/Contents/MacOS/kassiber-ui --cli --machine --data-root <book> …`) and its result:

1. `operator touch-id status`, unbound: `available: false`, reason
   `operator_policy_binding_required`. The helper path checks passed.
2. `operator touch-id enroll --passphrase-fd 3`: the broker starts, the
   caller and helper checks pass, fresh authentication binds the policy and
   the broker-launched helper fails. The error is `native_auth_failed`,
   message `kassiber native auth: [redacted]`. Touch ID is enrolled on this
   Mac, so the cause is the missing Keychain entitlement
   (`errSecMissingEntitlement`).
3. `operator touch-id status`, bound: `{"available": true, "configured":
   false}`. The helper ran with the Rust sidecar as its parent and exited 4.
4. The same `status` through `bin/kassiber`: identical. The helper's parent
   is the sidecar either way.
5. The sidecar matrix, each time with the app re-sealed:

   | Sidecar | Result |
   | --- | --- |
   | Ad-hoc | `native_auth_failed` |
   | Developer ID, identifier `kassiber` | `native_auth_failed` |
   | Developer ID, no hardened runtime | accepted |
   | Correct signature, outside the bundle | `native_auth_failed` |
   | Correct signature, restored | accepted |

6. `enroll` through `bin/kassiber` (the parent is the shell): refused by the
   CLI's caller check with `native_auth_unavailable`, "unexpected signing
   identity".
7. `operator unlock --auth touch-id`: without a TTY,
   `interaction_required`. Under `script` (a pty), the request goes through
   the broker and the helper's `broker-get`, and returns
   `operator_native_auth_not_configured` with the enroll hint.
8. Cleanup: no `Kassiber Operator Biometric Enrollment` item exists for the
   test account. `operator stop` was run. No broker or sidecar process is
   left.

To finish the hardware run, add the Developer ID provisioning profile:
1. Copy it to `Contents/embedded.provisionprofile`.
2. Re-sign the app with the `scripts/macos_release.py` entitlements
   (`com.apple.application-identifier`, `com.apple.developer.team-identifier`,
   `keychain-access-groups`).
3. Run steps 2, 3 and 7 again. Expect `{"configured": true, "auth":
   "touch_id"}`, the system Touch ID prompt on unlock, and a lease.
4. Run `operator touch-id forget --passphrase-fd 3` and check that status
   shows `configured: false`.

Finding: the broker's detail filter (`sanitize_traceback_text`, the same in
Python's `log_ring.py`) redacts everything after `auth:`. Every helper error
therefore reaches the user as `kassiber native auth: [redacted]`.

## Needs the owner

- The Developer ID **provisioning profile** for `at.bitcoinaustria.kassiber`,
  and a ~5 minute session, to run the actual Touch ID prompt (B notes).
- **Notarization and a timestamped signature** were deliberately not done.
  `spctl` rejects the local signed copy.
- **Keychain prompts after the switch:** keep the sidecar identifier and team
  so items written by the Python sidecar stay readable without a dialog.
  Check once with a real signed Python build before release.
- **Decisions:**
  - SQLite `fullfsync` on macOS (A8).
  - The `auth:` redaction that hides helper errors (B).
  - The `platform_roots()` doc wording (A6).
  - Whether to fix the listed drift failures (A2).
- Every product-binary run (including the build script's daemon smoke) adds
  lock files under `~/.kassiber/run/operator-owners` for its disposable book.
  They are harmless and unrelated to real books.

## Branches and commits

- `mac-validation` (from `origin/t3code/rust-mac-validation` `acd3f3e00`):
  - `ec99b14d2`: the net fixture fix.
  - Merge of `t3code/rust-touch-id`.
  - This report.
- `t3code/rust-touch-id` (from `acd3f3e00`):
  - `3cdc7f4ae`: the R6-4 port.
  - `44d66fbd6`: the R6-4 docs.

Nothing was pushed.
