//! `--operator-native-auth`: the operator broker's Touch ID helper (lib.rs
//! `run_operator_native_auth_helper`, docs/reference/operator-broker.md).
//!
//! The signed CLI's broker starts the desktop executable as
//! `<bundle>.app/Contents/MacOS/kassiber-ui --operator-native-auth <action> --account <sha256>
//! --ready-fd N --go-fd N [--output-fd N | --secret-fd N]`. It runs before any window exists
//! (`main` checks for it first) and exits with its code:
//! 1. The launch gate: write `R` on the ready pipe, wait for `G` on the go pipe (the broker
//!    checks this helper's live code signature in between).
//! 2. The parent check: the parent must be the bundled, production-signed CLI sidecar
//!    (`Contents/Resources/binaries/kassiber-cli-<arch>-apple-darwin`) with the helper's own
//!    TeamIdentifier, checked statically (`codesign`) and live (Security.framework against a
//!    Developer ID requirement pinned to the sidecar identifier and that team).
//! 3. The action, on the data-protection Keychain's operator item (no preview fallback):
//!    `broker-get` writes the passphrase to the inherited output pipe (exit 4 when none),
//!    `store` reads it from the inherited secret pipe (at most 16 KiB), `delete`, `status`
//!    (exit 0 enrolled, 4 not). Failures exit 1 with `kassiber native auth: <text>` on stderr.
//!
//! Buffers holding the passphrase are cleared before the helper exits. Off macOS the helper
//! answers that Touch ID is unavailable.

/// Runs the helper when the process was started as one; the exit code, else None.
pub fn run_if_requested() -> Option<i32> {
    imp::run()
}

/// Reads the enrollment secret from the broker's pipe: at most 16 KiB, bytes kept exactly.
#[cfg(any(target_os = "macos", test))]
pub fn read_secret(reader: impl std::io::Read) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
    use std::io::Read;
    let mut secret = zeroize::Zeroizing::new(Vec::new());
    if let Err(error) = reader.take(16 * 1024 + 1).read_to_end(&mut secret) {
        return Err(format!("could not read native auth secret: {error}"));
    }
    if secret.len() > 16 * 1024 {
        return Err("native auth secret exceeds 16 KiB".to_string());
    }
    Ok(secret)
}

#[cfg(any(test, target_os = "macos"))]
pub fn sidecar_filename_for_arch(arch: &str) -> Option<&'static str> {
    match arch {
        "aarch64" => Some("kassiber-cli-aarch64-apple-darwin"),
        "x86_64" => Some("kassiber-cli-x86_64-apple-darwin"),
        _ => None,
    }
}

/// The Developer ID requirement the live parent must satisfy.
#[cfg(any(test, target_os = "macos"))]
pub fn sidecar_requirement(identifier: &str, team: &str) -> Result<String, String> {
    if identifier.is_empty()
        || !identifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err("native-auth sidecar signing identifier is invalid".to_string());
    }
    if team.len() != 10
        || !team
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    {
        return Err("native-auth helper has an invalid TeamIdentifier".to_string());
    }
    Ok(format!(
        "anchor apple generic and identifier \"{identifier}\" and certificate leaf[subject.OU] = \"{team}\" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    ))
}

#[cfg(not(target_os = "macos"))]
mod imp {
    pub fn run() -> Option<i32> {
        if std::env::args().nth(1).as_deref() == Some("--operator-native-auth") {
            eprintln!("kassiber native auth: Touch ID is unavailable on this platform");
            Some(1)
        } else {
            None
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::env;
    use std::ffi::OsString;
    use std::fs::File;
    use std::io::{Read, Write};
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStringExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::super::biometric::Native;
    use super::super::store::NativeSecretStore;
    use super::super::touch::Keychain;
    use super::{read_secret, sidecar_filename_for_arch, sidecar_requirement};

    pub fn run() -> Option<i32> {
        let args: Vec<String> = env::args().skip(1).collect();
        if args.first().map(String::as_str) != Some("--operator-native-auth") {
            return None;
        }
        let result = (|| -> Result<i32, String> {
            verify_parent()?;
            wait_for_broker_release(&args)?;
            let action = args
                .get(1)
                .map(String::as_str)
                .ok_or("missing native auth action")?;
            let account = flag(&args, "--account").ok_or("missing native auth account")?;
            let kc = Keychain {
                store: &NativeSecretStore,
                bio: &Native,
                macos: true,
            };
            match action {
                "broker-get" => {
                    let output_fd = fd(&args, "--output-fd")?;
                    if output_fd <= 2 {
                        return Err("native auth output must be an inherited pipe".to_string());
                    }
                    let Some(secret) = kc.operator_get(account)? else {
                        return Ok(4);
                    };
                    // The Zeroizing buffer clears itself when it goes out of scope.
                    unsafe { File::from_raw_fd(output_fd) }
                        .write_all(&secret)
                        .map_err(|e| format!("could not write native auth result: {e}"))?;
                    Ok(0)
                }
                "store" => {
                    let secret_fd = fd(&args, "--secret-fd")?;
                    let secret = read_secret(unsafe { File::from_raw_fd(secret_fd) })?;
                    kc.operator_store(account, &secret)?;
                    Ok(0)
                }
                "delete" => {
                    kc.operator_delete(account)?;
                    Ok(0)
                }
                "status" => Ok(if kc.operator_configured(account)? {
                    0
                } else {
                    4
                }),
                _ => Err("unsupported native auth action".to_string()),
            }
        })();
        Some(match result {
            Ok(code) => code,
            Err(error) => {
                eprintln!("kassiber native auth: {error}");
                1
            }
        })
    }

    fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
        args.iter()
            .position(|v| v == name)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    fn fd(args: &[String], name: &str) -> Result<i32, String> {
        flag(args, name)
            .ok_or_else(|| format!("missing {name}"))?
            .parse::<i32>()
            .map_err(|_| format!("invalid {name}"))
    }

    fn wait_for_broker_release(args: &[String]) -> Result<(), String> {
        let ready_fd = fd(args, "--ready-fd")?;
        let go_fd = fd(args, "--go-fd")?;
        if ready_fd <= 2 || go_fd <= 2 || ready_fd == go_fd {
            return Err("native auth launch gate requires inherited pipes".to_string());
        }
        let mut ready = unsafe { File::from_raw_fd(ready_fd) };
        ready
            .write_all(b"R")
            .map_err(|e| format!("could not signal native auth readiness: {e}"))?;
        drop(ready);
        let mut go = unsafe { File::from_raw_fd(go_fd) };
        let mut signal = [0_u8; 1];
        go.read_exact(&mut signal)
            .map_err(|e| format!("could not receive native auth release: {e}"))?;
        if signal != *b"G" {
            return Err("native auth launch gate received an invalid release".to_string());
        }
        Ok(())
    }

    fn verify_parent() -> Result<(), String> {
        unsafe extern "C" {
            fn getppid() -> i32;
        }
        let parent_pid = unsafe { getppid() };
        let parent_path = parent_path(parent_pid)?
            .canonicalize()
            .map_err(|e| format!("could not resolve native-auth parent executable: {e}"))?;
        let helper_path = env::current_exe()
            .map_err(|e| format!("could not resolve native-auth helper: {e}"))?
            .canonicalize()
            .map_err(|e| format!("could not canonicalize native-auth helper: {e}"))?;
        let bundle_root = helper_path
            .ancestors()
            .find(|p| p.extension().and_then(|v| v.to_str()) == Some("app"))
            .ok_or("native-auth helper is not inside a signed app bundle")?;
        let sidecar_root = bundle_root
            .join("Contents")
            .join("Resources")
            .join("binaries");
        let parent_name = parent_path
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or_default();
        let expected = sidecar_filename_for_arch(env::consts::ARCH)
            .ok_or("native-auth helper does not support this macOS architecture")?;
        if !parent_path.starts_with(&sidecar_root) || parent_name != expected {
            return Err(
                "native-auth helper parent is not the bundled Kassiber sidecar".to_string(),
            );
        }
        let (_, helper_team) = codesign_identity(&helper_path)?;
        let (parent_identifier, parent_team) = codesign_identity(&parent_path)?;
        if helper_team != parent_team {
            return Err("native-auth helper and broker signatures do not match".to_string());
        }
        if parent_identifier != expected {
            return Err(
                "native-auth helper parent has an unexpected signing identifier".to_string(),
            );
        }
        let requirement = sidecar_requirement(expected, &helper_team)?;
        verify_running_parent(parent_pid, &requirement)
    }

    fn parent_path(parent_pid: i32) -> Result<PathBuf, String> {
        unsafe extern "C" {
            fn proc_pidpath(pid: i32, buffer: *mut std::ffi::c_void, buffersize: u32) -> i32;
        }
        let mut buffer = vec![0_u8; 4096];
        let length =
            unsafe { proc_pidpath(parent_pid, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if length <= 0 {
            return Err("could not inspect native-auth parent process".to_string());
        }
        buffer.truncate(length as usize);
        while buffer.last() == Some(&0) {
            buffer.pop();
        }
        Ok(PathBuf::from(OsString::from_vec(buffer)))
    }

    fn verify_running_parent(parent_pid: i32, requirement_text: &str) -> Result<(), String> {
        use core_foundation::base::{CFRelease, TCFType};
        use core_foundation::dictionary::CFDictionary;
        use core_foundation::number::CFNumber;
        use core_foundation::string::CFString;
        use security_framework_sys::code_signing::{
            SecCodeCheckValidity, SecCodeCopyGuestWithAttributes, SecCodeRef,
            SecRequirementCreateWithString, SecRequirementRef, kSecGuestAttributePid,
        };
        use std::ptr;

        let pid_key = unsafe { CFString::wrap_under_get_rule(kSecGuestAttributePid) };
        let pid_value = CFNumber::from(parent_pid);
        let attributes = CFDictionary::from_CFType_pairs(&[(pid_key, pid_value)]);
        let mut running_code: SecCodeRef = ptr::null_mut();
        let guest_status = unsafe {
            SecCodeCopyGuestWithAttributes(
                ptr::null_mut(),
                attributes.as_concrete_TypeRef(),
                0,
                &mut running_code,
            )
        };
        if guest_status != 0 || running_code.is_null() {
            return Err(format!(
                "could not resolve the running native-auth parent code ({guest_status})"
            ));
        }
        let requirement_string = CFString::new(requirement_text);
        let mut requirement: SecRequirementRef = ptr::null_mut();
        let requirement_status = unsafe {
            SecRequirementCreateWithString(
                requirement_string.as_concrete_TypeRef(),
                0,
                &mut requirement,
            )
        };
        if requirement_status != 0 || requirement.is_null() {
            unsafe { CFRelease(running_code.cast()) };
            return Err(format!(
                "could not compile the native-auth sidecar requirement ({requirement_status})"
            ));
        }
        let validity_status = unsafe { SecCodeCheckValidity(running_code, 0, requirement) };
        unsafe {
            CFRelease(requirement.cast());
            CFRelease(running_code.cast());
        }
        if validity_status != 0 {
            return Err(format!(
                "running native-auth parent did not match the bundled sidecar requirement ({validity_status})"
            ));
        }
        Ok(())
    }

    /// (Identifier, TeamIdentifier) of a strictly valid signature; ad-hoc or unsigned code fails.
    fn codesign_identity(path: &Path) -> Result<(String, String), String> {
        let verified = Command::new("/usr/bin/codesign")
            .args(["--verify", "--strict"])
            .arg(path)
            .output()
            .map_err(|e| format!("could not verify native-auth code signature: {e}"))?;
        if !verified.status.success() {
            return Err("native-auth code signature verification failed".to_string());
        }
        let details = Command::new("/usr/bin/codesign")
            .args(["-dv", "--verbose=4"])
            .arg(path)
            .output()
            .map_err(|e| format!("could not inspect native-auth code signature: {e}"))?;
        if !details.status.success() {
            return Err("native-auth code signature inspection failed".to_string());
        }
        let stderr = String::from_utf8_lossy(&details.stderr);
        let identifier = stderr
            .lines()
            .find_map(|l| l.strip_prefix("Identifier="))
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .ok_or_else(|| "native-auth code signature has no Identifier".to_string())?;
        let team = stderr
            .lines()
            .find_map(|l| l.strip_prefix("TeamIdentifier="))
            .filter(|t| !t.is_empty() && *t != "not set")
            .map(str::to_owned)
            .ok_or_else(|| "native-auth helper requires a production TeamIdentifier".to_string())?;
        Ok((identifier, team))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn pipe_preserves_normalized_secret_bytes() {
        let secret = b"passphrase-with-significant-newline\n";
        assert_eq!(read_secret(Cursor::new(secret)).unwrap().as_slice(), secret);
    }

    #[test]
    fn pipe_refuses_more_than_16_kib() {
        assert!(read_secret(Cursor::new(vec![b'a'; 16 * 1024])).is_ok());
        assert_eq!(
            read_secret(Cursor::new(vec![b'a'; 16 * 1024 + 1])).unwrap_err(),
            "native auth secret exceeds 16 KiB"
        );
    }

    #[test]
    fn sidecar_identity_is_architecture_specific() {
        assert_eq!(
            sidecar_filename_for_arch("aarch64"),
            Some("kassiber-cli-aarch64-apple-darwin")
        );
        assert_eq!(
            sidecar_filename_for_arch("x86_64"),
            Some("kassiber-cli-x86_64-apple-darwin")
        );
        assert_eq!(sidecar_filename_for_arch("riscv64"), None);
    }

    #[test]
    fn sidecar_requirement_pins_identifier_team_and_distribution() {
        let r = sidecar_requirement("kassiber-cli-aarch64-apple-darwin", "A1B2C3D4E5").unwrap();
        assert!(r.contains("identifier \"kassiber-cli-aarch64-apple-darwin\""));
        assert!(r.contains("leaf[subject.OU] = \"A1B2C3D4E5\""));
        assert!(r.contains("1.2.840.113635.100.6.1.13"));
        assert!(sidecar_requirement("kassiber cli", "A1B2C3D4E5").is_err());
        assert!(sidecar_requirement("kassiber-cli", "a1b2c3d4e5").is_err());
        assert!(sidecar_requirement("kassiber-cli", "SHORT").is_err());
    }
}
