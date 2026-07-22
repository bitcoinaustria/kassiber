# Linux Packaging Operator TODO

The repository now owns the package builders, channel renderers, CI validation,
and guarded publication workflow. The remaining work needs external accounts,
credentials, infrastructure, or policy decisions and cannot be completed in a
source-only pull request.

## Required before the first channel publish

- [ ] Cut a correctly versioned release. The `vX.Y.Z` tag, Python/Tauri
  metadata, Debian/RPM metadata, and embedded `BUILD_INFO.json` must all be
  `X.Y.Z`. Do not seed a channel from an older mismatched release.
- [ ] Create the protected GitHub environment `linux-packaging-production`,
  require named human approvers, and restrict deployments to the protected
  `main` branch. The workflow separately verifies the requested release tag.
- [ ] Run `publish-linux-channels.yml` once with every `publish_*` input false.
  This verifies the release assets and renders all channel definitions without
  changing an external service.
- [ ] Enable immutable GitHub releases if available to the organization. The
  workflow also binds every mutating job to the checksum manifest produced by
  its `prepare` job; never bypass that same-run artifact boundary.

## Signed APT and DNF repositories

- [ ] Provision project-owned object storage and HTTPS/CDN hosting. Set
  `LINUX_REPOSITORY_BASE_URL`, `LINUX_REPOSITORY_S3_URI`,
  `LINUX_REPOSITORY_S3_ENDPOINT` when required, and
  `LINUX_REPOSITORY_AWS_REGION` in the protected environment.
- [ ] Create an offline certification primary key and a time-bounded archive
  signing subkey. Store an offline backup and revocation certificate
  separately. Give CI only the signing subkey through
  `LINUX_ARCHIVE_GPG_PRIVATE_KEY` and
  `LINUX_ARCHIVE_GPG_PASSPHRASE`.
- [ ] Add least-privilege object-store credentials through
  `LINUX_REPOSITORY_AWS_ACCESS_KEY_ID` and
  `LINUX_REPOSITORY_AWS_SECRET_ACCESS_KEY`. Scope them to the repository
  prefix and deny bucket administration.
- [ ] Publish to `prerelease`, then test clean install, same-surface upgrade,
  CLI/desktop swap, removal, expired metadata, rollback, and key rotation on
  Ubuntu 22.04/24.04, Debian 12, Fedora 43, and Fedora 44.
- [ ] Add availability/expiry monitoring and two named maintainers before
  publishing repository URLs or `apt`/`dnf` commands in user documentation.

## Hosted community channels

- [ ] COPR: create the project and Fedora 43/44 chroots, set `COPR_PROJECT`,
  and store the owner configuration as `COPR_CONFIG`. Review the first binary
  RPMs and COPR signatures before advertising the repository.
- [ ] AUR: register and own `kassiber-bin` and `kassiber-cli-bin`; store a
  dedicated SSH key and pinned host keys as `AUR_SSH_PRIVATE_KEY` and
  `AUR_KNOWN_HOSTS`. Review the generated diffs and first clean-chroot builds.
- [ ] Nix: create the project-owned flake repository, set
  `NIX_CHANNEL_REPOSITORY` and optional `NIX_CHANNEL_BRANCH`, and add the
  least-privilege `NIX_CHANNEL_TOKEN`. Build both outputs before publishing
  install instructions.
- [ ] OBS: create the project plus `kassiber` and `kassiber-cli` packages, set
  `OBS_PROJECT`, and store a dedicated `osc` configuration as `OSC_CONFIG`.
  Start with Tumbleweed and do not claim Leap/SLES until their runtime floors
  pass.

## Still blocked or intentionally deferred

- [ ] Linux ARM64: obtain or reproducibly build pinned Linux aarch64
  `bdkpython` and `lwk` wheels, then add native ARM CI and every release/channel
  artifact together.
- [ ] RHEL 9 and older enterprise Linux: lower and prove the frozen CLI glibc
  floor below the current 2.35 requirement; validate desktop WebKitGTK
  availability separately.
- [ ] Alpine APK: prove the full dependency and desktop stack under musl; do
  not wrap the current glibc build.
- [ ] Snap: choose and validate confinement, local backend access, Secret
  Service, deep links, single-instance behavior, and store ownership.
- [ ] Flathub: do not submit while its current policy excludes AI-assisted
  application code, documentation, or submissions. Re-evaluate with a human
  maintainer if the policy changes.

GitHub-hosted standard runners are expected to fit the current validation
matrix for a public repository. External hosting, domains, signing-key custody,
larger runners, and optional third-party services may incur costs chosen by the
operator.
