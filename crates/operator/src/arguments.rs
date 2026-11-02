//! Secret-bearing command arguments and durations (`client.py`).
//!
//! `prepare_arguments` replaces each `--*-fd N`, `--*-fd=N` and the one
//! `--*-stdin` flag with an opaque `broker-secret-<hex>` label and reads the
//! value, so the command line the broker sees holds no secret and the
//! values travel only in `S` frames.

use crate::client::PreparedArguments;
use crate::error::{app, Error, Result};
use crate::protocol::MAX_SECRET_PAYLOAD_BYTES;
use kassiber_pyjson::{BigInt, PyStr};
use std::ffi::OsStr;
use std::io::Read;
use zeroize::Zeroizing;

/// `kassiber.secrets.prompt.MAX_PASSPHRASE_BYTES`, which the CLI child
/// enforces when it reads a brokered secret.
pub const MAX_PASSPHRASE_BYTES: usize = 8192;
/// `MAX_CLIENT_SECRET_BYTES`: what both the child and the wire accept.
pub const MAX_CLIENT_SECRET_BYTES: usize = if MAX_PASSPHRASE_BYTES < MAX_SECRET_PAYLOAD_BYTES {
    MAX_PASSPHRASE_BYTES
} else {
    MAX_SECRET_PAYLOAD_BYTES
};

/// `os.fsdecode`: UTF-8 with surrogate escapes on POSIX, UTF-16 with lone
/// surrogates kept on Windows.
pub fn os_text(text: &OsStr) -> PyStr {
    #[cfg(unix)]
    {
        let mut bytes = std::os::unix::ffi::OsStrExt::as_bytes(text);
        let mut points = Vec::new();
        while !bytes.is_empty() {
            match std::str::from_utf8(bytes) {
                Ok(valid) => {
                    points.extend(valid.chars().map(u32::from));
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    points.extend(
                        std::str::from_utf8(&bytes[..valid])
                            .unwrap_or_default()
                            .chars()
                            .map(u32::from),
                    );
                    let end = valid + error.error_len().unwrap_or(bytes.len() - valid);
                    points.extend(bytes[valid..end].iter().map(|b| 0xdc00 + u32::from(*b)));
                    bytes = &bytes[end..];
                }
            }
        }
        PyStr::from_code_points(points).unwrap_or_default()
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let points = char::decode_utf16(text.encode_wide())
            .map(|unit| unit.map_or_else(|error| u32::from(error.unpaired_surrogate()), u32::from));
        PyStr::from_code_points(points).unwrap_or_default()
    }
}

/// How the arguments' secret values are read.
pub struct SecretSources<'a> {
    /// `sys.stdin.buffer`.
    pub stdin: &'a mut dyn Read,
    /// `os.fdopen(fd, "rb", closefd=True)`.
    pub open_fd: &'a dyn Fn(i32) -> Box<dyn Read>,
    /// `secrets.token_hex(16)`.
    pub token: &'a dyn Fn() -> Result<String>,
}

/// `prepare_arguments`: replace secret fd/stdin inputs with opaque labels.
pub fn prepare_arguments(argv: &[String], sources: SecretSources<'_>) -> Result<PreparedArguments> {
    let mut prepared = PreparedArguments::default();
    let mut index = 0;
    let mut stdin_consumed = false;
    // On any failure `prepared` (and every value read so far) is dropped,
    // which wipes the values.
    while index < argv.len() {
        let token = &argv[index];
        let (flag, inline_value) = match token.split_once('=') {
            Some((flag, value)) => (flag, Some(value)),
            None => (token.as_str(), None),
        };
        if let Some(inline_value) =
            inline_value.filter(|_| flag.starts_with("--") && flag.ends_with("-fd"))
        {
            let label = format!("broker-secret-{}", (sources.token)()?);
            let value = read_secret_fd(parse_secret_fd(inline_value)?, sources.open_fd)?;
            prepared.secrets.push((label.clone(), value));
            prepared.argv.extend([flag.to_owned(), label]);
            index += 1;
            continue;
        }
        if token.starts_with("--") && token.ends_with("-fd") {
            let Some(value) = argv.get(index + 1) else {
                return Err(Error::App(app(
                    "operator_invalid_command",
                    "secret fd flag requires a value",
                )));
            };
            let label = format!("broker-secret-{}", (sources.token)()?);
            let value = read_secret_fd(parse_secret_fd(value)?, sources.open_fd)?;
            prepared.secrets.push((label.clone(), value));
            prepared.argv.extend([token.clone(), label]);
            index += 2;
            continue;
        }
        if token.starts_with("--") && token.ends_with("-stdin") {
            if stdin_consumed {
                return Err(Error::App(app(
                    "multiple_stdin_secrets",
                    "only one stdin secret can be brokered per command",
                )));
            }
            let label = format!("broker-secret-{}", (sources.token)()?);
            let value = read_limited(sources.stdin)?;
            prepared.secrets.push((label.clone(), value));
            prepared.argv.extend([
                format!("{}fd", &token[..token.len() - "stdin".len()]),
                label,
            ]);
            stdin_consumed = true;
            index += 1;
            continue;
        }
        prepared.argv.push(token.clone());
        index += 1;
    }
    Ok(prepared)
}

fn read_secret_fd(fd: i32, open_fd: &dyn Fn(i32) -> Box<dyn Read>) -> Result<Zeroizing<Vec<u8>>> {
    let mut handle = open_fd(fd);
    read_limited(handle.as_mut()).map_err(|error| match error {
        Error::Io(_) => Error::App(app(
            "secret_input_error",
            "could not read brokered secret fd",
        )),
        other => other,
    })
}

/// `_parse_secret_fd`.
fn parse_secret_fd(value: &str) -> Result<i32> {
    let fd = kassiber_pytext::parse_int(value).map_err(|_| {
        Error::App(app(
            "operator_invalid_command",
            "secret fd flag requires an integer descriptor",
        ))
    })?;
    if fd < BigInt::from(0) {
        return Err(Error::App(app(
            "operator_invalid_command",
            "secret fd flag requires a non-negative descriptor",
        )));
    }
    // os.fdopen raises OverflowError beyond a C int; Python does not catch it.
    i32::try_from(fd).map_err(|_| Error::Value("fd is greater than maximum".into()))
}

/// `_read_limited`: at most the client limit, trailing CR/LF removed,
/// never empty.
pub fn read_limited(handle: &mut dyn Read) -> Result<Zeroizing<Vec<u8>>> {
    let mut value = Zeroizing::new(Vec::with_capacity(MAX_CLIENT_SECRET_BYTES + 1));
    let mut chunk = Zeroizing::new(vec![0u8; 4096]);
    loop {
        let want = 4096.min(MAX_CLIENT_SECRET_BYTES + 1 - value.len());
        let read = match handle.read(&mut chunk[..want]) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(Error::Io(error)),
        };
        if read == 0 {
            break;
        }
        value.extend_from_slice(&chunk[..read]);
        if value.len() > MAX_CLIENT_SECRET_BYTES {
            return Err(Error::App(app(
                "operator_secret_too_large",
                "brokered secret exceeds the application input limit",
            )));
        }
    }
    while matches!(value.last(), Some(b'\n' | b'\r')) {
        if let Some(last) = value.last_mut() {
            *last = 0;
        }
        value.pop();
    }
    if value.is_empty() {
        return Err(Error::App(app(
            "secret_input_error",
            "brokered secret is empty",
        )));
    }
    Ok(value)
}

/// `parse_duration`: `30m`, `8h`, `2d`; at least one minute.
pub fn parse_duration(value: &str) -> Result<BigInt> {
    let text = kassiber_pytext::lower(kassiber_pytext::strip(value));
    let invalid_unit = || {
        Error::App(app(
            "operator_invalid_duration",
            "duration must use m, h, or d (for example 30m or 8h)",
        ))
    };
    let mut chars = text.chars();
    let unit = chars.next_back().ok_or_else(invalid_unit)?;
    let multiplier = match unit {
        'm' => 60,
        'h' => 3600,
        'd' => 86400,
        _ => return Err(invalid_unit()),
    };
    if text.chars().count() < 2 {
        return Err(invalid_unit());
    }
    let amount = kassiber_pytext::parse_int(chars.as_str()).map_err(|_| {
        Error::App(app(
            "operator_invalid_duration",
            "invalid operator duration",
        ))
    })?;
    let seconds = amount * BigInt::from(multiplier);
    if seconds < BigInt::from(60) {
        return Err(Error::App(app(
            "operator_invalid_duration",
            "operator duration must be at least 1 minute",
        )));
    }
    Ok(seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> impl Fn() -> Result<String> {
        let next = std::cell::Cell::new(0u32);
        move || {
            next.set(next.get() + 1);
            Ok(format!("{:032x}", next.get()))
        }
    }

    fn no_fd(_: i32) -> Box<dyn Read> {
        panic!("no descriptor expected")
    }

    #[test]
    fn prepare_arguments_respects_child_and_wire_secret_limits() {
        assert_eq!(MAX_CLIENT_SECRET_BYTES, MAX_PASSPHRASE_BYTES);
        const { assert!(MAX_CLIENT_SECRET_BYTES <= MAX_SECRET_PAYLOAD_BYTES) };
        let payload = vec![b's'; MAX_CLIENT_SECRET_BYTES];
        let token = counter();
        let argv: Vec<String> = ["backends", "create", "--token-stdin"]
            .map(String::from)
            .into();
        let prepared = prepare_arguments(
            &argv,
            SecretSources {
                stdin: &mut payload.as_slice(),
                open_fd: &no_fd,
                token: &token,
            },
        )
        .unwrap();
        assert_eq!(prepared.secrets[0].1.as_slice(), payload.as_slice());
        assert_eq!(
            prepared.argv,
            [
                "backends",
                "create",
                "--token-fd",
                &format!("broker-secret-{:032x}", 1)
            ]
        );
        let mut over = payload.clone();
        over.push(b's');
        let error = prepare_arguments(
            &argv,
            SecretSources {
                stdin: &mut over.as_slice(),
                open_fd: &no_fd,
                token: &token,
            },
        )
        .err()
        .unwrap();
        assert_eq!(error.code(), Some("operator_secret_too_large"));
    }

    #[test]
    fn fd_forms_and_their_refusals_match_python() {
        let token = counter();
        let open_fd = |fd: i32| -> Box<dyn Read> {
            match fd {
                3 => Box::new(&b"inline\r\n"[..]),
                4 => Box::new(&b"spaced\n\n"[..]),
                _ => Box::new(std::io::Cursor::new(Vec::new())),
            }
        };
        let argv: Vec<String> = ["--a-fd=3", "--b-fd", "4", "--x=y", "plain"]
            .map(String::from)
            .into();
        let prepared = prepare_arguments(
            &argv,
            SecretSources {
                stdin: &mut std::io::empty(),
                open_fd: &open_fd,
                token: &token,
            },
        )
        .unwrap();
        assert_eq!(
            prepared.argv,
            [
                "--a-fd".to_owned(),
                format!("broker-secret-{:032x}", 1),
                "--b-fd".to_owned(),
                format!("broker-secret-{:032x}", 2),
                "--x=y".to_owned(),
                "plain".to_owned(),
            ]
        );
        assert_eq!(prepared.secrets[0].1.as_slice(), b"inline");
        assert_eq!(prepared.secrets[1].1.as_slice(), b"spaced");
        for (argv, code, message) in [
            (
                vec!["--a-fd"],
                "operator_invalid_command",
                "secret fd flag requires a value",
            ),
            (
                vec!["--a-fd", "x"],
                "operator_invalid_command",
                "secret fd flag requires an integer descriptor",
            ),
            (
                vec!["--a-fd=-1"],
                "operator_invalid_command",
                "secret fd flag requires a non-negative descriptor",
            ),
            (
                vec!["--a-fd", "5"],
                "secret_input_error",
                "brokered secret is empty",
            ),
            (
                vec!["--a-stdin", "--b-stdin"],
                "multiple_stdin_secrets",
                "only one stdin secret can be brokered per command",
            ),
            // Partitioned at the first "=", "--x=y-fd" still ends with -fd.
            (
                vec!["--x=y-fd"],
                "operator_invalid_command",
                "secret fd flag requires a value",
            ),
        ] {
            let argv: Vec<String> = argv.into_iter().map(String::from).collect();
            let error = prepare_arguments(
                &argv,
                SecretSources {
                    stdin: &mut &b"once"[..],
                    open_fd: &open_fd,
                    token: &token,
                },
            )
            .err()
            .unwrap();
            assert_eq!(
                (error.code(), error.to_string().as_str()),
                (Some(code), message)
            );
        }
        let failing = |_: i32| -> Box<dyn Read> {
            struct Broken;
            impl Read for Broken {
                fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                    Err(std::io::Error::from_raw_os_error(9))
                }
            }
            Box::new(Broken)
        };
        let error = prepare_arguments(
            &["--a-fd".into(), "7".into()],
            SecretSources {
                stdin: &mut std::io::empty(),
                open_fd: &failing,
                token: &token,
            },
        )
        .err()
        .unwrap();
        assert_eq!(error.to_string(), "could not read brokered secret fd");
        assert_eq!(error.code(), Some("secret_input_error"));
    }

    #[test]
    fn durations_match_python() {
        for (text, seconds) in [
            ("30m", 1800),
            (" 8H ", 28800),
            ("1d", 86400),
            ("1_0m", 600),
            ("+2h", 7200),
        ] {
            assert_eq!(
                parse_duration(text).unwrap(),
                BigInt::from(seconds),
                "{text}"
            );
        }
        for (text, message) in [
            ("m", "duration must use m, h, or d (for example 30m or 8h)"),
            (
                "30s",
                "duration must use m, h, or d (for example 30m or 8h)",
            ),
            ("", "duration must use m, h, or d (for example 30m or 8h)"),
            ("xm", "invalid operator duration"),
            ("0m", "operator duration must be at least 1 minute"),
            ("-1h", "operator duration must be at least 1 minute"),
        ] {
            let error = parse_duration(text).unwrap_err();
            assert_eq!(error.code(), Some("operator_invalid_duration"));
            assert_eq!(error.to_string(), message, "{text}");
        }
    }

    #[test]
    fn os_text_keeps_undecodable_bytes_as_surrogate_escapes() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let text = os_text(OsStr::from_bytes(b"/tmp/B\xfccher"));
            let points: Vec<u32> = text.code_points().collect();
            assert_eq!(&points[5..], &[0x42, 0xdcfc, 0x63, 0x68, 0x65, 0x72]);
        }
    }
}
