# Release Signing

Kassiber follows Sparrow's OpenPGP release-verification shape: one versioned
SHA-256 manifest covers every downloadable artifact, and one detached OpenPGP
signature authenticates that manifest. The signed manifest header also binds
the semantic version because Kassiber's public artifact filenames deliberately
omit it; an old signed manifest cannot be renamed into a newer release.

This is separate from the update notification and from platform code signing.
The notification only links to GitHub. OpenPGP lets a user authenticate bytes
after downloading them; Apple notarization and Windows Authenticode satisfy
their operating systems' execution policies.

It is also separate from the Linux archive key. PR #465's APT/DNF publisher
keeps a time-bounded archive signing key in the protected packaging environment
so it can refresh repository metadata. The general release key stays offline.
Use separate primary keys, not two subkeys under one primary: release
verification intentionally accepts a signing subkey through its certified
primary fingerprint, so sharing a primary would let the CI-held archive subkey
authenticate a forged general release manifest.

## Current transition state

The release workflow now generates:

```text
kassiber-<version>-manifest.txt
```

The CLI can verify that manifest, its signature, and a selected artifact with
`kassiber verify-download`. Kassiber does **not** yet publish a permanent
release public key or fingerprint, so current releases remain unauthenticated.
Do not attach an `.asc` file or describe a release as signed until the key
ceremony and publication steps below are complete.
`packaging/release/signing-policy.json` therefore remains disabled. Publication
workflows may render unsigned channel definitions for a dry run, but they refuse
all external publishing until that code-reviewed policy is enabled.

## One-time release-key ceremony

Create a dedicated OpenPGP certification key with a signing-capable subkey on
an offline machine. Keep the primary secret key offline and maintain encrypted,
tested backups in separate physical locations. The release workflow must never
receive the private key, a private-key export, or its passphrase.

Before using the key for a Kassiber release:

1. Record the full primary-key fingerprint during the offline ceremony.
2. Export only the public key as `kassiber-release.asc`.
3. Publish the public key and full fingerprint in this repository, on the
   Kassiber website, and through at least one independently controlled Bitcoin
   Austria channel.
4. Add the public key at `packaging/release/kassiber-release.asc`, set the full
   primary fingerprint in `packaging/release/signing-policy.json`, and enable
   that policy in a reviewed commit. Create the protected `release-production`
   GitHub environment with named human approvers.
5. Add the public key to packaged Kassiber builds only after reviewers have
   compared those independent publications.
6. Sign and publish a key-transition statement whenever the release key is
   rotated or revoked.

The fingerprint—not a short key ID, email address, or key bundled with a
release—is the root of trust.

## Creating a signed release

The GitHub workflow builds the artifacts and deterministically creates the
manifest. On the offline signing machine, verify the tag and source commit,
download the manifest, and inspect its complete artifact list. Then create the
detached ASCII-armored signature with the repository helper:

```bash
python scripts/release_manifest.py sign \
  --manifest kassiber-0.23.0-manifest.txt \
  --fingerprint '<FULL_PRIMARY_KEY_FINGERPRINT>'
```

The helper selects the key by full fingerprint, signs with SHA-512, refuses to
overwrite an existing signature by default, and verifies the newly produced
signature before returning. It creates:

```text
kassiber-0.23.0-manifest.txt.asc
```

Upload the signature next to the manifest and packages. For production signed
releases, build into a GitHub draft. A second operator runs
`finalize-signed-release.yml`; it downloads the existing assets, verifies the
signature and every manifest entry, rejects missing or unexpected assets,
renders Homebrew hashes from that authenticated manifest, and publishes the
existing draft. It never rebuilds or replaces an artifact. The Linux channel
workflow independently authenticates the same manifest before deriving APT,
DNF, AUR, or Nix inputs.

Both production workflows must be dispatched from protected `main` and reject
release tags whose commits are not in `origin/main` history before executing
code from the tag. Protect creation, update, and deletion of `v*` tags with a
repository ruleset restricted to the release maintainers as well; the ancestry
check is a second boundary, not a substitute for protected tag administration.

## User verification

Until Kassiber bundles the permanent public key, users must obtain the public
key and full fingerprint through independent trusted channels:

```bash
kassiber verify-download kassiber-macos-arm64.dmg \
  --manifest kassiber-0.23.0-manifest.txt \
  --signature kassiber-0.23.0-manifest.txt.asc \
  --public-key kassiber-release.asc \
  --fingerprint '<FULL_PRIMARY_KEY_FINGERPRINT>'
```

Verification is local. Kassiber inspects the supplied public key, confirms its
primary fingerprint, dearmors it into a temporary isolated keyring, and uses
`gpgv` to authenticate the detached signature over a bounded manifest snapshot.
It requires that snapshot's signed version to match its filename and only then
compares the artifact's SHA-256 hash. The comment header remains compatible
with `sha256sum --check`.
A version, signature, fingerprint, or hash mismatch is a hard failure: do not
install or run the file.

This command requires the local `gpg` and `gpgv` executables during the
transition. Verification never imports the key into the user's keyring or
starts a secret-key agent. Once the permanent key is available, packaged
clients can bundle the public key and an in-app verifier without changing the
manifest or signature format.

## Signed-release checklist

- Tag resolves to the reviewed release commit.
- Artifact matrix is complete; no raw sidecars or unexpected filenames exist.
- Manifest was generated after all artifacts and contains each artifact once.
- Detached signature validates against the independently published full
  primary-key fingerprint.
- A second operator independently verifies one artifact with
  `kassiber verify-download`.
- Release notes identify the manifest, signature, and fingerprint locations.
- Draft is published only after all checks pass.
- Homebrew and Linux channel hashes came from the authenticated manifest.
- The offline release primary key is distinct from the CI-held Linux archive
  signing primary key.
- Platform signing/notarization state is stated separately and accurately.
