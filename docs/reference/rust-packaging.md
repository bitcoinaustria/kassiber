# Rust Packaging

[Plan 21](../plan/21-rust-backend.md) stage 7 replaces the PyInstaller sidecar
with the Rust `kassiber` binary in every release artifact. This reference is
the inventory of those artifacts, what changes for each, how the Linux set was
verified, and what still needs a native macOS or Windows run. The release flow
itself (drafts, manifest signing, notarization, channels) is unchanged and
lives in [prerelease binaries](prerelease-binaries.md),
[release signing](release-signing.md), [macOS release](macos-release.md) and
[Linux packaging](linux-packaging.md).

## One binary everywhere

Every leg of `.github/workflows/prerelease-binaries.yml` runs
`scripts/build-rust-cli.sh --target <triple>`: `cargo build --release --locked
-p kassiber --bin kassiber` without the `test-harness` feature (no fake store,
test kinds, probe mode or egress observer). The script then

- refuses a crate version that differs from `pyproject.toml`, and a
  `--version` other than `Kassiber <version> (<channel>, commit <12 hex>)`;
- strips symbols (Linux and Windows through Cargo; macOS with Xcode's `strip`,
  see below);
- remaps the checkout and Cargo home out of panic paths and pins
  `SOURCE_DATE_EPOCH` (vendored OpenSSL otherwise embeds its build time);
- refuses a Linux binary that needs a glibc newer than the 2.35 floor or links
  anything besides glibc and `libgcc_s` (`scripts/check-linux-glibc-floor.sh`),
  and a macOS binary that links any non-system dylib.

The workflow smokes that exact file (`scripts/smoke-packaged-cli.sh`), proves
on Linux that it opens no network socket (`tests/test_packaged_cli_no_egress.py`),
packages it as the CLI artifact and stages the same bytes as the desktop
sidecar. There is no separate sidecar build and no one-dir payload.

The production binary serves the assistant's chat (`ai.chat`, its controls
and history), the `chat`/`chats` leaves and the tool-running `mcp serve`/`mcp
call` leaves; the `native-chat` and `native-mcp` gates are gone
([native chat](rust-native-chat.md)). Desktop kinds the Rust backend does not
serve yet answer `not_implemented`.

## Files beside the executable

Everything else the backend needs is compiled in with `include_str!` /
`include_bytes!`: the book schema and migrations, the Typst report template and
its fonts (`crates/pdf/fonts`: DM Sans, JetBrains Mono, Liberation Sans), the
bundled Kraken BTC/EUR and BTC/USD hourly and daily rates
(`crates/rates/data/kraken`, about 15 MB of the binary), the AI skill
references (`crates/ai/skill_references`), the pairing-service catalog, the
OCR prompt, and the WebPKI roots used when the host has no CA bundle. Three
things are not compiled in:

| File | Where | Why |
| --- | --- | --- |
| `provider_broker/index.mjs` | `<executable dir>/provider_broker/` | `kassiber_ai::broker::bundled_script()` resolves the real executable path and looks there. Without it, CLI-backed AI providers report the broker as unavailable. It still needs Node.js 20+ on the host, as before. |
| `BUILD_INFO.json` | beside the executable in archives and sidecar directories | Readable build provenance; `scripts/macos_release.py` refuses to sign an app without one. The binary itself carries channel and commit from compile time. |
| Host CA bundle | the system store | `rustls-native-certs`, with `SSL_CERT_FILE` authoritative and WebPKI roots as the fallback, matching the frozen Python bootstrap ([Linux packaging](linux-packaging.md#frozen-tls-trust)). |

## Artifact inventory

Sizes are bytes. "Python" is the published v0.22.78 asset. "Rust" is the local
Linux build below (host glibc 2.44, so functional evidence only; the release
floor build runs on Ubuntu 22.04 in CI).

| Artifact | Python sidecar (today) | Rust | Python size | Rust size |
| --- | --- | --- | ---: | ---: |
| `kassiber-cli-linux-x64.tar.gz` | one-file PyInstaller `kassiber` + `BUILD_INFO.json` | `kassiber` + `BUILD_INFO.json` + `provider_broker/index.mjs` | 89,237,493 | 49,102,562 |
| `kassiber-cli-linux-x64.deb` | one-file binary at `/usr/bin/kassiber`; `Depends: libc6 (>= 2.35), zlib1g` | `/usr/lib/kassiber-cli/{kassiber,provider_broker/index.mjs}`, `/usr/bin/kassiber` symlink; `Depends: libc6 (>= 2.35), libgcc-s1` | 86,678,852 | needs dpkg-deb |
| `kassiber-cli-linux-x64.rpm` | same as the deb; `Requires: zlib` | same layout as the deb; zlib dropped (rpmbuild derives `libgcc_s`) | 89,263,758 | needs rpmbuild |
| `kassiber-linux-x64.deb` | Tauri deb; sidecar `/usr/lib/Kassiber/binaries/kassiber-cli-x86_64-unknown-linux-gnu` (one-file) | same path, Rust binary, plus `binaries/provider_broker/` and `binaries/BUILD_INFO.json` | 96,342,954 | 57,763,444 |
| `kassiber-linux-x64.rpm` | repacked from the desktop deb | unchanged repack | 96,084,918 | needs rpmbuild |
| `kassiber-linux-x64.AppImage` | Tauri AppImage, same sidecar, bundled GTK/WebKit, host Wayland client | same, Rust sidecar | 174,234,104 | needs linuxdeploy |
| `kassiber-cli-macos-arm64.tar.gz` | final: the notarized app + `kassiber` launcher symlink | unchanged shape; the app carries the Rust sidecar | 79,452,271 | needs native run |
| `kassiber-macos-arm64.app.zip`, `.dmg` | one-dir sidecar `binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin` + `_internal/` | same path, single Rust Mach-O, no `_internal/`, broker and `BUILD_INFO.json` beside it | 80,701,241 / 103,805,146 | needs native run |
| `kassiber-cli-windows-x64.zip` | one-file `kassiber.exe` + `BUILD_INFO.json` | `kassiber.exe` + `BUILD_INFO.json` + `provider_broker/index.mjs` | 76,666,370 | needs native run |
| `kassiber-windows-x64.msi`, `.exe` | `binaries/kassiber-cli-x86_64-pc-windows-msvc.exe` + `bin/kassiber.exe` (PATH copy) | same, Rust; `bin/provider_broker/` beside the PATH copy | 82,616,320 / 79,044,700 | needs native run |
| `kassiber-<version>-manifest.txt` (+ `.sig`) | SHA-256 of every asset; OpenSSH signature | unchanged | — | generated locally, unsigned |

No release artifact carries an SBOM today; the Rust build does not add one.
Channel renderers consume these assets: AUR (`scripts/render_aur.py`) and Nix
(`scripts/render_nix.py`) now install the CLI under `/usr/lib/kassiber-cli`
(`$out/libexec/kassiber-cli` in Nix) with the broker beside it and depend on
`gcc-libs` / `stdenv.cc.cc.lib` instead of zlib; Homebrew is unchanged.

Bundled sidecar names keep the Rust target triples, so the supervisor
(`bundled_sidecar` in `ui-tauri/src-tauri/src/supervisor.rs`), the macOS bundle
launcher (`ui-tauri/src-tauri/bin/kassiber`), the Linux `/usr/bin/kassiber`
forwarder and the Windows PATH copy find the Rust binary unchanged.

## Linux x86_64

`scripts/build-linux-x64-rust-artifacts.sh` runs the CI Linux legs locally and
writes `dist/linux-x64/`. It preflights `dpkg-deb`, `rpmbuild`/`rpm2cpio`/`cpio`
and Tauri's cached `linuxdeploy` tools and stops unless `--allow-partial` is
given. It never downloads anything itself; Tauri would download `linuxdeploy`
on its first AppImage build.

### glibc floor and crypto linkage

Release builds run on `ubuntu-22.04` (glibc 2.35), the floor the deb and rpm
metadata declare. The floor check reads the non-weak version needs
(`readelf -V`; weak needs such as Rust's `pidfd_spawnp@GLIBC_2.39` do not stop
the loader) and prints the `objdump -T` maximum for the record. A build on a
newer host fails it; on this machine (glibc 2.44) the binary needs
`__isoc23_strtol@GLIBC_2.38` (C code compiled against glibc 2.38+ headers) and
`atan2f`/`log10f`/`acosf@GLIBC_2.43`. The desktop job runs the same check on
`kassiber-ui` and on the sidecar inside the deb and the AppImage.

SQLCipher links vendored OpenSSL statically on Linux and Windows
(`bundled-sqlcipher-vendored-openssl`) and CommonCrypto on macOS
(`bundled-sqlcipher`). The Linux binary therefore links only `libc.so.6`,
`libm.so.6`, `libgcc_s.so.1` and the loader. On macOS, a set `OPENSSL_DIR`
(or `OPENSSL_LIB_DIR`/`OPENSSL_INCLUDE_DIR`/`OPENSSL_STATIC`) would make
`libsqlite3-sys` link that OpenSSL, typically Homebrew's `libcrypto`, which
loads on the build Mac only. `build-rust-cli.sh` unsets those variables for the
macOS build and its `otool -L` check refuses any non-system dylib.

### Reproducibility

Two clean builds of the same commit at the same checkout and target paths, with
the same `SOURCE_DATE_EPOCH`, produced byte-identical binaries
(SHA-256 `09ee1b68…5ebe`, 125,047,552 bytes, both runs). Without
`SOURCE_DATE_EPOCH` they differed only in OpenSSL's `built on:` string, which
shifts `.rodata`. Builds from different paths still differ in two path strings
`--remap-path-prefix` cannot reach:

- vendored OpenSSL's `ENGINESDIR`/`MODULESDIR` and `OPENSSLDIR` defaults point
  into Cargo's `OUT_DIR`;
- `crates/rates` names the Kraken source directory through
  `env!("CARGO_MANIFEST_DIR")` in an error string.

CI always builds at the runner's fixed workspace path, so CI rebuilds of a tag
are reproducible; a different machine must use the same paths. The archive
packager uses GNU tar with sorted names, fixed owner and `--mtime` from
`SOURCE_DATE_EPOCH`, and `gzip -n`.

### Verification on this machine

Run on 2026-11-07, commit `dc9b62a48`, glibc floor overridden to the host's
2.44 (so these artifacts are not publishable):

- `scripts/smoke-packaged-cli.sh` passed: `--version`/`--help`, status before
  and after `init` (plaintext book), daemon handshake, bundled Kraken import
  (255,181 rows), `secrets init` (book header no longer `SQLite format 3`; a
  passphrase-less status refused with `interaction_required`) and status with
  `--db-passphrase-fd`, the Austrian moving-average journal and E1kv XLSX
  export, descriptor derivation.
- `tests/test_packaged_cli_no_egress.py` passed against the packaged binary:
  launch, fresh status, status with saved `.invalid` Esplora and loopback
  Electrum backends, SQLCipher status, and a daemon answering display reads
  and then idling 8 s, under a seccomp filter that kills the process on any
  IPv4/IPv6/packet socket (loopback included) or `io_uring_setup`, with libc's
  resolver calls recorded by an `LD_PRELOAD` shim. Its controls show both
  watchers fire, and a user-triggered `wallets sync` of the packaged binary
  against a loopback Esplora dies on its first socket.
- The CLI archive and the Tauri desktop deb were built (the deb natively by
  Tauri's bundler). The deb's sidecar is byte-identical to the smoked CLI
  binary, the broker and `BUILD_INFO.json` sit beside it, and `kassiber-ui`
  itself needs no glibc newer than 2.34 even from this host. Rebuilding the
  archive from the same inputs gave the same SHA-256.
- The stripped binary is 125,047,552 bytes (156 MB unstripped); the CLI
  archive shrinks 45% and the desktop deb 40% against the Python artifacts.
  About 15 MB of the binary is the embedded Kraken rate history.

### Missing tools (not downloaded)

| Tool | Needed for | Source |
| --- | --- | --- |
| `dpkg-deb` (`dpkg`) | CLI `.deb`; desktop `.rpm` (repacked from the deb) | Arch `dpkg` package / Ubuntu base |
| `rpmbuild`, `rpm2cpio` (`rpm-tools`) | CLI and desktop `.rpm` | Arch `rpm-tools` |
| `linuxdeploy-x86_64.AppImage`, `linuxdeploy-plugin-appimage.AppImage`, `linuxdeploy-plugin-gtk.sh` in `~/.cache/tauri` | AppImage | Tauri downloads them from GitHub on first use |
| Ubuntu 22.04 build environment (docker group membership or an `ubuntu:22.04` image with Rust 1.99.1, Node 22, pnpm 10.33.0 and the Tauri apt packages) | a floor-compliant local build | CI's `ubuntu-22.04` runner already provides it |

## Signing

The local artifacts and the manifest are unsigned. Release signing is
unchanged: after the tagged workflow stages the draft and the macOS app is
signed and notarized, the release operator signs the manifest through the
Bitwarden SSH agent:

```bash
export SSH_AUTH_SOCK="$HOME/.bitwarden-ssh-agent.sock"
ssh-add -l -E sha256   # must list SHA256:UzYeHzOEIbanmYDAylIaGhI6dGFvhNOkszPXwUgzo9M
uv run --locked python scripts/release_manifest.py sign \
  --manifest kassiber-<version>-manifest.txt \
  --public-key ~/.ssh/kassiber-release.pub
```

That writes `kassiber-<version>-manifest.txt.sig`; `finalize-signed-release.yml`
verifies it before publishing ([release signing](release-signing.md)).

## macOS arm64 (needs a native run)

The sidecar keeps the Python sidecar's file name, location and signing
identifier: `Contents/Resources/binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin`,
identifier `kassiber-cli-aarch64-apple-darwin`, Bitcoin Austria's team. The
Touch ID helper's code requirement pins exactly that identifier and team, and
Keychain items the signed Python sidecar wrote stay readable without a prompt
only for the same pair. `scripts/macos_release.py sign` already signs every
Mach-O under its file name; `verify` now also refuses an app whose sidecar is
missing from that location or signed under another identifier.
`build-rust-cli.sh` ad-hoc signs the binary under that identifier after
stripping, and the local app script re-signs it (and re-seals the bundle) if
Tauri's ad-hoc signing changed it.

Rust's own Mach-O stripping goes through rustup's `rust-objcopy`, which in a
minimal toolchain cannot load `libLLVM.dylib` and only warns. The build
therefore links unstripped and runs Xcode's `strip -x`, then re-signs. (Adding
the `llvm-tools` rustup component would also fix `rust-objcopy`.)

Native run, on an Apple Silicon Mac with Xcode command line tools, rustup and
pnpm 10.33.0:

```bash
./scripts/build-macos-arm64-rust-app.sh --no-backup     # local "Kassiber Rust Dev" app
codesign -d --verbose=2 "dist/macos-arm64/Kassiber Rust Dev.app/Contents/Resources/binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin"
# Identifier=kassiber-cli-aarch64-apple-darwin
```

The release path is the CI macOS leg plus the [macOS release runbook](macos-release.md)
(`prepare_macos_release.py`, `macos_release.py sign`, `notarize`, `verify
--smoke`). Still to prove natively: the strip/re-sign step, the bundled
sidecar's identifier after Tauri, Touch ID unlock against a Developer ID
signed Rust sidecar, and Keychain continuity for items the Python sidecar wrote.

## Windows x86_64 (needs a native run)

`scripts/build-windows-x64-rust-app.sh` mirrors the CI Windows legs in Git
Bash (MSVC build tools, rustup, Node 22, pnpm 10.33.0, Python 3.11, 7-Zip):

```bash
./scripts/build-windows-x64-rust-app.sh
```

It builds and smokes `kassiber.exe` (operator unlock included), writes the CLI
zip, stages the sidecar and the PATH copy `bin/kassiber.exe` with the broker
beside each, and builds the MSI and NSIS setup into `dist/windows-x64/`.
Nothing on Windows has been run for this change: the build, the SQLCipher
open, the keyring formats and the installers all need that run. The
OS-level no-egress check is Linux-only.

## CI changes

- `build-cli` no longer syncs the Python environment or runs PyInstaller. It
  installs uv's managed Python only for `write_build_info.py` and the smoke's
  JSON assertions, checks every version copy against the backend crate, and
  builds, smokes and packages the Rust binary. Linux runs the no-egress check.
- `build-desktop` refuses a staged `_internal/` payload, requires the broker
  and `BUILD_INFO.json` beside the sidecar, copies the broker beside the
  Windows PATH copy, and on Linux runs the glibc floor check on the shell and
  on the sidecar inside both the deb and the AppImage.
- Path filters follow the Rust inputs (`crates/**`, embedded data, skill
  references, the provider broker and the new scripts) instead of the frozen
  Python CLI inputs.
- The test-suite xdist mode in CI is unchanged.
