//! One running Kassiber (`tauri_plugin_single_instance`): a second launch hands its links to the
//! running app, which comes to the front, and exits.
//!
//! Windows and Linux use Rune's `single_instance` (a named pipe / a Unix socket in the user's
//! runtime folder); the links arrive through `App::on_open_url`, and a launch without links
//! brings the window forward (Rune's reopen default). macOS keeps one process per bundle itself,
//! and hands links to it as Apple events, but a second copy started directly (the executable
//! from a shell, `open -n`) would run beside the first; Rune's `single_instance` says yes there.
//! So on macOS this claims a Unix socket in the user's private temporary folder
//! (`$TMPDIR`, mode 0700): a later launch connects, sends its arguments, waits for the answer and
//! exits; the first reads them on a thread of its own and hands the links over.
//!
//! Agent tooling (headless renders, scripts) never claims or forwards: it runs beside the app.

pub const ID: &str = super::notify::APP_ID;

/// Makes this process the app's only one. False when another one took this launch: return
/// from `main` without a window.
pub fn claim() -> bool {
    if rune::is_headless() || crate::persist::file::tooling_run() {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        mac::claim()
    }
    #[cfg(not(target_os = "macos"))]
    {
        rune::single_instance(ID)
    }
}

/// What a later launch sends: `MAGIC`, then its arguments separated by NUL.
const MAGIC: &[u8] = b"kassiber-instance 1\0";
const ACCEPTED: u8 = b'A';
const MAX_MESSAGE: u64 = 1 << 20;

/// The arguments in a message, or None for bytes that aren't one.
pub fn decode(msg: &[u8]) -> Option<Vec<String>> {
    let body = msg.strip_prefix(MAGIC)?;
    let text = std::str::from_utf8(body).ok()?;
    Some(
        text.split('\0')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}

pub fn encode(args: &[String]) -> Vec<u8> {
    let mut m = MAGIC.to_vec();
    m.extend_from_slice(args.join("\0").as_bytes());
    m
}

#[cfg(target_os = "macos")]
mod mac {
    use std::io::{Read, Write};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::*;

    fn socket_path() -> PathBuf {
        // `$TMPDIR` is the per-user `/var/folders/…/T/` (0700) under macOS.
        std::env::temp_dir().join(format!("{ID}.instance.sock"))
    }

    /// Hands this launch to a running app. True when it took it.
    fn forward(path: &std::path::Path) -> bool {
        let Ok(mut s) = UnixStream::connect(path) else {
            return false;
        };
        let _ = s.set_write_timeout(Some(Duration::from_secs(2)));
        let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
        let args: Vec<String> = std::env::args().skip(1).collect();
        if s.write_all(&encode(&args)).is_err() || s.shutdown(std::net::Shutdown::Write).is_err() {
            return false;
        }
        let mut answer = [0u8; 1];
        s.read_exact(&mut answer).is_ok() && answer[0] == ACCEPTED
    }

    pub fn claim() -> bool {
        let path = socket_path();
        for _ in 0..2 {
            if forward(&path) {
                return false;
            }
            // Nobody answers: a socket left by a process that died. Take it over.
            let _ = std::fs::remove_file(&path);
            match UnixListener::bind(&path) {
                Ok(listener) => {
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ =
                            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
                    }
                    let _ = std::thread::Builder::new()
                        .name("kassiber-instance".into())
                        .spawn(move || serve(listener));
                    return true;
                }
                // Another launch bound it between our tries: hand this one to it.
                Err(_) => continue,
            }
        }
        // Can't reach or claim it: open a window rather than lose the launch.
        true
    }

    fn serve(listener: UnixListener) {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { continue };
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let mut msg = Vec::new();
            if (&mut s).take(MAX_MESSAGE).read_to_end(&mut msg).is_err() {
                continue;
            }
            let Some(args) = decode(&msg) else { continue };
            let _ = s.write_all(&[ACCEPTED]);
            super::super::relaunched(args);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_and_strangers_are_refused() {
        let args = vec!["kassiber://settings/privacy".to_owned(), "--x".to_owned()];
        assert_eq!(decode(&encode(&args)), Some(args));
        assert_eq!(decode(&encode(&[])), Some(vec![]));
        assert_eq!(decode(b"GET / HTTP/1.1\r\n"), None);
    }
}
