//! Portable identity vectors only. No claim of macOS code-signing validation.
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn digest(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn requirement(identifier: &str, team: &str) -> Result<String, &'static str> {
    if identifier.is_empty()
        || !identifier
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err("invalid identifier");
    }
    if team.len() != 10
        || !team
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return Err("invalid team");
    }
    Ok(format!(
        "anchor apple generic and identifier \"{identifier}\" and certificate leaf[subject.OU] = \"{team}\" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    ))
}
fn sidecar(arch: &str) -> Option<&'static str> {
    match arch {
        "aarch64" => Some("kassiber-cli-aarch64-apple-darwin"),
        "x86_64" => Some("kassiber-cli-x86_64-apple-darwin"),
        _ => None,
    }
}
fn stable_location(executable: &Path) -> std::io::Result<PathBuf> {
    let executable = executable.canonicalize()?;
    if let (Some(appdir), Some(appimage)) =
        (std::env::var_os("APPDIR"), std::env::var_os("APPIMAGE"))
    {
        if executable.starts_with(Path::new(&appdir).canonicalize()?) {
            return Path::new(&appimage).canonicalize();
        }
    }
    Ok(executable)
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    assert_eq!(
        args.len(),
        3,
        "canonical executable/location and helper cdhash"
    );
    let id = "at.bitcoinaustria.kassiber";
    let team = "6Q4R2C3GJK";
    let cdhash = &args[2];
    assert!(
        [40, 64].contains(&cdhash.len())
            && cdhash
                .bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    let location = stable_location(Path::new(&args[1])).unwrap();
    println!(
        "{}",
        json!({"build":{"version":"0.22.78","channel":"prerelease","commit":"8709d4304c3c","frozen":true,"origin":&digest(&location.to_string_lossy())[..12]},"helper_opaque":digest(&format!("kassiber-native-auth-signature-v1:{id}:{team}:{cdhash}")),"helper_requirement":requirement(id,team).unwrap(),"arm_requirement":requirement(sidecar("aarch64").unwrap(),team).unwrap(),"intel_requirement":requirement(sidecar("x86_64").unwrap(),team).unwrap()})
    );
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_identifiers() {
        assert!(requirement("identifier\" or true", "6Q4R2C3GJK").is_err());
        assert!(requirement("valid", "adhoc").is_err());
        assert!(requirement("valid", "6q4r2c3gjk").is_err());
        assert!(sidecar("arm").is_none());
    }
}
