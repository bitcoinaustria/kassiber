//! Release publication: the deterministic SHA-256 manifest, its OpenSSH
//! signature through the agent, the code-reviewed signing policy, and the
//! checks that a release directory matches its authenticated manifest.
//! Ported from `scripts/release_manifest.py` and the parts of
//! `kassiber/release_verification.py` it used; `kassiber-release manifest`
//! prints what that script printed.
//!
//! One deliberate difference: signatures are checked in process, as
//! `kassiber verify-download` does (`verify::verify_signature`), instead of
//! through `ssh-keygen -Y verify`. Signing still asks `ssh-keygen -Y sign`,
//! because the release key lives in an SSH agent.
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;

use crate::verify::{
    expand, fingerprint, key_fields, manifest_entries, manifest_line, manifest_version, name,
    normalize_fingerprint, parse_public_key, release_version, sha256_file, single_key_line,
    small_file, verify_signature, MANIFEST_HEADER, MANIFEST_VERSION_PREFIX, MAX_MANIFEST_BYTES,
    MAX_PUBLIC_KEY_BYTES, MAX_SIGNATURE_BYTES, NAMESPACE, PRINCIPAL,
};
use crate::{strip, Result};

pub const SIGNATURE_SUFFIX: &str = ".sig";
const MAX_SIGNING_POLICY_BYTES: u64 = 64 * 1024;
const SIGNING_POLICY_SCHEMA_VERSION: i64 = 2;
/// Generous: the Bitwarden SSH agent waits for a per-use approval click.
const SIGNING_TIMEOUT: Duration = Duration::from_secs(600);

/// `str(Path(value))`: collapse repeated separators and `.` segments and
/// drop a trailing separator, as `pathlib` does; `..` stays.
pub fn py_path(value: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in value.components() {
        match component {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// `Path(value).expanduser()` with `pathlib`'s normalization.
pub fn user_path(value: &str) -> PathBuf {
    py_path(&expand(value))
}

/// `Path.resolve()` (non-strict): follow symlinks of the existing prefix,
/// then apply the rest lexically.
pub fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut resolved = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => resolved.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            Component::Normal(part) => {
                resolved.push(part);
                if let Ok(real) = std::fs::canonicalize(&resolved) {
                    resolved = real;
                }
            }
        }
    }
    resolved
}

fn text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn file_error(message: String) -> AppError {
    AppError::new("release_verification_file_error", message)
}

/// `release_manifest_name(version)`.
pub fn manifest_name(version: &str) -> Result<String> {
    Ok(format!(
        "kassiber-{}-manifest.txt",
        release_version(version)?
    ))
}

/// `_parse_release_manifest_bytes`: the entries, never empty.
fn entries(raw: &[u8], source: &str, expected: &str) -> Result<Vec<(String, String)>> {
    let entries = manifest_entries(raw, source, expected)?;
    if entries.is_empty() {
        return Err(
            AppError::new("invalid_release_manifest", "Release manifest is empty")
                .with_details(dict([("path", s(source))])),
        );
    }
    Ok(entries)
}

/// `parse_release_manifest(path)`.
pub fn parse_manifest(path: &Path) -> Result<Vec<(String, String)>> {
    let raw = small_file(path, MAX_MANIFEST_BYTES, "release manifest")?;
    entries(&raw, &text(path), &manifest_version(&name(path))?)
}

/// `generate_release_manifest(release_dir, version, excluded_names)`.
pub fn generate(directory: &Path, version: &str, excluded: &[String]) -> Result<PathBuf> {
    if !directory.is_dir() {
        return Err(file_error(format!(
            "Release directory does not exist: {}",
            text(directory)
        )));
    }
    let output = directory.join(manifest_name(version)?);
    let output_name = name(&output);
    let signature_name = format!("{output_name}{SIGNATURE_SUFFIX}");
    let mut artifacts = Vec::new();
    let listing = std::fs::read_dir(directory).map_err(|_| {
        file_error(format!(
            "Release directory does not exist: {}",
            text(directory)
        ))
    })?;
    for candidate in listing {
        let candidate = candidate
            .map_err(|_| {
                file_error(format!(
                    "Could not inspect release directory: {}",
                    text(directory)
                ))
            })?
            .path();
        let candidate_name = name(&candidate);
        if excluded.contains(&candidate_name)
            || candidate_name == output_name
            || candidate_name == signature_name
        {
            continue;
        }
        if manifest_line(&format!("{}  {candidate_name}", "0".repeat(64))).is_none() {
            return Err(AppError::new(
                "invalid_release_artifact_name",
                format!("Release artifact filename is not manifest-safe: {candidate_name}"),
            ));
        }
        if candidate.is_symlink() || !candidate.is_file() {
            return Err(file_error(format!(
                "Release artifact is not a regular non-symlink file: {candidate_name}"
            )));
        }
        artifacts.push((candidate_name, candidate));
    }
    if artifacts.is_empty() {
        return Err(AppError::new(
            "empty_release_artifact_set",
            "No release artifacts found",
        ));
    }
    let normalized = manifest_version(&output_name)?;
    artifacts.sort_by(|a, b| a.0.cmp(&b.0));
    let mut lines = format!("{MANIFEST_HEADER}\n{MANIFEST_VERSION_PREFIX}{normalized}\n");
    for (artifact, path) in &artifacts {
        lines.push_str(&format!("{}  {artifact}\n", sha256_file(path)?));
    }
    let temporary = output.with_file_name(format!(".{output_name}.{}.tmp", std::process::id()));
    let written = (|| -> std::io::Result<()> {
        let mut handle = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        handle.write_all(lines.as_bytes())?;
        handle.flush()?;
        handle.sync_all()?;
        std::fs::rename(&temporary, &output)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(file_error(format!(
            "Could not write release manifest: {}",
            text(&output)
        )));
    }
    Ok(output)
}

/// `_verify_release_artifact_entries`.
fn verify_entries(
    release_dir: &Path,
    entries: &[(String, String)],
    manifest_name: &str,
    require_complete: bool,
) -> Result<Vec<(&'static str, Value)>> {
    if release_dir.is_symlink() || !release_dir.is_dir() {
        return Err(file_error(format!(
            "Release directory does not exist: {}",
            text(release_dir)
        )));
    }
    let metadata = [
        manifest_name.to_owned(),
        format!("{manifest_name}{SIGNATURE_SUFFIX}"),
    ];
    let listing = std::fs::read_dir(release_dir).map_err(|_| {
        file_error(format!(
            "Could not inspect release directory: {}",
            text(release_dir)
        ))
    })?;
    let mut actual = std::collections::BTreeSet::new();
    for candidate in listing {
        let candidate = candidate
            .map_err(|_| {
                file_error(format!(
                    "Could not inspect release directory: {}",
                    text(release_dir)
                ))
            })?
            .path();
        let candidate_name = name(&candidate);
        if metadata.contains(&candidate_name) {
            continue;
        }
        if candidate.is_symlink() || !candidate.is_file() {
            return Err(AppError::new(
                "release_artifact_set_mismatch",
                format!("Unexpected non-regular release asset: {candidate_name}"),
            )
            .with_hint("Do not publish this release."));
        }
        actual.insert(candidate_name);
    }
    let expected: std::collections::BTreeSet<String> =
        entries.iter().map(|(file, _)| file.clone()).collect();
    let missing: Vec<Value> = expected
        .difference(&actual)
        .map(|n| s(n.as_str()))
        .collect();
    let unexpected: Vec<Value> = actual
        .difference(&expected)
        .map(|n| s(n.as_str()))
        .collect();
    if !unexpected.is_empty() || (require_complete && !missing.is_empty()) || actual.is_empty() {
        let message = if require_complete {
            "Release assets do not exactly match the authenticated manifest"
        } else {
            "Release assets are not a non-empty subset of the authenticated manifest"
        };
        return Err(AppError::new("release_artifact_set_mismatch", message)
            .with_hint("Do not publish this release.")
            .with_details(dict([
                ("missing", Value::List(missing)),
                ("unexpected", Value::List(unexpected)),
            ])));
    }
    for filename in &actual {
        let digest = &entries.iter().find(|(file, _)| file == filename).unwrap().1;
        let observed = sha256_file(&release_dir.join(filename))?;
        if observed != *digest {
            return Err(AppError::new(
                "release_artifact_hash_mismatch",
                format!("SHA-256 mismatch for {filename}"),
            )
            .with_hint("Do not install or run this file. Delete it and download the release again.")
            .with_details(dict([
                ("artifact", s(filename.as_str())),
                ("expected_sha256", s(digest.as_str())),
                ("actual_sha256", s(observed)),
            ])));
        }
    }
    Ok(vec![
        ("verified", Value::Bool(true)),
        ("artifact_count", Value::Int(actual.len().into())),
        ("complete", Value::Bool(require_complete)),
        ("manifest", s(manifest_name)),
    ])
}

fn same_dir(a: &Path, b: &Path) -> bool {
    let parent = a.parent().map(|p| {
        if p.as_os_str().is_empty() {
            Path::new(".")
        } else {
            p
        }
    });
    parent.is_some_and(|parent| resolve(parent) == resolve(b))
}

/// `verify_release_artifacts(release_dir, manifest, require_complete)`.
pub fn verify_artifacts(
    release_dir: &Path,
    manifest: &Path,
    require_complete: bool,
) -> Result<Vec<(&'static str, Value)>> {
    if !same_dir(manifest, release_dir) {
        return Err(AppError::new(
            "release_artifact_set_mismatch",
            "Release manifest must be inside the release directory",
        ));
    }
    let raw = small_file(manifest, MAX_MANIFEST_BYTES, "release manifest")?;
    let manifest_file = name(manifest);
    let entries = entries(&raw, &text(manifest), &manifest_version(&manifest_file)?)?;
    verify_entries(release_dir, &entries, &manifest_file, require_complete)
}

fn signature_result(signer: String) -> [(&'static str, Value); 3] {
    [
        ("signer_fingerprint", s(signer)),
        ("signer_principal", s(PRINCIPAL)),
        ("signature_namespace", s(NAMESPACE)),
    ]
}

/// `verify_release_directory(...)`: authenticate the manifest, then the set.
pub fn verify_release(
    release_dir: &Path,
    manifest: &Path,
    signature: &Path,
    public_key: &Path,
    expected: &str,
    require_complete: bool,
) -> Result<Vec<(&'static str, Value)>> {
    if !(same_dir(manifest, release_dir) && same_dir(signature, release_dir)) {
        return Err(AppError::new(
            "release_artifact_set_mismatch",
            "Release manifest and signature must be inside the release directory",
        ));
    }
    let manifest_file = name(manifest);
    if name(signature) != format!("{manifest_file}{SIGNATURE_SUFFIX}") {
        return Err(AppError::new(
            "release_artifact_set_mismatch",
            "Release signature filename does not match the manifest",
        ));
    }
    let manifest_bytes = small_file(manifest, MAX_MANIFEST_BYTES, "release manifest")?;
    let signature_bytes = small_file(signature, MAX_SIGNATURE_BYTES, "release signature")?;
    let public_key_bytes = small_file(public_key, MAX_PUBLIC_KEY_BYTES, "release public key")?;
    let signer = verify_signature(
        &manifest_bytes,
        &signature_bytes,
        &public_key_bytes,
        expected,
    )?;
    let entries = entries(
        &manifest_bytes,
        &text(manifest),
        &manifest_version(&manifest_file)?,
    )?;
    let mut result = verify_entries(release_dir, &entries, &manifest_file, require_complete)?;
    result.push(("signature", s(name(signature))));
    result.extend(signature_result(signer));
    Ok(result)
}

fn policy_error(message: impl Into<String>) -> AppError {
    AppError::new("invalid_release_signing_policy", message)
}

/// `_policy_path(root, value, label)`.
fn policy_path(root: &Path, value: &str, label: &str) -> Result<PathBuf> {
    let relative = Path::new(value);
    if relative.is_absolute() {
        return Err(policy_error(format!(
            "Release {label} path must be repository-relative"
        )));
    }
    let resolved = resolve(&root.join(relative));
    if !resolved.starts_with(root) {
        return Err(policy_error(format!(
            "Release {label} path escapes the repository"
        )));
    }
    Ok(resolved)
}

/// `_parse_allowed_signers(raw)`.
fn allowed_signers(raw: &[u8]) -> Result<(String, Vec<u8>)> {
    let line = single_key_line(
        raw,
        "release allowed_signers file",
        "invalid_release_signing_policy",
    )?;
    let tokens: Vec<&str> = line
        .split(crate::is_space)
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() < 4
        || tokens[0] != PRINCIPAL
        || tokens[1] != format!("namespaces=\"{NAMESPACE}\"")
    {
        return Err(policy_error(
            "Release allowed_signers entry must name only the release principal and namespace",
        ));
    }
    key_fields(tokens[2], tokens[3])
        .map(|blob| (tokens[2].to_owned(), blob))
        .ok_or_else(|| {
            policy_error("Release allowed_signers entry has no valid OpenSSH Ed25519 key")
        })
}

fn field<'a>(payload: &'a Value, key: &str) -> Option<&'a Value> {
    kassiber_envelope::get(payload, key)
}

fn nonempty_str(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::Str(text)) => text.as_str().filter(|t| !t.is_empty()).map(str::to_owned),
        _ => None,
    }
}

/// `load_release_signing_policy(path, repository_root, require_enabled)`.
pub fn load_policy(
    path: &Path,
    repository_root: &Path,
    require_enabled: bool,
) -> Result<Vec<(&'static str, Value)>> {
    let raw = small_file(path, MAX_SIGNING_POLICY_BYTES, "release signing policy")?;
    let payload = std::str::from_utf8(&raw)
        .ok()
        .and_then(|text| kassiber_pytext::loads(text).ok())
        .ok_or_else(|| policy_error("Release signing policy is not valid JSON"))?;
    let schema_ok = matches!(&payload, Value::Dict(_))
        && matches!(field(&payload, "schema_version"), Some(Value::Int(n)) if *n == SIGNING_POLICY_SCHEMA_VERSION.into());
    if !schema_ok {
        return Err(policy_error(
            "Release signing policy has no supported schema",
        ));
    }
    let enabled = match field(&payload, "enabled") {
        Some(Value::Bool(enabled)) => Some(*enabled),
        _ => None,
    };
    let fingerprint_value = match field(&payload, "fingerprint") {
        Some(Value::Str(text)) => text.as_str().map(str::to_owned),
        _ => None,
    };
    let public_key_value = nonempty_str(field(&payload, "public_key_path"));
    let allowed_value = nonempty_str(field(&payload, "allowed_signers_path"));
    let (Some(enabled), Some(fingerprint_value), Some(public_key_value), Some(allowed_value)) =
        (enabled, fingerprint_value, public_key_value, allowed_value)
    else {
        return Err(policy_error("Release signing policy fields are invalid"));
    };
    if field(&payload, "namespace") != Some(&s(NAMESPACE))
        || field(&payload, "principal") != Some(&s(PRINCIPAL))
    {
        return Err(policy_error(
            "Release signing policy must use the Kassiber release namespace and principal",
        )
        .with_details(dict([
            ("expected_namespace", s(NAMESPACE)),
            ("expected_principal", s(PRINCIPAL)),
        ])));
    }
    let root = resolve(&user_path(&repository_root.to_string_lossy()));
    let public_key_path = policy_path(&root, &public_key_value, "public key")?;
    let allowed_path = policy_path(&root, &allowed_value, "allowed_signers")?;
    let mut result = vec![
        ("enabled", Value::Bool(enabled)),
        ("fingerprint", s("")),
        ("namespace", s(NAMESPACE)),
        ("principal", s(PRINCIPAL)),
        ("public_key_path", s(text(&public_key_path))),
        ("allowed_signers_path", s(text(&allowed_path))),
    ];
    if !enabled {
        if !strip(&fingerprint_value).is_empty() {
            return Err(policy_error(
                "Disabled release signing policy must not carry a fingerprint",
            ));
        }
        if require_enabled {
            return Err(AppError::new(
                "release_signing_not_enabled",
                "Signed release publication is not enabled",
            )
            .with_hint(
                "Publish the release key and enable the code-reviewed signing policy first.",
            ));
        }
        return Ok(result);
    }
    let normalized = normalize_fingerprint(&fingerprint_value)?;
    let (public_type, public_blob) = parse_public_key(&small_file(
        &public_key_path,
        MAX_PUBLIC_KEY_BYTES,
        "release public key",
    )?)?;
    if fingerprint(&public_blob) != normalized {
        return Err(policy_error(
            "Release public key does not match the pinned policy fingerprint",
        )
        .with_details(dict([("expected_fingerprint", s(normalized.as_str()))])));
    }
    let (allowed_type, allowed_blob) = allowed_signers(&small_file(
        &allowed_path,
        MAX_PUBLIC_KEY_BYTES,
        "release allowed_signers file",
    )?)?;
    if allowed_type != public_type || allowed_blob != public_blob {
        return Err(policy_error(
            "Release allowed_signers key does not match the pinned public key",
        )
        .with_details(dict([("expected_fingerprint", s(normalized.as_str()))])));
    }
    result[1].1 = s(normalized);
    Ok(result)
}

/// `ssh_keygen_command(executable)`.
fn ssh_keygen(executable: Option<&str>) -> Result<PathBuf> {
    match executable {
        Some(path) if !path.is_empty() => Ok(PathBuf::from(path)),
        _ => which("ssh-keygen").ok_or_else(|| {
            AppError::new(
                "ssh_keygen_unavailable",
                "OpenSSH's ssh-keygen is required to verify this release signature",
            )
            .with_hint("Install OpenSSH 8.1 or newer, then run kassiber verify-download again.")
        }),
    }
}

/// `shutil.which(name)`.
fn which(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                candidate.is_file()
                    && candidate
                        .metadata()
                        .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
            }
            #[cfg(not(unix))]
            {
                candidate.is_file()
            }
        })
}

/// `ssh_keygen_output_is_unsupported(output)`.
fn keygen_unsupported(output: &str) -> bool {
    let lower = output.to_lowercase();
    lower.contains("usage: ssh-keygen")
        || ["unknown", "illegal", "invalid", "unrecognized"]
            .iter()
            .any(|word| lower.contains(&format!("{word} option")))
}

fn signing_failed(message: &str) -> AppError {
    AppError::new("release_signing_failed", message)
}

/// `sign_manifest(manifest, public_key, expected_fingerprint, ...)`.
pub fn sign(
    manifest: &Path,
    public_key: &Path,
    expected_fingerprint: &str,
    output: Option<&Path>,
    ssh_keygen_executable: Option<&str>,
    overwrite: bool,
) -> Result<PathBuf> {
    let expected = normalize_fingerprint(expected_fingerprint)?;
    let public_key_bytes = small_file(public_key, MAX_PUBLIC_KEY_BYTES, "release public key")?;
    if fingerprint(&parse_public_key(&public_key_bytes)?.1) != expected {
        return Err(AppError::new(
            "release_fingerprint_mismatch",
            "Release public key does not match the pinned policy fingerprint",
        )
        .with_hint("Select the Kassiber release public key named in the signing policy.")
        .with_details(dict([("expected_fingerprint", s(expected.as_str()))])));
    }
    parse_manifest(manifest)?;
    let manifest_bytes = small_file(manifest, MAX_MANIFEST_BYTES, "release manifest")?;
    let signature = match output {
        Some(path) => path.to_path_buf(),
        None => manifest.with_file_name(format!("{}{SIGNATURE_SUFFIX}", name(manifest))),
    };
    let exists = || {
        AppError::new(
            "release_signature_exists",
            format!("Signature already exists: {}", text(&signature)),
        )
        .with_hint("Pass --overwrite only when intentionally replacing this signature.")
    };
    if signature.exists() && !overwrite {
        return Err(exists());
    }
    let keygen = ssh_keygen(ssh_keygen_executable)?;
    // -U makes ssh-keygen use only the agent key: it must never fall back to
    // a private-key file that happens to sit next to the release .pub. The
    // manifest goes over stdin and the signature comes back on stdout, so
    // ssh-keygen never names, prompts for, or overwrites a file itself.
    let spawned = Command::new(&keygen)
        .args(["-Y", "sign", "-n", NAMESPACE, "-U", "-f"])
        .arg(public_key)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child =
        spawned.map_err(|_| signing_failed("ssh-keygen could not sign the release manifest"))?;
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&manifest_bytes);
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < SIGNING_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(50))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(signing_failed(
                    "ssh-keygen could not sign the release manifest",
                ));
            }
        }
    };
    let _ = writer.join();
    let read_all = |stream: Option<&mut dyn std::io::Read>| {
        let mut bytes = Vec::new();
        if let Some(stream) = stream {
            let _ = stream.read_to_end(&mut bytes);
        }
        bytes
    };
    let signature_bytes = read_all(child.stdout.as_mut().map(|s| s as &mut dyn std::io::Read));
    let stderr = read_all(child.stderr.as_mut().map(|s| s as &mut dyn std::io::Read));
    if !status.success() || signature_bytes.is_empty() {
        let stderr = &stderr[..stderr.len().min(4096)];
        if keygen_unsupported(&String::from_utf8_lossy(stderr)) {
            return Err(AppError::new(
                "ssh_keygen_unsupported",
                "This ssh-keygen cannot create OpenSSH signatures",
            )
            .with_hint("Install OpenSSH 8.1 or newer."));
        }
        return Err(signing_failed("ssh-keygen could not sign the release manifest")
            .with_hint("Check that SSH_AUTH_SOCK points at the agent holding the release key and approve the request."));
    }
    if signature_bytes.len() as u64 > MAX_SIGNATURE_BYTES {
        return Err(signing_failed(
            "ssh-keygen produced an unexpectedly large signature",
        ));
    }
    let manifest_bytes = small_file(manifest, MAX_MANIFEST_BYTES, "release manifest")?;
    if let Err(error) = verify_signature(
        &manifest_bytes,
        &signature_bytes,
        &public_key_bytes,
        &expected,
    ) {
        return Err(signing_failed(
            "The new release signature did not verify against the pinned fingerprint",
        )
        .with_details(dict([
            ("expected_fingerprint", s(expected.as_str())),
            ("verification_error", s(error.code.as_str())),
        ])));
    }
    let temporary =
        signature.with_file_name(format!(".{}.{}.tmp", name(&signature), std::process::id()));
    let _ = std::fs::remove_file(&temporary);
    let write_failed = || {
        signing_failed(&format!(
            "Could not write release signature: {}",
            text(&signature)
        ))
    };
    let written = (|| -> std::io::Result<()> {
        let mut handle = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        handle.write_all(&signature_bytes)?;
        handle.flush()?;
        handle.sync_all()
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(write_failed());
    }
    let installed = if overwrite {
        std::fs::rename(&temporary, &signature).map_err(|_| write_failed())
    } else {
        // Atomic no-clobber publication: fails if the signature appeared meanwhile.
        match std::fs::hard_link(&temporary, &signature) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Err(exists()),
            Err(_) => Err(write_failed()),
        }
    };
    let _ = std::fs::remove_file(&temporary);
    installed?;
    Ok(signature)
}
