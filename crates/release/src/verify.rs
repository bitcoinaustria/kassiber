//! `verify_download`: authenticate the manifest with a pinned OpenSSH key,
//! then verify the selected artifact's SHA-256. No network access.
use std::io::Read;
use std::path::{Path, PathBuf};

use base64::Engine;
use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use sha2::{Digest, Sha256};

use crate::{splitlines, strip, Result};

pub(crate) const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
pub(crate) const MAX_SIGNATURE_BYTES: u64 = 16 * 1024;
pub(crate) const MAX_PUBLIC_KEY_BYTES: u64 = 16 * 1024;
pub(crate) const NAMESPACE: &str = "kassiber-release";
pub(crate) const PRINCIPAL: &str = "release@kassiber";
pub(crate) const MANIFEST_HEADER: &str = "# Kassiber release manifest v1";
pub(crate) const MANIFEST_VERSION_PREFIX: &str = "# Version: ";
const SIGNATURE_HEADER: &[u8] = b"-----BEGIN SSH SIGNATURE-----";

fn file_error(message: String, path: &Path, label: &str) -> AppError {
    AppError::new("release_verification_file_error", message).with_details(dict([
        ("path", s(path.to_string_lossy().as_ref())),
        ("label", s(label)),
    ]))
}

/// `_open_regular_file`: no final symlink, regular files only.
pub(crate) fn open_regular(path: &Path, label: &str) -> Result<(std::fs::File, u64)> {
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
pub(crate) fn capitalize(label: &str) -> String {
    let mut chars = label.chars();
    match chars.next() {
        Some(first) => {
            first.to_ascii_uppercase().to_string() + &chars.as_str().to_ascii_lowercase()
        }
        None => String::new(),
    }
}

/// `_require_small_regular_file(path, limit, label)`.
pub(crate) fn small_file(path: &Path, limit: u64, label: &str) -> Result<Vec<u8>> {
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
pub(crate) fn expand(value: &str) -> PathBuf {
    kassiber_paths::Context::from_env()
        .ok()
        .and_then(|context| context.expand_user(Path::new(value)).ok())
        .unwrap_or_else(|| PathBuf::from(value))
}

/// `normalize_fingerprint(value)`: a complete `SHA256:` + 43 base64 chars.
pub(crate) fn normalize_fingerprint(value: &str) -> Result<String> {
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
pub(crate) fn key_fields(key_type: &str, encoded: &str) -> Option<Vec<u8>> {
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
pub(crate) fn single_key_line(raw: &[u8], label: &str, code: &str) -> Result<String> {
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
pub(crate) fn parse_public_key(raw: &[u8]) -> Result<(String, Vec<u8>)> {
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

fn signature_failed(expected: &str) -> AppError {
    AppError::new(
        "release_signature_verification_failed",
        "Release manifest has no valid signature from the expected Kassiber release key",
    )
    .with_hint("Do not install or run the release artifact.")
    .with_details(dict([("expected_fingerprint", s(expected))]))
}

/// C `isspace` in the C and UTF-8 locales, which `b64_pton` skips.
fn c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// OpenBSD `b64_pton` as OpenSSH's `sshbuf_b64tod` calls it: whitespace is
/// skipped anywhere, padding is required for a partial quantum, nothing but
/// whitespace may follow it, and the bits it drops must be zero.
fn b64_pton(text: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut state, mut pending) = (0u8, 0u8);
    let mut bytes = text.iter().copied();
    let mut padded = false;
    for byte in bytes.by_ref() {
        if c_space(byte) {
            continue;
        }
        if byte == b'=' {
            padded = true;
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        (state, pending) = match state {
            0 => (1, value << 2),
            1 => {
                out.push(pending | value >> 4);
                (2, (value & 0x0f) << 4)
            }
            2 => {
                out.push(pending | value >> 2);
                (3, (value & 0x03) << 6)
            }
            _ => {
                out.push(pending | value);
                (0, 0)
            }
        };
    }
    if !padded {
        return (state == 0).then_some(out);
    }
    match state {
        2 => {
            // A second '=' must follow, possibly after whitespace.
            if bytes.by_ref().find(|byte| !c_space(*byte)) != Some(b'=') {
                return None;
            }
        }
        3 => {}
        _ => return None,
    }
    (bytes.all(c_space) && pending == 0).then_some(out)
}

/// OpenSSH's `sshsig_dearmor`: the header at offset zero ending in LF or
/// CRLF, the body up to the first `\n-----END SSH SIGNATURE-----` (anything
/// after it is ignored), at most one trailing NUL, then base64.
fn dearmor(armored: &[u8]) -> Option<Vec<u8>> {
    const FOOTER: &[u8] = b"\n-----END SSH SIGNATURE-----";
    let rest = armored.strip_prefix(SIGNATURE_HEADER)?;
    let rest = rest
        .strip_prefix(b"\r\n")
        .or_else(|| rest.strip_prefix(b"\n"))?;
    let end = rest.windows(FOOTER.len()).position(|w| w == FOOTER)?;
    let mut body = &rest[..end];
    // sshbuf_dup_string refuses an embedded NUL unless it is the last byte.
    if let Some(nul) = body.iter().position(|b| *b == 0) {
        if nul + 1 != body.len() {
            return None;
        }
        body = &body[..nul];
    }
    b64_pton(body)
}

/// Big-endian SSH wire fields, read the way `sshbuf_get_*` does.
struct Fields<'a>(&'a [u8]);
impl<'a> Fields<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (head, tail) = (self.0.get(..count)?, self.0.get(count..)?);
        self.0 = tail;
        Some(head)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
    fn string(&mut self) -> Option<&'a [u8]> {
        let length = usize::try_from(self.u32()?).ok()?;
        self.take(length)
    }
    /// `sshbuf_get_cstring`: no embedded NUL.
    fn cstring(&mut self) -> Option<&'a [u8]> {
        self.string().filter(|value| !value.contains(&0))
    }
}

/// `ssh-keygen -Y verify` with the one allowed-signers entry Python writes
/// (`release@kassiber namespaces="kassiber-release" <pinned key>`): OpenSSH's
/// SSHSIG parsing and checks, with ssh-key's signed-data encoding and
/// Ed25519 verification. The pinned blob has already passed `key_fields`.
fn sshsig_verifies(armored: &[u8], message: &[u8], key_type: &str, pinned: &[u8]) -> bool {
    sshsig(armored, message, key_type, pinned).is_some()
}

fn sshsig(armored: &[u8], message: &[u8], key_type: &str, pinned: &[u8]) -> Option<()> {
    use ssh_key::{public::Ed25519PublicKey, Algorithm, HashAlg, Signature, SshSig};
    let blob = dearmor(armored)?;
    let mut sig = Fields(&blob);
    // sshsig_parse_preamble: later versions are refused, version 0 is not.
    if sig.take(6)? != b"SSHSIG" || sig.u32()? > 1 {
        return None;
    }
    let (key, namespace) = (sig.string()?, sig.cstring()?);
    // The reserved field is read and ignored; signed data always uses "".
    let _reserved = sig.string()?;
    let (hash, inner) = (sig.cstring()?, sig.string()?);
    if !sig.0.is_empty() || namespace != NAMESPACE.as_bytes() {
        return None;
    }
    let hash = match hash {
        b"sha256" => HashAlg::Sha256,
        b"sha512" => HashAlg::Sha512,
        _ => return None,
    };
    // sshsig_check_allowed_keys: sshkey_froms refuses trailing data and the
    // key encodings are fixed-width, so equal keys are equal blobs.
    let mut pinned_fields = Fields(pinned);
    let (_, public, application) = (
        pinned_fields.string()?,
        pinned_fields.string()?,
        pinned_fields.string(),
    );
    if key != pinned || application.is_some_and(|app| app.contains(&0)) {
        return None;
    }
    let mut inner = Fields(inner);
    if inner.cstring()? != key_type.as_bytes() {
        return None;
    }
    let raw = inner.string()?;
    let trailer = match application {
        Some(_) => Some(inner.take(5)?),
        None => None,
    };
    if !inner.0.is_empty() || raw.len() != 64 {
        return None;
    }
    let signed = SshSig::signed_data(NAMESPACE, hash, message).ok()?;
    // ssh_ed25519_sk_verify: the authenticator signed SHA-256(application),
    // flags and counter, then SHA-256 of the SSHSIG signed data.
    let payload = match (application, trailer) {
        (Some(application), Some(trailer)) => [
            &Sha256::digest(application)[..],
            trailer,
            &Sha256::digest(&signed)[..],
        ]
        .concat(),
        _ => signed,
    };
    let ed25519 = Signature::new(Algorithm::Ed25519, raw).ok()?;
    let public = Ed25519PublicKey(public.try_into().ok()?);
    signature::Verifier::verify(&public, &payload, &ed25519).ok()
}

/// `verify_signature_bytes(...)`: the pinned fingerprint is the root of trust.
pub fn verify_signature(
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
    if sshsig_verifies(signature, manifest, &key_type, &blob) {
        return Ok(expected);
    }
    Err(signature_failed(&expected))
}

/// `_normalize_release_version(version)`.
pub(crate) fn release_version(version: &str) -> Result<String> {
    let normalized = version.strip_prefix('v').unwrap_or(version);
    if normalized != strip(normalized)
        || normalized.starts_with('v')
        || crate::version::parse_version(normalized).is_none()
    {
        return Err(AppError::new(
            "invalid_release_version",
            format!("Invalid release version: {version}"),
        ));
    }
    Ok(normalized.to_owned())
}

/// `_release_version_from_manifest_name(name)`.
pub(crate) fn manifest_version(name: &str) -> Result<String> {
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
pub(crate) fn manifest_line(line: &str) -> Option<(&str, &str)> {
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
pub(crate) fn manifest_entries(
    raw: &[u8],
    source: &str,
    expected: &str,
) -> Result<Vec<(String, String)>> {
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
pub(crate) fn sha256_file(path: &Path) -> Result<String> {
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
pub(crate) fn name(path: &Path) -> String {
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
    fn manifest_lines_and_versions() {
        let digest = "a".repeat(64);
        assert!(manifest_line(&format!("{digest}  kassiber-1.0.0.dmg")).is_some());
        assert!(manifest_line(&format!("{digest} kassiber.dmg")).is_none());
        assert!(manifest_line(&format!("{digest}  .hidden")).is_none());
        assert_eq!(
            manifest_version("kassiber-v1.2.3-manifest.txt").unwrap(),
            "1.2.3"
        );
        assert!(manifest_version("kassiber-vv1.2.3-manifest.txt").is_err());
    }
}
