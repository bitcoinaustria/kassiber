# Linux Packaging Roadmap

Kassiber does not currently publish an APT, RPM, AUR, Nix, Snap, Flatpak, or
APK channel. The Linux release assets are direct-download x86_64 Debian and
AppImage files plus a CLI archive and CLI-only Debian package. Do not document
`apt upgrade`, `dnf upgrade`, or another repository upgrade command until the
corresponding repository is live and its candidate can be verified.

This document records the packaging boundary and rollout order. `TODO.md`
remains the execution backlog.

## Release gates

Every package-manager channel must fail closed unless all of these checks pass:

1. The release tag without its leading `v`, the Python package version, the
   desktop package version, every native package version, and the embedded
   `BUILD_INFO.json` version are identical.
2. Every artifact is built from the tag commit, has a published SHA-256, and
   has build provenance. Repository metadata is generated only after the
   complete artifact matrix succeeds.
3. CLI-only package `kassiber-cli` and desktop package `kassiber` conflict and
   replace one another because both own `/usr/bin/kassiber`. Both provide the
   virtual `kassiber-command`. The desktop package additionally owns
   `/usr/bin/kassiber-ui`, the desktop file, icons, and its bundled sidecar.
4. The package's architecture and runtime floor were exercised on every
   distribution named as supported. A package merely building is not evidence
   that it runs on an older glibc or a different libc.
5. Repository payloads and content-addressed metadata are uploaded before the
   signed top-level metadata. APT switches through one `InRelease` object. DNF
   publishes a complete immutable, suite-scoped snapshot and then switches one
   mirrorlist object, so clients never observe a mixed metadata/signature pair.

The release workflow enforces the first gate for tag and publishing runs.

## Package-manager upgrade guidance is deliberately absent

The update checker treats Linux `.deb`/`.rpm` installs as manual: it shows the
GitHub release link and never an `apt`/`dnf` command. Package contents cannot
prove how a package was obtained — a directly downloaded GitHub `.deb` and the
byte-identical artifact later indexed by a repository are the same bytes — and
the local package database has only `/var/lib/dpkg/status`-level provenance.
Before the updater may show a package-manager command, a future change must:

1. query the configured repository manager for the exact Kassiber package;
2. verify the candidate's signed repository origin/label against a built-in
   Kassiber allowlist (live repository URL plus pinned archive-key
   fingerprint, neither of which exists yet);
3. verify that the advertised version is actually present in that repository;
4. show, but never execute, the manager command only after those checks pass.

That detection machinery ships with the repository-pinning feature itself, not
before.

## APT foundation

`scripts/build-apt-repository.sh` turns completed `kassiber` and
`kassiber-cli` Debian packages into a new repository tree. It:

- rejects unknown packages, undeclared architectures, duplicate
  package/version/architecture tuples, and pre-existing output paths;
- emits `Packages`, deterministic `Packages.gz`, SHA-256 by-hash indices, and
  Release metadata with `Valid-Until`;
- requires a full signing-key fingerprint unless `--unsigned` is explicitly
  selected for a local test;
- emits both `InRelease` and `Release.gpg` when signing;
- supports `NotAutomatic: yes` plus `ButAutomaticUpgrades: yes` for an opt-in
  prerelease suite.

An unsigned local dry run is:

```bash
./scripts/build-apt-repository.sh \
  --input /path/to/release-debs \
  --output /tmp/kassiber-apt \
  --suite prerelease \
  --architecture amd64 \
  --not-automatic \
  --but-automatic-upgrades \
  --unsigned
```

A release operator supplies an already-unlocked GnuPG home and the full
fingerprint instead of `--unsigned`. The script never imports or exports a
private key. `--release-epoch` exists for deterministic tests; production must
use the default current publication time so `Date` and `Valid-Until` cannot be
stale after rebuilding an older tag.

No public repository URL or install command belongs in user documentation
until the signed tree, public key, custom domain, retention policy, and
operational owner exist.

## RPM, AUR, and Nix foundation

The release workflow emits binary RPMs for both package surfaces. The specs
preserve the same `kassiber`/`kassiber-cli` ownership boundary as the Debian
packages, declare the current glibc floor, and carry conditional
Fedora/openSUSE runtime package names.

`scripts/build-rpm-repository.sh` rejects unknown packages, undeclared
architectures, unsigned production use, and pre-existing
outputs. It signs every binary RPM and `repodata/repomd.xml`, and gives payloads
canonical versioned NEVRA filenames. The S3 publisher writes complete DNF
snapshots below the selected `stable` or `prerelease` suite before changing its
mirrorlist pointer. Fedora 43 and 44 CI builds, signs, installs, swaps CLI to
desktop, and removes the packages with DNF in disposable containers.

`scripts/render_aur.py` generates separate project-owned `kassiber-bin` and
`kassiber-cli-bin` repositories with pinned release URLs, SHA-256 checksums,
conflicts/provides metadata, license files, and launchers. CI compares
the rendered `.SRCINFO` to `makepkg --printsrcinfo` and runs `namcap`. Before
publication credentials are configured, the guarded release job also builds,
installs, smokes, and removes both recipes against their real release assets in
a clean pinned Arch container.

`scripts/render_nix.py` generates an x86_64-only binary flake with explicit
`binaryNativeCode` provenance, pinned release hashes, a desktop AppImage
wrapper, and an auto-patched GUI-free CLI derivation. The external
channel workflow locks, checks, builds both outputs, and smokes the CLI in a
credential-free copy before it pushes its project-owned repository.

`.github/workflows/publish-linux-channels.yml` is manual,
defaults every external publish switch to false, and puts every mutating job
behind `linux-packaging-production`. During the current key-transition state it
may use the versioned manifest only to render a no-publication dry run. Once the
code-reviewed release-signing policy is enabled, it authenticates the detached
manifest signature before deriving any APT/DNF, AUR, or Nix input;
external publication fails closed without that signature.

COPR and OBS submission (and the source-RPM packaging they need) were
deliberately removed from this foundation; they return in the change that
actually provisions those projects.

The exact external setup and launch checklist is
[Linux Packaging Operator TODO](linux-packaging-operator-todo.md).

### APT ownership and key management

The recommended production shape is:

- **Owner:** Bitcoin Austria, with two named maintainers able to revoke a
  release and rotate access.
- **Inputs:** immutable `.deb` artifacts from one successful tagged workflow;
  never rebuild one package independently under an existing version.
- **Host:** object storage/CDN behind a project-owned HTTPS name such as
  `packages.bitcoinaustria.at`. GitHub Pages is a poor long-term binary host
  because its documented repository and bandwidth limits are small relative
  to Kassiber's roughly 100 MB packages.
- **Keys:** a dedicated offline archive certification primary key and a
  time-bounded archive signing subkey. This primary is distinct from Kassiber's
  general offline release-signing primary. CI receives only the archive signing
  subkey through a protected
  release environment. Store an encrypted offline backup and a pre-generated
  revocation certificate separately. Pin its full primary fingerprint in the
  protected `LINUX_ARCHIVE_GPG_FINGERPRINT` variable; the publisher requires
  exactly one imported primary identity and refuses any mismatch. Publish the
  minimal public key from the project-owned domain and its fingerprint through
  an independently controlled Bitcoin Austria channel, and configure it with
  APT `Signed-By`, never `apt-key`.
- **Cadence:** publish only completed tagged prereleases to `prerelease`; add a
  `stable` suite only when Kassiber starts making stable releases. Regenerate
  metadata before `Valid-Until` even during a release pause.
- **Rollback/revocation:** retain versioned pool artifacts, atomically publish
  new metadata that removes a bad candidate, and publish a fixed version with a
  strictly greater Debian version. APT does not automatically downgrade an
  already installed bad version. Key compromise requires a new key and an
  out-of-band migration notice; a signature from the compromised key cannot be
  trusted as its own revocation.

## Channel phases and acceptance criteria

| Phase/channel | Package inputs and ownership | Acceptance criteria | Typical maintenance |
| --- | --- | --- | --- |
| 0: release/channel foundations | Project-owned workflows, Debian/RPM packages, AUR/Nix renderers | Tag/version equality blocks publish; signed local APT/DNF repositories pass verification; AUR metadata and Nix evaluation pass | Review whenever release/package metadata changes |
| 1: signed APT prerelease, amd64 | Existing desktop and CLI Debian packages; Bitcoin Austria owns host/key | Ubuntu 22.04/24.04 and Debian 12 clean install, cross-surface replacement, upgrade, remove, signed-origin, expiry, and rollback drills pass; repository is atomic and monitored | About 1-2 hours/month after automation, plus key/host incident duty |
| 2: lower CLI glibc floor + Linux ARM64 | Native x86_64/aarch64 CLI builds; upstream or project-built BDK/LWK wheels | CLI is built on the oldest promised glibc; `readelf` and container smokes prove the floor; native ARM runner builds and runs BDK, LWK, SQLCipher, PyInstaller, Debian, and desktop/AppImage matrices | Dependency-wheel monitoring every release |
| 3: AUR `-bin` recipes | Project-owned `kassiber-bin` and `kassiber-cli-bin` recipes consume immutable release assets | Renderer/metadata CI is green; then `namcap`, clean-chroot build, checksum, file ownership, conflicts/provides, desktop metadata, install/upgrade/remove pass on x86_64; ARM64 is added only with real assets | Review each release and respond to AUR comments; automation cannot replace maintainer review |
| 4: project flake, then Nixpkgs | Interim binary derivations declare `binaryNativeCode`; prefer source derivations later | Rendered flake evaluates in CI; then `nix build`, CLI/desktop smoke and sandbox paths pass on x86_64. Add aarch64 only with real release assets; maintain the upstream flake before Nixpkgs submission | Flake input/hash bumps per release; Nixpkgs review and stable backports |
| 5: RPM + COPR | Reintroduce rebuildable `kassiber` and `kassiber-cli` SRPMs with the COPR submission itself; COPR owns hosted repository signing | Fedora 43/44 lifecycle CI is green; then COPR `rpmlint`/mock builds and published signatures pass for each enabled chroot; no generic RHEL claim | Follow Fedora releases/EOL; rebuild and test each tag, roughly 1-2 hours/month |
| 6: OBS for openSUSE | Reuse source packaging where practical; a named Bitcoin Austria/openSUSE maintainer owns the OBS project | Tumbleweed first; Leap/SLES only after their glibc and WebKitGTK floors pass; OBS-signed repositories and zypper lifecycle tests pass per target | Track OBS target changes and openSUSE/SLES lifecycle separately from COPR |
| Deferred: Snap | Source build or staged desktop package; project owns Snap name/store credentials | Strict confinement must support data roots, attachments, local backends, Secret Service, deep links, and single-instance D-Bus. Classic confinement requires store approval and explicit user trust | High: store credentials, tracks, confinement regressions |
| Blocked: Flathub | No submission while current policy excludes AI-assisted application code/docs/submissions | Re-check the live policy; proceed only after written eligibility is clear and a human can maintain the submission without violating it | Policy and sandbox review every update |
| Deferred: Alpine APK | Native musl source build, not a glibc compatibility shim | BDK/LWK/SQLCipher/PyInstaller and desktop WebKitGTK all build and run under musl on x86_64 and aarch64; `abuild` and repository signing pass | High until upstream musl artifacts exist |

The ordering is intentionally not "all RPM systems at once." The current
Ubuntu 22.04 artifact floor cannot represent older enterprise distributions,
and the desktop's WebKitGTK package availability differs by target. AppImage
reduces dependency packaging but does not erase its build host's glibc floor.

## ARM64 CI requirements

Linux ARM64 is a release matrix, not a filename change:

1. Obtain pinned `bdkpython` and `lwk` Linux aarch64 wheels or reproducibly
   build and attest them from their pinned sources. The current lock has Linux
   aarch64 SQLCipher and pyrage wheels, but the pinned BDK and LWK releases are
   x86_64-only on Linux.
2. Use GitHub's native Ubuntu 22.04 ARM runner for both the PyInstaller CLI and
   Tauri desktop build. Tauri's AppImage tooling does not cross-compile ARM;
   native ARM or emulation is required.
3. Add `linux-arm64`, `aarch64-unknown-linux-gnu`, Debian `arm64`, and matching
   sidecar artifact entries. Extend checksums, release-set validation, and every
   channel renderer rather than publishing a partial architecture.
4. Run the same packaged CLI/database/observer smokes as x86_64, inspect ELF
   architecture and GLIBC requirements, then test Debian/AppImage execution on
   an ARM64 Ubuntu 22.04 baseline.
5. Do not publish a reduced ARM build that silently omits BDK or LWK. Kassiber's
   existing packaged-platform rule requires both observers.

## Primary policy references

- [APT archive authentication](https://manpages.debian.org/testing/apt/apt-secure.8.en.html)
  and [Debian repository format](https://wiki.debian.org/DebianRepository/Format)
- [Fedora packaging guidelines](https://docs.fedoraproject.org/en-US/packaging-guidelines/)
  and [COPR documentation](https://docs.pagure.org/copr.copr/)
- [Open Build Service user guide](https://openbuildservice.org/help/manuals/obs-user-guide/)
- [AUR submission guidelines](https://wiki.archlinux.org/title/AUR_submission_guidelines)
- [Nixpkgs manual](https://nixos.org/manual/nixpkgs/stable/)
- [Snap confinement](https://snapcraft.io/docs/explanation/security/snap-confinement/)
- [Flathub requirements](https://docs.flathub.org/docs/for-app-authors/requirements)
- [Tauri Linux distribution](https://v2.tauri.app/distribute/) and
  [AppImage baseline/ARM notes](https://v2.tauri.app/distribute/appimage/)
