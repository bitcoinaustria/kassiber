use std::io;
#[cfg(unix)]
use std::io::Write;
use std::path::Path;
#[cfg(unix)]
use std::process::{Command, Stdio};

/// Writes `contents` to `path` as an owner-only executable that can be run
/// immediately, replacing any file already there.
///
/// Exec fails with `ETXTBSY` while any process holds the file open for
/// writing. Had this process written it, a child that another test thread
/// forked meanwhile would hold the descriptor until it execs. A short-lived
/// `sh` writes it instead, so no descriptor outlives the call.
#[cfg(unix)]
pub fn write_executable(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let mut writer = Command::new("/bin/sh")
        .args([
            "-c",
            r#"umask 077 && cat > "$1.$$" && chmod 700 "$1.$$" && mv -f "$1.$$" "$1""#,
            "sh",
        ])
        .arg(path)
        .stdin(Stdio::piped())
        .spawn()?;
    let written = writer
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(contents.as_ref());
    let status = writer.wait()?;
    written?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "writing {} exited with {status}",
            path.display()
        )))
    }
}

#[cfg(not(unix))]
pub fn write_executable(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    std::fs::write(path, contents)
}
