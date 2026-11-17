//! The user-local `kassiber` terminal command (src-tauri `terminal_command_*`): a small launcher
//! script in `~/.local/bin` or `~/bin` that runs this app as the CLI (`--cli`, see `cli`), plus
//! the managed PATH block in the user's shell profile when that folder isn't on PATH yet.
//! Settings › Terminal and the `--install-terminal-command` / `--remove-terminal-command` launch
//! flags both use it. Windows commands come from the installer.
//!
//! Every path comes from [`Env`], so tests run against a temporary home and never touch the
//! real one.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

pub const COMMAND_NAME: &str = "kassiber";
pub const MARKER: &str = "Kassiber desktop CLI launcher. Managed by Kassiber Settings.";
pub const PATH_BLOCK_START: &str = "# >>> kassiber terminal command >>>";
pub const PATH_BLOCK_END: &str = "# <<< kassiber terminal command <<<";
const WINDOWS_INSTALLER: &str =
    "Install Kassiber with the Windows installer to add its native CLI executable to PATH.";

/// The process environment the command is computed from.
#[derive(Clone, Debug)]
pub struct Env {
    pub home: Option<PathBuf>,
    pub shell: Option<PathBuf>,
    pub path: Option<std::ffi::OsString>,
    pub current_exe: Option<PathBuf>,
    pub appimage: Option<PathBuf>,
    pub local_app_data: Option<PathBuf>,
    /// Package-manager installs that own the command (`/opt/homebrew/bin/kassiber`, …).
    pub package_candidates: Vec<PathBuf>,
}

impl Env {
    pub fn current() -> Env {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        #[cfg(target_os = "macos")]
        let package_candidates = vec![
            PathBuf::from("/opt/homebrew/bin/kassiber"),
            PathBuf::from("/usr/local/bin/kassiber"),
        ];
        #[cfg(target_os = "linux")]
        let package_candidates = vec![
            PathBuf::from("/usr/bin/kassiber"),
            PathBuf::from("/usr/local/bin/kassiber"),
        ];
        #[cfg(target_os = "windows")]
        let package_candidates = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(|p| vec![p.join("bin").join("kassiber.exe")]))
            .unwrap_or_default();
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        let package_candidates = Vec::new();
        Env {
            home: var("HOME")
                .or_else(|| var("USERPROFILE"))
                .map(PathBuf::from),
            shell: var("SHELL").map(PathBuf::from),
            path: var("PATH"),
            current_exe: std::env::current_exe().ok(),
            appimage: var("APPIMAGE").map(PathBuf::from),
            local_app_data: var("LOCALAPPDATA").map(PathBuf::from),
            package_candidates,
        }
    }

    fn on_path(&self, dir: &Path) -> bool {
        self.path
            .as_ref()
            .is_some_and(|p| std::env::split_paths(p).any(|c| c == dir))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub platform: &'static str,
    pub available: bool,
    pub installed: bool,
    pub managed: bool,
    pub needs_repair: bool,
    pub conflict: bool,
    pub path_on_path: bool,
    pub command: String,
    pub bin_dir: String,
    pub command_path: String,
    pub target_path: String,
    pub path_hint: String,
    pub message: String,
}

struct Paths {
    platform: &'static str,
    bin_dir: PathBuf,
    command_path: PathBuf,
    target_path: PathBuf,
}

fn platform() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unsupported"
    }
}

fn filename() -> &'static str {
    if cfg!(target_os = "windows") {
        "kassiber.exe"
    } else {
        COMMAND_NAME
    }
}

/// `bundled_macos_terminal_launcher`: `Contents/Resources/bin/kassiber` beside `Contents/MacOS`.
fn bundled_macos_launcher(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    if macos.file_name()? != "MacOS" {
        return None;
    }
    let launcher = macos
        .parent()?
        .join("Resources")
        .join("bin")
        .join(COMMAND_NAME);
    launcher.is_file().then_some(launcher)
}

/// `terminal_command_target_path`: the AppImage (Linux), the bundle's console launcher (macOS),
/// else this executable.
fn target_path(env: &Env) -> Result<PathBuf, String> {
    if cfg!(target_os = "linux")
        && let Some(a) = &env.appimage
    {
        return Ok(a.clone());
    }
    let exe = env
        .current_exe
        .clone()
        .ok_or_else(|| "Could not locate the Kassiber desktop executable.".to_owned())?;
    if cfg!(target_os = "macos")
        && let Some(l) = bundled_macos_launcher(&exe)
    {
        return Ok(l);
    }
    Ok(exe)
}

fn paths(env: &Env) -> Result<Paths, String> {
    let target_path = target_path(env)?;
    let home = env.home.clone().ok_or_else(|| {
        "Could not locate your home folder for a user-owned terminal command.".to_owned()
    })?;
    let candidates = if cfg!(target_os = "windows") {
        let mut c = Vec::new();
        if let Some(l) = &env.local_app_data {
            c.push(l.join("Kassiber").join("bin"));
        }
        c.push(home.join(".kassiber").join("bin"));
        c
    } else {
        vec![home.join(".local").join("bin"), home.join("bin")]
    };
    let bin_dir = candidates
        .iter()
        .find(|p| env.on_path(p))
        .cloned()
        .or_else(|| candidates.first().cloned())
        .ok_or_else(|| "Could not choose a user-owned terminal command directory.".to_owned())?;
    Ok(Paths {
        platform: platform(),
        command_path: bin_dir.join(filename()),
        bin_dir,
        target_path,
    })
}

fn quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', "'\"'\"'"))
}

/// `bundled_terminal_launcher_path`: `…/Resources/bin/kassiber`, which needs no `--cli`.
fn bundled_launcher_path(p: &Path) -> bool {
    let name = |p: Option<&Path>| {
        p.and_then(Path::file_name)
            .and_then(|n| n.to_str())
            .map(str::to_owned)
    };
    name(Some(p)).as_deref() == Some(COMMAND_NAME)
        && name(p.parent()).as_deref() == Some("bin")
        && name(p.parent().and_then(Path::parent)).as_deref() == Some("Resources")
}

/// `terminal_command_contents`.
pub fn contents(target: &Path) -> String {
    let cli_flag = if bundled_launcher_path(target) {
        ""
    } else {
        " --cli"
    };
    let t = target.to_string_lossy();
    format!(
        "#!/bin/sh\n# {MARKER}\n# target: {t}\nexec {}{cli_flag} \"$@\"\n",
        quote(&t)
    )
}

fn transient(target: &Path) -> bool {
    let t = target.to_string_lossy();
    t.starts_with("/Volumes/") || t.contains("/AppTranslocation/")
}

/// `terminal_command_target_is_available`: not from a disk image or a translocated copy (macOS).
fn target_available(target: &Path) -> bool {
    !(cfg!(target_os = "macos") && transient(target))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileState {
    Missing,
    Current,
    ManagedStale,
    Conflict,
}

fn inspect(p: &Paths) -> Result<FileState, String> {
    if !cfg!(target_os = "windows")
        && std::fs::symlink_metadata(&p.command_path).is_ok_and(|m| m.file_type().is_symlink())
    {
        let target = std::fs::read_link(&p.command_path)
            .map_err(|e| format!("Could not inspect terminal command symlink: {e}"))?;
        // A direct symlink works, but the installer normalizes it to the script form.
        return Ok(if target == p.target_path {
            FileState::ManagedStale
        } else {
            FileState::Conflict
        });
    }
    match std::fs::read_to_string(&p.command_path) {
        Ok(c) if c == contents(&p.target_path) => Ok(FileState::Current),
        Ok(c) if c.contains(MARKER) => Ok(FileState::ManagedStale),
        Ok(_) => Ok(FileState::Conflict),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(FileState::Missing),
        Err(e) => Err(format!("Could not inspect terminal command: {e}")),
    }
}

/// `terminal_command_message`.
fn message(
    available: bool,
    installed: bool,
    repair: bool,
    conflict: bool,
    on_path: bool,
) -> &'static str {
    if !available {
        "Move Kassiber into Applications before installing the terminal command."
    } else if conflict {
        "A different command already exists at the install path."
    } else if repair {
        "The terminal command is managed by Kassiber but points at an older app path."
    } else if installed && on_path {
        "The terminal command is installed and appears on PATH."
    } else if installed {
        "The terminal command is installed; add its folder to PATH if your shell cannot find it."
    } else {
        "Install the user-local terminal command to run kassiber from a shell."
    }
}

/// `terminal_command_profile_path`: the login profile the user's shell reads.
pub fn profile_path(home: &Path, shell: Option<&Path>) -> PathBuf {
    match shell.and_then(Path::file_name).and_then(|n| n.to_str()) {
        Some("fish") => home
            .join(".config")
            .join("fish")
            .join("conf.d")
            .join("kassiber.fish"),
        Some("zsh") => home.join(".zprofile"),
        Some("bash") if home.join(".bash_profile").exists() => home.join(".bash_profile"),
        Some("bash") => home.join(".profile"),
        _ if cfg!(target_os = "macos") => home.join(".zprofile"),
        _ => home.join(".profile"),
    }
}

/// `terminal_command_profile_block`.
pub fn profile_block(bin_dir: &Path, fish: bool) -> String {
    let q = quote(&bin_dir.to_string_lossy());
    if fish {
        format!(
            "{PATH_BLOCK_START}\nif not contains -- {q} $PATH\n    set -gx PATH {q} $PATH\nend\n{PATH_BLOCK_END}"
        )
    } else {
        format!(
            "{PATH_BLOCK_START}\ncase \":$PATH:\" in\n  *:{q}:*) ;;\n  *) export PATH={q}:\"$PATH\" ;;\nesac\n{PATH_BLOCK_END}"
        )
    }
}

/// `terminal_command_profile_without_block`.
pub fn profile_without_block(contents: &str) -> Result<String, String> {
    match (contents.find(PATH_BLOCK_START), contents.find(PATH_BLOCK_END)) {
        (None, None) => Ok(contents.to_owned()),
        (Some(start), Some(end)) if end >= start => {
            let end = end + PATH_BLOCK_END.len();
            let before = contents[..start].trim_end_matches('\n');
            let after = contents[end..].trim_start_matches('\n');
            Ok(match (before.is_empty(), after.is_empty()) {
                (true, true) => String::new(),
                (false, true) => format!("{before}\n"),
                (true, false) => format!("{after}\n"),
                (false, false) => format!("{before}\n\n{after}\n"),
            })
        }
        _ => Err("The shell profile contains an incomplete Kassiber PATH block; repair it manually before continuing.".to_owned()),
    }
}

/// `terminal_command_profile_update`: drop the managed block, and add it back unless removing.
pub fn profile_update(profile: &Path, bin_dir: &Path, remove: bool) -> Result<(), String> {
    let existing = match std::fs::read_to_string(profile) {
        Ok(c) => c,
        Err(e) if e.kind() == ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("Could not read shell profile: {e}")),
    };
    let mut updated = profile_without_block(&existing)?;
    if !remove {
        let fish = profile.extension().and_then(|e| e.to_str()) == Some("fish");
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        if !updated.is_empty() {
            updated.push('\n');
        }
        updated.push_str(&profile_block(bin_dir, fish));
        updated.push('\n');
    }
    if let Some(parent) = profile.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create shell profile directory: {e}"))?;
    }
    std::fs::write(profile, updated).map_err(|e| format!("Could not update shell profile: {e}"))
}

fn selected_profile(env: &Env) -> Result<PathBuf, String> {
    let home = env
        .home
        .as_deref()
        .ok_or_else(|| "Could not locate your home folder.".to_owned())?;
    Ok(profile_path(home, env.shell.as_deref()))
}

/// `terminal_command_path_is_managed`: the selected profile carries the block for `bin_dir`.
fn path_is_managed(env: &Env, bin_dir: &Path) -> bool {
    if cfg!(target_os = "windows") {
        return false;
    }
    selected_profile(env)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .is_some_and(|c| {
            c.contains(&profile_block(bin_dir, false)) || c.contains(&profile_block(bin_dir, true))
        })
}

/// `terminal_command_path_hint`.
fn path_hint(env: &Env, bin_dir: &Path) -> String {
    if cfg!(target_os = "windows") {
        return format!("Add {} to your user PATH.", bin_dir.display());
    }
    let shown = env
        .home
        .as_deref()
        .and_then(|h| bin_dir.strip_prefix(h).ok())
        .map(|r| format!("$HOME/{}", r.to_string_lossy()))
        .unwrap_or_else(|| bin_dir.to_string_lossy().into_owned());
    format!("export PATH=\"{shown}:$PATH\"")
}

/// `terminal_command_status` (local file reads only).
pub fn status(env: &Env) -> Result<Status, String> {
    let p = paths(env)?;
    let s = |p: &Path| p.to_string_lossy().into_owned();
    if let Some(cmd) = env.package_candidates.iter().find(|c| c.is_file()) {
        return Ok(Status {
            platform: p.platform,
            available: false,
            installed: true,
            path_on_path: true,
            command: COMMAND_NAME.into(),
            bin_dir: cmd.parent().map(s).unwrap_or_default(),
            command_path: s(cmd),
            target_path: s(&p.target_path),
            message: "The terminal command is installed and managed by your installer or package manager.".into(),
            ..Status::default()
        });
    }
    if cfg!(target_os = "windows") {
        return Ok(Status {
            platform: p.platform,
            command: COMMAND_NAME.into(),
            bin_dir: s(&p.bin_dir),
            command_path: s(&p.command_path),
            target_path: s(&p.target_path),
            message: WINDOWS_INSTALLER.into(),
            ..Status::default()
        });
    }
    let state = inspect(&p)?;
    let available = target_available(&p.target_path);
    let on_path = env.on_path(&p.bin_dir) || path_is_managed(env, &p.bin_dir);
    let installed = state == FileState::Current;
    let needs_repair = state == FileState::ManagedStale;
    let conflict = state == FileState::Conflict;
    Ok(Status {
        platform: p.platform,
        available,
        installed,
        managed: installed || needs_repair,
        needs_repair,
        conflict,
        path_on_path: on_path,
        command: COMMAND_NAME.into(),
        bin_dir: s(&p.bin_dir),
        command_path: s(&p.command_path),
        target_path: s(&p.target_path),
        path_hint: path_hint(env, &p.bin_dir),
        message: message(available, installed, needs_repair, conflict, on_path).into(),
    })
}

/// `install_terminal_command`: write (or repair) the launcher, then make sure its folder is on
/// PATH through the selected shell profile.
pub fn install(env: &Env) -> Result<(), String> {
    let p = paths(env)?;
    if env.package_candidates.iter().any(|c| c.is_file()) {
        return Ok(());
    }
    if cfg!(target_os = "windows") {
        return Err(WINDOWS_INSTALLER.into());
    }
    if !target_available(&p.target_path) {
        return Err("Move Kassiber out of the disk image and into Applications before installing the terminal command.".into());
    }
    match inspect(&p)? {
        FileState::Missing | FileState::Current => {}
        FileState::ManagedStale => std::fs::remove_file(&p.command_path)
            .map_err(|e| format!("Could not replace terminal command: {e}"))?,
        FileState::Conflict => {
            return Err(format!(
                "{} already exists and is not managed by Kassiber. Move it aside first if you want Kassiber to install this command there.",
                p.command_path.display()
            ));
        }
    }
    if inspect(&p)? != FileState::Current {
        std::fs::create_dir_all(&p.bin_dir)
            .map_err(|e| format!("Could not create terminal command directory: {e}"))?;
        std::fs::write(&p.command_path, contents(&p.target_path))
            .map_err(|e| format!("Could not write terminal command: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let set = std::fs::metadata(&p.command_path).and_then(|m| {
                let mut perm = m.permissions();
                perm.set_mode(0o755);
                std::fs::set_permissions(&p.command_path, perm)
            });
            set.map_err(|e| format!("Could not make terminal command executable: {e}"))?;
        }
    }
    // `ensure_terminal_command_path`.
    if env.on_path(&p.bin_dir) {
        return Ok(());
    }
    profile_update(&selected_profile(env)?, &p.bin_dir, false)
}

/// `remove_terminal_command`: delete a launcher Kassiber manages and its PATH blocks.
pub fn remove(env: &Env) -> Result<(), String> {
    let p = paths(env)?;
    if cfg!(target_os = "windows") {
        return Err(
            "The native Windows CLI executable is managed by the Kassiber installer.".into(),
        );
    }
    match inspect(&p)? {
        FileState::Missing => {}
        FileState::Current | FileState::ManagedStale => std::fs::remove_file(&p.command_path)
            .map_err(|e| format!("Could not remove terminal command: {e}"))?,
        FileState::Conflict => {
            return Err(format!(
                "{} is not managed by Kassiber, so it was left untouched.",
                p.command_path.display()
            ));
        }
    }
    // `remove_terminal_command_path`: every profile the installer may have written to.
    let home = env
        .home
        .as_deref()
        .ok_or_else(|| "Could not locate your home folder.".to_owned())?;
    for profile in [
        home.join(".zprofile"),
        home.join(".bash_profile"),
        home.join(".profile"),
        home.join(".config")
            .join("fish")
            .join("conf.d")
            .join("kassiber.fish"),
    ] {
        if profile.exists() {
            profile_update(&profile, &p.bin_dir, true)?;
        }
    }
    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
pub enum HelperAction {
    Install,
    Remove,
}

/// `terminal_command_helper_action`: the flag alone, nothing after it.
pub fn helper_action(args: &[String]) -> Option<Result<HelperAction, String>> {
    let action = match args.first().map(String::as_str) {
        Some("--install-terminal-command") => HelperAction::Install,
        Some("--remove-terminal-command") => HelperAction::Remove,
        _ => return None,
    };
    if args.len() != 1 {
        return Some(Err(
            "terminal-command helper flags do not accept additional arguments".into(),
        ));
    }
    Some(Ok(action))
}

/// `run_terminal_command_helper`: the launch flags' exit code, printing the status.
pub fn run_helper(args: &[String]) -> Option<i32> {
    let action = helper_action(args)?;
    let env = Env::current();
    let result = action.and_then(|a| {
        match a {
            HelperAction::Install => install(&env),
            HelperAction::Remove => remove(&env),
        }?;
        status(&env)
    });
    Some(match result {
        Ok(s) => {
            println!("{}", s.message);
            println!("Command: {}", s.command_path);
            0
        }
        Err(e) => {
            eprintln!("kassiber terminal command: {e}");
            1
        }
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn home(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("kassiber-rune-term-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn env(home: &Path, shell: &str, path: &str) -> Env {
        Env {
            home: Some(home.to_path_buf()),
            shell: Some(PathBuf::from(shell)),
            path: Some(path.into()),
            current_exe: Some(PathBuf::from(
                "/Applications/Kassiber.app/Contents/MacOS/Kassiber",
            )),
            appimage: None,
            local_app_data: None,
            package_candidates: vec![],
        }
    }

    #[test]
    fn launcher_targets_the_desktop_executable_with_cli_flag() {
        let c = contents(Path::new(
            "/Applications/Kassiber.app/Contents/MacOS/Kassiber",
        ));
        assert_eq!(
            c,
            format!(
                "#!/bin/sh\n# {MARKER}\n# target: /Applications/Kassiber.app/Contents/MacOS/Kassiber\nexec '/Applications/Kassiber.app/Contents/MacOS/Kassiber' --cli \"$@\"\n"
            )
        );
        let c = contents(Path::new(
            "/Applications/Kassiber.app/Contents/Resources/bin/kassiber",
        ));
        assert!(c.ends_with(
            "exec '/Applications/Kassiber.app/Contents/Resources/bin/kassiber' \"$@\"\n"
        ));
        assert_eq!(quote("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn helper_flags_are_explicit_and_argument_free() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            helper_action(&v(&["--install-terminal-command"])),
            Some(Ok(HelperAction::Install))
        );
        assert_eq!(
            helper_action(&v(&["--remove-terminal-command"])),
            Some(Ok(HelperAction::Remove))
        );
        assert!(matches!(
            helper_action(&v(&["--install-terminal-command", "x"])),
            Some(Err(_))
        ));
        assert_eq!(helper_action(&v(&["status"])), None);
        assert_eq!(helper_action(&[]), None);
    }

    #[test]
    fn profile_update_is_idempotent_and_removes_only_managed_block() {
        let h = home("profile");
        let profile = h.join(".zprofile");
        let bin = h.join(".local/bin");
        std::fs::write(&profile, "export A=1").unwrap();
        profile_update(&profile, &bin, false).unwrap();
        profile_update(&profile, &bin, false).unwrap();
        let installed = std::fs::read_to_string(&profile).unwrap();
        assert_eq!(installed.matches(PATH_BLOCK_START).count(), 1);
        assert!(installed.starts_with("export A=1\n\n# >>> kassiber terminal command >>>\n"));
        profile_update(&profile, &bin, true).unwrap();
        assert_eq!(std::fs::read_to_string(&profile).unwrap(), "export A=1\n");
        std::fs::write(&profile, format!("x\n{PATH_BLOCK_START}\n")).unwrap();
        assert!(profile_update(&profile, &bin, false).is_err());
        let fish = profile_block(&bin, true);
        assert!(fish.contains("set -gx PATH"));
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn profile_follows_the_login_shell() {
        let h = home("shell");
        assert_eq!(
            profile_path(&h, Some(Path::new("/bin/zsh"))),
            h.join(".zprofile")
        );
        assert_eq!(
            profile_path(&h, Some(Path::new("/usr/bin/fish"))),
            h.join(".config/fish/conf.d/kassiber.fish")
        );
        assert_eq!(
            profile_path(&h, Some(Path::new("/bin/bash"))),
            h.join(".profile")
        );
        std::fs::write(h.join(".bash_profile"), "").unwrap();
        assert_eq!(
            profile_path(&h, Some(Path::new("/bin/bash"))),
            h.join(".bash_profile")
        );
        let _ = std::fs::remove_dir_all(&h);
    }

    #[test]
    fn install_status_and_remove_round_trip_in_a_temporary_home() {
        let h = home("install");
        let e = env(&h, "/bin/zsh", "/usr/bin:/bin");
        let s = status(&e).unwrap();
        assert!(!s.installed && !s.path_on_path && !s.conflict);
        assert_eq!(
            s.message,
            "Install the user-local terminal command to run kassiber from a shell."
        );
        assert_eq!(s.path_hint, "export PATH=\"$HOME/.local/bin:$PATH\"");
        install(&e).unwrap();
        let s = status(&e).unwrap();
        // Off PATH, the managed profile block counts as on PATH.
        assert!(s.installed && s.managed && s.path_on_path, "{s:?}");
        assert_eq!(
            s.message,
            "The terminal command is installed and appears on PATH."
        );
        let profile = std::fs::read_to_string(h.join(".zprofile")).unwrap();
        assert!(profile.contains(&profile_block(&h.join(".local/bin"), false)));
        let launcher = h.join(".local/bin/kassiber");
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&launcher).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        // Another app version: the managed launcher needs a repair, and install repairs it.
        let moved = Env {
            current_exe: Some(PathBuf::from(
                "/Applications/Kassiber 2.app/Contents/MacOS/Kassiber",
            )),
            ..e.clone()
        };
        assert!(status(&moved).unwrap().needs_repair);
        install(&moved).unwrap();
        assert!(status(&moved).unwrap().installed);
        remove(&moved).unwrap();
        assert!(!launcher.exists());
        assert!(
            !std::fs::read_to_string(h.join(".zprofile"))
                .unwrap()
                .contains(PATH_BLOCK_START)
        );
        // A foreign command is never overwritten or removed.
        std::fs::create_dir_all(h.join(".local/bin")).unwrap();
        std::fs::write(&launcher, "#!/bin/sh\necho mine\n").unwrap();
        assert!(status(&e).unwrap().conflict);
        assert!(install(&e).is_err() && remove(&e).is_err());
        assert_eq!(
            std::fs::read_to_string(&launcher).unwrap(),
            "#!/bin/sh\necho mine\n"
        );
        // A folder already on PATH needs no profile edit.
        std::fs::remove_file(&launcher).unwrap();
        let on = env(
            &h,
            "/bin/zsh",
            &format!("{}:/usr/bin", h.join(".local/bin").display()),
        );
        std::fs::remove_file(h.join(".zprofile")).ok();
        install(&on).unwrap();
        assert!(!h.join(".zprofile").exists());
        let _ = std::fs::remove_dir_all(&h);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_disk_image_copy_cannot_install() {
        let h = home("dmg");
        let e = Env {
            current_exe: Some(PathBuf::from(
                "/Volumes/Kassiber/Kassiber.app/Contents/MacOS/Kassiber",
            )),
            ..env(&h, "/bin/zsh", "/bin")
        };
        assert!(!status(&e).unwrap().available);
        assert!(install(&e).is_err());
        let _ = std::fs::remove_dir_all(&h);
    }
}
