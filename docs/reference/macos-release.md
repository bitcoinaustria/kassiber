# macOS release runbook

Kassiber separates build, local Developer ID signing, automated notarization,
and offline OpenPGP release authentication. No Apple signing private key is
uploaded to GitHub. The Apple team is `6Q4R2C3GJK` (Bitcoin Austria); changing
that trust root requires a reviewed code change in `scripts/macos_release.py`.

## One-time operator setup

1. Install Xcode command-line tools, Python 3.11+, and authenticated GitHub CLI
   on the signing Mac. Use a clean, reviewed checkout of current `main`.
2. Keep the Developer ID Application certificate and corresponding private
   key in the login keychain. Select its exact SHA-1 certificate fingerprint
   from `security find-identity -v -p codesigning`. This fingerprint selects a
   local identity; it is not a file-integrity hash or an SSH key.
3. Maintain an encrypted, recovery-tested backup of the identity outside Git.
   Record certificate expiry and renew before then. The initially inspected
   certificate expires 2027-02-01. Never revoke an old certificate merely to
   rotate it: revocation can affect users of previously shipped apps.
   Register the explicit App ID `at.bitcoinaustria.kassiber` and obtain its
   **Developer ID distribution provisioning profile**,
   authorizing the selected certificate and app Keychain group. No additional
   portal capabilities were needed for the validated production profile;
   the helper sets the narrow Keychain entitlements during signing.
   The helper requires this profile;
   certificate-only signing would leave Data-Protection-Keychain / operator
   Touch ID unavailable. Only the outer app gets the exact app identifier,
   team identifier and app-local Keychain group; sidecars/libraries get no
   entitlements. No debug, library-validation or executable-memory exemptions
   are inherited from previews. The profile is embedded as public authorization
   material; it does not contain the private signing key. Profile expiry also
   matters for future launches, so record and renew it separately.
4. Create a dedicated App Store Connect **team API key** for notarization.
   Store its `.p8` contents as `NOTARY_KEY_P8`, key ID as `NOTARY_KEY_ID`, and
   issuer ID as `NOTARY_ISSUER_ID` in GitHub environment `macos-notarization`.
   Enter secrets directly in GitHub or through `gh secret set` stdin, never
   in chat, committed files, command arguments, screenshots, or logs.
   This is a notary credential, **not** the Developer ID private key.
5. Restrict that environment to protected `main`. Protect workflow changes
   with CODEOWNERS/review and restrict tag creation/update/deletion to release
   maintainers. No PR job receives these credentials. Use hosted ephemeral
   runners, not a general Actions runner on the signing Mac.
6. Configure `release-production` for the current **single maintainer**:
   allow only the `main` branch and manually dispatch finalization after
   local signing/verification. Do not require a second reviewer or enable
   prevent-self-review while only one operator is available. This is explicit
   single-person authorization, not a four-eyes process. A second independent
   reviewer can be added later without changing the signing pipeline.
   Complete the separate offline
   [OpenPGP key ceremony](release-signing.md), publish/independently verify its
   public fingerprint, and enable `packaging/release/signing-policy.json`.
   Without that policy final publication intentionally fails.

The repository implements the workflow, not account administration. Confirm
the environments, secret names, protection rules and actual key validity before
calling a release production-ready. An API key may permit other App Store
Connect operations according to its role; treat it as a sensitive credential.
Setup verification on 2026-09-05 confirmed the production provisioning profile
and all three secret names in `macos-notarization`, restricted to branch `main`.
Secret values and live notarization authentication have not been tested.
Final-publication environment and branch/tag protection still need verification;
these account settings are not silently applied by this code change.
Decide solo-compatible branch rules before activation;
requiring another person's approval would deadlock the current team.

## Each release

1. Review/test the exact commit on `main`, set the package version, and create
   its protected `v<VERSION>` tag. The existing `prerelease-binaries` workflow
   builds with locked dependencies and leaves a **draft**, never a public
   unsigned release. Branch/test builds remain workflow artifacts.
2. Record the successful build run ID. From the reviewed tooling checkout run:

   ```sh
   python3 scripts/prepare_macos_release.py \
     --tag v<VERSION> --run-id <BUILD_RUN_ID> \
     --identity <40_HEX_CERTIFICATE_FINGERPRINT> \
     --provisioning-profile /absolute/path/DeveloperID.provisionprofile \
     --work-dir /absolute/new/private/release-directory --submit
   ```

   The directory must not exist. The helper checks the official repository,
   workflow, successful run, event and exact tag commit before downloading.
   It parses embedded metadata without executing downloaded binaries, signs
   every Mach-O from inside out with secure timestamps and hardened runtime,
   seals the app and DMG, uploads the signed input to the draft and dispatches
   `notarize-macos`. Omitting `--submit` stops after local signing.
   Keychain access may require your local approval. Do not enable blanket
   access for arbitrary tools or disable library validation to pass a build.
3. CI checks the input hash, Apple signature/team and source/version, submits
   the signed DMG, and waits for **Accepted**. It staples/verifies both DMG
   and app, creates the final ZIP and CLI archive from the same sealed app,
   replaces only draft macOS assets, and regenerates the complete manifest.
   The original CI onefile macOS CLI is a preview only: its embedded libraries
   cannot be post-signed. The final macOS CLI archive includes the same app
   runtime; its terminal launcher does not open the GUI. Linux stays CLI-only.
4. Download the complete final draft into a fresh directory. On a clean Mac,
   verify the manifest hashes and run:

   ```sh
   python3 scripts/macos_release.py verify --release-dir /path/to/final-assets \
     --commit <FULL_TAG_COMMIT> --version <VERSION>
   ```

   Also test a quarantined browser download/drag-to-Applications install,
   first launch, CLI `--version` and an isolated empty-book smoke on a machine
   without the signing key or a prior Gatekeeper exception. Test offline after
   download. Automated `codesign`, `stapler` and `spctl` checks are necessary,
   not a substitute for this installation test; never clear quarantine to
   make a release pass. Notarization is not a guarantee of bug-free software.
5. Only now use the existing offline OpenPGP manifest signing procedure and
   attach its `.asc`. Dispatch `finalize-signed-release` on `main`. It checks
   the exact complete file set, pinned OpenPGP identity, Developer ID signatures,
   tickets, Gatekeeper assessment and matching app contents across all three
   macOS distributions before publication. Homebrew gets the final hashes.

## Failure and retry

Every intermediate state is a draft. A failed notarization, expired identity,
missing ticket, mismatched version/hash or missing OpenPGP signature must never
fall back to unsigned publication. A successful notarization leaves its JSON
response in the workflow's `notarization-evidence` artifact. For timeouts or
rejections, inspect the Apple submission in `notarytool history/log` using
your notary credentials; do not revoke keys or disable checks to retry.

If local signing succeeded but dispatch failed, upload the existing signed
DMG (without clobbering another input), then dispatch `notarize-macos` on main
with the tag and its printed SHA-256. Rerun notarization with the same input
hash if the workflow failed before promotion. If promotion partially failed,
the draft may contain mixed intermediate files: never sign its manifest until
a successful full rerun completes and local verification passes. Once an
`.asc` exists, notarization refuses mutations. A published release is immutable
to these helpers: corrections require a new version/tag, not replacement.
If publication succeeds but the subsequent Homebrew push fails, rerun
`finalize-signed-release` with the same inputs. It re-verifies the complete
published release, skips publication, and retries the tap update without
replacing assets. The tap is deliberately updated only after downloads are
publicly available. Final verification also runs the verified CLI's version
and help commands on the disposable runner; local verification does not execute
the app unless explicitly passed `--smoke`.

## What reproducible means here

The runbook and scripts are repeatable and bind operations to a source commit,
build run/attempt, input hash and final manifest. Source provenance is embedded
inside the signed app as `RELEASE_SOURCE.json`. Locked dependency inputs do
**not** prove a reproducible compiler/toolchain: runner images, Apple secure
timestamps, notary tickets and archive metadata can differ. We do not claim
byte-identical independently rebuilt binaries. Once sealed, the exact approved
artifact bytes are promoted without rebuilding or modifying them.

Before the first real release, test protected Touch ID enrollment/unlock/forget
on actual biometric hardware using a temporary book. Hosted runners cannot
prove a real Touch ID interaction. Static entitlements, profile/certificate
matching, OS distribution policy and native executable launch are automated
gates, not a claim that this hardware test already passed.

References: [Apple custom notarization](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow),
[PyInstaller macOS signing](https://pyinstaller.org/en/stable/feature-notes.html#macos-binary-code-signing),
[GitHub workflow security](https://docs.github.com/en/actions/reference/security/secure-use).
