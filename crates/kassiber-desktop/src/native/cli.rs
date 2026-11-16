//! The installed app doubles as the `kassiber` CLI (`desktop_cli_args` + `supervisor::run_cli`
//! in src-tauri), and answers its launch-only helper flags (`--install-terminal-command`,
//! `--remove-terminal-command`, `--operator-native-auth`) before any window or daemon starts.

use std::path::{Path, PathBuf};
use std::process::Command;

/// What `main` does before the app starts: exit with this code instead of opening a window.
pub fn early_exit() -> Option<i32> {
    let args = process_args();
    if let Some(code) = operator_native_auth(&args) {
        return Some(code);
    }
    if let Some(code) = super::terminal::run_helper(&args) {
        return Some(code);
    }
    // Forward before the window exists, so `kassiber` stays a real CLI in headless shells, SSH
    // sessions and CI: nothing here starts a window or the desktop daemon.
    let exe_is_kassiber = std::env::current_exe()
        .ok()
        .as_deref()
        .is_some_and(exe_stem_is_kassiber);
    desktop_cli_args(&args, exe_is_kassiber).map(run_cli)
}

/// The process arguments after the program, without macOS's `-psn_…` launch argument.
pub fn process_args() -> Vec<String> {
    std::env::args()
        .skip(1)
        .filter(|a| !a.starts_with("-psn_"))
        .collect()
}

/// `exe_stem_is_kassiber`: `kassiber`, `Kassiber` (the macOS bundle's executable), `kassiber.exe`.
pub fn exe_stem_is_kassiber(exe: &Path) -> bool {
    exe.file_stem()
        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("kassiber"))
}

/// `desktop_cli_args`: `--cli …` / `cli …` always forwards; any arguments forward when the
/// executable itself is named `kassiber`. No arguments opens the app.
pub fn desktop_cli_args(args: &[String], exe_is_kassiber: bool) -> Option<Vec<String>> {
    if args.is_empty() {
        return None;
    }
    if args[0] == "--cli" || args[0] == "cli" {
        return Some(args[1..].to_vec());
    }
    exe_is_kassiber.then(|| args.to_vec())
}

/// `--operator-native-auth`: the operator broker's Touch ID helper. This build has no
/// Keychain/biometric bridge (it needs a signed bundle with a provisioning profile), so it
/// answers as Tauri does where Touch ID is unavailable instead of opening a window.
fn operator_native_auth(args: &[String]) -> Option<i32> {
    (args.first().map(String::as_str) == Some("--operator-native-auth")).then(|| {
        eprintln!("kassiber native auth: Touch ID is unavailable in this build");
        1
    })
}

/// The `kassiber` binary and the working directory to run it in, resolved as the desktop
/// daemon's (`resolve_kassiber_command`): `KASSIBER_BIN`, the bundled sidecar, the checkout's
/// build. `source` names the one used, for the failure hint.
pub struct Resolved {
    pub program: PathBuf,
    pub cwd: PathBuf,
    pub source: &'static str,
}

/// `sidecar_filename`.
pub fn sidecar_filename() -> Option<String> {
    let triple = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    };
    let ext = if cfg!(windows) { ".exe" } else { "" };
    Some(format!("kassiber-cli-{triple}{ext}"))
}

/// `bundled_sidecar`: the macOS one-dir layout, the `binaries/` one, the bare file.
pub fn bundled_sidecar(resource_dir: &Path) -> Option<PathBuf> {
    let name = sidecar_filename()?;
    [
        resource_dir
            .join("binaries")
            .join("kassiber-cli")
            .join(&name),
        resource_dir.join("binaries").join(&name),
        resource_dir.join(&name),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// `resource_dir_before_tauri`'s candidates: the AppImage, the macOS bundle's Resources, the
/// Windows install folder, `/usr/lib/Kassiber`.
pub fn resource_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = std::env::var_os("APPDIR").filter(|v| !v.is_empty()) {
        out.push(PathBuf::from(dir).join("usr").join("lib").join("Kassiber"));
    }
    if let Some(parent) = std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(Path::parent)
    {
        #[cfg(target_os = "macos")]
        if let Some(contents) = parent.parent() {
            out.push(contents.join("Resources"));
        }
        #[cfg(target_os = "windows")]
        out.push(parent.to_path_buf());
        let _ = parent;
    }
    #[cfg(target_os = "linux")]
    out.push(PathBuf::from("/usr/lib/Kassiber"));
    out
}

/// The checkout's `target/{release,debug}/kassiber`, as this app's daemon launch finds it.
fn checkout_build() -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target");
    let exe = format!("kassiber{}", std::env::consts::EXE_SUFFIX);
    ["release", "debug"]
        .into_iter()
        .map(|p| root.join(p).join(&exe))
        .find(|p| p.is_file())
}

pub fn resolve(
    env_bin: Option<std::ffi::OsString>,
    resource_dirs: &[PathBuf],
    checkout: Option<PathBuf>,
) -> Result<Resolved, (String, &'static str)> {
    if let Some(bin) = env_bin.filter(|v| !v.is_empty()) {
        return Ok(Resolved {
            program: PathBuf::from(bin),
            cwd: std::env::temp_dir(),
            source: "env_bin",
        });
    }
    if let Some(sidecar) = resource_dirs.iter().find_map(|d| bundled_sidecar(d)) {
        // A bundle's Resources folder is read-only; the CLI gets a writable scratch cwd.
        return Ok(Resolved {
            program: sidecar,
            cwd: std::env::temp_dir(),
            source: "bundled_sidecar",
        });
    }
    if let Some(bin) = checkout {
        let cwd = bin
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir);
        return Ok(Resolved {
            program: bin,
            cwd,
            source: "checkout_build",
        });
    }
    Err((
        "Kassiber daemon binary not found: the bundled kassiber sidecar is missing".into(),
        "Reinstall the desktop package, or set KASSIBER_BIN to a kassiber binary.",
    ))
}

/// The binary this app runs as its CLI.
pub fn kassiber_command() -> Result<Resolved, (String, &'static str)> {
    resolve(
        std::env::var_os("KASSIBER_BIN"),
        &resource_dirs(),
        checkout_build().filter(|_| cfg!(feature = "tooling")),
    )
}

fn failure_hint(source: &str) -> &'static str {
    match source {
        "bundled_sidecar" => {
            "The bundled Kassiber CLI sidecar failed to start; reinstall the desktop package or set KASSIBER_BIN to override it."
        }
        "env_bin" => "Set KASSIBER_BIN to a working kassiber binary, or unset it.",
        _ => {
            "Rebuild the kassiber binary with `cargo build -p kassiber --bin kassiber`, or set KASSIBER_BIN."
        }
    }
}

/// `run_cli`: the CLI with this process's stdio, its exit code ours. The operator broker's
/// native-auth helper is not offered (`KASSIBER_NATIVE_AUTH_HELPER` stays unset): this build
/// can't do Touch ID, so the CLI keeps its own passphrase path.
pub fn run_cli(args: Vec<String>) -> i32 {
    let cmd = match kassiber_command() {
        Ok(c) => c,
        Err((message, hint)) => {
            eprintln!("{message}");
            eprintln!("{hint}");
            return 1;
        }
    };
    match Command::new(&cmd.program)
        .args(&args)
        .current_dir(&cmd.cwd)
        .status()
    {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("Could not start Kassiber CLI with {:?}: {e}", cmd.program);
            eprintln!("{}", failure_hint(cmd.source));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn cli_forwarding_decides_as_tauri() {
        assert_eq!(desktop_cli_args(&[], true), None);
        assert_eq!(
            desktop_cli_args(&v(&["--cli", "status"]), false),
            Some(v(&["status"]))
        );
        assert_eq!(
            desktop_cli_args(&v(&["cli", "--format", "json", "status"]), false),
            Some(v(&["--format", "json", "status"]))
        );
        assert_eq!(desktop_cli_args(&v(&["status"]), false), None);
        assert_eq!(
            desktop_cli_args(&v(&["status"]), true),
            Some(v(&["status"]))
        );
        assert_eq!(desktop_cli_args(&v(&["--cli"]), false), Some(vec![]));
        assert!(exe_stem_is_kassiber(Path::new(
            "/Applications/Kassiber.app/Contents/MacOS/Kassiber"
        )));
        assert!(exe_stem_is_kassiber(Path::new(
            "C:/Program Files/Kassiber/kassiber.exe"
        )));
        assert!(!exe_stem_is_kassiber(Path::new("/x/kassiber-desktop")));
    }

    #[test]
    fn resolution_prefers_the_override_then_the_bundle() {
        let dir = std::env::temp_dir().join(format!("kassiber-rune-cli-{}", std::process::id()));
        let bundle = dir.join("Resources");
        let name = sidecar_filename().unwrap();
        std::fs::create_dir_all(bundle.join("binaries").join("kassiber-cli")).unwrap();
        // A directory where the sidecar would be is not a sidecar.
        std::fs::create_dir_all(bundle.join("binaries").join(&name)).unwrap();
        assert!(bundled_sidecar(&bundle).is_none());
        std::fs::write(
            bundle.join("binaries").join("kassiber-cli").join(&name),
            b"",
        )
        .unwrap();
        let r = resolve(
            None,
            std::slice::from_ref(&bundle),
            Some("/x/target/release/kassiber".into()),
        )
        .ok()
        .unwrap();
        assert_eq!(r.source, "bundled_sidecar");
        let r = resolve(
            Some("/bin/echo".into()),
            std::slice::from_ref(&bundle),
            None,
        )
        .ok()
        .unwrap();
        assert_eq!(
            (r.source, r.program),
            ("env_bin", PathBuf::from("/bin/echo"))
        );
        let r = resolve(None, &[], Some("/x/target/release/kassiber".into()))
            .ok()
            .unwrap();
        assert_eq!((r.source, r.cwd), ("checkout_build", PathBuf::from("/x")));
        assert!(resolve(Some("".into()), &[], None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
