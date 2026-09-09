# macOS release runbook

Kassiber separates build, protected-CI Developer ID signing and notarization,
and offline OpenPGP release authentication. The Developer ID identity is stored
only as an encrypted GitHub environment secret and imported into an ephemeral
runner keychain. The Apple team is `6Q4R2C3GJK` (Bitcoin Austria); changing
that trust root requires a reviewed code change in `scripts/macos_release.py`.

## One-time operator setup

1. Export the dedicated Developer ID Application identity once as password-
   protected PKCS#12. Store its base64 as `MACOS_CERTIFICATE_P12_BASE64`, its
   export password as `MACOS_CERTIFICATE_PASSWORD`, and the base64 Developer ID
   provisioning profile as `MACOS_PROVISIONING_PROFILE_BASE64` in the protected
   GitHub environment `macos-notarization`. Never commit these values.
2. Keep an independently recoverable copy of the Developer ID identity. CI
   discovers its exact SHA-1 fingerprint after import and requires exactly one
   Developer ID Application identity for Apple team `6Q4R2C3GJK`.
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
   allow only the `main` branch and manually dispatch finalization after CI
   signing/notarization and independent local verification. Do not require a second reviewer or enable
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
Setup verification on 2026-09-07 confirmed the production provisioning profile,
the notarization secret names, and `macos-notarization` restricted to branch
`main`. The three CI-signing secrets must be installed and exercised once before
calling the automated path production-ready.
Final-publication environment and branch/tag protection still need verification;
these account settings are not silently applied by this code change.
Decide solo-compatible branch rules before activation;
requiring another person's approval would deadlock the current team.

## Each release

Before tagging, write and review the new `## <VERSION>` section in
`CHANGELOG.md`, using concise Added/Fixed bullets with PR links (and Breaking
Changes when necessary). Derive it from first-parent history since the previous
release; include each merged PR once. Commit it with the version bump. The
publish job reads the changelog from the tagged source and copies that section
unchanged into the GitHub draft. Missing, duplicate or empty sections stop
publication. Keep transient CI/notarization status out of this historical text.
Preview locally with `python3 scripts/release_notes.py --version v<VERSION>
--output /private/tmp/kassiber-release-notes.md`. No personal skill is required.

1. Review/test the exact commit on `main`, set the package version, and create
   its protected `v<VERSION>` tag. The existing `prerelease-binaries` workflow
   builds with locked dependencies and leaves a **draft**, never a public
   unsigned release. Branch/test builds remain workflow artifacts.
2. Record the successful build run ID and dispatch the protected workflow:

   ```sh
   gh workflow run notarize-macos.yml --repo bitcoinaustria/kassiber --ref main \
     -f tag_name=v<VERSION> -f build_run_id=<BUILD_RUN_ID>
   ```

   CI uses a new private temporary directory. The helper checks the official repository,
   workflow, successful run, event and exact tag commit before downloading.
   It parses embedded metadata without executing downloaded binaries, prepares
   and checks Mach-O linkage that Homebrew can preserve without rewriting,
   and signs every Mach-O from inside out with secure timestamps and hardened runtime,
   then seals the app and DMG before notarization in the same protected job.
   Before signing it rechecks that the release is an unsigned draft and that
   the successful official build names the exact tag commit. Promotion repeats
   the draft/tag checks before replacing assets. These checks do not replace
   tag protection; a failed job leaves the draft unpublished for inspection.
   The PKCS#12, password and profile are unavailable to pull-request jobs and
   removed with the temporary keychain even when the job fails.
3. CI retains the exact signed input DMG in the draft before submitting it.
   It checks the input hash, Apple signature/team and source/version, submits
   once, preserves the Apple submission ID and a SHA-256/source-bound receipt,
   and polls for up to two minutes. A pending run ends successfully with an
   explicit pending summary; it does not verify, promote or publish artifacts.
   Resume the same submission as described below. After **Accepted**, CI staples/verifies both DMG
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
   Also exercise both Homebrew installation routes with the candidate assets
   and verify the installed app's Developer ID seal after Homebrew finishes.
   Archive verification alone cannot prove that an installer preserved it.
5. Only now use the existing offline OpenPGP manifest signing procedure and
   attach its `.asc`. Dispatch `finalize-signed-release` on `main`. It checks
   the exact complete file set, pinned OpenPGP identity, Developer ID signatures,
   tickets, Gatekeeper assessment and matching app contents across all three
   macOS distributions before publication. Homebrew gets the final hashes.
   The macOS formula keeps the app intact under `libexec`, disabling Python
   metadata cleanup for that subtree and preserving `@rpath` library IDs.
   The sealed-artifact checks reject linkage that would still require rewriting.

## Failure and retry

### Resume an Apple submission

Download `notarization-evidence` from the previous run. `submission.json` records
the submission ID, exact signed input SHA-256, source commit and version.
The signed `kassiber-macos-signing-input.dmg` stays in the draft until promotion.
For the older evidence format, take the ID from `notarization.json` and the
input SHA-256 from that run's dispatch inputs. Use the same draft/tag:

```sh
gh workflow run notarize-macos.yml --repo bitcoinaustria/kassiber --ref main \
  -f tag_name=v<VERSION> -f input_sha256=<EXACT_SIGNED_INPUT_SHA256> \
  -f submission_id=<APPLE_SUBMISSION_UUID>
```

Omit `build_run_id`: resuming must not rebuild, re-sign or upload a new Apple
submission. Candidate resumes also retain their `candidate=true` and
`source_commit` inputs. Resume reads retry three times on tool errors; a failed
submit is never blindly retried because Apple may already have received it.
Apple's `Accepted` status alone is insufficient: stapling, ticket validation,
code signatures and source/version checks must succeed for these exact bytes.
Rejections preserve `notarization-log.json` when Apple makes it available.
Pending runs require a later dispatch; the workflow does not schedule itself.

### Unpublished candidates without a version tag

An operator may notarize an exact main-history build for private testing without
creating a Git tag or publishing a prerelease. This draft is only an Apple
notarization handoff for commits already merged into main. PR and unmerged branch
builds remain workflow artifacts and cannot use this path. Use an official successful
`prerelease-binaries.yml` build-only run (`publish_release=false`), verify its
repository, workflow, source commit and run attempt, and download that run's
`kassiber-desktop-macos-arm64-preview` artifact. Read the version from the pinned
commit's `pyproject.toml`; the signing helper also checks the embedded build
commit/version and app bundle version.

Create an **unpublished draft** whose identifier is
`macos-candidate-<FULL_COMMIT>-<BUILD_RUN_ID>` and whose `target_commitish` is that
full commit. Do not create a matching Git tag. Candidate signing uses the existing
`macos_release.py sign` arguments plus `--candidate-id <IDENTIFIER>` and this
explicit `source.json` shape (all placeholders must be replaced):

```json
{
  "kind": "candidate",
  "candidate_id": "macos-candidate-<FULL_COMMIT>-<BUILD_RUN_ID>",
  "commit": "<FULL_COMMIT>",
  "version": "<SOURCE_VERSION>",
  "build_run": "<BUILD_RUN_ID>",
  "build_attempt": 1,
  "unsigned_app_sha256": "<DOWNLOADED_ZIP_SHA256>"
}
```

Upload only the locally signed `kassiber-macos-signing-input.dmg` to that draft.
Dispatch `notarize-macos.yml` from protected `main` with `tag_name=<IDENTIFIER>`,
`candidate=true`, `source_commit=<FULL_COMMIT>` and `input_sha256=<SIGNED_DMG_SHA256>`.
The workflow rechecks the draft target and absence of a Git tag before uploading
the verified results. It leaves the three notarized macOS distributions in the
draft; it does not generate a version-release manifest, publish, run a finalizer,
or update package channels. Download and verify them with `macos_release.py verify`
using the same `--candidate-id`, `--commit` and `--version` (plus `--smoke` only on
a disposable verification Mac). Ordinary release verification rejects candidate
provenance. The tag-based `prepare_macos_release.py` helper is for releases and
does not prepare these candidates.

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
