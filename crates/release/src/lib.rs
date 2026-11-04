//! `kassiber update` and `kassiber verify-download`.
//!
//! Update checks contact GitHub only after the explicit command, with the
//! persisted consent and outside offline mode, through the shared egress gate.
//! Release verification is local and in process: it pins the release key by
//! fingerprint, checks the OpenSSH signature the way `ssh-keygen -Y verify`
//! does with one canonical allowed-signers entry, and then checks the
//! artifact against the authenticated SHA-256 manifest.
pub mod update;
pub mod verify;

use kassiber_envelope::AppError;

pub type Result<T> = std::result::Result<T, AppError>;

/// `str.isspace()` for one character, the separator set Python's
/// `str.split()` and `str.strip()` use.
fn is_space(c: char) -> bool {
    kassiber_pytext::is_space(c as u32)
}

/// `str.strip()`.
fn strip(text: &str) -> &str {
    text.trim_matches(is_space)
}

/// `str.splitlines()`.
fn splitlines(text: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((index, c)) = chars.next() {
        let breaks = matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        );
        if !breaks {
            continue;
        }
        lines.push(&text[start..index]);
        let mut end = index + c.len_utf8();
        if c == '\r' {
            if let Some((next, '\n')) = chars.peek().copied() {
                chars.next();
                end = next + 1;
            }
        }
        start = end;
    }
    if start < text.len() {
        lines.push(&text[start..]);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitlines_and_strip_follow_python() {
        assert_eq!(splitlines("a\r\nb\rc\x0bd\n"), ["a", "b", "c", "d"]);
        assert_eq!(splitlines("a\n\nb"), ["a", "", "b"]);
        assert_eq!(strip("\x1c x \x1f"), "x");
    }
}
