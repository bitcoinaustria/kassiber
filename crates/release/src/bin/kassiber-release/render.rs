//! The Linux channel and Homebrew renderers, ported from
//! `scripts/render_aur.py`, `scripts/render_nix.py` and
//! `scripts/render_homebrew.py`. Output bytes are the scripts' bytes.
use std::path::Path;

use sha2::{Digest, Sha256};

const RELEASE_URL_BASE: &str = "https://github.com/bitcoinaustria/kassiber/releases/download";
const TAP_CASK_NAME: &str = "bitcoinaustria/kassiber/kassiber";
const TAP_FORMULA_NAME: &str = "bitcoinaustria/kassiber/kassiber-cli";

pub type Result<T> = std::result::Result<T, String>;

fn io(path: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |error| format!("{}: {error}", path.display())
}

fn repr(text: &str) -> String {
    // Python's repr of a str, for the `invalid ... version: {version!r}` messages.
    if text.contains('\'') && !text.contains('"') {
        format!("\"{text}\"")
    } else {
        format!("'{}'", text.replace('\'', "\\'"))
    }
}

/// `[0-9]+(?:\.[0-9A-Za-z_<extra>]+)+` over the whole text.
fn version_shape(text: &str, extra: &[u8]) -> bool {
    let mut parts = text.split('.');
    let first = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    !first.is_empty()
        && first.bytes().all(|b| b.is_ascii_digit())
        && !rest.is_empty()
        && rest.iter().all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || extra.contains(&b))
        })
}

fn sha_shape(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `re.match(r"^...$", text)`: `$` also matches before one final newline.
fn dollar(text: &str) -> &str {
    text.strip_suffix('\n').unwrap_or(text)
}

fn without_v(version: &str) -> &str {
    version.strip_prefix('v').unwrap_or(version)
}

fn sha256_file(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(io(path))?;
    Ok(Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// `Path.write_text(content)` then `chmod`.
fn write(path: &Path, content: &str, executable: bool) -> Result<()> {
    std::fs::write(path, content).map_err(io(path))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).map_err(io(path))?;
    }
    let _ = executable;
    Ok(())
}

/// `shutil.copyfile`: contents only, never the mode.
fn copy(source: &Path, destination: &Path) -> Result<()> {
    let bytes = std::fs::read(source).map_err(io(source))?;
    std::fs::write(destination, bytes).map_err(io(destination))
}

fn aur_version(version: &str) -> Result<&str> {
    let normalized = without_v(version);
    if !version_shape(normalized, b"") {
        return Err(format!("invalid AUR version: {}", repr(version)));
    }
    Ok(normalized)
}

fn exact_sha<'a>(sha256: &'a str, artifact: &str) -> Result<&'a str> {
    if !sha_shape(sha256) {
        return Err(format!(
            "sha256 for {artifact} must be 64 lowercase hex characters"
        ));
    }
    Ok(sha256)
}

pub fn aur_desktop(root: &Path, version: &str, sha256: &str, output: &Path) -> Result<()> {
    let version = aur_version(version)?;
    let artifact = "kassiber-linux-x64.AppImage";
    let artifact_sha256 = exact_sha(sha256, artifact)?;
    std::fs::create_dir_all(output).map_err(io(output))?;
    write(
        &output.join("kassiber"),
        "#!/bin/sh\nexport APPIMAGE_EXTRACT_AND_RUN=1\nexec /opt/kassiber/kassiber.AppImage --cli \"$@\"\n",
        true,
    )?;
    write(
        &output.join("kassiber-ui"),
        "#!/bin/sh\nexport APPIMAGE_EXTRACT_AND_RUN=1\nexec /opt/kassiber/kassiber.AppImage \"$@\"\n",
        true,
    )?;
    write(
        &output.join("kassiber.desktop"),
        "[Desktop Entry]\nType=Application\nName=Kassiber\nComment=Local-first Bitcoin accounting suite\nExec=kassiber-ui\nIcon=kassiber-ui\nTerminal=false\nCategories=Office;Finance;\n",
        false,
    )?;
    copy(
        &root.join("ui-tauri/src-tauri/icons/128x128.png"),
        &output.join("kassiber.png"),
    )?;
    copy(&root.join("LICENSE"), &output.join("LICENSE"))?;
    let local_sources = [
        "kassiber",
        "kassiber-ui",
        "kassiber.desktop",
        "kassiber.png",
        "LICENSE",
    ];
    let local_hashes = local_sources
        .iter()
        .map(|name| sha256_file(&output.join(name)))
        .collect::<Result<Vec<_>>>()?;
    let renamed = format!("kassiber-bin-{version}.AppImage");
    let url = format!("{RELEASE_URL_BASE}/v{version}/{artifact}");
    let mut source_lines: Vec<String> = local_sources
        .iter()
        .map(|name| format!("        '{name}'"))
        .collect();
    source_lines.push(format!("        '{renamed}::{url}'"));
    let mut hash_lines: Vec<String> = local_hashes
        .iter()
        .map(|value| format!("          '{value}'"))
        .collect();
    hash_lines.push(format!("          '{artifact_sha256}'"));
    let pkgbuild = format!(
        r#"pkgname=kassiber-bin
pkgver={version}
pkgrel=1
pkgdesc='Local-first Bitcoin accounting desktop application'
arch=('x86_64')
url='https://github.com/bitcoinaustria/kassiber'
license=('AGPL-3.0-only')
depends=('glibc' 'gcc-libs' 'gtk3' 'webkit2gtk-4.1')
provides=('kassiber' 'kassiber-command')
conflicts=('kassiber' 'kassiber-cli' 'kassiber-cli-bin')
options=('!strip')
source=(
{sources}
)
sha256sums=(
{hashes}
)

package() {{
  install -Dpm755 "${{srcdir}}/{renamed}"     "${{pkgdir}}/opt/kassiber/kassiber.AppImage"
  install -Dpm755 "${{srcdir}}/kassiber" "${{pkgdir}}/usr/bin/kassiber"
  install -Dpm755 "${{srcdir}}/kassiber-ui" "${{pkgdir}}/usr/bin/kassiber-ui"
  install -Dpm644 "${{srcdir}}/kassiber.desktop"     "${{pkgdir}}/usr/share/applications/Kassiber.desktop"
  install -Dpm644 "${{srcdir}}/kassiber.png"     "${{pkgdir}}/usr/share/icons/hicolor/128x128/apps/kassiber-ui.png"
  install -Dpm644 "${{srcdir}}/LICENSE"     "${{pkgdir}}/usr/share/licenses/${{pkgname}}/LICENSE"
}}
"#,
        sources = source_lines.join("\n"),
        hashes = hash_lines.join("\n"),
    );
    write(&output.join("PKGBUILD"), &pkgbuild, false)?;
    let mut source_info: Vec<String> = local_sources
        .iter()
        .map(|name| format!("\tsource = {name}"))
        .collect();
    source_info.push(format!("\tsource = {renamed}::{url}"));
    let mut hash_info: Vec<String> = local_hashes
        .iter()
        .map(|value| format!("\tsha256sums = {value}"))
        .collect();
    hash_info.push(format!("\tsha256sums = {artifact_sha256}"));
    let srcinfo = format!(
        "pkgbase = kassiber-bin
\tpkgdesc = Local-first Bitcoin accounting desktop application
\tpkgver = {version}
\tpkgrel = 1
\turl = https://github.com/bitcoinaustria/kassiber
\tarch = x86_64
\tlicense = AGPL-3.0-only
\tdepends = glibc
\tdepends = gcc-libs
\tdepends = gtk3
\tdepends = webkit2gtk-4.1
\tprovides = kassiber
\tprovides = kassiber-command
\tconflicts = kassiber
\tconflicts = kassiber-cli
\tconflicts = kassiber-cli-bin
\toptions = !strip
{}
{}

pkgname = kassiber-bin
",
        source_info.join("\n"),
        hash_info.join("\n"),
    );
    write(&output.join(".SRCINFO"), &srcinfo, false)
}

pub fn aur_cli(root: &Path, version: &str, sha256: &str, output: &Path) -> Result<()> {
    let version = aur_version(version)?;
    let artifact = "kassiber-cli-linux-x64.tar.gz";
    let artifact_sha256 = exact_sha(sha256, artifact)?;
    std::fs::create_dir_all(output).map_err(io(output))?;
    copy(&root.join("LICENSE"), &output.join("LICENSE"))?;
    let license_sha256 = sha256_file(&output.join("LICENSE"))?;
    let renamed = format!("kassiber-cli-bin-{version}.tar.gz");
    let url = format!("{RELEASE_URL_BASE}/v{version}/{artifact}");
    let pkgbuild = format!(
        r#"pkgname=kassiber-cli-bin
pkgver={version}
pkgrel=1
pkgdesc='Local-first Bitcoin accounting CLI'
arch=('x86_64')
url='https://github.com/bitcoinaustria/kassiber'
license=('AGPL-3.0-only')
depends=('glibc' 'gcc-libs')
provides=('kassiber-cli' 'kassiber-command')
conflicts=('kassiber' 'kassiber-bin' 'kassiber-cli')
options=('!strip')
source=('LICENSE'
        '{renamed}::{url}')
sha256sums=('{license_sha256}'
            '{artifact_sha256}')

package() {{
  # The binary finds the AI provider broker beside its resolved executable.
  install -Dpm755 "${{srcdir}}/kassiber-cli-linux-x64/kassiber"     "${{pkgdir}}/usr/lib/kassiber-cli/kassiber"
  install -Dpm644 "${{srcdir}}/kassiber-cli-linux-x64/provider_broker/index.mjs"     "${{pkgdir}}/usr/lib/kassiber-cli/provider_broker/index.mjs"
  install -dm755 "${{pkgdir}}/usr/bin"
  ln -s ../lib/kassiber-cli/kassiber "${{pkgdir}}/usr/bin/kassiber"
  install -Dpm644 "${{srcdir}}/LICENSE"     "${{pkgdir}}/usr/share/licenses/${{pkgname}}/LICENSE"
}}
"#
    );
    write(&output.join("PKGBUILD"), &pkgbuild, false)?;
    let srcinfo = format!(
        "pkgbase = kassiber-cli-bin
\tpkgdesc = Local-first Bitcoin accounting CLI
\tpkgver = {version}
\tpkgrel = 1
\turl = https://github.com/bitcoinaustria/kassiber
\tarch = x86_64
\tlicense = AGPL-3.0-only
\tdepends = glibc
\tdepends = gcc-libs
\tprovides = kassiber-cli
\tprovides = kassiber-command
\tconflicts = kassiber
\tconflicts = kassiber-bin
\tconflicts = kassiber-cli
\toptions = !strip
\tsource = LICENSE
\tsource = {renamed}::{url}
\tsha256sums = {license_sha256}
\tsha256sums = {artifact_sha256}

pkgname = kassiber-cli-bin
"
    );
    write(&output.join(".SRCINFO"), &srcinfo, false)
}

pub fn nix(
    root: &Path,
    version: &str,
    sha256_desktop: &str,
    sha256_cli: &str,
    output: &Path,
) -> Result<()> {
    let normalized = without_v(version);
    if !version_shape(normalized, b"") {
        return Err(format!("invalid Nix package version: {}", repr(version)));
    }
    let version = normalized;
    let desktop_artifact = "kassiber-linux-x64.AppImage";
    let cli_artifact = "kassiber-cli-linux-x64.tar.gz";
    let desktop_sha256 = exact_sha(sha256_desktop, desktop_artifact)?;
    let cli_sha256 = exact_sha(sha256_cli, cli_artifact)?;
    std::fs::create_dir_all(output).map_err(io(output))?;
    copy(&root.join("LICENSE"), &output.join("LICENSE"))?;
    let desktop_url = format!("{RELEASE_URL_BASE}/v{version}/{desktop_artifact}");
    let cli_url = format!("{RELEASE_URL_BASE}/v{version}/{cli_artifact}");
    let flake = include_str!("flake.nix.in")
        .replace("@VERSION@", version)
        .replace("@CLI_URL@", &cli_url)
        .replace("@CLI_SHA256@", cli_sha256)
        .replace("@DESKTOP_URL@", &desktop_url)
        .replace("@DESKTOP_SHA256@", desktop_sha256);
    let path = output.join("flake.nix");
    std::fs::write(&path, flake).map_err(io(&path))
}

fn homebrew_version(version: &str) -> Result<&str> {
    let normalized = without_v(version);
    if !version_shape(dollar(normalized), b"-") {
        return Err(format!("invalid Homebrew version: {}", repr(version)));
    }
    Ok(normalized)
}

fn homebrew_sha<'a>(sha256: &'a str, artifact: &str) -> Result<&'a str> {
    if !sha_shape(dollar(sha256)) {
        return Err(format!(
            "sha256 for {artifact} must be 64 lowercase hex characters"
        ));
    }
    Ok(sha256)
}

pub fn cask(version: &str, sha256: &str) -> Result<String> {
    let cask_version = homebrew_version(version)?;
    let dmg_sha256 = homebrew_sha(sha256, "kassiber-macos-arm64.dmg")?;
    Ok(format!(
        r##"cask "kassiber" do
  version "{cask_version}"
  sha256 "{dmg_sha256}"

  url "{RELEASE_URL_BASE}/v#{{version}}/kassiber-macos-arm64.dmg"
  name "Kassiber"
  desc "Local-first Bitcoin accounting suite"
  homepage "https://github.com/bitcoinaustria/kassiber"

  depends_on arch: :arm64

  app "Kassiber.app"
  binary "#{{appdir}}/Kassiber.app/Contents/Resources/bin/kassiber",
         target: "kassiber"

  zap trash: [
    "~/Library/Application Support/at.bitcoinaustria.kassiber",
    "~/Library/Preferences/at.bitcoinaustria.kassiber.plist",
    "~/Library/Saved Application State/at.bitcoinaustria.kassiber.savedState",
  ]

  # Homebrew has no cask<->formula conflict stanza (the cask one only accepts
  # cask:), so the overlap is surfaced as a caveat on both sides.
  caveats <<~EOS
    The {TAP_FORMULA_NAME} formula installs the same
    `kassiber` command. Install either this cask or the formula, not both.
  EOS
end
"##
    ))
}

pub fn cli_formula(
    version: &str,
    sha256_macos_arm64: &str,
    sha256_linux_x64: &str,
) -> Result<String> {
    let formula_version = homebrew_version(version)?;
    let macos_arm64 = homebrew_sha(sha256_macos_arm64, "kassiber-cli-macos-arm64.tar.gz")?;
    let linux_x64 = homebrew_sha(sha256_linux_x64, "kassiber-cli-linux-x64.tar.gz")?;
    Ok(format!(
        r##"class KassiberCli < Formula
  desc "Local-first Bitcoin accounting CLI"
  homepage "https://github.com/bitcoinaustria/kassiber"
  version "{formula_version}"
  license "AGPL-3.0-only"

  on_macos do
    # Homebrew must not rewrite the Developer ID-sealed runtime or metadata.
    preserve_rpath
    skip_clean "libexec/Kassiber.app"
    on_arm do
      url "{RELEASE_URL_BASE}/v#{{version}}/kassiber-cli-macos-arm64.tar.gz"
      sha256 "{macos_arm64}"
    end
  end

  on_linux do
    on_intel do
      url "{RELEASE_URL_BASE}/v#{{version}}/kassiber-cli-linux-x64.tar.gz"
      sha256 "{linux_x64}"
    end
  end

  def install
    if OS.mac?
      # Keep the notarized bundle intact; its launcher resolves symlinks.
      libexec.install "Kassiber.app"
      (bin/"kassiber").write_env_script libexec/"Kassiber.app/Contents/Resources/bin/kassiber",
                                       KASSIBER_HOMEBREW_PACKAGE: "formula"
    else
      bin.install "kassiber"
    end
  end

  # Formulae cannot declare conflicts with casks, so the overlap is surfaced
  # as user guidance only, mirroring the cask-side caveat.
  def caveats
    <<~EOS
      The Kassiber desktop cask ("{TAP_CASK_NAME}") links its own
      `kassiber` command. Install either kassiber-cli or the desktop cask,
      not both.
    EOS
  end

  test do
    assert_match "Kassiber", shell_output("#{{bin}}/kassiber --version")
  end
end
"##
    ))
}

/// `Path.parent.mkdir(parents=True)` then `write_text`.
pub fn write_rendered(output: &Path, rendered: &str) -> Result<()> {
    if let Some(parent) = output.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    std::fs::write(output, rendered).map_err(io(output))
}
