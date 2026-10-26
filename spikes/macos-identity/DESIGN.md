# Rust sidecar signing and Touch ID design

Verdict: feasible with conditions. This Linux machine cannot validate
Security.framework, Developer ID signatures, Gatekeeper, Keychain ACLs or
biometric hardware. The Cargo/Python spike checks only portable requirement,
identity-hash and build-origin serialization. The following is a proposed
port of existing boundaries, not a claim of a completed signed flow.

## What is checked today

1. `kassiber/operator/build.py` returns `{version,channel,commit,frozen,origin}`.
   Version comes from packaged metadata or the package version; empty channel
   and commit become null; commit is trimmed to 12 characters. Source origin
   is SHA-256 of the resolved Python package directory, truncated to 12 hex
   characters. Frozen origin uses resolved `sys.executable`. If both APPDIR
   and APPIMAGE exist and the executable is inside resolved APPDIR, it hashes
   resolved APPIMAGE instead, avoiding a new identity on every AppImage mount.
   The broker and client compare build dictionaries before handing off
   secrets / admitting same-build work. This is an accidental-build-mixing
   guard, explicitly **not authentication against same-user malicious code**.
2. Python's native-auth bridge requires macOS and a configured executable
   `KASSIBER_NATIVE_AUTH_HELPER` at the resolved enclosing app's
   `Contents/MacOS/kassiber-ui`. It inspects Identifier, TeamIdentifier and
   CDHash, requiring helper identifier `at.bitcoinaustria.kassiber`, a
   10-character uppercase ASCII alphanumeric team and 40/64 lowercase hex hash.
   Its Developer ID requirement is:

   ```text
   anchor apple generic and identifier "<identifier>" and certificate leaf[subject.OU] = "<team>" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists
   ```

   The opaque helper identity is SHA-256 of ASCII
   `kassiber-native-auth-signature-v1:{identifier}:{team}:{cdhash}`.
   `native_auth_caller_identity` inspects the live CLI **parent PID** and
   requires the entire triple to equal the configured helper's triple.
   A standalone sidecar launched by an arbitrary shell cannot impersonate
   the app's signed CLI launcher merely by inheriting the helper variable.
3. Before a helper action, the broker verifies the helper file and expected
   opaque identity, spawns the helper with ready/go pipes, waits up to five
   seconds for `R`, validates its **live PID** against the Developer ID
   requirement and exact triple, then sends `G`. The helper cannot touch
   Keychain before release. Enrollment reads a broker-created inherited fd;
   `broker-get` returns only through a broker-created inherited output fd.
   No secret goes through argv, stdout, a caller-chosen socket or back to the
   invoking CLI. Buffers are wiped, and reads are limited to 16 KiB.
4. The helper, before the gate and before **all** actions (get/store/delete/
   status), obtains its parent's path with `proc_pidpath(getppid())` and
   canonicalizes it. The parent must be under the same app's
   `Contents/Resources/binaries` with exact basename:

   | Architecture | Parent basename **and signing identifier** |
   | --- | --- |
   | Apple Silicon | `kassiber-cli-aarch64-apple-darwin` |
   | Intel | `kassiber-cli-x86_64-apple-darwin` |

   Both helper and parent static signatures are verified strictly, their
   teams must match, and the parent identifier must equal that basename.
   `SecCodeCopyGuestWithAttributes(kSecGuestAttributePid)` and
   `SecCodeCheckValidity` then verify the **running parent**, against the
   exact architecture-specific identifier, verified helper team, Apple
   anchor and Developer ID Application leaf OID above. A plausible path or
   matching static file alone does not pass.
5. Runtime code derives the team from the verified helper; it does not
   hardcode the release team's literal value. The release pipeline separately
   pins Apple team **6Q4R2C3GJK**. Current operator credentials use
   `AccessibleWhenUnlockedThisDeviceOnly` + `BIOMETRY_CURRENT_SET`, with no
   unsigned/ad-hoc/application-only fallback. The account hashes the current
   filesystem-bound project identity plus native-auth generation; keep that
   derivation and managed-policy binding exactly.

## Concrete port and packaging

- Keep `kassiber-ui` as the signed helper and CLI launcher. Package the single
  Rust backend Mach-O at the **same architecture-specific sidecar filename
  and location** above. Rust can build a binary called `kassiber`; packaging
  renames that artifact. No helper allowlist change is necessary. Changing
  only the sidecar basename to `kassiber` would fail today's parent check.
- Port the Python broker's full native-auth state machine: helper path
  canonicalization, strict static signature, expected opaque triple, live
  caller PID validation, inherited ready/go gate, live spawned PID check,
  secret-pipe direction/limits/cleanup, child liveness and zeroization.
  Use macOS Security.framework code APIs for the live-PID requirement and
  signing-information comparison. The requirement must include all current
  predicates; neither TeamIdentifier alone nor a CDHash of a disk file is
  sufficient. Keep project policy, inode/database binding, generation,
  namespace and lease rules in the Rust core.
- Preserve build-identity JSON field names and null/trim rules. For Rust,
  hash the canonical executable location (also for a dev binary) and keep
  the guarded APPDIR/APPIMAGE rule. Explicit embedded build metadata decides
  packaged (`frozen=true`) versus dev (`false`); Rust being compiled must
  not imply that it is a production distribution. Embed version/channel/
  commit at build time; never infer them from ambient runtime env. Different
  Python/Rust binaries/commits must remain different builds, requiring an
  idle broker restart before unlock. The spike pins the existing wire shape,
  not a new anti-tamper protocol or hash of the source tree.
- Sign the inner Mach-O with Developer ID Application, explicit identifier
  equal to its filename, secure timestamp and `--options runtime`, **no
  entitlements**. Statically vendored SQLCipher/OpenSSL eliminates those
  crypto dylibs; inspect `otool -L` for any remaining non-system libraries
  and sign/package them inside out as today. No JIT, unsigned executable
  memory, DYLD or disable-library-validation exemptions are needed for this
  Rust sidecar. Keep the separate TypeScript provider broker's packaging.
- Outer app identifier stays `at.bitcoinaustria.kassiber`. Preserve embedded
  Developer ID provisioning profile and exactly these app entitlements:

   ```text
   com.apple.application-identifier = 6Q4R2C3GJK.at.bitcoinaustria.kassiber
   com.apple.developer.team-identifier = 6Q4R2C3GJK
   keychain-access-groups = [6Q4R2C3GJK.at.bitcoinaustria.kassiber]
   ```

  The outer app performs protected Keychain access. Do not grant its
  entitlement/group to the sidecar as a shortcut. Existing ordinary Python
  remembered-unlock items may legitimately require a new executable ACL
  authorization; never broaden ACLs silently or copy biometric secrets into
  the ordinary CLI namespace.
- Replace PyInstaller's macOS one-dir collector with an artifact copy; keep
  the signing/notary pipeline, inner-to-outer signing order, pinned team,
  source-bound receipts, manifest and offline launch gates. The release
  validator currently discovers `BUILD_INFO.json`; supply an explicit signed
  app resource and embedded Mach-O build metadata, and validate their
  consistency against expected source/version **without executing downloads**.
  Remove Python `_internal`/extension discovery assumptions only in that
  packaging stage. One Mach-O is simpler to sign than a Python runtime plus
  many extensions, but still needs hardened-runtime signing and notarization.
- Continue shipping the app-bearing CLI archive for Touch ID. A standalone
  Mach-O can be notarized in a ZIP, but Apple cannot staple a raw binary or
  ZIP. Keep stapled app/DMG (or design a signed installer package) for offline
  first launch. A raw CLI outside the app also cannot satisfy the existing
  helper's same-bundle parent contract. Do not publish internal sidecars as
  standalone release assets to avoid that contract.

## Owner checklist on actual Macs

Use disposable books and copied builds, never existing owner credentials.

1. Build/package arm64 and x86_64 with embedded matching version/channel/
   source; run native arch (and Intel hardware or a separately recorded
   Rosetta result). Confirm no interpreter or `_internal` dependency.
2. Sign through the protected pipeline. Inspect `codesign -dv --verbose=4`
   for the app and sidecar, `codesign --verify --strict`, `otool -L`, app
   entitlements and profile; check exact identifiers, team, runtime flag,
   secure timestamps and empty sidecar entitlements.
3. From the app's CLI launcher, enroll, unlock, status, forget. Prove the
   helper gets an actual **Rust broker parent** and that the current-biometry
   Keychain access prompts on real Touch ID hardware. Check no secret on
   stdout/stderr/argv or diagnostics. Confirm no prompt on ordinary status.
4. Reject an ad-hoc sidecar, wrong-team signed sidecar, correctly signed but
   wrong identifier, renamed/moved outside-bundle parent, and direct helper
   invocation from a shell. Include same-team unrelated signed executable.
5. Hold the helper at readiness, replace the on-disk file in a disposable
   bundle, and verify live-PID/triple mismatch is refused before enrollment
   secret or Keychain action. Test early child exit, wrong/missing gate byte,
   timeout, invalid fd, oversized secret and cancellation cleanup.
6. Mix old/new app helper hashes and Python/Rust broker builds; require
   mismatch and broker restart before any passphrase handoff. Test AppImage
   origin vectors on Linux separately; they are not macOS identity evidence.
7. Reopen ordinary Python-created remembered-unlock items with the Rust
   executable; record ACL prompts and approve through the native user flow.
   Confirm desktop/operator/CLI services remain independent and forget does
   not revoke another namespace.
8. Verify database/passphrase rotation, filesystem replacement, project
   rename, stale generation and policy changes keep existing invalidation
   barriers. Test revoked credentials and changed biometric enrollment.
9. Notarize the exact signed artifact, inspect Apple's log, retain its input
   hash/receipt, staple app and DMG, rebuild final archive from sealed bytes,
   and verify release manifest. A signature alone is insufficient.
10. Download onto a clean Mac and test first launch while offline under
    quarantine with `spctl --assess --type execute --verbose=4` and
    `xcrun stapler validate`. Test terminal launcher and GUI daemon startup.
11. Exercise both archived and installed app CLI paths, upgrade from the
    current app, and rollback to it without changing existing book/keyring
    formats. Record hardware/OS/signing versions and failures in private
    owner validation evidence; Linux's passing vectors do not close R1.

## Sources

Current [broker build identity](../../kassiber/operator/build.py),
[native-auth bridge](../../kassiber/operator/native_auth.py),
[desktop helper](../../ui-tauri/src-tauri/src/lib.rs) (parent check near 4197,
live Security.framework check near 4306),
[protected Keychain policy](../../ui-tauri/src-tauri/src/secret_store.rs),
[operator reference](../../docs/reference/operator-broker.md),
[macOS release runbook](../../docs/reference/macos-release.md), and
[release signer](../../scripts/macos_release.py).

Apple's primary documentation: [Hardened Runtime](https://developer.apple.com/documentation/security/hardened-runtime),
[notarization workflow and stapling limits](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow),
[distribution testing](https://developer.apple.com/documentation/xcode/packaging-mac-software-for-distribution).
