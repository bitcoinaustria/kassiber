# Release Signing

For local Apple signing and automated notarization, follow the
[macOS release runbook](macos-release.md). Build workflows stage drafts;
final publication requires both macOS validation and the SSH signature policy
below.

Kassiber follows Sparrow's release-verification shape: one versioned SHA-256
manifest covers every downloadable artifact, and one detached signature
authenticates that manifest. The signature is an OpenSSH signature
(`ssh-keygen -Y sign`) in the `kassiber-release` namespace. The signed manifest
header also binds the semantic version because Kassiber's public artifact
filenames deliberately omit it, so an old signed manifest cannot be renamed
into a newer release.

This is separate from the update notification and from platform code signing.
The notification only links to GitHub. The manifest signature lets a user
authenticate bytes after downloading them; Apple notarization and Windows
Authenticode satisfy their operating systems' execution policies.

It is also separate from the Linux archive key. PR #465's APT/DNF publisher
keeps a time-bounded OpenPGP archive signing key in the protected packaging
environment because package managers require OpenPGP there. That key never
authenticates a release manifest: the verifier accepts only the pinned SSH key.
The channel workflow also refuses an archive key whose primary key or subkey
is the same key material as the release key.

## Release key

| | |
| --- | --- |
| Algorithm | Ed25519 (`ssh-ed25519`) |
| Fingerprint | `SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M` |
| Public key | [`packaging/release/kassiber-release.pub`](../../packaging/release/kassiber-release.pub) |
| Allowed signers | [`packaging/release/allowed_signers`](../../packaging/release/allowed_signers) |
| Principal | `release@kassiber` |
| Namespace | `kassiber-release` |

```text
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILAm3bbSbPI3QCjLFDrMPP1vGhqt3O3BieWJhTGToVBy kassiber-release
```

The full SHA256 fingerprint is the root of trust. A key comment, email
address, short fingerprint, or key file shipped with a download is not.
Compare the fingerprint in this repository with the Kassiber website and at
least one independently controlled Bitcoin Austria channel before trusting it.

[`packaging/release/signing-policy.json`](../../packaging/release/signing-policy.json)
is the code-reviewed policy that publication workflows load. It pins the
fingerprint, principal, namespace, public key, and allowed-signers file. The
loader rejects the policy unless the public key and the single allowed-signers
entry both hash to the pinned fingerprint. That entry may carry only
`namespaces="kassiber-release"`, with no other options.

## Key custody

The release key is a dedicated Ed25519 key held in the owner's Bitwarden vault
and used only through the Bitwarden SSH agent, which asks for approval on
every signature. The private key never exists as a file on disk. Never export
it, and never give it to CI, a workflow secret, or another agent. Release
workflows receive only the public key.

### Accepted residual risk (owner decision, 2026-09-30)

The owner accepted this custody model on 2026-09-30 instead of an offline
OpenPGP primary key. The key is not offline. Anyone who controls the unlocked
vault account, or the unlocked workstation while the agent is running, can
sign a manifest that users and the finalizer will accept as a genuine release.
The mitigations are:

- per-use approval in the Bitwarden SSH agent;
- a short vault timeout;
- a hardware second factor on the vault account.

The draft-only publication path, exact asset-set checks, and Developer ID
checks still limit what a forged manifest can publish through Kassiber's own
workflows. They do not help users who verify a forged manifest obtained
elsewhere. Moving to a hardware-backed key (below) removes the dependence on
vault and workstation integrity.

## Creating a signed release

The GitHub workflow builds the artifacts and deterministically creates the
manifest. On the signing workstation, verify the tag and source commit,
download the manifest, and inspect its complete artifact list. Then point the
shell at the Bitwarden agent, confirm the agent offers the release key, and
run the repository helper:

```bash
export SSH_AUTH_SOCK="$HOME/.bitwarden-ssh-agent.sock"
ssh-add -l -E sha256   # must list SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M
uv run --locked python scripts/release_manifest.py sign \
  --manifest kassiber-0.23.0-manifest.txt \
  --public-key ~/.ssh/kassiber-release.pub
```

`--public-key` is a copy of the committed public key; the matching private key
must be in the agent only. Keep no private key file named
`~/.ssh/kassiber-release`, because `ssh-keygen` falls back to the private key
file next to a `.pub` when the agent does not hold that key.

The helper loads the enabled policy and refuses a public key whose fingerprint
differs from it. It refuses to overwrite an existing signature unless you pass
`--overwrite`. It passes the manifest to `ssh-keygen -Y sign -n
kassiber-release` on stdin, verifies the new signature against the pinned
fingerprint, and only then writes:

```text
kassiber-0.23.0-manifest.txt.sig
```

Approve the request in Bitwarden only while this command is running. Upload the
signature next to the manifest and packages. Do not attach the public key or
an allowed-signers file to the release; users must get them independently.

For production releases, build into a GitHub draft. The release operator (or,
when available, an independent second operator) runs
`finalize-signed-release.yml`. It downloads the existing assets, verifies the
signature and every manifest entry, rejects missing or unexpected assets,
renders Homebrew hashes from that authenticated manifest, and publishes the
existing draft. It never rebuilds or replaces an artifact. The Linux channel
workflow independently authenticates the same manifest before deriving APT,
DNF, AUR, or Nix inputs. Both workflows require OpenSSH 8.1 or newer on the
runner.

Both production workflows must be dispatched from protected `main`. Before
executing code from the tag, they reject release tags whose commits are not in
`origin/main` history. Also protect creation, update, and deletion of `v*` tags
with a repository ruleset restricted to the release maintainers. The ancestry
check is a second boundary, not a substitute for protected tag administration.

## User verification

Obtain `kassiber-release.pub` from this repository or the Kassiber website, and
compare its fingerprint with a second independent channel. Verification needs
OpenSSH 8.1 or newer (`ssh -V`).

With Kassiber installed:

```bash
kassiber verify-download kassiber-macos-arm64.dmg \
  --manifest kassiber-0.23.0-manifest.txt \
  --signature kassiber-0.23.0-manifest.txt.sig \
  --public-key kassiber-release.pub \
  --fingerprint 'SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M'
```

Verification is local and makes no network request. Kassiber reads bounded
copies of the manifest, signature, and key, then hashes the key in Python and
requires the pinned fingerprint. It writes one canonical allowed-signers entry
for that key into a private temporary directory and runs `ssh-keygen -Y verify`
with principal `release@kassiber`, namespace `kassiber-release`, and the
manifest snapshot on stdin, without a shell. It accepts the signature only if
`ssh-keygen` succeeds and reports a good signature from that exact key. It then
requires the signed version to match the manifest filename, and only then
compares the artifact's SHA-256 hash. A missing or too-old `ssh-keygen`, or a
version, signature, fingerprint, or hash mismatch, is a hard failure: do not
install or run the file.

Without Kassiber, on macOS or Linux:

```bash
ssh-keygen -lf kassiber-release.pub
# must print SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M
printf 'release@kassiber namespaces="kassiber-release" %s\n' \
  "$(cut -d' ' -f1,2 kassiber-release.pub)" > allowed_signers
ssh-keygen -Y verify -f allowed_signers -I release@kassiber \
  -n kassiber-release -s kassiber-0.23.0-manifest.txt.sig \
  < kassiber-0.23.0-manifest.txt
# must print: Good "kassiber-release" signature for release@kassiber with ED25519 key SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M
sha256sum --check --ignore-missing kassiber-0.23.0-manifest.txt          # Linux
shasum -a 256 --check --ignore-missing kassiber-0.23.0-manifest.txt      # macOS
```

On Windows, use the OpenSSH client (Windows Settings → Optional features, or a
current Win32-OpenSSH release if `ssh -V` reports older than 8.1). Run the
signature check through `cmd` so the manifest bytes reach `ssh-keygen`
unchanged. PowerShell pipelines can re-encode text.

```powershell
ssh-keygen -lf kassiber-release.pub
$key = (Get-Content -Raw kassiber-release.pub).Trim().Split(' ')[0..1] -join ' '
[IO.File]::WriteAllText("$PWD\allowed_signers", "release@kassiber namespaces=`"kassiber-release`" $key`n")
cmd /c "ssh-keygen -Y verify -f allowed_signers -I release@kassiber -n kassiber-release -s kassiber-0.23.0-manifest.txt.sig < kassiber-0.23.0-manifest.txt"
Get-FileHash -Algorithm SHA256 kassiber-windows-x64.msi   # compare with the manifest line
```

Packaged builds do not embed the release key yet. `verify-download` always
requires an explicit key file and fingerprint. The key may be embedded, with an
in-app verifier, only after reviewers have compared the independent
publications of the fingerprint. That change keeps the manifest and signature
format.

## Key rotation and revocation

Rotate with a signed transition statement. Write a short text statement naming
the old and new fingerprints, the effective date, and the first release the new
key will sign. Sign it with the old key in the `kassiber-key-transition`
namespace (and, when possible, with the new key as well):

```bash
ssh-keygen -Y sign -n kassiber-key-transition -f ~/.ssh/kassiber-release.pub transition.txt
```

Publish the statement and signatures through the same three channels as the
fingerprint. Then replace `kassiber-release.pub`, `allowed_signers`, and the
policy fingerprint in one reviewed commit, and update the key table above.
Keep the previous key and fingerprint listed with their validity period so
users can still verify older releases.

If the key or vault is compromised, a statement signed by that key proves
nothing. Publish a revocation notice through all independent channels, stop
finalization, rotate to a new key, and name every release that could be
affected.

### Upgrading to a hardware key

The verifier already accepts `sk-ssh-ed25519@openssh.com` keys. Moving to a
FIDO2 `ed25519-sk` key (`ssh-keygen -t ed25519-sk`) is a normal rotation: new
public key, allowed-signers entry, and policy fingerprint, with no code or
signature-format change. Verifying security-key signatures requires OpenSSH
8.2 or newer.

## Signed-release checklist

- Tag resolves to the reviewed release commit.
- Artifact matrix is complete; no raw sidecars or unexpected filenames exist.
- Manifest was generated after all artifacts and contains each artifact once.
- The `.sig` validates against the independently published SHA256 fingerprint.
- The operator verifies the final artifacts with `kassiber verify-download`;
  use an independent second operator when one is available. Do not describe
  single-maintainer verification as independent human review.
- Release notes identify the manifest, signature, and fingerprint locations.
- Draft is published only after all checks pass.
- Homebrew and Linux channel hashes came from the authenticated manifest.
- The SSH release key and the CI-held OpenPGP Linux archive key are distinct.
- Platform signing/notarization state is stated separately and accurately.
