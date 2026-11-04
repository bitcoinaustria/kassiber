//! `verify_download`: authenticate the manifest with a pinned OpenSSH key,
//! then verify the selected artifact's SHA-256. No network access.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use base64::Engine;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use sha2::{Digest, Sha256};

use crate::{splitlines, strip, Result};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_SIGNATURE_BYTES: u64 = 16 * 1024;
const MAX_PUBLIC_KEY_BYTES: u64 = 16 * 1024;
const MAX_SSH_KEYGEN_OUTPUT_BYTES: usize = 16 * 1024;
const NAMESPACE: &str = "kassiber-release";
const PRINCIPAL: &str = "release@kassiber";
const MINIMUM_OPENSSH_VERSION: &str = "8.1";
const SSH_KEYGEN_TIMEOUT: Duration = Duration::from_secs(30);
const MANIFEST_HEADER: &str = "# Kassiber release manifest v1";
const MANIFEST_VERSION_PREFIX: &str = "# Version: ";
const SIGNATURE_HEADER: &[u8] = b"-----BEGIN SSH SIGNATURE-----";

fn file_error(message: String, path: &Path, label: &str) -> AppError {
    AppError::new("release_verification_file_error", message).with_details(dict([
        ("path", s(path.to_string_lossy().as_ref())),
        ("label", s(label)),
    ]))
}

/// `_open_regular_file`: no final symlink, regular files only.
fn open_regular(path: &Path, label: &str) -> Result<(std::fs::File, u64)> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(nix::libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| {
        file_error(
            format!("Could not read {label}: {}", path.display()),
            path,
            label,
        )
    })?;
    let metadata = file.metadata().map_err(|_| {
        file_error(
            format!("Could not inspect {label}: {}", path.display()),
            path,
            label,
        )
    })?;
    if !metadata.is_file() {
        let mut title = label.to_owned();
        title[..1].make_ascii_uppercase();
        return Err(file_error(
            format!("{title} is not a regular file: {}", path.display()),
            path,
            label,
        ));
    }
    Ok((file, metadata.len()))
}
/// `str.capitalize()` for the ASCII labels used here.
fn capitalize(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => {
            first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()
        }
        None => String::new(),
    }
}

/// `_require_small_regular_file(path, limit, label)`.
fn small_file(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>> {
    let too_large = || {
        AppError::new(
            "release_verification_file_error",
            format!("{} is unexpectedly large", capitalize(label)),
        )
        .with_details(dict([
            ("path", s(path.to_string_lossy().as_ref())),
            ("maximum_bytes", Value::Int(limit.into())),
        ]))
    };
    let (file, size) = open_regular(path, label)?;
    if size > limit {
        return Err(too_large());
    }
    let mut content = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut content)
        .map_err(|_| {
            file_error(
                format!("Could not read {label}: {}", path.display()),
                path,
                label,
            )
        })?;
    if content.len() as u64 > limit {
        return Err(too_large());
    }
    Ok(content)
}

/// `Path(value).expanduser()`.
fn expand(value: &str) -> PathBuf {
    kassiber_paths::Context::from_env()
        .ok()
        .and_then(|context| context.expand_user(Path::new(value)).ok())
        .unwrap_or_else(|| PathBuf::from(value))
}

/// `normalize_fingerprint(value)`: a complete `SHA256:` + 43 base64 chars.
fn normalize_fingerprint(value: &str) -> Result<String> {
    let normalized = strip(value);
    let valid = normalized.strip_prefix("SHA256:").is_some_and(|rest| {
        rest.len() == 43
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    });
    if !valid {
        return Err(AppError::new(
            "invalid_release_fingerprint",
            "Release-key fingerprint must be a complete OpenSSH SHA256 fingerprint",
        )
        .with_hint("Copy the complete SHA256:... Kassiber release-key fingerprint from an independent trusted source."));
    }
    Ok(normalized.to_owned())
}

/// `base64.b64decode(text, validate=True)`: strict alphabet, correct padding.
fn b64decode(text: &str) -> Option<Vec<u8>> {
    let data = text.trim_end_matches('=');
    let padding = text.len() - data.len();
    if padding > 2
        || !data
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/')
    {
        return None;
    }
    let required = match data.len() % 4 {
        0 => 0,
        1 => return None,
        2 => 2,
        _ => 1,
    };
    if padding < required {
        return None;
    }
    base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(data)
        .or_else(|_| {
            // Python ignores non-zero trailing bits that a strict decoder refuses.
            base64::engine::GeneralPurpose::new(
                &base64::alphabet::STANDARD,
                base64::engine::GeneralPurposeConfig::new()
                    .with_decode_allow_trailing_bits(true)
                    .with_decode_padding_mode(base64::engine::DecodePaddingMode::RequireNone),
            )
            .decode(data)
        })
        .ok()
}

/// `_parse_key_fields(key_type, encoded)`: one Ed25519 or Ed25519-SK key blob.
fn key_fields(key_type: &str, encoded: &str) -> Option<Vec<u8>> {
    if !["ssh-ed25519", "sk-ssh-ed25519@openssh.com"].contains(&key_type) {
        return None;
    }
    let blob = b64decode(encoded)?;
    let mut fields = Vec::new();
    let mut offset = 0usize;
    while offset < blob.len() {
        let length = u32::from_be_bytes(blob.get(offset..offset + 4)?.try_into().ok()?) as usize;
        let start = offset + 4;
        let end = start.checked_add(length)?;
        fields.push(blob.get(start..end)?.to_vec());
        offset = end;
    }
    let expected = if key_type == "ssh-ed25519" { 2 } else { 3 };
    (fields.len() == expected
        && fields[0] == key_type.as_bytes()
        && fields[1].len() == 32
        && (expected == 2 || fields[2].starts_with(b"ssh:")))
    .then_some(blob)
}

/// `_single_key_line(raw, label, code)`.
fn single_key_line(raw: &[u8], label: &str, code: &str) -> Result<String> {
    let text = std::str::from_utf8(raw)
        .ok()
        .filter(|text| text.is_ascii())
        .ok_or_else(|| {
            AppError::new(
                code,
                format!("{} is not an OpenSSH text file", capitalize(label)),
            )
        })?;
    let lines: Vec<&str> = splitlines(text)
        .into_iter()
        .map(strip)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect();
    if lines.len() != 1 {
        return Err(AppError::new(
            code,
            format!("{} must contain exactly one key entry", capitalize(label)),
        ));
    }
    Ok(lines[0].to_owned())
}

/// `parse_ssh_public_key(raw)`.
fn parse_public_key(raw: &[u8]) -> Result<(String, Vec<u8>)> {
    let code = "invalid_release_public_key";
    let line = single_key_line(raw, "release public key", code)?;
    let tokens: Vec<&str> = line
        .split(crate::is_space)
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() < 2 {
        return Err(AppError::new(
            code,
            "Release public key is not an OpenSSH public key",
        ));
    }
    key_fields(tokens[0], tokens[1])
        .map(|blob| (tokens[0].to_owned(), blob))
        .ok_or_else(|| {
            AppError::new(
                code,
                "Release public key must be one OpenSSH Ed25519 or Ed25519-SK public key",
            )
        })
}

/// `ssh_key_fingerprint(blob)`.
pub fn fingerprint(blob: &[u8]) -> String {
    let digest = Sha256::digest(blob);
    format!(
        "SHA256:{}",
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest)
    )
}

/// `allowed_signers_entry(key_type, blob)`: the only entry trusted.
fn allowed_signers(key_type: &str, blob: &[u8]) -> String {
    format!(
        "{PRINCIPAL} namespaces=\"{NAMESPACE}\" {key_type} {}\n",
        base64::engine::general_purpose::STANDARD.encode(blob)
    )
}

/// `ssh_keygen_command()`: `shutil.which("ssh-keygen")` over `PATH`.
fn ssh_keygen() -> Result<PathBuf> {
    let found = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| {
                if dir.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    dir
                }
            })
            .map(|dir| dir.join("ssh-keygen"))
            .find(|candidate| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    candidate
                        .metadata()
                        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                }
                #[cfg(not(unix))]
                {
                    candidate.is_file()
                }
            })
    });
    found.ok_or_else(|| {
        AppError::new(
            "ssh_keygen_unavailable",
            "OpenSSH's ssh-keygen is required to verify this release signature",
        )
        .with_hint(format!(
            "Install OpenSSH {MINIMUM_OPENSSH_VERSION} or newer, then run kassiber verify-download again."
        ))
    })
}

fn signature_failed(expected: &str) -> AppError {
    AppError::new(
        "release_signature_verification_failed",
        "Release manifest has no valid signature from the expected Kassiber release key",
    )
    .with_hint("Do not install or run the release artifact.")
    .with_details(dict([("expected_fingerprint", s(expected))]))
}

/// `_GOOD_SIGNATURE.fullmatch(line)` → the signing key's fingerprint.
fn good_signature(line: &str) -> Option<&str> {
    let rest = line.strip_prefix(&format!(
        "Good \"{NAMESPACE}\" signature for {PRINCIPAL} with "
    ))?;
    let (kind, rest) = rest.split_once(" key ")?;
    if kind.is_empty()
        || !kind
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
    {
        return None;
    }
    normalize_fingerprint(rest)
        .ok()
        .filter(|f| f == rest)
        .map(|_| rest)
}

/// `ssh_keygen_output_is_unsupported(output)`.
fn unsupported(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("usage: ssh-keygen")
        || ["unknown", "illegal", "invalid", "unrecognized"]
            .iter()
            .any(|word| lower.contains(&format!("{word} option")))
}

/// `_bounded_text(value)`.
fn bounded(bytes: &[u8]) -> String {
    String::from_utf8_lossy(&bytes[..bytes.len().min(MAX_SSH_KEYGEN_OUTPUT_BYTES)]).into_owned()
}

/// `verify_signature_bytes(...)`: the pinned fingerprint is the root of trust.
fn verify_signature(
    manifest: &[u8],
    signature: &[u8],
    public_key: &[u8],
    expected: &str,
) -> Result<String> {
    let expected = normalize_fingerprint(expected)?;
    let (key_type, blob) = parse_public_key(public_key)?;
    if fingerprint(&blob) != expected {
        return Err(AppError::new(
            "release_fingerprint_mismatch",
            "Release public key does not match the expected fingerprint",
        )
        .with_hint(
            "Obtain the public key and full fingerprint again from independent trusted sources.",
        )
        .with_details(dict([("expected_fingerprint", s(expected.as_str()))])));
    }
    let leading = signature
        .iter()
        .position(|b| !(b.is_ascii_whitespace() || *b == 0x0b))
        .unwrap_or(signature.len());
    if !signature[leading..].starts_with(SIGNATURE_HEADER) {
        return Err(AppError::new(
            "release_signature_verification_failed",
            "Release signature is not a detached OpenSSH signature",
        )
        .with_hint("Do not install or run the release artifact.")
        .with_details(dict([("expected_fingerprint", s(expected.as_str()))])));
    }
    let program = ssh_keygen()?;
    let failed = || {
        AppError::new(
            "release_signature_verification_failed",
            "ssh-keygen could not verify the release signature",
        )
        .with_hint("Do not install or run the release artifact.")
        .with_details(dict([("expected_fingerprint", s(expected.as_str()))]))
    };
    let workspace = tempfile::Builder::new()
        .prefix("kassiber-sshsig-")
        .tempdir()
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(workspace.path(), std::fs::Permissions::from_mode(0o700))
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    }
    let signers = workspace.path().join("allowed_signers");
    let signature_path = workspace.path().join("release-manifest.txt.sig");
    std::fs::write(&signers, allowed_signers(&key_type, &blob))
        .and_then(|_| std::fs::write(&signature_path, signature))
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let mut child = Command::new(&program)
        .args(["-Y", "verify", "-f"])
        .arg(&signers)
        .args(["-I", PRINCIPAL, "-n", NAMESPACE, "-s"])
        .arg(&signature_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| failed())?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let input = manifest.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let err = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < SSH_KEYGEN_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(10))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(failed());
            }
        }
    };
    let _ = writer.join();
    let stdout = bounded(&out.join().unwrap_or_default());
    let stderr = bounded(&err.join().unwrap_or_default());
    drop(workspace);
    let signed_by: std::collections::BTreeSet<&str> = splitlines(&stdout)
        .into_iter()
        .filter_map(|line| good_signature(strip(line)))
        .collect();
    if status.success() && signed_by.len() == 1 && signed_by.contains(expected.as_str()) {
        return Ok(expected);
    }
    if !status.success() && unsupported(&format!("{stdout}\n{stderr}")) {
        return Err(AppError::new(
            "ssh_keygen_unsupported",
            format!("This ssh-keygen cannot verify OpenSSH signatures (OpenSSH {MINIMUM_OPENSSH_VERSION} or newer is required)"),
        )
        .with_hint(format!(
            "Install OpenSSH {MINIMUM_OPENSSH_VERSION} or newer, then run kassiber verify-download again."
        )));
    }
    Err(signature_failed(&expected))
}

/// `_normalize_release_version(version)`.
fn release_version(version: &str) -> Result<String> {
    let normalized = version.strip_prefix('v').unwrap_or(version);
    if normalized != strip(normalized)
        || normalized.starts_with('v')
        || crate::update::parse_version(normalized).is_none()
    {
        return Err(AppError::new(
            "invalid_release_version",
            format!("Invalid release version: {version}"),
        ));
    }
    Ok(normalized.to_owned())
}

/// `_release_version_from_manifest_name(name)`.
fn manifest_version(name: &str) -> Result<String> {
    match name
        .strip_prefix("kassiber-")
        .and_then(|rest| rest.strip_suffix("-manifest.txt"))
    {
        Some(version) => release_version(version),
        None => Err(AppError::new(
            "invalid_release_manifest",
            format!("Invalid Kassiber release manifest filename: {name}"),
        )),
    }
}

/// `_MANIFEST_LINE.fullmatch(line)`: `(sha256, filename)`.
fn manifest_line(line: &str) -> Option<(&str, &str)> {
    let (digest, filename) = (line.get(..64)?, line.get(64..)?.strip_prefix("  ")?);
    let digest_ok = digest
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    let first = filename.bytes().next()?;
    let name_ok = first.is_ascii_alphanumeric()
        && filename.len() <= 255
        && filename
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b));
    (digest_ok && name_ok).then_some((digest, filename))
}

/// `_parse_release_manifest_bytes(raw, source, expected_version)`.
fn manifest_entries(raw: &[u8], source: &str, expected: &str) -> Result<Vec<(String, String)>> {
    let path = || dict([("path", s(source))]);
    let text = std::str::from_utf8(raw).map_err(|_| {
        AppError::new(
            "invalid_release_manifest",
            "Release manifest is not valid UTF-8",
        )
        .with_details(path())
    })?;
    let lines = splitlines(text);
    if lines.len() < 3 || lines[0] != MANIFEST_HEADER {
        return Err(AppError::new(
            "invalid_release_manifest",
            "Release manifest has no supported Kassiber format header",
        )
        .with_details(path()));
    }
    if lines[1] != format!("{MANIFEST_VERSION_PREFIX}{expected}") {
        let signed = lines[1]
            .strip_prefix(MANIFEST_VERSION_PREFIX)
            .unwrap_or(lines[1]);
        let signed: String = signed.chars().take(128).collect();
        return Err(AppError::new(
            "release_manifest_version_mismatch",
            "Signed release version does not match the manifest filename",
        )
        .with_hint("Do not install or run artifacts from this release.")
        .with_details(dict([
            ("path", s(source)),
            ("expected_version", s(expected)),
            ("signed_version", s(signed)),
        ])));
    }
    let mut entries: Vec<(String, String)> = Vec::new();
    for (index, line) in lines[2..].iter().enumerate() {
        let number = index + 3;
        let Some((digest, filename)) = manifest_line(line) else {
            return Err(AppError::new(
                "invalid_release_manifest",
                format!("Invalid release manifest line {number}"),
            )
            .with_details(dict([
                ("path", s(source)),
                ("line", Value::Int(number.into())),
            ])));
        };
        if entries.iter().any(|(name, _)| name == filename) {
            return Err(AppError::new(
                "invalid_release_manifest",
                format!("Duplicate release manifest entry: {filename}"),
            )
            .with_details(dict([("path", s(source)), ("filename", s(filename))])));
        }
        entries.push((filename.to_owned(), digest.to_owned()));
    }
    Ok(entries)
}

/// `sha256_file(path)`.
fn sha256_file(path: &Path) -> Result<String> {
    let label = "release artifact";
    let (mut file, _) = open_regular(path, label)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|_| {
            file_error(
                format!("Could not read {label}: {}", path.display()),
                path,
                label,
            )
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// `pathlib.PurePath.name`.
fn name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `verify_download(artifact, manifest, signature, public_key, fingerprint)`.
pub fn verify_download(
    artifact: &str,
    manifest: &str,
    signature: &str,
    public_key: &str,
    expected: &str,
) -> Result<Value> {
    let manifest_path = expand(manifest);
    let signature_path = expand(signature);
    let manifest_bytes = small_file(&manifest_path, MAX_MANIFEST_BYTES, "release manifest")?;
    let signature_bytes = small_file(&signature_path, MAX_SIGNATURE_BYTES, "release signature")?;
    let public_key_bytes = small_file(
        &expand(public_key),
        MAX_PUBLIC_KEY_BYTES,
        "release public key",
    )?;
    let signer = verify_signature(
        &manifest_bytes,
        &signature_bytes,
        &public_key_bytes,
        expected,
    )?;
    let manifest_name = name(&manifest_path);
    let entries = manifest_entries(
        &manifest_bytes,
        &manifest_path.to_string_lossy(),
        &manifest_version(&manifest_name)?,
    )?;
    if entries.is_empty() {
        return Err(
            AppError::new("invalid_release_manifest", "Release manifest is empty").with_details(
                dict([("path", s(manifest_path.to_string_lossy().as_ref()))]),
            ),
        );
    }
    let artifact_path = expand(artifact);
    let filename = name(&artifact_path);
    let Some((_, digest)) = entries.iter().find(|(name, _)| *name == filename) else {
        return Err(AppError::new(
            "artifact_not_in_release_manifest",
            format!("Release manifest does not contain {filename}"),
        )
        .with_details(dict([
            ("artifact", s(filename.as_str())),
            ("manifest", s(manifest_name.as_str())),
        ])));
    };
    let actual = sha256_file(&artifact_path)?;
    if actual != *digest {
        return Err(AppError::new(
            "release_artifact_hash_mismatch",
            format!("SHA-256 mismatch for {filename}"),
        )
        .with_hint("Do not install or run this file. Delete it and download the release again.")
        .with_details(dict([
            ("artifact", s(filename.as_str())),
            ("expected_sha256", s(digest.as_str())),
            ("actual_sha256", s(actual.as_str())),
        ])));
    }
    Ok(dict([
        ("verified", Value::Bool(true)),
        ("artifact", s(filename)),
        ("sha256", s(actual)),
        ("manifest", s(manifest_name)),
        ("signature", s(name(&signature_path))),
        ("signer_fingerprint", s(signer)),
        ("signer_principal", s(PRINCIPAL)),
        ("signature_namespace", s(NAMESPACE)),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_and_key_parsing_follow_python() {
        assert_eq!(b64decode("AAAA").unwrap(), vec![0, 0, 0]);
        assert_eq!(b64decode("AA==").unwrap(), vec![0]);
        assert!(b64decode("AA=").is_none());
        assert!(b64decode("AAAAA").is_none());
        assert!(b64decode("AA*A").is_none());
        let mut blob = Vec::new();
        for field in [b"ssh-ed25519".as_slice(), &[7u8; 32]] {
            blob.extend((field.len() as u32).to_be_bytes());
            blob.extend(field);
        }
        let encoded = base64::engine::general_purpose::STANDARD.encode(&blob);
        let line = format!("# comment\n\nssh-ed25519 {encoded} release\n");
        let (kind, parsed) = parse_public_key(line.as_bytes()).unwrap();
        assert_eq!((kind.as_str(), parsed), ("ssh-ed25519", blob.clone()));
        assert_eq!(fingerprint(&blob).len(), 50);
        let two = format!("ssh-ed25519 {encoded}\nssh-ed25519 {encoded}\n");
        assert_eq!(
            parse_public_key(two.as_bytes()).unwrap_err().code,
            "invalid_release_public_key"
        );
    }

    #[test]
    fn manifest_lines_versions_and_good_signatures() {
        let digest = "a".repeat(64);
        assert!(manifest_line(&format!("{digest}  kassiber-1.0.0.dmg")).is_some());
        assert!(manifest_line(&format!("{digest} kassiber.dmg")).is_none());
        assert!(manifest_line(&format!("{digest}  .hidden")).is_none());
        assert_eq!(
            manifest_version("kassiber-v1.2.3-manifest.txt").unwrap(),
            "1.2.3"
        );
        assert!(manifest_version("kassiber-vv1.2.3-manifest.txt").is_err());
        let fp = format!("SHA256:{}", "A".repeat(43));
        let line = format!(
            "Good \"kassiber-release\" signature for release@kassiber with ED25519 key {fp}"
        );
        assert_eq!(good_signature(&line), Some(fp.as_str()));
        assert!(good_signature(&format!("{line}x")).is_none());
        assert!(unsupported("ssh-keygen: illegal option -- Y"));
    }
}
