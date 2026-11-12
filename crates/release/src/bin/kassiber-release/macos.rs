//! `kassiber-release macos sign|notarize|verify|prepare`: local Developer ID
//! signing and keyless CI notarization of one sealed app, ported from
//! `scripts/macos_release.py` and `scripts/prepare_macos_release.py`.
//!
//! No package code is executed while signing. Secrets are read by Apple's
//! tools from the local keychain, never accepted as command-line passwords.
//! Apple's tools run exactly as the scripts ran them; the checks around them
//! (archive members, provenance, Mach-O linkage, entitlements) are the
//! scripts' checks. The CLI `.tar.gz` comes from the `tar` crate rather than
//! Python's `tarfile`, so its bytes differ (Python's gzip header carried the
//! creation time, so they were never reproducible); `verify` checks its
//! members against the sealed app as before.
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant, SystemTime};

use kassiber_envelope::{dict, s};
use kassiber_pyjson::{Options, Value as Json};
use sha2::{Digest, Sha256};

use crate::args::{flag, parse, required, value, Spec, Usage};
use crate::usage;

pub const TEAM: &str = "6Q4R2C3GJK";
pub const APP_ID: &str = "at.bitcoinaustria.kassiber";
const APP_ZIP: &str = "kassiber-macos-arm64.app.zip";
const DMG: &str = "kassiber-macos-arm64.dmg";
const CLI_TAR: &str = "kassiber-cli-macos-arm64.tar.gz";
const INPUT_DMG: &str = "kassiber-macos-signing-input.dmg";
// The backend sidecar keeps the Python sidecar's file name, bundle location
// and signing identifier (the file name, which `sign` passes as
// --identifier). The Touch ID helper's code requirement pins that identifier
// and the team, and Keychain items the signed Python sidecar wrote stay
// readable without a prompt only for the same identifier and team.
const SIDECAR: &str = "Contents/Resources/binaries/kassiber-cli/kassiber-cli-aarch64-apple-darwin";
const NOTARY_POLL: Duration = Duration::from_secs(30);
const NOTARY_TIMEOUT_SECONDS: u64 = 50 * 60;
const SIGNING_WORKFLOW_REF: &str =
    "bitcoinaustria/kassiber/.github/workflows/notarize-macos.yml@refs/heads/main";
const REPO: &str = "bitcoinaustria/kassiber";
const MACHO: [[u8; 4]; 8] = [
    [0xfe, 0xed, 0xfa, 0xce],
    [0xce, 0xfa, 0xed, 0xfe],
    [0xfe, 0xed, 0xfa, 0xcf],
    [0xcf, 0xfa, 0xed, 0xfe],
    [0xca, 0xfe, 0xba, 0xbe],
    [0xbe, 0xba, 0xfe, 0xca],
    [0xca, 0xfe, 0xba, 0xbf],
    [0xbf, 0xba, 0xfe, 0xca],
];
/// `plistlib.dumps(APP_ENTITLEMENTS)`, byte for byte.
pub const ENTITLEMENTS_PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">
<plist version=\"1.0\">
<dict>
\t<key>com.apple.application-identifier</key>
\t<string>6Q4R2C3GJK.at.bitcoinaustria.kassiber</string>
\t<key>com.apple.developer.team-identifier</key>
\t<string>6Q4R2C3GJK</string>
\t<key>keychain-access-groups</key>
\t<array>
\t\t<string>6Q4R2C3GJK.at.bitcoinaustria.kassiber</string>
\t</array>
</dict>
</plist>
";

/// What the scripts' `except` clauses told apart.
#[derive(Debug)]
pub enum Error {
    /// `ValueError`: a fixed message without paths or secrets.
    Value(String),
    /// `OSError`.
    Os(String),
    /// `subprocess.CalledProcessError`: never print the arguments.
    Process,
}

pub type Result<T> = std::result::Result<T, Error>;

fn fail<T>(message: &str) -> Result<T> {
    Err(Error::Value(message.to_owned()))
}

fn os(error: std::io::Error) -> Error {
    Error::Os(error.to_string())
}

fn app_entitlements() -> plist::Value {
    let group = format!("{TEAM}.{APP_ID}");
    let mut map = plist::Dictionary::new();
    map.insert(
        "com.apple.application-identifier".into(),
        plist::Value::String(group.clone()),
    );
    map.insert(
        "com.apple.developer.team-identifier".into(),
        plist::Value::String(TEAM.into()),
    );
    map.insert(
        "keychain-access-groups".into(),
        plist::Value::Array(vec![plist::Value::String(group)]),
    );
    plist::Value::Dictionary(map)
}

/// The scripts' `run(*args)`: check=True, stdout as text.
fn run(args: &[std::ffi::OsString]) -> Result<String> {
    let output = capture(args)?;
    Ok(output.0)
}

/// `(stdout, stderr)` of a tool that must succeed.
fn capture(args: &[std::ffi::OsString]) -> Result<(String, String)> {
    #[cfg(test)]
    if let Some(result) = tests::fake_run(args) {
        return result;
    }
    let output = Command::new(&args[0])
        .args(&args[1..])
        .stdin(Stdio::null())
        .output()
        .map_err(os)?;
    if !output.status.success() {
        return Err(Error::Process);
    }
    Ok((
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    ))
}

fn arg(path: &Path) -> std::ffi::OsString {
    path.as_os_str().to_owned()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn sha256(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(os)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(os)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex(&digest.finalize()))
}

fn lower_hex(text: &str, len: usize) -> bool {
    text.len() == len
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn check_digest(path: &Path, expected: &str) -> Result<()> {
    if !lower_hex(expected, 64) || sha256(path)? != expected {
        return fail("Artifact SHA-256 mismatch");
    }
    Ok(())
}

/// SHA-1, only to match a keychain identity against a profile certificate.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend(((data.len() as u64) * 8).to_be_bytes());
    for chunk in message.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(*word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        for (slot, value) in h.iter_mut().zip([a, b, c, d, e]) {
            *slot = slot.wrapping_add(value);
        }
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn json_get<'a>(value: &'a Json, key: &str) -> Option<&'a Json> {
    kassiber_envelope::get(value, key)
}

fn json_str(value: Option<&Json>) -> Option<&str> {
    match value {
        Some(Json::Str(text)) => text.as_str(),
        _ => None,
    }
}

fn loads(text: &str) -> Result<Json> {
    kassiber_pytext::loads(text).map_err(|_| Error::Value("Expecting value: invalid JSON".into()))
}

fn dumps_indent(value: &Json, sort_keys: bool) -> String {
    kassiber_pyjson::dumps(value, &Options::new().indent(2).sort_keys(sort_keys)).unwrap()
}

/// `str(value)` of a JSON-decoded Python value, where the checks used it.
fn py_str(value: Option<&Json>) -> Option<String> {
    match value? {
        Json::Str(text) => text.as_str().map(str::to_owned),
        Json::Int(number) => Some(number.to_string()),
        Json::Bool(true) => Some("True".into()),
        Json::Bool(false) => Some("False".into()),
        Json::Null => Some("None".into()),
        _ => None,
    }
}

// ---- profile, entitlements, certificates --------------------------------

fn parse_plist(bytes: &[u8]) -> Result<plist::Value> {
    plist::Value::from_reader(std::io::Cursor::new(bytes))
        .map_err(|_| Error::Value("Invalid property list".into()))
}

/// `validate_profile_data(data)`.
pub fn validate_profile_data(data: &plist::Value, now: SystemTime) -> Result<()> {
    let empty = plist::Dictionary::new();
    let dictionary = data.as_dictionary().unwrap_or(&empty);
    let entitlements = dictionary
        .get("Entitlements")
        .and_then(plist::Value::as_dictionary)
        .unwrap_or(&empty);
    let team_ok = dictionary.get("TeamIdentifier")
        == Some(&plist::Value::Array(vec![plist::Value::String(
            TEAM.into(),
        )]));
    let devices = dictionary.get("ProvisionsAllDevices") == Some(&plist::Value::Boolean(true));
    let unexpired = dictionary
        .get("ExpirationDate")
        .and_then(plist::Value::as_date)
        .is_some_and(|date| SystemTime::from(date) > now);
    if !(team_ok && devices && unexpired) {
        return fail("Require an unexpired Bitcoin Austria Developer ID distribution profile");
    }
    let expected = app_entitlements();
    let expected = expected.as_dictionary().unwrap();
    for key in [
        "com.apple.application-identifier",
        "com.apple.developer.team-identifier",
    ] {
        if entitlements.get(key) != expected.get(key) {
            return fail("Provisioning profile does not authorize the exact production app");
        }
    }
    let group = format!("{TEAM}.{APP_ID}");
    let wildcard = format!("{TEAM}.*");
    let authorized = match entitlements.get("keychain-access-groups") {
        None => false,
        Some(plist::Value::Array(groups)) => groups
            .iter()
            .any(|g| g.as_string().is_some_and(|g| g == group || g == wildcard)),
        // `in` on a string is a substring test.
        Some(plist::Value::String(text)) => text.contains(&group) || text.contains(&wildcard),
        Some(_) => false,
    };
    if !authorized {
        return fail("Provisioning profile does not authorize the app Keychain group");
    }
    Ok(())
}

/// `load_profile(path)`.
fn load_profile(path: &Path) -> Result<plist::Value> {
    let metadata = std::fs::symlink_metadata(path).ok();
    let size_ok = std::fs::metadata(path).is_ok_and(|m| m.len() <= 1024 * 1024);
    if metadata.is_none_or(|m| m.file_type().is_symlink()) || !path.is_file() || !size_ok {
        return fail("Invalid provisioning profile file");
    }
    let xml = run(&[
        "/usr/bin/security".into(),
        "cms".into(),
        "-D".into(),
        "-i".into(),
        arg(path),
    ])?;
    let data = parse_plist(xml.as_bytes())?;
    validate_profile_data(&data, SystemTime::now())?;
    Ok(data)
}

fn verify_entitlements(path: &Path, expected: &plist::Value) -> Result<()> {
    let xml = run(&[
        "/usr/bin/codesign".into(),
        "-d".into(),
        "--entitlements".into(),
        "-".into(),
        "--xml".into(),
        arg(path),
    ])?;
    let actual = if kassiber_pytext::strip(&xml).is_empty() {
        plist::Value::Dictionary(plist::Dictionary::new())
    } else {
        parse_plist(xml.as_bytes())?
    };
    if &actual != expected {
        return fail("Unexpected code entitlements (no debug/runtime exemptions allowed)");
    }
    Ok(())
}

fn certificates(profile: &plist::Value) -> Vec<Vec<u8>> {
    profile
        .as_dictionary()
        .and_then(|d| d.get("DeveloperCertificates"))
        .and_then(plist::Value::as_array)
        .map(|certs| {
            certs
                .iter()
                .filter_map(|c| c.as_data().map(<[u8]>::to_vec))
                .collect()
        })
        .unwrap_or_default()
}

fn temp_dir(prefix: &str) -> Result<tempdir::TempDir> {
    tempdir::TempDir::new(prefix)
}

/// A private scratch directory removed on drop (`tempfile.TemporaryDirectory`).
mod tempdir {
    use std::path::{Path, PathBuf};

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new(prefix: &str) -> super::Result<Self> {
            let base = std::env::temp_dir();
            for attempt in 0..100u32 {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .subsec_nanos();
                let path = base.join(format!(
                    "{prefix}{}{nanos:08x}{attempt}",
                    std::process::id()
                ));
                let mut builder = std::fs::DirBuilder::new();
                #[cfg(unix)]
                std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
                match builder.create(&path) {
                    Ok(()) => return Ok(Self(path)),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(super::os(error)),
                }
            }
            Err(super::Error::Os(
                "could not create a temporary directory".into(),
            ))
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

fn verify_profile_certificate(app: &Path, profile: &plist::Value) -> Result<()> {
    let tmp = temp_dir("kassiber-public-cert-")?;
    let prefix = tmp.path().join("cert");
    run(&[
        "/usr/bin/codesign".into(),
        "-d".into(),
        format!("--extract-certificates={}", prefix.display()).into(),
        arg(app),
    ])?;
    let leaf = std::fs::read(tmp.path().join("cert0")).map_err(os)?;
    if !certificates(profile).contains(&leaf) {
        return fail("App signing certificate is not authorized by the profile");
    }
    Ok(())
}

// ---- archives -------------------------------------------------------------

/// `PurePosixPath(name).parts`.
fn posix_parts(name: &str) -> (bool, Vec<&str>) {
    let absolute = name.starts_with('/');
    let parts: Vec<&str> = name
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    (absolute, parts)
}

/// The member checks `extract_app` makes before `ditto` sees the archive.
pub fn check_app_archive(archive: &Path) -> Result<()> {
    let file = std::fs::File::open(archive).map_err(os)?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|_| Error::Value("File is not a zip file".into()))?;
    let mut seen = BTreeSet::new();
    let mut total: u64 = 0;
    for index in 0..zip.len() {
        let member = zip
            .by_index_raw(index)
            .map_err(|_| Error::Value("Bad zip member".into()))?;
        let name = member
            .name()
            .map_err(|_| Error::Value("Bad zip member name".into()))?
            .into_owned();
        let mode = member.external_attributes() >> 16;
        let (absolute, parts) = posix_parts(&name);
        let kind = mode & 0o170000;
        if absolute
            || parts.contains(&"..")
            || name.contains('\\')
            || parts.is_empty()
            || parts[0] != "Kassiber.app"
            || seen.contains(&name)
            || !matches!(kind, 0 | 0o100000 | 0o040000)
        {
            return fail("Unsafe or unexpected app archive member");
        }
        total += member.size();
        seen.insert(name);
        if total > 4 * 1024u64.pow(3) || seen.len() > 100_000 {
            return fail("App archive exceeds extraction limits");
        }
    }
    Ok(())
}

/// `extract_app(archive, destination)`.
fn extract_app(archive: &Path, destination: &Path) -> Result<PathBuf> {
    check_app_archive(archive)?;
    run(&[
        "/usr/bin/ditto".into(),
        "-x".into(),
        "-k".into(),
        arg(archive),
        arg(destination),
    ])?;
    let app = destination.join("Kassiber.app");
    if !app.is_dir() {
        return fail("Missing Kassiber.app");
    }
    Ok(app)
}

/// Every path under `root`, sorted as `sorted(root.rglob("*"))` sorts them;
/// symlinks are listed, not followed.
fn walk(root: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir).map_err(os)? {
            let path = entry.map_err(os)?.path();
            let kind = std::fs::symlink_metadata(&path).map_err(os)?.file_type();
            if kind.is_dir() {
                pending.push(path.clone());
            }
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// `validate_app(app, commit, version)`.
pub fn validate_app(app: &Path, commit: &str, version: &str) -> Result<()> {
    if !lower_hex(commit, 40) {
        return fail("Expected full source commit");
    }
    let info = parse_plist(&std::fs::read(app.join("Contents/Info.plist")).map_err(os)?)?;
    let info = info.as_dictionary().cloned().unwrap_or_default();
    if info
        .get("CFBundleIdentifier")
        .and_then(plist::Value::as_string)
        != Some(APP_ID)
    {
        return fail("Unexpected app identity (dev builds cannot be released)");
    }
    if info
        .get("CFBundleShortVersionString")
        .and_then(plist::Value::as_string)
        != Some(version)
    {
        return fail("App version does not match expected source version");
    }
    let metadata: Vec<PathBuf> = walk(app)?
        .into_iter()
        .filter(|p| p.file_name().is_some_and(|n| n == "BUILD_INFO.json"))
        .collect();
    if metadata.is_empty() {
        return fail("Missing embedded build provenance");
    }
    for path in metadata {
        let data = loads(&std::fs::read_to_string(&path).map_err(os)?)?;
        if json_str(json_get(&data, "commit")) != Some(commit)
            || json_str(json_get(&data, "version")) != Some(version)
        {
            return fail("Embedded build provenance does not match release");
        }
        if !matches!(
            json_str(json_get(&data, "channel")),
            Some("release" | "prerelease")
        ) {
            return fail("Development artifact cannot be released");
        }
    }
    Ok(())
}

fn digits(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit())
}

/// `validate_source(source, commit, version, candidate_id, input_digest)`.
pub fn validate_source(
    source: &Json,
    commit: &str,
    version: &str,
    candidate_id: Option<&str>,
    input_digest: Option<&str>,
) -> Result<()> {
    let get = |key| json_get(source, key);
    let build_run = py_str(get("build_run")).unwrap_or_default();
    let attempt_ok = matches!(get("build_attempt"), Some(Json::Int(n)) if *n >= 1.into());
    if json_str(get("commit")) != Some(commit) || !digits(&build_run) || !attempt_ok {
        return fail("Missing or mismatched signed source provenance");
    }
    match candidate_id.filter(|c| !c.is_empty()) {
        Some(candidate) => {
            let expected = format!("macos-candidate-{commit}-{build_run}");
            let shape = candidate
                .strip_prefix("macos-candidate-")
                .and_then(|rest| rest.split_once('-'))
                .is_some_and(|(hash, run)| lower_hex(hash, 40) && digits(run));
            let keys: BTreeSet<&str> = match source {
                Json::Dict(pairs) => pairs
                    .iter()
                    .filter_map(|(k, _)| match k {
                        kassiber_pyjson::Key::Str(text) => text.as_str(),
                        _ => None,
                    })
                    .collect(),
                _ => BTreeSet::new(),
            };
            let wanted: BTreeSet<&str> = [
                "kind",
                "candidate_id",
                "commit",
                "version",
                "build_run",
                "build_attempt",
                "unsigned_app_sha256",
            ]
            .into();
            let digest_ok = py_str(get("unsigned_app_sha256"))
                .or_else(|| get("unsigned_app_sha256").is_none().then(String::new))
                .is_some_and(|d| lower_hex(&d, 64));
            if !shape
                || candidate != expected
                || json_str(get("candidate_id")) != Some(candidate)
                || json_str(get("kind")) != Some("candidate")
                || json_str(get("version")) != Some(version)
                || keys != wanted
                || !digest_ok
            {
                return fail("Candidate provenance does not match the explicit candidate");
            }
        }
        None => {
            if json_str(get("tag")) != Some(&format!("v{version}"))
                || json_str(get("kind")) == Some("candidate")
                || get("candidate_id").is_some()
            {
                return fail("Release provenance must name the expected version tag");
            }
        }
    }
    if let Some(digest) = input_digest {
        if json_str(get("unsigned_app_sha256")) != Some(digest) {
            return fail("Signing provenance does not match input");
        }
    }
    Ok(())
}

/// `code_files(app)`.
pub fn code_files(app: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    for path in walk(app)? {
        let kind = std::fs::symlink_metadata(&path).map_err(os)?.file_type();
        if kind.is_symlink() {
            return fail("Unexpected symlink in sealed app");
        }
        if kind.is_file() {
            let mut head = Vec::new();
            std::fs::File::open(&path)
                .map_err(os)?
                .take(4)
                .read_to_end(&mut head)
                .map_err(os)?;
            if MACHO.iter().any(|magic| head == magic) {
                result.push(path);
            }
        }
    }
    if result.is_empty() {
        return fail("No Mach-O code in app");
    }
    Ok(result)
}

// ---- Mach-O linkage ---------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
pub struct Linkage {
    pub install_id: Option<String>,
    pub dependencies: Vec<String>,
    pub rpaths: Vec<String>,
}

/// `^Load command N\s*$`: the number, if the line is one.
fn load_command_line(line: &str) -> Option<&str> {
    let number = line
        .strip_prefix("Load command ")?
        .trim_end_matches(|c: char| c.is_whitespace());
    digits(number).then_some(number)
}

/// The first `^\s+<key> <value>...` line of a block, as `(rest)` after the key.
fn field_line<'a>(block: &[&'a str], key: &str) -> Option<&'a str> {
    block.iter().find_map(|line| {
        let trimmed = line.trim_start();
        (trimmed.len() < line.len())
            .then(|| trimmed.strip_prefix(key)?.strip_prefix(' '))
            .flatten()
    })
}

/// `read_linkage` over `otool -arch all -l` output.
pub fn parse_linkage(output: &str) -> Result<Vec<Linkage>> {
    let lines: Vec<&str> = output.lines().collect();
    let mut slices: Vec<Vec<Vec<&str>>> = Vec::new();
    let mut started = false;
    for line in lines {
        match load_command_line(line) {
            Some("0") => {
                started = true;
                slices.push(vec![Vec::new()]);
            }
            Some(_) if started => slices.last_mut().unwrap().push(Vec::new()),
            _ if started => slices.last_mut().unwrap().last_mut().unwrap().push(line),
            _ => {}
        }
    }
    if slices.is_empty() {
        return fail("Missing Mach-O load commands");
    }
    let dependency_commands = [
        "LC_LOAD_DYLIB",
        "LC_LOAD_WEAK_DYLIB",
        "LC_REEXPORT_DYLIB",
        "LC_LOAD_UPWARD_DYLIB",
        "LC_LAZY_LOAD_DYLIB",
    ];
    let mut result = Vec::new();
    for architecture in slices {
        let mut linkage = Linkage {
            install_id: None,
            dependencies: Vec::new(),
            rpaths: Vec::new(),
        };
        let mut identities = Vec::new();
        for block in architecture {
            let command = field_line(&block, "cmd")
                .map(|rest| rest.trim_end())
                .filter(|kind| {
                    kind.starts_with("LC_")
                        && kind[3..]
                            .bytes()
                            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
                });
            let Some(kind) = command else {
                return fail("Malformed Mach-O load command");
            };
            let wanted =
                dependency_commands.contains(&kind) || kind == "LC_ID_DYLIB" || kind == "LC_RPATH";
            if !wanted {
                if kind.ends_with("_DYLIB") || kind == "LC_DYLD_ENVIRONMENT" {
                    return fail("Unsupported Mach-O linkage command");
                }
                continue;
            }
            let key = if kind == "LC_RPATH" { "path" } else { "name" };
            let value = block.iter().find_map(|line| {
                let trimmed = line.trim_start();
                if trimmed.len() == line.len() {
                    return None;
                }
                let rest = trimmed.strip_prefix(key)?.strip_prefix(' ')?.trim_end();
                let (value, offset) = rest.rsplit_once(" (offset ")?;
                let number = offset.strip_suffix(')')?;
                (!value.is_empty() && digits(number)).then(|| value.to_owned())
            });
            let Some(value) = value else {
                return fail("Malformed Mach-O linkage path");
            };
            match kind {
                "LC_RPATH" => linkage.rpaths.push(value),
                "LC_ID_DYLIB" => identities.push(value),
                _ => linkage.dependencies.push(value),
            }
        }
        if identities.len() > 1 {
            return fail("Multiple install IDs in one Mach-O architecture");
        }
        linkage.install_id = identities.pop();
        result.push(linkage);
    }
    Ok(result)
}

fn read_linkage(path: &Path) -> Result<Vec<Linkage>> {
    parse_linkage(&run(&[
        "/usr/bin/otool".into(),
        "-arch".into(),
        "all".into(),
        "-l".into(),
        arg(path),
    ])?)
}

/// `Path.resolve()`, non-strict.
fn resolve(path: &Path) -> PathBuf {
    kassiber_release::publish::resolve(path)
}

/// `check_linkage_path(value, app, path, rpath)`.
pub fn check_linkage_path(value: &str, app: &Path, path: &Path, rpath: bool) -> Result<()> {
    if value.is_empty() || value.contains('\\') || value.chars().any(|c| (c as u32) < 32) {
        return fail("Invalid Mach-O linkage path");
    }
    if !rpath
        && ["/usr/lib/", "/System/Library/", "@rpath/"]
            .iter()
            .any(|prefix| value.starts_with(prefix))
        && value
            .trim_start_matches('/')
            .split('/')
            .all(|part| !matches!(part, "" | "." | ".."))
    {
        return Ok(());
    }
    let executable_dir = app.join("Contents/MacOS");
    for (prefix, base) in [
        ("@loader_path", path.parent().unwrap_or(Path::new(""))),
        ("@executable_path", executable_dir.as_path()),
    ] {
        if value == prefix || value.starts_with(&format!("{prefix}/")) {
            let suffix = value[prefix.len()..]
                .strip_prefix('/')
                .unwrap_or(&value[prefix.len()..]);
            let parts_ok =
                suffix.is_empty() || suffix.split('/').all(|part| !matches!(part, "" | "."));
            if parts_ok && resolve(&base.join(suffix)).starts_with(resolve(app)) {
                return Ok(());
            }
        }
    }
    fail("Unsafe or unsupported Mach-O dependency/rpath")
}

/// `linkage_changes(app)` with an injectable `otool` reader.
pub fn linkage_changes_with(
    app: &Path,
    read: &mut dyn FnMut(&Path) -> Result<Vec<Linkage>>,
) -> Result<Vec<(PathBuf, String)>> {
    let mut changes = Vec::new();
    for path in code_files(app)? {
        let slices = read(&path)?;
        let mut normalized: BTreeSet<Option<String>> = BTreeSet::new();
        for architecture in &slices {
            let unique: BTreeSet<&String> = architecture.rpaths.iter().collect();
            if unique.len() != architecture.rpaths.len() {
                return fail("Duplicate Mach-O rpath within one architecture");
            }
            for dependency in &architecture.dependencies {
                check_linkage_path(dependency, app, &path, false)?;
            }
            for rpath in &architecture.rpaths {
                check_linkage_path(rpath, app, &path, true)?;
            }
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
            match &architecture.install_id {
                None => {
                    normalized.insert(None);
                }
                Some(identity) if identity.starts_with("@rpath/") => {
                    check_linkage_path(identity, app, &path, false)?;
                    normalized.insert(Some(identity.clone()));
                }
                Some(identity)
                    if posix_parts(identity).1.last().map(|n| n.to_string()) == name
                        && !identity.contains('\\') =>
                {
                    normalized.insert(Some(format!("@rpath/{}", name.unwrap())));
                }
                Some(_) => return fail("Unsupported Mach-O install ID"),
            }
        }
        if normalized.len() != 1 {
            return fail("Ambiguous Mach-O install IDs across architectures");
        }
        let identity = normalized.into_iter().next().unwrap();
        if slices.iter().any(|a| a.install_id != identity) {
            changes.push((path, identity.expect("an install ID to normalize")));
        }
    }
    Ok(changes)
}

fn linkage_changes(app: &Path) -> Result<Vec<(PathBuf, String)>> {
    linkage_changes_with(app, &mut |path| read_linkage(path))
}

fn verify_linkage(app: &Path) -> Result<()> {
    if !linkage_changes(app)?.is_empty() {
        return fail("Mach-O install ID would be rewritten by Homebrew");
    }
    Ok(())
}

/// Homebrew preserve_rpath protects @rpath IDs only. Normalize before
/// sealing; do not rewrite dependency references or try to repair unknown
/// build layouts.
fn prepare_linkage(app: &Path) -> Result<()> {
    let changes = linkage_changes(app)?;
    for (path, identity) in &changes {
        run(&[
            "/usr/bin/install_name_tool".into(),
            "-id".into(),
            identity.into(),
            arg(path),
        ])?;
    }
    if !changes.is_empty() {
        verify_linkage(app)?;
    }
    Ok(())
}

// ---- signatures -------------------------------------------------------------

fn verify_code(path: &Path, runtime: bool) -> Result<()> {
    let requirement = format!(
        "anchor apple generic and certificate leaf[subject.OU] = \"{TEAM}\" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    );
    run(&[
        "/usr/bin/codesign".into(),
        "--verify".into(),
        "--strict".into(),
        "-R".into(),
        format!("={requirement}").into(),
        arg(path),
    ])?;
    let (_, details) = capture(&[
        "/usr/bin/codesign".into(),
        "-d".into(),
        "--verbose=4".into(),
        arg(path),
    ])?;
    if !details.contains("Timestamp=") || (runtime && !details.contains("runtime")) {
        return fail("Missing secure timestamp or hardened runtime");
    }
    Ok(())
}

fn verify_sidecar_identity(app: &Path) -> Result<()> {
    let sidecar = app.join(SIDECAR);
    let is_link = std::fs::symlink_metadata(&sidecar).is_ok_and(|m| m.file_type().is_symlink());
    if !sidecar.is_file() || is_link {
        return fail("Backend sidecar is missing from its bundle location");
    }
    let (_, details) = capture(&[
        "/usr/bin/codesign".into(),
        "-d".into(),
        "--verbose=2".into(),
        arg(&sidecar),
    ])?;
    let identifiers: Vec<&str> = details
        .lines()
        .filter_map(|line| line.strip_prefix("Identifier="))
        .collect();
    let name = sidecar.file_name().unwrap().to_string_lossy();
    if identifiers != [name.as_ref()] {
        return fail("Backend sidecar is not signed under its stable identifier");
    }
    Ok(())
}

fn verify_app(
    app: &Path,
    commit: &str,
    version: &str,
    ticket: bool,
    candidate_id: Option<&str>,
) -> Result<()> {
    validate_app(app, commit, version)?;
    verify_sidecar_identity(app)?;
    verify_linkage(app)?;
    let profile = load_profile(&app.join("Contents/embedded.provisionprofile"))?;
    verify_profile_certificate(app, &profile)?;
    let source = loads(
        &std::fs::read_to_string(app.join("Contents/Resources/RELEASE_SOURCE.json")).map_err(os)?,
    )?;
    validate_source(&source, commit, version, candidate_id, None)?;
    let outer = app.join("Contents/MacOS/kassiber-ui");
    let empty = plist::Value::Dictionary(plist::Dictionary::new());
    for path in code_files(app)? {
        verify_code(&path, true)?;
        let expected = if path == outer {
            app_entitlements()
        } else {
            empty.clone()
        };
        verify_entitlements(&path, &expected)?;
    }
    verify_code(app, true)?;
    verify_entitlements(app, &app_entitlements())?;
    run(&[
        "/usr/bin/codesign".into(),
        "--verify".into(),
        "--deep".into(),
        "--strict".into(),
        arg(app),
    ])?;
    if ticket {
        run(&[
            "/usr/bin/xcrun".into(),
            "stapler".into(),
            "validate".into(),
            arg(app),
        ])?;
        run(&[
            "/usr/sbin/spctl".into(),
            "--assess".into(),
            "--type".into(),
            "execute".into(),
            "--verbose=4".into(),
            arg(app),
        ])?;
        // CMS decoding above is not treated as Apple's authorization
        // verdict. Let the OS validate the complete provisioned distribution.
        run(&[
            "/usr/bin/syspolicy_check".into(),
            "distribution".into(),
            arg(app),
        ])?;
    }
    Ok(())
}

/// Allow CI signing only in the protected notarization workflow on main.
/// Local signing stays available.
pub fn require_trusted_signing_context(env: &dyn Fn(&str) -> Option<String>) -> Result<()> {
    if env("CI").is_none_or(|v| v.is_empty()) {
        return Ok(());
    }
    if env("GITHUB_ACTIONS").as_deref() != Some("true")
        || env("GITHUB_WORKFLOW_REF").as_deref() != Some(SIGNING_WORKFLOW_REF)
        || env("GITHUB_EVENT_NAME").as_deref() != Some("workflow_dispatch")
        || env("GITHUB_REF").as_deref() != Some("refs/heads/main")
    {
        return fail("CI signing runs only in the protected notarize-macos workflow on main");
    }
    Ok(())
}

fn process_env(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// `shutil.copy2`: contents, mode and times.
fn copy2(source: &Path, destination: &Path) -> Result<()> {
    std::fs::copy(source, destination).map_err(os)?;
    let metadata = std::fs::metadata(source).map_err(os)?;
    let times = std::fs::FileTimes::new()
        .set_accessed(metadata.accessed().map_err(os)?)
        .set_modified(metadata.modified().map_err(os)?);
    std::fs::File::options()
        .write(true)
        .open(destination)
        .and_then(|file| file.set_times(times))
        .map_err(os)
}

pub struct SignArgs {
    pub archive: PathBuf,
    pub sha256: String,
    pub identity: String,
    pub commit: String,
    pub version: String,
    pub output: PathBuf,
    pub source: PathBuf,
    pub provisioning_profile: PathBuf,
    pub candidate_id: Option<String>,
}

fn mkdir_new(path: &Path) -> Result<()> {
    if path.exists() {
        return Err(Error::Os(format!(
            "[Errno 17] File exists: '{}'",
            path.display()
        )));
    }
    std::fs::create_dir_all(path).map_err(os)
}

pub fn sign(args: &SignArgs) -> Result<()> {
    require_trusted_signing_context(&process_env)?;
    let identity_ok =
        args.identity.len() == 40 && args.identity.bytes().all(|b| b.is_ascii_hexdigit());
    if !identity_ok {
        return fail("Select an exact keychain certificate SHA-1 fingerprint");
    }
    check_digest(&args.archive, &args.sha256)?;
    let profile = load_profile(&args.provisioning_profile)?;
    let wanted = args.identity.to_uppercase();
    if !certificates(&profile)
        .iter()
        .any(|cert| hex(&sha1(cert)).to_uppercase() == wanted)
    {
        return fail("Selected signing identity is not authorized by the provisioning profile");
    }
    mkdir_new(&args.output)?;
    let tmp = temp_dir("kassiber-sign-")?;
    let stage = tmp.path().join("payload");
    std::fs::create_dir(&stage).map_err(os)?;
    let app = extract_app(&resolve(&args.archive), &stage)?;
    validate_app(&app, &args.commit, &args.version)?;
    copy2(
        &args.provisioning_profile,
        &app.join("Contents/embedded.provisionprofile"),
    )?;
    let entitlements = tmp.path().join("entitlements.plist");
    std::fs::write(&entitlements, ENTITLEMENTS_PLIST).map_err(os)?;
    let source = loads(&std::fs::read_to_string(&args.source).map_err(os)?)?;
    validate_source(
        &source,
        &args.commit,
        &args.version,
        args.candidate_id.as_deref(),
        Some(&args.sha256),
    )?;
    std::fs::write(
        app.join("Contents/Resources/RELEASE_SOURCE.json"),
        format!("{}\n", dumps_indent(&source, true)),
    )
    .map_err(os)?;
    prepare_linkage(&app)?;
    // Explicit inner-to-outer signing. Never --deep sign or disable library
    // validation: every bundled library receives the same Team ID.
    for path in code_files(&app)? {
        let name = path.file_name().unwrap().to_os_string();
        run(&[
            "/usr/bin/codesign".into(),
            "--force".into(),
            "--sign".into(),
            args.identity.as_str().into(),
            "--timestamp".into(),
            "--options".into(),
            "runtime".into(),
            "--identifier".into(),
            name,
            arg(&path),
        ])?;
    }
    run(&[
        "/usr/bin/codesign".into(),
        "--force".into(),
        "--sign".into(),
        args.identity.as_str().into(),
        "--timestamp".into(),
        "--options".into(),
        "runtime".into(),
        "--identifier".into(),
        APP_ID.into(),
        "--entitlements".into(),
        arg(&entitlements),
        arg(&app),
    ])?;
    verify_app(
        &app,
        &args.commit,
        &args.version,
        false,
        args.candidate_id.as_deref(),
    )?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", stage.join("Applications")).map_err(os)?;
    let image = resolve(&args.output).join(INPUT_DMG);
    run(&[
        "/usr/bin/hdiutil".into(),
        "create".into(),
        "-volname".into(),
        "Kassiber".into(),
        "-srcfolder".into(),
        arg(&stage),
        "-format".into(),
        "UDZO".into(),
        arg(&image),
    ])?;
    run(&[
        "/usr/bin/codesign".into(),
        "--sign".into(),
        args.identity.as_str().into(),
        "--timestamp".into(),
        arg(&image),
    ])?;
    verify_code(&image, false)?;
    let printed = dict([
        ("input", s(image.to_string_lossy().as_ref())),
        ("sha256", s(sha256(&image)?)),
        ("commit", s(args.commit.as_str())),
        ("version", s(args.version.as_str())),
    ]);
    println!("{}", dumps_indent(&printed, false));
    Ok(())
}

fn app_from_image(image: &Path, stage: &Path) -> Result<PathBuf> {
    let mount = stage.join("mount");
    std::fs::create_dir(&mount).map_err(os)?;
    run(&[
        "/usr/bin/hdiutil".into(),
        "attach".into(),
        "-readonly".into(),
        "-nobrowse".into(),
        "-noautoopen".into(),
        "-mountpoint".into(),
        arg(&mount),
        arg(image),
    ])?;
    let copied = (|| {
        let allowed = [
            "Kassiber.app",
            "Applications",
            ".DS_Store",
            ".Trashes",
            ".fseventsd",
        ];
        for entry in std::fs::read_dir(&mount).map_err(os)? {
            let name = entry.map_err(os)?.file_name();
            if !allowed.iter().any(|a| name == *a) {
                return fail("Unexpected disk image contents");
            }
        }
        let app = stage.join("Kassiber.app");
        run(&[
            "/usr/bin/ditto".into(),
            arg(&mount.join("Kassiber.app")),
            arg(&app),
        ])?;
        Ok(app)
    })();
    let detached = run(&["/usr/bin/hdiutil".into(), "detach".into(), arg(&mount)]);
    let app = copied?;
    detached?;
    Ok(app)
}

pub struct NotaryArgs {
    pub image: PathBuf,
    pub sha256: String,
    pub commit: String,
    pub version: String,
    pub candidate_id: Option<String>,
    pub output: PathBuf,
    pub profile: String,
    pub keychain: PathBuf,
    pub submission_id: String,
    pub wait_seconds: i64,
    pub pending_ok: bool,
}

/// `time.sleep`; tests do not wait.
fn pause(duration: Duration) {
    if !cfg!(test) {
        std::thread::sleep(duration);
    }
}

fn uuid_shape(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups
            .iter()
            .zip([8, 4, 4, 4, 12])
            .all(|(group, len)| lower_hex(group, len))
}

fn status(result: &Json) -> Option<&str> {
    json_str(json_get(result, "status"))
}

/// Submit once, or resume an explicit ID; keep the bytes/identity binding.
fn submit_and_wait(args: &NotaryArgs, evidence: &Path) -> Result<Json> {
    let resume = args.submission_id.as_str();
    if !resume.is_empty() && !uuid_shape(resume) {
        return fail("Invalid Apple submission ID");
    }
    let tail = |operation: Vec<std::ffi::OsString>| {
        let mut command: Vec<std::ffi::OsString> =
            vec!["/usr/bin/xcrun".into(), "notarytool".into()];
        command.extend(operation);
        command.extend([
            "--keychain-profile".into(),
            args.profile.as_str().into(),
            "--keychain".into(),
            arg(&args.keychain),
            "--output-format".into(),
            "json".into(),
        ]);
        command
    };
    // Retrying reads is safe; never automatically retry an ambiguous submit.
    let query = |operation: Vec<std::ffi::OsString>| -> Result<Json> {
        for attempt in 0..3u64 {
            match run(&tail(operation.clone())) {
                Ok(text) => return loads(&text),
                Err(Error::Process) if attempt < 2 => pause(Duration::from_secs(5 * (attempt + 1))),
                Err(error) => return Err(error),
            }
        }
        unreachable!()
    };
    let mut result = if resume.is_empty() {
        loads(&run(&tail(vec!["submit".into(), arg(&args.image)]))?)?
    } else {
        query(vec!["info".into(), resume.into()])?
    };
    let Some(submission) = json_str(json_get(&result, "id"))
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
    else {
        return fail("Notary submission did not return an ID");
    };
    let persist = |result: &Json| -> Result<()> {
        if json_str(json_get(result, "id")) != Some(submission.as_str()) {
            return fail("Apple response changed submission ID");
        }
        std::fs::write(evidence, format!("{}\n", dumps_indent(result, false))).map_err(os)?;
        // Keep a separate receipt: Apple's response stays unmodified.
        let receipt = dict([
            ("submission_id", s(submission.as_str())),
            ("sha256", s(args.sha256.as_str())),
            ("commit", s(args.commit.as_str())),
            ("version", s(args.version.as_str())),
            (
                "candidate_id",
                args.candidate_id.as_deref().map_or(Json::Null, s),
            ),
        ]);
        std::fs::write(
            evidence.with_file_name("submission.json"),
            format!("{}\n", dumps_indent(&receipt, false)),
        )
        .map_err(os)
    };
    if !resume.is_empty() && submission != resume {
        return fail("Apple response does not match requested submission");
    }
    persist(&result)?;
    println!("Apple notary submission: {submission}");
    let deadline = Instant::now() + Duration::from_secs(args.wait_seconds.max(0) as u64);
    while !matches!(status(&result), Some("Accepted" | "Invalid" | "Rejected")) {
        if Instant::now() >= deadline {
            if args.pending_ok {
                return Ok(result);
            }
            return Err(Error::Value(format!(
                "Notarization still pending after {} minutes; submission {submission} is preserved in evidence",
                NOTARY_TIMEOUT_SECONDS / 60
            )));
        }
        pause(NOTARY_POLL);
        result = query(vec!["info".into(), submission.as_str().into()])?;
        persist(&result)?;
        let shown = match json_get(&result, "status") {
            Some(Json::Str(text)) => text.as_str().unwrap_or_default().to_owned(),
            Some(other) => kassiber_pyjson::dumps(other, &Options::new()).unwrap_or_default(),
            None => "unknown".into(),
        };
        println!("Apple notary status: {shown}");
    }
    Ok(result)
}

/// The CLI archive: `kassiber-cli-macos-arm64/` with a copy of the sealed
/// app and the `kassiber` launcher symlink, entries in sorted order.
fn write_cli_archive(cli_root: &Path, output: &Path) -> Result<()> {
    let file = std::fs::File::create(output).map_err(os)?;
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::best());
    let mut builder = tar::Builder::new(encoder);
    builder.follow_symlinks(false);
    let name = cli_root.file_name().unwrap();
    let mut entries = vec![cli_root.to_path_buf()];
    let mut ordered = Vec::new();
    // tarfile.add recursion: a directory, then its sorted children, depth first.
    while let Some(path) = entries.pop() {
        ordered.push(path.clone());
        if std::fs::symlink_metadata(&path).map_err(os)?.is_dir() {
            let mut children: Vec<PathBuf> = std::fs::read_dir(&path)
                .map_err(os)?
                .map(|e| e.map(|e| e.path()))
                .collect::<std::io::Result<_>>()
                .map_err(os)?;
            children.sort();
            entries.extend(children.into_iter().rev());
        }
    }
    for path in ordered {
        let relative = path.strip_prefix(cli_root).unwrap();
        let archived = Path::new(name).join(relative);
        builder
            .append_path_with_name(&path, &archived)
            .map_err(os)?;
    }
    builder
        .into_inner()
        .and_then(|encoder| encoder.finish())
        .map(|_| ())
        .map_err(os)
}

/// `shutil.copytree(app, destination)`: files with mode and times.
fn copytree(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination).map_err(os)?;
    for entry in std::fs::read_dir(source).map_err(os)? {
        let path = entry.map_err(os)?.path();
        let target = destination.join(path.file_name().unwrap());
        if path.is_dir() {
            copytree(&path, &target)?;
        } else {
            copy2(&path, &target)?;
        }
    }
    let permissions = std::fs::metadata(source).map_err(os)?.permissions();
    std::fs::set_permissions(destination, permissions).map_err(os)
}

pub fn notarize(args: &NotaryArgs) -> Result<()> {
    check_digest(&args.image, &args.sha256)?;
    verify_code(&args.image, false)?;
    mkdir_new(&args.output)?;
    let tmp = temp_dir("kassiber-notary-")?;
    let app = app_from_image(&resolve(&args.image), tmp.path())?;
    let candidate = args.candidate_id.as_deref();
    verify_app(&app, &args.commit, &args.version, false, candidate)?;
    // Apple creates tickets for both this signed image and its inner app.
    let result = submit_and_wait(args, &args.output.join("notarization.json"))?;
    if status(&result) == Some("In Progress") && args.pending_ok {
        println!(
            "Apple submission pending; resume the same ID and SHA-256. No artifacts promoted."
        );
        return Ok(());
    }
    if status(&result) != Some("Accepted") {
        let id = json_str(json_get(&result, "id"))
            .unwrap_or_default()
            .to_owned();
        let log = run(&[
            "/usr/bin/xcrun".into(),
            "notarytool".into(),
            "log".into(),
            id.into(),
            "--keychain-profile".into(),
            args.profile.as_str().into(),
            "--keychain".into(),
            arg(&args.keychain),
        ]);
        match log {
            Ok(log) => {
                std::fs::write(args.output.join("notarization-log.json"), log).map_err(os)?
            }
            Err(Error::Process) => {
                println!("Apple rejection log unavailable; submission ID retained.")
            }
            Err(other) => return Err(other),
        }
        return fail("Notarization was not Accepted; no release files are ready");
    }
    let image = args.output.join(DMG);
    copy2(&args.image, &image)?;
    for path in [&image, &app] {
        run(&[
            "/usr/bin/xcrun".into(),
            "stapler".into(),
            "staple".into(),
            arg(path),
        ])?;
        run(&[
            "/usr/bin/xcrun".into(),
            "stapler".into(),
            "validate".into(),
            arg(path),
        ])?;
    }
    verify_app(&app, &args.commit, &args.version, true, candidate)?;
    verify_code(&image, false)?;
    run(&[
        "/usr/sbin/spctl".into(),
        "--assess".into(),
        "--type".into(),
        "open".into(),
        "--context".into(),
        "context:primary-signature".into(),
        "--verbose=4".into(),
        arg(&image),
    ])?;
    run(&[
        "/usr/bin/ditto".into(),
        "-c".into(),
        "-k".into(),
        "--keepParent".into(),
        arg(&app),
        arg(&args.output.join(APP_ZIP)),
    ])?;
    // A single notarized app supplies the CLI too, so the CLI and the
    // desktop share one sealed sidecar and its signing identity.
    let cli_root = tmp.path().join("kassiber-cli-macos-arm64");
    std::fs::create_dir(&cli_root).map_err(os)?;
    copytree(&app, &cli_root.join("Kassiber.app"))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        "Kassiber.app/Contents/Resources/bin/kassiber",
        cli_root.join("kassiber"),
    )
    .map_err(os)?;
    write_cli_archive(&cli_root, &args.output.join(CLI_TAR))
}

/// `verify_cli_archive(archive_path, app)`.
pub fn verify_cli_archive(archive_path: &Path, app: &Path) -> Result<()> {
    let mut expected: BTreeMap<String, PathBuf> = BTreeMap::new();
    for path in walk(app)? {
        if path.is_file() {
            let relative = path
                .strip_prefix(app)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            expected.insert(relative, path);
        }
    }
    let file = std::fs::File::open(archive_path).map_err(os)?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let prefix = "kassiber-cli-macos-arm64/";
    let root = prefix.trim_end_matches('/');
    let mut actual = BTreeSet::new();
    let mut seen = BTreeSet::new();
    let mut launcher = false;
    for member in archive.entries().map_err(os)? {
        let mut member = member.map_err(os)?;
        let raw = member.path_bytes().into_owned();
        let name = String::from_utf8_lossy(&raw)
            .trim_end_matches('/')
            .to_owned();
        if seen.contains(&name) || !name.starts_with(root) || posix_parts(&name).1.contains(&"..") {
            return fail("Duplicate/unsafe CLI archive member");
        }
        seen.insert(name.clone());
        if seen.len() > 100_000 {
            return fail("CLI archive exceeds member limit");
        }
        let kind = member.header().entry_type();
        let app_prefix = format!("{prefix}Kassiber.app/");
        if kind.is_dir() {
            if name != root
                && name != format!("{prefix}Kassiber.app")
                && !name.starts_with(&app_prefix)
            {
                return fail("Unexpected CLI directory");
            }
            continue;
        }
        if name == format!("{prefix}kassiber") && kind.is_symlink() {
            let target = member
                .link_name_bytes()
                .map(|t| String::from_utf8_lossy(&t).into_owned());
            if target.as_deref() != Some("Kassiber.app/Contents/Resources/bin/kassiber") {
                return fail("Unexpected CLI launcher");
            }
            launcher = true;
            continue;
        }
        let is_file = matches!(kind, tar::EntryType::Regular | tar::EntryType::Continuous)
            || kind.as_byte() == 0;
        if !is_file || !name.starts_with(&app_prefix) {
            return fail("Unexpected CLI archive member");
        }
        let relative = name[app_prefix.len()..].to_owned();
        let Some(source) = expected.get(&relative) else {
            return fail("CLI member does not match sealed app");
        };
        let metadata = std::fs::metadata(source).map_err(os)?;
        if member.header().size().map_err(os)? != metadata.len() {
            return fail("CLI member does not match sealed app");
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if member.header().mode().map_err(os)? & 0o7777
                != metadata.permissions().mode() & 0o7777
            {
                return fail("CLI permissions differ from sealed app");
            }
        }
        let mut digest = Sha256::new();
        let mut buffer = vec![0; 1 << 20];
        loop {
            let read = member.read(&mut buffer).map_err(os)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
        if hex(&digest.finalize()) != sha256(source)? {
            return fail("CLI archive and notarized app differ");
        }
        actual.insert(relative);
    }
    if !launcher || actual != expected.keys().cloned().collect() {
        return fail("CLI archive is incomplete");
    }
    Ok(())
}

pub struct VerifyArgs {
    pub release_dir: PathBuf,
    pub commit: String,
    pub version: String,
    pub candidate_id: Option<String>,
    pub smoke: bool,
}

/// `{relative path: sha256}` of every file but the code-signature index.
fn payload(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    for path in walk(root)? {
        if !path.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        if relative != "Contents/CodeResources" {
            out.insert(relative, sha256(&path)?);
        }
    }
    Ok(out)
}

pub fn verify(args: &VerifyArgs) -> Result<()> {
    let tmp = temp_dir("kassiber-verify-")?;
    let stage = tmp.path();
    let candidate = args.candidate_id.as_deref();
    let app = extract_app(&args.release_dir.join(APP_ZIP), &stage.join("zip"))?;
    verify_app(&app, &args.commit, &args.version, true, candidate)?;
    let image = args.release_dir.join(DMG);
    verify_code(&image, false)?;
    run(&[
        "/usr/bin/xcrun".into(),
        "stapler".into(),
        "validate".into(),
        arg(&image),
    ])?;
    run(&[
        "/usr/sbin/spctl".into(),
        "--assess".into(),
        "--type".into(),
        "open".into(),
        "--context".into(),
        "context:primary-signature".into(),
        "--verbose=4".into(),
        arg(&image),
    ])?;
    // Compare actual sealed resources, not just outer CDHash, across all
    // distributions. Stapled ticket bytes may differ inside the DMG.
    let dmg_stage = stage.join("dmg");
    std::fs::create_dir(&dmg_stage).map_err(os)?;
    let dmg_app = app_from_image(&image, &dmg_stage)?;
    verify_app(&dmg_app, &args.commit, &args.version, false, candidate)?;
    if payload(&app)? != payload(&dmg_app)? {
        return fail("DMG and app ZIP differ");
    }
    verify_cli_archive(&args.release_dir.join(CLI_TAR), &app)?;
    if args.smoke {
        // Only opt in on a disposable verification Mac, after signature and
        // ticket checks. Never execute build downloads on the signer.
        let launcher = app.join("Contents/Resources/bin/kassiber");
        let version = run(&[arg(&launcher), "--version".into()])?;
        if !version.contains(&args.commit[..args.commit.len().min(12)]) {
            return fail("Packaged CLI reported a different build");
        }
        run(&[arg(&launcher), "--help".into()])?;
        // Starting the entitled outer executable also exercises the OS
        // provisioning-profile launch check (the CLI sidecar is unentitled).
        run(&[
            arg(&app.join("Contents/MacOS/kassiber-ui")),
            "--cli".into(),
            "--version".into(),
        ])?;
    }
    Ok(())
}

// ---- prepare (prepare_macos_release.py) -------------------------------------

fn gh_json(args: &[&str]) -> Result<Json> {
    let mut command: Vec<std::ffi::OsString> = vec!["gh".into()];
    command.extend(args.iter().map(std::ffi::OsString::from));
    loads(&run(&command)?)
}

fn require_unsigned_draft(tag: &str, expected_commit: Option<&str>) -> Result<()> {
    let release = gh_json(&[
        "release",
        "view",
        tag,
        "--repo",
        REPO,
        "--json",
        "isDraft,assets",
    ])?;
    let draft = matches!(json_get(&release, "isDraft"), Some(Json::Bool(true)));
    let signed = match json_get(&release, "assets") {
        Some(Json::List(assets)) => assets
            .iter()
            .any(|a| json_str(json_get(a, "name")).is_some_and(|n| n.ends_with(".sig"))),
        _ => return fail("Release view returned no assets"),
    };
    if !draft || signed {
        return fail("Require an unsigned draft release");
    }
    if let Some(expected) = expected_commit {
        let commit = gh_json(&["api", &format!("repos/{REPO}/commits/{tag}")])?;
        if json_str(json_get(&commit, "sha")) != Some(expected) {
            return fail("Release tag changed after build verification");
        }
    }
    Ok(())
}

fn tag_shape(tag: &str) -> bool {
    tag.strip_prefix('v').is_some_and(|version| {
        crate::notes::extract_notes(&format!("## {version}\nx\n"), version).is_ok()
    })
}

pub struct PrepareArgs {
    pub tag: String,
    pub run_id: String,
    pub identity: String,
    pub provisioning_profile: PathBuf,
    pub work_dir: PathBuf,
    pub submit: bool,
}

pub fn prepare(args: &PrepareArgs) -> Result<()> {
    if !tag_shape(&args.tag) {
        return fail("Invalid release tag");
    }
    if !digits(&args.run_id) {
        return fail("Invalid build run ID");
    }
    require_unsigned_draft(&args.tag, None)?;
    let build = gh_json(&["api", &format!("repos/{REPO}/actions/runs/{}", args.run_id)])?;
    let commit_payload = gh_json(&["api", &format!("repos/{REPO}/commits/{}", args.tag)])?;
    let Some(commit) = json_str(json_get(&commit_payload, "sha")).map(str::to_owned) else {
        return fail("Commit lookup returned no sha");
    };
    let head_repository =
        json_get(&build, "head_repository").and_then(|r| json_str(json_get(r, "full_name")));
    if json_str(json_get(&build, "conclusion")) != Some("success")
        || json_str(json_get(&build, "head_sha")) != Some(commit.as_str())
        || !matches!(
            json_str(json_get(&build, "event")),
            Some("push" | "workflow_dispatch")
        )
        || json_str(json_get(&build, "path")) != Some(".github/workflows/prerelease-binaries.yml")
        || head_repository != Some(REPO)
    {
        return fail("Run must be a successful official release build of the exact tag commit");
    }
    mkdir_new(&args.work_dir)?;
    let download = args.work_dir.join("download");
    run(&[
        "gh".into(),
        "run".into(),
        "download".into(),
        args.run_id.as_str().into(),
        "--repo".into(),
        REPO.into(),
        "--name".into(),
        "kassiber-desktop-macos-arm64-preview".into(),
        "--dir".into(),
        arg(&download),
    ])?;
    let archive = download.join(APP_ZIP);
    let digest = sha256(&archive)?;
    let receipt = dict([
        ("tag", s(args.tag.as_str())),
        ("commit", s(commit.as_str())),
        ("build_run", s(args.run_id.as_str())),
        (
            "build_attempt",
            json_get(&build, "run_attempt")
                .cloned()
                .unwrap_or(Json::Null),
        ),
        ("unsigned_app_sha256", s(digest.as_str())),
    ]);
    let source = args.work_dir.join("source.json");
    std::fs::write(&source, format!("{}\n", dumps_indent(&receipt, false))).map_err(os)?;
    sign(&SignArgs {
        archive,
        sha256: digest,
        identity: args.identity.clone(),
        commit: commit.clone(),
        version: args.tag[1..].to_owned(),
        output: args.work_dir.join("signed"),
        source,
        provisioning_profile: args.provisioning_profile.clone(),
        candidate_id: None,
    })?;
    if args.submit {
        let image = args.work_dir.join("signed").join(INPUT_DMG);
        let image_digest = sha256(&image)?;
        // Signing and upload may take minutes. Refresh both boundaries; these
        // checks are not atomic with GitHub writes and do not replace tag protection.
        require_unsigned_draft(&args.tag, Some(&commit))?;
        // No clobber: changing an existing handoff requires explicit operator recovery.
        run(&[
            "gh".into(),
            "release".into(),
            "upload".into(),
            args.tag.as_str().into(),
            arg(&image),
            "--repo".into(),
            REPO.into(),
        ])?;
        require_unsigned_draft(&args.tag, Some(&commit))?;
        run(&[
            "gh".into(),
            "workflow".into(),
            "run".into(),
            "notarize-macos.yml".into(),
            "--repo".into(),
            REPO.into(),
            "--ref".into(),
            "main".into(),
            "-f".into(),
            format!("tag_name={}", args.tag).into(),
            "-f".into(),
            format!("input_sha256={image_digest}").into(),
        ])?;
        println!(
            "Notarization dispatched. Release remains a draft pending offline manifest signing."
        );
    }
    Ok(())
}

// ---- command line -----------------------------------------------------------

fn path(value: &str) -> PathBuf {
    kassiber_release::publish::py_path(Path::new(value))
}

pub fn main(argv: &[String]) -> std::result::Result<ExitCode, Usage> {
    let Some((command, rest)) = argv.split_first() else {
        return Err(usage("the following arguments are required: command"));
    };
    if command == "prepare" {
        let args = parse(
            rest,
            &[
                required("tag"),
                required("run-id"),
                required("identity"),
                required("provisioning-profile"),
                required("work-dir"),
                flag("submit"),
            ],
        )?;
        let result = prepare(&PrepareArgs {
            tag: args.req("tag").into(),
            run_id: args.req("run-id").into(),
            identity: args.req("identity").into(),
            provisioning_profile: path(args.req("provisioning-profile")),
            work_dir: path(args.req("work-dir")),
            submit: args.flag("submit"),
        });
        return Ok(match result {
            Ok(()) => ExitCode::SUCCESS,
            Err(Error::Value(message)) => {
                eprintln!("Release preparation failed: {message}; no release published.");
                ExitCode::from(1)
            }
            Err(Error::Os(_)) => {
                eprintln!("Release preparation failed (OSError); no release published.");
                ExitCode::from(1)
            }
            Err(Error::Process) => {
                eprintln!("Release preparation failed (CalledProcessError); no release published.");
                ExitCode::from(1)
            }
        });
    }
    let mut specs: Vec<Spec> = vec![
        required("commit"),
        required("version"),
        value("candidate-id"),
    ];
    match command.as_str() {
        "verify" => specs.extend([required("release-dir"), flag("smoke")]),
        "sign" | "notarize" => specs.extend([required("output"), required("sha256")]),
        other => {
            return Err(usage(format!(
                "argument command: invalid choice: '{other}' (choose from 'sign', 'notarize', 'verify', 'prepare')"
            )))
        }
    }
    if command == "sign" {
        specs.extend([
            required("archive"),
            required("identity"),
            required("source"),
            required("provisioning-profile"),
        ]);
    }
    if command == "notarize" {
        specs.extend([
            required("image"),
            required("profile"),
            required("keychain"),
            value("submission-id"),
            value("wait-seconds"),
            flag("pending-ok"),
        ]);
    }
    let args = parse(rest, &specs)?;
    let candidate_id = args.get("candidate-id").map(str::to_owned);
    let result = match command.as_str() {
        "sign" => sign(&SignArgs {
            archive: path(args.req("archive")),
            sha256: args.req("sha256").into(),
            identity: args.req("identity").into(),
            commit: args.req("commit").into(),
            version: args.req("version").into(),
            output: path(args.req("output")),
            source: path(args.req("source")),
            provisioning_profile: path(args.req("provisioning-profile")),
            candidate_id,
        }),
        "notarize" => {
            let wait = args.get("wait-seconds").unwrap_or("3000");
            let Ok(wait_seconds) = kassiber_pytext::strip(wait).parse::<i64>() else {
                return Err(usage(format!(
                    "argument --wait-seconds: invalid int value: '{wait}'"
                )));
            };
            notarize(&NotaryArgs {
                image: path(args.req("image")),
                sha256: args.req("sha256").into(),
                commit: args.req("commit").into(),
                version: args.req("version").into(),
                candidate_id,
                output: path(args.req("output")),
                profile: args.req("profile").into(),
                keychain: path(args.req("keychain")),
                submission_id: args.get("submission-id").unwrap_or("").into(),
                wait_seconds,
                pending_ok: args.flag("pending-ok"),
            })
        }
        _ => verify(&VerifyArgs {
            release_dir: path(args.req("release-dir")),
            commit: args.req("commit").into(),
            version: args.req("version").into(),
            candidate_id,
            smoke: args.flag("smoke"),
        }),
    };
    Ok(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // Never print subprocess arguments: tools may carry credentials.
            let (kind, message) = match error {
                Error::Value(message) => ("ValueError", message),
                Error::Os(message) => ("OSError", message),
                Error::Process => (
                    "CalledProcessError",
                    "platform tool rejected artifact; inspect locally".to_owned(),
                ),
            };
            eprintln!("macOS release {command} failed: {kind}: {message}");
            ExitCode::from(1)
        }
    })
}

#[cfg(test)]
mod tests;
